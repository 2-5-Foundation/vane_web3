pub mod cryptography;
pub mod p2p;
pub mod rpc;
pub mod telemetry;
pub mod tx_processing;
use db::DbWorker;
use p2p::P2pWorker;
pub use primitives;
use rpc::RpcWorker;
use telemetry::TelemetryWorker;
use tx_processing::TxProcessingWorker;

/// Main thread to be spawned by the application
/// this encompasses all node's logic and processing flow
pub struct MainServiceWorker {
    db_worker: DbWorker,
    rpc_worker: RpcWorker,
    tx_processing_worker: TxProcessingWorker,
    p2p_worker: P2pWorker,
    telemetry_worker: TelemetryWorker,
}

impl MainServiceWorker {}
