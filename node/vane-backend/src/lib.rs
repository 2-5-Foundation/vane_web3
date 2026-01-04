pub mod server;

// Re-export commonly used types
pub use server::VaneSwarmServer;

use anyhow::Result;
use log::{error, info};
use primitives::data_structure::{BackendEvent, SystemNotification};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, Mutex};
use server::{JsonRpcServer, MetricService, MetricsServer};

/// Starts the Vane backend server with both JSON-RPC and metrics servers
pub async fn start_backend_servers() -> Result<()> {
    env_logger::Builder::new()
        .format(|buf, record| {
            use std::io::Write;

            writeln!(
                buf,
                "{} {:<5} [{}:{}] [{}] {}",
                chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                record.level(),
                record.file().unwrap_or("unknown"),
                record.line().unwrap_or(0),
                record.target(),
                record.args()
            )
        })
        .filter_level(log::LevelFilter::Info)
        .init();

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

    let metrics_handle = tokio::spawn(async move {
        if let Err(err) = metrics_server.start().await {
            error!("Metrics server error: {}", err);
        }
    });

    let jsonrpc_handle = tokio::spawn(async move {
        if let Err(err) = jsonrpc_server.start().await {
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
