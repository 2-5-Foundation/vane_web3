extern crate alloc;
extern crate core;

pub mod cryptography;
pub mod interface;
pub mod logging;
pub mod p2p;
pub mod tx_processing;

use alloc::{format, rc::Rc, string::String, sync::Arc, vec};
use core::{cell::RefCell, str::FromStr};
use std::collections::HashMap;

use crate::p2p::{host_get_dht, host_set_dht};
use crate::{
    interface::{PublicInterfaceWorker, PublicInterfaceWorkerJs},
    p2p::{P2pNetworkService, WasmP2pWorker},
    tx_processing::WasmTxProcessingWorker,
};
use anyhow::{anyhow, Error};
use codec::Decode;
use db_wasm::{DbWorker, InMemoryDbWorker, OpfsRedbWorker};
use futures::{future, FutureExt};
use gloo_timers::future::TimeoutFuture;
use libp2p::{kad::QueryId, multiaddr::Protocol, Multiaddr, PeerId};
use log::{debug, error, info, warn};
use lru::LruCache;
use primitives::data_structure::{
    ChainSupported, DbTxStateMachine, DbWorkerInterface, HashId, NetworkCommand, StorageExport,
    SwarmMessage, TtlWrapper, TxStateMachine, TxStatus, UserAccount,
};
use wasm_bindgen::{prelude::wasm_bindgen, JsValue};
use wasm_timer::TryFutureExt;
#[derive(Clone)]
pub struct WasmMainServiceWorker {
    pub db_worker: Rc<DbWorker>,
    pub public_interface_worker: Rc<RefCell<PublicInterfaceWorker>>,
    pub wasm_tx_processing_worker: Rc<RefCell<WasmTxProcessingWorker>>,
    // for swarm events
    pub p2p_worker: Rc<RefCell<WasmP2pWorker>>,
    //telemetry_worker: TelemetryWorker,
    pub p2p_network_service: Rc<RefCell<P2pNetworkService>>,
    // channels for layers communication
    /// sender channel to propagate transaction state to rpc layer
    /// this serve as an update channel to the user
    pub rpc_sender_channel: Rc<RefCell<tokio_with_wasm::alias::sync::mpsc::Sender<TxStateMachine>>>,
    /// receiver channel to handle the updates made by user from rpc
    pub user_rpc_update_recv_channel:
        Rc<RefCell<tokio_with_wasm::alias::sync::mpsc::Receiver<TxStateMachine>>>,
    /// channel to handle dht query results
    pub dht_query_result_channel:
        Rc<RefCell<tokio_with_wasm::alias::sync::mpsc::Receiver<(Option<Multiaddr>, u32)>>>,
    // query id -> (txn, target_id)
    pub dht_query_context: Rc<RefCell<HashMap<u32, (Rc<RefCell<TxStateMachine>>, String)>>>,

    // moka cache
    pub lru_cache: Rc<RefCell<LruCache<u32, TtlWrapper<TxStateMachine>>>>,
}

impl WasmMainServiceWorker {
    pub(crate) async fn new(
        relay_node_multi_addr: String,
        account: String,
        network: String,
        live: bool,
        libp2p_key: String,
        storage: Option<StorageExport>,
    ) -> Result<Self, anyhow::Error> {
        // CHANNELS
        // ===================================================================================== //
        // for rpc messages back and forth propagation
        let (rpc_sender_channel, rpc_recv_channel) =
            tokio_with_wasm::alias::sync::mpsc::channel(10);
        let (user_rpc_update_sender_channel, user_rpc_update_recv_channel) =
            tokio_with_wasm::alias::sync::mpsc::channel(10);

        // for p2p network commands
        let (p2p_command_tx, p2p_command_recv) =
            tokio_with_wasm::alias::sync::mpsc::channel::<NetworkCommand>(10);

        let (dht_query_result_tx, dht_query_result_recv) =
            tokio_with_wasm::alias::sync::mpsc::channel::<(Option<Multiaddr>, u32)>(10);

        // DATABASE WORKER (LOCAL AND REMOTE )
        // ===================================================================================== //
        let db = DbWorker::initialize_inmemory_db_client("vane.db").await?;

        let db_worker = Rc::new(db);

        // Use bounded cache to prevent memory overflow in WASM environment
        // 10 entries should be sufficient for most use cases while preventing unbounded growth
        let lru_cache: Rc<RefCell<LruCache<u32, TtlWrapper<TxStateMachine>>>> = Rc::new(RefCell::new(
            LruCache::new(std::num::NonZeroUsize::new(10).unwrap()),
        ));

        // PEER TO PEER NETWORKING WORKER
        // ===================================================================================== //

        let p2p_worker = WasmP2pWorker::new(
            live,
            db_worker.clone(),
            relay_node_multi_addr,
            account.clone(),
            p2p_command_recv,
            dht_query_result_tx,
            libp2p_key,
        )
        .await
        .map_err(|e| anyhow::anyhow!("P2P worker creation failed: {:?}", e))?;

        let p2p_network_service =
            P2pNetworkService::new(Rc::new(p2p_command_tx), p2p_worker.clone())?;

        let user_account = UserAccount {
            multi_addr: p2p_worker
                .user_circuit_multi_addr
                .clone()
                .with_p2p(p2p_worker.node_id)
                .expect("failed to add p2p protocol")
                .to_string(),
            accounts: vec![(account, ChainSupported::from(network.as_str()))],
        };
        db_worker.set_user_account(user_account).await?;

        // Apply storage export data if provided
        if let Some(storage_export) = storage {
            Self::apply_storage_export(db_worker.clone(), storage_export).await?;
        }

        // TRANSACTION RPC WORKER / PUBLIC INTERFACE
        // ===================================================================================== //

        let public_interface_worker = PublicInterfaceWorker::new(
            db_worker.clone(),
            Rc::new(p2p_worker.clone()),
            Rc::new(p2p_network_service.clone()),
            Rc::new(RefCell::new(rpc_recv_channel)),
            Rc::new(RefCell::new(user_rpc_update_sender_channel)),
            p2p_worker.node_id,
            lru_cache.clone(),
        )
        .await
        .map_err(|e| anyhow::anyhow!("Public interface worker creation failed: {:?}", e))?;

        // TRANSACTION PROCESSING LAYER
        // ===================================================================================== //

        let wasm_tx_processing_worker = WasmTxProcessingWorker::new()?;
        // ===================================================================================== //

        Ok(Self {
            db_worker,
            public_interface_worker: Rc::new(RefCell::new(public_interface_worker)),
            wasm_tx_processing_worker: Rc::new(RefCell::new(wasm_tx_processing_worker)),
            p2p_worker: Rc::new(RefCell::new(p2p_worker)),
            p2p_network_service: Rc::new(RefCell::new(p2p_network_service)),
            rpc_sender_channel: Rc::new(RefCell::new(rpc_sender_channel)),
            user_rpc_update_recv_channel: Rc::new(RefCell::new(user_rpc_update_recv_channel)),
            lru_cache,
            dht_query_result_channel: Rc::new(RefCell::new(dht_query_result_recv)),
            dht_query_context: Rc::new(RefCell::new(HashMap::new())),
        })
    }

    pub fn start_swarm_handler(&self) -> Result<(), Error> {
        let (sender_channel, mut recv_channel) = tokio_with_wasm::alias::sync::mpsc::channel(256);

        // Start swarm and get it ready to send messages
        let p2p_worker_clone = self.p2p_worker.clone();
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(e) = p2p_worker_clone
                .borrow_mut()
                .start_swarm(Rc::new(RefCell::new(sender_channel)))
                .await
            {
                error!("start swarm failed: {}", e);
            }
        });

        // Set up message processing using the receiver channel
        let self_clone = self.clone();
        let tx_worker = self.wasm_tx_processing_worker.borrow().clone();

        wasm_bindgen_futures::spawn_local(async move {
            loop {
                while let Some(swarm_msg_result) = recv_channel.recv().await {
                    // Process directly in this task
                    // system failures should result in shutting down the node and reporting
                    if let Err(e) = self_clone
                        .handle_swarm_message(swarm_msg_result, &tx_worker)
                        .await
                    {
                        error!("Error processing swarm message: {}", e);
                    }
                }
            }
        });

        Ok(())
    }

    pub async fn handle_swarm_message(
        &self,
        swarm_msg_result: Result<SwarmMessage, Error>,
        txn_processing_worker: &WasmTxProcessingWorker,
    ) -> Result<(), Error> {
        match swarm_msg_result {
            Ok(swarm_msg) => match swarm_msg {
                // receiver incoming request
                SwarmMessage::WasmRequest { data, inbound_id } => {
                    let mut decoded_req: TxStateMachine = data;
                    let inbound_req_id = inbound_id.get_hash_id();
                    decoded_req.inbound_req_id = Some(inbound_req_id);

                    debug!(target: "MainServiceWorker",
                          "decoded request: {:?}", &decoded_req);
                    // Use non-blocking try_send for WASM environment
                    if let Err(e) = self
                        .rpc_sender_channel
                        .borrow_mut()
                        .try_send(decoded_req.clone())
                    {
                        error!("Failed to send to RPC channel: {}", e);
                    }

                    // receiver gets 5 mins to respond
                    let now = (js_sys::Date::now() / 1000.0) as u32;

                    self.lru_cache
                        .borrow_mut()
                        .push(decoded_req.tx_nonce.into(), TtlWrapper::new(decoded_req, now));

                    info!(target: "MainServiceWorker",
                          "propagating txn msg as request");
                }

                SwarmMessage::WasmResponse { data, outbound_id } => {
                    // sender receives the response from the receiver
                    let mut decoded_resp: TxStateMachine = data;
                    let outbound_req_id = outbound_id.get_hash_id();
                    decoded_resp.outbound_req_id = Some(outbound_req_id);

                    // Drop updates if already reverted in cache (authoritative sender state)
                    if let Some(existing) = self
                        .lru_cache
                        .borrow()
                        .peek(&decoded_resp.tx_nonce.into())
                        .cloned()
                    {
                        if let TxStatus::Reverted(_) = existing.get_value().status {
                            warn!(target: "MainServiceWorker", "ignoring inbound response for reverted tx: {}", decoded_resp.tx_nonce);
                            return Ok(());
                        }
                        // Version gating: ignore stale updates
                        if decoded_resp.tx_version < existing.get_value().tx_version {
                            warn!(target: "MainServiceWorker", "ignoring stale inbound response (version {}) < (local {})", decoded_resp.tx_version, existing.get_value().tx_version);
                            return Ok(());
                        }
                    }

                    match txn_processing_worker
                        .validate_receiver_and_sender_address(&decoded_resp, "Receiver")
                    {
                        Ok(_) => {
                            decoded_resp.recv_confirmation_passed();
                            decoded_resp.increment_version();
                            info!(target: "MainServiceWorker", "receiver confirmation passed");

                            let mut tx_processing = self.wasm_tx_processing_worker.borrow_mut();
                            if let Err(e) = tx_processing.create_tx(&mut decoded_resp).await {
                                // send the error to the rpc layer
                                // there should be error reporting worker
                                error!(target: "MainServiceWorker", "failed to create tx: {e}");
                                decoded_resp.status =
                                    TxStatus::TxError(format!("Failed to create transaction "));
                                

                                // sender gets 5 mins to sign and submit tx
                                if let Some(ttl_wrapper) = self.lru_cache.borrow_mut().get_mut(&decoded_resp.tx_nonce.into()){
                                    ttl_wrapper.update_value(decoded_resp.clone());

                                    self.rpc_sender_channel
                                    .borrow_mut()
                                    .send(decoded_resp.clone())
                                    .await?;

                                } else {
                                    // for hear send with TxError (failed to create transaction)
                                    decoded_resp.status = TxStatus::TxError("Failed to create transaction".to_string());
                                    let now = (js_sys::Date::now() / 1000.0) as u32;
                                    self.lru_cache
                                        .borrow_mut()
                                        .push(decoded_resp.tx_nonce.into(), TtlWrapper::new(decoded_resp.clone(), now));
                                    self.rpc_sender_channel
                                        .borrow_mut()
                                        .send(decoded_resp.clone())
                                        .await?;

                                    error!("Failed to get transaction from cache; expired");
                                    
                                }
                                
                                return Ok(())
                            }
                        }
                        Err(err) => {
                            decoded_resp.recv_confirmation_failed();
                            decoded_resp.increment_version();
                            error!(target: "MainServiceWorker",
                                  "receiver confirmation failed: {err}");

                            let db_tx = DbTxStateMachine {
                                tx_hash: vec![],
                                amount: decoded_resp.amount.clone(),
                                token: decoded_resp.token.clone(),
                                sender: decoded_resp.sender_address.clone(),
                                receiver: decoded_resp.receiver_address.clone(),
                                sender_network: decoded_resp.sender_address_network.clone(),
                                receiver_network: decoded_resp.receiver_address_network.clone(),
                                success: false,
                            };
                            self.db_worker.update_failed_tx(db_tx).await?;
                        }
                    }

                    if let Err(e) = self
                        .rpc_sender_channel
                        .borrow_mut()
                        .try_send(decoded_resp.clone())
                    {
                        //handle this error on the error worker
                        error!("Failed to send response to RPC channel: {}", e);
                        return Err(e.into());
                    }

                    {

                        if let Some(ttl_wrapper) = self.lru_cache.borrow_mut().get_mut(&decoded_resp.tx_nonce.into()) {
                            ttl_wrapper.update_value(decoded_resp.clone());
                        } else {
                            decoded_resp.status = TxStatus::TxError("Transaction expired".to_string());
                            
                            let now = (js_sys::Date::now() / 1000.0) as u32;
                            let new_wrapper = TtlWrapper::new(decoded_resp.clone(), now);
                            
                            self.lru_cache
                                .borrow_mut()
                                .push(decoded_resp.tx_nonce.into(), new_wrapper);
                            
                            self.rpc_sender_channel
                                .borrow_mut()
                                .send(decoded_resp.clone())
                                .await?;
                            return Ok(());
                        }
                    }

                    debug!(target: "MainServiceWorker",
                          "propagating txn msg as response to rpc layer for user interaction: {decoded_resp:?}");
                }
                _ => {}
            },
            Err(err) => {
                error!("Error in swarm message: {err}");
            }
        }
        Ok(())
    }

    pub async fn handle_genesis_tx_state(
        &mut self,
        txn: Rc<RefCell<TxStateMachine>>,
    ) -> Result<(), Error> {
        let target_id = txn.borrow().receiver_address.clone();

        // take handles out of `self` so we don't capture `self` in the spawned task
        let db = self.db_worker.clone();
        let p2p_network_service = self.p2p_network_service.clone();
        let p2p_worker = self.p2p_worker.clone();
        let rpc_sender_channel = self.rpc_sender_channel.clone();
        let lru_cache = self.lru_cache.clone();

        // But first check if it is the same user, if it is then just send it to self no p2p
        let target_user_profile = db.get_user_account().await?;
        let (receiver_in_profile, sender_in_profile) = {
            let txn_borrow = txn.borrow();
            let receiver_in_profile = target_user_profile
                .accounts
                .iter()
                .any(|(acc, _)| *acc == txn_borrow.receiver_address);
            let sender_in_profile = target_user_profile
                .accounts
                .iter()
                .any(|(acc, _)| *acc == txn_borrow.sender_address);
            (receiver_in_profile, sender_in_profile)
        };

        if receiver_in_profile && sender_in_profile {
            // send to interface/rpc make the receiver confirm
            let mut ttl_wrapper = lru_cache.borrow_mut().get(&txn.borrow().tx_nonce.into()).ok_or(anyhow::anyhow!(" Failed get transaction from cache"))?.clone();
            ttl_wrapper.update_value(txn.borrow().clone());
            lru_cache
                .borrow_mut()
                .push(txn.borrow().tx_nonce.into(), ttl_wrapper);

            rpc_sender_channel
                .borrow_mut()
                .send(txn.borrow().clone())
                .await?;
            return Ok(());
        }

        info!(target: "MainServiceWorker", "Checking PeerId in DHT");

        let p2p_network_service = p2p_network_service.clone();
        let rpc_sender_channel = rpc_sender_channel.clone();
        let lru_cache = lru_cache.clone();
        let txn = txn.clone();
        let target_id = target_id.clone();
        let db_worker = db.clone();

        wasm_bindgen_futures::spawn_local(async move {
            info!(target: "MainServiceWorker", "🔍 Starting DHT lookup for target: {}", target_id);

            // Exponential backoff retry: 3 attempts (0.5s, 1.5s, 3.0s) between attempts
            let mut attempt: u8 = 1;
            let max_attempts: u8 = 3;
            loop {
                let dht = host_get_dht(target_id.clone()).fuse();
                let timeout = TimeoutFuture::new(60_000).fuse();
                futures::pin_mut!(dht, timeout);

                match future::select(dht, timeout).await {
                    // DHT returned
                    future::Either::Left((Ok(resp), _)) => {
                        if let Some(err) = resp.error {
                            warn!(target: "MainServiceWorker", "DHT get returned error (attempt {}/{}): {}", attempt, max_attempts, err);
                        } else {
                            let maybe_addr = resp
                                .value
                                .and_then(|s| (!s.is_empty()).then(|| Multiaddr::try_from(s).ok()))
                                .flatten();

                            if let Some(multi_addr) = maybe_addr {
                                let peer_id = match multi_addr.clone().pop() {
                                    Some(libp2p::multiaddr::Protocol::P2p(id)) => id,
                                    _ => {
                                        error!("peer id missing in multiaddr");
                                        return;
                                    }
                                };

                                // record to db
                                if let Err(e) = db_worker
                                    .record_saved_user_peers(
                                        target_id.clone(),
                                        multi_addr.to_string(),
                                    )
                                    .await
                                {
                                    error!("record_saved_user_peers failed: {target_id}: {e:?}");
                                    return;
                                }

                                if let Err(e) = p2p_network_service
                                    .borrow_mut()
                                    .wasm_send_request(txn.clone(), peer_id, multi_addr)
                                    .await
                                {
                                    error!("wasm_send_request failed: {e:?}");
                                    let mut tx = txn.borrow_mut().clone();
                                    tx.status =
                                        TxStatus::TxError("Failed to reach receiver".to_string());
                                    let _ = rpc_sender_channel.borrow_mut().send(tx.clone()).await;

                                    // error worker handler
                                    let mut ttl_wrapper = lru_cache.borrow_mut().get(&tx.tx_nonce.into()).expect("Failed to get transaction from cache; panic").clone();
                                    ttl_wrapper.update_value(tx.clone());
                                    lru_cache.borrow_mut().push(tx.tx_nonce.into(), ttl_wrapper.clone());
                                    return;
                                }

                                return;
                            } else {
                                warn!(target: "MainServiceWorker", "DHT returned empty value (attempt {}/{})", attempt, max_attempts);
                            }
                        }
                    }

                    // here we should have the error worker
                    // DHT internal error
                    future::Either::Left((Err(e), _)) => {
                        warn!(target: "MainServiceWorker", "host_get_dht internal error (attempt {}/{}): {e:?}", attempt, max_attempts);
                    }

                    // timeout
                    future::Either::Right((_elapsed, _)) => {
                        warn!(target: "MainServiceWorker", "⏳ DHT timed out for {} (attempt {}/{})", target_id, attempt, max_attempts);
                    }
                }

                if attempt >= max_attempts {
                    // give up and notify not registered
                    info!(target: "MainServiceWorker", "DHT: receiver did not register");
                    let mut t = txn.borrow_mut().clone();
                    t.recv_not_registered();

                    let _ = rpc_sender_channel.borrow_mut().send(t.clone()).await;
                    
                    let mut ttl_wrapper = lru_cache.borrow_mut().get(&t.tx_nonce.into()).expect("Failed to get transaction from cache; panic").clone();
                    ttl_wrapper.update_value(t.clone());
                    lru_cache.borrow_mut().push(t.tx_nonce.into(), ttl_wrapper.clone());
                    return;
                }

                // backoff before next attempt
                let backoff_ms: u32 = match attempt {
                    1 => 500,
                    2 => 1500,
                    _ => 3000,
                };
                attempt += 1;
                TimeoutFuture::new(backoff_ms).await;
            }
        });

        return Ok(());

        // match db.get_saved_user_peers(target_id.clone()).await {
        //     Ok(addr_str) => {
        //         let multi_addr = addr_str
        //             .parse::<Multiaddr>()
        //             .map_err(|e| anyhow::anyhow!("failed to parse multiaddr: {e}"))?;

        //         let peer_id = match multi_addr.clone().pop() {
        //             Some(Protocol::P2p(id)) => id,
        //             _ => return Err(anyhow::anyhow!("peer id not found")),
        //         };

        //         // if it fails here, it either means, the peer is not currently online or the receiver changed their peer id
        //         p2p_network_service
        //             .borrow_mut()
        //             .dial_to_peer_id(multi_addr.clone(), &peer_id)
        //             .await?;

        //         p2p_network_service
        //             .borrow_mut()
        //             .wasm_send_request(txn.clone(), peer_id, multi_addr)
        //             .await?;

        //         return Ok(());
        //     }

        //     Err(_) => {
        //         // 2) DB miss → spawn DHT fallback and return immediately

        //     }
        // }
    }

    pub async fn handle_recv_addr_confirmed_tx_state(
        &self,
        txn: Rc<RefCell<TxStateMachine>>,
    ) -> Result<(), Error> {
        // check if the receiver & sender is the same vane account
        let target_user_profile = self.db_worker.get_user_account().await?;
        let (receiver_in_profile, sender_in_profile) = {
            let txn_borrow = txn.borrow();
            let receiver_in_profile = target_user_profile
                .accounts
                .iter()
                .any(|(acc, _)| *acc == txn_borrow.receiver_address);
            let sender_in_profile = target_user_profile
                .accounts
                .iter()
                .any(|(acc, _)| *acc == txn_borrow.sender_address);
            (receiver_in_profile, sender_in_profile)
        };

        if receiver_in_profile && sender_in_profile {
            let mut txn_inner = txn.borrow().clone();
            let validation_result = {
                let tx_processing = self.wasm_tx_processing_worker.borrow();
                tx_processing.validate_receiver_and_sender_address(&txn_inner, "Receiver")
            };

            match validation_result {
                Ok(_) => {
                    txn_inner.recv_confirmation_passed();
                    txn_inner.increment_version();
                    info!(target: "MainServiceWorker", "receiver confirmation passed");

                    let mut tx_processing = self.wasm_tx_processing_worker.borrow_mut();
                    if let Err(e) = tx_processing.create_tx(&mut txn_inner).await {
                        // send the error to the rpc layer
                        // there should be error reporting worker
                        error!(target: "MainServiceWorker", "failed to create tx: {e}");
                        txn_inner.status =
                            TxStatus::TxError("Failed to create transaction".to_string());
                        
                        if let Some(mut ttl_wrapper) = self.lru_cache.borrow_mut().get_mut(&txn_inner.tx_nonce.into()) {
                            ttl_wrapper.update_value(txn_inner.clone());
                            self.lru_cache
                                .borrow_mut()
                                .push(txn_inner.tx_nonce.into(), ttl_wrapper.clone());

                            self.rpc_sender_channel
                                .borrow_mut()
                                .send(txn_inner.clone())
                                .await?;
                            return Ok(());
                        }else{
                           error!("Failed to get transaction from cache; expired");
                           return Err(anyhow::anyhow!("Failed to get transaction from cache; expired").into());
                        }
                        
                        // should not continue with the tx
                        return Ok(());
                    }
                }
                Err(err) => {
                    txn_inner.recv_confirmation_failed();
                    txn_inner.increment_version();
                    error!(target: "MainServiceWorker",
                      "receiver confirmation failed: {err}");

                    let db_tx = DbTxStateMachine {
                        tx_hash: vec![],
                        amount: txn_inner.amount.clone(),
                        token: txn_inner.token.clone(),
                        sender: txn_inner.sender_address.clone(),
                        receiver: txn_inner.receiver_address.clone(),
                        sender_network: txn_inner.sender_address_network.clone(),
                        receiver_network: txn_inner.receiver_address_network.clone(),
                        success: false,
                    };
                    self.db_worker.update_failed_tx(db_tx).await?;
                }
            }
            if let Err(e) = self
                .rpc_sender_channel
                .borrow_mut()
                .try_send(txn_inner.clone())
            {
                //handle this error on the error worker
                error!("Failed to send response to RPC channel: {}", e);
                return Err(e.into());
            }

            if let Some(ttl_wrapper) = self.lru_cache.borrow_mut().get_mut(&txn_inner.tx_nonce.into()) {
                ttl_wrapper.update_value(txn_inner.clone());
                
            } else {
                error!("Failed to get transaction from cache; expired");
                return Err(anyhow::anyhow!("Failed to get transaction from cache; expired").into());
            }
            
            debug!(target: "MainServiceWorker",
                          "propagating txn msg as response to rpc layer for user interaction: {txn_inner:?}");

            return Ok(());
        }
        // if the receiver is not the same user, send to p2p network
        self.p2p_network_service
            .borrow_mut()
            .wasm_send_response(txn)
            .await?;
        Ok(())
    }

    pub async fn handle_sender_confirmed_tx_state(
        &self,
        txn: Rc<RefCell<TxStateMachine>>,
    ) -> Result<(), Error> {
        let mut txn_inner = txn.borrow_mut().clone();

        // verify receiver again
        self.wasm_tx_processing_worker
            .borrow()
            .validate_receiver_and_sender_address(&txn_inner, "Receiver")?;

        // verify sender
        if let Err(e) = self.wasm_tx_processing_worker
            .borrow()
            .validate_receiver_and_sender_address(&txn_inner, "Sender"){
                error!(target: "MainServiceWorker","sender confirmation failed: {e}");
                txn_inner.sender_confirmation_failed();
                let now = (js_sys::Date::now() / 1000.0) as u32;
                self.lru_cache.borrow_mut().push(txn_inner.tx_nonce.into(), TtlWrapper::new(txn_inner.clone(), now));
                self.rpc_sender_channel
                    .borrow_mut()
                    .send(txn_inner.clone())
                    .await?;
                return Ok(());
        }
        info!(target: "MainServiceWorker","sender confirmation passed");
        // verify multi id
        if self
            .wasm_tx_processing_worker
            .borrow()
            .validate_multi_id(&txn_inner)
        {
            info!(target: "MainServiceWorker","multiId verification passed");
            // TODO! handle submission errors
            // signed and ready to be submitted to target chain
            match self
                .wasm_tx_processing_worker
                .borrow_mut()
                .submit_tx(txn_inner.clone())
                .await
            {
                Ok(tx_hash) => {
                    // update user via rpc on tx success
                    txn_inner.tx_submission_passed(tx_hash);
                    info!(target: "MainServiceWorker","tx submission passed");

                    // update local db on success tx
                    let db_tx = DbTxStateMachine {
                        tx_hash: tx_hash.to_vec(),
                        amount: txn_inner.amount.clone(),
                        token: txn_inner.token.clone(),
                        sender: txn_inner.sender_address.clone(),
                        receiver: txn_inner.receiver_address.clone(),
                        sender_network: txn_inner.sender_address_network.clone(),
                        receiver_network: txn_inner.receiver_address_network.clone(),
                        success: true,
                    };
                    self.db_worker.update_success_tx(db_tx).await?;
                    info!(target: "MainServiceWorker","Db recorded success tx");

                    if let Some(ttl_wrapper) = self.lru_cache.borrow_mut().get_mut(&txn_inner.tx_nonce.into()) {
                        ttl_wrapper.update_value(txn_inner.clone());

                        self.rpc_sender_channel
                            .borrow_mut()
                            .send(txn_inner.clone())
                            .await?;
                    }else{
                        let now = (js_sys::Date::now() / 1000.0) as u32;
                        self.lru_cache.borrow_mut().push(txn_inner.tx_nonce.into(), TtlWrapper::new(txn_inner.clone(), now));
                        self.rpc_sender_channel
                            .borrow_mut()
                            .send(txn_inner.clone())
                            .await?;
                    }
                }
                Err(err) => {
                    // here some errors wont get the tx to be resubmitted
                    error!(target: "MainServiceWorker","tx submission failed: {err:?}");
                    txn_inner.tx_submission_failed("Failed to submit transaction".to_string());

                    if let Some(ttl_wrapper) = self.lru_cache.borrow_mut().get_mut(&txn_inner.tx_nonce.into()) {
                        ttl_wrapper.update_value(txn_inner.clone());

                        self.rpc_sender_channel
                            .borrow_mut()
                            .send(txn_inner.clone())
                            .await?;
                    }else{
                        let now = (js_sys::Date::now() / 1000.0) as u32;
                        self.lru_cache.borrow_mut().push(txn_inner.tx_nonce.into(), TtlWrapper::new(txn_inner.clone(), now));
                        self.rpc_sender_channel
                            .borrow_mut()
                            .send(txn_inner.clone())
                            .await?;
                    }
                }
            }
        } else {
            // non original sender confirmed, return error, send to rpc
            txn_inner.status = TxStatus::TxError("failed to match original sender and receiver".to_string());
            error!(target: "MainServiceWorker","Non original sender or receiver signed");

            if let Some(ttl_wrapper) = self.lru_cache.borrow_mut().get_mut(&txn_inner.tx_nonce.into()) {
                ttl_wrapper.update_value(txn_inner.clone());
                
                self.rpc_sender_channel
                    .borrow_mut()
                    .send(txn_inner.clone())
                    .await?;

            }else{
                let now = (js_sys::Date::now() / 1000.0) as u32;
                    self.lru_cache.borrow_mut().push(txn_inner.tx_nonce.into(), TtlWrapper::new(txn_inner.clone(), now));
                    self.rpc_sender_channel
                    .borrow_mut()
                    .send(txn_inner.clone())
                    .await?;
            }

            let db_tx = DbTxStateMachine {
                tx_hash: vec![],
                amount: txn_inner.amount.clone(),
                token: txn_inner.token.clone(),
                sender: txn_inner.sender_address.clone(),
                receiver: txn_inner.receiver_address.clone(),
                sender_network: txn_inner.sender_address_network.clone(),
                receiver_network: txn_inner.receiver_address_network.clone(),
                success: false,
            };
            self.db_worker.update_failed_tx(db_tx).await?;
            info!(target: "MainServiceWorker","Db recorded failed tx");
        }

        Ok(())
    }

    pub async fn handle_reverted_tx_state(
        &self,
        txn: Rc<RefCell<TxStateMachine>>,
    ) -> Result<(), Error> {
        let txn_inner = txn.borrow().clone();
        info!(target:"MainServiceWorker","revert for tx {:?} ({:?})", txn_inner.tx_nonce, txn_inner.status);

        // Both ends in this profile?
        let prof = self.db_worker.get_user_account().await?;
        let both_in_profile = prof
            .accounts
            .iter()
            .any(|(a, _)| *a == txn_inner.receiver_address)
            && prof
                .accounts
                .iter()
                .any(|(a, _)| *a == txn_inner.sender_address);

        // If not, best-effort disconnect + cleanup
        if !both_in_profile {
            // 1) Try DB
            let mut multi_addr = self
                .db_worker
                .get_saved_user_peers(txn_inner.receiver_address.clone())
                .await
                .ok()
                .and_then(|s| Multiaddr::try_from(s).ok());

            // 2) If DB miss, do short DHT lookup (async, with timeout)
            if multi_addr.is_none() {
                let dht = host_get_dht(txn_inner.receiver_address.clone()).fuse();
                let timeout = TimeoutFuture::new(3_000).fuse();
                futures::pin_mut!(dht, timeout);

                let addr_opt: Option<String> = match future::select(dht, timeout).await {
                    future::Either::Left((Ok(r), _)) => r.value,
                    future::Either::Right((_elapsed, _)) => {
                        warn!(target:"MainServiceWorker","revert: DHT lookup timed out for {}", txn_inner.receiver_address);
                        None
                    }
                    future::Either::Left((Err(e), _)) => {
                        warn!(target:"MainServiceWorker","revert: DHT lookup error for {}: {:?}", txn_inner.receiver_address, e);
                        None
                    }
                };

                multi_addr = addr_opt.and_then(|s| Multiaddr::try_from(s).ok());
            }

            // 3) Disconnect + delete (best-effort)
            if let Some(mut ma) = multi_addr.clone() {
                let ma_str = ma.to_string();
                if let Some(Protocol::P2p(id)) = ma.pop() {
                    if let Err(e) = self
                        .p2p_network_service
                        .borrow_mut()
                        .disconnect_from_peer_id(&id)
                        .await
                    {
                        warn!(target:"MainServiceWorker","disconnect failed: {e}");
                    }
                }
                if let Err(e) = self.db_worker.delete_saved_peer(&ma_str).await {
                    warn!(target:"MainServiceWorker","peer delete failed {}: {e}", ma_str);
                }
            }
        }

        // Record failed + notify + cache
        self.db_worker
            .update_failed_tx(DbTxStateMachine {
                tx_hash: vec![],
                amount: txn_inner.amount.clone(),
                token: txn_inner.token.clone(),
                sender: txn_inner.sender_address.clone(),
                receiver: txn_inner.receiver_address.clone(),
                sender_network: txn_inner.sender_address_network.clone(),
                receiver_network: txn_inner.receiver_address_network.clone(),
                success: false,
            })
            .await?;

        info!(target: "MainServiceWorker", "revert: sending txn to rpc layer");

        if let Some(ttl_wrapper) = self.lru_cache.borrow_mut().get_mut(&txn_inner.tx_nonce.into()) {
            ttl_wrapper.update_value(txn_inner.clone());
            
            self.rpc_sender_channel
                .borrow_mut()
                .send(txn_inner.clone())
                .await?;
        }else{
            error!("Failed to get transaction from cache; expired");
            return Err(anyhow::anyhow!("Failed to get transaction from cache; expired").into());
        }

        Ok(())
    }

    pub async fn handle_public_interface_tx_updates(&mut self) -> Result<(), anyhow::Error> {
        while let Some(txn) = {
            let mut receiver = self.user_rpc_update_recv_channel.borrow_mut();
            receiver.recv().await
        } {
            // handle the incoming transaction per its state
            let status = txn.status.clone();
            match status {
                TxStatus::Genesis => {
                    info!(target:"MainServiceWorker","handling incoming genesis tx updates");
                    debug!(target:"MainServiceWorker","handling incoming genesis tx updates: {:?}",txn.clone());
                    self.handle_genesis_tx_state(Rc::new(RefCell::new(txn.clone())))
                        .await?;
                }

                TxStatus::RecvAddrConfirmed => {
                    info!(target:"MainServiceWorker","handling incoming receiver addr-confirmation tx updates");
                    debug!(target:"MainServiceWorker","handling incoming receiver addr-confirmation tx updates: {:?}",txn.clone());

                    self.handle_recv_addr_confirmed_tx_state(Rc::new(RefCell::new(txn.clone())))
                        .await?;
                }

                TxStatus::NetConfirmed => {
                    todo!()
                }

                TxStatus::SenderConfirmed => {
                    info!(target:"MainServiceWorker","handling incoming sender addr-confirmed tx updates");
                    debug!(target:"MainServiceWorker","handling incoming sender addr-confirmed tx updates: {:?}",txn.clone());

                    self.handle_sender_confirmed_tx_state(Rc::new(RefCell::new(txn.clone())))
                        .await?;
                }

                TxStatus::Reverted(_) => {
                    info!(target:"MainServiceWorker","handling incoming reverted tx updates");
                    debug!(target:"MainServiceWorker","handling incoming reverted tx updates: {:?}",txn.clone());

                    self.handle_reverted_tx_state(Rc::new(RefCell::new(txn.clone())))
                        .await?;
                }

                _ => {}
            };
        }
        Ok(())
    }

    pub async fn run(
        relay_node_multi_addr: String,
        account: String,
        network: String,
        live: bool,
        libp2p_key: String,
        storage: Option<StorageExport>,
    ) -> Result<PublicInterfaceWorker, anyhow::Error> {
        info!("\n🔥 =========== Vane Web3 =========== 🔥\n");

        // ====================================================================================== //
        let main_worker = Self::new(
            relay_node_multi_addr,
            account,
            network,
            live,
            libp2p_key,
            storage,
        )
        .await?;

        // ====================================================================================== //

        // Clone necessary parts to avoid borrow conflicts while keeping concurrent execution
        let user_rpc_update_recv_channel = main_worker.user_rpc_update_recv_channel.clone();
        let rpc_sender_channel = main_worker.rpc_sender_channel.clone();

        let dht_query_result_channel = main_worker.dht_query_result_channel.clone();

        let dht_query_context = main_worker.dht_query_context.clone();
        let lru_cache = main_worker.lru_cache.clone();
        let db_worker = main_worker.db_worker.clone();
        let wasm_tx_processing_worker = main_worker.wasm_tx_processing_worker.clone();
        let p2p_worker = main_worker.p2p_worker.clone();
        let p2p_network_service = main_worker.p2p_network_service.clone();
        let public_interface_worker = main_worker.public_interface_worker.clone();

        let tx_update_future = async move {
            // Create a temporary worker with cloned data for tx updates
            let mut temp_worker = WasmMainServiceWorker {
                user_rpc_update_recv_channel,
                rpc_sender_channel,
                dht_query_result_channel,
                dht_query_context,
                lru_cache,
                db_worker,
                wasm_tx_processing_worker,
                p2p_worker,
                p2p_network_service,
                public_interface_worker,
            };
            temp_worker.handle_public_interface_tx_updates().await
        };

        // Extract public_interface_worker before moving main_worker into futures
        let public_interface_worker = main_worker.public_interface_worker.borrow().clone();

        let swarm_handler_future = async move { main_worker.start_swarm_handler() };

        // Spawn both futures as background tasks instead of using select!
        // This prevents one from being cancelled when the other completes
        // Use wasm_bindgen_futures::spawn_local for WASM compatibility
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(err) = tx_update_future.await {
                error!("tx watch handle error: {err}");
            }
        });

        wasm_bindgen_futures::spawn_local(async move {
            if let Err(err) = swarm_handler_future.await {
                error!("swarm handle error: {err}");
            }
        });

        // In WASM environment, we don't want to block forever
        // Return the public interface worker so JavaScript can interact with it
        // The spawned tasks will continue running in the background

        Ok(public_interface_worker)
    }

    /// Apply storage export data to the database
    async fn apply_storage_export(
        db_worker: Rc<impl DbWorkerInterface>,
        storage_export: StorageExport,
    ) -> Result<(), anyhow::Error> {
        info!("Applying storage export data to database");

        // Apply user account if provided
        if let Some(user_account) = storage_export.user_account {
            db_worker.set_user_account(user_account).await?;
        }

        // Apply nonce - set it to the exported value
        db_worker.set_nonce(storage_export.nonce).await?;

        // Apply saved peers using record_saved_user_peers method
        for saved_peer in storage_export.all_saved_peers {
            for account_id in saved_peer.account_ids {
                db_worker
                    .record_saved_user_peers(account_id, saved_peer.peer_id.clone())
                    .await?;
            }
        }

        // Apply successful transactions
        for tx in storage_export.success_transactions {
            db_worker.update_success_tx(tx).await?;
        }

        // Apply failed transactions
        for tx in storage_export.failed_transactions {
            db_worker.update_failed_tx(tx).await?;
        }

        info!("Successfully applied storage export data");
        Ok(())
    }
}

#[wasm_bindgen]
pub async fn start_vane_web3(
    relay_node_multi_addr: String,
    account: String,
    network: String,
    live: bool,
    libp2p_key: String,
    storage: JsValue,
) -> Result<PublicInterfaceWorkerJs, JsValue> {
    let storage = if storage.is_undefined() || storage.is_null() {
        None
    } else {
        Some(StorageExport::from_js_value_unconditional(storage)?)
    };

    // Initialize WASM logging to forward logs to JavaScript
    if live {
        let _ = crate::logging::init_wasm_logging();
    } else {
        let _ = crate::logging::init_debug_logging();
    }

    log::debug!("Rust debug log test from inside WASM");

    match WasmMainServiceWorker::run(
        relay_node_multi_addr,
        account,
        network,
        live,
        libp2p_key,
        storage,
    )
    .await
    {
        Ok(public_interface_worker) => {
            // Convert the PublicInterfaceWorker to PublicInterfaceWorkerJs and return it
            let js_worker =
                PublicInterfaceWorkerJs::new(Rc::new(RefCell::new(public_interface_worker)));
            Ok(js_worker)
        }
        Err(e) => {
            let error_msg = format!("Failed to start WASM node: {}", e);
            error!("{}", error_msg);
            Err(JsValue::from_str(&error_msg))
        }
    }
}
