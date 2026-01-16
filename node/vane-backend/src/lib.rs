pub mod server;
pub mod db;
#[cfg(test)]
pub mod checks;

// Re-export commonly used types
pub use server::VaneSwarmServer;
pub use db::{D1Client, D1Config, TxLifecycle, MetricsCounter};

use anyhow::Result;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use log::{error, info, warn};
use primitives::data_structure::{BackendEvent, SystemNotification, TxStateMachine};
use server::{ClientSnapshot, JsonRpcServer, MetricService, verify_client_key_middleware};
use hex;
use std::collections::HashSet;
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
            .route("/client-metrics-summary", get(get_client_metrics_summary))
            .route("/tx-metrics", get(get_tx_metrics))
            .route("/getTxCounts", get(get_tx_counts))
            .route("/getTotalTxCount", get(get_total_tx_count))
            .route("/getReceiverConfirmedCount", get(get_receiver_confirmed_count))
            .route("/getRevertedCount", get(get_reverted_count))
            .route("/getCompletedCount", get(get_completed_count))
            .route("/getRevertedBeforeConfirmCount", get(get_reverted_before_confirm_count))
            .route("/tx-json-by-sender/{sender}", get(get_tx_json_by_sender))
            .route("/tx-json-by-receiver/{receiver}", get(get_tx_json_by_receiver))
            .route("/tx-json-by-pair/{sender}/{receiver}", get(get_tx_json_by_pair))
            .route("/tx-lifecycle-by-pair/{sender}/{receiver}", get(get_tx_lifecycle_by_pair))
            .with_state(self.service.clone());

        let tcp_listener = TcpListener::bind(addr).await?;
        let local_addr = tcp_listener.local_addr()?;

        info!("Backend metrics server listening on http://{}", local_addr);
        info!(
            "Metrics summary endpoint (GET): http://{}/metrics-summary",
            local_addr
        );
        info!(
            "Client metrics summary endpoint (GET): http://{}/client-metrics-summary",
            local_addr
        );
        info!(
            "Transaction metrics endpoint (GET): http://{}/tx-metrics",
            local_addr
        );
        info!(
            "Transaction counts endpoint (GET): http://{}/tx-counts",
            local_addr
        );
        info!(
            "Transaction JSON by sender endpoint (GET): http://{}/tx-json-by-sender/{{sender}}",
            local_addr
        );
        info!(
            "Transaction JSON by receiver endpoint (GET): http://{}/tx-json-by-receiver/{{receiver}}",
            local_addr
        );
        info!(
            "Transaction JSON by pair endpoint (GET): http://{}/tx-json-by-pair/{{sender}}/{{receiver}}",
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
                "active_peers": metrics.active_peers.get() as usize,
                "pending_requests": metrics.pending_requests.get() as usize,
                "new_clients_joined": metrics.new_clients_joined.get(),
            }
        })),
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

// Helper function to parse amount from tx_json
fn parse_amount_from_tx_json(tx_json: &str) -> u128 {
    match serde_json::from_str::<TxStateMachine>(tx_json) {
        Ok(tx) => tx.amount,
        Err(e) => {
            error!("Failed to parse tx_json for amount: {}", e);
            0
        }
    }
}

// Helper function to parse sender_address from tx_json (fallback to db field)
fn get_sender_from_tx(tx_json: &str, fallback_sender: &str) -> String {
    match serde_json::from_str::<TxStateMachine>(tx_json) {
        Ok(tx) => tx.sender_address,
        Err(_) => fallback_sender.to_string(),
    }
}

// Compute the 7 metrics from database
async fn compute_tx_metrics(db: &D1Client) -> Result<serde_json::Value> {
    // 1. Senders completed successfully (distinct senders where completed=1)
    let completed_txs = db.get_tx_lifecycle_filtered(Some(true), None, None).await?;
    let senders_completed: HashSet<String> = completed_txs
        .iter()
        .map(|tx| get_sender_from_tx(&tx.tx_json, &tx.sender_address))
        .collect();
    
    // 2. Senders reverted before receiver confirmation (reverted=1 AND receiver_confirmed=0)
    let reverted_before_confirm = db
        .get_tx_lifecycle_filtered(None, Some(true), Some(false))
        .await?;
    let senders_reverted_before: HashSet<String> = reverted_before_confirm
        .iter()
        .map(|tx| get_sender_from_tx(&tx.tx_json, &tx.sender_address))
        .collect();

    // 3. Senders reverted after receiver confirmation (reverted=1 AND receiver_confirmed=1)
    let reverted_after_confirm = db
        .get_tx_lifecycle_filtered(None, Some(true), Some(true))
        .await?;
    let senders_reverted_after: HashSet<String> = reverted_after_confirm
        .iter()
        .map(|tx| get_sender_from_tx(&tx.tx_json, &tx.sender_address))
        .collect();

    // 4. Value from reverted tx (sum amount from tx_json where reverted=1)
    let all_reverted = db.get_tx_lifecycle_filtered(None, Some(true), None).await?;
    let value_reverted: u128 = all_reverted
        .iter()
        .map(|tx| parse_amount_from_tx_json(&tx.tx_json))
        .sum();

    // 5. Value from successful tx (sum amount from tx_json where completed=1)
    let value_successful: u128 = completed_txs
        .iter()
        .map(|tx| parse_amount_from_tx_json(&tx.tx_json))
        .sum();

    // 6. Receivers who confirmed (distinct receivers where receiver_confirmed=1)
    let confirmed_txs = db.get_tx_lifecycle_filtered(None, None, Some(true)).await?;
    let receivers_confirmed: HashSet<String> = confirmed_txs
        .iter()
        .map(|tx| tx.receiver_address.clone())
        .collect();

    // 7. Receivers who confirmed then sender reverted (receiver_confirmed=1 AND reverted=1)
    let receivers_confirmed_reverted: HashSet<String> = reverted_after_confirm
        .iter()
        .map(|tx| tx.receiver_address.clone())
        .collect();

    Ok(serde_json::json!({
        "senders_completed_successfully": senders_completed.len(),
        "senders_reverted_before_confirmation": senders_reverted_before.len(),
        "senders_reverted_after_confirmation": senders_reverted_after.len(),
        "value_reverted_tx": value_reverted.to_string(), // Use string for large u128
        "value_successful_tx": value_successful.to_string(),
        "receivers_who_confirmed": receivers_confirmed.len(),
        "receivers_confirmed_then_sender_reverted": receivers_confirmed_reverted.len(),
    }))
}

async fn get_tx_metrics(State(service): State<MetricService>) -> impl IntoResponse {
    if let Some(db) = service.get_db() {
        match compute_tx_metrics(&db).await {
            Ok(metrics) => (StatusCode::OK, Json(metrics)),
            Err(e) => {
                error!("Failed to compute tx metrics: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": format!("Failed to compute metrics: {}", e)})),
                )
            }
        }
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "Database not available"})),
        )
    }
}

async fn get_tx_json_by_sender(
    Path(sender): Path<String>,
    State(service): State<MetricService>,
) -> impl IntoResponse {
    if let Some(db) = service.get_db() {
        match db.get_tx_json_by_sender(&sender).await {
            Ok(tx_jsons) => (StatusCode::OK, Json(serde_json::json!({"tx_json": tx_jsons}))),
            Err(e) => {
                error!("Failed to get tx json by sender: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": format!("Failed to query: {}", e)})),
                )
            }
        }
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "Database not available"})),
        )
    }
}

async fn get_tx_json_by_receiver(
    Path(receiver): Path<String>,
    State(service): State<MetricService>,
) -> impl IntoResponse {
    if let Some(db) = service.get_db() {
        match db.get_tx_json_by_receiver(&receiver).await {
            Ok(tx_jsons) => (StatusCode::OK, Json(serde_json::json!({"tx_json": tx_jsons}))),
            Err(e) => {
                error!("Failed to get tx json by receiver: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": format!("Failed to query: {}", e)})),
                )
            }
        }
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "Database not available"})),
        )
    }
}

async fn get_tx_json_by_pair(
    Path((sender, receiver)): Path<(String, String)>,
    State(service): State<MetricService>,
) -> impl IntoResponse {
    if let Some(db) = service.get_db() {
        match db.get_tx_json_by_pair(&sender, &receiver).await {
            Ok(tx_jsons) => (StatusCode::OK, Json(serde_json::json!({"tx_json": tx_jsons}))),
            Err(e) => {
                error!("Failed to get tx json by pair: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": format!("Failed to query: {}", e)})),
                )
            }
        }
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "Database not available"})),
        )
    }
}

async fn get_tx_lifecycle_by_pair(
    Path((sender, receiver)): Path<(String, String)>,
    State(service): State<MetricService>,
) -> impl IntoResponse {
    if let Some(db) = service.get_db() {
        match db.get_tx_lifecycle_by_pair(&sender, &receiver).await {
            Ok(txs) => (StatusCode::OK, Json(serde_json::json!({"transactions": txs}))),
            Err(e) => {
                error!("Failed to get tx lifecycle by pair: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": format!("Failed to query: {}", e)})),
                )
            }
        }
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "Database not available"})),
        )
    }
}

#[derive(Deserialize)]
struct TxCountsParams {
    address: String,
    sig: String, // hex-encoded signature
}

async fn get_tx_counts(
    Query(params): Query<TxCountsParams>,
    State(service): State<MetricService>,
) -> impl IntoResponse {
    // Decode signature from hex
    let sig = match hex::decode(&params.sig) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("Invalid signature format: {}", e)})),
            );
        }
    };

    // Verify signature
    match verify_client_key_middleware(sig, params.address.clone()) {
        Ok(true) => {}
        Ok(false) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "Invalid client verification"})),
            );
        }
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("Signature verification failed: {}", e)})),
            );
        }
    }

    match service.get_tx_counts().await {
        Ok((total, receiver_confirmed, reverted, completed, reverted_before_confirm)) => {
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "total": total,
                    "receiver_confirmed": receiver_confirmed,
                    "reverted": reverted,
                    "completed": completed,
                    "reverted_before_confirm": reverted_before_confirm
                })),
            )
        }
        Err(e) => {
            error!("Failed to get tx counts: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Failed to get tx counts: {}", e)})),
            )
        }
    }
}

async fn get_total_tx_count(
    Query(params): Query<TxCountsParams>,
    State(service): State<MetricService>,
) -> impl IntoResponse {
    let sig = match hex::decode(&params.sig) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("Invalid signature format: {}", e)})),
            );
        }
    };

    match verify_client_key_middleware(sig, params.address.clone()) {
        Ok(true) => {}
        Ok(false) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "Invalid client verification"})),
            );
        }
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("Signature verification failed: {}", e)})),
            );
        }
    }

    match service.get_total_tx_count().await {
        Ok(count) => {
            (
                StatusCode::OK,
                Json(serde_json::json!({"total": count})),
            )
        }
        Err(e) => {
            error!("Failed to get total tx count: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Failed to get total tx count: {}", e)})),
            )
        }
    }
}

async fn get_receiver_confirmed_count(
    Query(params): Query<TxCountsParams>,
    State(service): State<MetricService>,
) -> impl IntoResponse {
    let sig = match hex::decode(&params.sig) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("Invalid signature format: {}", e)})),
            );
        }
    };

    match verify_client_key_middleware(sig, params.address.clone()) {
        Ok(true) => {}
        Ok(false) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "Invalid client verification"})),
            );
        }
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("Signature verification failed: {}", e)})),
            );
        }
    }

    match service.get_receiver_confirmed_count().await {
        Ok(count) => {
            (
                StatusCode::OK,
                Json(serde_json::json!({"receiver_confirmed": count})),
            )
        }
        Err(e) => {
            error!("Failed to get receiver confirmed count: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Failed to get receiver confirmed count: {}", e)})),
            )
        }
    }
}

async fn get_reverted_count(
    Query(params): Query<TxCountsParams>,
    State(service): State<MetricService>,
) -> impl IntoResponse {
    let sig = match hex::decode(&params.sig) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("Invalid signature format: {}", e)})),
            );
        }
    };

    match verify_client_key_middleware(sig, params.address.clone()) {
        Ok(true) => {}
        Ok(false) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "Invalid client verification"})),
            );
        }
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("Signature verification failed: {}", e)})),
            );
        }
    }

    match service.get_reverted_count().await {
        Ok(count) => {
            (
                StatusCode::OK,
                Json(serde_json::json!({"reverted": count})),
            )
        }
        Err(e) => {
            error!("Failed to get reverted count: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Failed to get reverted count: {}", e)})),
            )
        }
    }
}

async fn get_completed_count(
    Query(params): Query<TxCountsParams>,
    State(service): State<MetricService>,
) -> impl IntoResponse {
    let sig = match hex::decode(&params.sig) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("Invalid signature format: {}", e)})),
            );
        }
    };

    match verify_client_key_middleware(sig, params.address.clone()) {
        Ok(true) => {}
        Ok(false) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "Invalid client verification"})),
            );
        }
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("Signature verification failed: {}", e)})),
            );
        }
    }

    match service.get_completed_count().await {
        Ok(count) => {
            (
                StatusCode::OK,
                Json(serde_json::json!({"completed": count})),
            )
        }
        Err(e) => {
            error!("Failed to get completed count: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Failed to get completed count: {}", e)})),
            )
        }
    }
}

async fn get_reverted_before_confirm_count(
    Query(params): Query<TxCountsParams>,
    State(service): State<MetricService>,
) -> impl IntoResponse {
    let sig = match hex::decode(&params.sig) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("Invalid signature format: {}", e)})),
            );
        }
    };

    match verify_client_key_middleware(sig, params.address.clone()) {
        Ok(true) => {}
        Ok(false) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "Invalid client verification"})),
            );
        }
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("Signature verification failed: {}", e)})),
            );
        }
    }

    match service.get_reverted_before_confirm_count().await {
        Ok(count) => {
            (
                StatusCode::OK,
                Json(serde_json::json!({"reverted_before_confirm": count})),
            )
        }
        Err(e) => {
            error!("Failed to get reverted before confirm count: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Failed to get reverted before confirm count: {}", e)})),
            )
        }
    }
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

    // Initialize D1Client if environment variables are set
    let db = match crate::db::D1Config::from_env() {
        Ok(config) => {
            info!("D1 database configuration found, initializing client...");
            Some(Arc::new(crate::db::D1Client::new(config)))
        }
        Err(e) => {
            warn!("D1 database not configured: {}. Running without database.", e);
            None
        }
    };

    let metric_service = Arc::new(MetricService::new(db.clone()));
    let (event_sender, _) = broadcast::channel::<BackendEvent>(100);
    let (system_notification_sender, _) = mpsc::channel::<SystemNotification>(100);
    let swarm_server = Arc::new(Mutex::new(VaneSwarmServer::new(
        metric_service.clone(),
        event_sender.clone(),
        system_notification_sender,
        request_ttl_seconds,
        db.clone(), // Pass db to VaneSwarmServer
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
