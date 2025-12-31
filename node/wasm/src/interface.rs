extern crate alloc;

use alloc::{
    rc::Rc,
    string::{String, ToString},
    vec::Vec,
};
use codec::Encode;
use core::{cell::RefCell, fmt, str::FromStr};

use anyhow::anyhow;
use async_stream::stream;
use db_wasm::{DbWorker, OpfsRedbWorker};
use futures::StreamExt;
use gloo_timers::future::TimeoutFuture;
use log::{debug, error, info, trace};
use lru::LruCache;
use reqwasm::http::{Request, RequestMode};
use serde_wasm_bindgen;
use sp_core::blake2_256;
use sp_runtime::{format, traits::Zero};
use tokio_with_wasm::alias::sync::{
    mpsc::{Receiver, Sender},
    {Mutex, MutexGuard},
};
use wasm_bindgen::prelude::*;

use crate::{
    cryptography::{verify_public_bytes, verify_route},
    p2p::{P2pEventNotifSubSystem, P2pNetworkService, WasmP2pWorker},
    HashMap,
};

use primitives::data_structure::{
    AccountInfo, BackendEvent, ChainSupported, ChainTransactionType, DbTxStateMachine,
    DbWorkerInterface, NetworkCommand, SignatureType, StorageExport, Token, TtlWrapper,
    TxStateMachine, TxStateMachineLike, TxStatus, UserAccount, UserMetrics, VanePayload,
};

#[derive(Clone)]
pub struct PublicInterfaceWorker {
    /// local database worker
    pub db_worker: Rc<DbWorker>,
    // p2pworker
    pub p2p_worker: Rc<WasmP2pWorker>,
    /// p2p network service
    pub p2p_network_service: Rc<P2pNetworkService>,
    /// receiving end of transaction which will be polled in websocket , updating state of tx to end user
    pub rpc_receiver_channel: Rc<RefCell<Receiver<TxStateMachine>>>,
    /// sender channel when user updates the transaction state, propagating to main service worker
    pub user_rpc_update_sender_channel:
        Rc<RefCell<Sender<VanePayload<TxStateMachine, SignatureType>>>>,
    // txn_counter
    // HashMap<txn_counter,Integrity hash>
    pub tx_integrity: Rc<RefCell<HashMap<u32, [u8; 32]>>>,
    //// tx pending store
    pub lru_cache: Rc<RefCell<LruCache<u32, TtlWrapper<TxStateMachine>>>>, // initial fees, after dry running tx initialy without optimization
    /// Flag to track if a watcher is already active to prevent multiple concurrent watchers
    pub watcher_active: Rc<RefCell<bool>>,
    /// List of registered callbacks for transaction updates
    pub tx_callbacks: Rc<RefCell<Vec<js_sys::Function>>>,
    /// p2p event notification worker for notifying user about the full connection cycle
    pub p2p_event_notif_sub_system: Rc<P2pEventNotifSubSystem>,
    /// Flag to track if p2p watcher is already active
    pub p2p_watcher_active: Rc<RefCell<bool>>,
    /// List of registered callbacks for p2p event updates
    pub p2p_callbacks: Rc<RefCell<Vec<js_sys::Function>>>,
}

impl PublicInterfaceWorker {
    pub async fn new(
        db_worker: Rc<DbWorker>,
        p2p_event_notif_sub_system: Rc<P2pEventNotifSubSystem>,
        p2p_worker: Rc<WasmP2pWorker>,
        p2p_network_service: Rc<P2pNetworkService>,
        rpc_recv_channel: Rc<RefCell<Receiver<TxStateMachine>>>,
        user_rpc_update_sender_channel: Rc<
            RefCell<Sender<VanePayload<TxStateMachine, SignatureType>>>,
        >,
        lru_cache: Rc<RefCell<LruCache<u32, TtlWrapper<TxStateMachine>>>>,
    ) -> Result<Self, JsValue> {
        Ok(Self {
            db_worker,
            p2p_worker,
            p2p_network_service,
            rpc_receiver_channel: rpc_recv_channel,
            user_rpc_update_sender_channel,
            lru_cache,
            tx_integrity: Rc::new(RefCell::new(HashMap::new())),
            watcher_active: Rc::new(RefCell::new(false)),
            tx_callbacks: Rc::new(RefCell::new(Vec::new())),
            p2p_event_notif_sub_system,
            p2p_watcher_active: Rc::new(RefCell::new(false)),
            p2p_callbacks: Rc::new(RefCell::new(Vec::new())),
        })
    }

    pub fn compute_tx_integrity_hash(tx: &TxStateMachine) -> [u8; 32] {
        let data_to_hash = (
            tx.sender_address.clone(),
            tx.receiver_address.clone(),
            tx.sender_address_network.clone(),
            tx.receiver_address_network.clone(),
            tx.multi_id.clone(),
            tx.token.clone(),
            tx.amount.clone(),
        )
            .encode();

        let integrity_hash = blake2_256(&data_to_hash);
        integrity_hash
    }

    // TODO: see how we can deserialize bytes data in solana and in EVM so that we can hash the params
    pub fn compute_tx_call_payload_integrity_hash(
        tx: &TxStateMachine,
    ) -> Result<[u8; 32], JsError> {
        // check for each and extract the data from call_payload
        let integrity_hash = match &tx.call_payload {
            Some(ChainTransactionType::Solana {
                call_payload: _,
                latest_block_height: _,
            }) => {
                let data_to_hash = (
                    tx.sender_address.clone(),
                    tx.receiver_address.clone(),
                    tx.sender_address_network.clone(),
                    tx.receiver_address_network.clone(),
                    tx.multi_id.clone(),
                    tx.token.clone(),
                    tx.amount.clone(),
                )
                    .encode();
                let integrity_hash = blake2_256(&data_to_hash);
                integrity_hash
            }
            Some(ChainTransactionType::Ethereum {
                eth_unsigned_tx_fields,
                call_payload: _,
            }) => {
                let data_to_hash = (
                    tx.sender_address.clone(),
                    tx.receiver_address.clone(),
                    tx.sender_address_network.clone(),
                    tx.receiver_address_network.clone(),
                    tx.multi_id.clone(),
                    tx.token.clone(),
                    tx.amount.clone(),
                )
                    .encode();
                let integrity_hash = blake2_256(&data_to_hash);
                integrity_hash
            }
            Some(ChainTransactionType::Bnb {
                call_payload: _,
                bnb_legacy_tx_fields,
            }) => {
                let data_to_hash = (
                    tx.sender_address.clone(),
                    tx.receiver_address.clone(),
                    tx.sender_address_network.clone(),
                    tx.receiver_address_network.clone(),
                    tx.multi_id.clone(),
                    tx.token.clone(),
                    tx.amount.clone(),
                )
                    .encode();
                let integrity_hash = blake2_256(&data_to_hash);
                integrity_hash
            }
            None => {
                return Err(JsError::new("Call payload is not available"));
            }
        };

        Ok(integrity_hash)
    }

    pub async fn initiate_transaction(
        &self,
        sig: Vec<u8>,
        sender: String,
        receiver: String,
        amount: u128,
        token: Token,
        code_word: String,
        sender_network: ChainSupported,
        receiver_network: ChainSupported,
    ) -> Result<JsValue, JsError> {
        info!("initiated sending transaction: receiver: {}, amount: {}, token: {:?}, sender_network: {:?}, receiver_network: {:?}, code_word: {}", receiver, amount, token, sender_network, receiver_network, code_word);

        {
            let _sender_network_verified =
                verify_public_bytes(sender.as_str(), &token, sender_network)
                    .map_err(|e| JsError::new(&format!("{:?}", e)))?;
            let _receiver_network_verified =
                verify_public_bytes(receiver.as_str(), &token, receiver_network)
                    .map_err(|e| JsError::new(&format!("{:?}", e)))?;
            // add if the route is supported
            if let Err(e) = verify_route(sender_network.clone(), receiver_network.clone()) {
                return Err(JsError::new(&format!("{:?}", e)));
            }
        }

        info!("successfully initially verified sender and receiver and related network bytes");
        // construct the tx
        let mut sender_recv = sender.as_bytes().to_vec();
        sender_recv.extend_from_slice(receiver.as_bytes());
        let multi_addr = blake2_256(&sender_recv[..]);

        // failure to this should stop the node and not continue
        let nonce = self
            .db_worker
            .get_nonce()
            .await
            .map_err(|e| JsError::new(&format!("{:?}", e)))?
            + 1;
        // update the db on nonce
        // failure to this should stop the node and not continue
        self.db_worker
            .increment_nonce()
            .await
            .map_err(|e| JsError::new(&format!("{:?}", e)))?;

        let tx_state_machine = TxStateMachine {
            sender_address: sender,
            sender_public_key: None,
            receiver_public_key: None,
            receiver_address: receiver,
            multi_id: multi_addr,
            recv_signature: None,
            status: TxStatus::default(),
            amount,
            fees_amount: 0.0,
            signed_call_payload: None,
            call_payload: None,
            inbound_req_id: None,
            outbound_req_id: None,
            tx_nonce: nonce,
            tx_version: 0,
            token,
            code_word,
            sender_address_network: sender_network,
            receiver_address_network: receiver_network,
        };

        // propagate the tx to lower layer (Main service worker layer)
        let sender_channel = self.user_rpc_update_sender_channel.borrow_mut();

        let tx_integrity_hash = Self::compute_tx_integrity_hash(&tx_state_machine);
        self.tx_integrity
            .borrow_mut()
            .insert(tx_state_machine.tx_nonce.into(), tx_integrity_hash);

        let vane_payload = VanePayload::new(tx_state_machine.clone(), sig);

        sender_channel
            .send(vane_payload)
            .await
            .map_err(|_| anyhow!("failed to send initial tx state to sender channel"))
            .map_err(|e| JsError::new(&format!("{:?}", e)))?;

        let now = (js_sys::Date::now() / 1000.0) as u32;
        self.lru_cache.borrow_mut().push(
            tx_state_machine.tx_nonce.into(),
            TtlWrapper::new(tx_state_machine.clone(), now),
        );

        info!("propagated initiated transaction to tx handling layer");
        // Return the constructed tx to JS so callers can keep a handle
        return serde_wasm_bindgen::to_value(&tx_state_machine)
            .map_err(|e| JsError::new(&format!("Serialization error: {:?}", e)));
    }

    pub async fn sender_confirm(&self, sig: Vec<u8>, tx: JsValue) -> Result<(), JsError> {
        let mut tx: TxStateMachine = TxStateMachine::from_js_value_unconditional(tx)?;
        info!("sender_confirming transaction: {:?}", tx);
        let tx_integrity_hash = Self::compute_tx_integrity_hash(&tx);

        let recv_tx_integrity = self
            .tx_integrity
            .borrow_mut()
            .get(&tx.tx_nonce)
            .ok_or(JsError::new(&format!(
                " Failed get transaction integrity from cache"
            )))?
            .clone();
        if tx_integrity_hash != recv_tx_integrity {
            error!("Transaction integrity failed");
            Err(anyhow!("Transaction integrity failed".to_string()))
                .map_err(|e| JsError::new(&format!("{:?}", e)))?
        }

        let sender_channel = self.user_rpc_update_sender_channel.borrow_mut();
        // Guard: ignore if already reverted
        if let TxStatus::Reverted(_) = tx.status {
            error!("Transaction already reverted");
            return Err(JsError::new("Transaction already reverted"));
        }

        if let TxStatus::ReceiverNotRegistered = tx.status {
            error!("Receiver not registered");
            return Err(JsError::new("Receiver not registered"));
        }

        if let TxStatus::RecvAddrFailed = tx.status {
            error!("Receiver address failed");
            return Err(JsError::new("Receiver address failed"));
        }

        if tx.signed_call_payload.is_none() && tx.status != TxStatus::RecvAddrConfirmationPassed {
            // return error as receiver hasnt confirmed yet or sender hasnt confirmed on his turn
            error!("Wait for Receiver to confirm or sender should confirm");
            Err(anyhow!(
                "Wait for Receiver to confirm or sender should confirm".to_string(),
            ))
            .map_err(|e| JsError::new(&format!("{:?}", e)))?;
        } else {
            if !matches!(tx.status, TxStatus::TxSubmissionPassed { hash: _ })
                && !matches!(tx.status, TxStatus::FailedToSubmitTxn(_))
            {
                tx.sender_confirmation();
            }

            // this path meaning we have already submitted the transaction
            tx.increment_version();

            let mut ttl_wrapper = self
                .lru_cache
                .borrow_mut()
                .get(&tx.tx_nonce.into())
                .ok_or(JsError::new(&format!(
                    " Failed get transaction from cache, transaction expired"
                )))?
                .clone();

            ttl_wrapper.update_value(tx.clone());
            let _ = self
                .lru_cache
                .borrow_mut()
                .push(tx.tx_nonce.into(), ttl_wrapper);

            let sender = sender_channel.clone();
            let vane_payload = VanePayload::new(tx.clone(), sig);

            sender
                .send(vane_payload)
                .await
                .map_err(|_| {
                    anyhow!("failed to send sender confirmation tx state to sender-channel")
                })
                .map_err(|e| JsError::new(&format!("{:?}", e)))?;
        }
        Ok(())
    }

    /// watch notifications coming from p2p communication to notify the user about the transaction updates
    pub async fn watch_p2p_notifications(
        &self,
        callback: &js_sys::Function,
    ) -> Result<(), JsError> {
        let callback = callback.clone();

        // Add the callback to our list
        self.p2p_callbacks.borrow_mut().push(callback);

        // Start the background watcher if it's not already running
        if !*self.p2p_watcher_active.borrow() {
            self.start_p2p_notifications_watcher().await?;
        }

        Ok(())
    }
    /// Register a callback for transaction updates. Multiple callbacks can be registered.
    /// The background watcher will be started automatically if it's not already running.
    pub async fn watch_tx_updates(&self, callback: &js_sys::Function) -> Result<(), JsError> {
        let callback = callback.clone();

        // Add the callback to our list
        self.tx_callbacks.borrow_mut().push(callback);

        // Start the background watcher if it's not already running
        if !*self.watcher_active.borrow() {
            self.start_background_watcher().await?;
        }

        Ok(())
    }

    /// Start the background watcher task (internal method)
    async fn start_background_watcher(&self) -> Result<(), JsError> {
        if *self.watcher_active.borrow() {
            return Ok(()); // Already running
        }

        // Mark watcher as active
        *self.watcher_active.borrow_mut() = true;

        let receiver_channel = self.rpc_receiver_channel.clone();
        let callbacks = self.tx_callbacks.clone();
        let watcher_active = self.watcher_active.clone();
        let tx_integrity_store = self.tx_integrity.clone();

        // Spawn a single background task to continuously poll for transaction updates
        wasm_bindgen_futures::spawn_local(async move {
            let mut receiver = receiver_channel.borrow_mut();

            loop {
                match receiver.recv().await {
                    Some(tx_update) => {
                        debug!("watch_tx_updates: {:?}", tx_update);
                        // save in tx_integrity
                        let tx_integrity_hash = Self::compute_tx_integrity_hash(&tx_update);
                        tx_integrity_store
                            .borrow_mut()
                            .insert(tx_update.tx_nonce.into(), tx_integrity_hash);
                        // Convert transaction to JS value
                        let tx_js_value = serde_wasm_bindgen::to_value(&tx_update)
                            .unwrap_or_else(|_| JsValue::NULL);

                        // Call all registered callbacks
                        let callbacks_guard = callbacks.borrow();
                        for callback in callbacks_guard.iter() {
                            let _ = callback.call1(&wasm_bindgen::JsValue::UNDEFINED, &tx_js_value);
                        }
                    }
                    None => {
                        info!("Transaction update channel closed");
                        // Mark watcher as inactive when done
                        *watcher_active.borrow_mut() = false;
                        break;
                    }
                }
            }
        });

        Ok(())
    }

    /// Start the p2p notifications background watcher task (internal method)
    async fn start_p2p_notifications_watcher(&self) -> Result<(), JsError> {
        if *self.p2p_watcher_active.borrow() {
            return Ok(()); // Already running
        }

        // Mark watcher as active
        *self.p2p_watcher_active.borrow_mut() = true;
        info!("Starting P2P notifications watcher");

        let receiver_channel = self.p2p_event_notif_sub_system.recv.clone();
        let callbacks = self.p2p_callbacks.clone();
        let watcher_active = self.p2p_watcher_active.clone();

        // Spawn a single background task to continuously poll for p2p event updates
        wasm_bindgen_futures::spawn_local(async move {
            info!("P2P notifications watcher task started");
            let mut receiver = receiver_channel.borrow_mut();

            loop {
                match receiver.recv().await {
                    Some(event) => {
                        // Convert p2p event to JS value using serde
                        let event_js_value =
                            serde_wasm_bindgen::to_value(&event).unwrap_or_else(|e| {
                                error!("Failed to serialize P2P event: {:?}", e);
                                JsValue::NULL
                            });

                        // Call all registered callbacks
                        let callbacks_guard = callbacks.borrow();
                        for callback in callbacks_guard.iter() {
                            if let Err(e) =
                                callback.call1(&wasm_bindgen::JsValue::UNDEFINED, &event_js_value)
                            {
                                error!("Error calling P2P notification callback: {:?}", e);
                            }
                        }
                    }
                    None => {
                        info!("P2P notification channel closed");
                        // Mark watcher as inactive when done
                        *watcher_active.borrow_mut() = false;
                        break;
                    }
                }
            }
        });

        Ok(())
    }

    /// Unsubscribe from all transaction updates (stops watcher and clears all callbacks)
    pub fn unsubscribe_watch_tx_updates(&self) -> Result<(), JsError> {
        *self.watcher_active.borrow_mut() = false;
        self.tx_callbacks.borrow_mut().clear();
        info!("Unsubscribed from all transaction updates");
        Ok(())
    }

    /// Unsubscribe from all p2p notifications (stops watcher and clears all callbacks)
    pub fn unsubscribe_watch_p2p_notifications(&self) -> Result<(), JsError> {
        *self.p2p_watcher_active.borrow_mut() = false;
        self.p2p_callbacks.borrow_mut().clear();
        info!("Unsubscribed from all p2p notifications");
        Ok(())
    }

    pub async fn fetch_pending_tx_updates(&self, sig: Vec<u8>) -> Result<JsValue, JsError> {
        // call fetch_pending_tx_updates from backend
        let account_id = self.p2p_worker.user_account_id.clone();
        let command_tx = self.p2p_network_service.p2p_command_tx.clone();
        command_tx
            .send(NetworkCommand::FetchPendingTransactions { sig, account_id })
            .await?;

        TimeoutFuture::new(2000).await;

        // flush out valyues that have been expired
        let now = (js_sys::Date::now() / 1000.0) as u32;
        info!("🔑 NOW: {:?}", now);
        let time_to_live = 60 * 40; // 40 minutes
        let expired_keys = {
            let cache = self.lru_cache.borrow();
            cache
                .iter()
                .filter_map(|(k, v)| {
                    info!("🔑 TTL: {:?}", v.ttl);
                    if v.is_expired(now, time_to_live) {
                        Some(*k)
                    } else {
                        None
                    }
                })
                .collect::<Vec<u32>>()
        };

        info!("🔑 EXPIRED KEYS: {:?}", expired_keys);

        {
            let mut cache = self.lru_cache.borrow_mut();
            for key in expired_keys {
                cache.pop(&key);
            }
        }

        let tx_updates = {
            let cache = self.lru_cache.borrow();
            let mut tx_integrity = self.tx_integrity.borrow_mut();
            cache
                .iter()
                .map(|(_k, v)| {
                    let tx_integrity_hash = Self::compute_tx_integrity_hash(v.get_value());
                    tx_integrity.insert(v.get_value().tx_nonce.into(), tx_integrity_hash);
                    v.get_value().clone()
                })
                .collect::<Vec<TxStateMachine>>()
        };
        debug!("lru: {tx_updates:#?}");

        serde_wasm_bindgen::to_value(&tx_updates)
            .map_err(|e| JsError::new(&format!("Serialization error: {:?}", e)))
    }

    pub async fn add_account(
        &self,
        account_id: String,
        network: ChainSupported,
    ) -> Result<(), JsError> {
        self.db_worker
            .update_user_account(account_id, network)
            .await
            .map_err(|e| JsError::new(&format!("Failed to add account to database: {:?}", e)))?;
        Ok(())
    }

    pub async fn verify_tx_call_payload(&self, tx: JsValue) -> Result<(), JsError> {
        let tx: TxStateMachine = TxStateMachine::from_js_value_unconditional(tx)?;

        let tx_call_payload_integrity_hash = Self::compute_tx_call_payload_integrity_hash(&tx)?;
        let tx_integrity_hash = Self::compute_tx_integrity_hash(&tx);

        // check if the tx_integrity_hash is the same as the tx_integrity_hash in the cache
        let cached_tx_integrity_hash = self
            .tx_integrity
            .borrow_mut()
            .get(&tx.tx_nonce)
            .ok_or(JsError::new(&format!(
                " Failed get transaction integrity from cache"
            )))?
            .clone();

        if tx_integrity_hash != cached_tx_integrity_hash {
            return Err(JsError::new("Transaction integrity failed"));
        }

        // check if if its the same with the integrity in call_payload
        if tx_call_payload_integrity_hash != tx_integrity_hash {
            return Err(JsError::new("Transaction call payload integrity failed"));
        }

        Ok(())
    }

    pub async fn receiver_confirm(&self, sig: Vec<u8>, tx: JsValue) -> Result<(), JsError> {
        let mut tx: TxStateMachine = TxStateMachine::from_js_value_unconditional(tx)?;
        let sender_channel = self.user_rpc_update_sender_channel.borrow_mut();

        if tx.status != TxStatus::Genesis {
            return Err(JsError::new("Transaction not in initial state"));
        }
        // Guard: ignore if already reverted
        if let TxStatus::Reverted(_) = tx.status {
            return Err(JsError::new("Transaction already reverted"));
        }
        if let TxStatus::ReceiverNotRegistered = tx.status {
            return Err(JsError::new("Receiver not registered"));
        }
        if let TxStatus::RecvAddrFailed = tx.status {
            return Err(JsError::new("Receiver address failed"));
        }

        let tx_integrity_hash = Self::compute_tx_integrity_hash(&tx);
        let recv_tx_integrity = self
            .tx_integrity
            .borrow_mut()
            .get(&tx.tx_nonce)
            .ok_or(JsError::new(&format!(
                " Failed get transaction integrity from cache"
            )))?
            .clone();
        if tx_integrity_hash != recv_tx_integrity {
            Err(anyhow!("Transaction integrity failed".to_string()))
                .map_err(|e| JsError::new(&format!("{:?}", e)))?
        }

        if tx.recv_signature.is_none() {
            Err(anyhow!("Receiver did not confirm".to_string()))
                .map_err(|e| JsError::new(&format!("{:?}", e)))?
        }

        tx.recv_confirmed();

        tx.increment_version();
        let vane_payload = VanePayload::new(tx.clone(), sig);

        sender_channel
            .send(vane_payload)
            .await
            .map_err(|_| anyhow!("failed to send recv confirmation tx state to sender channel"))
            .map_err(|e| JsError::new(&format!("{:?}", e)))?;

        Ok(())
    }

    pub async fn revert_transaction(
        &self,
        sig: Vec<u8>,
        tx: JsValue,
        reason: Option<String>,
    ) -> Result<(), JsError> {
        let mut tx: TxStateMachine = TxStateMachine::from_js_value_unconditional(tx)?;

        match tx.status {
            TxStatus::Reverted(ref reason) => {
                info!("revertTransaction: transaction already reverted");
                let sender = self.user_rpc_update_sender_channel.borrow_mut();
                // Immediately reflect in local cache for UI/state reads
                self.lru_cache
                    .borrow_mut()
                    .push(tx.tx_nonce.into(), TtlWrapper::new(tx.clone(), 600));

                let vane_payload = VanePayload::new(tx.clone(), sig);

                sender
                    .send(vane_payload)
                    .await
                    .map_err(|_| {
                        anyhow!("failed to send revert transaction tx state to sender channel")
                    })
                    .map_err(|e| JsError::new(&format!("{:?}", e)))?;
                Ok(())
            }
            _ => {
                tx.status =
                    TxStatus::Reverted(reason.unwrap_or("Intended receiver not met".to_string()));
                // Immediately reflect in local cache for UI/state reads
                {
                    let mut cache = self.lru_cache.borrow_mut();
                    if let Some(ttl_wrapper) = cache.get_mut(&tx.tx_nonce.into()) {
                        ttl_wrapper.update_value(tx.clone());
                        let ttl_wrapper_clone = ttl_wrapper.clone();
                        drop(cache);
                        self.lru_cache
                            .borrow_mut()
                            .push(tx.tx_nonce.into(), ttl_wrapper_clone);
                    } else {
                        drop(cache);
                        let now = (js_sys::Date::now() / 1000.0) as u32;
                        self.lru_cache
                            .borrow_mut()
                            .push(tx.tx_nonce.into(), TtlWrapper::new(tx.clone(), now));
                    }
                }

                let sender = self.user_rpc_update_sender_channel.borrow_mut();
                let vane_payload = VanePayload::new(tx.clone(), sig);

                sender
                    .send(vane_payload)
                    .await
                    .map_err(|_| {
                        anyhow!("failed to send revert transaction tx state to sender channel")
                    })
                    .map_err(|e| JsError::new(&format!("{:?}", e)))?;
                Ok(())
            }
        }
    }

    // export all storage
    pub async fn export_storage(&self) -> Result<JsValue, JsError> {
        // Call all getter methods and collect data
        let user_account = self.db_worker.get_user_account().await.ok();
        let nonce = self.db_worker.get_nonce().await.unwrap_or(0);
        let success_transactions = self.db_worker.get_success_txs().await.unwrap_or_default();
        let failed_transactions = self.db_worker.get_failed_txs().await.unwrap_or_default();

        // Create the export structure
        let storage_export = StorageExport {
            user_account,
            nonce,
            success_transactions,
            failed_transactions,
        };

        // Convert to JSON and return as JsValue
        serde_wasm_bindgen::to_value(&storage_export)
            .map_err(|e| JsError::new(&format!("Failed to serialize storage data: {}", e)))
    }

    pub fn delete_tx_in_cache(&self, tx: JsValue) -> Result<(), JsError> {
        let tx: TxStateMachine = TxStateMachine::from_js_value_unconditional(tx)?;
        self.lru_cache.borrow_mut().pop(&tx.tx_nonce.into());
        info!("Deleted transaction from cache: {:?}", tx.tx_nonce);
        Ok(())
    }

    // Cache maintenance
    pub fn clear_reverted_from_cache(&self) -> Result<(), JsError> {
        let keys: Vec<u32> = self
            .lru_cache
            .borrow()
            .iter()
            .filter_map(|(k, v)| match v.get_value().status {
                TxStatus::Reverted(_) => Some(*k),
                _ => None,
            })
            .collect();

        let mut cache = self.lru_cache.borrow_mut();
        for key in keys {
            let _ = cache.pop(&key);
        }
        info!("Cleared reverted transactions from cache");
        Ok(())
    }

    pub fn clear_finalized_from_cache(&self) -> Result<(), JsError> {
        let keys: Vec<u32> = self
            .lru_cache
            .borrow()
            .iter()
            .filter_map(|(k, v)| match v.get_value().status {
                TxStatus::Reverted(_) | TxStatus::TxSubmissionPassed { hash: _ } => Some(*k),
                _ => None,
            })
            .collect();

        let mut cache = self.lru_cache.borrow_mut();
        for key in keys {
            let _ = cache.pop(&key);
        }
        info!("Cleared finalized (reverted/submitted) transactions from cache");
        Ok(())
    }

    pub fn clear_cache(&self) -> Result<(), JsError> {
        self.lru_cache.borrow_mut().clear();
        self.tx_integrity.borrow_mut().clear();
        info!("Cleared all from cache");
        Ok(())
    }
}

// =================== The interface =================== //

#[wasm_bindgen]
pub struct PublicInterfaceWorkerJs {
    inner: Rc<RefCell<PublicInterfaceWorker>>,
}

impl PublicInterfaceWorkerJs {
    pub fn new(inner: Rc<RefCell<PublicInterfaceWorker>>) -> Self {
        Self { inner }
    }
}

#[wasm_bindgen]
impl PublicInterfaceWorkerJs {
    #[wasm_bindgen(js_name = "initiateTransaction")]
    pub async fn initiate_transaction(
        &self,
        sig: Vec<u8>,
        sender: String,
        receiver: String,
        amount: u128,
        token: JsValue,
        code_word: String,
        sender_network: JsValue,
        receiver_network: JsValue,
    ) -> Result<JsValue, JsError> {
        let token: Token = Token::from_js_value_unconditional(token)?;
        let sender_network_chain: ChainSupported =
            ChainSupported::from_js_value_unconditional(sender_network)?;
        let receiver_network_chain: ChainSupported =
            ChainSupported::from_js_value_unconditional(receiver_network)?;

        self.inner
            .borrow()
            .initiate_transaction(
                sig,
                sender,
                receiver,
                amount,
                token,
                code_word,
                sender_network_chain,
                receiver_network_chain,
            )
            .await
    }

    #[wasm_bindgen(js_name = "senderConfirm")]
    pub async fn sender_confirm(&self, sig: Vec<u8>, tx: JsValue) -> Result<(), JsError> {
        self.inner.borrow().sender_confirm(sig, tx).await?;
        Ok(())
    }

    #[wasm_bindgen(js_name = "watchTxUpdates")]
    pub async fn watch_tx_updates(&self, callback: &js_sys::Function) -> Result<(), JsError> {
        self.inner.borrow().watch_tx_updates(callback).await?;
        Ok(())
    }

    #[wasm_bindgen(js_name = "unsubscribeWatchTxUpdates")]
    pub fn unsubscribe_watch_tx_updates(&self) -> Result<(), JsError> {
        self.inner.borrow().unsubscribe_watch_tx_updates()?;
        Ok(())
    }

    #[wasm_bindgen(js_name = "watchP2pNotifications")]
    pub async fn watch_p2p_notifications(
        &self,
        callback: &js_sys::Function,
    ) -> Result<(), JsError> {
        self.inner
            .borrow()
            .watch_p2p_notifications(callback)
            .await?;
        Ok(())
    }

    #[wasm_bindgen(js_name = "unsubscribeWatchP2pNotifications")]
    pub fn unsubscribe_watch_p2p_notifications(&self) -> Result<(), JsError> {
        self.inner.borrow().unsubscribe_watch_p2p_notifications()?;
        Ok(())
    }

    #[wasm_bindgen(js_name = "fetchPendingTxUpdates")]
    pub async fn fetch_pending_tx_updates(&self, sig: Vec<u8>) -> Result<JsValue, JsError> {
        let tx_updates = self.inner.borrow().fetch_pending_tx_updates(sig).await?;
        Ok(tx_updates)
    }

    #[wasm_bindgen(js_name = "addAccount")]
    pub async fn add_account(&self, account_id: String, network: JsValue) -> Result<(), JsError> {
        let network_chain: ChainSupported = ChainSupported::from_js_value_unconditional(network)?;
        self.inner
            .borrow()
            .add_account(account_id, network_chain)
            .await?;
        Ok(())
    }

    #[wasm_bindgen(js_name = "receiverConfirm")]
    pub async fn receiver_confirm(&self, sig: Vec<u8>, tx: JsValue) -> Result<(), JsError> {
        self.inner.borrow().receiver_confirm(sig, tx).await?;
        Ok(())
    }

    #[wasm_bindgen(js_name = "verifyTxCallPayload")]
    pub async fn verify_tx_call_payload(&self, tx: JsValue) -> Result<(), JsError> {
        self.inner.borrow().verify_tx_call_payload(tx).await?;
        Ok(())
    }

    #[wasm_bindgen(js_name = "revertTransaction")]
    pub async fn revert_transaction(
        &self,
        sig: Vec<u8>,
        tx: JsValue,
        reason: Option<String>,
    ) -> Result<(), JsError> {
        self.inner
            .borrow()
            .revert_transaction(sig, tx, reason)
            .await?;
        Ok(())
    }

    #[wasm_bindgen(js_name = "exportStorage")]
    pub async fn export_storage(&self) -> Result<JsValue, JsError> {
        let storage_export = self.inner.borrow().export_storage().await?;
        Ok(storage_export)
    }

    #[wasm_bindgen(js_name = "deleteTxInCache")]
    pub fn delete_tx_in_cache(&self, tx: JsValue) -> Result<(), JsError> {
        self.inner.borrow().delete_tx_in_cache(tx)
    }

    #[wasm_bindgen(js_name = "clearRevertedFromCache")]
    pub fn clear_reverted_from_cache(&self) -> Result<(), JsError> {
        self.inner.borrow().clear_reverted_from_cache()?;
        Ok(())
    }

    #[wasm_bindgen(js_name = "clearFinalizedFromCache")]
    pub fn clear_finalized_from_cache(&self) -> Result<(), JsError> {
        self.inner.borrow().clear_finalized_from_cache()?;
        Ok(())
    }

    #[wasm_bindgen(js_name = "clearCache")]
    pub fn clear_cache(&self) -> Result<(), JsError> {
        self.inner.borrow().clear_cache()?;
        Ok(())
    }
}
