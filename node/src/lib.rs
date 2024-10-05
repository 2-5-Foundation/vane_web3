extern crate alloc;
mod cryptography;
pub mod p2p;
pub mod rpc;
pub mod telemetry;
pub mod tx_processing;

use alloc::sync::Arc;
use alloy::hex;
use anyhow::{anyhow, Error};
use codec::Decode;
use db::DbWorker;
use libp2p::futures::{FutureExt, StreamExt};
use libp2p::request_response::Message;
use libp2p::PeerId;
use log::{error, warn};
use p2p::P2pWorker;
pub use primitives;
use primitives::data_structure::{ChainSupported, Fields, PeerRecord, TxStateMachine, TxStatus};
use rpc::TransactionRpcWorker;
use telemetry::TelemetryWorker;
use tokio::io::AsyncBufReadExt;
use tokio::sync::Mutex;
use tx_processing::TxProcessingWorker;

/// Main thread to be spawned by the application
/// this encompasses all node's logic and processing flow
#[derive(Clone)]
pub struct MainServiceWorker {
    db_worker: Arc<Mutex<DbWorker>>,
    tx_rpc_worker: Arc<Mutex<TransactionRpcWorker>>,
    tx_processing_worker: Arc<Mutex<TxProcessingWorker>>,
    p2p_worker: Arc<Mutex<P2pWorker>>,
    //telemetry_worker: TelemetryWorker,
}

impl MainServiceWorker {
    pub async fn new() -> Result<Self, anyhow::Error> {
        let (sender_channel, recv_channel) = tokio::sync::mpsc::channel(u8::MAX as usize);
        let shared_recv_channel = Arc::new(Mutex::new(recv_channel));
        let txn_rpc_worker =
            TransactionRpcWorker::new(shared_recv_channel.clone(), sender_channel).await?;

        let tx_processing_worker = TxProcessingWorker::new(
            shared_recv_channel.clone(),
            (
                ChainSupported::Bnb,
                ChainSupported::Ethereum,
                ChainSupported::Solana,
            ),
        )
        .await?;

        let db_worker = txn_rpc_worker.db_worker.clone();

        let self_peer_id = libp2p::identity::Keypair::generate_ed25519();
        let peer_account = PeerRecord {
            peer_address: self_peer_id
                .public()
                .to_peer_id()
                .to_base58()
                .as_bytes()
                .to_vec(),
            accountId1: None,
            accountId2: None,
            accountId3: None,
            accountId4: None,
            multi_addr: txn_rpc_worker.url.to_string().as_bytes().to_vec(),
            keypair: Some(
                self_peer_id
                    .to_protobuf_encoding()
                    .map_err(|_| anyhow!("failed to encode keypair"))?,
            ),
        };

        db_worker
            .lock()
            .await
            .record_user_peerId(peer_account.clone())
            .await?;

        let p2p_worker = P2pWorker::new(peer_account).await?;

        Ok(Self {
            db_worker,
            tx_rpc_worker: Arc::new(Mutex::new(txn_rpc_worker)),
            tx_processing_worker: Arc::new(Mutex::new(tx_processing_worker)),
            p2p_worker: Arc::new(Mutex::new(p2p_worker)),
        })
    }

    /// handle swarm events; this includes
    /// 1. sender requests to receiver to attest ownership and correctness of the recv address
    /// 2. receiver response and sender handling submission of the tx
    pub(crate) async fn handle_swarm_event_messages(
        p2p_worker: Arc<Mutex<P2pWorker>>,
        txn_rpc_worker: Arc<Mutex<TransactionRpcWorker>>,
    ) -> Result<(), anyhow::Error> {
        while let Ok(mut swarm_msg_result) = p2p_worker.lock().await.start_swarm().await {
            match swarm_msg_result.next().await {
                Some(swarm_msg) => {
                    match swarm_msg {
                        Ok(msg) => {
                            // handle the req and resp
                            match msg {
                                // context of a receiver, receiving the request and handling it
                                // at this point, the receiver should;
                                // 1. send the tx to the rpc
                                // 2. sign the message attesting ownership of the private key and can control the acc in X network
                                // 3. update the tx state machine and send it back to the initial sender
                                Message::Request {
                                    request_id,
                                    request,
                                    ..
                                } => {
                                    let decoded_req: TxStateMachine =
                                        Decode::decode(&mut &request[..])
                                            .expect("failed to decode request body");
                                    // send it to be signed via Rpc
                                    let re = txn_rpc_worker
                                        .lock()
                                        .await
                                        .sender_channel
                                        .lock()
                                        .await
                                        .send(Arc::new(Mutex::new(decoded_req)))
                                        .await;
                                }

                                // context of a sender, receiving the response from the target receiver
                                // the sender should;
                                // 1.verify the recv signature public key to the one binded in the multi address
                                // 2. send the tx to be signed to rpc
                                // 3. take the tx and submit to the chain
                                Message::Response {
                                    request_id,
                                    response,
                                } => {
                                    todo!()
                                }
                            }
                        }
                        Err(err) => {
                            error!("failed to return swarm msg event; caused by: {err:?}")
                        }
                    }
                }
                None => {
                    warn!("no new messages from swarm")
                }
            }
        }
        Ok(())
    }

    /// genesis state of initialized tx is being handled by the following stages
    /// 1. check if the receiver address peer id is saved in local db if not then search in remote db
    /// 2. getting the recv peer-id then dial the target peer-id (receiver)
    /// 3. send the tx-state-machine object to receiver target id via p2p swarm for receiver to sign and attest ownership and correctness of the address
    pub(crate) async fn handle_genesis_tx_state(
        &self,
        txn: Arc<Mutex<TxStateMachine>>,
    ) -> Result<(), anyhow::Error> {
        // dial to target peer id from tx receiver
        let target_id = txn.lock().await.receiver_address.clone();
        // check if the acc is present in local db
        if let Ok(acc) = self
            .db_worker
            .lock()
            .await
            .get_saved_user_peers(target_id)
            .await
        {
            // dial the target
            let multi_addr = String::from_utf8(acc.multi_addr)?.parse()?;
            let peer_id = PeerId::from_bytes(&acc.node_id)?;
            self.p2p_worker
                .lock()
                .await
                .dial_to_peer_id(multi_addr, peer_id)
                .await?;
            // send the tx-state-machine to target peerId
            // todo!()
        } else {
            // fetch from remote db
            let acc_ids = self
                .tx_rpc_worker
                .lock()
                .await
                .airtable_client
                .lock()
                .await
                .list_all_peers()
                .await?;
            let target_id_addr = hex::encode(txn.lock().await.receiver_address.clone());

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
                    let multi_addr = result_peer
                        .clone()
                        .expect("failed to get multi addr")
                        .1
                        .parse()?;
                    let peer_id = PeerId::from_bytes(
                        result_peer
                            .clone()
                            .expect("failed to get peer id")
                            .0
                            .as_bytes(),
                    )?;

                    self.p2p_worker
                        .lock()
                        .await
                        .dial_to_peer_id(multi_addr, peer_id)
                        .await?;
                    // send the tx-state-machine to target peerId
                    // todo!()

                    // save the target peer id to local db
                    let peer_record = result_peer.clone().unwrap().2;
                    self.db_worker
                        .lock()
                        .await
                        .record_saved_user_peers(peer_record)
                        .await?;
                } else {
                    Err(anyhow!("unexpected error; user not registered to vane web3, tell the user is missing out on safety"))?
                }
            } else {
                Err(anyhow!(
                    "user not registered to vane web3, tell the user is missing out on safety"
                ))?
            }
        }
        Ok(())
    }

    /// after receiver receives `tx-state-machine` from p2p swarm message,
    /// signs the msg via rpc and send a response to sender of tx via p2p-swarm
    /// then after the state of tx is `addrConfirmed` the sender will verify signature and submit tx
    pub(crate) async fn handle_addr_confirmed_tx_state(
        txn: Arc<Mutex<TxStateMachine>>,
    ) -> Result<(), anyhow::Error> {
        todo!()
    }

    /// this for now is same as `handle_addr_confirmed_tx_state`
    pub(crate) async fn handle_net_confirmed_tx_state(
        txn: Arc<Mutex<TxStateMachine>>,
    ) -> Result<(), anyhow::Error> {
        todo!()
    }

    /// all user interactions are done via rpc, after user sends rpc as updated (`tx-state-machine`) as argument,
    /// the tx object will be send to channel to be handled depending on its current state
    pub(crate) async fn handle_incoming_rpc_tx_updates(&self) -> Result<(), anyhow::Error> {
        while let Some(txn) = self
            .tx_rpc_worker
            .lock()
            .await
            .receiver_channel
            .lock()
            .await
            .recv()
            .await
        {
            // check the state of tx
            match txn.lock().await.status {
                TxStatus::genesis => {
                    self.handle_genesis_tx_state(txn).await?;
                }
                TxStatus::addrConfirmed => {}
                TxStatus::netConfirmed => {
                    todo!()
                }
            }
        }
        Ok(())
    }

    /// compose all workers and run logically, the p2p swarm worker will be running indefinately on background same as rpc worker
    pub async fn run(&self) -> Result<(), anyhow::Error> {
        // start rpc server

        // listen to p2p swarm events
        let p2p_worker = self.p2p_worker.clone();
        let txn_rpc_worker = self.tx_rpc_worker.clone();

        let swarm_result_handle = tokio::spawn(async move {
            let res = Self::handle_swarm_event_messages(p2p_worker, txn_rpc_worker).await;
        });

        // watch tx messages from tx rpc worker and pass it to p2p to be verified by receiver
        self.handle_incoming_rpc_tx_updates().await?;
        // ============================================
        // run the swarm event listener and handler as a background task
        swarm_result_handle.await?;
        Ok(())
    }
}
