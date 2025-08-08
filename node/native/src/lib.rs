mod cryptography;
pub mod p2p;
pub mod rpc;
pub mod tx_processing;

use crate::p2p::P2pNetworkService;

use anyhow::{Error, anyhow};
use codec::Decode;
use core::str::FromStr;
use hex_literal::hex;
use primitives::data_structure::DbWorkerInterface;

use libp2p::request_response::{InboundRequestId, Message, ResponseChannel};
use libp2p::{Multiaddr, PeerId};
use log::{error, info, warn};
use primitives::data_structure::{
    ChainSupported, DbTxStateMachine, HashId, NetworkCommand, PeerRecord, RedisAccountProfile,
    SwarmMessage, TxStateMachine, TxStatus,
};
pub use rand::Rng;

/// Main thread to be spawned by the application
/// this encompasses all node's logic and processing flow
pub use lib_imports::*;

mod lib_imports {
    pub use crate::p2p::P2pWorker;
    pub use crate::rpc::TransactionRpcWorker;
    pub use crate::rpc::{RedisClient, TransactionRpcServer};
    pub use crate::tx_processing::TxProcessingWorker;
    pub use db::LocalDbWorker;
    pub use db::db::saved_peers::Data;
    pub use jsonrpsee::server::ServerBuilder;
    pub use libp2p::futures::{FutureExt, StreamExt};
    pub use local_ip_address::local_ip;
    pub use moka::future::Cache as AsyncCache;
    pub use redis::AsyncCommands;
    pub use std::hash::{DefaultHasher, Hash, Hasher};
    pub use std::net::SocketAddr;
    pub use std::sync::Arc;
    pub use tokio::sync::Mutex;
    pub use tokio::sync::mpsc::{Receiver, Sender};
}

#[derive(Clone)]
pub struct MainServiceWorker {
    pub db_worker: Arc<Mutex<LocalDbWorker>>,
    pub tx_rpc_worker: Arc<Mutex<TransactionRpcWorker>>,
    pub tx_processing_worker: Arc<Mutex<TxProcessingWorker>>,
    pub redis_client: RedisClient,
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

impl MainServiceWorker {
    pub(crate) async fn new(
        db_url_path: Option<String>,
        port: Option<u16>,
        redis_url: String,
        account_profile_hash: String,
        accounts: Vec<(String, String)>,
    ) -> Result<Self, anyhow::Error> {
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

        let redis_client = RedisClient::open(redis_url)?;

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
                } else {
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

        let moka_cache = AsyncCache::builder()
            .max_capacity(10)
            .name("TxStateMachine rpc tracker")
            .time_to_live(tokio::time::Duration::from_secs(600))
            .build();

        // PEER TO PEER NETWORKING WORKER
        // ===================================================================================== //

        let p2p_worker = P2pWorker::new(
            redis_client.clone(),
            db_worker.clone(),
            p2p_port,
            account_profile_hash,
            accounts,
            p2p_command_recv,
        )
        .await?;

        let p2p_network_service =
            P2pNetworkService::new(Arc::new(p2p_command_tx), p2p_worker.clone())?;

        // TRANSACTION PROCESSING LAYER
        // ===================================================================================== //

        let tx_processing_worker = TxProcessingWorker::new((
            ChainSupported::Bnb,
            ChainSupported::Ethereum,
            ChainSupported::Solana,
        ))
        .await?;

        // TRANSACTION RPC WORKER
        // ===================================================================================== //

        let txn_rpc_worker = TransactionRpcWorker::new(
            redis_client.clone(),
            db_worker.clone(),
            Arc::new(Mutex::new(rpc_recv_channel)),
            Arc::new(Mutex::new(user_rpc_update_sender_channel)),
            rpc_port,
            p2p_worker.node_id,
            moka_cache.clone(),
        )
        .await?;

        // ===================================================================================== //

        Ok(Self {
            db_worker,
            tx_rpc_worker: Arc::new(Mutex::new(txn_rpc_worker)),
            tx_processing_worker: Arc::new(Mutex::new(tx_processing_worker)),
            redis_client,
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
                            if let Err(e) = self
                                .rpc_sender_channel
                                .lock()
                                .await
                                .send(decoded_req.clone())
                                .await
                            {
                                error!("failed to send to rpc channel: {e}");
                                self.tx_rpc_worker
                                    .lock()
                                    .await
                                    .send_error(
                                        "channel",
                                        "Failed to send to RPC channel",
                                        Some(&e.to_string()),
                                    )
                                    .await;
                            }

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

                                    if let Err(e) = tx_processing.create_tx(&mut decoded_resp).await
                                    {
                                        error!("failed to create tx: {e}");
                                        self.tx_rpc_worker
                                            .lock()
                                            .await
                                            .send_error(
                                                "execution",
                                                "Failed to create transaction",
                                                Some(&e.to_string()),
                                            )
                                            .await;
                                    }

                                    info!(target:"MainServiceWorker","created a signable transaction");
                                }
                                Err(err) => {
                                    decoded_resp.recv_confirmation_failed();
                                    error!(target:"MainServiceWorker","receiver confirmation failed, reason: {err}");

                                    // Add error to RPC worker for user reporting
                                    self.tx_rpc_worker
                                        .lock()
                                        .await
                                        .send_error(
                                            "execution",
                                            "Receiver confirmation failed",
                                            Some(&err.to_string()),
                                        )
                                        .await;

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
                            if let Err(e) = self
                                .rpc_sender_channel
                                .lock()
                                .await
                                .send(decoded_resp.clone())
                                .await
                            {
                                error!("failed to send to rpc channel: {e}");
                                self.tx_rpc_worker
                                    .lock()
                                    .await
                                    .send_error(
                                        "channel",
                                        "Failed to send to RPC channel",
                                        Some(&e.to_string()),
                                    )
                                    .await;
                            }

                            self.moka_cache
                                .insert(decoded_resp.tx_nonce.into(), decoded_resp.clone())
                                .await;

                            info!(target: "MainServiceWorker","propagating txn msg as a response to rpc layer for user interaction: {decoded_resp:?}");
                        }
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
                let multi_addr = acc
                    .multi_addr
                    .expect("multi addr not found")
                    .parse::<Multiaddr>()?;
                let peer_id = PeerId::from_str(acc.peer_id.expect("peerId not found").as_str())?;

                // ========================================================================= //
                let mut p2p_network_service = self.p2p_network_service.lock().await;

                {
                    if let Err(e) = p2p_network_service
                        .dial_to_peer_id(multi_addr.clone(), &peer_id)
                        .await
                    {
                        error!("failed to dial to peer id: {e}");
                        self.tx_rpc_worker
                            .lock()
                            .await
                            .send_error(
                                "p2p network service",
                                "Failed to dial to peer id",
                                Some(&e.to_string()),
                            )
                            .await;
                    }
                }

                // wait for dialing to complete
                tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

                {
                    if let Err(e) = p2p_network_service
                        .send_request(txn.clone(), peer_id, multi_addr)
                        .await
                    {
                        error!("failed to send request to peer id: {e}");
                        self.tx_rpc_worker
                            .lock()
                            .await
                            .send_error(
                                "p2p network service",
                                "Failed to send request to peer id",
                                Some(&e.to_string()),
                            )
                            .await;
                    }
                }
            }
            Err(_err) => {
                // Add database error to RPC worker
                self.tx_rpc_worker
                    .lock()
                    .await
                    .send_error(
                        "database",
                        "Failed to find target peer in local database",
                        Some(&_err.to_string()),
                    )
                    .await;

                // fetch from remote db
                info!(target:"MainServiceWorker","target peer not found in local db, fetching from remote redis db");
                let target_id_addr = {
                    let tx = txn.lock().await;
                    tx.receiver_address.clone()
                };

                let target_user_profile: Option<RedisAccountProfile> = {
                    let mut redis_conn =
                        match self.redis_client.get_multiplexed_async_connection().await {
                            Ok(conn) => conn,
                            Err(err) => {
                                self.tx_rpc_worker
                                    .lock()
                                    .await
                                    .send_error(
                                        "database",
                                        "Failed to connect to Redis",
                                        Some(&err.to_string()),
                                    )
                                    .await;
                                return Err(anyhow!(
                                    "failed to connect to redis, caused by: {err}"
                                ));
                            }
                        };

                    match redis_conn
                        .hget::<String, String, String>("ACCOUNT_LINK".to_string(), target_id_addr)
                        .await
                    {
                        Ok(redis_target_user_hash_id) => {
                            // at this point per design we should have the target user profile in redis
                            match redis_conn
                                .hget::<String, String, String>(
                                    "ACCOUNT_PROFILE".to_string(),
                                    redis_target_user_hash_id,
                                )
                                .await
                            {
                                Ok(target_user_profile) => {
                                    match serde_json::from_str::<RedisAccountProfile>(
                                        &target_user_profile,
                                    ) {
                                        Ok(profile) => Some(profile),
                                        Err(err) => {
                                            self.tx_rpc_worker
                                                .lock()
                                                .await
                                                .send_error(
                                                    "database",
                                                    "Failed to parse Redis user profile",
                                                    Some(&err.to_string()),
                                                )
                                                .await;
                                            return Err(anyhow!(
                                                "failed to parse target user profile, caused by: {err}"
                                            ));
                                        }
                                    }
                                }
                                Err(err) => {
                                    self.tx_rpc_worker
                                        .lock()
                                        .await
                                        .send_error(
                                            "database",
                                            "Failed to get user profile from Redis",
                                            Some(&err.to_string()),
                                        )
                                        .await;
                                    None
                                }
                            }
                        }
                        Err(err) => {
                            self.tx_rpc_worker
                                .lock()
                                .await
                                .send_error(
                                    "database",
                                    "Failed to get user hash ID from Redis",
                                    Some(&err.to_string()),
                                )
                                .await;
                            None
                        }
                    }
                };

                if let Some(result_peer) = target_user_profile {
                    // dial the target
                    info!(target:"MainServiceWorker","target peer found in remote db: {result_peer:?} \n");

                    let multi_addr = result_peer
                        .multi_addr
                        .parse::<Multiaddr>()
                        .map_err(|err| anyhow!("failed to parse multi addr, caused by: {err}"))?;
                    let peer_id = PeerId::from_str(&*result_peer.peer_id)?;

                    // save the target peer id to local db
                    let peer_record = result_peer.into();
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
                        if let Err(e) = p2p_network_service
                            .dial_to_peer_id(multi_addr.clone(), &peer_id)
                            .await
                        {
                            error!("failed to dial to peer id: {e}");
                            self.tx_rpc_worker
                                .lock()
                                .await
                                .send_error(
                                    "p2p network service",
                                    "Failed to dial to peer id",
                                    Some(&e.to_string()),
                                )
                                .await;
                        }
                    }

                    // wait for dialing to complete
                    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

                    {
                        if let Err(e) = p2p_network_service
                            .send_request(txn.clone(), peer_id, multi_addr)
                            .await
                        {
                            error!("failed to send request to peer id: {e}");
                            self.tx_rpc_worker
                                .lock()
                                .await
                                .send_error(
                                    "p2p network service",
                                    "Failed to send request to peer id",
                                    Some(&e.to_string()),
                                )
                                .await;
                        }
                    }
                } else {
                    // Add network error to RPC worker
                    self.tx_rpc_worker
                        .lock()
                        .await
                        .send_error(
                            "network",
                            "Target peer not found in remote database",
                            Some("User is missing out on safety transaction"),
                        )
                        .await;

                    // return tx state as error on sender rpc
                    let mut txn = txn.lock().await.clone();
                    txn.recv_not_registered();

                    if let Err(e) = self.rpc_sender_channel.lock().await.send(txn.clone()).await {
                        error!("failed to send to rpc channel: {e}");
                        self.tx_rpc_worker
                            .lock()
                            .await
                            .send_error(
                                "channel",
                                "Failed to send to RPC channel",
                                Some(&e.to_string()),
                            )
                            .await;
                    }

                    self.moka_cache.insert(txn.tx_nonce.into(), txn).await;

                    warn!(target: "MainServiceWorker","target peer not found in remote db,tell the user is missing out on safety transaction");
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
        if let Err(e) = self
            .p2p_network_service
            .lock()
            .await
            .send_response(id, txn)
            .await
        {
            error!("failed to send response to peer id: {e}");
            self.tx_rpc_worker
                .lock()
                .await
                .send_error(
                    "p2p network service",
                    "Failed to send response to peer id",
                    Some(&e.to_string()),
                )
                .await;
        }
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
        if let Err(e) = self
            .tx_processing_worker
            .lock()
            .await
            .validate_receiver_sender_address(&txn_inner, "Sender")
        {
            error!("failed to validate receiver sender address: {e}");
            self.tx_rpc_worker
                .lock()
                .await
                .send_error(
                    "tx processing worker",
                    "Failed to validate receiver sender address",
                    Some(&e.to_string()),
                )
                .await;
        }

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

                    if let Err(e) = self
                        .rpc_sender_channel
                        .lock()
                        .await
                        .send(txn_inner.clone())
                        .await
                    {
                        error!("failed to send to rpc channel: {e}");
                        self.tx_rpc_worker
                            .lock()
                            .await
                            .send_error(
                                "channel",
                                "Failed to send to RPC channel",
                                Some(&e.to_string()),
                            )
                            .await;
                    }

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
                    if let Err(e) = self
                        .rpc_sender_channel
                        .lock()
                        .await
                        .send(txn_inner.clone())
                        .await
                    {
                        error!("failed to send to rpc channel: {e}");
                        self.tx_rpc_worker
                            .lock()
                            .await
                            .send_error(
                                "channel",
                                "Failed to send to RPC channel",
                                Some(&e.to_string()),
                            )
                            .await;
                    }
                }
            }
        } else {
            // non original sender confirmed, return error, send to rpc
            txn_inner.sender_confirmation_failed();
            error!(target: "MainServiceWorker","Non original sender signed");

            if let Err(e) = self
                .rpc_sender_channel
                .lock()
                .await
                .send(txn_inner.clone())
                .await
            {
                error!("failed to send to rpc channel: {e}");
                self.tx_rpc_worker
                    .lock()
                    .await
                    .send_error(
                        "channel",
                        "Failed to send to RPC channel",
                        Some(&e.to_string()),
                    )
                    .await;
            }
        }

        Ok(())
    }

    /// this for now is same as `handle_addr_confirmed_tx_state`
    pub(crate) async fn handle_net_confirmed_tx_state(&self) -> Result<(), anyhow::Error> {
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
                        if let Some(peer_id) = self
                            .db_worker
                            .lock()
                            .await
                            .get_saved_user_peers(tx.receiver_address.clone())
                            .await?
                            .peer_id
                        {
                            PeerId::from_str(peer_id.as_str()).unwrap()
                        } else {
                            return Err(anyhow!("Reverting: peer id not found"));
                        }
                    };
                    self.p2p_network_service
                        .lock()
                        .await
                        .disconnect_from_peer_id(&peer_id)
                        .await?;
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
    pub async fn run(
        db_url: Option<String>,
        port: Option<u16>,
        redis_url: String,
        account_profile_hash: String,
        accounts: Vec<(String, String)>,
    ) -> Result<(), anyhow::Error> {
        info!(
            "\n🔥 =========== Vane Web3 =========== 🔥\n\
             An intelligent safety layer that validates web3 transactions before execution.\n\
             Prevents fund loss from address errors, network mismatches,\n\
             and transaction mistakes by providing real-time verification and recovery options for your crypto transactions.\n"
        );

        // ====================================================================================== //
        let main_worker =
            Self::new(db_url, port, redis_url, account_profile_hash, accounts).await?;
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
    pub async fn e2e_new(
        port: u16,
        db: &str,
        redis_url: String,
        account_profile_hash: String,
        accounts: Vec<(String, String)>,
    ) -> Result<Self, Error> {
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
        let redis_client = RedisClient::open(redis_url)?;

        let moka_cache = AsyncCache::builder()
            .max_capacity(10)
            .name("TxStateMachine rpc tracker")
            .time_to_live(tokio::time::Duration::from_secs(600))
            .build();

        // PEER TO PEER NETWORKING WORKER
        // ===================================================================================== //
        let (rpc_port, p2p_port) = (port - 100, port - 589);
        let p2p_worker = P2pWorker::new(
            redis_client.clone(),
            db_worker.clone(),
            p2p_port,
            account_profile_hash,
            accounts,
            p2p_command_recv,
        )
        .await?;

        let p2p_network_service =
            P2pNetworkService::new(Arc::new(p2p_command_tx), p2p_worker.clone())?;

        // TRANSACTION PROCESSING LAYER
        // ===================================================================================== //

        let tx_processing_worker = TxProcessingWorker::new((
            ChainSupported::Bnb,
            ChainSupported::Ethereum,
            ChainSupported::Solana,
        ))
        .await?;

        // TRANSACTION RPC WORKER
        // ===================================================================================== //

        let txn_rpc_worker = TransactionRpcWorker::new(
            redis_client.clone(),
            db_worker.clone(),
            Arc::new(Mutex::new(rpc_recv_channel)),
            Arc::new(Mutex::new(user_rpc_update_sender_channel)),
            rpc_port,
            p2p_worker.node_id,
            moka_cache.clone(),
        )
        .await?;

        // ===================================================================================== //

        Ok(Self {
            db_worker,
            tx_rpc_worker: Arc::new(Mutex::new(txn_rpc_worker)),
            tx_processing_worker: Arc::new(Mutex::new(tx_processing_worker)),
            redis_client: redis_client,
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
