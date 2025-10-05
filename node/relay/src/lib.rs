mod metric_server;
mod p2p;

use crate::metric_server::{metrics_server, MetricService};
use crate::p2p::RelayP2pWorker;
use anyhow::anyhow;
use log::{error, info};
pub use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct MainRelayServerService {}

impl MainRelayServerService {
    pub async fn run(
        dns: String,
        port: u16,
        live: bool,
        private_key: Option<String>,
    ) -> Result<Self, anyhow::Error> {
        info!(" 🦀...vane relay server starting...🚀 ");

        let p2p_worker = RelayP2pWorker::new(dns, port, live, private_key).await?;
        let p2p_worker = Arc::new(Mutex::new(p2p_worker));

        // Create one shared metric service
        let metric_service = MetricService::new();

        // Start metrics server on a different port, using the same service
        let metrics_port = 9945;
        let metrics_service_clone = metric_service.clone();
        let metrics_handle = tokio::spawn(async move {
            if let Err(err) = metrics_server(metrics_service_clone, metrics_port).await {
                error!("Metrics server error: {}", err);
            }
        });

        // Spawn P2P worker with pure Tokio instead of TaskManager
        let p2p_worker_clone = p2p_worker.clone();
        let metrics_for_p2p = metric_service.clone();
        let p2p_handle = tokio::spawn(async move {
            let mut p2p_worker = p2p_worker_clone.lock().await;
            p2p_worker.metrics = std::sync::Arc::new(metrics_for_p2p);
            let swarm_res = p2p_worker.start_swarm().await;
            if let Err(err) = swarm_res {
                error!("swarm start encountered error: caused by {err}");
            }
        });

        // Start metrics server independently (don't block on it)
        tokio::spawn(async move {
            if let Err(err) = metrics_handle.await {
                error!("Metrics server task failed: {:?}", err);
            }
        });

        // Only wait for P2P (the critical service)
        if let Err(err) = p2p_handle.await {
            error!("P2P worker task error: {:?}", err);
        }

        Ok(Self {})
    }
}
