#![no_std]
extern crate alloc;
extern crate core;

pub mod interface;
pub mod p2p;
pub mod tx_processing;
pub mod cryptography;

use crate::p2p::P2pNetworkService;

use alloc::vec;
use alloc::sync::Arc;
use anyhow::{anyhow, Error};
use codec::Decode;
use core::str::FromStr;
use hex_literal::hex;
use primitives::data_structure::DbWorkerInterface;

// -------------------------------- WASM ------------------------------ //
use lib_wasm_imports::*;

mod lib_wasm_imports {
    pub use crate::p2p::WasmP2pWorker;
    pub use crate::interface::PublicInterfaceWorker;
    pub use crate::tx_processing::WasmTxProcessingWorker;
    pub use alloc::rc::Rc;
    pub use core::cell::RefCell;
    pub use db_wasm::OpfsRedbWorker;
    pub use futures::FutureExt;
    pub use lru::LruCache;
    pub use wasm_bindgen::prelude::wasm_bindgen;
    pub use primitives::data_structure::DbWorkerInterface;
    pub use primitives::data_structure::{ChainSupported, TxStateMachine, TxStatus, DbTxStateMachine, SwarmMessage};
    pub use primitives::data_structure::HashId;
    pub use alloc::sync::Arc;
    pub use anyhow::{anyhow, Error};
    pub use codec::Decode;
    pub use core::str::FromStr;
    pub use log::{error, info, warn};
    pub use alloc::format;
    pub use alloc::string::String;
    pub use primitives::data_structure::NetworkCommand;
    pub use libp2p::Multiaddr;
    pub use libp2p::PeerId;
    pub use wasm_bindgen::JsValue;
}


#[derive(Clone)]
pub struct WasmMainServiceWorker {
    pub db_worker: Rc<OpfsRedbWorker>,
    pub public_interface_worker: Rc<RefCell<PublicInterfaceWorker>>,
    pub wasm_tx_processing_worker: Rc<RefCell<WasmTxProcessingWorker>>,
    // for swarm events
    pub p2p_worker: Rc<RefCell<WasmP2pWorker>>,
    //telemetry_worker: TelemetryWorker,
    pub p2p_network_service: Rc<RefCell<P2pNetworkService>>,
    // channels for layers communication
    /// sender channel to propagate transaction state to rpc layer
    /// this serve as an update channel to the user
    pub rpc_sender_channel: Rc<RefCell<tokio_with_wasm::sync::mpsc::Sender<TxStateMachine>>>,
    /// receiver channel to handle the updates made by user from rpc
    pub user_rpc_update_recv_channel:
        Rc<RefCell<tokio_with_wasm::sync::mpsc::Receiver<TxStateMachine>>>,
    // moka cache
    pub lru_cache: RefCell<LruCache<u64, TxStateMachine>>,
}

impl WasmMainServiceWorker {
    pub(crate) async fn new(db_url_path: Option<String>, p2p_port: u16, dns: String) -> Result<Self, anyhow::Error> {
        // CHANNELS
        // ===================================================================================== //
        // for rpc messages back and forth propagation
        let (rpc_sender_channel, rpc_recv_channel) = tokio_with_wasm::sync::mpsc::channel(10);
        let (user_rpc_update_sender_channel, user_rpc_update_recv_channel) =
            tokio_with_wasm::sync::mpsc::channel(10);

        // for p2p network commands
        let (p2p_command_tx, p2p_command_recv) =
            tokio_with_wasm::sync::mpsc::channel::<NetworkCommand>(10);

        // DATABASE WORKER (LOCAL AND REMOTE )
        // ===================================================================================== //
        let mut db_url = String::new();
        if let Some(url) = db_url_path {
            db_url = url
        } else {
            db_url = String::from("db/dev.db")
        }
        let db = OpfsRedbWorker::initialize_db_client(db_url.as_str()).await?;

        let mut p2p_port: u16 = 0;

        let returned_pots = db.get_ports().await?;
        if let Some(ports) = returned_pots {
            p2p_port = ports.p_2_p_port as u16;
        } else {
            p2p_port = p2p_port
        }

        let db_worker = Rc::new(db);

        let lru_cache: LruCache<u64, TxStateMachine> = LruCache::unbounded();

        // PEER TO PEER NETWORKING WORKER
        // ===================================================================================== //

        let p2p_worker = WasmP2pWorker::new(
            db_worker.clone(),
            p2p_port,
            dns,
            p2p_command_recv,
        )
        .await.map_err(|e| anyhow::anyhow!("P2P worker creation failed: {:?}", e))?;

        let p2p_network_service =
            P2pNetworkService::new(Rc::new(p2p_command_tx), p2p_worker.clone())?;

        // TRANSACTION RPC WORKER / PUBLIC INTERFACE
        // ===================================================================================== //

        let public_interface_worker = PublicInterfaceWorker::new(
            db_worker.clone(),
            Rc::new(RefCell::new(rpc_recv_channel)),
            Rc::new(RefCell::new(user_rpc_update_sender_channel)),
            p2p_worker.node_id,
            lru_cache.clone(),
        )
        .await.map_err(|e| anyhow::anyhow!("Public interface worker creation failed: {:?}", e))?;

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
        })
    }

    pub fn start_swarm_handler(&self) -> Result<(), Error> {
        let (sender_channel, mut recv_channel) = tokio_with_wasm::sync::mpsc::channel(256);

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
        &self,
        txn: Rc<RefCell<TxStateMachine>>,
    ) -> Result<(), Error> {
        // dial to target peer id from tx receiver
        let target_id = txn.borrow().receiver_address.clone();

        // check if the acc is present in local db
        // First try local DB
        let target_peer_result = {
            // Release DB lock immediately after query
            self.db_worker.get_saved_user_peers(target_id.clone()).await
        };

        match target_peer_result {
            Ok(acc) => {
                info!(target:"MainServiceWorker","target peer found in local db");
                // dial the target
                let multi_addr = acc
                    .multi_addr
                    .expect("multi addr is not found")
                    .parse::<Multiaddr>()?;

                let peer_id = PeerId::from_str(&acc.peer_id.expect("peer id not found"))?;

                // ========================================================================= //
                self.p2p_network_service
                    .borrow_mut()
                    .dial_to_peer_id(multi_addr.clone(), &peer_id)
                    .await?;

                self.p2p_network_service
                    .borrow_mut()
                    .wasm_send_request(txn.clone(), peer_id, multi_addr)
                    .await?;
            }
            Err(_err) => {
                // fetch from DHT


                // info!(target:"MainServiceWorker","target peer not found in local db, fetching from remote db");

                // let acc_ids = self.airtable_client.list_all_peers().await?;

                // let target_id_addr = txn.borrow().receiver_address.clone();

                // if !acc_ids.is_empty() {
                //     let result_peer = acc_ids.into_iter().find_map(|discovery| {
                //         match discovery
                //             .clone()
                //             .account_ids
                //             .into_iter()
                //             .find(|addr| addr == &target_id_addr)
                //         {
                //             Some(_) => {
                //                 let peer_record: PeerRecord = discovery.clone().into();
                //                 Some((discovery.peer_id, discovery.multi_addr, peer_record))
                //             }
                //             None => None,
                //         }
                //     });

                //     if result_peer.is_some() {
                //         // dial the target
                //         info!(target:"MainServiceWorker","target peer found in remote db: {result_peer:?} \n");
                //         let multi_addr = result_peer
                //             .clone()
                //             .expect("failed to get multi addr")
                //             .1
                //             .unwrap()
                //             .parse::<Multiaddr>()
                //             .map_err(|err| {
                //                 anyhow!("failed to parse multi addr, caused by: {err}")
                //             })?;
                //         let peer_id = PeerId::from_str(
                //             &*result_peer
                //                 .clone()
                //                 .expect("failed to parse peer id")
                //                 .0
                //                 .expect("failed to parse peerId"),
                //         )?;

                //         // save the target peer id to local db
                //         let peer_record = result_peer.clone().unwrap().2;
                //         info!(target: "MainServiceWorker","recording target peer id to local db");

                //         // ========================================================================= //
                //         self.db_worker.record_saved_user_peers(peer_record).await?;

                //         // ========================================================================= //
                //         self.p2p_network_service
                //             .borrow_mut()
                //             .dial_to_peer_id(multi_addr.clone(), &peer_id)
                //             .await?;
                //         self.p2p_network_service
                //             .borrow_mut()
                //             .wasm_send_request(txn.clone(), peer_id, multi_addr)
                //             .await?;
                //     } else {
                //         // return tx state as error on sender rpc
                //         let mut txn = txn.borrow_mut().clone();
                //         txn.recv_not_registered();
                //         self.rpc_sender_channel
                //             .borrow_mut()
                //             .send(txn.clone())
                //             .await?;
                //         self.lru_cache.borrow_mut().push(txn.tx_nonce.into(), txn);

                //         error!(target: "MainServiceWorker","target peer not found in remote db,tell the user is missing out on safety transaction");
                //     }
                // }
            }
        }
        Ok(())
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
        // verify multi id
        if self
            .wasm_tx_processing_worker
            .borrow()
            .validate_multi_id(&txn_inner)
        {
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

    pub async fn handle_public_interface_tx_updates(&self) -> Result<(), anyhow::Error> {
        while let Some(txn) = self.user_rpc_update_recv_channel.borrow_mut().recv().await {
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

    pub async fn run(db_url: Option<String>, p2p_port: u16, dns: String) -> Result<PublicInterfaceWorker, anyhow::Error> {
        info!(
            "\n🔥 =========== Vane Web3 =========== 🔥\n\
             A safety layer for web3 transactions, allows you to feel secure when sending and receiving \n\
             tokens without the fear of selecting the wrong address or network. \n\
             It provides a safety net, giving you room to make mistakes without losing all your funds.\n"
        );

        // ====================================================================================== //
        let main_worker = Self::new(db_url, p2p_port, dns).await?;

        // ====================================================================================== //

        let p2p_worker = main_worker.p2p_worker.clone();
        let txn_processing_worker = main_worker.wasm_tx_processing_worker.borrow_mut().clone();

        // ====================================================================================== //

        futures::select! {
            tx_watch_result = main_worker.handle_public_interface_tx_updates().fuse() => {
                if let Err(err) = tx_watch_result {
                    error!("tx watch handle error: {err}");
                }
            },
            swarm_result = async {
                main_worker.start_swarm_handler()
            }.fuse() => {
                if let Err(err) = swarm_result {
                    error!("swarm handle error: {err}");
                }
            }
        }

        let public_interface_worker = main_worker.public_interface_worker.borrow().clone();
        Ok(public_interface_worker)
    }
}

// #[wasm_bindgen]
// pub async fn start_vane_web3(db_url: Option<String>, p2p_port: u16) -> Result<PublicInterfaceWorker, JsValue> {
//     let worker = WasmMainServiceWorker::run(db_url, p2p_port)
//         .await
//         .map_err(|e| JsValue::from_str(&format!("{:?}", e)))?;

//     Ok(worker)
// }