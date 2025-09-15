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
use log::{error, info, warn};
use lru::LruCache;
use primitives::data_structure::{
    ChainSupported, DbTxStateMachine, DbWorkerInterface, HashId, NetworkCommand, SwarmMessage,
    TxStateMachine, TxStatus, UserAccount,
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
    pub lru_cache: RefCell<LruCache<u64, TxStateMachine>>,
}

impl WasmMainServiceWorker {
    pub(crate) async fn new(
        relay_node_multi_addr: String,
        account: String,
        network: String,
        live: bool,
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
        let db = if live {
            DbWorker::initialize_opfs_db_client("vane.db").await?
        } else {
            DbWorker::initialize_inmemory_db_client("vane.db").await?
        };

        let db_worker = Rc::new(db);

        // Use bounded cache to prevent memory overflow in WASM environment
        // 10,000 entries should be sufficient for most use cases while preventing unbounded growth
        let lru_cache: LruCache<u64, TxStateMachine> =
            LruCache::new(std::num::NonZeroUsize::new(10).unwrap());

        // PEER TO PEER NETWORKING WORKER
        // ===================================================================================== //

        let p2p_worker = WasmP2pWorker::new(
            db_worker.clone(),
            relay_node_multi_addr,
            account.clone(),
            p2p_command_recv,
            dht_query_result_tx,
        )
        .await
        .map_err(|e| anyhow::anyhow!("P2P worker creation failed: {:?}", e))?;

        let p2p_network_service =
            P2pNetworkService::new(Rc::new(p2p_command_tx), p2p_worker.clone())?;

        let user_account = UserAccount {
            multi_addr: p2p_worker.user_circuit_multi_addr.to_string(),
            accounts: vec![(account, ChainSupported::from(network.as_str()))],
        };
        db_worker.set_user_account(user_account).await?;

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

        let wasm_tx_processing_worker = WasmTxProcessingWorker::new((
            ChainSupported::Bnb,
            ChainSupported::Ethereum,
            ChainSupported::Solana,
        ))?;
        // ===================================================================================== //

        Ok(Self {
            db_worker,
            public_interface_worker: Rc::new(RefCell::new(public_interface_worker)),
            wasm_tx_processing_worker: Rc::new(RefCell::new(wasm_tx_processing_worker)),
            p2p_worker: Rc::new(RefCell::new(p2p_worker)),
            p2p_network_service: Rc::new(RefCell::new(p2p_network_service)),
            rpc_sender_channel: Rc::new(RefCell::new(rpc_sender_channel)),
            user_rpc_update_recv_channel: Rc::new(RefCell::new(user_rpc_update_recv_channel)),
            lru_cache: RefCell::new(lru_cache),
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
            while let Some(swarm_msg_result) = recv_channel.recv().await {
                // Process directly in this task
                if let Err(e) = self_clone
                    .handle_swarm_message(swarm_msg_result, &tx_worker)
                    .await
                {
                    error!("Error processing swarm message: {}", e);
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
                SwarmMessage::WasmRequest { data, inbound_id } => {
                    let mut decoded_req: TxStateMachine = data;
                    let inbound_req_id = inbound_id.get_hash_id();
                    decoded_req.inbound_req_id = Some(inbound_req_id);

                    // Use non-blocking try_send for WASM environment
                    if let Err(e) = self
                        .rpc_sender_channel
                        .borrow_mut()
                        .try_send(decoded_req.clone())
                    {
                        error!("Failed to send to RPC channel: {}", e);
                        return Err(e.into());
                    }

                    self.lru_cache
                        .borrow_mut()
                        .push(decoded_req.tx_nonce.into(), decoded_req.clone());

                    info!(target: "MainServiceWorker",
                          "propagating txn msg as request: {decoded_req:?}");
                }

                SwarmMessage::WasmResponse { data, outbound_id } => {
                    let mut decoded_resp: TxStateMachine = data;
                    let outbound_req_id = outbound_id.get_hash_id();
                    decoded_resp.outbound_req_id = Some(outbound_req_id);

                    match txn_processing_worker
                        .validate_receiver_sender_address(&decoded_resp, "Receiver")
                    {
                        Ok(_) => {
                            decoded_resp.recv_confirmation_passed();
                            info!(target: "MainServiceWorker", "receiver confirmation passed");

                            let mut tx_processing = self.wasm_tx_processing_worker.borrow_mut();
                            tx_processing.create_tx(&mut decoded_resp).await?;
                        }
                        Err(err) => {
                            decoded_resp.recv_confirmation_failed();
                            error!(target: "MainServiceWorker",
                                  "receiver confirmation failed: {err}");

                            let db_tx = DbTxStateMachine {
                                tx_hash: vec![],
                                amount: decoded_resp.amount,
                                network: decoded_resp.network,
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
                        error!("Failed to send response to RPC channel: {}", e);
                        return Err(e.into());
                    }

                    self.lru_cache
                        .borrow_mut()
                        .push(decoded_resp.tx_nonce.into(), decoded_resp.clone());

                    info!(target: "MainServiceWorker",
                          "propagating txn msg as response: {decoded_resp:?}");
                }
                _ => {}
            },
            Err(err) => {
                info!("Error in swarm message: {err}");
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
        let rpc_sender_channel = self.rpc_sender_channel.clone();
        let lru_cache = self.lru_cache.clone();

        // 1) try local DB first
        match db.get_saved_user_peers(target_id.clone()).await {
            Ok(addr_str) => {
                let multi_addr = addr_str
                    .parse::<Multiaddr>()
                    .map_err(|e| anyhow::anyhow!("failed to parse multiaddr: {e}"))?;

                let peer_id = match multi_addr.clone().pop() {
                    Some(Protocol::P2p(id)) => id,
                    _ => return Err(anyhow::anyhow!("peer id not found")),
                };

                p2p_network_service
                    .borrow_mut()
                    .dial_to_peer_id(multi_addr.clone(), &peer_id)
                    .await?;

                p2p_network_service
                    .borrow_mut()
                    .wasm_send_request(txn.clone(), peer_id, multi_addr)
                    .await?;

                return Ok(());
            }

            Err(_) => {
                // 2) DB miss → spawn DHT fallback and return immediately
                info!(target: "MainServiceWorker", "DB miss, spawning DHT fallback");
                let p2p_network_service = p2p_network_service.clone();
                let rpc_sender_channel = rpc_sender_channel.clone();
                let lru_cache = lru_cache.clone();
                let txn = txn.clone();
                let target_id = target_id.clone();

                wasm_bindgen_futures::spawn_local(async move {
                    info!("🔍 Starting DHT lookup for target: {}", target_id);
                    let dht = host_get_dht(target_id.clone()).fuse();
                    let timeout = TimeoutFuture::new(30_000).fuse(); // 15s
                    futures::pin_mut!(dht, timeout);

                    match future::select(dht, timeout).await {
                        // DHT returned
                        future::Either::Left((Ok(resp), _)) => {
                            if let Some(err) = resp.error {
                                error!("DHT failed: {}", err);

                                let mut t = txn.borrow_mut().clone();
                                t.recv_not_registered();

                                let _ = rpc_sender_channel.borrow_mut().send(t.clone()).await;
                                lru_cache.borrow_mut().push(t.tx_nonce.into(), t);
                                return;
                            }

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

                                if let Err(e) = p2p_network_service
                                    .borrow_mut()
                                    .dial_to_peer_id(multi_addr.clone(), &peer_id)
                                    .await
                                {
                                    error!("dial_to_peer_id failed: {e:?}");
                                    return;
                                }

                                TimeoutFuture::new(2_000).await;

                                if let Err(e) = p2p_network_service
                                    .borrow_mut()
                                    .wasm_send_request(txn, peer_id, multi_addr)
                                    .await
                                {
                                    error!("wasm_send_request failed: {e:?}");
                                    return;
                                }
                            } else {
                                warn!("DHT returned no address for {}", target_id);

                                let mut t = txn.borrow_mut().clone();
                                t.recv_not_registered();

                                let _ = rpc_sender_channel.borrow_mut().send(t.clone()).await;
                                lru_cache.borrow_mut().push(t.tx_nonce.into(), t);
                            }
                        }

                        // DHT internal error
                        future::Either::Left((Err(e), _)) => {
                            error!("host_get_dht internal error: {e:?}");

                            let mut t = txn.borrow_mut().clone();
                            t.recv_not_registered();

                            let _ = rpc_sender_channel.borrow_mut().send(t.clone()).await;
                            lru_cache.borrow_mut().push(t.tx_nonce.into(), t);
                        }

                        // timeout
                        future::Either::Right((_elapsed, _)) => {
                            warn!("⏳ DHT timed out for {}", target_id);

                            let mut t = txn.borrow_mut().clone();
                            t.recv_not_registered();

                            let _ = rpc_sender_channel.borrow_mut().send(t.clone()).await;
                            lru_cache.borrow_mut().push(t.tx_nonce.into(), t);
                        }
                    }
                });

                return Ok(());
            }
        }
    }

    pub async fn handle_recv_addr_confirmed_tx_state(
        &self,
        id: u64,
        txn: Rc<RefCell<TxStateMachine>>,
    ) -> Result<(), Error> {
        self.p2p_network_service
            .borrow_mut()
            .wasm_send_response(id, txn)
            .await?;
        Ok(())
    }

    pub async fn handle_sender_confirmed_tx_state(
        &self,
        txn: Rc<RefCell<TxStateMachine>>,
    ) -> Result<(), Error> {
        let mut txn_inner = txn.borrow_mut().clone();

        // verify sender
        self.wasm_tx_processing_worker
            .borrow()
            .validate_receiver_sender_address(&txn_inner, "Sender")?;
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
                    self.rpc_sender_channel
                        .borrow_mut()
                        .send(txn_inner.clone())
                        .await?;
                    // update local db on success tx
                    let db_tx = DbTxStateMachine {
                        tx_hash: tx_hash.to_vec(),
                        amount: txn_inner.amount.clone(),
                        network: txn_inner.network.clone(),
                        success: true,
                    };
                    self.db_worker.update_success_tx(db_tx).await?;
                }
                Err(err) => {
                    txn_inner.tx_submission_failed(format!(
                        "{err:?}: the tx will be resubmitted rest assured"
                    ));
                    self.rpc_sender_channel.borrow_mut().send(txn_inner).await?;
                }
            }
        } else {
            // non original sender confirmed, return error, send to rpc
            txn_inner.sender_confirmation_failed();
            error!(target: "MainServiceWorker","Non original sender signed");
            self.rpc_sender_channel.borrow_mut().send(txn_inner).await?;
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
                    info!(target:"MainServiceWorker","handling incoming genesis tx updates: {:?} \n",txn.clone());
                    self.handle_genesis_tx_state(Rc::new(RefCell::new(txn.clone())))
                        .await?;
                }

                TxStatus::RecvAddrConfirmed => {
                    info!(target:"MainServiceWorker","handling incoming receiver addr-confirmation tx updates: {:?} \n",txn.clone());

                    let inbound_id = txn.inbound_req_id.expect("no inbound req id found");
                    self.handle_recv_addr_confirmed_tx_state(
                        inbound_id,
                        Rc::new(RefCell::new(txn.clone())),
                    )
                    .await?;
                }

                TxStatus::NetConfirmed => {
                    todo!()
                }

                TxStatus::SenderConfirmed => {
                    info!(target:"MainServiceWorker","handling incoming sender addr-confirmed tx updates: {:?} \n",txn.clone());

                    self.handle_sender_confirmed_tx_state(Rc::new(RefCell::new(txn.clone())))
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
    ) -> Result<PublicInterfaceWorker, anyhow::Error> {
        info!("\n🔥 =========== Vane Web3 =========== 🔥\n");

        // ====================================================================================== //
        let main_worker = Self::new(relay_node_multi_addr, account, network, live).await?;

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
}

#[wasm_bindgen]
pub async fn start_vane_web3(
    relay_node_multi_addr: String,
    account: String,
    network: String,
    live: bool,
) -> Result<PublicInterfaceWorkerJs, JsValue> {
    // Set up panic hook for better error reporting
    console_error_panic_hook::set_once();

    // Initialize WASM logging to forward logs to JavaScript
    if live {
        let _ = crate::logging::init_wasm_logging();
    } else {
        let _ = crate::logging::init_debug_logging();
    }

    match WasmMainServiceWorker::run(relay_node_multi_addr, account, network, live).await {
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
