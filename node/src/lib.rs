extern crate alloc;
mod cryptography;
pub mod p2p;
pub mod rpc;
pub mod telemetry;
pub mod tx_processing;

use crate::rpc::TransactionRpcServer;
use alloc::sync::Arc;
use alloy::hex;
use anyhow::{anyhow, Error};
use codec::{Decode, Encode};
use db::DbWorker;
use jsonrpsee::server::ServerBuilder;
use libp2p::futures::{FutureExt, StreamExt};
use libp2p::request_response::{Message, ResponseChannel};
use libp2p::PeerId;
use log::{error, info, warn};
use p2p::P2pWorker;
use primitives::data_structure::{
    new_tx_state_from_mutex, ChainSupported, DbTxStateMachine, Fields, PeerRecord, TxStateMachine,
    TxStatus,
};
use rpc::TransactionRpcWorker;
use std::net::SocketAddr;
use telemetry::TelemetryWorker;
use tinyrand::{Rand, Xorshift};
use tokio::io::AsyncBufReadExt;
use tokio::sync::{Mutex, OwnedMutexGuard};
use tx_processing::TxProcessingWorker;

/// Main thread to be spawned by the application
/// this encompasses all node's logic and processing flow
#[derive(Clone)]
pub struct MainServiceWorker {
    db_worker: Arc<Mutex<DbWorker>>,
    tx_rpc_worker: Arc<Mutex<TransactionRpcWorker>>,
    // for rpc server usage, it is the same as tx_rpc_worker but it is not behind a shared reference
    tx_rpc_worker_cloned: Arc<TransactionRpcWorker>,
    tx_processing_worker: Arc<Mutex<TxProcessingWorker>>,
    p2p_worker: Arc<Mutex<P2pWorker>>,
    // holder for the response channel given by the request
    response_channel: Arc<Mutex<Option<ResponseChannel<Result<Vec<u8>, Error>>>>>, //telemetry_worker: TelemetryWorker,
}

impl MainServiceWorker {
    pub(crate) async fn new() -> Result<Self, anyhow::Error> {
        let (sender_channel, recv_channel) = tokio::sync::mpsc::channel(u8::MAX as usize);
        let shared_recv_channel = Arc::new(Mutex::new(recv_channel));

        let port = Xorshift::default().next_lim_u16(u16::MAX - 100);

        let txn_rpc_worker =
            TransactionRpcWorker::new(shared_recv_channel.clone(), sender_channel.clone(), port)
                .await?;

        let txn_rpc_worker_cloned =
            TransactionRpcWorker::new(shared_recv_channel.clone(), sender_channel, port).await?;

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
            tx_rpc_worker_cloned: Arc::new(txn_rpc_worker_cloned),
            tx_processing_worker: Arc::new(Mutex::new(tx_processing_worker)),
            p2p_worker: Arc::new(Mutex::new(p2p_worker)),
            response_channel: Arc::new(Mutex::new(None)),
        })
    }

    /// handle swarm events; this includes
    /// 1. sender sending requests to receiver to attest ownership and correctness of the recv address
    /// 2. receiver response and sender handling submission of the tx
    pub(crate) async fn handle_swarm_event_messages(
        &self,
        p2p_worker: Arc<Mutex<P2pWorker>>,
        txn_rpc_worker: Arc<Mutex<TransactionRpcWorker>>,
        txn_processing_worker: Arc<Mutex<TxProcessingWorker>>,
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
                                    channel,
                                } => {
                                    let _ = self.response_channel.lock().await.insert(channel);
                                    let decoded_req: TxStateMachine =
                                        Decode::decode(&mut &request[..])
                                            .expect("failed to decode request body");
                                    // send it to be signed via Rpc
                                    txn_rpc_worker
                                        .lock()
                                        .await
                                        .sender_channel
                                        .lock()
                                        .await
                                        .send(Arc::new(Mutex::new(decoded_req)))
                                        .await?
                                }

                                // context of a sender, receiving the response from the target receiver
                                // the sender should;
                                // 1. verify the recv signature public key to the one binded in the multi address
                                // 2. send the tx to be signed to rpc
                                Message::Response {
                                    request_id,
                                    response,
                                } => {
                                    let resp = response.map_err(|err| {
                                        anyhow!("error on the response, caused by; {err}")
                                    })?;
                                    let decoded_resp: TxStateMachine =
                                        Decode::decode(&mut &resp[..])
                                            .expect("failed to decode request body");

                                    txn_processing_worker
                                        .lock()
                                        .await
                                        .validate_receiver_address(&decoded_resp)
                                        .await?;
                                    // send it to be signed via Rpc by the sender
                                    txn_rpc_worker
                                        .lock()
                                        .await
                                        .sender_channel
                                        .lock()
                                        .await
                                        .send(Arc::new(Mutex::new(decoded_resp)))
                                        .await?
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
            self.p2p_worker
                .lock()
                .await
                .send_request(txn.clone(), peer_id)
                .await?;
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
                    self.p2p_worker
                        .lock()
                        .await
                        .send_request(txn.clone(), peer_id)
                        .await?;

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

    /// send the response to the sender via p2p swarm
    pub(crate) async fn handle_recv_addr_confirmed_tx_state(
        &self,
        txn: Arc<Mutex<TxStateMachine>>,
    ) -> Result<(), anyhow::Error> {
        let channel = self.response_channel.lock().await.take().ok_or(anyhow!(
            "response channel not available yet; not yet received any request from p2p swarm"
        ))?;
        if channel.is_open() {
            self.p2p_worker
                .lock()
                .await
                .send_response(channel, txn)
                .await?
        } else {
            Err(anyhow!("response channel timed out"))?
        }
        Ok(())
    }

    /// last stage, submit the txn to rpc to be signed and then submit to the target chain
    pub(crate) async fn handle_sender_confirmed_tx_state(
        &self,
        txn: Arc<Mutex<TxStateMachine>>,
    ) -> Result<(), Error> {
        let txn_guard = txn.lock().await;
        let txn_inner = new_tx_state_from_mutex(txn_guard);

        if let Some(_) = txn_inner.signed_call_payload {
            // TODO! handle submission errors
            // signed and ready to be submitted to target chain
            let tx_hash = self
                .tx_processing_worker
                .lock()
                .await
                .submit_tx(txn_inner.clone())
                .await?;
            // record to local db
            let db_tx = DbTxStateMachine {
                tx_hash: tx_hash.to_vec(),
                amount: txn_inner.amount.clone(),
                network: txn_inner.network.clone(),
                success: true,
            };
            self.db_worker.lock().await.update_success_tx(db_tx).await?;
        } else {
            // not signed yet, send to rpc to be signed
            let to_send_tx = self
                .tx_processing_worker
                .lock()
                .await
                .create_tx(txn_inner)
                .await?;
            self.tx_rpc_worker
                .lock()
                .await
                .sender_channel
                .lock()
                .await
                .send(Arc::new(Mutex::new(to_send_tx)))
                .await?;
        }

        Ok(())
    }

    /// this for now is same as `handle_addr_confirmed_tx_state`
    pub(crate) async fn handle_net_confirmed_tx_state(
        &self,
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
                    self.handle_genesis_tx_state(txn.clone()).await?;
                }
                TxStatus::addrConfirmed => {
                    self.handle_recv_addr_confirmed_tx_state(txn.clone())
                        .await?;
                }
                TxStatus::netConfirmed => {
                    todo!()
                }
                TxStatus::senderConfirmed => {
                    self.handle_sender_confirmed_tx_state(txn.clone()).await?;
                }
            };
        }
        Ok(())
    }

    /// Start rpc server with default url
    pub(crate) async fn start_rpc_server(&self) -> Result<SocketAddr, anyhow::Error> {
        let server_builder = ServerBuilder::new();

        let url = self.tx_rpc_worker.lock().await.url.clone();
        let rpc_handler = Arc::try_unwrap(self.tx_rpc_worker_cloned.clone())
            .map_err(|_err| anyhow!("expected to have only 1 strong reference"))?;

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
    pub async fn run() -> Result<(), anyhow::Error> {
        let main_worker = Self::new().await?;
        // start rpc server
        main_worker.start_rpc_server().await?;

        let p2p_worker = main_worker.p2p_worker.clone();
        let txn_rpc_worker = main_worker.tx_rpc_worker.clone();
        let txn_processing_worker = main_worker.tx_processing_worker.clone();

        let cloned_main_worker = main_worker.clone();
        let rpc_result_handle = tokio::spawn(async move {
            // watch tx messages from tx rpc worker and pass it to p2p to be verified by receiver
            let res = cloned_main_worker.handle_incoming_rpc_tx_updates().await;
        });

        // listen to p2p swarm events
        let swarm_result_handle = tokio::spawn(async move {
            let res = main_worker
                .handle_swarm_event_messages(p2p_worker, txn_rpc_worker, txn_processing_worker)
                .await;
            if let Err(err) = res {
                error!("swarm handle encountered error; caused by {err}");
            } else {
                info!("swarm handle running; handling events messages ✅")
            }
        });

        // ============================================
        // run the swarm and rpc event listener and handler as a background task
        rpc_result_handle.await?;
        swarm_result_handle.await?;
        Ok(())
    }
}
