mod p2p;
mod rpc;

use crate::p2p::RelayP2pWorker;
use crate::rpc::RelayServerRpcServer;
use crate::rpc::RelayServerRpcWorker;
use anyhow::anyhow;
use jsonrpsee::server::ServerBuilder;
use libp2p::futures::FutureExt;
use log::{error, info};
pub use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct MainRelayServerService {}

impl MainRelayServerService {
    pub async fn run(dns: String, port: u16, live: bool, private_key: Option<String>) -> Result<Self, anyhow::Error> {
        info!(" 🦀...vane relay server starting...🚀 ");

        let p2p_worker = RelayP2pWorker::new(dns, port, live, private_key).await?;
        let p2p_worker = Arc::new(Mutex::new(p2p_worker));

        let rpc_worker = RelayServerRpcWorker::new();

        // Spawn P2P worker with pure Tokio instead of TaskManager
        let p2p_worker_clone = p2p_worker.clone();
        let p2p_handle = tokio::spawn(async move {
            let mut p2p_worker = p2p_worker_clone.lock().await;
            let swarm_res = p2p_worker.start_swarm().await;
            if let Err(err) = swarm_res {
                error!("swarm start encountered error: caused by {err}");
            }
        });

        Self::start_rpc_server(rpc_worker).await;

        // Wait for P2P task
        p2p_handle.await?;
        Ok(Self {})
    }

    pub async fn start_rpc_server(
        rpc_worker: RelayServerRpcWorker,
    ) -> Result<SocketAddr, anyhow::Error> {
        let server_builder = ServerBuilder::new();
        let url = "[::1]:9944".to_string();

        let server = server_builder.build(url).await?;
        let address = server
            .local_addr()
            .map_err(|err| anyhow!("failed to get address: {}", err))?;
        let handle = server
            .start(rpc_worker.into_rpc())
            .map_err(|err| anyhow!("rpc handler error: {}", err))?;

        tokio::spawn(handle.stopped());
        Ok(address)
    }
}
