use libp2p::futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, Stream, StreamExt};
use libp2p::relay::Behaviour as RelayBehaviour;
use libp2p::request_response::json::Behaviour as JsonBehaviour;
use libp2p::request_response::{Behaviour, Event, InboundRequestId, Message, OutboundRequestId};
use libp2p::request_response::{Codec, ProtocolSupport, ResponseChannel};
use libp2p::swarm::{derive_prelude, NetworkBehaviour};
use libp2p::swarm::{NetworkInfo, SwarmEvent};
use libp2p::{Multiaddr, PeerId, StreamProtocol, Swarm, SwarmBuilder};
use libp2p_kad::store::{MemoryStore, MemoryStoreConfig};
use libp2p_kad::{
    Behaviour as DhtBehaviour, GetRecordError, GetRecordOk, GetRecordResult, QueryId, QueryResult, K_VALUE
};

use libp2p::relay::Event as RelayServerEvent;
use libp2p::request_response::Event as JsonEvent;
use libp2p_kad::Event as DhtEvent;

use anyhow::anyhow;
use log::{error, info, trace};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, AtomicU64, Ordering};
use std::time::Instant;
use tokio::sync::{Mutex, mpsc};
use tokio::time::{Duration, interval};

use primitives::data_structure::{WasmDhtRequest, WasmDhtResponse};
use crate::telemetry::{RelayServerMetrics, NetworkMetrics, DhtMetrics, RelayMetrics, SystemMetrics, MetricsCounters};

#[derive(NetworkBehaviour)]
#[behaviour(prelude = "libp2p::swarm::derive_prelude")]
pub struct RelayServerBehaviour<TStore> {
    pub relay_client: libp2p::relay::Behaviour,
    pub dht: DhtBehaviour<TStore>,
    pub dht_app_json: JsonBehaviour<WasmDhtRequest, WasmDhtResponse>,
}

impl<TStore> RelayServerBehaviour<TStore> {
    pub fn new(
        relay_behaviour: libp2p::relay::Behaviour,
        dht: DhtBehaviour<TStore>,
        dht_app_json: JsonBehaviour<WasmDhtRequest, WasmDhtResponse>,
    ) -> Self {
        Self {
            relay_client: relay_behaviour,
            dht,
            dht_app_json,
        }
    }
}

pub struct RelayP2pWorker {
    pub swarm: Arc<Mutex<Swarm<RelayServerBehaviour<MemoryStore>>>>,
    pub multi_addr: Multiaddr,
    pub response_channel: HashMap<QueryId, (ResponseChannel<WasmDhtResponse>, String)>,
    /// Metrics counters for tracking statistics
    metrics_counters: Arc<MetricsCounters>,
    /// Start time for uptime calculation
    start_time: Instant,
}

impl RelayP2pWorker {
    pub fn new(dns: String, port: u16) -> Result<Self, anyhow::Error> {
        let self_keypair = libp2p::identity::Keypair::generate_ed25519();
        let peer_id = PeerId::from(self_keypair.public());

        let url = format!("/dns/{}/tcp/{}/p2p/{}", dns, port, peer_id);
        let multi_addr = url
            .parse()
            .map_err(|err| anyhow!("failed to parse multi addr, caused by: {err}"))?;

        let request_response_config = libp2p::request_response::Config::default()
            .with_request_timeout(core::time::Duration::from_secs(60)); // 1 minute waiting time for a response

        let json_behaviour = JsonBehaviour::new(
            [(
                StreamProtocol::new("/dht_request_response_protocol"),
                ProtocolSupport::Full,
            )],
            request_response_config,
        );

        let relay_behaviour = RelayServerBehaviour::<MemoryStore>::new(
            libp2p::relay::Behaviour::new(peer_id, libp2p::relay::Config::default()),
            DhtBehaviour::<MemoryStore>::new(
                peer_id,
                MemoryStore::with_config(
                    peer_id,
                    MemoryStoreConfig {
                        max_records: 10_000,
                        max_value_bytes: 50,
                        max_provided_keys: 10_000,
                        max_providers_per_key: K_VALUE.get(),
                    },
                ),
            ),
            json_behaviour,
        );

        let transport_tcp = libp2p::tcp::Config::new().nodelay(true).port_reuse(true);

        let swarm = SwarmBuilder::with_existing_identity(self_keypair)
            .with_tokio()
            .with_tcp(
                transport_tcp,
                libp2p::tls::Config::new,
                libp2p::yamux::Config::default,
            )?
            .with_behaviour(|_| relay_behaviour)?
            .with_swarm_config(|cfg| {
                cfg.with_idle_connection_timeout(tokio::time::Duration::from_secs(300))
            })
            .build();

        Ok(Self {
            swarm: Arc::new(Mutex::new(swarm)),
            multi_addr,
            response_channel: HashMap::new(),
            metrics_counters: Arc::new(MetricsCounters::default()),
            start_time: Instant::now(),
        })
    }

    pub async fn start_swarm(&mut self) -> Result<(), anyhow::Error> {
        let multi_addr = &self.multi_addr;
        let _listening_id = self.swarm.lock().await.listen_on(multi_addr.clone())?;
        trace!(target:"p2p","relay server listening to: {:?}",multi_addr);

        loop {
            // Get the next event from the swarm
            let swarm_event = {
                let mut swarm = self.swarm.lock().await;
                swarm.select_next_some().await
            };

            match swarm_event {
                SwarmEvent::Behaviour(relay_server_behaviour) => match relay_server_behaviour {
                    RelayServerBehaviourEvent::Dht(dht_event) => {
                        self.handle_dht_event(dht_event).await?;
                    }
                    RelayServerBehaviourEvent::RelayClient(relay_event) => {
                        self.handle_relay_event(relay_event).await;
                    }
                    RelayServerBehaviourEvent::DhtAppJson(dht_app_json_event) => {
                        self.handle_dht_app_json_event(dht_app_json_event).await;
                    }
                },
                SwarmEvent::ConnectionEstablished {
                    peer_id,
                    established_in,
                    ..
                } => {
                    info!(target:"p2p","connection established {:?}: duration {:?}",peer_id,established_in);
                    self.metrics_counters.active_connections.fetch_add(1, Ordering::Relaxed);
                    self.metrics_counters.successful_connections.fetch_add(1, Ordering::Relaxed);
                    self.metrics_counters.total_connections.fetch_add(1, Ordering::Relaxed);
                }
                SwarmEvent::ConnectionClosed { peer_id, cause, .. } => {
                    info!(target:"p2p","connection closed: {:?}, reason {:?}",peer_id,cause);
                    self.metrics_counters.active_connections.fetch_sub(1, Ordering::Relaxed);
                }
                SwarmEvent::Dialing { peer_id, .. } => {
                    info!(target:"p2p","dialing: {:?}",peer_id);
                }
                _ => {
                    info!(target:"p2p","swarm event: {:?}",swarm_event);
                }
            }
        }
    }

    async fn get_peer_from_dht(&self, address: &String) -> QueryId {
        let mut swarm = self.swarm.lock().await;
        let query_id = swarm.behaviour_mut().dht.get_record(address.as_bytes().to_vec().into());
        query_id
    }

    async fn handle_dht_event(&mut self, event: DhtEvent) -> Result<(), anyhow::Error> {
        match event {
            DhtEvent::InboundRequest { request } => {
                info!(target:"p2p","dht inbound request: {:?}",request);
            }
            DhtEvent::OutboundQueryProgressed {
                result, stats, id, ..
            } => match result {
                QueryResult::GetRecord(GetRecordResult::Ok(GetRecordOk::FoundRecord(record))) => {
                    self.metrics_counters.successful_queries.fetch_add(1, Ordering::Relaxed);
                    self.metrics_counters.pending_queries.fetch_sub(1, Ordering::Relaxed);
                    if let Some((channel, key)) = self.response_channel.remove(&id) {
                        let peer_id = if key.as_bytes() == record.record.key.to_vec().as_slice() {
                            let peer_id = PeerId::from_bytes(record.record.value.as_slice()).expect("failed to parse peer id");
                            Some(peer_id)
                        } else {
                            error!(target:"p2p","dht key mismatch: {:?}, {:?}",key,record.record.key);
                            None
                        };
                        let response = WasmDhtResponse {
                            peer_id
                        };
                        self.swarm
                                .lock()
                                .await
                                .behaviour_mut()
                                .dht_app_json
                                .send_response(channel, response).map_err(|e| anyhow!("failed to send response: {:?}",e))?;
                        self.metrics_counters.successful_responses.fetch_add(1, Ordering::Relaxed);
                    } else {
                        error!("response channel not found for query id: {:?}", id);
                    }
                }
                QueryResult::GetRecord(GetRecordResult::Err(GetRecordError::NotFound { .. })) => {
                    self.metrics_counters.failed_queries.fetch_add(1, Ordering::Relaxed);
                    self.metrics_counters.pending_queries.fetch_sub(1, Ordering::Relaxed);
                }
                _ => {}
            },
            DhtEvent::RoutingUpdated {
                peer,
                addresses,
                is_new_peer,
                ..
            } => {
                info!(target:"p2p","dht routing updated: peer {:?}, addresses {:?}, is_new_peer {:?}",peer,addresses,is_new_peer);
            }
            DhtEvent::UnroutablePeer { peer } => {
                info!(target:"p2p","dht unroutable peer: {:?}",peer);
            }
            DhtEvent::ModeChanged { new_mode } => {
                info!(target:"p2p","dht mode changed: {:?}",new_mode);
            }
            DhtEvent::RoutablePeer { peer, address } => {
                info!(target:"p2p","dht routable peer: {:?}",address);
            }
            DhtEvent::PendingRoutablePeer { peer, address } => {
                info!(target:"p2p","dht pending routable peer: {:?}",address);
            }
        }
        Ok(())
    }

    async fn handle_relay_event(&self, event: RelayServerEvent) {
        match event {
            RelayServerEvent::CircuitClosed {
                src_peer_id,
                dst_peer_id,
                error,
            } => {
                info!(target:"p2p","relay message, circuit closed src_peer_id: {:?}, dst_peer_id: {:?}, error: {:?}",src_peer_id,dst_peer_id,error);
            }
            RelayServerEvent::CircuitReqAccepted {
                src_peer_id,
                dst_peer_id,
            } => {
                info!(target:"p2p","relay message, circuit req accepted src_peer_id: {:?}, dst_peer_id: {:?}",src_peer_id,dst_peer_id);
                self.metrics_counters.active_circuits.fetch_add(1, Ordering::Relaxed);
                self.metrics_counters.total_circuits_created.fetch_add(1, Ordering::Relaxed);
            }
            RelayServerEvent::CircuitReqDenied {
                src_peer_id,
                dst_peer_id,
                status,
            } => {
                info!(target:"p2p","relay message, circuit req denied status: {:?}, src_peer_id: {:?}, dst_peer_id: {:?}",status,src_peer_id,dst_peer_id);
                self.metrics_counters.circuit_errors.fetch_add(1, Ordering::Relaxed);
            }
            RelayServerEvent::ReservationReqAccepted {
                src_peer_id,
                renewed,
            } => {
                info!(target:"p2p","relay message, reservation req accepted src_peer_id: {:?}, renewed: {:?}",src_peer_id,renewed);
                self.metrics_counters.active_reservations.fetch_add(1, Ordering::Relaxed);
                self.metrics_counters.total_reservations_created.fetch_add(1, Ordering::Relaxed);
            }
            RelayServerEvent::ReservationReqDenied {
                src_peer_id,
                status,
            } => {
                info!(target:"p2p","relay message, reservation req denied status: {:?}, src_peer_id: {:?}",status,src_peer_id);
                self.metrics_counters.reservation_denials.fetch_add(1, Ordering::Relaxed);
            }
            RelayServerEvent::ReservationTimedOut { src_peer_id } => {
                info!(target:"p2p","relay message, reservation timed out: {:?}",src_peer_id);
                self.metrics_counters.active_reservations.fetch_sub(1, Ordering::Relaxed);
                self.metrics_counters.total_reservations_expired.fetch_add(1, Ordering::Relaxed);
            }
            RelayServerEvent::ReservationClosed { src_peer_id } => {
                info!(target:"p2p","relay message, reservation closed: {:?}",src_peer_id);
                self.metrics_counters.active_reservations.fetch_sub(1, Ordering::Relaxed);
            }
            _ => {
                trace!(target:"p2p","relay message: {:?}",event);
            }
        }
    }

    async fn handle_dht_app_json_event(&mut self, event: JsonEvent<WasmDhtRequest, WasmDhtResponse>) {
        match event {
            JsonEvent::Message { peer, message, .. } => match message {
                Message::<WasmDhtRequest, WasmDhtResponse>::Request {
                    request_id,
                    request,
                    channel,
                } => {
                    info!(target:"p2p","dht app fetching peerId for address: {:?}",&request.key);
                    self.metrics_counters.total_requests_received.fetch_add(1, Ordering::Relaxed);
                    self.metrics_counters.pending_queries.fetch_add(1, Ordering::Relaxed);
                    let query_id = self.get_peer_from_dht(&request.key).await;
                    self.response_channel
                        .insert(query_id, (channel, request.key));
                }
                _ => {}
            },
            JsonEvent::ResponseSent { peer, .. } => {
                trace!(target:"p2p","dht app json response sent to {:?}",peer);
            }
            JsonEvent::InboundFailure { peer, error, .. } => {
                info!(target:"p2p","dht app json inbound failure: {:?}, {:?}",peer,error);
            }
            JsonEvent::OutboundFailure { peer, error, .. } => {
                info!(target:"p2p","dht app json outbound failure: {:?}, {:?}",peer,error);
            }
        }
    }

    pub async fn get_network_info(&self) -> Result<NetworkInfo, anyhow::Error> {
        let swarm = self.swarm.lock().await;
        let network_info = swarm.network_info();
        Ok(network_info)
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

        // Collect DHT metrics
        let successful_queries = counters.successful_queries.load(Ordering::Relaxed);
        let failed_queries = counters.failed_queries.load(Ordering::Relaxed);
        let total_queries = successful_queries + failed_queries;
        
        metrics.dht = DhtMetrics {
            records: counters.total_records.load(Ordering::Relaxed),
            query_success_rate: if total_queries > 0 {
                (successful_queries as f64 / total_queries as f64) * 100.0
            } else {
                0.0
            },
            pending_queries: counters.pending_queries.load(Ordering::Relaxed),
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
            memory_mb: 0, // TODO: Implement memory tracking
            cpu_percent: 0.0, // TODO: Implement CPU tracking
        };

        metrics
    }

    /// Start continuous metrics collection and send to channel
    pub async fn start_metrics_collection(
        self: Arc<Self>,
        metrics_tx: mpsc::Sender<RelayServerMetrics>,
    ) {
        let mut interval = interval(Duration::from_secs(30)); // 30 second interval
        
        loop {
            interval.tick().await;
            
            let metrics = self.collect_metrics().await;
            
            if let Err(e) = metrics_tx.send(metrics).await {
                error!(target: "p2p", "Failed to send metrics: {:?}", e);
                break;
            }
        }
    }

    /// Get a reference to metrics counters for external updates
    pub fn metrics_counters(&self) -> &Arc<MetricsCounters> {
        &self.metrics_counters
    }
}
