extern crate alloc;
mod cryptography;
pub mod p2p;
pub mod rpc;
pub mod telemetry;
pub mod tx_processing;

use alloc::sync::Arc;
use anyhow::anyhow;
use db::DbWorker;
use p2p::P2pWorker;
pub use primitives;
use primitives::data_structure::{ChainSupported, Fields, PeerRecord, TxStateMachine};
use rpc::TransactionRpcWorker;
use telemetry::TelemetryWorker;
use tokio::io::AsyncBufReadExt;
use tokio::sync::Mutex;
use tx_processing::TxProcessingWorker;

/// Main thread to be spawned by the application
/// this encompasses all node's logic and processing flow
pub struct MainServiceWorker {
    db_worker: Arc<Mutex<DbWorker>>,
    tx_rpc_worker: TransactionRpcWorker,
    tx_processing_worker: TxProcessingWorker,
    p2p_worker: P2pWorker,
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
            tx_rpc_worker: txn_rpc_worker,
            tx_processing_worker,
            p2p_worker,
        })
    }
    pub async fn run(&self) -> Result<(), anyhow::Error> {
        // watch tx messages from tx rpc worker and pass it to p2p to be verified by receiver
        while let Some(txs) = self
            .tx_rpc_worker
            .receiver_channel
            .lock()
            .await
            .recv()
            .await
        {}
        Ok(())
    }
}
