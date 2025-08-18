mod p2p;
mod rpc;
mod telemetry;

use crate::p2p::RelayP2pWorker;
use crate::rpc::RelayServerRpcServer;
use crate::rpc::RelayServerRpcWorker;
use crate::telemetry::{MetricsCounters, RelayServerMetrics, TelemetryWorker};
use anyhow::anyhow;
use jsonrpsee::server::ServerBuilder;
use libp2p::futures::FutureExt;
use log::{error, info};
pub use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct MainRelayServerService {}

impl MainRelayServerService {
    pub async fn run(dns: String, port: u16, live: bool) -> Result<Self, anyhow::Error> {
        info!(" 🦀...vane relay server starting...🚀 ");

        let (relay_metrics_sender_channel, relay_metrics_recv_channel) =
            tokio::sync::mpsc::channel::<RelayServerMetrics>(10);

        let metrics_counters = Arc::new(MetricsCounters::default());

        let p2p_worker = RelayP2pWorker::new(dns, port, live, metrics_counters.clone())?;
        let p2p_worker = Arc::new(Mutex::new(p2p_worker));

        let mut telemetry_worker = TelemetryWorker::new(metrics_counters);
        telemetry_worker.set_swarm(p2p_worker.lock().await.get_swarm());

        let rpc_worker = RelayServerRpcWorker::new(relay_metrics_recv_channel);

        let tokio_handle = tokio::runtime::Handle::current();
        let mut task_manager = sc_service::TaskManager::new(tokio_handle, None)?;

        {
            let p2p_worker = p2p_worker.clone();
            task_manager.spawn_essential_handle().spawn_blocking(
                Box::leak(Box::new("p2p_swarm".to_string())),
                "p2p_swarm",
                async move {
                    let mut p2p_worker = p2p_worker.lock().await;
                    let swarm_res = p2p_worker.start_swarm().await;
                    if let Err(err) = swarm_res {
                        error!("swarm start encountered error: caused by {err}");
                    }
                }
                .boxed(),
            )
        }

        {
            let telemetry_worker = Arc::new(telemetry_worker);
            let metrics_tx = relay_metrics_sender_channel;
            task_manager.spawn_essential_handle().spawn_blocking(
                Box::leak(Box::new("telemetry_metrics".to_string())),
                "telemetry_metrics",
                async move {
                    telemetry_worker.start_metrics_collection(metrics_tx).await;
                }
                .boxed(),
            )
        }

        Self::start_rpc_server(rpc_worker).await;

        task_manager.future().await?;
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
