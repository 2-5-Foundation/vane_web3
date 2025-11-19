use std::collections::{HashMap, HashSet};
use std::sync::Once;

use core::{cell::RefCell, pin::Pin, str::FromStr};

use alloc::{boxed::Box, collections::VecDeque, format, rc::Rc, string::String};

use anyhow::{anyhow, Error};
use futures::{future::Either, FutureExt, StreamExt};
use gloo_timers::future::TimeoutFuture;
use log::{debug, error, info, trace, warn};
use serde::Deserialize;
use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen_futures::wasm_bindgen::closure::Closure;
use web_sys::wasm_bindgen::JsCast;

use libp2p::{
    core::transport::{upgrade, OrTransport, Transport as TransportTrait},
    kad::{
        store::{MemoryStore, MemoryStoreConfig},
        Behaviour as DhtBehaviour, Config as KademliaConfig, Event as DhtEvent, GetProvidersOk,
        GetProvidersResult, GetRecordOk, GetRecordResult, InboundRequest, PeerRecord, QueryResult,
    },
    multiaddr::Protocol,
    relay::client::{Behaviour as RelayClientBehaviour, Event as RelayClientEvent},
    request_response::{
        json::Behaviour as JsonBehaviour, Behaviour, Codec, Event, Message, ProtocolSupport,
        ResponseChannel,
    },
    swarm::{derive_prelude, NetworkBehaviour, SwarmEvent},
    Multiaddr, PeerId, StreamProtocol, Swarm, SwarmBuilder,
};
use libp2p_websocket_websys;

pub use codec::Encode;
use db_wasm::{DbWorker, OpfsRedbWorker};
use primitives::data_structure::{
    ConnectionState, DHTResponse, DbWorkerInterface, HashId, NetworkCommand, P2pEventResult,
    SwarmMessage, TxStateMachine,
};

// p2p event notification worker
#[derive(Clone)]
pub struct P2pEventNotifSubSystem {
    pub sender: Rc<RefCell<tokio_with_wasm::alias::sync::mpsc::Sender<P2pEventResult>>>,
    pub recv: Rc<RefCell<tokio_with_wasm::alias::sync::mpsc::Receiver<P2pEventResult>>>,
}

impl P2pEventNotifSubSystem {
    pub fn new(sender: Rc<RefCell<tokio_with_wasm::alias::sync::mpsc::Sender<P2pEventResult>>>, recv: Rc<RefCell<tokio_with_wasm::alias::sync::mpsc::Receiver<P2pEventResult>>>) -> Self {
        Self { sender, recv }
    }
}

#[derive(Clone)]
pub struct WasmP2pWorker {
    pub live: bool,
    pub node_id: PeerId,
    pub user_circuit_multi_addr: Multiaddr,
    pub relay_multi_addr: Multiaddr,
    pub user_account_id: String,
    pub wasm_swarm: Rc<RefCell<Swarm<WasmRelayBehaviour>>>,
    pub wasm_p2p_command_recv:
        Rc<RefCell<tokio_with_wasm::alias::sync::mpsc::Receiver<NetworkCommand>>>,
    pub wasm_pending_request:
        Rc<RefCell<HashMap<u64, ResponseChannel<Result<TxStateMachine, String>>>>>,
    pub current_req: VecDeque<SwarmMessage>,
    pub dht_channel_query: Rc<tokio_with_wasm::alias::sync::mpsc::Sender<(Option<Multiaddr>, u32)>>,
    pub dht_announce_once: Rc<Once>,
    pub relay_connection_state: Rc<RefCell<ConnectionState>>,
    pub p2p_event_notif_sub_system: Rc<P2pEventNotifSubSystem>,
    //  cache of the receiver address matched with peerId
    pub receiver_address_cache: Rc<RefCell<HashMap<PeerId, String>>>,
}

#[derive(NetworkBehaviour)]
#[behaviour(prelude = "libp2p::swarm::derive_prelude")]
pub struct WasmRelayBehaviour {
    pub relay_client: libp2p::relay::client::Behaviour,
    pub app_json: JsonBehaviour<TxStateMachine, Result<TxStateMachine, String>>,
    pub app_client_dht: DhtBehaviour<MemoryStore>,
}

impl WasmRelayBehaviour {
    pub fn new(
        protocols: impl IntoIterator<Item = (StreamProtocol, libp2p::request_response::ProtocolSupport)>,
        config: libp2p::request_response::Config,
        relay_behaviour: libp2p::relay::client::Behaviour,
        dht_behaviour: DhtBehaviour<MemoryStore>,
    ) -> Self {
        let json_behaviour = JsonBehaviour::new(protocols, config);

        Self {
            relay_client: relay_behaviour,
            app_json: json_behaviour,
            app_client_dht: dht_behaviour,
        }
    }
}

impl WasmP2pWorker {
    pub async fn new(
        live: bool,
        db_worker: Rc<DbWorker>,
        relay_node_multi_addr: String,
        user_account_id: String,
        command_recv_channel: tokio_with_wasm::alias::sync::mpsc::Receiver<NetworkCommand>,
        dht_query_result_tx: tokio_with_wasm::alias::sync::mpsc::Sender<(Option<Multiaddr>, u32)>,
        p2p_event_notif_sub_system: Rc<P2pEventNotifSubSystem>,
    ) -> Result<Self, anyhow::Error> {
        let self_keypair = libp2p::identity::Keypair::generate_ed25519();

        let peer_id = self_keypair.public().to_peer_id().to_base58();

        let peer_id: PeerId = PeerId::from_str(&peer_id)
            .map_err(|err| anyhow!("failed to convert PeerId, caused by: {err}"))?;
        // user account and multi_addr
        let relay_multi_addr =
            Multiaddr::try_from(relay_node_multi_addr).expect("failed to parse relay multiaddr");
            
        let user_circuit_multi_addr = relay_multi_addr
            .clone()
            .with(libp2p::multiaddr::Protocol::P2pCircuit);

        let (relay_transport, relay_behaviour) = libp2p::relay::client::new(peer_id.clone());

        let ws_base = libp2p_websocket_websys::Transport::default();

        let combined_transport = OrTransport::new(relay_transport, ws_base)
            .upgrade(upgrade::Version::V1)
            .authenticate(libp2p::noise::Config::new(&self_keypair)?)
            .multiplex(libp2p::yamux::Config::default())
            .boxed();

        let request_response_config = libp2p::request_response::Config::default()
            .with_request_timeout(core::time::Duration::from_secs(60 * 60)); // 10 minutes waiting time for a response with 10 second buffer

        let mut dht_config = KademliaConfig::new(StreamProtocol::new("/vane_dht_protocol"));
        dht_config.set_kbucket_inserts(libp2p::kad::BucketInserts::Manual);

        let dht_behaviour = DhtBehaviour::with_config(
            peer_id.clone(),
            MemoryStore::with_config(
                peer_id.clone(),
                MemoryStoreConfig {
                    max_records: 10_000,
                    max_value_bytes: 2048,
                    max_providers_per_key: 1024,
                    max_provided_keys: 100,
                },
            ),
            dht_config,
        );

        let combined_behaviour = WasmRelayBehaviour::new(
            vec![(
                StreamProtocol::new("/wasm_relay_client_protocol"),
                ProtocolSupport::Full,
            )],
            request_response_config,
            relay_behaviour,
            dht_behaviour,
        );

        let mut wasm_swarm = Swarm::new(
            combined_transport,
            combined_behaviour,
            peer_id,
            libp2p::swarm::Config::with_wasm_executor()
                .with_idle_connection_timeout(std::time::Duration::from_secs(60 * 60)),
        );

        wasm_swarm.add_external_address(user_circuit_multi_addr.clone());

        Ok(Self {
            live,
            node_id: peer_id,
            user_circuit_multi_addr,
            relay_multi_addr,
            user_account_id,
            wasm_p2p_command_recv: Rc::new(RefCell::new(command_recv_channel)),
            current_req: Default::default(),
            wasm_swarm: Rc::new(RefCell::new(wasm_swarm)),
            wasm_pending_request: Rc::new(RefCell::new(Default::default())),
            dht_channel_query: Rc::new(dht_query_result_tx),
            dht_announce_once: Rc::new(Once::new()),
            relay_connection_state: Rc::new(RefCell::new(ConnectionState::default())),
            p2p_event_notif_sub_system: p2p_event_notif_sub_system,
            receiver_address_cache: Rc::new(RefCell::new(HashMap::new())),
        })
    }

    pub async fn handle_swarm_events(
        &self,
        pending_request: Rc<RefCell<HashMap<u64, ResponseChannel<Result<TxStateMachine, String>>>>>,
        events: SwarmEvent<WasmRelayBehaviourEvent>,
        sender: Rc<
            RefCell<tokio_with_wasm::alias::sync::mpsc::Sender<Result<SwarmMessage, Error>>>,
        >,
    ) {
        match events {
            // Custom events from the wasm relay behaviour
            SwarmEvent::Behaviour(wasm_relay_behaviour) => match wasm_relay_behaviour {
                WasmRelayBehaviourEvent::AppJson(app_json_event) => {
                    self.handle_app_json_events(app_json_event, sender).await;
                }
                WasmRelayBehaviourEvent::RelayClient(relay_client_event) => {
                    self.handle_relay_events(
                        relay_client_event,
                        self.relay_connection_state.clone(),
                    )
                    .await;
                }
                WasmRelayBehaviourEvent::AppClientDht(app_client_dht_event) => {
                    self.handle_dht_events(app_client_dht_event).await;
                }
            },
            // Generic swarm events
            SwarmEvent::ConnectionEstablished {
                peer_id,
                num_established,
                ..
            } => {
                info!(target:"p2p","🟢 Connected to peer {} (connections: {})", peer_id, num_established);
                let receiver_address = self.receiver_address_cache.borrow().get(&peer_id).cloned().unwrap_or_else(|| {
                    warn!(target:"p2p", "Peer {} disconnected but not in address cache, using unknown", peer_id);
                    "unknown".to_string()
                });
                self.p2p_event_notif_sub_system
                    .sender
                    .borrow_mut()
                    .send(P2pEventResult::ReceiverConnected {
                        peer_id: peer_id.clone().to_base58(),
                        address: receiver_address,
                    })
                    .await
                    .expect("failed to send p2p event result");
            }

            SwarmEvent::IncomingConnection { send_back_addr, .. } => {
                info!(target:"p2p","📥 Incoming connection from {}", send_back_addr)
            }

            SwarmEvent::Dialing { peer_id, .. } => {
                info!(target:"p2p","📞 Dialing peer {}", peer_id.map(|p| p.to_string()).unwrap_or_else(|| "unknown".to_string()));

                if let Some(peer_id) = peer_id {
                    if let Some(address) = self.receiver_address_cache.borrow().get(&peer_id) {
                        self.p2p_event_notif_sub_system
                            .sender
                            .borrow_mut()
                            .send(P2pEventResult::Dialing {
                                peer_id: Some(peer_id.clone().to_base58()),
                                address: Some(address.clone()),
                            })
                            .await
                            .expect("failed to send p2p event result");
                    }
                }else{

                    self.p2p_event_notif_sub_system
                        .sender
                        .borrow_mut()
                        .send(P2pEventResult::Dialing {
                            peer_id: None,
                            address: None
                        })
                        .await
                        .expect("failed to send p2p event result");
                }
            }
            SwarmEvent::ListenerError { error, .. } => {
                error!(target: "p2p","❌ Listener error: {}", error);
            }
            SwarmEvent::ConnectionClosed { peer_id, cause, .. } => {
                info!(target:"p2p","🔴 Connection closed to peer {} (cause: {:?})", peer_id, cause);

                // Check if this was the relay peer
                let relay_peer_id = self.relay_multi_addr.iter().find_map(|protocol| {
                    if let Protocol::P2p(relay_peer) = protocol {
                        Some(relay_peer)
                    } else {
                        None
                    }
                });

                // Check if this is the relay peer
                let is_relay = relay_peer_id.map_or(false, |relay_peer| peer_id == relay_peer);

                if is_relay {
                    let now = (js_sys::Date::now() / 1000.0) as u64; // Convert from ms to seconds
                    *self.relay_connection_state.borrow_mut() =
                        ConnectionState::Disconnected(now);

                    self.p2p_event_notif_sub_system
                        .sender
                        .borrow_mut()
                        .send(P2pEventResult::RelayerConnectionClosed)
                        .await
                        .expect("failed to send p2p event result");

                    info!(target:"p2p","🔴 Relay connection lost, cause: {:?}", cause);
                } else {
                    // Handle regular peer disconnection
                    // Try to get address from cache, but don't panic if not found
                    let receiver_address = self
                        .receiver_address_cache
                        .borrow()
                        .get(&peer_id)
                        .cloned()
                        .unwrap_or_else(|| {
                            warn!(target:"p2p", "Peer {} disconnected but not in address cache, using unknown", peer_id);
                            "unknown".to_string()
                        });

                    self.p2p_event_notif_sub_system
                        .sender
                        .borrow_mut()
                        .send(P2pEventResult::PeerConnectionClosed {
                            peer_id: peer_id.clone().to_base58(),
                            address: receiver_address,
                        })
                        .await
                        .expect("failed to send p2p event result");
                }
            }
            SwarmEvent::IncomingConnectionError { error, .. } => {
                self.p2p_event_notif_sub_system
                    .sender
                    .borrow_mut()
                    .send(P2pEventResult::RecvIncomingConnectionError {
                        error: error.to_string(),
                    })
                    .await
                    .expect("failed to send p2p event result");
                error!(target:"p2p","❌ Incoming connection failed: {}", error)
            }

            SwarmEvent::OutgoingConnectionError { error, peer_id,.. } => {
                // idk if the peer is already resolved

                if let Some(peer_id) = peer_id {
                    if let Some(address) = self.receiver_address_cache.borrow().get(&peer_id) {
                        self.p2p_event_notif_sub_system
                            .sender
                            .borrow_mut()
                            .send(P2pEventResult::SenderOutgoingConnectionError {
                                error: error.to_string(),
                                address: Some(address.clone()),
                            })
                            .await
                            .expect("failed to send p2p event result");
                    }
                }else{
                    self.p2p_event_notif_sub_system
                        .sender
                        .borrow_mut()
                        .send(P2pEventResult::SenderOutgoingConnectionError {
                            error: error.to_string(),
                            address: None,
                        })
                        .await
                        .expect("failed to send p2p event result");
                }
                error!(target:"p2p","❌ Outgoing connection failed: {}", error)
            }

            SwarmEvent::ListenerClosed {
                reason, addresses, ..
            } => {
                self.p2p_event_notif_sub_system
                    .sender
                    .borrow_mut()
                    .send(P2pEventResult::PeerIsOffline)
                    .await
                    .expect("failed to send p2p event result");

                info!(target:"p2p","🔌 Listener closed: {:?} {:?}", reason, addresses)
            }
            SwarmEvent::NewListenAddr { address, .. } => {
                info!(target:"p2p","🎧 Listening on: {}", &address);
            }
            SwarmEvent::ExpiredListenAddr { address, .. } => {
                info!(target:"p2p","⏰ Listener address expired: {}", address)
            }
            SwarmEvent::NewExternalAddrCandidate { address, .. } => {
                info!(target:"p2p","🌐 External address candidate: {}", address)
            }
            SwarmEvent::ExternalAddrConfirmed { address } => {
                info!(target:"p2p","⚡ External address confirmed: {}", address);
                let account_key = self.user_account_id.clone();
                let mut value = address.to_string();
                if !self.live {
                    value = value.replace("/tcp/443", "/tcp/30333");
                }

                self.p2p_event_notif_sub_system
                    .sender
                    .borrow_mut()
                    .send(P2pEventResult::PeerIsOnline)
                    .await
                    .expect("failed to send p2p event result");

                // Use Once to ensure DHT announcement happens only once
                self.dht_announce_once.call_once(|| {
                    info!(target: "p2p", "Announcing to DHT for the first time");
                    wasm_bindgen_futures::spawn_local(async move {
                        // Retry up to 3 times with simple backoff
                        for attempt in 1..=3u32 {
                            match host_set_dht(account_key.clone(), value.clone()).await {
                                Ok(response) => {
                                    if let Some(error_msg) = &response.error {
                                        warn!(target: "p2p", "DHT announce attempt {attempt} failed: {error_msg}");
                                    } else {
                                        info!(target: "p2p","DHT record added and started providing: {response:?}");
                                        return;
                                    }
                                }
                                Err(e) => {
                                    warn!(target: "p2p","DHT announce attempt {attempt} internal error: {e:?}");
                                }
                            }
                            // backoff 300ms, 600ms, 900ms
                            let delay_ms = 300 * attempt;
                            TimeoutFuture::new(delay_ms.into()).await;
                        }
                        // Exhausted retries
                        panic!("fatal: DHT announce failed after 3 attempts");
                    });
                });
            }
            SwarmEvent::ExternalAddrExpired { address } => {
                info!(target:"p2p","⚡ External address expired: {}", address);
            }
            SwarmEvent::NewExternalAddrOfPeer {
                peer_id, address, ..
            } => {
                info!(target:"p2p","⚡ New external address of peer: {}", address);
            }
            _ => {
                info!(target:"p2p","⚡ Unhandled swarm event: {:?}", events);
            }
        }
    }

    async fn handle_dht_events(&self, dht_event: DhtEvent) {
        match dht_event {
            DhtEvent::InboundRequest { request } => match request {
                InboundRequest::PutRecord { record, source, .. } => {
                    info!(target:"p2p","dht inbound request: PutRecord {{ source: {:?}, record: {:?} }}", source, record);
                }
                InboundRequest::FindNode { num_closer_peers } => {
                    info!(target:"p2p","dht inbound request: FindNode {{ num_closer_peers: {:?} }}", 
                              num_closer_peers);
                }
                InboundRequest::GetProvider {
                    num_closer_peers,
                    num_provider_peers,
                } => {
                    info!(target:"p2p","dht inbound request: GetProvider {{ num_closer_peers: {:?}, num_provider_peers: {:?} }}", 
                              num_closer_peers, num_provider_peers);
                }
                InboundRequest::AddProvider { record } => {
                    info!(target:"p2p","dht inbound request: AddProvider {{ key: {:?} }}", record);
                }
                InboundRequest::GetRecord {
                    num_closer_peers,
                    present_locally,
                } => {
                    info!(target:"p2p","dht inbound request: GetRecord {{ num_closer_peers: {:?}, present_locally: {:?} }}", num_closer_peers, present_locally);
                }
            },
            DhtEvent::OutboundQueryProgressed { id, result, .. } => match result {
                QueryResult::GetRecord(result) => match result {
                    GetRecordResult::Ok(GetRecordOk::FoundRecord(PeerRecord {
                        record, ..
                    })) => {
                        // gracefully shutdown point
                        // let multi_addr =
                        //     Multiaddr::try_from(record.value).expect("failed to parse multiaddr");
                        // self.dht_channel_query
                        //     .borrow_mut()
                        //     .send((Some(multi_addr), id))
                        //     .await
                        //     .expect("failed to send dht query result on the channel");
                    }
                    GetRecordResult::Err(e) => {
                        // self.dht_channel_query
                        //     .borrow_mut()
                        //     .send((None, id))
                        //     .await
                        //     .expect("failed to send dht query result on the channel");
                    }
                    GetRecordResult::Ok(GetRecordOk::FinishedWithNoAdditionalRecord {
                        cache_candidates,
                    }) => {
                        // self.dht_channel_query
                        //     .borrow_mut()
                        //     .send((None, id))
                        //     .await
                        //     .expect("failed to send dht query result on the channel");
                        // info!(target:"p2p","dht get record finished: no record found, cache candidates: {:?}", cache_candidates);
                    }
                },
                QueryResult::Bootstrap(result) => {
                    info!(target:"p2p","dht bootstrap result: {:?}", result);
                }
                QueryResult::GetClosestPeers(result) => {
                    info!(target:"p2p","dht get closest peers result: {:?}", result);
                }
                QueryResult::GetProviders(result) => match result {
                    GetProvidersResult::Ok(get_provider_ok) => match get_provider_ok {
                        GetProvidersOk::FoundProviders { key, providers } => {
                            info!(target:"p2p","dht get providers result, found providers: {:?}", providers);
                        }
                        GetProvidersOk::FinishedWithNoAdditionalRecord { closest_peers } => {
                            info!(target:"p2p","dht get providers result, closest peers: {:?}", closest_peers);
                        }
                    },
                    GetProvidersResult::Err(e) => {
                        info!(target:"p2p","dht get providers result: {:?}", e);
                    }
                },
                QueryResult::StartProviding(result) => {
                    info!(target:"p2p","dht start providing result: {:?}", result);
                }
                QueryResult::RepublishProvider(result) => {
                    info!(target:"p2p","dht republish provider result: {:?}", result);
                }
                QueryResult::RepublishRecord(result) => {
                    info!(target:"p2p","dht republish record result: {:?}", result);
                }
                QueryResult::PutRecord(result) => {
                    info!(target:"p2p","dht put record result: {:?}", result);
                }
            },
            DhtEvent::PendingRoutablePeer { peer, address } => {
                debug!(target: "p2p","🔄 DHT routing update: peer {} at {}", peer, address)
            }
            DhtEvent::RoutablePeer { peer, address } => {
                info!(target: "p2p","dht routable peer: {peer:?} {address:?}")
            }
            DhtEvent::UnroutablePeer { peer } => {
                info!(target: "p2p","dht unroutable peer: {peer:?}")
            }
            DhtEvent::RoutingUpdated {
                peer,
                is_new_peer,
                addresses,
                ..
            } => {
                info!(target: "p2p","dht routing updated: {peer:?} {is_new_peer:?} {addresses:?}")
            }
            DhtEvent::ModeChanged { new_mode } => {
                info!(target: "p2p","dht mode changed: {new_mode:?}")
            }
        }
    }

    async fn handle_relay_events(
        &self,
        relay_client_event: RelayClientEvent,
        connection_state: Rc<RefCell<ConnectionState>>,
    ) {
        match relay_client_event {
            RelayClientEvent::InboundCircuitEstablished { src_peer_id, .. } => {

                info!(target: "p2p","inbound circuit established: {src_peer_id:?}");
                let now = (js_sys::Date::now() / 1000.0) as u64; // Convert from ms to seconds
                *connection_state.borrow_mut() = ConnectionState::Connected(now);
            }
            RelayClientEvent::OutboundCircuitEstablished { relay_peer_id, .. } => {

                info!(target: "p2p","outbound circuit established: {relay_peer_id:?}");
                let now = (js_sys::Date::now() / 1000.0) as u64; // Convert from ms to seconds
                *connection_state.borrow_mut() = ConnectionState::Connected(now);       

                self.p2p_event_notif_sub_system
                    .sender
                    .borrow_mut()
                    .send(P2pEventResult::SenderCircuitEstablished)
                    .await
                    .expect("failed to send p2p event result");

            }
            RelayClientEvent::ReservationReqAccepted { relay_peer_id, .. } => {
                info!(target: "p2p","reservation request accepted: {relay_peer_id:?}");
                let now = (js_sys::Date::now() / 1000.0) as u64; // Convert from ms to seconds
                *connection_state.borrow_mut() = ConnectionState::Connected(now);

                self.p2p_event_notif_sub_system
                    .sender
                    .borrow_mut()
                    .send(P2pEventResult::ReservationAccepted)
                    .await
                    .expect("failed to send p2p event result");
            }
        }
    }

    async fn handle_app_json_events(
        &self,
        app_json_event: Event<TxStateMachine, Result<TxStateMachine, String>>,
        sender: Rc<
            RefCell<tokio_with_wasm::alias::sync::mpsc::Sender<Result<SwarmMessage, Error>>>,
        >,
    ) {
        match app_json_event {
            Event::Message { message, .. } => {
                info!(target: "p2p","received message: {message:?}");

                // update pending request for requests messages
                match message {
                    Message::Request {
                        channel,
                        request_id,
                        request,
                    } => {
                        let req_msg = SwarmMessage::WasmRequest {
                            data: request,
                            inbound_id: request_id,
                        };

                        let req_id_hash = request_id.get_hash_id();
                        info!(target: "p2p","stored response channel, with key: {req_id_hash}");
                        self.wasm_pending_request
                            .borrow_mut()
                            .insert(req_id_hash, channel);

                        if let Err(e) = sender.borrow_mut().send(Ok(req_msg)).await {
                            error!("Failed to send message: {}", e);
                        }
                        info!(target: "p2p","propagating txn request msg to main service worker");
                    }
                    Message::Response {
                        response,
                        request_id,
                    } => {
                        let data = response;
                        if let Ok(data) = data {
                            let resp_msg = SwarmMessage::WasmResponse {
                                data,
                                outbound_id: request_id,
                            };
                            if let Err(e) = sender.borrow_mut().send(Ok(resp_msg)).await {
                                error!("Failed to send message: {}", e);
                            }
                            info!(target: "p2p","propagating txn response msg to main service worker");
                        } else {
                            error!("failed to get response data: {data:?}");
                        }
                    }
                }
            }
            Event::OutboundFailure {
                error,
                peer,
                request_id,
                ..
            } => {
                let req_id_hash = request_id.get_hash_id();

                error!(target:"p2p","outbound error: {error:?} peerId: {peer}  request id: {req_id_hash}")
            }
            Event::InboundFailure {
                error, request_id, ..
            } => {
                let req_id_hash = request_id.get_hash_id();
                error!("inbound error: {error} on req_id: {req_id_hash}")
            }
            Event::ResponseSent {
                peer, request_id, ..
            } => {
                let req_id_hash = request_id.get_hash_id();
                info!(target: "p2p","response sent to: {peer:?}: req_id: {req_id_hash}")
            }
        }
    }

    pub async fn start_swarm(
        &self,
        sender_channel: Rc<
            RefCell<tokio_with_wasm::alias::sync::mpsc::Sender<Result<SwarmMessage, Error>>>,
        >,
    ) -> Result<(), Error> {
        self.wasm_swarm
            .borrow_mut()
            .dial(self.relay_multi_addr.clone())
            .map_err(|e| anyhow!("failed to dial relay node: {e}"))?;
        info!(target:"p2p","relay node dialed: {:?}",self.relay_multi_addr);

        TimeoutFuture::new(100).await;

        self.wasm_swarm
            .borrow_mut()
            .listen_on(self.user_circuit_multi_addr.clone())?;

        let full_circuit_addr = self
            .user_circuit_multi_addr
            .clone()
            .with(libp2p::multiaddr::Protocol::P2p(self.node_id.clone()));

        self.wasm_swarm
            .borrow_mut()
            .add_external_address(full_circuit_addr.clone());
        info!(target:"p2p","Added external circuit address: {:?}", full_circuit_addr);

        for _ in 0..100 {
            TimeoutFuture::new(100).await;
        }

        let sender = sender_channel.clone();
        let swarm = self.wasm_swarm.clone();
        let p2p_command_recv = self.wasm_p2p_command_recv.clone();
        let pending_request = self.wasm_pending_request.clone();
        let self_clone = self.clone();

        // Event-driven approach - no polling needed!
        wasm_bindgen_futures::spawn_local(async move {
            loop {
                let mut swarm = swarm.borrow_mut();
                let mut p2p_command_recv = p2p_command_recv.borrow_mut();

                futures::select! {
                    event = swarm.next().fuse() => {
                        if let Some(event) = event {
                            self_clone.handle_swarm_events(pending_request.clone(), event, sender.clone()).await
                        }
                    },
                    cmd = p2p_command_recv.recv().fuse() => {
                        match cmd {
                            Some(NetworkCommand::WasmSendResponse {response,channel}) => {
                                if channel.is_open() {
                                    swarm.behaviour_mut().app_json.send_response(channel,response)
                                        .map_err(|err|anyhow!("failed to send response; {err:?}"));
                                } else {
                                    error!("response channel is closed");
                                }
                            },
                            Some(NetworkCommand::WasmSendRequest {request,peer_id,target_multi_addr}) => {
                                if swarm.is_connected(&peer_id) {
                                    swarm.behaviour_mut().app_json.send_request(&peer_id,request);
                                    info!("request sent to peer: {peer_id:?}");
                                } else {
                                    info!("re-dialing as the peer is not connected: {peer_id:?}");
                                    swarm.dial(target_multi_addr).map_err(|err|anyhow!("failed to re dial; {err:?}"));
                                    swarm.behaviour_mut().app_json.send_request(&peer_id,request);
                                    info!("request sent to peer: {peer_id:?}");
                                }
                            },
                            Some(NetworkCommand::Dial {target_multi_addr,target_peer_id,oneshot_sender}) => {
                                // check first if the peer communication is already connected
                                if swarm.is_connected(&target_peer_id){
                                    info!("peer already connected: {target_peer_id}");
                                    if let Err(e) = oneshot_sender.send(Ok(())) {
                                        error!("failed to send oneshot sender: {:?}", e);
                                    }
                                }else{
                                    info!("dialing peer: {target_peer_id} ");
                                    match swarm.dial(target_multi_addr) {
                                        Ok(()) => {
                                            info!("peer dialed successfully: {target_peer_id}");
                                            if let Err(e) = oneshot_sender.send(Ok(())) {
                                                error!("failed to send oneshot sender: {:?}", e);
                                            }
                                        }
                                        Err(e) => {
                                            error!("failed to dial: {e}");
                                            if let Err(e) = oneshot_sender.send(Err(anyhow!("failed to dial: {e}"))) {
                                                error!("failed to send oneshot sender: {:?}", e);
                                            }
                                        }
                                    }
                                }
                            },
                            Some(NetworkCommand::Close {peer_id}) => {
                                swarm.disconnect_peer_id(peer_id).map_err(|err|anyhow!("failed to disconnect peer: {err:?}"));
                            },
                            Some(NetworkCommand::AddDhtAccount {account_id,value}) => {
                                let acc = account_id.clone();
                                let p2p_event_notif = self_clone.p2p_event_notif_sub_system.clone();
                                
                                wasm_bindgen_futures::spawn_local(async move {
                                    match host_set_dht(acc.clone(), value).await {
                                        Ok(response) => {
                                            if let Some(error_msg) = &response.error {
                                                error!(target: "p2p","Failed to add account record to DHT: error_msg={}", error_msg);
                                                p2p_event_notif
                                                    .sender
                                                    .borrow_mut()
                                                    .send(P2pEventResult::AccountAdditionFailed {
                                                        account_id: acc.clone(),
                                                    })
                                                    .await
                                                    .expect("failed to send p2p event result");
                                            } else {
                                                info!(target: "p2p","Added account record to DHT successfully: response={:?}", response);
                                                p2p_event_notif
                                                    .sender
                                                    .borrow_mut()
                                                    .send(P2pEventResult::AccountAddedSuccessfully {
                                                        account_id: acc.clone(),
                                                    })
                                                    .await
                                                    .expect("failed to send p2p event result");
                                            }
                                        }
                                        Err(e) => {
                                            error!(target: "p2p","Failed to add account record to DHT: error={:?}", e);
                                            p2p_event_notif
                                                .sender
                                                .borrow_mut()
                                                .send(P2pEventResult::AccountAdditionFailed {
                                                    account_id: acc.clone(),
                                                })
                                                .await
                                                .expect("failed to send p2p event result");
                                        }
                                    }
                                });

                            },
                            Some(NetworkCommand::GetDhtPeer {target_acc_id,response_sender}) => {
                                info!(target: "p2p", "Getting DHT peer: target_acc_id={:?}", target_acc_id);
                                // let key:libp2p::kad::RecordKey = target_acc_id.as_bytes().to_vec().into();
                                // info!(target: "p2p", "Getting DHT peer: key={:?}", &key);
                                // let resp = swarm.behaviour_mut().app_client_dht.get_record(key.clone());
                                // let query_id = swarm.behaviour_mut().app_client_dht.get_providers(key);
                                // response_sender.send(Ok(resp)).map_err(|err|anyhow!("failed to send response; {err:?}"));
                                let dht_channel_query = self_clone.dht_channel_query.clone();

                                wasm_bindgen_futures::spawn_local(async move {
                                    match host_get_dht(target_acc_id).await {
                                        Ok(response) => {
                                            // Check if DHT operation failed (error field is populated)
                                            if let Some(error_msg) = &response.error {
                                                // DHT operation failed, but the call itself succeeded
                                                let err = anyhow!("DHT operation failed: {}", error_msg);
                                                response_sender.send(Err(err)).expect("failed to send response");
                                                // retrying logic point,
                                                dht_channel_query
                                                    .send((None, response.random))
                                                    .await
                                                    .expect("failed to send dht query result on the channel");
                                            } else {
                                                // DHT operation succeeded, process the value
                                                let maybe_addr = response.value.and_then(|v| {
                                                    if v.is_empty() {
                                                        None
                                                    } else {
                                                        Multiaddr::try_from(v).ok()
                                                    }
                                                });
                                                info!(target: "p2p", "returned multiaddr: {:?}", maybe_addr);
                                                response_sender.send(Ok(response.random)).expect("failed to send response");

                                                dht_channel_query
                                                    .send((maybe_addr, response.random))
                                                    .await
                                                    .expect("failed to send dht query result on the channel");
                                            }
                                        }
                                        Err(e) => {
                                            // get_dht call itself failed
                                            let err = anyhow!("failed to get DHT peer, caused by internal error: {e:?}");
                                            response_sender.send(Err(err)).map_err(|err| anyhow!("failed to send response; {err:?}"));
                                            // Note: We don't send to dht_channel_query here since we don't have a response.random
                                        }
                                    }
                                });

                            }
                            _ => {}
                        }
                    }
                }
            }
        });
        Ok(())
    }
}

#[derive(Clone)]
pub struct P2pNetworkService {
    pub p2p_command_tx: Rc<tokio_with_wasm::alias::sync::mpsc::Sender<NetworkCommand>>,
    pub wasm_p2p_worker: WasmP2pWorker,
}

impl P2pNetworkService {
    pub fn new(
        p2p_command_tx: Rc<tokio_with_wasm::alias::sync::mpsc::Sender<NetworkCommand>>,
        p2p_worker: WasmP2pWorker,
    ) -> Result<Self, Error> {
        Ok(Self {
            p2p_command_tx,
            wasm_p2p_worker: p2p_worker,
        })
    }

    // dialing the target peer_id
    pub async fn dial_to_peer_id(
        &mut self,
        target_url: Multiaddr,
        peer_id: &PeerId,
    ) -> Result<(), anyhow::Error> {
        let (response_sender, response_receiver) =
            tokio_with_wasm::alias::sync::oneshot::channel::<Result<(), anyhow::Error>>();

        let dial_command = NetworkCommand::Dial {
            target_multi_addr: target_url.clone(),
            target_peer_id: peer_id.clone(),
            oneshot_sender: response_sender,
        };

        self.p2p_command_tx
            .send(dial_command)
            .await
            .map_err(|err| anyhow!("failed to send dial command; {err}"))?;

        // Wait for response with 10 second timeout
        let dht = response_receiver.fuse();
        let timeout = TimeoutFuture::new(10_000).fuse(); // 10 seconds in milliseconds
        futures::pin_mut!(dht, timeout);

        match futures::future::select(dht, timeout).await {
            futures::future::Either::Left((Ok(result), _)) => result,
            futures::future::Either::Left((Err(e), _)) => {
                Err(anyhow!("PeerId dialing channel error: {}", e))
            }
            futures::future::Either::Right((_elapsed, _)) => Err(anyhow!(
                "PeerId dialing timeout: peer {} did not respond within 10 seconds",
                peer_id
            )),
        }
    }

    // close the connection to the peer_id
    pub async fn disconnect_from_peer_id(&mut self, peer_id: &PeerId) -> Result<(), anyhow::Error> {
        let close_command = NetworkCommand::Close {
            peer_id: peer_id.clone(),
        };

        self.p2p_command_tx
            .send(close_command)
            .await
            .map_err(|err| anyhow!("failed to send close command; {err}"))?;
        Ok(())
    }

    pub async fn wasm_send_request(
        &mut self,
        request: Rc<RefCell<TxStateMachine>>,
        target_peer_id: PeerId,
        target_multi_addr: Multiaddr,
    ) -> Result<(), Error> {
        let req = request.borrow().clone();

        // cache the receiver address
        self.wasm_p2p_worker.receiver_address_cache
            .borrow_mut()
            .insert(target_peer_id, req.receiver_address.clone());

        let req_command = NetworkCommand::WasmSendRequest {
            request: req,
            peer_id: target_peer_id,
            target_multi_addr,
        };

        self.p2p_command_tx
            .send(req_command)
            .await
            .map_err(|err| anyhow!("failed to send req command; {err}"))?;
        trace!(target: "p2p","\n sending request command to the swarm thread ");

        Ok(())
    }

    pub async fn wasm_send_response(
        &mut self,
        response: Rc<RefCell<TxStateMachine>>,
    ) -> Result<(), anyhow::Error> {
        let txn_state = response.borrow().clone();

        // error handler worker for system should handle this
        let outbound_id = txn_state.inbound_req_id.expect("no outbound req id found");

        let channel = self
            .clone()
            .wasm_p2p_worker
            .wasm_pending_request
            .borrow_mut()
            .remove(&outbound_id)
            .ok_or(anyhow!("failed to get response channel"))?;

        let resp_command = NetworkCommand::WasmSendResponse {
            response: Ok(txn_state),
            channel,
        };
        self.p2p_command_tx.send(resp_command).await?;
        trace!(target: "p2p","sending response command");

        Ok(())
    }

    pub async fn add_account_to_dht(&self, account_id: String, value: String) -> Result<(), Error> {
        let add_dht_account_command = NetworkCommand::AddDhtAccount { account_id, value };
        self.p2p_command_tx.send(add_dht_account_command).await?;
        Ok(())
    }

    pub async fn get_dht_target_peer(&self, target_acc_id: String) -> Result<u32, Error> {
        info!(target: "p2p", "IT GOT FIRED GET DHT TARGET PEER");
        let (response_sender, mut response_receiver) =
            tokio_with_wasm::alias::sync::oneshot::channel::<Result<u32, Error>>();

        let get_dht_peer_command = NetworkCommand::GetDhtPeer {
            target_acc_id,
            response_sender,
        };

        self.p2p_command_tx
            .send(get_dht_peer_command)
            .await
            .map_err(|err| anyhow!("failed to send get dht peer command; {err}"))?;

        for _ in 0..100 {
            match response_receiver.try_recv() {
                Ok(result) => {
                    info!(target: "p2p", "result from get dht peer: {:?}", result);
                    return result;
                }
                Err(tokio_with_wasm::alias::sync::oneshot::error::TryRecvError::Empty) => {
                    TimeoutFuture::new(600).await;
                }
                Err(tokio_with_wasm::alias::sync::oneshot::error::TryRecvError::Closed) => {
                    return Err(anyhow!("Response channel was closed unexpectedly"));
                }
            }
        }
        Err(anyhow!("DHT query timeout"))
    }
}

// DHT Host Functions
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["hostFunctions", "hostDHT"], js_name = set)]
    fn set_dht_host(key: String, value: String) -> js_sys::Promise;

    #[wasm_bindgen(js_namespace = ["hostFunctions", "hostDHT"], js_name = get)]
    fn get_dht_host(key: String) -> js_sys::Promise;
}

pub async fn host_set_dht(key: String, value: String) -> Result<DHTResponse, anyhow::Error> {
    let promise = unsafe { set_dht_host(key, value) };
    let jsv = wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .map_err(|e| anyhow::anyhow!("Host function error: {:?}", e))?;
    let response: DHTResponse = serde_wasm_bindgen::from_value(jsv)
        .map_err(|e| anyhow::anyhow!("Deserialization error: {:?}", e))?;
    info!(target: "p2p", "response from host set dht: {:?}", response);
    Ok(response)
}

pub async fn host_get_dht(key: String) -> Result<DHTResponse, anyhow::Error> {
    let promise = unsafe { get_dht_host(key) };
    debug!(target: "p2p", "🔍 Got promise from get_dht_host");
    let jsv = wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .map_err(|e| anyhow::anyhow!("Host function error: {:?}", e))?;
    debug!(target: "p2p", "🔍 Got JSValue from promise: {:?}", jsv);

    let response: DHTResponse = serde_wasm_bindgen::from_value(jsv)
        .map_err(|e| anyhow::anyhow!("Deserialization error: {:?}", e))?;
    info!(target: "p2p", "response from host get dht: {:?}", response);
    Ok(response)
}
