use dashmap::DashMap;
use hex;
use sp_core::blake2_256;
use sp_runtime::traits::Verify;
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{anyhow, Result};
use jsonrpsee::{
    core::{async_trait, RpcResult, SubscriptionResult},
    proc_macros::rpc,
    server::ServerBuilder,
    PendingSubscriptionSink, SubscriptionMessage,
};
use log::{error, info, trace, warn};
use primitives::data_structure::{
    BackendEvent, ChainSupported, DbTxStateMachine, StorageExport, SystemNotification,
    TxStateMachine, TxStatus,
};
use prometheus_client::{
    metrics::{counter::Counter, gauge::Gauge},
    registry::Registry,
};
use serde::{Deserialize, Serialize};
use tokio::{
    net::TcpListener,
    sync::{broadcast, mpsc, Mutex},
};

pub type Data = Vec<u8>;

struct RequestEntry {
    data: Data,
    created_at: u64,
    original_multi_id: String, // Store original multi_id for lookups
}

impl RequestEntry {
    fn new(data: Data, original_multi_id: String) -> Self {
        Self {
            data,
            created_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            original_multi_id,
        }
    }

    fn is_expired(&self, ttl_seconds: u64) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        now.saturating_sub(self.created_at) > ttl_seconds
    }

    fn update_data(&mut self, data: Data) {
        self.data = data;
    }
}

/// Generate an extended multi_id from multi_id and current timestamp using blake2_256
fn generate_extended_multi_id(multi_id: &str, timestamp: u64) -> String {
    // Combine multi_id and timestamp into a single byte vector
    let mut data_to_hash = multi_id.as_bytes().to_vec();
    data_to_hash.extend_from_slice(&timestamp.to_le_bytes());
    
    // Hash using blake2_256 (returns [u8; 32])
    let hash = blake2_256(&data_to_hash);
    
    // Return hex-encoded string
    hex::encode(hash)
}

struct RequestListEntry {
    multi_ids: Vec<String>,
    created_at: u64,
}

impl RequestListEntry {
    fn new() -> Self {
        Self {
            multi_ids: Vec::new(),
            created_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        }
    }

    fn is_expired(&self, ttl_seconds: u64) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        now.saturating_sub(self.created_at) > ttl_seconds
    }

    fn push(&mut self, multi_id: String) {
        self.multi_ids.push(multi_id);
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TargetPeer {
    account_id: String,
    network: ChainSupported,
    times_requested: u16,
}
pub type TargetPeers = HashMap<String, TargetPeer>;

const VANE_SR25519_PUBLIC_KEY: &str =
    "0x9e00014e27effe047f489daf60849d63d4f870728d660e849a0ca7674f453b46";

pub fn verify_client_key_middleware(sig: Vec<u8>, msg: String) -> Result<bool, anyhow::Error> {
    use anyhow::anyhow;
    use sp_core::sr25519;

    let address = VANE_SR25519_PUBLIC_KEY.trim_start_matches("0x");

    let pub_bytes: [u8; 32] = hex::decode(address)?
        .try_into()
        .map_err(|_| anyhow!("Invalid public key length"))?;
    let public = sr25519::Public::from_raw(pub_bytes);

    let sig_bytes: [u8; 64] = sig
        .try_into()
        .map_err(|_| anyhow!("Invalid signature length"))?;
    let signature = sr25519::Signature::from_raw(sig_bytes);

    Ok(signature.verify(msg.as_bytes(), &public))
}

pub struct VaneSwarmServer {
    peers: HashMap<String, TargetPeers>,
    requests: Arc<DashMap<String, RequestEntry>>,
    sender_requests: Arc<DashMap<String, RequestListEntry>>,
    receiver_requests: Arc<DashMap<String, RequestListEntry>>,
    request_ttl_seconds: u64,
    metrics: Arc<MetricService>,
    event_sender: broadcast::Sender<BackendEvent>,
    system_notification_sender: mpsc::Sender<SystemNotification>,
    db: Option<Arc<crate::db::D1Client>>,
}

impl VaneSwarmServer {
    pub fn new(
        metrics: Arc<MetricService>,
        event_sender: broadcast::Sender<BackendEvent>,
        system_notification_sender: mpsc::Sender<SystemNotification>,
        request_ttl_seconds: u64,
        db: Option<Arc<crate::db::D1Client>>,
    ) -> Self {
        Self {
            peers: HashMap::new(),
            requests: Arc::new(DashMap::new()),
            sender_requests: Arc::new(DashMap::new()),
            receiver_requests: Arc::new(DashMap::new()),
            request_ttl_seconds,
            metrics,
            event_sender,
            system_notification_sender,
            db,
        }
    }

    fn cleanup_expired_requests(&self) {
        let mut expired_keys = Vec::new();
        
        for entry in self.requests.iter() {
            if entry.value().is_expired(self.request_ttl_seconds) {
                expired_keys.push(entry.key().clone());
            }
        }
        
        for key in expired_keys {
            self.requests.remove(&key);
        }
    }

    fn cleanup_expired_sender_receiver_requests(&self) {
        let mut expired_extended_multi_ids: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        for entry in self.requests.iter() {
            if entry.value().is_expired(self.request_ttl_seconds) {
                expired_extended_multi_ids.insert(entry.key().clone());
            }
        }

        let sender_keys: Vec<String> = self
            .sender_requests
            .iter()
            .map(|e| e.key().clone())
            .collect();
        for key in sender_keys {
            if let Some(mut entry) = self.sender_requests.get_mut(&key) {
                if entry.is_expired(self.request_ttl_seconds) {
                    drop(entry);
                    self.sender_requests.remove(&key);
                    continue;
                }

                if !expired_extended_multi_ids.is_empty() {
                    entry
                        .multi_ids
                        .retain(|extended_multi_id| !expired_extended_multi_ids.contains(extended_multi_id));
                    if entry.multi_ids.is_empty() {
                        drop(entry);
                        self.sender_requests.remove(&key);
                    }
                }
            }
        }

        let receiver_keys: Vec<String> = self
            .receiver_requests
            .iter()
            .map(|e| e.key().clone())
            .collect();
        for key in receiver_keys {
            if let Some(mut entry) = self.receiver_requests.get_mut(&key) {
                if entry.is_expired(self.request_ttl_seconds) {
                    drop(entry);
                    self.receiver_requests.remove(&key);
                    continue;
                }

                if !expired_extended_multi_ids.is_empty() {
                    entry
                        .multi_ids
                        .retain(|extended_multi_id| !expired_extended_multi_ids.contains(extended_multi_id));
                    if entry.multi_ids.is_empty() {
                        drop(entry);
                        self.receiver_requests.remove(&key);
                    }
                }
            }
        }
    }

    fn insert_request(&self, original_multi_id: String, data: Data) -> String {
        self.cleanup_expired_requests();
        self.cleanup_expired_sender_receiver_requests();
        
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let extended_multi_id = generate_extended_multi_id(&original_multi_id, timestamp);
        
        // Insert with extended_multi_id (multi_id + timestamp)
        self.requests.insert(extended_multi_id.clone(), RequestEntry::new(data, original_multi_id));
        
        extended_multi_id
    }

    fn update_request_by_extended_multi_id(&self, extended_multi_id: &str, data: Data) -> bool {
        self.cleanup_expired_requests();
        self.cleanup_expired_sender_receiver_requests();

        match self.requests.get_mut(extended_multi_id) {
            Some(mut entry) => {
                if entry.is_expired(self.request_ttl_seconds) {
                    let expired_data = entry.data.clone();
                    let original_multi_id = entry.original_multi_id.clone();
                    drop(entry);
                    self.requests.remove(extended_multi_id);
                    let event = BackendEvent::DataExpired {
                        multi_id: original_multi_id,
                        data: expired_data,
                    };
                    let _ = self.event_sender.send(event);
                    false
                } else {
                    entry.update_data(data);
                    true
                }
            }
            None => {
                // Extended multi_id not found - request may have expired or never existed
                warn!("Request with extended_multi_id {} not found for update", extended_multi_id);
                let event = BackendEvent::DataExpired {
                    multi_id: extended_multi_id.to_string(),
                    data: Vec::new(),
                };
                let _ = self.event_sender.send(event);
                false
            }
        }
    }
    
    // Helper to find extended_multi_id from original multi_id by searching
    fn find_extended_multi_id_by_original_multi_id(&self, original_multi_id: &str) -> Option<String> {
        for entry in self.requests.iter() {
            if entry.value().original_multi_id == original_multi_id {
                return Some(entry.key().clone());
            }
        }
        None
    }

    pub async fn handle_sender_request(&mut self, address: String, data: Data) -> Result<()> {
        info!(
            "Received sender request from address: {}", address
        );

        let received_event = BackendEvent::SenderRequestReceived {
            address: address.clone(),
            data: data.clone(),
        };
        let send_result = self.event_sender.send(received_event);
        match send_result {
            Ok(count) => info!(
                "Broadcast SenderRequestReceived event to {} receivers",
                count
            ),
            Err(e) => error!("Failed to broadcast SenderRequestReceived event: {}", e),
        }

        let tx_state: TxStateMachine = serde_json::from_slice(&data).map_err(|e| {
            error!(
                "Failed to decode TxStateMachine from sender {}: {}",
                address, e
            );
            anyhow!("Failed to decode TxStateMachine: {}", e)
        })?;

        let receiver_address = tx_state.receiver_address.clone();
        let receiver_network = tx_state.receiver_address_network;
        let multi_id_hex = hex::encode(tx_state.multi_id);

        if !self.peers.contains_key(&address) {
            let mut target_peers = HashMap::new();
            let peer = TargetPeer {
                account_id: receiver_address.clone(),
                network: receiver_network,
                times_requested: 1,
            };
            target_peers.insert(receiver_address.clone(), peer);
            self.peers.insert(address.clone(), target_peers);
            self.metrics.record_peer_added().await;
            self.update_metrics().await;

            let notification = SystemNotification::PeerAdded {
                address: address.clone(),
                account_id: receiver_address.clone(),
                network: receiver_network,
            };

            info!("succesfully added new peer: {:?}", notification);

            let _ = self.system_notification_sender.try_send(notification);
        } else {
            if let Some(target_peers) = self.peers.get_mut(&address) {
                if let Some(peer) = target_peers.get_mut(&receiver_address) {
                    peer.times_requested += 1;
                    info!("Updated peer request count - sender: {}, receiver: {}, times_requested: {}", address, receiver_address, peer.times_requested);
                } else {
                    let new_peer = TargetPeer {
                        account_id: receiver_address.clone(),
                        network: receiver_network,
                        times_requested: 1,
                    };

                    target_peers.insert(receiver_address.clone(), new_peer);
                    let notification = SystemNotification::PeerAdded {
                        address: address.clone(),
                        account_id: receiver_address.clone(),
                        network: receiver_network,
                    };

                    info!("succesfully added new peer: {:?}", notification);
                }
            }
        }

        let notification = SystemNotification::RequestQueued {
            address: receiver_address.clone(),
        };
        let _ = self
            .system_notification_sender
            .try_send(notification.clone());
        trace!(
            "succesfully queued request for receiver: {:?}",
            notification
        );

        // Insert request and get the extended_multi_id (multi_id + timestamp)
        let extended_multi_id = self.insert_request(multi_id_hex.clone(), data.clone());
        
        // Write to database (async, non-blocking)
        if let Some(db) = &self.db {
            let tx_json = String::from_utf8_lossy(&data).to_string();
            let tx_lifecycle = crate::db::TxLifecycle {
                extended_multi_id: extended_multi_id.clone(),
                sender_address: address.clone(),
                receiver_address: receiver_address.clone(),
                tx_json,
                receiver_confirmed: false,
                reverted: false,
                completed: false,
            };
            let db_clone = db.clone();
            tokio::spawn(async move {
                if let Err(e) = db_clone.upsert_tx_lifecycle(&tx_lifecycle).await {
                    error!("Failed to write tx_lifecycle to DB: {}", e);
                }
            });
        }
        
        // Store extended_multi_id in sender/receiver request lists
        self.sender_requests
            .entry(address.clone())
            .or_insert_with(RequestListEntry::new)
            .push(extended_multi_id.clone());
        self.receiver_requests
            .entry(receiver_address.clone())
            .or_insert_with(RequestListEntry::new)
            .push(extended_multi_id);
        self.metrics.record_sender_request(&address).await;
        self.update_metrics().await;

        let event = BackendEvent::SenderRequestHandled {
            address: receiver_address.clone(),
            data,
        };
        let send_result = self.event_sender.send(event.clone());
        match send_result {
            Ok(count) => info!(
                "Broadcast SenderRequestHandled event to {} receivers",
                count
            ),
            Err(e) => error!("Failed to broadcast SenderRequestHandled event: {}", e),
        }

        let trimmed_data = if event.get_data().len() > 100 {
            format!("{:?}...", &event.get_data()[..97])
        } else {
            format!("{:?}", event.get_data())
        };

        info!(
            "succesfully handled sender request: address: {}, data: {}",
            event.get_address(),
            trimmed_data
        );
        Ok(())
    }

    pub async fn handle_sender_confirmation(&mut self, address: String, data: Data) -> Result<()> {
        info!("Received sender confirmation from address: {}", address);

        let tx_state: TxStateMachine = serde_json::from_slice(&data).map_err(|e| {
            error!(
                "Failed to decode TxStateMachine from sender confirmation {}: {}",
                address, e
            );
            anyhow!("Failed to decode TxStateMachine: {}", e)
        })?;

        let multi_id_hex = hex::encode(tx_state.multi_id);

        // Ensure the peer relationship exists (sender -> receiver)
        let receiver_address = tx_state.receiver_address.clone();
        if !self
            .peers
            .get(&address)
            .map(|targets| targets.contains_key(&receiver_address))
            .unwrap_or(false)
        {
            warn!(
                "Peer not found for sender {} and receiver {} during sender confirmation",
                address, receiver_address
            );
            return Err(anyhow!(
                "Peer not found for sender {} and receiver {}",
                address,
                receiver_address
            ));
        }

        // Find the extended_multi_id for this transaction
        let extended_multi_id = match self.find_extended_multi_id_by_original_multi_id(&multi_id_hex) {
            Some(key) => key,
            None => {
                warn!(
                    "Transaction with multi_id {} not found for sender confirmation from {}",
                    multi_id_hex, address
                );
                return Err(anyhow!(
                    "Transaction with multi_id {} not found",
                    multi_id_hex
                ));
            }
        };

        // Update the stored transaction data using extended_multi_id
        if self.update_request_by_extended_multi_id(&extended_multi_id, data.clone()) {
            info!(
                "Updated transaction (sender confirmation) with multi_id {}",
                multi_id_hex
            );
        } else {
            warn!(
                "Failed to update transaction with multi_id {} for sender confirmation",
                multi_id_hex
            );
            return Err(anyhow!(
                "Failed to update transaction with multi_id {}",
                multi_id_hex
            ));
        }

        // Update tx_lifecycle in database (async, non-blocking)
        if let Some(db) = &self.db {
            let tx_json = String::from_utf8_lossy(&data).to_string();
            let tx_lifecycle = crate::db::TxLifecycle {
                extended_multi_id: extended_multi_id.clone(),
                sender_address: address.clone(),
                receiver_address: receiver_address.clone(),
                tx_json,
                receiver_confirmed: true, // Confirmation implies receiver confirmed
                reverted: false,
                completed: false,
            };
            let db_clone = db.clone();
            tokio::spawn(async move {
                if let Err(e) = db_clone.upsert_tx_lifecycle(&tx_lifecycle).await {
                    error!("Failed to update tx_lifecycle in DB: {}", e);
                }
            });
        }

        // Emit backend event for sender confirmation
        let event = BackendEvent::SenderConfirmed {
            address: address.clone(),
            data,
        };
        let _ = self.event_sender.send(event.clone());

        let trimmed_data = if event.get_data().len() > 100 {
            format!("{:?}...", &event.get_data()[..97])
        } else {
            format!("{:?}", event.get_data())
        };

        info!(
            "Successfully handled sender confirmation: address: {}, data: {}",
            event.get_address(),
            trimmed_data
        );

        Ok(())
    }

    pub async fn handle_sender_revertation(&mut self, address: String, data: Data) -> Result<()> {
        info!("Received sender revertation from address: {}", address);

        let tx_state: TxStateMachine = serde_json::from_slice(&data).map_err(|e| {
            error!(
                "Failed to decode TxStateMachine from sender revertation {}: {}",
                address, e
            );
            anyhow!("Failed to decode TxStateMachine: {}", e)
        })?;

        let multi_id_hex = hex::encode(tx_state.multi_id);
        let receiver_address = tx_state.receiver_address.clone();

        info!(
            "in server, receiver address: {}, sender address: {}, tx revertation status: {:?}",
            tx_state.receiver_address, receiver_address, tx_state.status
        );

        // Find the extended_multi_id for this transaction
        let extended_multi_id = match self.find_extended_multi_id_by_original_multi_id(&multi_id_hex) {
            Some(key) => key,
            None => {
                warn!(
                    "Transaction with multi_id {} not found for sender revertation from {}",
                    multi_id_hex, address
                );
                return Err(anyhow!(
                    "Transaction with multi_id {} not found",
                    multi_id_hex
                ));
            }
        };

        // Update the stored transaction data using extended_multi_id
        if self.update_request_by_extended_multi_id(&extended_multi_id, data.clone()) {
            info!(
                "Updated transaction (sender revertation) with multi_id {}",
                multi_id_hex
            );
        } else {
            warn!(
                "Failed to update transaction with multi_id {} for sender revertation",
                multi_id_hex
            );
            return Err(anyhow!(
                "Failed to update transaction with multi_id {}",
                multi_id_hex
            ));
        }

        // Update reverted status in database (async, non-blocking)
        // Phase tracking (before/after receiver confirmation) is handled by receiver_confirmed flag
        if let Some(db) = &self.db {
            let db_clone = db.clone();
            let extended_multi_id_clone = extended_multi_id.clone();
            let tx_json = String::from_utf8_lossy(&data).to_string();
            let address_clone = address.clone();
            let receiver_address_clone = receiver_address.clone();
            let data_clone = data.clone();
            tokio::spawn(async move {
                // First try to get current state to preserve receiver_confirmed status
                match db_clone.get_tx_lifecycle(&extended_multi_id_clone).await {
                    Ok(Some(existing)) => {
                        let tx_lifecycle = crate::db::TxLifecycle {
                            extended_multi_id: extended_multi_id_clone,
                            sender_address: address_clone,
                            receiver_address: receiver_address_clone,
                            tx_json: String::from_utf8_lossy(&data_clone).to_string(),
                            receiver_confirmed: existing.receiver_confirmed, // Preserve existing status
                            reverted: true,
                            completed: false,
                        };
                        if let Err(e) = db_clone.upsert_tx_lifecycle(&tx_lifecycle).await {
                            error!("Failed to update tx_reverted in DB: {}", e);
                        }
                    }
                    _ => {
                        // If not found, just update reverted flag
                        if let Err(e) = db_clone.update_tx_reverted(&extended_multi_id_clone, true).await {
                            error!("Failed to update tx_reverted in DB: {}", e);
                        }
                    }
                }
            });
        }

        // Emit backend event for sender revertation
        let event = BackendEvent::SenderReverted {
            address: address.clone(),
            data: data.clone(),
        };
        let _ = self.event_sender.send(event.clone());

        let trimmed_data = if event.get_data().len() > 100 {
            format!("{:?}...", &event.get_data()[..97])
        } else {
            format!("{:?}", event.get_data())
        };

        info!(
            "Successfully handled sender revertation: address: {}, data: {}",
            event.get_address(),
            trimmed_data
        );

        // Peer and receiver_request cleanup (formerly in `disconnect_peer`)
        let account_id = receiver_address.as_str();

        // Find the extended_multi_id and remove from receiver_requests
        if let Some(extended_multi_id) = self.find_extended_multi_id_by_original_multi_id(&multi_id_hex) {
            if let Some(mut entry) = self.receiver_requests.get_mut(account_id) {
                entry.multi_ids.retain(|emid| emid != &extended_multi_id);
                if entry.multi_ids.is_empty() {
                    drop(entry);
                    self.receiver_requests.remove(account_id);
                }
                info!(
                    "Removed extended_multi_id {} (multi_id: {}) from receiver_requests for account_id: {}",
                    extended_multi_id, multi_id_hex, account_id
                );
            } else {
                warn!(
                    "Receiver address {} not found in receiver_requests during revertation cleanup",
                    account_id
                );
            }
            
            // Remove the actual request
            self.requests.remove(&extended_multi_id);
        } else {
            warn!(
                "Could not find extended_multi_id for multi_id {} during revertation cleanup",
                multi_id_hex
            );
        }

        // Find the extended_multi_id and also clean up from sender_requests
        if let Some(extended_multi_id) = self.find_extended_multi_id_by_original_multi_id(&multi_id_hex) {
            // Remove from sender_requests for the sender address
            if let Some(mut entry) = self.sender_requests.get_mut(&address) {
                entry.multi_ids.retain(|emid| emid != &extended_multi_id);
                if entry.multi_ids.is_empty() {
                    drop(entry);
                    self.sender_requests.remove(&address);
                }
            }
        }
        
        let mut sender_key_to_remove: Option<String> = None;

        for (sender_key, target_peers) in self.peers.iter_mut() {
            if let Some(peer) = target_peers.get(account_id) {
                if peer.times_requested == 1 {
                    target_peers.remove(account_id);
                    if target_peers.is_empty() {
                        sender_key_to_remove = Some(sender_key.clone());
                    }
                    self.metrics.record_peer_removed().await;

                    let notification = SystemNotification::PeerRemoved {
                        address: sender_key.clone(),
                    };
                    let _ = self.system_notification_sender.try_send(notification);

                    let event = BackendEvent::PeerDisconnected {
                        account_id: account_id.to_string(),
                    };
                    let _ = self.event_sender.send(event.clone());
                    info!(
                        "succesfully disconnected peer during revertation: {:?}",
                        event
                    );

                    if let Some(key) = sender_key_to_remove {
                        self.peers.remove(&key);
                    }
                    self.update_metrics().await;
                    return Ok(());
                } else {
                    info!(
                        "No-op: times_requested is {} (more than 1) for peer - sender: {}, account_id: {} during revertation cleanup",
                        peer.times_requested, sender_key, account_id
                    );
                    return Ok(());
                }
            }
        }

        warn!(
            "Peer with account_id {} not found during revertation cleanup",
            account_id
        );

        Ok(())
    }

    pub async fn handle_tx_submission_updates(
        &mut self,
        address: String,
        data: Data,
    ) -> Result<()> {
        info!("Received tx submission update from address: {}", address);

        let tx_state: TxStateMachine = serde_json::from_slice(&data).map_err(|e| {
            error!(
                "Failed to decode TxStateMachine from tx submission update {}: {}",
                address, e
            );
            anyhow!("Failed to decode TxStateMachine: {}", e)
        })?;

        let multi_id_hex = hex::encode(tx_state.multi_id);
        let receiver_address = tx_state.receiver_address.clone();

        // Ensure the peer relationship exists (sender -> receiver)
        if !self
            .peers
            .get(&address)
            .map(|targets| targets.contains_key(&receiver_address))
            .unwrap_or(false)
        {
            warn!(
                "Peer not found for sender {} and receiver {} during tx submission update",
                address, receiver_address
            );
            return Err(anyhow!(
                "Peer not found for sender {} and receiver {}",
                address,
                receiver_address
            ));
        }

        // Find the extended_multi_id for this transaction
        let extended_multi_id = match self.find_extended_multi_id_by_original_multi_id(&multi_id_hex) {
            Some(key) => key,
            None => {
                warn!(
                    "Transaction with multi_id {} not found for tx submission update from {}",
                    multi_id_hex, address
                );
                return Err(anyhow!(
                    "Transaction with multi_id {} not found",
                    multi_id_hex
                ));
            }
        };

        // Update the stored transaction data using extended_multi_id
        if self.update_request_by_extended_multi_id(&extended_multi_id, data.clone()) {
            info!(
                "Updated transaction (tx submission update) with multi_id {}",
                multi_id_hex
            );
        } else {
            warn!(
                "Failed to update transaction with multi_id {} for tx submission update",
                multi_id_hex
            );
            return Err(anyhow!(
                "Failed to update transaction with multi_id {}",
                multi_id_hex
            ));
        }

        // Update completed status in database (async, non-blocking)
        if let Some(db) = &self.db {
            let db_clone = db.clone();
            let extended_multi_id_clone = extended_multi_id.clone();
            tokio::spawn(async move {
                if let Err(e) = db_clone.update_tx_completed(&extended_multi_id_clone, true).await {
                    error!("Failed to update tx_completed in DB: {}", e);
                }
            });
        }

        // Emit backend event for tx submission update
        let event = BackendEvent::TxSubmitted {
            address: address.clone(),
            data,
        };
        let _ = self.event_sender.send(event.clone());

        let trimmed_data = if event.get_data().len() > 100 {
            format!("{:?}...", &event.get_data()[..97])
        } else {
            format!("{:?}", event.get_data())
        };

        info!(
            "Successfully handled tx submission update: address: {}, data: {}",
            event.get_address(),
            trimmed_data
        );

        Ok(())
    }

    pub async fn handle_receiver_response(&mut self, address: String, data: Data) -> Result<()> {
        let tx_state: TxStateMachine = serde_json::from_slice(&data).map_err(|e| {
            error!(
                "Failed to decode TxStateMachine from receiver {}: {}",
                address, e
            );
            anyhow!("Failed to decode TxStateMachine: {}", e)
        })?;

        info!(
            "Received receiver response from address: {}  with tx state: {:?}",
            format!("{}...{}", &address[..4], &address[address.len() - 4..]),
            tx_state
        );

        let multi_id_hex = hex::encode(tx_state.multi_id);

        let received_event = BackendEvent::ReceiverResponseReceived {
            address: address.clone(),
            data: data.clone(),
        };
        let _ = self.event_sender.send(received_event);

        // Try to find existing request by original multi_id
        let extended_multi_id = if let Some(emid) = self.find_extended_multi_id_by_original_multi_id(&multi_id_hex) {
            if self.update_request_by_extended_multi_id(&emid, data.clone()) {
                info!("Updated existing request with multi_id: {} (extended_multi_id: {})", multi_id_hex, emid);
                emid
            } else {
                // Request expired, insert as new
                let new_extended_multi_id = self.insert_request(multi_id_hex.clone(), data.clone());
                info!("Inserted new request with multi_id: {} (extended_multi_id: {})", multi_id_hex, new_extended_multi_id);
                new_extended_multi_id
            }
        } else {
            // No existing request found, insert as new
            let new_extended_multi_id = self.insert_request(multi_id_hex.clone(), data.clone());
            info!("Inserted new request with multi_id: {} (extended_multi_id: {})", multi_id_hex, new_extended_multi_id);
            new_extended_multi_id
        };

        // Update receiver_confirmed in database (async, non-blocking)
        if let Some(db) = &self.db {
            let db_clone = db.clone();
            let extended_multi_id_clone = extended_multi_id.clone();
            tokio::spawn(async move {
                if let Err(e) = db_clone.update_receiver_confirmed(&extended_multi_id_clone, true).await {
                    error!("Failed to update receiver_confirmed in DB: {}", e);
                }
            });
        }

        self.metrics.record_receiver_response(&address).await;
        self.update_metrics().await;

        let notification = SystemNotification::RequestProcessed {
            address: address.clone(),
        };
        let _ = self.system_notification_sender.try_send(notification);

        let event = BackendEvent::ReceiverResponseHandled {
            address: address.clone(),
            data,
        };
        let _ = self.event_sender.send(event.clone());

        let trimmed_data = if event.get_data().len() > 100 {
            format!("{:?}...", &event.get_data()[..97])
        } else {
            format!("{:?}", event.get_data())
        };

        info!(
            "succesfully handled receiver response: address: {}, data: {}",
            event.get_address(),
            trimmed_data
        );
        Ok(())
    }

    pub async fn fetch_pending_transactions(&self, address: String) -> Result<Vec<TxStateMachine>> {
        info!("Fetching pending transactions for address: {}", address);
        // try fetching from both sender and receiver requests and get the multi_ids and fetch the data from the requests
        let mut pending_transactions = Vec::new();
        if let Some(sender_requests) = self.sender_requests.get(&address) {
            info!(
                "Found {} sender requests for address: {}",
                sender_requests.multi_ids.len(),
                address
            );
            for extended_multi_id in &sender_requests.multi_ids {
                if let Some(request) = self.requests.get(extended_multi_id) {
                    let tx_state: TxStateMachine =
                        serde_json::from_slice(&request.data).map_err(|e| {
                            error!(
                                "Failed to decode TxStateMachine from sender {}: {}",
                                address, e
                            );
                            anyhow!("Failed to decode TxStateMachine: {}", e)
                        })?;

                    info!("pending transaction on sender requests: {:?}", tx_state);
                    pending_transactions.push(tx_state);
                }
            }
        }
        if let Some(receiver_requests) = self.receiver_requests.get(&address) {
            info!(
                "Found {} receiver requests for address: {}",
                receiver_requests.multi_ids.len(),
                address
            );
            for extended_multi_id in &receiver_requests.multi_ids {
                if let Some(request) = self.requests.get(extended_multi_id) {
                    let tx_state: TxStateMachine =
                        serde_json::from_slice(&request.data).map_err(|e| {
                            error!(
                                "Failed to decode TxStateMachine from receiver {}: {}",
                                address, e
                            );
                            anyhow!("Failed to decode TxStateMachine: {}", e)
                        })?;
                    if !matches!(tx_state.status, TxStatus::Reverted(_)) {
                        info!("pending transaction on receiver requests: {:?}", tx_state);
                        pending_transactions.push(tx_state);
                    }
                }
            }
        }

        Ok(pending_transactions)
    }

    async fn update_metrics(&self) {
        self.cleanup_expired_requests();
        self.cleanup_expired_sender_receiver_requests();
        let peer_count = self.peers.len() as i64;
        let request_count = self.requests.len() as i64;
        self.metrics.update_active_peers(peer_count).await;
        self.metrics.update_pending_requests(request_count).await;
    }
}


#[rpc(server)]
pub trait BackendRpc {
    /// Handle sender request
    /// params:
    ///
    /// - `address`: The sender address
    /// - `data`: Request data (TxStateMachine encoded as JSON bytes)
    #[method(name = "handleSenderRequest")]
    async fn handle_sender_request(
        &self,
        sig: Vec<u8>,
        address: String,
        data: Vec<u8>,
    ) -> RpcResult<()>;

    /// Handle receiver response
    /// params:
    ///
    /// - `address`: The receiver address
    /// - `data`: Response data
    #[method(name = "handleReceiverResponse")]
    async fn handle_receiver_response(
        &self,
        sig: Vec<u8>,
        address: String,
        data: Vec<u8>,
    ) -> RpcResult<()>;

    /// Handle sender confirmation
    ///  params:
    ///
    /// - `address`: The sender address
    /// - `data`: updated TxStateMachine
    #[method(name = "handleSenderConfirmation")]
    async fn handle_sender_confirmation(
        &self,
        sig: Vec<u8>,
        address: String,
        data: Vec<u8>,
    ) -> RpcResult<()>;

    /// Handle sender reveration
    ///  params:
    ///
    /// - `address`: The sender address
    /// - `data`: updated TxStateMachine
    #[method(name = "handleSenderRevertation")]
    async fn handle_sender_revertation(
        &self,
        sig: Vec<u8>,
        address: String,
        data: Vec<u8>,
    ) -> RpcResult<()>;

    /// Handle tx submission updates
    /// params:
    ///
    /// - `address`: The sender address
    /// - `data`: updated TxStateMachinexf
    #[method(name = "handleTxSubmissionUpdates")]
    async fn handle_tx_submission_updates(
        &self,
        sig: Vec<u8>,
        address: String,
        data: Vec<u8>,
    ) -> RpcResult<()>;

    /// Fetch pending transactions
    /// params:
    ///
    /// - `address`: The address to fetch pending transactions for
    #[method(name = "fetchPendingTransactions")]
    async fn fetch_pending_transactions(&self, sig: Vec<u8>, address: String) -> RpcResult<()>;

    /// Subscribe to events filtered by address
    /// params:
    ///
    /// - `address`: The address to filter events for
    #[subscription(name = "subscribeToEvents", item = BackendEvent)]
    async fn subscribe_to_events(&self, address: String, sig: Vec<u8>) -> SubscriptionResult;

    /// Get transaction counts
    /// Returns transaction counts: (total, receiver_confirmed, reverted, completed, reverted_before_confirm)
    #[method(name = "getTxCounts")]
    async fn get_tx_counts(&self, sig: Vec<u8>, address: String) -> RpcResult<(usize, usize, usize, usize, usize)>;

    /// Get total transaction count
    #[method(name = "getTotalTxCount")]
    async fn get_total_tx_count(&self, sig: Vec<u8>, address: String) -> RpcResult<usize>;

    /// Get receiver confirmed transaction count
    #[method(name = "getReceiverConfirmedCount")]
    async fn get_receiver_confirmed_count(&self, sig: Vec<u8>, address: String) -> RpcResult<usize>;

    /// Get reverted transaction count
    #[method(name = "getRevertedCount")]
    async fn get_reverted_count(&self, sig: Vec<u8>, address: String) -> RpcResult<usize>;

    /// Get completed transaction count
    #[method(name = "getCompletedCount")]
    async fn get_completed_count(&self, sig: Vec<u8>, address: String) -> RpcResult<usize>;

    /// Get reverted before confirmation transaction count
    #[method(name = "getRevertedBeforeConfirmCount")]
    async fn get_reverted_before_confirm_count(&self, sig: Vec<u8>, address: String) -> RpcResult<usize>;
}

#[derive(Clone)]
pub struct BackendRpcHandler {
    swarm_server: Arc<Mutex<VaneSwarmServer>>,
    event_sender: broadcast::Sender<BackendEvent>,
}

impl BackendRpcHandler {
    pub fn new(
        swarm_server: Arc<Mutex<VaneSwarmServer>>,
        event_sender: broadcast::Sender<BackendEvent>,
    ) -> Self {
        Self {
            swarm_server,
            event_sender,
        }
    }
}

#[async_trait]
impl BackendRpcServer for BackendRpcHandler {
    async fn handle_sender_request(
        &self,
        sig: Vec<u8>,
        address: String,
        data: Vec<u8>,
    ) -> RpcResult<()> {

        let is_valid = verify_client_key_middleware(sig, address.clone())
            .map_err(|e| jsonrpsee::core::Error::Custom(e.to_string()))?;

        if !is_valid {
            return Err(jsonrpsee::core::Error::Custom(
                "Invalid client verification".to_string(),
            ).into());
        }

        info!("RPC: handle_sender_request called for address: {}", address);
        let mut server = self.swarm_server.lock().await;
        server
            .handle_sender_request(address, data)
            .await
            .map_err(|e| {
                error!("RPC: Failed to handle sender request: {}", e);
                jsonrpsee::core::Error::Custom(e.to_string())
            })?;
        Ok(())
    }

    async fn handle_receiver_response(
        &self,
        sig: Vec<u8>,
        address: String,
        data: Vec<u8>,
    ) -> RpcResult<()> {

        let is_valid = verify_client_key_middleware(sig, address.clone())
            .map_err(|e| jsonrpsee::core::Error::Custom(e.to_string()))?;

        if !is_valid {
            return Err(jsonrpsee::core::Error::Custom(
                "Invalid client verification".to_string(),
            ).into());
        }

        info!(
            "RPC: handle_receiver_response called for address: {}",
            address
        );
        let mut server = self.swarm_server.lock().await;
        server
            .handle_receiver_response(address, data)
            .await
            .map_err(|e| {
                error!("RPC: Failed to handle receiver response: {}", e);
                jsonrpsee::core::Error::Custom(e.to_string())
            })?;
        Ok(())
    }

    async fn handle_sender_confirmation(
        &self,
        sig: Vec<u8>,
        address: String,
        data: Vec<u8>,
    ) -> RpcResult<()> {

        let is_valid = verify_client_key_middleware(sig, address.clone())
            .map_err(|e| jsonrpsee::core::Error::Custom(e.to_string()))?;

        if !is_valid {
            return Err(jsonrpsee::core::Error::Custom(
                "Invalid client verification".to_string(),
            ).into());
        }

        info!(
            "RPC: handle_sender_confirmation called for address: {}",
            address
        );
        let mut server = self.swarm_server.lock().await;
        server
            .handle_sender_confirmation(address, data)
            .await
            .map_err(|e| {
                error!("RPC: Failed to handle sender confirmation: {}", e);
                jsonrpsee::core::Error::Custom(e.to_string())
            })?;
        Ok(())
    }

    async fn handle_sender_revertation(
        &self,
        sig: Vec<u8>,
        address: String,
        data: Vec<u8>,
    ) -> RpcResult<()> {

        let is_valid = verify_client_key_middleware(sig, address.clone())
            .map_err(|e| jsonrpsee::core::Error::Custom(e.to_string()))?;

        if !is_valid {
            return Err(jsonrpsee::core::Error::Custom(
                "Invalid client verification".to_string(),
            ).into());
        }

        info!(
            "RPC: handle_sender_revertation called for address: {}",
            address
        );
        let mut server = self.swarm_server.lock().await;
        server
            .handle_sender_revertation(address, data)
            .await
            .map_err(|e| {
                error!("RPC: Failed to handle sender revertation: {}", e);
                jsonrpsee::core::Error::Custom(e.to_string())
            })?;
        Ok(())
    }

    async fn handle_tx_submission_updates(
        &self,
        sig: Vec<u8>,
        address: String,
        data: Vec<u8>,
    ) -> RpcResult<()> {

        let is_valid = verify_client_key_middleware(sig, address.clone())
            .map_err(|e| jsonrpsee::core::Error::Custom(e.to_string()))?;

        if !is_valid {
            return Err(jsonrpsee::core::Error::Custom(
                "Invalid client verification".to_string(),
            ).into());
        }

        info!(
            "RPC: handle_tx_submission_updates called for address: {}",
            address
        );
        let mut server = self.swarm_server.lock().await;
        server
            .handle_tx_submission_updates(address, data)
            .await
            .map_err(|e| {
                error!("RPC: Failed to handle tx submission updates: {}", e);
                jsonrpsee::core::Error::Custom(e.to_string())
            })?;
        Ok(())
    }

    async fn fetch_pending_transactions(&self, sig: Vec<u8>, address: String) -> RpcResult<()> {
       
        let is_valid = verify_client_key_middleware(sig, address.clone())
        .map_err(|e| jsonrpsee::core::Error::Custom(e.to_string()))?;

        if !is_valid {
            return Err(jsonrpsee::core::Error::Custom(
                "Invalid client verification".to_string(),
            ).into());
        }

        info!(
            "RPC: fetch_pending_transactions called for address: {}",
            address
        );
        let server = self.swarm_server.lock().await;
        let transactions = server
            .fetch_pending_transactions(address.clone())
            .await
            .map_err(|e| {
                error!("RPC: Failed to fetch pending transactions: {}", e);
                jsonrpsee::core::Error::Custom(e.to_string())
            })?;

        let event = BackendEvent::PendingTransactionsFetched {
            address: address.clone(),
            transactions: transactions.clone(),
        };
        let _ = self.event_sender.send(event);

        Ok(())
    }

    async fn subscribe_to_events(
        &self,
        pending: PendingSubscriptionSink,
        address: String,
        sig: Vec<u8>,
    ) -> SubscriptionResult {

        let sink = pending.accept().await.map_err(|e| {
            error!(
                "RPC: Failed to accept subscription for address {}: {:?}",
                address, e
            );
            anyhow!("failed to accept subscription")
        })?;


        let is_valid = verify_client_key_middleware(sig, address.clone())
            .map_err(|e| jsonrpsee::core::Error::Custom(e.to_string()))?;

        if !is_valid {
            return Err(jsonrpsee::core::Error::Custom(
                "Invalid client verification".to_string(),
            ).into());
        }

        info!("RPC: subscribe_to_events called for address: {}", address);
     

        let server = self.swarm_server.lock().await;
        server.metrics.record_new_client_joined().await;
        drop(server);

        let mut receiver = self.event_sender.subscribe();
        info!("Subscription active for address: {}", address);

        loop {
            match receiver.recv().await {
                Ok(event) => {
                    let event_type = match &event {
                        BackendEvent::SenderRequestReceived { .. } => "SenderRequestReceived",
                        BackendEvent::SenderRequestHandled { .. } => "SenderRequestHandled",
                        BackendEvent::SenderConfirmed { .. } => "SenderConfirmed",
                        BackendEvent::SenderReverted { .. } => "SenderReverted",
                        // SenderConfirmed reused for tx submission updates too
                        BackendEvent::ReceiverResponseReceived { .. } => "ReceiverResponseReceived",
                        BackendEvent::ReceiverResponseHandled { .. } => "ReceiverResponseHandled",
                        BackendEvent::PeerDisconnected { .. } => "PeerDisconnected",
                        BackendEvent::DataExpired { .. } => "DataExpired",
                        BackendEvent::PendingTransactionsFetched { .. } => {
                            "PendingTransactionsFetched"
                        }
                        BackendEvent::TxSubmitted { .. } => "TxSubmitted",
                    };
                    let event_addr = event.get_address();
                    trace!("Subscription received event - type: {}, event_address: {}, subscription_address: {}", 
                   event_type, event_addr, address);

                    let should_send = match &event {
                        BackendEvent::SenderRequestReceived {
                            address: event_address,
                            data,
                        } => {
                            let matches_direct = event_address == &address;
                            let matches_tx = match serde_json::from_slice::<TxStateMachine>(data) {
                                Ok(tx) => {
                                    if matches!(tx.status, TxStatus::Reverted(_)) {
                                        trace!("Event SenderRequestReceived filtered - transaction is reverted");
                                        false
                                    } else {
                                        let matches = tx.sender_address == address
                                            || tx.receiver_address == address;
                                        if !matches {
                                            trace!("Event SenderRequestReceived filtered - subscription: {}, event_addr: {}, tx_sender: {}, tx_receiver: {}", 
                                          address, event_address, tx.sender_address, tx.receiver_address);
                                        } else {
                                            trace!("Event SenderRequestReceived matches - subscription: {}, event_addr: {}, tx_sender: {}, tx_receiver: {}", 
                                          address, event_address, tx.sender_address, tx.receiver_address);
                                        }
                                        matches
                                    }
                                }
                                Err(e) => {
                                    warn!("Failed to parse TxStateMachine in SenderRequestReceived for subscription {}: {}", address, e);
                                    false
                                }
                            };
                            matches_direct || matches_tx
                        }
                        BackendEvent::SenderRequestHandled {
                            address: event_address,
                            data,
                        } => {
                            let matches_direct = event_address == &address;
                            let matches_tx = match serde_json::from_slice::<TxStateMachine>(data) {
                                Ok(tx) => {
                                    if matches!(tx.status, TxStatus::Reverted(_)) {
                                        trace!("Event SenderRequestHandled filtered - transaction is reverted");
                                        false
                                    } else {
                                        let matches = tx.sender_address == address
                                            || tx.receiver_address == address;
                                        if !matches {
                                            trace!("Event SenderRequestHandled filtered - subscription: {}, event_addr: {}, tx_sender: {}, tx_receiver: {}", 
                                          address, event_address, tx.sender_address, tx.receiver_address);
                                        } else {
                                            trace!("Event SenderRequestHandled matches - subscription: {}, event_addr: {}, tx_sender: {}, tx_receiver: {}", 
                                          address, event_address, tx.sender_address, tx.receiver_address);
                                        }
                                        matches
                                    }
                                }
                                Err(e) => {
                                    warn!("Failed to parse TxStateMachine in SenderRequestHandled for subscription {}: {}", address, e);
                                    false
                                }
                            };
                            matches_direct || matches_tx
                        }
                        BackendEvent::SenderConfirmed {
                            address: event_address,
                            data,
                        } => {
                            let matches_direct = event_address == &address;
                            let matches_tx = match serde_json::from_slice::<TxStateMachine>(data) {
                                Ok(tx) => {
                                    if matches!(tx.status, TxStatus::Reverted(_)) {
                                        trace!("Event SenderConfirmed filtered - transaction is reverted");
                                        false
                                    } else {
                                        let matches = tx.sender_address == address
                                            || tx.receiver_address == address;
                                        if !matches {
                                            trace!("Event SenderConfirmed filtered - subscription: {}, event_addr: {}, tx_sender: {}, tx_receiver: {}", 
                                          address, event_address, tx.sender_address, tx.receiver_address);
                                        } else {
                                            trace!("Event SenderConfirmed matches - subscription: {}, event_addr: {}, tx_sender: {}, tx_receiver: {}", 
                                          address, event_address, tx.sender_address, tx.receiver_address);
                                        }
                                        matches
                                    }
                                }
                                Err(e) => {
                                    warn!("Failed to parse TxStateMachine in SenderConfirmed for subscription {}: {}", address, e);
                                    false
                                }
                            };
                            matches_direct || matches_tx
                        }
                        BackendEvent::SenderReverted {
                            address: event_address,
                            data,
                        } => {
                            let matches_direct = event_address == &address;
                            let matches_tx = match serde_json::from_slice::<TxStateMachine>(data) {
                                Ok(tx) => {
                                    if matches!(tx.status, TxStatus::Reverted(_)) {
                                        trace!("Event SenderReverted filtered - transaction is reverted");
                                        false
                                    } else {
                                        let matches = tx.sender_address == address
                                            || tx.receiver_address == address;
                                        if !matches {
                                            trace!("Event SenderReverted filtered - subscription: {}, event_addr: {}, tx_sender: {}, tx_receiver: {}", 
                                          address, event_address, tx.sender_address, tx.receiver_address);
                                        } else {
                                            trace!("Event SenderReverted matches - subscription: {}, event_addr: {}, tx_sender: {}, tx_receiver: {}", 
                                          address, event_address, tx.sender_address, tx.receiver_address);
                                        }
                                        matches
                                    }
                                }
                                Err(e) => {
                                    warn!("Failed to parse TxStateMachine in SenderReverted for subscription {}: {}", address, e);
                                    false
                                }
                            };
                            matches_direct || matches_tx
                        }
                        BackendEvent::ReceiverResponseReceived {
                            address: event_address,
                            data,
                        } => {
                            let matches_direct = event_address == &address;
                            let matches_tx = match serde_json::from_slice::<TxStateMachine>(data) {
                                Ok(tx) => {
                                    if matches!(tx.status, TxStatus::Reverted(_)) {
                                        trace!("Event ReceiverResponseReceived filtered - transaction is reverted");
                                        false
                                    } else {
                                        let matches = tx.sender_address == address
                                            || tx.receiver_address == address;
                                        if !matches {
                                            trace!("Event ReceiverResponseReceived filtered - subscription: {}, event_addr: {}, tx_sender: {}, tx_receiver: {}", 
                                          address, event_address, tx.sender_address, tx.receiver_address);
                                        } else {
                                            trace!("Event ReceiverResponseReceived matches - subscription: {}, event_addr: {}, tx_sender: {}, tx_receiver: {}", 
                                          address, event_address, tx.sender_address, tx.receiver_address);
                                        }
                                        matches
                                    }
                                }
                                Err(e) => {
                                    warn!("Failed to parse TxStateMachine in ReceiverResponseReceived for subscription {}: {}", address, e);
                                    false
                                }
                            };
                            matches_direct || matches_tx
                        }
                        BackendEvent::ReceiverResponseHandled {
                            address: event_address,
                            data,
                        } => {
                            let matches_direct = event_address == &address;
                            let matches_tx = match serde_json::from_slice::<TxStateMachine>(data) {
                                Ok(tx) => {
                                    if matches!(tx.status, TxStatus::Reverted(_)) {
                                        trace!("Event ReceiverResponseHandled filtered - transaction is reverted");
                                        false
                                    } else {
                                        let matches = tx.sender_address == address
                                            || tx.receiver_address == address;
                                        if !matches {
                                            trace!("Event ReceiverResponseHandled filtered - subscription: {}, event_addr: {}, tx_sender: {}, tx_receiver: {}", 
                                          address, event_address, tx.sender_address, tx.receiver_address);
                                        } else {
                                            trace!("Event ReceiverResponseHandled matches - subscription: {}, event_addr: {}, tx_sender: {}, tx_receiver: {}", 
                                          address, event_address, tx.sender_address, tx.receiver_address);
                                        }
                                        matches
                                    }
                                }
                                Err(e) => {
                                    warn!("Failed to parse TxStateMachine in ReceiverResponseHandled for subscription {}: {}", address, e);
                                    false
                                }
                            };
                            matches_direct || matches_tx
                        }
                        BackendEvent::PeerDisconnected { account_id } => account_id == &address,
                        BackendEvent::DataExpired { multi_id: _, data } => {
                            serde_json::from_slice::<TxStateMachine>(data)
                                .ok()
                                .map(|tx| {
                                    if matches!(tx.status, TxStatus::Reverted(_)) {
                                        false
                                    } else {
                                        tx.sender_address == address
                                            || tx.receiver_address == address
                                    }
                                })
                                .unwrap_or(false)
                        }
                        BackendEvent::PendingTransactionsFetched {
                            address: event_address,
                            ..
                        } => event_address == &address,
                        BackendEvent::TxSubmitted {
                            address: event_address,
                            data,
                        } => {
                            let matches_direct = event_address == &address;
                            let matches_tx = match serde_json::from_slice::<TxStateMachine>(data) {
                                Ok(tx) => {
                                    if matches!(tx.status, TxStatus::Reverted(_)) {
                                        trace!(
                                            "Event TxSubmitted filtered - transaction is reverted"
                                        );
                                        false
                                    } else {
                                        tx.sender_address == address
                                            || tx.receiver_address == address
                                    }
                                }
                                Err(e) => {
                                    warn!("Failed to parse TxStateMachine in TxSubmitted for subscription {}: {}", address, e);
                                    false
                                }
                            };
                            matches_direct || matches_tx
                        }
                    };

                    if should_send {
                        info!("Sending event to subscription for address: {}", address);
                        let subscription_msg =
                            SubscriptionMessage::from_json(&event).map_err(|e| {
                                error!("Failed to serialize event for subscription: {}", e);
                                anyhow!("failed to serialize event: {}", e)
                            })?;

                        if let Err(e) = sink.send(subscription_msg).await {
                            warn!(
                                "Failed to send event to subscription for address {}: {}",
                                address, e
                            );
                            break;
                        }
                        info!(
                            "Event successfully sent to subscription for address: {}",
                            address
                        );
                    } else {
                        trace!("Event filtered out - subscription: {}, event_type: {}, event_address: {}", 
                              address, event_type, event_addr);
                    }
                }
                Err(e) => {
                    error!("Subscription receiver error for address {}: {}", address, e);
                    info!(
                        "Subscription ended for address: {} (receiver error)",
                        address
                    );
                    break;
                }
            }
        }

        info!("Subscription ended for address: {}", address);
        Ok(())
    }

    async fn get_tx_counts(
        &self,
        sig: Vec<u8>,
        address: String,
    ) -> RpcResult<(usize, usize, usize, usize, usize)> {
        let is_valid = verify_client_key_middleware(sig, address.clone())
            .map_err(|e| jsonrpsee::core::Error::Custom(e.to_string()))?;

        if !is_valid {
            return Err(jsonrpsee::core::Error::Custom(
                "Invalid client verification".to_string(),
            ).into());
        }

        info!("RPC: get_tx_counts called for address: {}", address);
        let server = self.swarm_server.lock().await;
        let counts = server.metrics.get_tx_counts().await.map_err(|e| {
            error!("RPC: Failed to get tx counts: {}", e);
            jsonrpsee::core::Error::Custom(e.to_string())
        })?;
        Ok(counts)
    }

    async fn get_total_tx_count(
        &self,
        sig: Vec<u8>,
        address: String,
    ) -> RpcResult<usize> {
        let is_valid = verify_client_key_middleware(sig, address.clone())
            .map_err(|e| jsonrpsee::core::Error::Custom(e.to_string()))?;

        if !is_valid {
            return Err(jsonrpsee::core::Error::Custom(
                "Invalid client verification".to_string(),
            ).into());
        }

        info!("RPC: get_total_tx_count called for address: {}", address);
        let server = self.swarm_server.lock().await;
        let count = server.metrics.get_total_tx_count().await.map_err(|e| {
            error!("RPC: Failed to get total tx count: {}", e);
            jsonrpsee::core::Error::Custom(e.to_string())
        })?;
        Ok(count)
    }

    async fn get_receiver_confirmed_count(
        &self,
        sig: Vec<u8>,
        address: String,
    ) -> RpcResult<usize> {
        let is_valid = verify_client_key_middleware(sig, address.clone())
            .map_err(|e| jsonrpsee::core::Error::Custom(e.to_string()))?;

        if !is_valid {
            return Err(jsonrpsee::core::Error::Custom(
                "Invalid client verification".to_string(),
            ).into());
        }

        info!("RPC: get_receiver_confirmed_count called for address: {}", address);
        let server = self.swarm_server.lock().await;
        let count = server.metrics.get_receiver_confirmed_count().await.map_err(|e| {
            error!("RPC: Failed to get receiver confirmed count: {}", e);
            jsonrpsee::core::Error::Custom(e.to_string())
        })?;
        Ok(count)
    }

    async fn get_reverted_count(
        &self,
        sig: Vec<u8>,
        address: String,
    ) -> RpcResult<usize> {
        let is_valid = verify_client_key_middleware(sig, address.clone())
            .map_err(|e| jsonrpsee::core::Error::Custom(e.to_string()))?;

        if !is_valid {
            return Err(jsonrpsee::core::Error::Custom(
                "Invalid client verification".to_string(),
            ).into());
        }

        info!("RPC: get_reverted_count called for address: {}", address);
        let server = self.swarm_server.lock().await;
        let count = server.metrics.get_reverted_count().await.map_err(|e| {
            error!("RPC: Failed to get reverted count: {}", e);
            jsonrpsee::core::Error::Custom(e.to_string())
        })?;
        Ok(count)
    }

    async fn get_completed_count(
        &self,
        sig: Vec<u8>,
        address: String,
    ) -> RpcResult<usize> {
        let is_valid = verify_client_key_middleware(sig, address.clone())
            .map_err(|e| jsonrpsee::core::Error::Custom(e.to_string()))?;

        if !is_valid {
            return Err(jsonrpsee::core::Error::Custom(
                "Invalid client verification".to_string(),
            ).into());
        }

        info!("RPC: get_completed_count called for address: {}", address);
        let server = self.swarm_server.lock().await;
        let count = server.metrics.get_completed_count().await.map_err(|e| {
            error!("RPC: Failed to get completed count: {}", e);
            jsonrpsee::core::Error::Custom(e.to_string())
        })?;
        Ok(count)
    }

    async fn get_reverted_before_confirm_count(
        &self,
        sig: Vec<u8>,
        address: String,
    ) -> RpcResult<usize> {
        let is_valid = verify_client_key_middleware(sig, address.clone())
            .map_err(|e| jsonrpsee::core::Error::Custom(e.to_string()))?;

        if !is_valid {
            return Err(jsonrpsee::core::Error::Custom(
                "Invalid client verification".to_string(),
            ).into());
        }

        info!("RPC: get_reverted_before_confirm_count called for address: {}", address);
        let server = self.swarm_server.lock().await;
        let count = server.metrics.get_reverted_before_confirm_count().await.map_err(|e| {
            error!("RPC: Failed to get reverted before confirm count: {}", e);
            jsonrpsee::core::Error::Custom(e.to_string())
        })?;
        Ok(count)
    }
}

pub struct JsonRpcServer {
    rpc_handler: BackendRpcHandler,
    url: String,
}

impl JsonRpcServer {
    pub fn new(
        swarm_server: Arc<Mutex<VaneSwarmServer>>,
        event_sender: broadcast::Sender<BackendEvent>,
        port: u16,
    ) -> Result<Self> {
        let rpc_handler = BackendRpcHandler::new(swarm_server, event_sender);
        let url = format!("127.0.0.1:{}", port);
        Ok(Self { rpc_handler, url })
    }

    pub async fn start(&self) -> Result<SocketAddr> {
        let server_builder = ServerBuilder::new();
        let server = server_builder.build(&self.url).await?;
        let address = server
            .local_addr()
            .map_err(|err| anyhow!("failed to get address: {}", err))?;

        let handler = self.rpc_handler.clone().into_rpc();

        let handle = server
            .start(handler)
            .map_err(|err| anyhow!("rpc handler error: {}", err))?;

        info!("Backend JSON-RPC server listening on ws://{}", address);

        handle.stopped().await;
        Ok(address)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BackendEventsSummary {
    sender_requests_total: u64,
    receiver_responses_total: u64,
    active_peers: usize,
    pending_requests: usize,
    new_clients_joined: u64,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ClientMetricsPayload {
    pub peer_id: String,
    pub client_type: String,
    pub timestamp: u64,
    pub storage_export: StorageExport,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ClientSnapshot {
    pub account_id: String,
    pub success_txs: u64,
    pub failed_txs: u64,
    pub failed_transactions: Vec<DbTxStateMachine>,
}

pub struct ClientMetricsStore {
    pub last_update_times: HashMap<String, SystemTime>,
    pub last_totals: HashMap<String, (u64, u64)>,
    pub total_success_txs: Counter,
    pub total_failed_txs: Counter,
    pub tx_success_rate_percent: Gauge,
    pub per_client: HashMap<String, ClientSnapshot>,
}

pub struct BackendMetrics {
    pub sender_requests_total: Counter,
    pub receiver_responses_total: Counter,
    pub peers_added_total: Counter,
    pub peers_removed_total: Counter,
    pub active_peers: Gauge,
    pub pending_requests: Gauge,
    pub new_clients_joined: Counter,
    // Track last written values for delta calculation
    pub last_written_sender_requests: u64,
    pub last_written_receiver_responses: u64,
    pub last_written_peers_added: u64,
    pub last_written_peers_removed: u64,
    pub last_written_new_clients_joined: u64,
}

impl BackendMetrics {
    pub fn new() -> Self {
        Self {
            sender_requests_total: Counter::default(),
            receiver_responses_total: Counter::default(),
            peers_added_total: Counter::default(),
            peers_removed_total: Counter::default(),
            active_peers: Gauge::default(),
            pending_requests: Gauge::default(),
            new_clients_joined: Counter::default(),
            last_written_sender_requests: 0,
            last_written_receiver_responses: 0,
            last_written_peers_added: 0,
            last_written_peers_removed: 0,
            last_written_new_clients_joined: 0,
        }
    }

    pub fn register(&self, registry: &mut Registry) {
        registry.register(
            "backend_sender_requests_total",
            "Total sender requests received",
            self.sender_requests_total.clone(),
        );
        registry.register(
            "backend_receiver_responses_total",
            "Total receiver responses received",
            self.receiver_responses_total.clone(),
        );
        registry.register(
            "backend_peers_added_total",
            "Total peers added",
            self.peers_added_total.clone(),
        );
        registry.register(
            "backend_peers_removed_total",
            "Total peers removed",
            self.peers_removed_total.clone(),
        );
        registry.register(
            "backend_active_peers",
            "Current number of active peers",
            self.active_peers.clone(),
        );
        registry.register(
            "backend_pending_requests",
            "Current number of pending requests",
            self.pending_requests.clone(),
        );
        registry.register(
            "backend_new_clients_joined",
            "Total number of new clients that have joined",
            self.new_clients_joined.clone(),
        );
    }
}

impl ClientMetricsStore {
    pub fn new() -> Self {
        Self {
            last_update_times: HashMap::new(),
            last_totals: HashMap::new(),
            total_success_txs: Counter::default(),
            total_failed_txs: Counter::default(),
            tx_success_rate_percent: Gauge::default(),
            per_client: HashMap::new(),
        }
    }

    pub fn register(&self, registry: &mut Registry) {
        registry.register(
            "backend_total_success_txs",
            "Total successful transactions",
            self.total_success_txs.clone(),
        );
        registry.register(
            "backend_total_failed_txs",
            "Total failed transactions",
            self.total_failed_txs.clone(),
        );
        registry.register(
            "backend_tx_success_rate_percent",
            "Transaction success rate (percent)",
            self.tx_success_rate_percent.clone(),
        );
    }

    pub fn update_from_exported_storage(&mut self, payload: ClientMetricsPayload) {
        let peer_id = payload.peer_id.clone();
        let client_type = payload.client_type.clone();
        let storage = &payload.storage_export;

        let account_id = storage
            .user_account
            .as_ref()
            .and_then(|ua| ua.accounts.first())
            .map(|(acc, _)| acc.clone())
            .unwrap_or_else(|| "unknown".to_string());

        self.last_update_times
            .insert(account_id.clone(), SystemTime::now());

        self.update_aggregated_metrics(&account_id, storage);

        info!(
            "Updated backend metrics for peer {} account {} ({}) - {} success txs, {} failed txs",
            peer_id,
            account_id,
            client_type,
            storage.success_transactions.len(),
            storage.failed_transactions.len()
        );
    }

    fn update_aggregated_metrics(&mut self, account_id: &str, storage: &StorageExport) {
        let current = (
            storage.success_transactions.len() as u64,
            storage.failed_transactions.len() as u64,
        );

        let last = self.last_totals.get(account_id).cloned().unwrap_or((0, 0));
        let delta_success = current.0.saturating_sub(last.0);
        let delta_failed = current.1.saturating_sub(last.1);

        if delta_success > 0 {
            self.total_success_txs.inc_by(delta_success);
        }
        if delta_failed > 0 {
            self.total_failed_txs.inc_by(delta_failed);
        }

        let total_tx = current.0 + current.1;
        if total_tx > 0 {
            let success_rate = (current.0 as f64 / total_tx as f64) * 100.0;
            self.tx_success_rate_percent
                .set(success_rate.round() as i64);
        }

        self.last_totals.insert(account_id.to_string(), current);

        let existing_snapshot = self
            .per_client
            .entry(account_id.to_string())
            .or_insert_with(|| ClientSnapshot {
                account_id: account_id.to_string(),
                success_txs: 0,
                failed_txs: 0,
                failed_transactions: Vec::new(),
            });

        existing_snapshot.success_txs = current.0;
        existing_snapshot.failed_txs = current.1;

        for new_tx in &storage.failed_transactions {
            let is_duplicate = existing_snapshot
                .failed_transactions
                .iter()
                .any(|existing_tx| existing_tx.tx_hash == new_tx.tx_hash);

            if !is_duplicate {
                existing_snapshot.failed_transactions.push(new_tx.clone());
            }
        }
    }
}

#[derive(Clone)]
pub struct MetricService {
    backend_metrics: Arc<Mutex<BackendMetrics>>,
    client_metrics: Arc<Mutex<ClientMetricsStore>>,
    db: Option<Arc<crate::db::D1Client>>,
}

impl MetricService {
    pub fn new(db: Option<Arc<crate::db::D1Client>>) -> Self {
        let mut registry = Registry::default();
        let backend_metrics = BackendMetrics::new();
        let client_metrics = ClientMetricsStore::new();

        backend_metrics.register(&mut registry);
        client_metrics.register(&mut registry);

        let service = Self {
            backend_metrics: Arc::new(Mutex::new(backend_metrics)),
            client_metrics: Arc::new(Mutex::new(client_metrics)),
            db: db.clone(),
        };

        // Start periodic DB write task (every 10 minutes)
        if let Some(db) = db {
            let service_clone = service.clone();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(600)); // 10 minutes
                loop {
                    interval.tick().await;
                    if let Err(e) = service_clone.write_metrics_to_db().await {
                        error!("Failed to write metrics to DB: {}", e);
                    }
                }
            });
        }

        service
    }

    // Write metrics to DB periodically (increment counters)
    async fn write_metrics_to_db(&self) -> Result<()> {
        if let Some(db) = &self.db {
            let mut metrics = self.backend_metrics.lock().await;
            
            // Get current in-memory values
            let sender_requests = metrics.sender_requests_total.get();
            let receiver_responses = metrics.receiver_responses_total.get();
            let peers_added = metrics.peers_added_total.get();
            let peers_removed = metrics.peers_removed_total.get();
            let new_clients_joined = metrics.new_clients_joined.get();

            // Get last written values (if tracking exists)
            let last_sender = metrics.last_written_sender_requests;
            let last_receiver = metrics.last_written_receiver_responses;
            let last_peers_added = metrics.last_written_peers_added;
            let last_peers_removed = metrics.last_written_peers_removed;
            let last_clients = metrics.last_written_new_clients_joined;

            // Calculate deltas
            let delta_sender = sender_requests.saturating_sub(last_sender);
            let delta_receiver = receiver_responses.saturating_sub(last_receiver);
            let delta_peers_added = peers_added.saturating_sub(last_peers_added);
            let delta_peers_removed = peers_removed.saturating_sub(last_peers_removed);
            let delta_clients = new_clients_joined.saturating_sub(last_clients);

            // Write deltas to DB (increment counters)
            for _ in 0..delta_sender {
                if let Err(e) = db.increment_sender_requests().await {
                    error!("Failed to increment sender_requests in DB: {}", e);
                }
            }
            for _ in 0..delta_receiver {
                if let Err(e) = db.increment_receiver_responses().await {
                    error!("Failed to increment receiver_responses in DB: {}", e);
                }
            }
            for _ in 0..delta_peers_added {
                if let Err(e) = db.increment_peers_added().await {
                    error!("Failed to increment peers_added in DB: {}", e);
                }
            }
            for _ in 0..delta_peers_removed {
                if let Err(e) = db.increment_peers_removed().await {
                    error!("Failed to increment peers_removed in DB: {}", e);
                }
            }
            for _ in 0..delta_clients {
                if let Err(e) = db.increment_new_clients_joined().await {
                    error!("Failed to increment new_clients_joined in DB: {}", e);
                }
            }

            // Update last written values
            metrics.last_written_sender_requests = sender_requests;
            metrics.last_written_receiver_responses = receiver_responses;
            metrics.last_written_peers_added = peers_added;
            metrics.last_written_peers_removed = peers_removed;
            metrics.last_written_new_clients_joined = new_clients_joined;

            info!("Wrote metrics to DB - sender: {}, receiver: {}, peers_added: {}, peers_removed: {}, clients: {}",
                  delta_sender, delta_receiver, delta_peers_added, delta_peers_removed, delta_clients);
        }
        Ok(())
    }

    pub fn get_client_metrics(&self) -> Arc<Mutex<ClientMetricsStore>> {
        Arc::clone(&self.client_metrics)
    }

    pub fn get_backend_metrics(&self) -> Arc<Mutex<BackendMetrics>> {
        Arc::clone(&self.backend_metrics)
    }

    pub async fn record_sender_request(&self, _address: &str) {
        let metrics = self.backend_metrics.lock().await;
        metrics.sender_requests_total.inc();
    }

    pub async fn record_receiver_response(&self, _address: &str) {
        let metrics = self.backend_metrics.lock().await;
        metrics.receiver_responses_total.inc();
    }

    pub async fn record_peer_added(&self) {
        let metrics = self.backend_metrics.lock().await;
        metrics.peers_added_total.inc();
        metrics.active_peers.inc();
    }

    pub async fn record_peer_removed(&self) {
        let metrics = self.backend_metrics.lock().await;
        metrics.peers_removed_total.inc();
        metrics.active_peers.dec();
    }

    pub async fn update_active_peers(&self, count: i64) {
        let metrics = self.backend_metrics.lock().await;
        metrics.active_peers.set(count);
    }

    pub async fn update_pending_requests(&self, count: i64) {
        let metrics = self.backend_metrics.lock().await;
        metrics.pending_requests.set(count);
    }

    pub async fn record_new_client_joined(&self) {
        let metrics = self.backend_metrics.lock().await;
        metrics.new_clients_joined.inc();
    }

    pub fn get_db(&self) -> Option<Arc<crate::db::D1Client>> {
        self.db.clone()
    }

    /// Get transaction counts from database
    /// Returns: (total, receiver_confirmed, reverted, completed, reverted_before_confirm)
    pub async fn get_tx_counts(&self) -> Result<(usize, usize, usize, usize, usize), anyhow::Error> {
        let db = self.db.as_ref()
            .ok_or_else(|| anyhow!("Database not available"))?;
        
        let all_txs = db.get_tx_lifecycle_filtered(None, None, None).await.unwrap_or_default();
        let receiver_confirmed = db.get_tx_lifecycle_filtered(None, None, Some(true)).await.unwrap_or_default();
        let reverted = db.get_tx_lifecycle_filtered(None, Some(true), None).await.unwrap_or_default();
        let completed = db.get_tx_lifecycle_filtered(Some(true), None, None).await.unwrap_or_default();
        let reverted_before_confirm = db.get_tx_lifecycle_filtered(None, Some(true), Some(false)).await.unwrap_or_default();
        
        Ok((
            all_txs.len(),
            receiver_confirmed.len(),
            reverted.len(),
            completed.len(),
            reverted_before_confirm.len(),
        ))
    }

    /// Get total transaction count
    pub async fn get_total_tx_count(&self) -> Result<usize, anyhow::Error> {
        let db = self.db.as_ref()
            .ok_or_else(|| anyhow!("Database not available"))?;
        
        let txs = db.get_tx_lifecycle_filtered(None, None, None).await.unwrap_or_default();
        Ok(txs.len())
    }

    /// Get receiver confirmed transaction count
    pub async fn get_receiver_confirmed_count(&self) -> Result<usize, anyhow::Error> {
        let db = self.db.as_ref()
            .ok_or_else(|| anyhow!("Database not available"))?;
        
        let txs = db.get_tx_lifecycle_filtered(None, None, Some(true)).await.unwrap_or_default();
        Ok(txs.len())
    }

    /// Get reverted transaction count
    pub async fn get_reverted_count(&self) -> Result<usize, anyhow::Error> {
        let db = self.db.as_ref()
            .ok_or_else(|| anyhow!("Database not available"))?;
        
        let txs = db.get_tx_lifecycle_filtered(None, Some(true), None).await.unwrap_or_default();
        Ok(txs.len())
    }

    /// Get completed transaction count
    pub async fn get_completed_count(&self) -> Result<usize, anyhow::Error> {
        let db = self.db.as_ref()
            .ok_or_else(|| anyhow!("Database not available"))?;
        
        let txs = db.get_tx_lifecycle_filtered(Some(true), None, None).await.unwrap_or_default();
        Ok(txs.len())
    }

    /// Get reverted before confirmation transaction count
    pub async fn get_reverted_before_confirm_count(&self) -> Result<usize, anyhow::Error> {
        let db = self.db.as_ref()
            .ok_or_else(|| anyhow!("Database not available"))?;
        
        let txs = db.get_tx_lifecycle_filtered(None, Some(true), Some(false)).await.unwrap_or_default();
        Ok(txs.len())
    }
}

