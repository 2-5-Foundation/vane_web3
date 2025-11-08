use hex;
use libp2p::core::transport::{upgrade, OrTransport};
use libp2p::futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, Stream, StreamExt};
use libp2p::kad::store::{MemoryStore, MemoryStoreConfig};
use libp2p::kad::{
    Behaviour as DhtBehaviour, Config as KademliaConfig, GetRecordOk, GetRecordResult,
    InboundRequest, PeerRecord, QueryResult, Record, K_VALUE,
};
use libp2p::relay::Behaviour as RelayBehaviour;
use libp2p::swarm::{derive_prelude, NetworkBehaviour};
use libp2p::swarm::{NetworkInfo, SwarmEvent};
use libp2p::{Multiaddr, PeerId, StreamProtocol, Swarm, SwarmBuilder};

use crate::metric_server::MetricService;
use crate::event_feedback::EventFeedbackManager;
use anyhow::anyhow;
use futures::future::Either;
use libp2p::kad::Event as DhtEvent;
use libp2p::relay::Event as RelayServerEvent;
use libp2p::Transport;
use log::{debug, error, info, trace};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{mpsc, Mutex};
use tokio::time::{interval, Duration};

use std::num::NonZeroU32;
#[derive(NetworkBehaviour)]
#[behaviour(prelude = "libp2p::swarm::derive_prelude")]
pub struct RelayServerBehaviour {
    pub relay_server: libp2p::relay::Behaviour,
}

impl RelayServerBehaviour {
    pub fn new(relay_behaviour: libp2p::relay::Behaviour) -> Self {
        Self {
            relay_server: relay_behaviour,
        }
    }
}

pub struct RelayP2pWorker {
    pub peer_id: PeerId,
    pub swarm: Arc<Mutex<Swarm<RelayServerBehaviour>>>,
    pub external_multi_addr: Multiaddr,
    pub listening_addr: (Multiaddr, Multiaddr),
    pub metrics: Arc<MetricService>,
    pub feedback_manager: Arc<EventFeedbackManager>,
}

impl RelayP2pWorker {
    pub async fn new(
        dns: String,
        port: u16,
        live: bool,
        private_key: Option<String>,
    ) -> Result<Self, anyhow::Error> {
        let self_keypair = if live {
            let private_key_str = private_key.unwrap();
            let private_key = private_key_str
                .strip_prefix("0x")
                .unwrap_or(&private_key_str)
                .to_string();
            let pk = hex::decode(&private_key)
                .map_err(|e| anyhow::anyhow!("failed to decode hex private key: {e}"))?;
            libp2p::identity::Keypair::ed25519_from_bytes(pk)
                .map_err(|e| anyhow::anyhow!("failed to decode private key: {e}"))?
        } else {
            libp2p::identity::Keypair::generate_ed25519()
        };
        let peer_id = PeerId::from(self_keypair.public());

        let external_multi_addr = Multiaddr::empty()
            .with(libp2p::multiaddr::Protocol::Dns4(dns.into()))
            .with(libp2p::multiaddr::Protocol::Tcp(port))
            .with(libp2p::multiaddr::Protocol::Wss("/".into()))
            .with(libp2p::multiaddr::Protocol::P2p(peer_id));

        info!(target:"p2p","relay server external multi addr: {:?}",&external_multi_addr);

        let ipv4_listening_addr = Multiaddr::from(std::net::Ipv4Addr::UNSPECIFIED) // 0.0.0.0
            .with(libp2p::multiaddr::Protocol::Tcp(port))
            .with(libp2p::multiaddr::Protocol::Ws("/".into()));

        let ipv6_listening_addr = Multiaddr::from(std::net::Ipv6Addr::UNSPECIFIED) // ::
            .with(libp2p::multiaddr::Protocol::Tcp(port))
            .with(libp2p::multiaddr::Protocol::Ws("/".into()));

        let relay_behaviour = RelayServerBehaviour::new(libp2p::relay::Behaviour::new(
            peer_id.clone(),
            libp2p::relay::Config {
                max_reservations: 10_000,
                max_reservations_per_peer: 1000,
                reservation_duration: Duration::from_secs(60 * 60),
                reservation_rate_limiters: vec![],
                max_circuits: 10_000,
                max_circuits_per_peer: 100,
                max_circuit_duration: Duration::from_secs(60 * 20),
                max_circuit_bytes: 128 * 1024,
                circuit_src_rate_limiters: vec![],
            },
        ));

        let mut relay_swarm = SwarmBuilder::with_existing_identity(self_keypair)
            .with_tokio()
            .with_websocket(
                |key: &libp2p::identity::Keypair| -> Result<_, anyhow::Error> {
                    libp2p::noise::Config::new(key).map_err(|_| anyhow!("noise init failed"))
                },
                || libp2p::yamux::Config::default(),
            )
            .await?
            .with_behaviour(|_| relay_behaviour)?
            .with_swarm_config(|cfg| {
                cfg.with_idle_connection_timeout(tokio::time::Duration::from_secs(600))
                // 10 mins
            })
            .build();

        relay_swarm.add_external_address(external_multi_addr.clone());

        let metrics = Arc::new(MetricService::new());
        let feedback_manager = Arc::new(EventFeedbackManager::new());

        Ok(Self {
            peer_id,
            swarm: Arc::new(Mutex::new(relay_swarm)),
            external_multi_addr,
            listening_addr: (ipv4_listening_addr, ipv6_listening_addr),
            metrics,
            feedback_manager,
        })
    }

    pub async fn start_swarm(&mut self) -> Result<(), anyhow::Error> {
        let listening_addr = &self.listening_addr.0;
        let _listening_id = self.swarm.lock().await.listen_on(listening_addr.clone())?;
        let listening_addr_ipv4 = &self.listening_addr.1;
        let _listening_id_ipv4 = self
            .swarm
            .lock()
            .await
            .listen_on(listening_addr_ipv4.clone())?;

        loop {
            let swarm_event = {
                let mut swarm = self.swarm.lock().await;
                swarm.select_next_some().await
            };
            match swarm_event {
                SwarmEvent::Behaviour(relay_server_behaviour) => match relay_server_behaviour {
                    RelayServerBehaviourEvent::RelayServer(relay_event) => {
                        self.handle_relay_event(relay_event).await;
                    }
                },
                SwarmEvent::ConnectionEstablished {
                    peer_id,
                    established_in,
                    endpoint,
                    ..
                } => {
                    info!(target:"p2p","connection established {:?}: duration {:?}",peer_id,established_in);
                    self.metrics.record_connection_event(
                        "established",
                        Some(&peer_id.to_string()),
                        None,
                    );
                    
                    // Send feedback
                    self.feedback_manager.notify_connection_established(
                        peer_id,
                        established_in.as_millis() as u64,
                        format!("{:?}", endpoint),
                    ).await;
                }
                SwarmEvent::ConnectionClosed { peer_id, cause, .. } => {
                    info!(target:"p2p","connection closed: {:?}, reason {:?}",peer_id,cause);
                    self.metrics.record_connection_event(
                        "closed",
                        Some(&peer_id.to_string()),
                        None,
                    );
                    
                    // Send feedback
                    self.feedback_manager.notify_connection_closed(
                        peer_id,
                        format!("{:?}", cause),
                    ).await;
                }
                SwarmEvent::Dialing { peer_id, .. } => {
                    info!(target:"p2p","dialing: {:?}",peer_id);
                    if let Some(pid) = peer_id {
                        self.metrics.record_connection_event(
                            "dialing",
                            Some(&pid.to_string()),
                            None,
                        );
                        
                        // Send feedback
                        self.feedback_manager.notify_connection_dialing(pid).await;
                    }
                }
                SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
                    info!(target:"p2p","outgoing connection error: {:?}, error {:?}",peer_id,error);
                    
                    // Send feedback
                    self.feedback_manager.notify_connection_error(
                        peer_id,
                        format!("{:?}", error),
                    ).await;
                }
                SwarmEvent::ListenerClosed {
                    listener_id,
                    addresses,
                    reason,
                    ..
                } => {
                    info!(target:"p2p","listener closed: {:?}, addresses {:?}, reason {:?}",listener_id,addresses,reason);
                    
                    // Send feedback
                    self.feedback_manager.notify_listener_closed(
                        format!("{:?}", listener_id),
                        addresses.iter().map(|a| a.to_string()).collect(),
                        format!("{:?}", reason),
                    ).await;
                }
                SwarmEvent::NewListenAddr { address, .. } => {
                    debug!(target:"p2p","new listen addr: {:?}",address);
                    
                    // Send feedback
                    self.feedback_manager.notify_listener_started(
                        address.to_string(),
                    ).await;
                }
                SwarmEvent::ExpiredListenAddr { address, .. } => {
                    debug!(target:"p2p","expired listen addr: {:?}",address);
                    
                    // Send feedback
                    self.feedback_manager.notify_listener_expired(
                        address.to_string(),
                    ).await;
                }
                SwarmEvent::NewExternalAddrCandidate { address, .. } => {
                    debug!(target:"p2p","new external addr candidate: {:?}",address);
                    
                    // Send feedback
                    self.feedback_manager.notify_external_addr_candidate(
                        address.to_string(),
                    ).await;
                }
                SwarmEvent::ExternalAddrConfirmed { address, .. } => {
                    debug!(target:"p2p","external addr confirmed: {:?}",address);
                    
                    // Send feedback
                    self.feedback_manager.notify_external_addr_confirmed(
                        address.to_string(),
                    ).await;
                }
                SwarmEvent::ExternalAddrExpired { address, .. } => {
                    debug!(target:"p2p","external addr expired: {:?}",address);
                    
                    // Send feedback
                    self.feedback_manager.notify_external_addr_expired(
                        address.to_string(),
                    ).await;
                }
                SwarmEvent::NewExternalAddrOfPeer {
                    peer_id, address, ..
                } => {
                    info!(target:"p2p","new external addr of peer: {:?}, {:?}",peer_id,address);
                    
                    // Send feedback
                    self.feedback_manager.notify_external_addr_of_peer(
                        peer_id,
                        address.to_string(),
                    ).await;
                }
                _ => {
                    info!(target:"p2p","swarm event: {:?}",swarm_event);
                    
                    // Send feedback for unknown events
                    self.feedback_manager.notify_unknown_event(
                        "unknown_swarm_event".to_string(),
                        format!("{:?}", swarm_event),
                    ).await;
                }
            }
        }
    }

    async fn handle_relay_event(&self, event: RelayServerEvent) {
        match event {
            RelayServerEvent::CircuitClosed {
                src_peer_id,
                dst_peer_id,
                error,
            } => {
                info!(target:"p2p","relay message, circuit closed src_peer_id: {:?}, dst_peer_id: {:?}, error: {:?}",src_peer_id,dst_peer_id,error);
                // Count only via pair-level recorder to avoid double counting
                self.metrics.record_circuit_event_pair(
                    "closed",
                    &src_peer_id.to_string(),
                    &dst_peer_id.to_string(),
                    error.as_ref().map(|e| format!("{:?}", e)).as_deref(),
                );
                
                // Send feedback
                self.feedback_manager.notify_circuit_closed(
                    src_peer_id,
                    dst_peer_id,
                    error.map(|e| format!("{:?}", e)),
                ).await;
            }
            RelayServerEvent::CircuitReqAccepted {
                src_peer_id,
                dst_peer_id,
            } => {
                info!(target:"p2p","relay message, circuit req accepted src_peer_id: {:?}, dst_peer_id: {:?}",src_peer_id,dst_peer_id);
                // Count only via pair-level recorder to avoid double counting
                self.metrics.record_circuit_event_pair(
                    "accepted",
                    &src_peer_id.to_string(),
                    &dst_peer_id.to_string(),
                    None,
                );
                
                // Send feedback
                self.feedback_manager.notify_circuit_accepted(
                    src_peer_id,
                    dst_peer_id,
                ).await;
            }
            RelayServerEvent::CircuitReqDenied {
                src_peer_id,
                dst_peer_id,
                status,
            } => {
                info!(target:"p2p","relay message, circuit req denied status: {:?}, src_peer_id: {:?}, dst_peer_id: {:?}",status,src_peer_id,dst_peer_id);
                self.metrics
                    .record_circuit_event("denied", Some(&src_peer_id.to_string()), None);
                
                // Send feedback
                self.feedback_manager.notify_circuit_denied(
                    src_peer_id,
                    dst_peer_id,
                    format!("{:?}", status),
                ).await;
            }
            RelayServerEvent::ReservationReqAccepted {
                src_peer_id,
                renewed,
            } => {
                info!(target:"p2p","relay message, reservation req accepted src_peer_id: {:?}, renewed: {:?}",src_peer_id,renewed);
                // Count only via peer-level recorder to avoid double counting
                self.metrics
                    .record_reservation_event_peer("accepted", &src_peer_id.to_string());
                
                // Send feedback
                self.feedback_manager.notify_reservation_accepted(
                    src_peer_id,
                    renewed,
                ).await;
            }
            RelayServerEvent::ReservationReqDenied {
                src_peer_id,
                status,
            } => {
                info!(target:"p2p","relay message, reservation req denied status: {:?}, src_peer_id: {:?}",status,src_peer_id);
                self.metrics.record_reservation_event(
                    "denied",
                    Some(&src_peer_id.to_string()),
                    None,
                );
                
                // Send feedback
                self.feedback_manager.notify_reservation_denied(
                    src_peer_id,
                    format!("{:?}", status),
                ).await;
            }
            RelayServerEvent::ReservationTimedOut { src_peer_id } => {
                info!(target:"p2p","relay message, reservation timed out: {:?}",src_peer_id);
                self.metrics.record_reservation_event(
                    "timed_out",
                    Some(&src_peer_id.to_string()),
                    None,
                );
                
                // Send feedback
                self.feedback_manager.notify_reservation_timed_out(
                    src_peer_id,
                ).await;
            }
            RelayServerEvent::ReservationClosed { src_peer_id } => {
                info!(target:"p2p","relay message, reservation closed: {:?}",src_peer_id);
                // Count only via peer-level recorder to avoid double counting
                self.metrics
                    .record_reservation_event_peer("closed", &src_peer_id.to_string());
                
                // Send feedback
                self.feedback_manager.notify_reservation_closed(
                    src_peer_id,
                    "graceful_close".to_string(),
                ).await;
            }
            _ => {
                trace!(target:"p2p","relay message: {:?}",event);
                
                // Send feedback for unknown relay events
                self.feedback_manager.notify_unknown_event(
                    "unknown_relay_event".to_string(),
                    format!("{:?}", event),
                ).await;
            }
        }
    }

    // ========== PUBLIC FEEDBACK API ==========

    /// Subscribe to all circuit events
    pub async fn subscribe_all_circuits(&self) -> tokio::sync::mpsc::UnboundedReceiver<crate::event_feedback::EventResult> {
        use crate::event_feedback::EventSubscription;
        self.feedback_manager.subscribe(EventSubscription::AllCircuits).await
    }

    /// Subscribe to all reservation events
    pub async fn subscribe_all_reservations(&self) -> tokio::sync::mpsc::UnboundedReceiver<crate::event_feedback::EventResult> {
        use crate::event_feedback::EventSubscription;
        self.feedback_manager.subscribe(EventSubscription::AllReservations).await
    }

    /// Subscribe to all connection events
    pub async fn subscribe_all_connections(&self) -> tokio::sync::mpsc::UnboundedReceiver<crate::event_feedback::EventResult> {
        use crate::event_feedback::EventSubscription;
        self.feedback_manager.subscribe(EventSubscription::AllConnections).await
    }

    /// Subscribe to all events
    pub async fn subscribe_all_events(&self) -> tokio::sync::mpsc::UnboundedReceiver<crate::event_feedback::EventResult> {
        use crate::event_feedback::EventSubscription;
        self.feedback_manager.subscribe(EventSubscription::AllEvents).await
    }

    /// Subscribe to a specific circuit (one-time feedback when closed/denied)
    pub async fn subscribe_circuit(
        &self,
        src_peer: PeerId,
        dst_peer: PeerId,
    ) -> tokio::sync::oneshot::Receiver<crate::event_feedback::EventResult> {
        self.feedback_manager.subscribe_circuit_once(src_peer, dst_peer).await
    }

    /// Subscribe to a specific reservation (one-time feedback when closed/denied/timeout)
    pub async fn subscribe_reservation(
        &self,
        peer: PeerId,
    ) -> tokio::sync::oneshot::Receiver<crate::event_feedback::EventResult> {
        self.feedback_manager.subscribe_reservation_once(peer).await
    }

    /// Subscribe to a specific connection (one-time feedback when closed/error)
    pub async fn subscribe_connection(
        &self,
        peer: PeerId,
    ) -> tokio::sync::oneshot::Receiver<crate::event_feedback::EventResult> {
        self.feedback_manager.subscribe_connection_once(peer).await
    }

    /// Get statistics on active events
    pub async fn get_feedback_stats(&self) -> (usize, usize, usize) {
        (
            self.feedback_manager.get_active_circuits_count().await,
            self.feedback_manager.get_active_reservations_count().await,
            self.feedback_manager.get_active_connections_count().await,
        )
    }
}
