use libp2p::futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, Stream, StreamExt};
use libp2p::relay::Behaviour as RelayBehaviour;
use libp2p::swarm::{derive_prelude, NetworkBehaviour};
use libp2p::swarm::{NetworkInfo, SwarmEvent};
use libp2p::{Multiaddr, PeerId, StreamProtocol, Swarm, SwarmBuilder};
use libp2p_kad::store::{MemoryStore, MemoryStoreConfig};
use libp2p_kad::{Behaviour as DhtBehaviour, Record, K_VALUE};

use libp2p::relay::Event as RelayServerEvent;
use libp2p_kad::Event as DhtEvent;

use anyhow::anyhow;
use log::{error, info, trace};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{mpsc, Mutex};
use tokio::time::{interval, Duration};

use crate::telemetry::{
    MetricsCounters, NetworkMetrics, RelayMetrics, RelayServerMetrics, SystemMetrics,
};

#[derive(NetworkBehaviour)]
#[behaviour(prelude = "libp2p::swarm::derive_prelude")]
pub struct RelayServerBehaviour<TStore> {
    pub relay_server: libp2p::relay::Behaviour,
    pub dht: DhtBehaviour<TStore>,
}

impl<TStore> RelayServerBehaviour<TStore> {
    pub fn new(relay_behaviour: libp2p::relay::Behaviour, dht: DhtBehaviour<TStore>) -> Self {
        Self {
            relay_server: relay_behaviour,
            dht,
        }
    }
}

pub struct RelayP2pWorker {
    pub peer_id: PeerId,
    pub swarm: Arc<Mutex<Swarm<RelayServerBehaviour<MemoryStore>>>>,
    pub external_multi_addr: Multiaddr,
    pub listening_addr: Multiaddr,
    /// Metrics counters for tracking statistics
    metrics_counters: Arc<MetricsCounters>,
    /// Start time for uptime calculation
    start_time: Instant,
}

impl RelayP2pWorker {
    pub fn new(
        dns: String,
        port: u16,
        metrics_counters: Arc<MetricsCounters>,
    ) -> Result<Self, anyhow::Error> {
        let self_keypair = libp2p::identity::Keypair::generate_ed25519();
        let peer_id = PeerId::from(self_keypair.public());

        let url = format!("/dns/{}/tcp/{}/p2p/{}", dns, port, peer_id);
        let external_multi_addr:Multiaddr = url
            .parse()
            .map_err(|err| anyhow!("failed to parse multi addr, caused by: {err}"))?;

        let listening_addr = format!("/ip6/::/tcp/{}", port);
        let listening_multi_addr = listening_addr
            .parse()
            .map_err(|err| anyhow!("failed to parse listening addr, caused by: {err}"))?;

        let dht_behaviour = DhtBehaviour::<MemoryStore>::new(
            peer_id.clone(),
            MemoryStore::with_config(
                peer_id,
                MemoryStoreConfig {
                    max_records: 10_000,
                    max_value_bytes: 50,
                    max_provided_keys: 10_000,
                    max_providers_per_key: K_VALUE.get(),
                },
            ),
        );
        let relay_behaviour = RelayServerBehaviour::<MemoryStore>::new(
            libp2p::relay::Behaviour::new(peer_id.clone(), libp2p::relay::Config::default()),
            dht_behaviour,
        );

        let transport_tcp = libp2p::tcp::Config::new().nodelay(true).port_reuse(true);

        let mut swarm = SwarmBuilder::with_existing_identity(self_keypair)
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

        swarm.add_external_address(external_multi_addr.clone());

        Ok(Self {
            peer_id,
            swarm: Arc::new(Mutex::new(swarm)),
            external_multi_addr,
            listening_addr: listening_multi_addr,
            metrics_counters,
            start_time: Instant::now(),
        })
    }

    pub async fn start_swarm(&mut self) -> Result<(), anyhow::Error> {
        let listening_addr = &self.listening_addr;
        let _listening_id = self.swarm.lock().await.listen_on(listening_addr.clone())?;
        trace!(target:"p2p","relay server listening to: {:?}",listening_addr.clone());

        self.swarm
            .lock()
            .await
            .behaviour_mut()
            .dht
            .add_address(&self.peer_id, listening_addr.clone());

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
                    self.metrics_counters
                        .active_connections
                        .fetch_add(1, Ordering::Relaxed);
                    self.metrics_counters
                        .successful_connections
                        .fetch_add(1, Ordering::Relaxed);
                    self.metrics_counters
                        .total_connections
                        .fetch_add(1, Ordering::Relaxed);
                }
                SwarmEvent::ConnectionClosed { peer_id, cause, .. } => {
                    info!(target:"p2p","connection closed: {:?}, reason {:?}",peer_id,cause);
                    self.metrics_counters
                        .active_connections
                        .fetch_sub(1, Ordering::Relaxed);
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

    async fn handle_dht_event(&mut self, event: DhtEvent) -> Result<(), anyhow::Error> {
        match event {
            DhtEvent::InboundRequest { request } => {
                info!(target:"p2p","dht inbound request: {:?}",request);
            }
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
            _ => {}
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
                self.metrics_counters
                    .active_circuits
                    .fetch_add(1, Ordering::Relaxed);
                self.metrics_counters
                    .total_circuits_created
                    .fetch_add(1, Ordering::Relaxed);
            }
            RelayServerEvent::CircuitReqDenied {
                src_peer_id,
                dst_peer_id,
                status,
            } => {
                info!(target:"p2p","relay message, circuit req denied status: {:?}, src_peer_id: {:?}, dst_peer_id: {:?}",status,src_peer_id,dst_peer_id);
                self.metrics_counters
                    .circuit_errors
                    .fetch_add(1, Ordering::Relaxed);
            }
            RelayServerEvent::ReservationReqAccepted {
                src_peer_id,
                renewed,
            } => {
                info!(target:"p2p","relay message, reservation req accepted src_peer_id: {:?}, renewed: {:?}",src_peer_id,renewed);
                self.metrics_counters
                    .active_reservations
                    .fetch_add(1, Ordering::Relaxed);
                self.metrics_counters
                    .total_reservations_created
                    .fetch_add(1, Ordering::Relaxed);
            }
            RelayServerEvent::ReservationReqDenied {
                src_peer_id,
                status,
            } => {
                info!(target:"p2p","relay message, reservation req denied status: {:?}, src_peer_id: {:?}",status,src_peer_id);
                self.metrics_counters
                    .reservation_denials
                    .fetch_add(1, Ordering::Relaxed);
            }
            RelayServerEvent::ReservationTimedOut { src_peer_id } => {
                info!(target:"p2p","relay message, reservation timed out: {:?}",src_peer_id);
                self.metrics_counters
                    .active_reservations
                    .fetch_sub(1, Ordering::Relaxed);
                self.metrics_counters
                    .total_reservations_expired
                    .fetch_add(1, Ordering::Relaxed);
            }
            RelayServerEvent::ReservationClosed { src_peer_id } => {
                info!(target:"p2p","relay message, reservation closed: {:?}",src_peer_id);
                self.metrics_counters
                    .active_reservations
                    .fetch_sub(1, Ordering::Relaxed);
            }
            _ => {
                trace!(target:"p2p","relay message: {:?}",event);
            }
        }
    }

    /// Get a reference to the swarm for telemetry worker
    pub fn get_swarm(&self) -> Arc<Mutex<Swarm<RelayServerBehaviour<MemoryStore>>>> {
        self.swarm.clone()
    }

    /// Get a reference to metrics counters for external updates
    pub fn metrics_counters(&self) -> &Arc<MetricsCounters> {
        &self.metrics_counters
    }
}
