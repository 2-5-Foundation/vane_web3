mod server;

use crate::server::{BackendEvent, JsonRpcServer, MetricService, MetricsServer, SystemNotification, VaneSwarmServer};
use anyhow::Result;
use log::{error, info};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, Mutex};

#[tokio::main]
async fn main() -> Result<()> {
    simple_logger::init_with_level(log::Level::Info)?;

    info!("🚀 Starting Vane Backend Server...");

    let metric_service = Arc::new(MetricService::new());
    let (event_sender, _) = broadcast::channel::<BackendEvent>(100);
    let (system_notification_sender, _) = mpsc::channel::<SystemNotification>(100);
    let swarm_server = Arc::new(Mutex::new(VaneSwarmServer::new(
        metric_service.clone(),
        event_sender.clone(),
        system_notification_sender,
    )));

    let metrics_server = MetricsServer::new((*metric_service).clone(), 9946);
    let jsonrpc_server = JsonRpcServer::new(swarm_server, event_sender, 9947)?;

    let metrics_server_clone = metrics_server;
    let metrics_handle = tokio::spawn(async move {
        if let Err(err) = metrics_server_clone.start().await {
            error!("Metrics server error: {}", err);
        }
    });

    let jsonrpc_server_clone = jsonrpc_server;
    let jsonrpc_handle = tokio::spawn(async move {
        if let Err(err) = jsonrpc_server_clone.start().await {
            error!("JSON-RPC server error: {}", err);
        }
    });

    info!("Backend server started successfully");
    info!("Metrics server running on port 9946");
    info!("JSON-RPC server running on port 9947");

    tokio::select! {
        result = metrics_handle => {
            if let Err(err) = result {
                error!("Metrics server task failed: {:?}", err);
            }
        }
        result = jsonrpc_handle => {
            if let Err(err) = result {
                error!("JSON-RPC server task failed: {:?}", err);
            }
        }
    }

    Ok(())
}
