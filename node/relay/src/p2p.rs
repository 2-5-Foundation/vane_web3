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

        Ok(Self {
            peer_id,
            swarm: Arc::new(Mutex::new(relay_swarm)),
            external_multi_addr,
            listening_addr: (ipv4_listening_addr, ipv6_listening_addr),
            metrics,
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
                    ..
                } => {
                    info!(target:"p2p","connection established {:?}: duration {:?}",peer_id,established_in);
                    self.metrics.record_connection_event(
                        "established",
                        Some(&peer_id.to_string()),
                        None,
                    );
                }
                SwarmEvent::ConnectionClosed { peer_id, cause, .. } => {
                    info!(target:"p2p","connection closed: {:?}, reason {:?}",peer_id,cause);
                    self.metrics.record_connection_event(
                        "closed",
                        Some(&peer_id.to_string()),
                        None,
                    );
                }
                SwarmEvent::Dialing { peer_id, .. } => {
                    info!(target:"p2p","dialing: {:?}",peer_id);
                    if let Some(pid) = peer_id {
                        self.metrics.record_connection_event(
                            "dialing",
                            Some(&pid.to_string()),
                            None,
                        );
                    }
                }
                SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
                    info!(target:"p2p","outgoing connection error: {:?}, error {:?}",peer_id,error);
                }
                SwarmEvent::ListenerClosed {
                    listener_id,
                    addresses,
                    reason,
                    ..
                } => {
                    info!(target:"p2p","listener closed: {:?}, addresses {:?}, reason {:?}",listener_id,addresses,reason);
                }
                SwarmEvent::NewListenAddr { address, .. } => {
                    debug!(target:"p2p","new listen addr: {:?}",address);
                }
                SwarmEvent::ExpiredListenAddr { address, .. } => {
                    debug!(target:"p2p","expired listen addr: {:?}",address);
                }
                SwarmEvent::NewExternalAddrCandidate { address, .. } => {
                    debug!(target:"p2p","new external addr candidate: {:?}",address);
                }
                SwarmEvent::ExternalAddrConfirmed { address, .. } => {
                    debug!(target:"p2p","external addr confirmed: {:?}",address);
                }
                SwarmEvent::ExternalAddrExpired { address, .. } => {
                    debug!(target:"p2p","external addr expired: {:?}",address);
                }
                SwarmEvent::NewExternalAddrOfPeer {
                    peer_id, address, ..
                } => {
                    info!(target:"p2p","new external addr of peer: {:?}, {:?}",peer_id,address);
                }
                _ => {
                    info!(target:"p2p","swarm event: {:?}",swarm_event);
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
            }
            RelayServerEvent::CircuitReqDenied {
                src_peer_id,
                dst_peer_id,
                status,
            } => {
                info!(target:"p2p","relay message, circuit req denied status: {:?}, src_peer_id: {:?}, dst_peer_id: {:?}",status,src_peer_id,dst_peer_id);
                self.metrics
                    .record_circuit_event("denied", Some(&src_peer_id.to_string()), None);
            }
            RelayServerEvent::ReservationReqAccepted {
                src_peer_id,
                renewed,
            } => {
                info!(target:"p2p","relay message, reservation req accepted src_peer_id: {:?}, renewed: {:?}",src_peer_id,renewed);
                // Count only via peer-level recorder to avoid double counting
                self.metrics
                    .record_reservation_event_peer("accepted", &src_peer_id.to_string());
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
            }
            RelayServerEvent::ReservationTimedOut { src_peer_id } => {
                info!(target:"p2p","relay message, reservation timed out: {:?}",src_peer_id);
                self.metrics.record_reservation_event(
                    "timed_out",
                    Some(&src_peer_id.to_string()),
                    None,
                );
            }
            RelayServerEvent::ReservationClosed { src_peer_id } => {
                info!(target:"p2p","relay message, reservation closed: {:?}",src_peer_id);
                // Count only via peer-level recorder to avoid double counting
                self.metrics
                    .record_reservation_event_peer("closed", &src_peer_id.to_string());
            }
            _ => {
                trace!(target:"p2p","relay message: {:?}",event);
            }
        }
    }
}
