use crate::p2p::RelayServerBehaviour;
use anyhow::anyhow;
use libp2p::swarm::NetworkInfo;
use libp2p::swarm::Swarm;
use libp2p_kad::store::MemoryStore;
use log::{error, info};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, Mutex};
use tokio::time::{interval, Duration};

/// Simplified metrics for relay P2P server monitoring
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayServerMetrics {
    /// Network metrics
    pub network: NetworkMetrics,
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
    pub failed_responses: AtomicUsize,
}

impl Default for MetricsCounters {
    fn default() -> Self {
        Self {
            active_connections: AtomicUsize::new(0),
            total_connections: AtomicUsize::new(0),
            failed_connections: AtomicUsize::new(0),
            successful_connections: AtomicUsize::new(0),
            active_circuits: AtomicUsize::new(0),
            total_circuits_created: AtomicUsize::new(0),
            total_circuits_closed: AtomicUsize::new(0),
            active_reservations: AtomicUsize::new(0),
            total_reservations_created: AtomicUsize::new(0),
            total_reservations_expired: AtomicUsize::new(0),
            circuit_errors: AtomicUsize::new(0),
            reservation_denials: AtomicUsize::new(0),
            total_requests_received: AtomicUsize::new(0),
            failed_responses: AtomicUsize::new(0),
        }
    }
}

/// Telemetry worker that handles metrics collection and reporting
pub struct TelemetryWorker {
    /// Metrics counters shared with P2P worker
    metrics_counters: Arc<MetricsCounters>,
    /// Start time for uptime calculation
    start_time: Instant,
    /// Optional swarm reference for network info (can be None to avoid deadlocks)
    swarm: Option<Arc<Mutex<Swarm<RelayServerBehaviour<MemoryStore>>>>>,
}

impl TelemetryWorker {
    pub fn new(metrics_counters: Arc<MetricsCounters>) -> Self {
        Self {
            metrics_counters,
            start_time: Instant::now(),
            swarm: None,
        }
    }

    /// Set the swarm reference for network info collection
    pub fn set_swarm(&mut self, swarm: Arc<Mutex<Swarm<RelayServerBehaviour<MemoryStore>>>>) {
        self.swarm = Some(swarm);
    }

    /// Collect current metrics from all counters
    pub async fn collect_metrics(&self) -> RelayServerMetrics {
        let network_info = self.get_network_info().await.ok();

        let mut metrics = RelayServerMetrics::new();

        // Collect network metrics
        let counters = &self.metrics_counters;
        let active_connections = counters.active_connections.load(Ordering::Relaxed);
        let successful_connections = counters.successful_connections.load(Ordering::Relaxed);
        let failed_connections = counters.failed_connections.load(Ordering::Relaxed);
        let total_connections = successful_connections + failed_connections;

        metrics.network = NetworkMetrics {
            peers: network_info.map(|info| info.num_peers()).unwrap_or(0),
            connections: active_connections,
            success_rate: if total_connections > 0 {
                (successful_connections as f64 / total_connections as f64) * 100.0
            } else {
                0.0
            },
        };

        // Collect relay metrics
        let total_circuits_created = counters.total_circuits_created.load(Ordering::Relaxed);
        let total_circuits_closed = counters.total_circuits_closed.load(Ordering::Relaxed);
        let total_circuits = total_circuits_created + total_circuits_closed;

        metrics.relay = RelayMetrics {
            active_circuits: counters.active_circuits.load(Ordering::Relaxed),
            circuit_success_rate: if total_circuits > 0 {
                (total_circuits_created as f64 / total_circuits as f64) * 100.0
            } else {
                0.0
            },
            reservations: counters.active_reservations.load(Ordering::Relaxed),
        };

        // Collect system metrics
        metrics.system = SystemMetrics {
            uptime: self.start_time.elapsed().as_secs(),
            memory_mb: 0,     // TODO: Implement memory tracking
            cpu_percent: 0.0, // TODO: Implement CPU tracking
        };

        metrics
    }

    /// Get network info with timeout to avoid deadlocks
    async fn get_network_info(&self) -> Result<NetworkInfo, anyhow::Error> {
        if let Some(swarm) = &self.swarm {
            // Use timeout to avoid deadlocks
            let swarm_guard = match tokio::time::timeout(
                Duration::from_millis(100), // 100ms timeout
                swarm.lock(),
            )
            .await
            {
                Ok(guard) => guard,
                Err(_) => {
                    return Err(anyhow!("Failed to acquire swarm lock within timeout"));
                }
            };
            let network_info = swarm_guard.network_info();
            Ok(network_info)
        } else {
            Err(anyhow!("No swarm reference available"))
        }
    }

    /// Start continuous metrics collection and send to channel
    pub async fn start_metrics_collection(
        self: Arc<Self>,
        metrics_tx: mpsc::Sender<RelayServerMetrics>,
    ) {
        let mut interval = interval(Duration::from_secs(30)); // 30 second interval

        info!("Starting telemetry metrics collection...");

        loop {
            interval.tick().await;

            let metrics = self.collect_metrics().await;

            if let Err(e) = metrics_tx.send(metrics).await {
                error!(target: "telemetry", "Failed to send metrics: {:?}", e);
                break;
            }
        }
    }

    /// Get a reference to metrics counters for external updates
    pub fn metrics_counters(&self) -> &Arc<MetricsCounters> {
        &self.metrics_counters
    }
}
