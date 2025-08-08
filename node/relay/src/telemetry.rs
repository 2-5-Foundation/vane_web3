use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicUsize, AtomicU64};
use std::time::{SystemTime, UNIX_EPOCH};

/// Simplified metrics for relay P2P server monitoring
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayServerMetrics {
    /// Network metrics
    pub network: NetworkMetrics,
    /// DHT metrics
    pub dht: DhtMetrics,
    /// Relay metrics
    pub relay: RelayMetrics,
    /// System metrics
    pub system: SystemMetrics,
    /// Timestamp
    pub timestamp: u64,
}

/// Simplified network metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkMetrics {
    /// Connected peers
    pub peers: usize,
    /// Active connections
    pub connections: usize,
    /// Connection success rate (%)
    pub success_rate: f64,
}

/// Simplified DHT metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DhtMetrics {
    /// Total records stored
    pub records: usize,
    /// Query success rate (%)
    pub query_success_rate: f64,
    /// Pending queries
    pub pending_queries: usize,
}

/// Simplified relay metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayMetrics {
    /// Active circuits
    pub active_circuits: usize,
    /// Circuit success rate (%)
    pub circuit_success_rate: f64,
    /// Active reservations
    pub reservations: usize,
}

/// Simplified system metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemMetrics {
    /// Uptime in seconds
    pub uptime: u64,
    /// Memory usage in MB
    pub memory_mb: u64,
    /// CPU usage (%)
    pub cpu_percent: f64,
}



impl RelayServerMetrics {
    pub fn new() -> Self {
        Self {
            network: NetworkMetrics::default(),
            dht: DhtMetrics::default(),
            relay: RelayMetrics::default(),
            system: SystemMetrics::default(),
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        }
    }
}

impl Default for NetworkMetrics {
    fn default() -> Self {
        Self {
            peers: 0,
            connections: 0,
            success_rate: 0.0,
        }
    }
}

impl Default for DhtMetrics {
    fn default() -> Self {
        Self {
            records: 0,
            query_success_rate: 0.0,
            pending_queries: 0,
        }
    }
}

impl Default for RelayMetrics {
    fn default() -> Self {
        Self {
            active_circuits: 0,
            circuit_success_rate: 0.0,
            reservations: 0,
        }
    }
}

impl Default for SystemMetrics {
    fn default() -> Self {
        Self {
            uptime: 0,
            memory_mb: 0,
            cpu_percent: 0.0,
        }
    }
}

/// Metrics counters for tracking various statistics
#[derive(Debug)]
pub struct MetricsCounters {
    // Network metrics
    pub active_connections: AtomicUsize,
    pub total_connections: AtomicUsize,
    pub failed_connections: AtomicUsize,
    pub successful_connections: AtomicUsize,
    
    // DHT metrics
    pub successful_queries: AtomicUsize,
    pub failed_queries: AtomicUsize,
    pub pending_queries: AtomicUsize,
    pub total_records: AtomicUsize,
    
    // Relay metrics
    pub active_circuits: AtomicUsize,
    pub total_circuits_created: AtomicUsize,
    pub total_circuits_closed: AtomicUsize,
    pub active_reservations: AtomicUsize,
    pub total_reservations_created: AtomicUsize,
    pub total_reservations_expired: AtomicUsize,
    pub circuit_errors: AtomicUsize,
    pub reservation_denials: AtomicUsize,
    
    // Request/Response metrics
    pub total_requests_received: AtomicUsize,
    pub successful_responses: AtomicUsize,
    pub failed_responses: AtomicUsize,
}

impl Default for MetricsCounters {
    fn default() -> Self {
        Self {
            active_connections: AtomicUsize::new(0),
            total_connections: AtomicUsize::new(0),
            failed_connections: AtomicUsize::new(0),
            successful_connections: AtomicUsize::new(0),
            successful_queries: AtomicUsize::new(0),
            failed_queries: AtomicUsize::new(0),
            pending_queries: AtomicUsize::new(0),
            total_records: AtomicUsize::new(0),
            active_circuits: AtomicUsize::new(0),
            total_circuits_created: AtomicUsize::new(0),
            total_circuits_closed: AtomicUsize::new(0),
            active_reservations: AtomicUsize::new(0),
            total_reservations_created: AtomicUsize::new(0),
            total_reservations_expired: AtomicUsize::new(0),
            circuit_errors: AtomicUsize::new(0),
            reservation_denials: AtomicUsize::new(0),
            total_requests_received: AtomicUsize::new(0),
            successful_responses: AtomicUsize::new(0),
            failed_responses: AtomicUsize::new(0),
        }
    }
}
