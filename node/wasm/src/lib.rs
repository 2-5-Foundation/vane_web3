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

use crate::{
    interface::{PublicInterfaceWorker, PublicInterfaceWorkerJs},
    p2p::{P2pEventNotifSubSystem, P2pNetworkService, WasmP2pWorker},
    tx_processing::WasmTxProcessingWorker,
};

use anyhow::{anyhow, Error};
use codec::Decode;
use db_wasm::{DbWorker, InMemoryDbWorker, OpfsRedbWorker};
use futures::{future, FutureExt};
use gloo_timers::future::TimeoutFuture;
use log::{debug, error, info, warn};
use lru::LruCache;
use primitives::data_structure::{
    BackendEvent, ChainSupported, DbTxStateMachine, DbWorkerInterface, NetworkCommand,
    SignatureType, StorageExport, SwarmMessage, TtlWrapper, TxStateMachine, TxStatus, UserAccount,
    VanePayload,
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
    pub user_rpc_update_recv_channel: Rc<
        RefCell<
            tokio_with_wasm::alias::sync::mpsc::Receiver<
                VanePayload<TxStateMachine, SignatureType>,
            >,
        >,
    >,
    // moka cache
    pub lru_cache: Rc<RefCell<LruCache<u32, TtlWrapper<TxStateMachine>>>>,
}

impl WasmMainServiceWorker {
    pub(crate) async fn new(
        relay_node_multi_addr: String,
        account: String,
        network: String,
        live: bool,
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

        let (p2p_event_notif_tx, p2p_event_notif_recv) =
            tokio_with_wasm::alias::sync::mpsc::channel::<BackendEvent>(20);

        // DATABASE WORKER (LOCAL AND REMOTE )
        // ===================================================================================== //
        let db = DbWorker::initialize_inmemory_db_client("vane.db").await?;

        let db_worker = Rc::new(db);

        // Use bounded cache to prevent memory overflow in WASM environment
        // 10 entries should be sufficient for most use cases while preventing unbounded growth
        let lru_cache: Rc<RefCell<LruCache<u32, TtlWrapper<TxStateMachine>>>> = Rc::new(
            RefCell::new(LruCache::new(std::num::NonZeroUsize::new(10).unwrap())),
        );

        // PEER TO PEER NETWORKING WORKER
        // ===================================================================================== //

        let p2p_event_notif_sub_system = Rc::new(P2pEventNotifSubSystem::new(
            Rc::new(RefCell::new(p2p_event_notif_tx)),
            Rc::new(RefCell::new(p2p_event_notif_recv)),
        ));

        let backend_url = relay_node_multi_addr.clone();

        let p2p_worker = WasmP2pWorker::new(
            live,
            relay_node_multi_addr.clone(),
            account.clone(),
            p2p_command_recv,
            p2p_event_notif_sub_system.clone(),
        )
        .await
        .map_err(|e| anyhow::anyhow!("P2P worker creation failed: {:?}", e))?;

        let p2p_network_service = P2pNetworkService::new(Rc::new(p2p_command_tx))?;

        let user_account = UserAccount {
            multi_addr: backend_url,
            accounts: vec![(account.clone(), ChainSupported::from(network.as_str()))],
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
            p2p_event_notif_sub_system.clone(),
            Rc::new(p2p_worker.clone()),
            Rc::new(p2p_network_service.clone()),
            Rc::new(RefCell::new(rpc_recv_channel)),
            Rc::new(RefCell::new(user_rpc_update_sender_channel)),
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
        })
    }

    pub fn start_swarm_handler(&self, sig: SignatureType) -> Result<(), Error> {
        let (sender_channel, mut recv_channel) = tokio_with_wasm::alias::sync::mpsc::channel(256);

        // Start network worker and get it ready to send messages
        let p2p_worker_clone = self.p2p_worker.clone();
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(e) = p2p_worker_clone
                .borrow()
                .start(sig, Rc::new(RefCell::new(sender_channel)))
                .await
            {
                error!("start network worker failed: {}", e);
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
                SwarmMessage::WasmRequest { data } => {
                    let decoded_req: TxStateMachine = data;

                    debug!(target: "MainServiceWorker", "decoded request: {:?}", &decoded_req);
                    // Use non-blocking try_send for WASM environment
                    if let Err(e) = self
                        .rpc_sender_channel
                        .borrow_mut()
                        .try_send(decoded_req.clone())
                    {
                        error!("Failed to send to RPC channel: {}", e);
                    }

                    let now = (js_sys::Date::now() / 1000.0) as u32;

                    self.lru_cache.borrow_mut().push(
                        decoded_req.tx_nonce.into(),
                        TtlWrapper::new(decoded_req, now),
                    );

                    info!(target: "MainServiceWorker",
                          "propagating txn msg as request");
                }

                SwarmMessage::WasmResponse { data } => {
                    // sender receives the response from the receiver
                    let mut decoded_resp: TxStateMachine = data;

                    info!(target: "MainServiceWorker", "Received response from relay network; tx status: {:?}", decoded_resp.status);
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

                    {
                        let mut cache = self.lru_cache.borrow_mut();
                        if let Some(ttl_wrapper) = cache.get_mut(&decoded_resp.tx_nonce.into()) {
                            ttl_wrapper.update_value(decoded_resp.clone());
                            let ttl_wrapper_clone = ttl_wrapper.clone();
                            drop(cache);
                            self.lru_cache
                                .borrow_mut()
                                .push(decoded_resp.tx_nonce.into(), ttl_wrapper_clone);
                        } else {
                            drop(cache);
                            decoded_resp.status =
                                TxStatus::TxError("Transaction expired".to_string());

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

                SwarmMessage::PendingTransactionsFetched {
                    address,
                    transactions,
                } => {
                    info!(target: "MainServiceWorker", "received pending transactions: {}",transactions.len());
                    // update lru cache with the pending transactions
                    let mut cache = self.lru_cache.borrow_mut();
                    for tx in transactions {
                        if let Some(ttl_wrapper) = cache.get_mut(&tx.tx_nonce.into()) {
                            let existing_tx = ttl_wrapper.get_value();
                            // Don't overwrite call_payload if the existing transaction already has it
                            let tx_to_use = if existing_tx.call_payload.is_some()
                                && tx.call_payload.is_none()
                            {
                                let mut updated_tx = tx.clone();
                                updated_tx.call_payload = existing_tx.call_payload.clone();
                                updated_tx
                            } else {
                                tx
                            };
                            ttl_wrapper.update_value(tx_to_use.clone());
                            let updated_wrapper = ttl_wrapper.clone();
                            cache.push(tx_to_use.tx_nonce.into(), updated_wrapper);
                        } else {
                            let now = (js_sys::Date::now() / 1000.0) as u32;
                            cache.push(tx.tx_nonce.into(), TtlWrapper::new(tx.clone(), now));
                        }
                    }
                    return Ok(());
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
        txn: Rc<RefCell<VanePayload<TxStateMachine, SignatureType>>>,
    ) -> Result<(), Error> {
        let db = self.db_worker.clone();
        let rpc_sender_channel = self.rpc_sender_channel.clone();
        let lru_cache = self.lru_cache.clone();

        let target_user_profile = db.get_user_account().await?;
        let (receiver_in_profile, sender_in_profile) = {
            let vane_payload = txn.borrow();

            let receiver_in_profile = target_user_profile
                .accounts
                .iter()
                .any(|(acc, _)| *acc == vane_payload.data.receiver_address);
            let sender_in_profile = target_user_profile
                .accounts
                .iter()
                .any(|(acc, _)| *acc == vane_payload.data.sender_address);
            (receiver_in_profile, sender_in_profile)
        };

        if receiver_in_profile && sender_in_profile {
            info!(target: "MainServiceWorker", "receiver and sender are in profile");
            let vane_payload = txn.borrow();
            let mut ttl_wrapper = lru_cache
                .borrow_mut()
                .get(&vane_payload.data.tx_nonce.into())
                .ok_or(anyhow::anyhow!(" Failed get transaction from cache"))?
                .clone();
            ttl_wrapper.update_value(vane_payload.data.clone());

            lru_cache
                .borrow_mut()
                .push(vane_payload.data.tx_nonce.into(), ttl_wrapper);

            rpc_sender_channel
                .borrow_mut()
                .send(vane_payload.data.clone())
                .await?;

            return Ok(());
        }

        info!(target: "MainServiceWorker", "Routing transaction through backend transport");

        self.p2p_network_service
            .borrow_mut()
            .wasm_send_request(txn.clone())
            .await?;

        Ok(())
    }

    pub async fn handle_recv_addr_confirmed_tx_state(
        &self,
        txn: Rc<RefCell<VanePayload<TxStateMachine, SignatureType>>>,
    ) -> Result<(), Error> {
        let target_user_profile = self.db_worker.get_user_account().await?;
        let (receiver_in_profile, sender_in_profile) = {
            let vane_payload = txn.borrow();
            let receiver_in_profile = target_user_profile
                .accounts
                .iter()
                .any(|(acc, _)| *acc == vane_payload.data.receiver_address);
            let sender_in_profile = target_user_profile
                .accounts
                .iter()
                .any(|(acc, _)| *acc == vane_payload.data.sender_address);
            (receiver_in_profile, sender_in_profile)
        };

        let is_local_tx = receiver_in_profile && sender_in_profile;

        if !is_local_tx {
            let vane_payload = txn.borrow().clone();
            let mut txn_inner = vane_payload.data.clone();
            self.lru_cache.borrow_mut().pop(&txn_inner.tx_nonce.into());
            info!(target: "MainServiceWorker", "Transaction Status before sending to relay network: {:?}", txn_inner.status);
            info!(target: "MainServiceWorker", "Sending response to relay network");

            self.p2p_network_service
                .borrow_mut()
                .wasm_send_response(Rc::new(RefCell::new(vane_payload)))
                .await?;

            return Ok(());
        }

        let vane_payload = txn.borrow().clone();
        let mut txn_inner = vane_payload.data.clone();
        let validation_result = {
            let tx_processing = self.wasm_tx_processing_worker.borrow();
            tx_processing.validate_receiver_and_sender_address(&txn_inner, "Receiver")
        };

        match validation_result {
            Ok(_) => {
                txn_inner.recv_confirmation_passed();
                txn_inner.increment_version();
                info!(target: "MainServiceWorker", "receiver confirmation passed");
            }
            Err(err) => {
                txn_inner.recv_confirmation_failed();
                txn_inner.increment_version();
                error!(target: "MainServiceWorker",
                  "receiver confirmation failed: {err}");
            }
        }

        if let Err(e) = self
            .rpc_sender_channel
            .borrow_mut()
            .try_send(txn_inner.clone())
        {
            error!("Failed to send response to RPC channel: {}", e);
            return Err(e.into());
        }

        if let Some(ttl_wrapper) = self
            .lru_cache
            .borrow_mut()
            .get_mut(&txn_inner.tx_nonce.into())
        {
            ttl_wrapper.update_value(txn_inner.clone());
        } else {
            error!("Failed to get transaction from cache; expired");
            return Err(anyhow::anyhow!("Failed to get transaction from cache; expired").into());
        }

        info!(target: "MainServiceWorker",
                      "propagating txn msg as response to rpc layer for user interaction: {txn_inner:?}");

        Ok(())
    }

    pub async fn handle_sender_confirmed_tx_state(
        &self,
        txn: Rc<RefCell<VanePayload<TxStateMachine, SignatureType>>>,
    ) -> Result<(), Error> {
        let vane_payload = txn.borrow();
        let mut txn_inner = vane_payload.data.clone();
        info!(target: "MainServiceWorker","sender confirmed tx state: {txn_inner:?}");

        // If the receiver is remote, notify backend to handle confirmation
        let prof = self.db_worker.get_user_account().await?;
        let both_in_profile = prof
            .accounts
            .iter()
            .any(|(a, _)| *a == txn_inner.receiver_address)
            && prof
                .accounts
                .iter()
                .any(|(a, _)| *a == txn_inner.sender_address);

        // here there should be 2 paths (mitigating the case where we cant just sign the raw tx on client side)
        // 1. When the tx is already been sent
        // 2. when the tx will be submitted on vane wasm app

        //-------------------------------------------------------------------------------------

        //------------------ A CASE WHERE THE TX IS ALREADY BEEN SENT -------------------------

        //-------------------------------------------------------------------------------------

        if let TxStatus::FailedToSubmitTxn(ref reason) = txn_inner.status {
            error!(target: "MainServiceWorker","tx submission failed: {reason}");
            // update local db on success tx
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

            {
                let mut cache = self.lru_cache.borrow_mut();

                if let Some(ttl) = cache.get_mut(&txn_inner.tx_nonce.into()) {
                    ttl.update_value(txn_inner.clone());
                    let ttl_clone = ttl.clone();
                    drop(cache);

                    self.lru_cache
                        .borrow_mut()
                        .push(txn_inner.tx_nonce.into(), ttl_clone);
                } else {
                    drop(cache);
                    let now = (js_sys::Date::now() / 1000.0) as u32;
                    self.lru_cache.borrow_mut().push(
                        txn_inner.tx_nonce.into(),
                        TtlWrapper::new(txn_inner.clone(), now),
                    );
                }
            }

            self.rpc_sender_channel
                .borrow_mut()
                .send(txn_inner.clone())
                .await?;

            return Ok(());
        }

        if let TxStatus::TxSubmissionPassed { hash: ref tx_hash } = txn_inner.status {
            info!(target: "MainServiceWorker","tx already submitted: {tx_hash:?}");
            // update local db on success tx
            let db_tx = DbTxStateMachine {
                tx_hash: tx_hash.clone(),
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
            {
                let mut cache = self.lru_cache.borrow_mut();

                if let Some(ttl) = cache.get_mut(&txn_inner.tx_nonce.into()) {
                    ttl.update_value(txn_inner.clone());
                    let ttl_clone = ttl.clone();
                    drop(cache);

                    self.lru_cache
                        .borrow_mut()
                        .push(txn_inner.tx_nonce.into(), ttl_clone);
                } else {
                    drop(cache);
                    let now = (js_sys::Date::now() / 1000.0) as u32;
                    self.lru_cache.borrow_mut().push(
                        txn_inner.tx_nonce.into(),
                        TtlWrapper::new(txn_inner.clone(), now),
                    );
                }
            }

            self.rpc_sender_channel
                .borrow_mut()
                .send(txn_inner.clone())
                .await?;
            return Ok(());
        }

        //-------------------------------------------------------------------------------------

        //----------------------------- A NORMAL SITUATION HERE -------------------------------

        //-------------------------------------------------------------------------------------

        // verify receiver again
        self.wasm_tx_processing_worker
            .borrow()
            .validate_receiver_and_sender_address(&txn_inner, "Receiver")?;

        // verify sender
        if let Err(e) = self
            .wasm_tx_processing_worker
            .borrow()
            .validate_receiver_and_sender_address(&txn_inner, "Sender")
        {
            error!(target: "MainServiceWorker","sender confirmation failed: {e}");
            txn_inner.sender_confirmation_failed();

            let confirm_command = NetworkCommand::ConfirmTransaction {
                sig: vane_payload.extra_data.clone(),
                account_id: txn_inner.sender_address.clone(),
                data: txn_inner.clone(),
            };
            let command_tx = self.p2p_network_service.borrow().p2p_command_tx.clone();
            command_tx.send(confirm_command).await?;

            let now = (js_sys::Date::now() / 1000.0) as u32;
            self.lru_cache.borrow_mut().push(
                txn_inner.tx_nonce.into(),
                TtlWrapper::new(txn_inner.clone(), now),
            );

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
                    txn_inner.tx_submission_passed(tx_hash.clone());
                    info!(target: "MainServiceWorker","tx submission passed");

                    // update local db on success tx
                    let db_tx = DbTxStateMachine {
                        tx_hash: tx_hash.clone(),
                        amount: txn_inner.amount.clone(),
                        token: txn_inner.token.clone(),
                        sender: txn_inner.sender_address.clone(),
                        receiver: txn_inner.receiver_address.clone(),
                        sender_network: txn_inner.sender_address_network.clone(),
                        receiver_network: txn_inner.receiver_address_network.clone(),
                        success: true,
                    };
                    self.db_worker.update_success_tx(db_tx).await?;

                    let submission_update_cmd = NetworkCommand::TxSubmissionUpdate {
                        sig: vane_payload.extra_data.clone(),
                        account_id: txn_inner.sender_address.clone(),
                        data: txn_inner.clone(),
                    };
                    let command_tx = self.p2p_network_service.borrow().p2p_command_tx.clone();
                    command_tx.send(submission_update_cmd).await?;

                    info!(target: "MainServiceWorker","Db recorded success tx");

                    if let Some(ttl_wrapper) = self
                        .lru_cache
                        .borrow_mut()
                        .get_mut(&txn_inner.tx_nonce.into())
                    {
                        ttl_wrapper.update_value(txn_inner.clone());

                        self.rpc_sender_channel
                            .borrow_mut()
                            .send(txn_inner.clone())
                            .await?;
                    } else {
                        let now = (js_sys::Date::now() / 1000.0) as u32;
                        self.lru_cache.borrow_mut().push(
                            txn_inner.tx_nonce.into(),
                            TtlWrapper::new(txn_inner.clone(), now),
                        );
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

                    let submission_update_cmd = NetworkCommand::TxSubmissionUpdate {
                        sig: vane_payload.extra_data.clone(),
                        account_id: txn_inner.sender_address.clone(),
                        data: txn_inner.clone(),
                    };
                    let command_tx = self.p2p_network_service.borrow().p2p_command_tx.clone();
                    command_tx.send(submission_update_cmd).await?;

                    if let Some(ttl_wrapper) = self
                        .lru_cache
                        .borrow_mut()
                        .get_mut(&txn_inner.tx_nonce.into())
                    {
                        ttl_wrapper.update_value(txn_inner.clone());

                        self.rpc_sender_channel
                            .borrow_mut()
                            .send(txn_inner.clone())
                            .await?;
                    } else {
                        let now = (js_sys::Date::now() / 1000.0) as u32;
                        self.lru_cache.borrow_mut().push(
                            txn_inner.tx_nonce.into(),
                            TtlWrapper::new(txn_inner.clone(), now),
                        );
                        self.rpc_sender_channel
                            .borrow_mut()
                            .send(txn_inner.clone())
                            .await?;
                    }
                }
            }
        } else {
            // non original sender confirmed, return error, send to rpc
            txn_inner.status =
                TxStatus::TxError("failed to match original sender and receiver".to_string());
            error!(target: "MainServiceWorker","Non original sender or receiver signed");

            let confirm_command = NetworkCommand::ConfirmTransaction {
                sig: vane_payload.extra_data.clone(),
                account_id: txn_inner.sender_address.clone(),
                data: txn_inner.clone(),
            };
            let command_tx = self.p2p_network_service.borrow().p2p_command_tx.clone();
            command_tx.send(confirm_command).await?;

            if let Some(ttl_wrapper) = self
                .lru_cache
                .borrow_mut()
                .get_mut(&txn_inner.tx_nonce.into())
            {
                ttl_wrapper.update_value(txn_inner.clone());

                self.rpc_sender_channel
                    .borrow_mut()
                    .send(txn_inner.clone())
                    .await?;
            } else {
                let now = (js_sys::Date::now() / 1000.0) as u32;
                self.lru_cache.borrow_mut().push(
                    txn_inner.tx_nonce.into(),
                    TtlWrapper::new(txn_inner.clone(), now),
                );
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
        txn: Rc<RefCell<VanePayload<TxStateMachine, SignatureType>>>,
    ) -> Result<(), Error> {
        let vane_payload = txn.borrow();
        let txn_inner = vane_payload.data.clone();
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

        // Backend transport handles cleanup for remote peers now as part of revertation.

        if !both_in_profile {
            let revert_command = NetworkCommand::RevertTransaction {
                sig: vane_payload.extra_data.clone(),
                account_id: txn_inner.sender_address.clone(),
                data: txn_inner.clone(),
            };
            let command_tx = self.p2p_network_service.borrow().p2p_command_tx.clone();
            command_tx.send(revert_command).await?;
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

        {
            let mut cache = self.lru_cache.borrow_mut();
            if let Some(ttl_wrapper) = cache.get_mut(&txn_inner.tx_nonce.into()) {
                ttl_wrapper.update_value(txn_inner.clone());
                let updated_wrapper = ttl_wrapper.clone();
                drop(cache);
                self.lru_cache
                    .borrow_mut()
                    .push(txn_inner.tx_nonce.into(), updated_wrapper);
            } else {
                let now = (js_sys::Date::now() / 1000.0) as u32;
                self.lru_cache.borrow_mut().push(
                    txn_inner.tx_nonce.into(),
                    TtlWrapper::new(txn_inner.clone(), now),
                );
            }
        }

        self.rpc_sender_channel
            .borrow_mut()
            .send(txn_inner.clone())
            .await?;

        Ok(())
    }

    pub async fn handle_public_interface_tx_updates(&mut self) -> Result<(), anyhow::Error> {
        while let Some(vane_payload) = {
            let mut receiver = self.user_rpc_update_recv_channel.borrow_mut();
            receiver.recv().await
        } {
            // handle the incoming transaction per its state
            let status = vane_payload.data.status.clone();
            match status {
                TxStatus::Genesis => {
                    info!(target:"MainServiceWorker","handling incoming genesis tx updates");
                    debug!(target:"MainServiceWorker","handling incoming genesis tx updates: {:?}",vane_payload.clone());
                    self.handle_genesis_tx_state(Rc::new(RefCell::new(vane_payload.clone())))
                        .await?;
                }

                TxStatus::RecvAddrConfirmed => {
                    info!(target:"MainServiceWorker","handling incoming receiver addr-confirmation tx updates");
                    debug!(target:"MainServiceWorker","handling incoming receiver addr-confirmation tx updates: {:?}",vane_payload.clone());

                    self.handle_recv_addr_confirmed_tx_state(Rc::new(RefCell::new(
                        vane_payload.clone(),
                    )))
                    .await?;
                }

                TxStatus::NetConfirmed => {
                    todo!()
                }

                TxStatus::SenderConfirmed
                | TxStatus::FailedToSubmitTxn(_)
                | TxStatus::TxSubmissionPassed { hash: _ } => {
                    info!(target:"MainServiceWorker","handling incoming sender addr-confirmed tx updates");
                    debug!(target:"MainServiceWorker","handling incoming sender addr-confirmed tx updates: {:?}",vane_payload.clone());

                    self.handle_sender_confirmed_tx_state(Rc::new(RefCell::new(
                        vane_payload.clone(),
                    )))
                    .await?;
                }

                TxStatus::Reverted(_) => {
                    info!(target:"MainServiceWorker","handling incoming reverted tx updates");
                    debug!(target:"MainServiceWorker","handling incoming reverted tx updates: {:?}",vane_payload.clone());

                    self.handle_reverted_tx_state(Rc::new(RefCell::new(vane_payload.clone())))
                        .await?;
                }

                _ => {}
            };
        }
        Ok(())
    }

    pub async fn run(
        sig: SignatureType,
        relay_node_multi_addr: String,
        account: String,
        network: String,
        live: bool,
        self_node: bool,
        storage: Option<StorageExport>,
    ) -> Result<PublicInterfaceWorker, anyhow::Error> {
        info!("\n🔥 =========== Vane Web3 =========== 🔥\n");

        // ====================================================================================== //
        let main_worker = Self::new(relay_node_multi_addr, account, network, live, storage).await?;

        // ====================================================================================== //

        // Clone necessary parts to avoid borrow conflicts while keeping concurrent execution
        let user_rpc_update_recv_channel = main_worker.user_rpc_update_recv_channel.clone();
        let rpc_sender_channel = main_worker.rpc_sender_channel.clone();

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

        // Spawn both futures as background tasks instead of using select!
        // This prevents one from being cancelled when the other completes
        // Use wasm_bindgen_futures::spawn_local for WASM compatibility
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(err) = tx_update_future.await {
                error!("tx watch handle error: {err}");
            }
        });

        if !self_node {
            let swarm_handler_future = async move { main_worker.start_swarm_handler(sig) };

            wasm_bindgen_futures::spawn_local(async move {
                if let Err(err) = swarm_handler_future.await {
                    error!("swarm handle error: {err}");
                }
            });
        }

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
    sig: SignatureType,
    relay_node_multi_addr: String,
    account: String,
    network: String,
    self_node: bool, // as running this node to send transactions to self
    live: bool,
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
        sig,
        relay_node_multi_addr,
        account,
        network,
        live,
        self_node,
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
