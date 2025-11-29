use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use anyhow::{anyhow, Result};
use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use jsonrpsee::{
    core::{async_trait, RpcResult, SubscriptionResult},
    PendingSubscriptionSink, SubscriptionMessage,
    proc_macros::rpc,
    server::ServerBuilder,
};
use log::{error, info, warn};
use primitives::data_structure::{ChainSupported, TxStateMachine};
use prometheus_client::{
    metrics::{counter::Counter, gauge::Gauge},
    registry::Registry,
};
use serde::{Deserialize, Serialize};
use tokio::{
    net::TcpListener,
    sync::{mpsc, Mutex as TokioMutex},
};

pub type Data = Vec<u8>;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TargetPeer {
    account_id: String,
    network: ChainSupported,
    times_requested: u16,
}
pub type TargetPeers = HashMap<String, TargetPeer>;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum BackendEvent {
    SenderRequestReceived {
        address: String,
        data: Data,
    },
    SenderRequestHandled {
        address: String,
        data: Data,
    },
    ReceiverResponseReceived {
        address: String,
        data: Data,
    },
    ReceiverResponseHandled {
        address: String,
        data: Data,
    },
    PeerDisconnected {
        account_id: String,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum SystemNotification {
    PeerAdded {
        address: String,
        account_id: String,
        network: ChainSupported,
    },
    PeerRemoved {
        address: String,
    },
    RequestQueued {
        address: String,
    },
    RequestProcessed {
        address: String,
    },
}

pub struct VaneSwarmServer {
    peers: HashMap<String, TargetPeers>,
    requests: HashMap<String, Data>,
    metrics: Arc<MetricService>,
    event_sender: mpsc::Sender<BackendEvent>,
    system_notification_sender: mpsc::Sender<SystemNotification>,
}

impl VaneSwarmServer {
    pub fn new(metrics: Arc<MetricService>, event_sender: mpsc::Sender<BackendEvent>, system_notification_sender: mpsc::Sender<SystemNotification>) -> Self {
        Self {
            peers: HashMap::new(),
            requests: HashMap::new(),
            metrics,
            event_sender,
            system_notification_sender,
        }
    }

    pub fn handle_sender_request(&mut self, address: String, data: Data) -> Result<()> {
        info!("Received sender request from address: {}", address);
        
        let received_event = BackendEvent::SenderRequestReceived {
            address: address.clone(),
            data: data.clone(),
        };
        let _ = self.event_sender.try_send(received_event);

        let tx_state: TxStateMachine = serde_json::from_slice(&data)
            .map_err(|e| {
                error!("Failed to decode TxStateMachine from sender {}: {}", address, e);
                anyhow!("Failed to decode TxStateMachine: {}", e)
            })?;

        let receiver_address = tx_state.receiver_address.clone();
        let receiver_network = tx_state.receiver_address_network;

        if !self.peers.contains_key(&address) {
            
            let mut target_peers = HashMap::new();
            let peer = TargetPeer {
                account_id: receiver_address.clone(),
                network: receiver_network,
                times_requested: 1,
            };
            target_peers.insert(receiver_address.clone(), peer);
            self.peers.insert(address.clone(), target_peers);
            self.metrics.record_peer_added();
            self.update_metrics();

            let notification = SystemNotification::PeerAdded {
                address: address.clone(),
                account_id: receiver_address.clone(),
                network: receiver_network,
            };

            info!("succesfully added new peer: {:?}", notification);

            let _ = self.system_notification_sender.try_send(notification);
        } else {
            if let Some(target_peers) = self.peers.get_mut(&address) {
                if let Some(peer) = target_peers.get_mut(&receiver_address) {
                    peer.times_requested += 1;
                    info!("Updated peer request count - sender: {}, receiver: {}, times_requested: {}", address, receiver_address, peer.times_requested);
                } else {
                    let new_peer = TargetPeer {
                        account_id: receiver_address.clone(),
                        network: receiver_network,
                        times_requested: 1,
                    };
                    
                    target_peers.insert(receiver_address.clone(), new_peer);
                    let notification = SystemNotification::PeerAdded {
                        address: address.clone(),
                        account_id: receiver_address.clone(),
                        network: receiver_network,
                    };
        
                    info!("succesfully added new peer: {:?}", notification);
                }
            }
        }

        let notification = SystemNotification::RequestQueued {
            address: receiver_address.clone(),
        };
        let _ = self.system_notification_sender.try_send(notification.clone());
        info!("succesfully queued request for receiver: {:?}", notification);

        self.requests.insert(receiver_address.clone(), data.clone());
        self.metrics.record_sender_request(&address);
        self.update_metrics();

        let event = BackendEvent::SenderRequestHandled {
            address: receiver_address.clone(),
            data,
        };
        let _ = self.event_sender.try_send(event.clone());
        info!("succesfully handled sender request: {:?}", event);
        Ok(())
    }

    pub fn handle_receiver_response(&mut self, address: String, data: Data) -> Result<()> {
        info!("Received receiver response from address: {}", address);
        
        let received_event = BackendEvent::ReceiverResponseReceived {
            address: address.clone(),
            data: data.clone(),
        };
        let _ = self.event_sender.try_send(received_event);

        self.requests.insert(address.clone(), data.clone());
        self.metrics.record_receiver_response(&address);
        self.update_metrics();

        let notification = SystemNotification::RequestProcessed {
            address: address.clone(),
        };
        let _ = self.system_notification_sender.try_send(notification);

        let event = BackendEvent::ReceiverResponseHandled {
            address: address.clone(),
            data,
        };
        let _ = self.event_sender.try_send(event);
        info!("Receiver response handled successfully - address: {}", address);
        Ok(())
    }

    pub fn disconnect_peer(&mut self, account_id: &str) -> Result<()> {
        info!("Disconnecting peer - account_id: {}", account_id);
        
        let mut sender_key_to_remove: Option<String> = None;
        
        for (sender_key, target_peers) in self.peers.iter_mut() {
            if let Some(peer) = target_peers.get(account_id) {
                if peer.times_requested == 1 {
                    target_peers.remove(account_id);
                    if target_peers.is_empty() {
                        sender_key_to_remove = Some(sender_key.clone());
                    }
                    self.metrics.record_peer_removed();
                    
                    let notification = SystemNotification::PeerRemoved {
                        address: sender_key.clone(),
                    };
                    let _ = self.system_notification_sender.try_send(notification);

                    let event = BackendEvent::PeerDisconnected {
                        account_id: account_id.to_string(),
                    };
                    let _ = self.event_sender.try_send(event.clone());
                    info!("succesfully disconnected peer: {:?}", event);
                    
                    if let Some(key) = sender_key_to_remove {
                        self.peers.remove(&key);
                    }
                    self.update_metrics();
                    return Ok(());
                } else {
                    info!("No-op: times_requested is {} (more than 1) for peer - sender: {}, account_id: {}", peer.times_requested, sender_key, account_id);
                    return Ok(());
                }
            }
        }

        warn!("Peer with account_id {} not found", account_id);
        Err(anyhow!("Peer with account_id {} not found", account_id))
    }

    fn update_metrics(&self) {
        let peer_count = self.peers.len() as i64;
        let request_count = self.requests.len() as i64;
        self.metrics.update_active_peers(peer_count);
        self.metrics.update_pending_requests(request_count);
    }
}

#[rpc(server)]
pub trait BackendRpc {
    /// Handle sender request
    /// params:
    ///
    /// - `address`: The sender address
    /// - `data`: Request data (TxStateMachine encoded as JSON bytes)
    #[method(name = "handleSenderRequest")]
    async fn handle_sender_request(&self, address: String, data: Vec<u8>) -> RpcResult<()>;

    /// Handle receiver response
    /// params:
    ///
    /// - `address`: The receiver address
    /// - `data`: Response data
    #[method(name = "handleReceiverResponse")]
    async fn handle_receiver_response(&self, address: String, data: Vec<u8>) -> RpcResult<()>;

    /// Disconnect a peer from target peers
    /// params:
    ///
    /// - `account_id`: Account ID of the peer to disconnect
    #[method(name = "disconnectPeer")]
    async fn disconnect_peer(&self, account_id: String) -> RpcResult<()>;

    /// Subscribe to events filtered by address
    /// params:
    ///
    /// - `address`: The address to filter events for
    #[subscription(name = "subscribeToEvents", item = BackendEvent)]
    async fn subscribe_to_events(&self, address: String) -> SubscriptionResult;
}

#[derive(Clone)]
pub struct BackendRpcHandler {
    swarm_server: Arc<Mutex<VaneSwarmServer>>,
    event_receiver: Arc<TokioMutex<mpsc::Receiver<BackendEvent>>>,
}

impl BackendRpcHandler {
    pub fn new(
        swarm_server: Arc<Mutex<VaneSwarmServer>>,
        event_receiver: mpsc::Receiver<BackendEvent>,
    ) -> Self {
        Self {
            swarm_server,
            event_receiver: Arc::new(TokioMutex::new(event_receiver)),
        }
    }
}

#[async_trait]
impl BackendRpcServer for BackendRpcHandler {
    async fn handle_sender_request(&self, address: String, data: Vec<u8>) -> RpcResult<()> {
        info!("RPC: handle_sender_request called for address: {}", address);
        let mut server = self.swarm_server.lock().unwrap();
        server.handle_sender_request(address, data)
            .map_err(|e| {
                error!("RPC: Failed to handle sender request: {}", e);
                jsonrpsee::core::Error::Custom(e.to_string())
            })?;
        Ok(())
    }

    async fn handle_receiver_response(&self, address: String, data: Vec<u8>) -> RpcResult<()> {
        info!("RPC: handle_receiver_response called for address: {}", address);
        let mut server = self.swarm_server.lock().unwrap();
        server.handle_receiver_response(address, data)
            .map_err(|e| {
                error!("RPC: Failed to handle receiver response: {}", e);
                jsonrpsee::core::Error::Custom(e.to_string())
            })?;
        Ok(())
    }

    async fn disconnect_peer(&self, account_id: String) -> RpcResult<()> {
        info!("RPC: disconnect_peer called - account_id: {}", account_id);
        let mut server = self.swarm_server.lock().unwrap();
        server.disconnect_peer(&account_id)
            .map_err(|e| {
                error!("RPC: Failed to disconnect peer: {}", e);
                jsonrpsee::core::Error::Custom(e.to_string())
            })?;
        Ok(())
    }

    async fn subscribe_to_events(&self, pending: PendingSubscriptionSink, address: String,) -> SubscriptionResult {
        info!("RPC: subscribe_to_events called for address: {}", address);
        let sink = pending.accept().await
            .map_err(|e| {
                error!("RPC: Failed to accept subscription for address {}: {:?}", address, e);
                anyhow!("failed to accept subscription")
            })?;

        let mut receiver = self.event_receiver.lock().await;
        info!("Subscription active for address: {}", address);

        while let Some(event) = receiver.recv().await {
            let should_send = match &event {
                BackendEvent::SenderRequestReceived { address: event_address, data } => {
                    event_address == &address || serde_json::from_slice::<TxStateMachine>(data)
                        .ok()
                        .map(|tx| tx.sender_address == address || tx.receiver_address == address)
                        .unwrap_or(false)
                }
                BackendEvent::SenderRequestHandled { address: event_address, data } => {
                    event_address == &address || serde_json::from_slice::<TxStateMachine>(data)
                        .ok()
                        .map(|tx| tx.sender_address == address || tx.receiver_address == address)
                        .unwrap_or(false)
                }
                BackendEvent::ReceiverResponseReceived { address: event_address, data } => {
                    event_address == &address || serde_json::from_slice::<TxStateMachine>(data)
                        .ok()
                        .map(|tx| tx.sender_address == address || tx.receiver_address == address)
                        .unwrap_or(false)
                }
                BackendEvent::ReceiverResponseHandled { address: event_address, data } => {
                    event_address == &address || serde_json::from_slice::<TxStateMachine>(data)
                        .ok()
                        .map(|tx| tx.sender_address == address || tx.receiver_address == address)
                        .unwrap_or(false)
                }
                BackendEvent::PeerDisconnected { account_id } => {
                    account_id == &address
                }
            };

            if should_send {
                let subscription_msg = SubscriptionMessage::from_json(&event)
                    .map_err(|e| {
                        error!("Failed to serialize event for subscription: {}", e);
                        anyhow!("failed to serialize event: {}", e)
                    })?;

                if let Err(e) = sink.send(subscription_msg).await {
                    error!("Failed to send event to subscription for address {}: {}", address, e);
                    break;
                }
            }
        }

        info!("Subscription ended for address: {}", address);
        Ok(())
    }
}

pub struct JsonRpcServer {
    rpc_handler: BackendRpcHandler,
    url: String,
}

impl JsonRpcServer {
    pub fn new(
        swarm_server: Arc<Mutex<VaneSwarmServer>>,
        event_receiver: mpsc::Receiver<BackendEvent>,
        port: u16,
    ) -> Result<Self> {
        let rpc_handler = BackendRpcHandler::new(swarm_server, event_receiver);
        let url = format!("127.0.0.1:{}", port);
        Ok(Self { rpc_handler, url })
    }

    pub async fn start(&self) -> Result<SocketAddr> {
        let server_builder = ServerBuilder::new();
        let server = server_builder.build(&self.url).await?;
        let address = server
            .local_addr()
            .map_err(|err| anyhow!("failed to get address: {}", err))?;

        let handler = self.rpc_handler.clone().into_rpc();

        let handle = server
            .start(handler)
            .map_err(|err| anyhow!("rpc handler error: {}", err))?;

        info!("Backend JSON-RPC server listening on ws://{}", address);
    
        handle.stopped().await;
        Ok(address)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BackendEventsSummary {
    sender_requests_total: u64,
    receiver_responses_total: u64,
    receiver_not_found_total: u64,
    active_peers: usize,
    pending_requests: usize,
}

pub struct BackendMetrics {
    pub sender_requests_total: Counter,
    pub receiver_responses_total: Counter,
    pub receiver_not_found_total: Counter,
    pub peers_added_total: Counter,
    pub peers_removed_total: Counter,
    pub active_peers: Gauge,
    pub pending_requests: Gauge,
}

impl BackendMetrics {
    pub fn new() -> Self {
        Self {
            sender_requests_total: Counter::default(),
            receiver_responses_total: Counter::default(),
            receiver_not_found_total: Counter::default(),
            peers_added_total: Counter::default(),
            peers_removed_total: Counter::default(),
            active_peers: Gauge::default(),
            pending_requests: Gauge::default(),
        }
    }

    pub fn register(&self, registry: &mut Registry) {
        registry.register(
            "backend_sender_requests_total",
            "Total sender requests received",
            self.sender_requests_total.clone(),
        );
        registry.register(
            "backend_receiver_responses_total",
            "Total receiver responses received",
            self.receiver_responses_total.clone(),
        );
        registry.register(
            "backend_receiver_not_found_total",
            "Total receiver not found events",
            self.receiver_not_found_total.clone(),
        );
        registry.register(
            "backend_peers_added_total",
            "Total peers added",
            self.peers_added_total.clone(),
        );
        registry.register(
            "backend_peers_removed_total",
            "Total peers removed",
            self.peers_removed_total.clone(),
        );
        registry.register(
            "backend_active_peers",
            "Current number of active peers",
            self.active_peers.clone(),
        );
        registry.register(
            "backend_pending_requests",
            "Current number of pending requests",
            self.pending_requests.clone(),
        );
    }
}

#[derive(Clone)]
pub struct MetricService {
    backend_metrics: Arc<Mutex<BackendMetrics>>,
}

impl MetricService {
    pub fn new() -> Self {
        let mut registry = Registry::default();
        let backend_metrics = BackendMetrics::new();

        backend_metrics.register(&mut registry);

        Self {
            backend_metrics: Arc::new(Mutex::new(backend_metrics)),
        }
    }

    pub fn get_backend_metrics(&self) -> Arc<Mutex<BackendMetrics>> {
        Arc::clone(&self.backend_metrics)
    }

    pub fn record_sender_request(&self, _address: &str) {
        if let Ok(metrics) = self.backend_metrics.lock() {
            metrics.sender_requests_total.inc();
        }
    }

    pub fn record_receiver_response(&self, _address: &str) {
        if let Ok(metrics) = self.backend_metrics.lock() {
            metrics.receiver_responses_total.inc();
        }
    }

    pub fn record_peer_added(&self) {
        if let Ok(metrics) = self.backend_metrics.lock() {
            metrics.peers_added_total.inc();
            metrics.active_peers.inc();
        }
    }

    pub fn record_peer_removed(&self) {
        if let Ok(metrics) = self.backend_metrics.lock() {
            metrics.peers_removed_total.inc();
            metrics.active_peers.dec();
        }
    }

    pub fn update_active_peers(&self, count: i64) {
        if let Ok(metrics) = self.backend_metrics.lock() {
            metrics.active_peers.set(count);
        }
    }

    pub fn update_pending_requests(&self, count: i64) {
        if let Ok(metrics) = self.backend_metrics.lock() {
            metrics.pending_requests.set(count);
        }
    }
}

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
            .with_state(self.service.clone());

        let tcp_listener = TcpListener::bind(addr).await?;
        let local_addr = tcp_listener.local_addr()?;

        info!("Backend metrics server listening on http://{}", local_addr);
        info!("Metrics summary endpoint (GET): http://{}/metrics-summary", local_addr);

        axum::serve(tcp_listener, app.into_make_service()).await?;
        Ok(())
    }
}

async fn get_metrics_summary(State(service): State<MetricService>) -> impl IntoResponse {
    match service.get_backend_metrics().lock() {
        Ok(metrics) => {
            let summary = BackendEventsSummary {
                sender_requests_total: metrics.sender_requests_total.get(),
                receiver_responses_total: metrics.receiver_responses_total.get(),
                receiver_not_found_total: metrics.receiver_not_found_total.get(),
                active_peers: metrics.active_peers.get() as usize,
                pending_requests: metrics.pending_requests.get() as usize,
            };
            (
                StatusCode::OK,
                Json(serde_json::json!({ "backend": summary })),
            )
        }
        Err(e) => {
            error!("Failed to acquire backend metrics lock: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Failed to read backend metrics"})),
            )
        }
    }
}
