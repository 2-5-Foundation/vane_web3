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

use anyhow::anyhow;
use futures::future::Either;
use libp2p::kad::Event as DhtEvent;
use libp2p::relay::Event as RelayServerEvent;
use libp2p::Transport;
use log::{error, info, trace};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{mpsc, Mutex};
use tokio::time::{interval, Duration};

use std::num::NonZeroU32;
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
    pub listening_addr: (Multiaddr, Multiaddr),
}

impl RelayP2pWorker {
    pub async fn new(dns: String, port: u16, live: bool) -> Result<Self, anyhow::Error> {
        let self_keypair = libp2p::identity::Keypair::generate_ed25519();
        let peer_id = PeerId::from(self_keypair.public());

        let external_multi_addr = if live {
            Multiaddr::empty()
                .with(libp2p::multiaddr::Protocol::Dns(dns.into()))
                .with(libp2p::multiaddr::Protocol::Tcp(port))
                .with(libp2p::multiaddr::Protocol::Ws("/".into()))
                .with(libp2p::multiaddr::Protocol::P2p(peer_id.clone()))
        } else {
            Multiaddr::from(Ipv6Addr::UNSPECIFIED)
                .with(libp2p::multiaddr::Protocol::Tcp(port))
                .with(libp2p::multiaddr::Protocol::Ws("/".into()))
                .with(libp2p::multiaddr::Protocol::P2p(peer_id.clone()))
        };

        let listening_multi_addr = Multiaddr::from(Ipv6Addr::UNSPECIFIED)
            .with(libp2p::multiaddr::Protocol::Tcp(port))
            .with(libp2p::multiaddr::Protocol::Ws("/".into()));

        let listening_multi_addr_ipv4 = Multiaddr::from(Ipv4Addr::UNSPECIFIED)
            .with(libp2p::multiaddr::Protocol::Tcp(port))
            .with(libp2p::multiaddr::Protocol::Ws("/".into()));

        let dht_behaviour = DhtBehaviour::with_config(
            peer_id.clone(),
            MemoryStore::with_config(
                peer_id.clone(),
                MemoryStoreConfig {
                    max_records: 50_000,              // Same as relay
                    max_value_bytes: 2048 * 50,       // Increase this
                    max_providers_per_key: 1024 * 50, // Keep this
                    max_provided_keys: 10_000,        // Increase this
                },
            ),
            KademliaConfig::new(StreamProtocol::new("/vane_dht_protocol")),
        );

        let mut relay_config = libp2p::relay::Config {
            max_reservations: 10_000,
            max_reservations_per_peer: 4,
            reservation_duration: Duration::from_secs(60 * 60),
            max_circuits: 1000,
            max_circuit_bytes: 1024 * 1024 * 10,
            max_circuit_duration: Duration::from_secs(20 * 60),
            max_circuits_per_peer: 20,
            ..Default::default()
        };

        relay_config
            .reservation_rate_per_peer(NonZeroU32::new(30).unwrap(), Duration::from_secs(120))
            .reservation_rate_per_ip(NonZeroU32::new(60).unwrap(), Duration::from_secs(60))
            .circuit_src_per_peer(NonZeroU32::new(20).unwrap(), Duration::from_secs(10))
            .circuit_src_per_ip(NonZeroU32::new(20).unwrap(), Duration::from_secs(10));

        let relay_behaviour = RelayServerBehaviour::<MemoryStore>::new(
            libp2p::relay::Behaviour::new(
                peer_id.clone(),
                libp2p::relay::Config {
                    max_reservations: 1000,
                    max_reservations_per_peer: 100,
                    reservation_duration: Duration::from_secs(60),
                    reservation_rate_limiters: vec![],
                    max_circuits: 1000,
                    max_circuits_per_peer: 100,
                    max_circuit_duration: Duration::from_secs(60),
                    max_circuit_bytes: 1024 * 1024 * 10,
                    circuit_src_rate_limiters: vec![],
                },
            ),
            dht_behaviour,
        );

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
                cfg.with_idle_connection_timeout(tokio::time::Duration::from_secs(300))
            })
            .build();

        relay_swarm.add_external_address(external_multi_addr.clone());

        Ok(Self {
            peer_id,
            swarm: Arc::new(Mutex::new(relay_swarm)),
            external_multi_addr,
            listening_addr: (listening_multi_addr, listening_multi_addr_ipv4),
        })
    }

    pub async fn start_swarm(&mut self) -> Result<(), anyhow::Error> {
        let listening_addr = &self.listening_addr.0;
        let _listening_id = self.swarm.lock().await.listen_on(listening_addr.clone())?;
        trace!(target:"p2p","relay server listening to: {:?}",listening_addr.clone());
        let listening_addr_ipv4 = &self.listening_addr.1;
        let _listening_id_ipv4 = self
            .swarm
            .lock()
            .await
            .listen_on(listening_addr_ipv4.clone())?;
        trace!(target:"p2p","relay server listening to ipv4: {:?}",listening_addr_ipv4.clone());

        self.swarm
            .lock()
            .await
            .behaviour_mut()
            .dht
            .add_address(&self.peer_id, listening_addr.clone());

        loop {
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
                }
                SwarmEvent::ConnectionClosed { peer_id, cause, .. } => {
                    info!(target:"p2p","connection closed: {:?}, reason {:?}",peer_id,cause);
                }
                SwarmEvent::Dialing { peer_id, .. } => {
                    info!(target:"p2p","dialing: {:?}",peer_id);
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
                    info!(target:"p2p","new listen addr: {:?}",address);
                }
                SwarmEvent::ExpiredListenAddr { address, .. } => {
                    info!(target:"p2p","expired listen addr: {:?}",address);
                }
                SwarmEvent::NewExternalAddrCandidate { address, .. } => {
                    info!(target:"p2p","new external addr candidate: {:?}",address);
                }
                SwarmEvent::ExternalAddrConfirmed { address, .. } => {
                    info!(target:"p2p","external addr confirmed: {:?}",address);
                }
                SwarmEvent::ExternalAddrExpired { address, .. } => {
                    info!(target:"p2p","external addr expired: {:?}",address);
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

    async fn handle_dht_event(&mut self, event: DhtEvent) -> Result<(), anyhow::Error> {
        match event {
            DhtEvent::InboundRequest { request } => match request {
                InboundRequest::PutRecord { record, source, .. } => {
                    info!(target:"p2p","dht inbound request: PutRecord {{ source: {:?}, record: {:?} }}", source, record);
                }
                InboundRequest::GetProvider {
                    num_closer_peers,
                    num_provider_peers,
                } => {
                    info!(target:"p2p","dht inbound request: GetProvider {{ num_closer_peers: {:?}, num_provider_peers: {:?} }}", 
                              num_closer_peers, num_provider_peers);
                }
                InboundRequest::GetRecord {
                    num_closer_peers,
                    present_locally,
                } => {
                    info!(target:"p2p","dht inbound request: GetRecord {{ num_closer_peers: {:?}, present_locally: {:?} }}", num_closer_peers, present_locally);
                }
                InboundRequest::AddProvider { record } => {
                    info!(target:"p2p","dht inbound request: AddProvider {{ key: {:?} }}", record);
                }
                InboundRequest::FindNode { num_closer_peers } => {
                    info!(target:"p2p","dht inbound request: FindNode {{ num_closer_peers: {:?} }}", 
                              num_closer_peers);
                }
            },
            DhtEvent::OutboundQueryProgressed { id, result, .. } => match result {
                QueryResult::GetRecord(GetRecordResult::Ok(GetRecordOk::FoundRecord(
                    PeerRecord { record, .. },
                ))) => {
                    info!(target:"p2p","dht found record: key={:?}, value={:?}", record.key, record.value);
                }
                QueryResult::GetRecord(GetRecordResult::Ok(
                    GetRecordOk::FinishedWithNoAdditionalRecord { .. },
                )) => {
                    info!(target:"p2p","dht get record finished: no record found");
                }
                QueryResult::GetRecord(GetRecordResult::Err(e)) => {
                    error!(target:"p2p","dht get record error: {:?}", e);
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
            }
            RelayServerEvent::CircuitReqDenied {
                src_peer_id,
                dst_peer_id,
                status,
            } => {
                info!(target:"p2p","relay message, circuit req denied status: {:?}, src_peer_id: {:?}, dst_peer_id: {:?}",status,src_peer_id,dst_peer_id);
            }
            RelayServerEvent::ReservationReqAccepted {
                src_peer_id,
                renewed,
            } => {
                info!(target:"p2p","relay message, reservation req accepted src_peer_id: {:?}, renewed: {:?}",src_peer_id,renewed);
            }
            RelayServerEvent::ReservationReqDenied {
                src_peer_id,
                status,
            } => {
                info!(target:"p2p","relay message, reservation req denied status: {:?}, src_peer_id: {:?}",status,src_peer_id);
            }
            RelayServerEvent::ReservationTimedOut { src_peer_id } => {
                info!(target:"p2p","relay message, reservation timed out: {:?}",src_peer_id);
            }
            RelayServerEvent::ReservationClosed { src_peer_id } => {
                info!(target:"p2p","relay message, reservation closed: {:?}",src_peer_id);
            }
            _ => {
                trace!(target:"p2p","relay message: {:?}",event);
            }
        }
    }
}
