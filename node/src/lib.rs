
extern crate alloc;
extern crate core;

mod cryptography;
mod light_clients;
pub mod p2p;
pub mod rpc;
pub mod telemetry;
pub mod tx_processing;

use crate::p2p::P2pNetworkService;

use alloc::sync::Arc;
use hex_literal::hex;
use anyhow::{anyhow, Error};
use codec::Decode;
use core::str::FromStr;
use primitives::data_structure::DbWorkerInterface;

use libp2p::request_response::{InboundRequestId, Message, ResponseChannel};
use libp2p::{Multiaddr, PeerId};
use log::{error, info, warn};
use primitives::data_structure::{
    ChainSupported, DbTxStateMachine, HashId, NetworkCommand, PeerRecord, SwarmMessage,
    TxStateMachine, TxStatus,
};
pub use rand::Rng;

/// Main thread to be spawned by the application
/// this encompasses all node's logic and processing flow

#[cfg(not(target_arch = "wasm32"))]
pub use lib_imports::*;

#[cfg(not(target_arch = "wasm32"))]
mod lib_imports {
    pub use crate::rpc::TransactionRpcWorker;
    pub use crate::tx_processing::TxProcessingWorker;
    pub use std::hash::{DefaultHasher, Hash, Hasher};
    pub use std::net::SocketAddr;
    pub use tokio::sync::Mutex;
    pub extern crate rcgen;
    pub use crate::p2p::P2pWorker;
    pub use crate::rpc::{Airtable, TransactionRpcServer};
    pub use db::db::saved_peers::Data;
    pub use db::LocalDbWorker;
    pub use jsonrpsee::server::ServerBuilder;
    pub use libp2p::futures::{FutureExt, StreamExt};
    pub use local_ip_address::local_ip;
    pub use moka::future::Cache as AsyncCache;
    pub use rcgen::{generate_simple_self_signed, CertifiedKey};
    pub use tokio::sync::mpsc::{Receiver, Sender};
}

// -------------------------------- WASM ------------------------------ //
#[cfg(target_arch = "wasm32")]
use lib_wasm_imports::*;

#[cfg(target_arch = "wasm32")]
mod lib_wasm_imports {
    pub use crate::p2p::WasmP2pWorker;
    pub use crate::rpc::AirtableWasm;
    pub use crate::rpc::PublicInterfaceWorker;
    pub use crate::tx_processing::WasmTxProcessingWorker;
    pub use alloc::rc::Rc;
    pub use core::cell::RefCell;
    pub use db_wasm::OpfsRedbWorker;
    pub use lru::LruCache;
    pub use futures::FutureExt;
    pub use wasm_bindgen::prelude::wasm_bindgen;

}

#[cfg(target_arch = "wasm32")]
#[derive(Clone)]
pub struct WasmMainServiceWorker {
    pub db_worker: Rc<OpfsRedbWorker>,
    pub public_interface_worker: Rc<RefCell<PublicInterfaceWorker>>,
    pub wasm_tx_processing_worker: Rc<RefCell<WasmTxProcessingWorker>>,
    pub airtable_client: AirtableWasm,
    // for swarm events
    pub p2p_worker: Rc<RefCell<WasmP2pWorker>>,
    //telemetry_worker: TelemetryWorker,
    pub p2p_network_service: Rc<RefCell<P2pNetworkService>>,
    // channels for layers communication
    /// sender channel to propagate transaction state to rpc layer
    /// this serve as an update channel to the user
    pub rpc_sender_channel: Rc<RefCell<tokio_with_wasm::sync::mpsc::Sender<TxStateMachine>>>,
    /// receiver channel to handle the updates made by user from rpc
    pub user_rpc_update_recv_channel: Rc<RefCell<tokio_with_wasm::sync::mpsc::Receiver<TxStateMachine>>>,
    // moka cache
    pub lru_cache: RefCell<LruCache<u64, TxStateMachine>>,
}

#[cfg(target_arch = "wasm32")]
impl WasmMainServiceWorker {
    pub(crate) async fn new(db_url_path: Option<String>) -> Result<Self, anyhow::Error> {
        // CHANNELS
        // ===================================================================================== //
        // for rpc messages back and forth propagation
        let (rpc_sender_channel, rpc_recv_channel) = tokio_with_wasm::sync::mpsc::channel(10);
        let (user_rpc_update_sender_channel, user_rpc_update_recv_channel) =
            tokio_with_wasm::sync::mpsc::channel(10);

        // for p2p network commands
        let (p2p_command_tx, p2p_command_recv) = tokio_with_wasm::sync::mpsc::channel::<NetworkCommand>(10);

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
            let p2_port = rand::thread_rng().gen_range(0..=u16::MAX);
            {
                db.set_ports(p2_port, p2_port).await? // TODO we should change the api as in wasm there is no RPC
            }
            p2p_port = p2_port
        }

        let db_worker = Rc::new(db);

        // fetch to the db, if not then set one
        let airtable_client = AirtableWasm::new()
            .await
            .map_err(|err| anyhow!("failed to instantiate airtable client, caused by: {err}"))?;

        let lru_cache: LruCache<u64, TxStateMachine> = LruCache::unbounded();

        // PEER TO PEER NETWORKING WORKER
        // ===================================================================================== //

        let p2p_worker = WasmP2pWorker::new(
            Rc::new(airtable_client.clone()),
            db_worker.clone(),
            p2p_port,
            p2p_command_recv,
        )
        .await?;

        let p2p_network_service =
            P2pNetworkService::new(Rc::new(p2p_command_tx), p2p_worker.clone())?;

        // TRANSACTION RPC WORKER / PUBLIC INTERFACE
        // ===================================================================================== //

        let public_interface_worker = PublicInterfaceWorker::new(
            airtable_client.clone(),
            db_worker.clone(),
            Rc::new(RefCell::new(rpc_recv_channel)),
            Rc::new(RefCell::new(user_rpc_update_sender_channel)),
            p2p_worker.node_id,
            lru_cache.clone(),
        )
        .await?;

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
            airtable_client,
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

                    self.lru_cache.borrow_mut()
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

                    self.lru_cache.borrow_mut()
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
                // fetch from remote db
                info!(target:"MainServiceWorker","target peer not found in local db, fetching from remote db");

                let acc_ids = self.airtable_client.list_all_peers().await?;

                let target_id_addr = txn.borrow().receiver_address.clone();

                if !acc_ids.is_empty() {
                    let result_peer = acc_ids.into_iter().find_map(|discovery| {
                        match discovery
                            .clone()
                            .account_ids
                            .into_iter()
                            .find(|addr| addr == &target_id_addr)
                        {
                            Some(_) => {
                                let peer_record: PeerRecord = discovery.clone().into();
                                Some((discovery.peer_id, discovery.multi_addr, peer_record))
                            }
                            None => None,
                        }
                    });

                    if result_peer.is_some() {
                        // dial the target
                        info!(target:"MainServiceWorker","target peer found in remote db: {result_peer:?} \n");
                        let multi_addr = result_peer
                            .clone()
                            .expect("failed to get multi addr")
                            .1
                            .unwrap()
                            .parse::<Multiaddr>()
                            .map_err(|err| {
                                anyhow!("failed to parse multi addr, caused by: {err}")
                            })?;
                        let peer_id = PeerId::from_str(
                            &*result_peer
                                .clone()
                                .expect("failed to parse peer id")
                                .0
                                .expect("failed to parse peerId"),
                        )?;

                        // save the target peer id to local db
                        let peer_record = result_peer.clone().unwrap().2;
                        info!(target: "MainServiceWorker","recording target peer id to local db");

                        // ========================================================================= //
                        self.db_worker.record_saved_user_peers(peer_record).await?;

                        // ========================================================================= //
                        self.p2p_network_service
                            .borrow_mut()
                            .dial_to_peer_id(multi_addr.clone(), &peer_id)
                            .await?;
                        self.p2p_network_service
                            .borrow_mut()
                            .wasm_send_request(txn.clone(), peer_id, multi_addr)
                            .await?;
                    } else {
                        // return tx state as error on sender rpc
                        let mut txn = txn.borrow_mut().clone();
                        txn.recv_not_registered();
                        self.rpc_sender_channel
                            .borrow_mut()
                            .send(txn.clone())
                            .await?;
                        self.lru_cache.borrow_mut().push(txn.tx_nonce.into(), txn);

                        error!(target: "MainServiceWorker","target peer not found in remote db,tell the user is missing out on safety transaction");
                    }
                }
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

    pub async fn run(db_url: Option<String>) -> Result<PublicInterfaceWorker, anyhow::Error> {
        info!(
            "\n🔥 =========== Vane Web3 =========== 🔥\n\
             A safety layer for web3 transactions, allows you to feel secure when sending and receiving \n\
             tokens without the fear of selecting the wrong address or network. \n\
             It provides a safety net, giving you room to make mistakes without losing all your funds.\n"
        );

        // ====================================================================================== //
        let mut main_worker = Self::new(db_url).await?;

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

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub async fn start_vane_web3(db_url: Option<String>) -> Result<PublicInterfaceWorker, JsValue> {
    let worker = WasmMainServiceWorker::run(db_url)
        .await
        .map_err(|e| JsValue::from_str(&e.to_string()))?;

    Ok(worker)
}
// ========================================================= NATIVE ========================================================= //

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone)]
pub struct MainServiceWorker {
    pub db_worker: Arc<Mutex<LocalDbWorker>>,
    pub tx_rpc_worker: Arc<Mutex<TransactionRpcWorker>>,
    pub tx_processing_worker: Arc<Mutex<TxProcessingWorker>>,
    pub airtable_client: Airtable,
    // for swarm events
    pub p2p_worker: Arc<Mutex<P2pWorker>>, //telemetry_worker: TelemetryWorker,
    pub p2p_network_service: Arc<Mutex<P2pNetworkService>>,
    // channels for layers communication
    /// sender channel to propagate transaction state to rpc layer
    /// this serve as an update channel to the user
    pub rpc_sender_channel: Arc<Mutex<Sender<TxStateMachine>>>,
    /// receiver channel to handle the updates made by user from rpc
    pub user_rpc_update_recv_channel: Arc<Mutex<Receiver<Arc<Mutex<TxStateMachine>>>>>,
    // moka cache
    pub moka_cache: AsyncCache<u64, TxStateMachine>,
}

#[cfg(not(target_arch = "wasm32"))]
impl MainServiceWorker {
    pub(crate) async fn new(db_url_path: Option<String>, port: Option<u16>, airtable_record_id: String) -> Result<Self, anyhow::Error> {
        // CHANNELS
        // ===================================================================================== //
        // for rpc messages back and forth propagation
        let (rpc_sender_channel, rpc_recv_channel) = tokio::sync::mpsc::channel(10);
        let (user_rpc_update_sender_channel, user_rpc_update_recv_channel) =
            tokio::sync::mpsc::channel(10);

        // for p2p network commands
        let (p2p_command_tx, p2p_command_recv) = tokio::sync::mpsc::channel::<NetworkCommand>(10);

        // DATABASE WORKER (LOCAL AND REMOTE )
        // ===================================================================================== //
        let mut db_url = String::new();
        if let Some(url) = db_url_path {
            db_url = url
        } else {
            db_url = String::from("db/dev.db")
        }
        let db = LocalDbWorker::initialize_db_client(db_url.as_str()).await?;

        let mut rpc_port: u16 = 0;
        let mut p2p_port: u16 = 0;

        let returned_pots = db.get_ports().await?;
        if let Some(ports) = returned_pots {
            rpc_port = ports.rpc as u16;
            p2p_port = ports.p_2_p_port as u16;
        } else {
            let (rp_port, p2_port) = {
                if let Some(port) = port {
                    (port, port - 541)
                }else{
                    let port = rand::thread_rng().gen_range(0..=u16::MAX);
                    (port, port - 541)
                }

            };
            {
                db.set_ports(rp_port, p2_port).await?
            }
            rpc_port = rp_port;
            p2p_port = p2_port
        }

        let db_worker = Arc::new(Mutex::new(db));

        // fetch to the db, if not then set one
        let airtable_client = Airtable::new()
            .await
            .map_err(|err| anyhow!("failed to instantiate airtable client, caused by: {err}"))?;

        let moka_cache = AsyncCache::builder()
            .max_capacity(10)
            .name("TxStateMachine rpc tracker")
            .time_to_live(tokio::time::Duration::from_secs(600))
            .build();

        // PEER TO PEER NETWORKING WORKER
        // ===================================================================================== //

        let p2p_worker = P2pWorker::new(
            Arc::new(Mutex::new(airtable_client.clone())),
            db_worker.clone(),
            p2p_port,
            airtable_record_id,
            p2p_command_recv,
        )
        .await?;

        let p2p_network_service =
            P2pNetworkService::new(Arc::new(p2p_command_tx), p2p_worker.clone())?;

        // TRANSACTION RPC WORKER
        // ===================================================================================== //

        let txn_rpc_worker = TransactionRpcWorker::new(
            airtable_client.clone(),
            db_worker.clone(),
            Arc::new(Mutex::new(rpc_recv_channel)),
            Arc::new(Mutex::new(user_rpc_update_sender_channel)),
            rpc_port,
            p2p_worker.node_id,
            moka_cache.clone(),
        )
        .await?;

        // TRANSACTION PROCESSING LAYER
        // ===================================================================================== //

        let tx_processing_worker = TxProcessingWorker::new((
            ChainSupported::Bnb,
            ChainSupported::Ethereum,
            ChainSupported::Solana,
        ))
        .await?;
        // ===================================================================================== //

        Ok(Self {
            db_worker,
            tx_rpc_worker: Arc::new(Mutex::new(txn_rpc_worker)),
            tx_processing_worker: Arc::new(Mutex::new(tx_processing_worker)),
            airtable_client,
            p2p_worker: Arc::new(Mutex::new(p2p_worker)),
            p2p_network_service: Arc::new(Mutex::new(p2p_network_service)),
            rpc_sender_channel: Arc::new(Mutex::new(rpc_sender_channel)),
            user_rpc_update_recv_channel: Arc::new(Mutex::new(user_rpc_update_recv_channel)),
            moka_cache,
        })
    }

    /// handle swarm events; this includes
    /// 1. sender sending requests to receiver to attest ownership and correctness of the recv address
    /// 2. receiver response and sender handling submission of the tx
    pub(crate) async fn handle_swarm_event_messages(
        &self,
        p2p_worker: Arc<Mutex<P2pWorker>>,
        txn_processing_worker: TxProcessingWorker,
    ) -> Result<(), Error> {
        let (sender_channel, mut recv_channel) = tokio::sync::mpsc::channel(256);

        // Start swarm first and keep it running infinitely
        tokio::spawn(async move {
            let res = p2p_worker.lock().await.start_swarm(sender_channel).await;
            if res.is_err() {
                error!("start swarm failed");
            }
        });


        loop {
            if let Some(swarm_msg_result) = recv_channel.recv().await {
                match swarm_msg_result {
                    Ok(swarm_msg) => match swarm_msg {

                        SwarmMessage::Request { data, inbound_id } => {
                            let mut decoded_req: TxStateMachine = Decode::decode(&mut &data[..])
                                .expect("failed to decode request body");

                            let inbound_req_id = inbound_id.get_hash_id();
                            println!("inbound req id: {inbound_req_id}");
                            decoded_req.inbound_req_id = Some(inbound_req_id);
                            // ===================================================================== //
                            // propagate transaction state to rpc layer for user updating (receiver updating)
                            self.rpc_sender_channel
                                .lock()
                                .await
                                .send(decoded_req.clone())
                                .await?;
                            self.moka_cache
                                .insert(decoded_req.tx_nonce.into(), decoded_req.clone())
                                .await;

                            info!(target: "MainServiceWorker","propagating txn msg as a request to rpc layer for user interaction: {decoded_req:?}");
                        }

                        SwarmMessage::Response { data, outbound_id } => {
                            let mut decoded_resp: TxStateMachine = Decode::decode(&mut &data[..])
                                .expect("failed to decode request body");

                            let outbound_req_id = outbound_id.get_hash_id();
                            decoded_resp.outbound_req_id = Some(outbound_req_id);
                            // ===================================================================== //
                            // handle error, by returning the tx status to the sender
                            match txn_processing_worker
                                .validate_receiver_sender_address(&decoded_resp, "Receiver")
                            {
                                Ok(_) => {
                                    decoded_resp.recv_confirmation_passed();
                                    info!(target:"MainServiceWorker","receiver confirmation passed");
                                    // create a signable tx for sender to sign upon confirmation
                                    let mut tx_processing =
                                        self.tx_processing_worker.lock().await.clone();
                                    tx_processing.create_tx(&mut decoded_resp).await?;

                                    info!(target:"MainServiceWorker","created a signable transaction");
                                }
                                Err(err) => {
                                    decoded_resp.recv_confirmation_failed();
                                    error!(target:"MainServiceWorker","receiver confirmation failed, reason: {err}");
                                    // record failed txn in local db
                                    let db_tx = DbTxStateMachine {
                                        tx_hash: vec![],
                                        amount: decoded_resp.amount,
                                        network: decoded_resp.network,
                                        success: false,
                                    };
                                    self.db_worker.lock().await.update_failed_tx(db_tx).await?;
                                    log::info!(target: "MainServiceWorker","Db recorded failed transaction");
                                }
                            }

                            // propagate transaction state to rpc layer for user updating ( this time sender verification)
                            self.rpc_sender_channel
                                .lock()
                                .await
                                .send(decoded_resp.clone())
                                .await?;

                            self.moka_cache
                                .insert(decoded_resp.tx_nonce.into(), decoded_resp.clone())
                                .await;

                            info!(target: "MainServiceWorker","propagating txn msg as a response to rpc layer for user interaction: {decoded_resp:?}");
                        },
                        _ => {}
                    },
                    Err(err) => {
                        info!("no new messages from swarm: {err}");
                        // Don't return error, just log and continue
                        continue;
                    }
                }
            }
        }
    }

    /// genesis state of initialized tx is being handled by the following stages
    /// 1. check if the receiver address peer id is saved in local db if not then search in remote db
    /// 2. getting the recv peer-id then dial the target peer-id (receiver)
    /// 3. send the tx-state-machine object to receiver target id via p2p swarm for receiver to sign and attest ownership and correctness of the address
    pub(crate) async fn handle_genesis_tx_state(
        &self,
        txn: Arc<Mutex<TxStateMachine>>,
    ) -> Result<(), Error> {
        // dial to target peer id from tx receiver
        let target_id = {
            let tx = txn.lock().await;
            tx.receiver_address.clone()
        };
        // check if the acc is present in local db
        // First try local DB
        let target_peer_result = {
            // Release DB lock immediately after query
            self.db_worker
                .lock()
                .await
                .get_saved_user_peers(target_id.clone())
                .await
        };

        match target_peer_result {
            Ok(acc) => {
                info!(target:"MainServiceWorker","target peer found in local db");
                // dial the target
                let multi_addr = acc.multi_addr.expect("multi addr not found").parse::<Multiaddr>()?;
                let peer_id = PeerId::from_str(acc.peer_id.expect("peerId not found").as_str())?;

                // ========================================================================= //
                let mut p2p_network_service = self.p2p_network_service.lock().await;

                {
                    p2p_network_service
                        .dial_to_peer_id(multi_addr.clone(), &peer_id)
                        .await?;
                }

                // wait for dialing to complete
                tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

                {
                    p2p_network_service
                        .send_request(txn.clone(), peer_id, multi_addr)
                        .await?;
                }
            }
            Err(_err) => {
                // fetch from remote db
                info!(target:"MainServiceWorker","target peer not found in local db, fetching from remote db");

                let acc_ids = self.airtable_client.list_all_peers().await?;

                let target_id_addr = {
                    let tx = txn.lock().await;
                    tx.receiver_address.clone()
                };

                if !acc_ids.is_empty() {
                    let result_peer = acc_ids.into_iter().find_map(|discovery| {
                        match discovery
                            .clone()
                            .account_ids
                            .into_iter()
                            .find(|addr| addr.account == target_id_addr)
                        {
                            Some(_) => {
                                let peer_record: PeerRecord = discovery.clone().into();
                                Some((discovery.peer_id, discovery.multi_addr, peer_record))
                            }
                            None => None,
                        }
                    });

                    if result_peer.is_some() {
                        // dial the target
                        info!(target:"MainServiceWorker","target peer found in remote db: {result_peer:?} \n");
                        let multi_addr = result_peer
                            .clone()
                            .expect("failed to get multi addr")
                            .1
                            .unwrap()
                            .parse::<Multiaddr>()
                            .map_err(|err| {
                                anyhow!("failed to parse multi addr, caused by: {err}")
                            })?;
                        let peer_id = PeerId::from_str(
                            &*result_peer
                                .clone()
                                .expect("GENESIS: failed to find peer id")
                                .0
                                .expect("GENESIS: failed to parse peerId"),
                        )?;

                        // save the target peer id to local db
                        let peer_record = result_peer.clone().unwrap().2;
                        info!(target: "MainServiceWorker","recording target peer id to local db");

                        // ========================================================================= //
                        {
                            self.db_worker
                                .lock()
                                .await
                                .record_saved_user_peers(peer_record)
                                .await?;
                        }

                        // ========================================================================= //
                        let mut p2p_network_service = self.p2p_network_service.lock().await;

                        {
                            p2p_network_service
                                .dial_to_peer_id(multi_addr.clone(), &peer_id)
                                .await?;
                        }

                        // wait for dialing to complete
                        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

                        {
                            p2p_network_service
                                .send_request(txn.clone(), peer_id, multi_addr)
                                .await?;
                        }
                    } else {
                        // return tx state as error on sender rpc
                        let mut txn = txn.lock().await.clone();
                        txn.recv_not_registered();
                        self.rpc_sender_channel
                            .lock()
                            .await
                            .send(txn.clone())
                            .await?;
                        self.moka_cache.insert(txn.tx_nonce.into(), txn).await;

                        error!(target: "MainServiceWorker","target peer not found in remote db,tell the user is missing out on safety transaction");
                    }
                }
            }
        }
        Ok(())
    }

    /// send the response to the sender via p2p swarm
    /// this will be executed on receiver's end
    pub(crate) async fn handle_recv_addr_confirmed_tx_state(
        &self,
        id: u64,
        txn: Arc<Mutex<TxStateMachine>>,
    ) -> Result<(), Error> {
        self.p2p_network_service
            .lock()
            .await
            .send_response(id, txn)
            .await?;
        Ok(())
    }

    /// last stage, submit the txn state-machine object to rpc to be signed and then submit to the target chain
    /// this will be executed on sender's end
    pub(crate) async fn handle_sender_confirmed_tx_state(
        &self,
        txn: Arc<Mutex<TxStateMachine>>,
    ) -> Result<(), Error> {
        let mut txn_inner = txn.lock().await.clone();

        // verify sender
        self.tx_processing_worker
            .lock()
            .await
            .validate_receiver_sender_address(&txn_inner, "Sender")?;
        log::info!(target: "MainServiceWorker","sender verification passed");
        // verify multi id
        if self
            .tx_processing_worker
            .lock()
            .await
            .validate_multi_id(&txn_inner)
        {
            // TODO! handle submission errors
            // signed and ready to be submitted to target chain
            log::info!(target: "MainServiceWorker","multiId verification passed");

            match self
                .tx_processing_worker
                .lock()
                .await
                .submit_tx(txn_inner.clone())
                .await
            {
                Ok(tx_hash) => {
                    // update user via rpc on tx success
                    txn_inner.tx_submission_passed(tx_hash);
                    self.rpc_sender_channel
                        .lock()
                        .await
                        .send(txn_inner.clone())
                        .await?;

                    // update local db on success tx
                    let db_tx = DbTxStateMachine {
                        tx_hash: tx_hash.to_vec(),
                        amount: txn_inner.amount.clone(),
                        network: txn_inner.network.clone(),
                        success: true,
                    };
                    self.db_worker.lock().await.update_success_tx(db_tx).await?;

                    log::info!(target: "MainServiceWorker","Db recorded success tx");
                }
                Err(err) => {
                    txn_inner.tx_submission_failed(format!(
                        "{err:?}: the tx will be resubmitted rest assured"
                    ));
                    self.rpc_sender_channel.lock().await.send(txn_inner).await?;
                }
            }
        } else {
            // non original sender confirmed, return error, send to rpc
            txn_inner.sender_confirmation_failed();
            error!(target: "MainServiceWorker","Non original sender signed");
            self.rpc_sender_channel.lock().await.send(txn_inner).await?;
        }

        Ok(())
    }

    /// this for now is same as `handle_addr_confirmed_tx_state`
    pub(crate) async fn handle_net_confirmed_tx_state(
        &self,
        _txn: Arc<Mutex<TxStateMachine>>,
    ) -> Result<(), anyhow::Error> {
        todo!()
    }

    /// all user interactions are done via rpc, after user sends rpc as updated (`tx-state-machine`) as argument,
    /// the tx object will be send to channel to be handled depending on its current state
    pub(crate) async fn handle_incoming_rpc_tx_updates(&self) -> Result<(), anyhow::Error> {
        while let Some(txn) = self.user_rpc_update_recv_channel.lock().await.recv().await {
            // handle the incoming transaction per its state
            let status = txn.lock().await.clone().status;
            match status {
                TxStatus::Genesis => {
                    info!(target:"MainServiceWorker","handling incoming genesis tx updates: {:?} \n",txn.lock().await.clone());
                    self.handle_genesis_tx_state(txn.clone()).await?;
                }

                TxStatus::RecvAddrConfirmed => {
                    info!(target:"MainServiceWorker","handling incoming receiver addr-confirmation tx updates: {:?} \n",txn.lock().await.clone());

                    let inbound_id = txn
                        .lock()
                        .await
                        .inbound_req_id
                        .expect("no inbound req id found");
                    self.handle_recv_addr_confirmed_tx_state(inbound_id, txn.clone())
                        .await?;
                }

                TxStatus::NetConfirmed => {
                    todo!()
                }

                TxStatus::SenderConfirmed => {
                    info!(target:"MainServiceWorker","handling incoming sender addr-confirmed tx updates: {:?} \n",txn.lock().await.clone());

                    self.handle_sender_confirmed_tx_state(txn.clone()).await?;
                }

                TxStatus::Reverted => {
                    let peer_id = { 
                        let tx = txn.lock().await;
                        if let Some(peer_id) = self.db_worker.lock().await.get_saved_user_peers(tx.receiver_address.clone()).await?.peer_id {
                            PeerId::from_str(peer_id.as_str()).unwrap()
                            
                        } else {
                            return Err(anyhow!("Reverting: peer id not found"));
                        }
                    };
                    self.p2p_network_service.lock().await.disconnect_from_peer_id(&peer_id).await?;
                }
                _ => {}
            };
        }
        Ok(())
    }

    /// Start rpc server with default url
    pub(crate) async fn start_rpc_server(&self) -> Result<SocketAddr, anyhow::Error> {
        let server_builder = ServerBuilder::new();

        // --------------------------- TLS CERT---------------------------------- //
        let url_names = vec!["197.168.1.177".to_string(), "localhost".to_string()];
        // let CertifiedKey { cert, key_pair } = generate_simple_self_signed(url_names)
        //     .map_err(|err| anyhow!("failed to generate tsl cert; {err:?}"))?;

        let url = self.tx_rpc_worker.lock().await.rpc_url.clone();
        let rpc_handler = self.tx_rpc_worker.clone().lock().await.clone();

        let server = server_builder.build(url).await?;
        let address = server
            .local_addr()
            .map_err(|err| anyhow!("failed to get address: {}", err))?;
        let handle = server
            .start(rpc_handler.into_rpc())
            .map_err(|err| anyhow!("rpc handler error: {}", err))?;

        tokio::spawn(handle.stopped());
        Ok(address)
    }

    /// compose all workers and run logically, the p2p swarm worker will be running indefinately on background same as rpc worker
    pub async fn run(db_url: Option<String>, port: Option<u16>, airtable_record_id: String) -> Result<(), anyhow::Error> {
        info!(
            "\n🔥 =========== Vane Web3 =========== 🔥\n\
             A safety layer for web3 transactions, allows you to feel secure when sending and receiving \n\
             tokens without the fear of selecting the wrong address or network. \n\
             It provides a safety net, giving you room to make mistakes without losing all your funds.\n"
        );

        // ====================================================================================== //
        let main_worker = Self::new(db_url,port,airtable_record_id).await?;
        // start rpc server
        let rpc_address = main_worker
            .start_rpc_server()
            .await
            .map_err(|err| anyhow!("failed to start rpc server, caused by: {err}"))?;

        info!(target: "RpcServer","listening to rpc url: {rpc_address}");
        // ====================================================================================== //

        let p2p_worker = main_worker.p2p_worker.clone();
        let txn_processing_worker = main_worker
            .tx_processing_worker
            .clone()
            .lock()
            .await
            .clone();

        // ====================================================================================== //

        let tokio_handle = tokio::runtime::Handle::current();
        let mut task_manager = sc_service::TaskManager::new(tokio_handle, None)?;

        // ====================================================================================== //

        {
            let cloned_main_worker = main_worker.clone();
            let task_name = "transaction-handling-task".to_string();
            task_manager.spawn_essential_handle().spawn_blocking(
                Box::leak(Box::new(task_name)),
                "transaction-handling",
                async move {
                    // watch tx messages from tx rpc worker and pass it to p2p to be verified by receiver
                    let res = cloned_main_worker.handle_incoming_rpc_tx_updates().await;
                    if let Err(err) = res {
                        error!("rpc handle encountered error: caused by {err}");
                    }
                }
                .boxed(),
            )
        }

        {
            let task_name = "swarm-p2p-task".to_string();
            task_manager.spawn_essential_handle().spawn_blocking(
                Box::leak(Box::new(task_name)),
                "swarm",
                async move {
                    let res = main_worker
                        .handle_swarm_event_messages(p2p_worker, txn_processing_worker)
                        .await;
                    if let Err(err) = res {
                        error!("swarm handle encountered error; caused by {err}");
                    }
                }
                .boxed(),
            )
        }

        task_manager.future().await?;

        Ok(())
    }

    // =================================== E2E ====================================== //

    #[cfg(feature = "e2e")]
    pub async fn e2e_new(port: u16, db: &str, airtable_record_id: String) -> Result<Self, Error> {
        // CHANNELS
        // ===================================================================================== //
        // for rpc messages back and forth propagation
        let (rpc_sender_channel, rpc_recv_channel) = tokio::sync::mpsc::channel(10);
        let (user_rpc_update_sender_channel, user_rpc_update_recv_channel) =
            tokio::sync::mpsc::channel(10);

        // for p2p network commands
        let (p2p_command_tx, p2p_command_recv) = tokio::sync::mpsc::channel::<NetworkCommand>(10);

        // DATABASE WORKER (LOCAL AND REMOTE )
        // ===================================================================================== //
        let db_worker = Arc::new(Mutex::new(LocalDbWorker::initialize_db_client(db).await?));

        // fetch to the db, if not then set one
        let airtable_client = Airtable::new()
            .await
            .map_err(|err| anyhow!("failed to instantiate airtable client, caused by: {err}"))?;

        let moka_cache = AsyncCache::builder()
            .max_capacity(10)
            .name("TxStateMachine rpc tracker")
            .time_to_live(tokio::time::Duration::from_secs(600))
            .build();

        // PEER TO PEER NETWORKING WORKER
        // ===================================================================================== //
        let (rpc_port, p2p_port) = (port - 100, port - 589);
        let p2p_worker = P2pWorker::new(
            Arc::new(Mutex::new(airtable_client.clone())),
            db_worker.clone(),
            p2p_port,
            airtable_record_id,
            p2p_command_recv,
        )
        .await?;

        let p2p_network_service =
            P2pNetworkService::new(Arc::new(p2p_command_tx), p2p_worker.clone())?;

        // TRANSACTION RPC WORKER
        // ===================================================================================== //

        let txn_rpc_worker = TransactionRpcWorker::new(
            airtable_client.clone(),
            db_worker.clone(),
            Arc::new(Mutex::new(rpc_recv_channel)),
            Arc::new(Mutex::new(user_rpc_update_sender_channel)),
            rpc_port,
            p2p_worker.node_id,
            moka_cache.clone(),
        )
        .await?;

        // TRANSACTION PROCESSING LAYER
        // ===================================================================================== //

        let tx_processing_worker = TxProcessingWorker::new((
            ChainSupported::Bnb,
            ChainSupported::Ethereum,
            ChainSupported::Solana,
        ))
        .await?;
        // ===================================================================================== //

        Ok(Self {
            db_worker,
            tx_rpc_worker: Arc::new(Mutex::new(txn_rpc_worker)),
            tx_processing_worker: Arc::new(Mutex::new(tx_processing_worker)),
            airtable_client,
            p2p_worker: Arc::new(Mutex::new(p2p_worker)),
            p2p_network_service: Arc::new(Mutex::new(p2p_network_service)),
            rpc_sender_channel: Arc::new(Mutex::new(rpc_sender_channel)),
            user_rpc_update_recv_channel: Arc::new(Mutex::new(user_rpc_update_recv_channel)),
            moka_cache,
        })
    }

    #[cfg(feature = "e2e")]
    pub async fn e2e_run(main_worker: MainServiceWorker) -> Result<(), anyhow::Error> {
        // ====================================================================================== //
        // start rpc server
        let rpc_address = main_worker
            .start_rpc_server()
            .await
            .map_err(|err| anyhow!("failed to start rpc server, caused by: {err}"))?;

        info!(target: "RpcServer","listening to rpc url: {rpc_address}");
        // ====================================================================================== //

        let p2p_worker = main_worker.p2p_worker.clone();
        let txn_processing_worker = main_worker
            .tx_processing_worker
            .clone()
            .lock()
            .await
            .clone();

        // ====================================================================================== //

        let tokio_handle = tokio::runtime::Handle::current();
        let mut task_manager = sc_service::TaskManager::new(tokio_handle, None)?;

        // ====================================================================================== //

        {
            let cloned_main_worker = main_worker.clone();
            let task_name = "transaction-handling-task".to_string();
            task_manager.spawn_essential_handle().spawn_blocking(
                Box::leak(Box::new(task_name)),
                "transaction-handling",
                async move {
                    // watch tx messages from tx rpc worker and pass it to p2p to be verified by receiver
                    let res = cloned_main_worker.handle_incoming_rpc_tx_updates().await;
                    if let Err(err) = res {
                        error!("rpc handle encountered error: caused by {err}");
                    }
                }
                .boxed(),
            )
        }

        {
            let task_name = "swarm-p2p-task".to_string();
            task_manager.spawn_essential_handle().spawn_blocking(
                Box::leak(Box::new(task_name)),
                "swarm",
                async move {
                    let res = main_worker
                        .handle_swarm_event_messages(p2p_worker, txn_processing_worker)
                        .await;
                    if let Err(err) = res {
                        error!("swarm handle encountered error; caused by {err}");
                    }
                }
                .boxed(),
            )
        }

        task_manager.future().await?;

        Ok(())
    }
}
