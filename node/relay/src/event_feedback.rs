use libp2p::{Multiaddr, PeerId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot, Mutex};
use log::{debug, warn};

/// Unified result type for all relay events
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EventResult {
    // Circuit events
    CircuitAccepted {
        src_peer_id: String,
        dst_peer_id: String,
        established_at: u64,
    },
    CircuitClosed {
        src_peer_id: String,
        dst_peer_id: String,
        duration_ms: u64,
        error: Option<String>,
        success: bool,
    },
    CircuitDenied {
        src_peer_id: String,
        dst_peer_id: String,
        status: String,
        reason: String,
    },

    // Reservation events
    ReservationAccepted {
        peer_id: String,
        renewed: bool,
        expires_at: u64,
    },
    ReservationClosed {
        peer_id: String,
        duration_ms: u64,
        reason: String,
    },
    ReservationDenied {
        peer_id: String,
        status: String,
        reason: String,
    },
    ReservationTimedOut {
        peer_id: String,
        duration_ms: u64,
    },

    // Connection events
    ConnectionEstablished {
        peer_id: String,
        duration_ms: u64,
        endpoint: String,
    },
    ConnectionClosed {
        peer_id: String,
        duration_ms: u64,
        cause: String,
    },
    ConnectionDialing {
        peer_id: String,
        started_at: u64,
    },
    ConnectionError {
        peer_id: Option<String>,
        error: String,
    },

    // Listener events
    ListenerStarted {
        address: String,
    },
    ListenerClosed {
        listener_id: String,
        addresses: Vec<String>,
        reason: String,
    },
    ListenerExpired {
        address: String,
    },

    // External address events
    ExternalAddrCandidate {
        address: String,
    },
    ExternalAddrConfirmed {
        address: String,
    },
    ExternalAddrExpired {
        address: String,
    },
    ExternalAddrOfPeer {
        peer_id: String,
        address: String,
    },

    // Generic event for unhandled cases
    UnknownEvent {
        event_type: String,
        details: String,
    },
}

impl EventResult {
    pub fn is_success(&self) -> bool {
        match self {
            EventResult::CircuitClosed { success, .. } => *success,
            EventResult::CircuitDenied { .. } => false,
            EventResult::ReservationDenied { .. } => false,
            EventResult::ReservationTimedOut { .. } => false,
            EventResult::ConnectionError { .. } => false,
            _ => true, // Most events represent successful operations
        }
    }

    pub fn get_peer_id(&self) -> Option<String> {
        match self {
            EventResult::CircuitAccepted { src_peer_id, .. } => Some(src_peer_id.clone()),
            EventResult::CircuitClosed { src_peer_id, .. } => Some(src_peer_id.clone()),
            EventResult::CircuitDenied { src_peer_id, .. } => Some(src_peer_id.clone()),
            EventResult::ReservationAccepted { peer_id, .. } => Some(peer_id.clone()),
            EventResult::ReservationClosed { peer_id, .. } => Some(peer_id.clone()),
            EventResult::ReservationDenied { peer_id, .. } => Some(peer_id.clone()),
            EventResult::ReservationTimedOut { peer_id, .. } => Some(peer_id.clone()),
            EventResult::ConnectionEstablished { peer_id, .. } => Some(peer_id.clone()),
            EventResult::ConnectionClosed { peer_id, .. } => Some(peer_id.clone()),
            EventResult::ConnectionDialing { peer_id, .. } => Some(peer_id.clone()),
            EventResult::ConnectionError { peer_id, .. } => peer_id.clone(),
            EventResult::ExternalAddrOfPeer { peer_id, .. } => Some(peer_id.clone()),
            _ => None,
        }
    }
}

/// Event subscription identifier
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub enum EventSubscription {
    // Circuit subscriptions (by peer pair)
    Circuit { src: PeerId, dst: PeerId },
    
    // Reservation subscriptions (by peer)
    Reservation { peer: PeerId },
    
    // Connection subscriptions (by peer)
    Connection { peer: PeerId },
    
    // Listener subscriptions (by address)
    Listener { address_hash: u64 },
    
    // Global subscriptions (all events of a type)
    AllCircuits,
    AllReservations,
    AllConnections,
    AllListeners,
    AllExternalAddrs,
    AllEvents,
}

impl EventSubscription {
    pub fn circuit_key(src: PeerId, dst: PeerId) -> Self {
        EventSubscription::Circuit { src, dst }
    }

    pub fn reservation_key(peer: PeerId) -> Self {
        EventSubscription::Reservation { peer }
    }

    pub fn connection_key(peer: PeerId) -> Self {
        EventSubscription::Connection { peer }
    }
}

/// Tracking info for active events
#[derive(Debug)]
struct EventTrackingInfo {
    started_at: Instant,
    metadata: HashMap<String, String>,
    feedback_txs: Vec<oneshot::Sender<EventResult>>,
}

/// Central event feedback manager
pub struct EventFeedbackManager {
    // Active event tracking
    active_circuits: Arc<Mutex<HashMap<(PeerId, PeerId), EventTrackingInfo>>>,
    active_reservations: Arc<Mutex<HashMap<PeerId, EventTrackingInfo>>>,
    active_connections: Arc<Mutex<HashMap<PeerId, EventTrackingInfo>>>,
    
    // Broadcast channels for global subscriptions
    circuit_broadcast: mpsc::UnboundedSender<EventResult>,
    reservation_broadcast: mpsc::UnboundedSender<EventResult>,
    connection_broadcast: mpsc::UnboundedSender<EventResult>,
    listener_broadcast: mpsc::UnboundedSender<EventResult>,
    external_addr_broadcast: mpsc::UnboundedSender<EventResult>,
    all_events_broadcast: mpsc::UnboundedSender<EventResult>,
}

impl EventFeedbackManager {
    pub fn new() -> Self {
        let (circuit_tx, _) = mpsc::unbounded_channel();
        let (reservation_tx, _) = mpsc::unbounded_channel();
        let (connection_tx, _) = mpsc::unbounded_channel();
        let (listener_tx, _) = mpsc::unbounded_channel();
        let (external_addr_tx, _) = mpsc::unbounded_channel();
        let (all_events_tx, _) = mpsc::unbounded_channel();

        Self {
            active_circuits: Arc::new(Mutex::new(HashMap::new())),
            active_reservations: Arc::new(Mutex::new(HashMap::new())),
            active_connections: Arc::new(Mutex::new(HashMap::new())),
            circuit_broadcast: circuit_tx,
            reservation_broadcast: reservation_tx,
            connection_broadcast: connection_tx,
            listener_broadcast: listener_tx,
            external_addr_broadcast: external_addr_tx,
            all_events_broadcast: all_events_tx,
        }
    }

    /// Subscribe to a specific event or event type
    pub async fn subscribe(&self, subscription: EventSubscription) -> mpsc::UnboundedReceiver<EventResult> {
        let (tx, rx) = mpsc::unbounded_channel();
        
        match subscription {
            EventSubscription::AllCircuits => {
                // Clone the sender and spawn a task to forward events
                let mut broadcast_rx = self.subscribe_broadcast(&self.circuit_broadcast).await;
                tokio::spawn(async move {
                    while let Some(event) = broadcast_rx.recv().await {
                        if tx.send(event).is_err() {
                            break;
                        }
                    }
                });
            }
            EventSubscription::AllReservations => {
                let mut broadcast_rx = self.subscribe_broadcast(&self.reservation_broadcast).await;
                tokio::spawn(async move {
                    while let Some(event) = broadcast_rx.recv().await {
                        if tx.send(event).is_err() {
                            break;
                        }
                    }
                });
            }
            EventSubscription::AllConnections => {
                let mut broadcast_rx = self.subscribe_broadcast(&self.connection_broadcast).await;
                tokio::spawn(async move {
                    while let Some(event) = broadcast_rx.recv().await {
                        if tx.send(event).is_err() {
                            break;
                        }
                    }
                });
            }
            EventSubscription::AllListeners => {
                let mut broadcast_rx = self.subscribe_broadcast(&self.listener_broadcast).await;
                tokio::spawn(async move {
                    while let Some(event) = broadcast_rx.recv().await {
                        if tx.send(event).is_err() {
                            break;
                        }
                    }
                });
            }
            EventSubscription::AllExternalAddrs => {
                let mut broadcast_rx = self.subscribe_broadcast(&self.external_addr_broadcast).await;
                tokio::spawn(async move {
                    while let Some(event) = broadcast_rx.recv().await {
                        if tx.send(event).is_err() {
                            break;
                        }
                    }
                });
            }
            EventSubscription::AllEvents => {
                let mut broadcast_rx = self.subscribe_broadcast(&self.all_events_broadcast).await;
                tokio::spawn(async move {
                    while let Some(event) = broadcast_rx.recv().await {
                        if tx.send(event).is_err() {
                            break;
                        }
                    }
                });
            }
            _ => {
                // For specific event subscriptions, we'll handle in the event methods
                warn!("Specific event subscription requires using subscribe_once methods");
            }
        }
        
        rx
    }

    async fn subscribe_broadcast(&self, tx: &mpsc::UnboundedSender<EventResult>) -> mpsc::UnboundedReceiver<EventResult> {
        let (new_tx, rx) = mpsc::unbounded_channel();
        // In production, maintain a list of subscribers
        rx
    }

    /// Subscribe to a specific circuit (one-time feedback)
    pub async fn subscribe_circuit_once(
        &self,
        src_peer: PeerId,
        dst_peer: PeerId,
    ) -> oneshot::Receiver<EventResult> {
        let (tx, rx) = oneshot::channel();
        let key = (src_peer, dst_peer);
        
        let mut circuits = self.active_circuits.lock().await;
        circuits.entry(key)
            .or_insert_with(|| EventTrackingInfo {
                started_at: Instant::now(),
                metadata: HashMap::new(),
                feedback_txs: Vec::new(),
            })
            .feedback_txs
            .push(tx);
        
        rx
    }

    /// Subscribe to a specific reservation (one-time feedback)
    pub async fn subscribe_reservation_once(
        &self,
        peer: PeerId,
    ) -> oneshot::Receiver<EventResult> {
        let (tx, rx) = oneshot::channel();
        
        let mut reservations = self.active_reservations.lock().await;
        reservations.entry(peer)
            .or_insert_with(|| EventTrackingInfo {
                started_at: Instant::now(),
                metadata: HashMap::new(),
                feedback_txs: Vec::new(),
            })
            .feedback_txs
            .push(tx);
        
        rx
    }

    /// Subscribe to a specific connection (one-time feedback)
    pub async fn subscribe_connection_once(
        &self,
        peer: PeerId,
    ) -> oneshot::Receiver<EventResult> {
        let (tx, rx) = oneshot::channel();
        
        let mut connections = self.active_connections.lock().await;
        connections.entry(peer)
            .or_insert_with(|| EventTrackingInfo {
                started_at: Instant::now(),
                metadata: HashMap::new(),
                feedback_txs: Vec::new(),
            })
            .feedback_txs
            .push(tx);
        
        rx
    }

    // ============ Circuit Event Handlers ============

    pub async fn notify_circuit_accepted(&self, src_peer: PeerId, dst_peer: PeerId) {
        let key = (src_peer, dst_peer);
        let now = Instant::now();
        
        let mut circuits = self.active_circuits.lock().await;
        circuits.insert(key, EventTrackingInfo {
            started_at: now,
            metadata: HashMap::new(),
            feedback_txs: Vec::new(),
        });

        let result = EventResult::CircuitAccepted {
            src_peer_id: src_peer.to_string(),
            dst_peer_id: dst_peer.to_string(),
            established_at: now.elapsed().as_millis() as u64,
        };

        self.broadcast_event(result).await;
    }

    pub async fn notify_circuit_closed(
        &self,
        src_peer: PeerId,
        dst_peer: PeerId,
        error: Option<String>,
    ) {
        let key = (src_peer, dst_peer);
        let mut circuits = self.active_circuits.lock().await;
        
        if let Some(info) = circuits.remove(&key) {
            let duration_ms = info.started_at.elapsed().as_millis() as u64;
            let success = error.is_none();
            
            let result = EventResult::CircuitClosed {
                src_peer_id: src_peer.to_string(),
                dst_peer_id: dst_peer.to_string(),
                duration_ms,
                error: error.clone(),
                success,
            };

            // Notify specific subscribers
            for tx in info.feedback_txs {
                let _ = tx.send(result.clone());
            }

            // Broadcast to all subscribers
            self.broadcast_event(result).await;
        }
    }

    pub async fn notify_circuit_denied(
        &self,
        src_peer: PeerId,
        dst_peer: PeerId,
        status: String,
    ) {
        let key = (src_peer, dst_peer);
        let mut circuits = self.active_circuits.lock().await;
        
        if let Some(info) = circuits.remove(&key) {
            let result = EventResult::CircuitDenied {
                src_peer_id: src_peer.to_string(),
                dst_peer_id: dst_peer.to_string(),
                status: status.clone(),
                reason: format!("Circuit denied: {}", status),
            };

            for tx in info.feedback_txs {
                let _ = tx.send(result.clone());
            }

            self.broadcast_event(result).await;
        }
    }

    // ============ Reservation Event Handlers ============

    pub async fn notify_reservation_accepted(&self, peer: PeerId, renewed: bool) {
        let now = Instant::now();
        let expires_at = now.elapsed().as_secs() + 3600; // 1 hour
        
        let mut reservations = self.active_reservations.lock().await;
        reservations.insert(peer, EventTrackingInfo {
            started_at: now,
            metadata: HashMap::new(),
            feedback_txs: Vec::new(),
        });

        let result = EventResult::ReservationAccepted {
            peer_id: peer.to_string(),
            renewed,
            expires_at,
        };

        self.broadcast_event(result).await;
    }

    pub async fn notify_reservation_closed(&self, peer: PeerId, reason: String) {
        let mut reservations = self.active_reservations.lock().await;
        
        if let Some(info) = reservations.remove(&peer) {
            let duration_ms = info.started_at.elapsed().as_millis() as u64;
            
            let result = EventResult::ReservationClosed {
                peer_id: peer.to_string(),
                duration_ms,
                reason: reason.clone(),
            };

            for tx in info.feedback_txs {
                let _ = tx.send(result.clone());
            }

            self.broadcast_event(result).await;
        }
    }

    pub async fn notify_reservation_denied(&self, peer: PeerId, status: String) {
        let mut reservations = self.active_reservations.lock().await;
        
        if let Some(info) = reservations.remove(&peer) {
            let result = EventResult::ReservationDenied {
                peer_id: peer.to_string(),
                status: status.clone(),
                reason: format!("Reservation denied: {}", status),
            };

            for tx in info.feedback_txs {
                let _ = tx.send(result.clone());
            }

            self.broadcast_event(result).await;
        }
    }

    pub async fn notify_reservation_timed_out(&self, peer: PeerId) {
        let mut reservations = self.active_reservations.lock().await;
        
        if let Some(info) = reservations.remove(&peer) {
            let duration_ms = info.started_at.elapsed().as_millis() as u64;
            
            let result = EventResult::ReservationTimedOut {
                peer_id: peer.to_string(),
                duration_ms,
            };

            for tx in info.feedback_txs {
                let _ = tx.send(result.clone());
            }

            self.broadcast_event(result).await;
        }
    }

    // ============ Connection Event Handlers ============

    pub async fn notify_connection_established(
        &self,
        peer: PeerId,
        duration_ms: u64,
        endpoint: String,
    ) {
        let now = Instant::now();
        
        let mut connections = self.active_connections.lock().await;
        connections.insert(peer, EventTrackingInfo {
            started_at: now,
            metadata: HashMap::new(),
            feedback_txs: Vec::new(),
        });

        let result = EventResult::ConnectionEstablished {
            peer_id: peer.to_string(),
            duration_ms,
            endpoint,
        };

        self.broadcast_event(result).await;
    }

    pub async fn notify_connection_closed(&self, peer: PeerId, cause: String) {
        let mut connections = self.active_connections.lock().await;
        
        if let Some(info) = connections.remove(&peer) {
            let duration_ms = info.started_at.elapsed().as_millis() as u64;
            
            let result = EventResult::ConnectionClosed {
                peer_id: peer.to_string(),
                duration_ms,
                cause: cause.clone(),
            };

            for tx in info.feedback_txs {
                let _ = tx.send(result.clone());
            }

            self.broadcast_event(result).await;
        }
    }

    pub async fn notify_connection_dialing(&self, peer: PeerId) {
        let result = EventResult::ConnectionDialing {
            peer_id: peer.to_string(),
            started_at: Instant::now().elapsed().as_millis() as u64,
        };

        self.broadcast_event(result).await;
    }

    pub async fn notify_connection_error(&self, peer: Option<PeerId>, error: String) {
        let result = EventResult::ConnectionError {
            peer_id: peer.map(|p| p.to_string()),
            error,
        };

        self.broadcast_event(result).await;
    }

    // ============ Listener Event Handlers ============

    pub async fn notify_listener_started(&self, address: String) {
        let result = EventResult::ListenerStarted { address };
        self.broadcast_event(result).await;
    }

    pub async fn notify_listener_closed(
        &self,
        listener_id: String,
        addresses: Vec<String>,
        reason: String,
    ) {
        let result = EventResult::ListenerClosed {
            listener_id,
            addresses,
            reason,
        };
        self.broadcast_event(result).await;
    }

    pub async fn notify_listener_expired(&self, address: String) {
        let result = EventResult::ListenerExpired { address };
        self.broadcast_event(result).await;
    }

    // ============ External Address Event Handlers ============

    pub async fn notify_external_addr_candidate(&self, address: String) {
        let result = EventResult::ExternalAddrCandidate { address };
        self.broadcast_event(result).await;
    }

    pub async fn notify_external_addr_confirmed(&self, address: String) {
        let result = EventResult::ExternalAddrConfirmed { address };
        self.broadcast_event(result).await;
    }

    pub async fn notify_external_addr_expired(&self, address: String) {
        let result = EventResult::ExternalAddrExpired { address };
        self.broadcast_event(result).await;
    }

    pub async fn notify_external_addr_of_peer(&self, peer: PeerId, address: String) {
        let result = EventResult::ExternalAddrOfPeer {
            peer_id: peer.to_string(),
            address,
        };
        self.broadcast_event(result).await;
    }

    // ============ Generic Event Handler ============

    pub async fn notify_unknown_event(&self, event_type: String, details: String) {
        let result = EventResult::UnknownEvent {
            event_type,
            details,
        };
        self.broadcast_event(result).await;
    }

    // ============ Broadcast Helper ============

    async fn broadcast_event(&self, event: EventResult) {
        // Broadcast to category-specific channels
        match &event {
            EventResult::CircuitAccepted { .. }
            | EventResult::CircuitClosed { .. }
            | EventResult::CircuitDenied { .. } => {
                let _ = self.circuit_broadcast.send(event.clone());
            }
            EventResult::ReservationAccepted { .. }
            | EventResult::ReservationClosed { .. }
            | EventResult::ReservationDenied { .. }
            | EventResult::ReservationTimedOut { .. } => {
                let _ = self.reservation_broadcast.send(event.clone());
            }
            EventResult::ConnectionEstablished { .. }
            | EventResult::ConnectionClosed { .. }
            | EventResult::ConnectionDialing { .. }
            | EventResult::ConnectionError { .. } => {
                let _ = self.connection_broadcast.send(event.clone());
            }
            EventResult::ListenerStarted { .. }
            | EventResult::ListenerClosed { .. }
            | EventResult::ListenerExpired { .. } => {
                let _ = self.listener_broadcast.send(event.clone());
            }
            EventResult::ExternalAddrCandidate { .. }
            | EventResult::ExternalAddrConfirmed { .. }
            | EventResult::ExternalAddrExpired { .. }
            | EventResult::ExternalAddrOfPeer { .. } => {
                let _ = self.external_addr_broadcast.send(event.clone());
            }
            _ => {}
        }

        // Always broadcast to all-events channel
        let _ = self.all_events_broadcast.send(event);
    }

    // ============ Statistics & Monitoring ============

    pub async fn get_active_circuits_count(&self) -> usize {
        self.active_circuits.lock().await.len()
    }

    pub async fn get_active_reservations_count(&self) -> usize {
        self.active_reservations.lock().await.len()
    }

    pub async fn get_active_connections_count(&self) -> usize {
        self.active_connections.lock().await.len()
    }

    pub async fn get_active_circuit_details(&self) -> Vec<(String, String, u64)> {
        let circuits = self.active_circuits.lock().await;
        circuits
            .iter()
            .map(|((src, dst), info)| {
                (
                    src.to_string(),
                    dst.to_string(),
                    info.started_at.elapsed().as_secs(),
                )
            })
            .collect()
    }
}

impl Default for EventFeedbackManager {
    fn default() -> Self {
        Self::new()
    }
}