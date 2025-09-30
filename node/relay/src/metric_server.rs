use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use prometheus_client::{
    encoding::text::encode,
    metrics::{counter::Counter, gauge::Gauge},
    registry::Registry,
};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use log::{info, warn, error};
use anyhow::Result;
use primitives::data_structure::{StorageExport, UserAccount, DbTxStateMachine, SavedPeerInfo, ChainSupported};

const METRICS_CONTENT_TYPE: &str = "application/openmetrics-text;charset=utf-8;version=1.0.0";
#[derive(Debug, Clone, serde::Serialize)]
pub struct PeerPair {
    pub src_peer_id: String,
    pub dst_peer_id: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CircuitClosedDetail {
    pub src_peer_id: String,
    pub dst_peer_id: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CircuitDeniedDetail {
    pub src_peer_id: String,
    pub dst_peer_id: String,
    pub status: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ReservationDeniedDetail {
    pub src_peer_id: String,
    pub status: String,
}
#[derive(Debug, Clone, serde::Serialize)]
pub struct ClientSnapshot {
    pub peer_id: String,
    pub success_txs: u64,
    pub failed_txs: u64,
    pub total_value_success: u64,
    pub total_value_failed: u64,
    pub saved_peers: u64,
}


// Simplified: no label-based families

#[derive(Debug, Deserialize, Serialize)]
pub struct ClientMetricsPayload {
    pub peer_id: String,
    pub client_type: String, // "wasm" or "native"
    pub timestamp: u64,
    pub storage_export: StorageExport,
}

pub struct RelayMetrics {
    pub circuits_accepted_total: Counter,
    pub circuits_closed_total: Counter,
    pub circuits_denied_total: Counter,
    pub reservations_accepted_total: Counter,
    pub reservations_closed_total: Counter,
    pub reservations_denied_total: Counter,
    pub reservations_timed_out_total: Counter,
    pub connections_total: Counter,
    // Per-peer details
    pub reservation_accepted_peers: std::collections::HashSet<String>,
    pub reservation_closed_peers: std::collections::HashSet<String>,
    pub circuit_accepted_pairs: Vec<PeerPair>,
    pub circuit_closed_pairs: Vec<CircuitClosedDetail>,
    pub circuit_denied: Vec<CircuitDeniedDetail>,
    pub reservation_denied: Vec<ReservationDeniedDetail>,
    pub reservation_timed_out_peers: Vec<String>,
}

pub struct ClientMetricsStore {
    pub last_update_times: HashMap<String, SystemTime>,
    pub last_totals: HashMap<String, (u64, u64, u64, u64, u64)>, // success_cnt, failed_cnt, value_success, value_failed, peers_cnt
    // Aggregates
    pub total_success_txs: Counter,
    pub total_failed_txs: Counter,
    pub total_value_success: Counter,
    pub total_value_failed: Counter,
    pub total_saved_peers: Counter,
    pub tx_success_rate_percent: Gauge,
    // Per-client snapshots
    pub per_client: HashMap<String, ClientSnapshot>,
}

// No additional label structs

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct TransactionMetricLabels {
    pub peer_id: String,
    pub account_id: String,
    pub client_type: String,
    pub status: String, // "success" or "failed"
    pub sender_network: String,
    pub receiver_network: String,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct ValueMetricLabels {
    pub peer_id: String,
    pub account_id: String,
    pub client_type: String,
    pub status: String, // "success" or "failed"
    pub value_type: String, // "total", "average", "count"
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct PeerNetworkLabels {
    pub reporting_peer_id: String,
    pub target_peer_id: String,
    pub client_type: String,
    pub network_type: String, // from ChainSupported
}

impl RelayMetrics {
    pub fn new() -> Self {
        Self {
            circuits_accepted_total: Counter::default(),
            circuits_closed_total: Counter::default(),
            circuits_denied_total: Counter::default(),
            reservations_accepted_total: Counter::default(),
            reservations_closed_total: Counter::default(),
            reservations_denied_total: Counter::default(),
            reservations_timed_out_total: Counter::default(),
            connections_total: Counter::default(),
            reservation_accepted_peers: std::collections::HashSet::new(),
            reservation_closed_peers: std::collections::HashSet::new(),
            circuit_accepted_pairs: Vec::new(),
            circuit_closed_pairs: Vec::new(),
            circuit_denied: Vec::new(),
            reservation_denied: Vec::new(),
            reservation_timed_out_peers: Vec::new(),
        }
    }

    pub fn register(&self, registry: &mut Registry) {
        registry.register("relay_circuits_accepted_total", "Total circuits accepted", self.circuits_accepted_total.clone());
        registry.register("relay_circuits_closed_total", "Total circuits closed", self.circuits_closed_total.clone());
        registry.register("relay_circuits_denied_total", "Total circuits denied", self.circuits_denied_total.clone());
        registry.register("relay_reservations_accepted_total", "Total reservations accepted", self.reservations_accepted_total.clone());
        registry.register("relay_reservations_closed_total", "Total reservations closed", self.reservations_closed_total.clone());
        registry.register("relay_reservations_denied_total", "Total reservations denied", self.reservations_denied_total.clone());
        registry.register("relay_reservations_timed_out_total", "Total reservations timed out", self.reservations_timed_out_total.clone());
        registry.register("relay_connections_total", "Total connection events", self.connections_total.clone());
    }
}

impl ClientMetricsStore {
    pub fn new() -> Self {
        Self {
            last_update_times: HashMap::new(),
            last_totals: HashMap::new(),
            total_success_txs: Counter::default(),
            total_failed_txs: Counter::default(),
            total_value_success: Counter::default(),
            total_value_failed: Counter::default(),
            total_saved_peers: Counter::default(),
            tx_success_rate_percent: Gauge::default(),
            per_client: HashMap::new(),
        }
    }

    pub fn register(&self, registry: &mut Registry) {
        registry.register("total_success_txs", "Total successful transactions", self.total_success_txs.clone());
        registry.register("total_failed_txs", "Total failed transactions", self.total_failed_txs.clone());
        registry.register("total_value_success", "Total value of successful transactions", self.total_value_success.clone());
        registry.register("total_value_failed", "Total value of failed transactions", self.total_value_failed.clone());
        registry.register("total_saved_peers", "Total saved peers", self.total_saved_peers.clone());
        registry.register("tx_success_rate_percent", "Transaction success rate (percent)", self.tx_success_rate_percent.clone());
    }

    pub fn update_from_exported_storage(&mut self, payload: ClientMetricsPayload) {
        let peer_id = payload.peer_id.clone();
        let client_type = payload.client_type.clone();
        let storage = &payload.storage_export;
        
        // Update last seen timestamp
        self.last_update_times.insert(peer_id.clone(), SystemTime::now());
        
        // Extract account_id from user_account if available
        let account_id = storage.user_account
            .as_ref()
            .and_then(|ua| ua.accounts.first())
            .map(|(acc, _)| acc.clone())
            .unwrap_or_else(|| "unknown".to_string());
        
        // Process basic metrics
        self.process_basic_metrics(&peer_id, &account_id, &client_type, storage);
        
        // Process transaction metrics
        self.process_transaction_metrics(&peer_id, &account_id, &client_type, storage);
        
        // Process value metrics
        self.process_value_metrics(&peer_id, &account_id, &client_type, storage);
        
        // Process peer network metrics
        self.process_peer_network_metrics(&peer_id, &client_type, storage);
        
        // Update aggregated system metrics
        self.update_aggregated_metrics(&client_type, storage);
        
        info!(
            "Updated metrics for peer {} account {} ({}) - {} success txs, {} failed txs, {} saved peers", 
            peer_id, 
            account_id, 
            client_type,
            storage.success_transactions.len(),
            storage.failed_transactions.len(),
            storage.all_saved_peers.len()
        );
    }
    
    fn process_basic_metrics(&mut self, _peer_id: &str, _account_id: &str, _client_type: &str, _storage: &StorageExport) {}
    
    fn process_transaction_metrics(&mut self, _peer_id: &str, _account_id: &str, _client_type: &str, _storage: &StorageExport) {}
    
    fn process_value_metrics(&mut self, _peer_id: &str, _account_id: &str, _client_type: &str, _storage: &StorageExport) {}
    
    fn process_peer_network_metrics(&mut self, _peer_id: &str, _client_type: &str, _storage: &StorageExport) {}
    
    fn infer_network_from_account(&self, account_id: &str) -> String {
        if account_id.starts_with("0x") && account_id.len() == 42 {
            "ethereum".to_string()
        } else if account_id.len() == 48 || account_id.starts_with("5") {
            "polkadot".to_string()
        } else if account_id.len() == 44 {
            "solana".to_string()
        } else if account_id.starts_with("T") {
            "tron".to_string()
        } else {
            "unknown".to_string()
        }
    }
    
    fn update_aggregated_metrics(&mut self, peer_id: &str, storage: &StorageExport) {
        // Compute deltas per peer to avoid double counting
        let current = (
            storage.success_transactions.len() as u64,
            storage.failed_transactions.len() as u64,
            storage.total_value_success as u64,
            storage.total_value_failed as u64,
            storage.all_saved_peers.len() as u64,
        );

        let last = self.last_totals.get(peer_id).cloned().unwrap_or((0, 0, 0, 0, 0));
        let delta_success = current.0.saturating_sub(last.0);
        let delta_failed = current.1.saturating_sub(last.1);
        let delta_val_success = current.2.saturating_sub(last.2);
        let delta_val_failed = current.3.saturating_sub(last.3);
        let delta_peers = current.4.saturating_sub(last.4);

        if delta_success > 0 { self.total_success_txs.inc_by(delta_success); }
        if delta_failed > 0 { self.total_failed_txs.inc_by(delta_failed); }
        if delta_val_success > 0 { self.total_value_success.inc_by(delta_val_success); }
        if delta_val_failed > 0 { self.total_value_failed.inc_by(delta_val_failed); }
        if delta_peers > 0 { self.total_saved_peers.inc_by(delta_peers); }

        // Success rate using current snapshot
        let total_tx = current.0 + current.1;
        if total_tx > 0 {
            let success_rate = (current.0 as f64 / total_tx as f64) * 100.0;
            self.tx_success_rate_percent.set(success_rate.round() as i64);
        }

        self.last_totals.insert(peer_id.to_string(), current);

        // Update per-client snapshot
        self.per_client.insert(
            peer_id.to_string(),
            ClientSnapshot {
                peer_id: peer_id.to_string(),
                success_txs: current.0,
                failed_txs: current.1,
                total_value_success: current.2,
                total_value_failed: current.3,
                saved_peers: current.4,
            },
        );
    }
    
    pub fn cleanup_stale_clients(&mut self, max_age: Duration) {
        let now = SystemTime::now();
        let mut stale_peers = Vec::new();
        
        for (peer_id, last_update) in &self.last_update_times {
            if let Ok(elapsed) = now.duration_since(*last_update) {
                if elapsed > max_age {
                    stale_peers.push(peer_id.clone());
                }
            }
        }
        
        for peer_id in stale_peers {
            self.last_update_times.remove(&peer_id);
            // Note: prometheus_client doesn't support removing metrics dynamically
            // In production, you might want to use a different approach or 
            // implement custom cleanup logic
            warn!("Client {} metrics are stale and should be cleaned up", peer_id);
        }
    }
}

#[derive(Clone)]
pub struct MetricService {
    reg: Arc<Mutex<Registry>>,
    relay_metrics: Arc<Mutex<RelayMetrics>>,
    client_metrics: Arc<Mutex<ClientMetricsStore>>,
}

impl MetricService {
    pub fn new() -> Self {
        let mut registry = Registry::default();
        let relay_metrics = RelayMetrics::new();
        let client_metrics = ClientMetricsStore::new();
        
        relay_metrics.register(&mut registry);
        client_metrics.register(&mut registry);
        
        Self {
            reg: Arc::new(Mutex::new(registry)),
            relay_metrics: Arc::new(Mutex::new(relay_metrics)),
            client_metrics: Arc::new(Mutex::new(client_metrics)),
        }
    }

    pub fn get_reg(&self) -> Arc<Mutex<Registry>> {
        Arc::clone(&self.reg)
    }
    
    pub fn get_relay_metrics(&self) -> Arc<Mutex<RelayMetrics>> {
        Arc::clone(&self.relay_metrics)
    }
    
    pub fn get_client_metrics(&self) -> Arc<Mutex<ClientMetricsStore>> {
        Arc::clone(&self.client_metrics)
    }
}

pub async fn metrics_server(service: MetricService, port: u16) -> Result<()> {
    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    
    // Start cleanup task for stale client metrics
    let client_metrics_cleanup = service.get_client_metrics();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(300)); // 5 minutes
        loop {
            interval.tick().await;
            if let Ok(mut metrics) = client_metrics_cleanup.try_lock() {
                metrics.cleanup_stale_clients(Duration::from_secs(600)); // 10 minutes
            }
        }
    });
    
    let app = Router::new()
        .route("/metrics", get(respond_with_metrics))
        .route("/client-metrics-summary", get(get_client_metrics_summary))
        .route("/relay-events-summary", get(get_relay_events_summary))
        .route("/client-metrics", post(handle_client_metrics))
        .with_state(service);
        
    let tcp_listener = TcpListener::bind(addr).await?;
    let local_addr = tcp_listener.local_addr()?;
    
    info!("Metrics server listening on http://{}/metrics", local_addr);
    info!("Client metrics endpoint: http://{}/client-metrics", local_addr);
    
    axum::serve(tcp_listener, app.into_make_service()).await?;
    Ok(())
}

async fn respond_with_metrics(State(service): State<MetricService>) -> impl IntoResponse {
    let mut sink = String::new();
    let reg = service.get_reg();
    
    match reg.lock() {
        Ok(registry) => {
            if let Err(e) = encode(&mut sink, &registry) {
                error!("Failed to encode metrics: {}", e);
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    [(axum::http::header::CONTENT_TYPE, "text/plain")],
                    format!("Failed to encode metrics: {}", e),
                );
            }
        }
        Err(e) => {
            error!("Failed to acquire registry lock: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(axum::http::header::CONTENT_TYPE, "text/plain")],
                format!("Failed to acquire registry lock: {}", e),
            );
        }
    }

    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, METRICS_CONTENT_TYPE)],
        sink,
    )
}

async fn handle_client_metrics(
    State(service): State<MetricService>,
    Json(payload): Json<ClientMetricsPayload>,
) -> impl IntoResponse {
    let account_id = payload.storage_export.user_account
        .as_ref()
        .and_then(|ua| ua.accounts.first())
        .map(|(acc, _)| acc.clone())
        .unwrap_or_else(|| "unknown".to_string());
    
    info!("Received storage export from client: {} account: {} ({})", payload.peer_id, account_id, payload.client_type);
    
    match service.get_client_metrics().lock() {
        Ok(mut client_metrics) => {
            client_metrics.update_from_exported_storage(payload);
            (StatusCode::OK, Json(serde_json::json!({"status": "success"})))
        }
        Err(e) => {
            error!("Failed to acquire client metrics lock: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Failed to update metrics"})),
            )
        }
    }
}

async fn get_client_metrics_summary(State(service): State<MetricService>) -> impl IntoResponse {
    match service.get_client_metrics().lock() {
        Ok(client_metrics) => {
            let list: Vec<ClientSnapshot> = client_metrics.per_client.values().cloned().collect();
            (StatusCode::OK, Json(serde_json::json!({
                "clients": list
            })))
        }
        Err(e) => {
            error!("Failed to acquire client metrics lock: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Failed to read client metrics"})),
            )
        }
    }
}

async fn get_relay_events_summary(State(service): State<MetricService>) -> impl IntoResponse {
    match service.get_relay_metrics().lock() {
        Ok(m) => {
            let summary = RelayEventsSummary {
                reservations: ReservationsSummary {
                    accepted_peers: m.reservation_accepted_peers.iter().cloned().collect(),
                    closed_peers: m.reservation_closed_peers.iter().cloned().collect(),
                },
                circuits: CircuitsSummary {
                    accepted: m.circuit_accepted_pairs.clone(),
                    closed: m.circuit_closed_pairs.clone(),
                    denied: m.circuit_denied.clone(),
                },
            };
            (StatusCode::OK, Json(serde_json::json!({ "relay": summary })))
        }
        Err(e) => {
            error!("Failed to acquire relay metrics lock: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Failed to read relay events"})),
            )
        }
    }
}

// Helper functions for relay metrics
impl MetricService {
    pub fn record_circuit_event(&self, event_type: &str, _peer_id: Option<&str>, _account_id: Option<&str>) {
        if let Ok(metrics) = self.relay_metrics.lock() {
            match event_type {
                "accepted" => { metrics.circuits_accepted_total.inc(); }
                "closed" => { metrics.circuits_closed_total.inc(); }
                _ => {}
            }
        }
    }
    
    pub fn record_reservation_event(&self, event_type: &str, _peer_id: Option<&str>, _account_id: Option<&str>) {
        if let Ok(metrics) = self.relay_metrics.lock() {
            match event_type {
                "accepted" => { metrics.reservations_accepted_total.inc(); }
                "closed" => { metrics.reservations_closed_total.inc(); }
                _ => {}
            }
        }
    }
    
    pub fn record_connection_event(&self, _event_type: &str, _peer_id: Option<&str>, _account_id: Option<&str>) {
        if let Ok(metrics) = self.relay_metrics.lock() {
            metrics.connections_total.inc();
        }
    }
}

// Extended helpers to record peer identities
impl MetricService {
    pub fn record_reservation_event_peer(&self, event_type: &str, peer_id: &str) {
        if let Ok(mut m) = self.relay_metrics.lock() {
            match event_type {
                "accepted" => {
                    m.reservations_accepted_total.inc();
                    m.reservation_accepted_peers.insert(peer_id.to_string());
                }
                "closed" => {
                    m.reservations_closed_total.inc();
                    m.reservation_closed_peers.insert(peer_id.to_string());
                }
                _ => {}
            }
        }
    }

    pub fn record_circuit_event_pair(&self, event_type: &str, src_peer_id: &str, dst_peer_id: &str, error: Option<&str>) {
        if let Ok(mut m) = self.relay_metrics.lock() {
            match event_type {
                "accepted" => {
                    m.circuits_accepted_total.inc();
                    m.circuit_accepted_pairs.push(PeerPair { src_peer_id: src_peer_id.to_string(), dst_peer_id: dst_peer_id.to_string() });
                }
                "closed" => {
                    m.circuits_closed_total.inc();
                    m.circuit_closed_pairs.push(CircuitClosedDetail { src_peer_id: src_peer_id.to_string(), dst_peer_id: dst_peer_id.to_string(), error: error.map(|e| e.to_string()) });
                }
                "denied" => {
                    m.circuits_denied_total.inc();
                    m.circuit_denied.push(CircuitDeniedDetail { src_peer_id: src_peer_id.to_string(), dst_peer_id: dst_peer_id.to_string(), status: error.unwrap_or("").to_string() });
                }
                _ => {}
            }
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
struct RelayEventsSummary {
    reservations: ReservationsSummary,
    circuits: CircuitsSummary,
}

#[derive(Debug, Clone, serde::Serialize)]
struct ReservationsSummary {
    accepted_peers: Vec<String>,
    closed_peers: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
struct CircuitsSummary {
    accepted: Vec<PeerPair>,
    closed: Vec<CircuitClosedDetail>,
    denied: Vec<CircuitDeniedDetail>,
}
