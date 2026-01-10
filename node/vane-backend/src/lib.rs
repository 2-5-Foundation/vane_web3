pub mod server;

#[cfg(test)]
pub mod checks;

// Re-export commonly used types
pub use server::VaneSwarmServer;

use anyhow::Result;
use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use log::{error, info};
use primitives::data_structure::{BackendEvent, DbTxStateMachine, StorageExport, SystemNotification};
use server::{ClientMetricsPayload, ClientSnapshot, JsonRpcServer, MetricService};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::{
    net::TcpListener,
    sync::{broadcast, mpsc, Mutex},
};

pub struct MetricsServer {
    service: MetricService,
    port: u16,
}

impl MetricsServer {
    pub fn new(service: MetricService, port: u16) -> Self {
        Self { service, port }
    }

    pub async fn start(&self) -> Result<()> {
        let addr: SocketAddr = ([127, 0, 0, 1], self.port).into();

        let app = Router::new()
            .route("/metrics-summary", get(get_metrics_summary))
            .route("/client-metrics", post(handle_client_metrics))
            .route("/client-metrics-summary", get(get_client_metrics_summary))
            .with_state(self.service.clone());

        let tcp_listener = TcpListener::bind(addr).await?;
        let local_addr = tcp_listener.local_addr()?;

        info!("Backend metrics server listening on http://{}", local_addr);
        info!(
            "Metrics summary endpoint (GET): http://{}/metrics-summary",
            local_addr
        );
        info!(
            "Client metrics endpoint (POST): http://{}/client-metrics",
            local_addr
        );
        info!(
            "Client metrics summary endpoint (GET): http://{}/client-metrics-summary",
            local_addr
        );

        axum::serve(tcp_listener, app.into_make_service()).await?;
        Ok(())
    }
}

async fn get_metrics_summary(State(service): State<MetricService>) -> impl IntoResponse {
    let metrics_arc = service.get_backend_metrics();
    let metrics = metrics_arc.lock().await;
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "backend": {
                "sender_requests_total": metrics.sender_requests_total.get(),
                "receiver_responses_total": metrics.receiver_responses_total.get(),
                "receiver_not_found_total": metrics.receiver_not_found_total.get(),
                "active_peers": metrics.active_peers.get() as usize,
                "pending_requests": metrics.pending_requests.get() as usize,
                "new_clients_joined": metrics.new_clients_joined.get(),
            }
        })),
    )
}

async fn handle_client_metrics(
    State(service): State<MetricService>,
    Json(payload): Json<ClientMetricsPayload>,
) -> impl IntoResponse {
    let account_id = payload
        .storage_export
        .user_account
        .as_ref()
        .and_then(|ua| ua.accounts.first())
        .map(|(acc, _)| acc.clone())
        .unwrap_or_else(|| "unknown".to_string());

    info!(
        "Received storage export from client: {} account: {} ({})",
        payload.peer_id, account_id, payload.client_type
    );

    let client_metrics_arc = service.get_client_metrics();
    let mut client_metrics = client_metrics_arc.lock().await;
    client_metrics.update_from_exported_storage(payload);
    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "success"})),
    )
}

async fn get_client_metrics_summary(State(service): State<MetricService>) -> impl IntoResponse {
    let client_metrics_arc = service.get_client_metrics();
    let client_metrics = client_metrics_arc.lock().await;
    let list: Vec<ClientSnapshot> = client_metrics.per_client.values().cloned().collect();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "clients": list
        })),
    )
}

/// Starts the Vane backend server with both JSON-RPC and metrics servers
pub async fn start_backend_servers(request_ttl_seconds: u64) -> Result<()> {
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
        request_ttl_seconds,
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
