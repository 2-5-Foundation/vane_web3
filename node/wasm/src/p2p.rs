use anyhow::{anyhow, Error};
pub use codec::Encode;
use core::pin::Pin;
use core::str::FromStr;
use log::{debug, error, info, trace};

// peer discovery
// app to app communication (i.e sending the tx to be verified by the receiver) and back

use primitives::data_structure::DbWorkerInterface;

use primitives::data_structure::{
    AccountInfo, ChainSupported, HashId, NetworkCommand, PeerRecord, RedisAccountProfile,
    SwarmMessage, TxStateMachine,
};

use sp_core::H256;

// ---------------------- libp2p common --------------------- //
use libp2p::futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, Stream};
use libp2p::request_response::{Behaviour, Event, InboundRequestId, Message, OutboundRequestId};
use libp2p::request_response::{Codec, ProtocolSupport, ResponseChannel};
use libp2p::swarm::SwarmEvent;
use libp2p::{Multiaddr, PeerId, StreamProtocol, Swarm, SwarmBuilder};

use p2p_wasm_imports::*;

mod p2p_wasm_imports {
    pub use alloc::rc::Rc;
    pub use core::cell::RefCell;
    pub use db_wasm::OpfsRedbWorker;
    pub use futures::StreamExt;
    pub use libp2p::request_response::json::Behaviour as JsonBehaviour;
    pub use libp2p::core::Transport as TransportTrait;
    pub use libp2p::core::transport::global_only::Transport;
    pub use libp2p::webtransport_websys as webrtc_websys;

    pub use wasm_bindgen_futures::wasm_bindgen::closure::Closure;
    pub use web_sys::wasm_bindgen::JsCast;
    pub use heapless::FnvIndexMap;
    pub use alloc::collections::VecDeque;
    pub use alloc::string::String;
    pub use alloc::format;
    pub use alloc::boxed::Box;
}

#[derive(Clone)]
pub struct WasmP2pWorker {
    pub node_id: PeerId,
    pub wasm_swarm: Rc<RefCell<Swarm<JsonBehaviour<TxStateMachine, Result<TxStateMachine,String>>>>>,
    pub url: Multiaddr,
    pub wasm_p2p_command_recv: Rc<RefCell<tokio_with_wasm::alias::sync::mpsc::Receiver<NetworkCommand>>>,
    pub wasm_pending_request: Rc<RefCell<FnvIndexMap<u64, ResponseChannel<Result<TxStateMachine, String>>,16>>>,
    pub current_req: VecDeque<SwarmMessage>,
}

impl WasmP2pWorker {
    pub async fn new(
        db_worker: Rc<OpfsRedbWorker>,
        port: u16,
        dns: String,
        command_recv_channel: tokio_with_wasm::alias::sync::mpsc::Receiver<NetworkCommand>,
    ) -> Result<Self, anyhow::Error> {
        let self_peer_id = libp2p::identity::Keypair::generate_ed25519();
        let peer_id = self_peer_id.public().to_peer_id().to_base58();
       
        // TODO:

        let p2p_url = format!("/ip6/{}/tcp/{}/p2p/{}", dns, port, peer_id);
        info!("listening to p2p url: {p2p_url}");
        

        let multi_addr: Multiaddr = p2p_url
            .parse()
            .map_err(|err| anyhow!("failed to parse multi addr, caused by: {err}"))?;

        let peer_id: PeerId = PeerId::from_str(&peer_id)
            .map_err(|err| anyhow!("failed to convert PeerId, caused by: {err}"))?;


        let request_response_config = libp2p::request_response::Config::default()
            .with_request_timeout(core::time::Duration::from_secs(600)); // 10 minutes waiting time for a response

        let json_behaviour = JsonBehaviour::new(
            [(
                StreamProtocol::new("/my-json-protocol"),
                ProtocolSupport::Full,
            )],
            request_response_config,
        );

        let wasm_swarm = SwarmBuilder::with_existing_identity(self_peer_id)
            .with_wasm_bindgen()
            .with_other_transport(|key| {
                webrtc_websys::Transport::new(webrtc_websys::Config::new(&key))
            })?
            .with_behaviour(|_| json_behaviour)?
            
            .build();

        Ok(Self {
            node_id: peer_id,
            wasm_p2p_command_recv: Rc::new(RefCell::new(command_recv_channel)),
            current_req: Default::default(),
            wasm_swarm: Rc::new(RefCell::new(wasm_swarm)),
            wasm_pending_request: Rc::new(RefCell::new(Default::default())),
            url: multi_addr,
        })
    }

    pub async fn handle_swarm_events(
        pending_request: Rc<RefCell<FnvIndexMap<u64, ResponseChannel<Result<TxStateMachine, String>>,16>>>,
        events: SwarmEvent<Event<TxStateMachine, Result<TxStateMachine, String>>>,
        sender: Rc<RefCell<tokio_with_wasm::alias::sync::mpsc::Sender<Result<SwarmMessage, Error>>>>,
    ) {
        match events {
            SwarmEvent::Behaviour(behaviour_event) => match behaviour_event {
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
                            pending_request.borrow_mut().insert(req_id_hash, channel);

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
                            }else{
                                error!("failed to get response data: {data:?}");
                            }
                            if let Err(e) = sender.borrow_mut().send(Ok(resp_msg)).await {
                                error!("Failed to send message: {}", e);
                            }
                            info!(target: "p2p","propagating txn response msg to main service worker");
                        }
                    }
                }
                Event::OutboundFailure {
                    error,
                    peer,
                    request_id,
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
                Event::ResponseSent { peer, request_id } => {
                    let req_id_hash = request_id.get_hash_id();
                    info!(target: "p2p","response sent to: {peer:?}: req_id: {req_id_hash}")
                }
            },
            SwarmEvent::ConnectionEstablished {
                peer_id,
                endpoint,
                num_established,
                ..
            } => {
                info!(target:"p2p","connection established: peer_id:{peer_id:?} endpoint:{endpoint:?} num_established:{num_established:?}")
            }
            SwarmEvent::IncomingConnection {
                local_addr,
                send_back_addr,
                ..
            } => {
                info!(target:"p2p","incoming connection: local_addr:{local_addr:?} send_back_addr:{send_back_addr:?}")
            }
            SwarmEvent::Dialing { peer_id, .. } => {
                info!(target:"p2p","dialing to {peer_id:?}")
            }
            SwarmEvent::ListenerError { error, .. } => {
                error!(target: "p2p","listener error: {error}");
            }
            SwarmEvent::ConnectionClosed {
                peer_id,
                endpoint,
                cause,
                ..
            } => {
                info!(target:"p2p","connection closed peer_id:{peer_id:?} endpoint:{endpoint:?} cause:{cause:?}")
            }
            SwarmEvent::IncomingConnectionError { error, .. } => {
                error!(target:"p2p","incoming connection error: {error:?}")
            }
            SwarmEvent::OutgoingConnectionError { error, .. } => {
                error!(target:"p2p","outgoing connection error: {error:?}")
            }
            SwarmEvent::ListenerClosed { reason, .. } => info!("listener closed: {reason:?}"),
            SwarmEvent::NewListenAddr { address, .. } => {
                info!(target:"p2p","new listener address: {address:?}")
            }
            SwarmEvent::ExpiredListenAddr { address, .. } => {
                info!(target:"p2p","expired listener add: {address:?}")
            }
            SwarmEvent::NewExternalAddrCandidate { address, .. } => {
                info!(target:"p2p","new external addr candidate: {address:?}")
            }
            _ => info!(target:"p2p","unhandled event"),
        }
    }

    pub async fn start_swarm(
        &mut self,
        sender_channel: Rc<
            RefCell<tokio_with_wasm::alias::sync::mpsc::Sender<Result<SwarmMessage, Error>>>,
        >,
    ) -> Result<(), Error> {
        let multi_addr = &self.url;
        let _listening_id = self.wasm_swarm.borrow_mut().listen_on(multi_addr.clone())?;
        trace!(target:"p2p","listening to: {:?}",multi_addr);

        let sender = sender_channel.clone();
        let swarm = self.wasm_swarm.clone();
        let p2p_command_recv = self.wasm_p2p_command_recv.clone();
        let pending_request = self.wasm_pending_request.clone();

        let callback = Closure::wrap(Box::new(move || {
            let swarm = Rc::clone(&swarm);
            let p2p_command_recv = Rc::clone(&p2p_command_recv);
            let pending_request = Rc::clone(&pending_request);
            let sender = Rc::clone(&sender);

            wasm_bindgen_futures::spawn_local(async move {
                let mut swarm = swarm.borrow_mut();
                let mut p2p_command_recv = p2p_command_recv.borrow_mut();

                tokio_with_wasm::alias::select! {
                    event = swarm.next() => {
                        if let Some(event) = event {
                            Self::handle_swarm_events(pending_request, event, sender).await
                        } else {
                            info!("no current swarm event")
                        }
                    },
                    cmd = p2p_command_recv.recv() => {
                        match cmd {
                            Some(NetworkCommand::WasmSendResponse {response,channel}) => {
                                if channel.is_open() {
                                    swarm.behaviour_mut().send_response(channel,response)
                                        .expect("failed to send response");
                                } else {
                                    error!("response channel is closed");
                                }
                            },
                            Some(NetworkCommand::WasmSendRequest {request,peer_id,target_multi_addr}) => {
                                if swarm.is_connected(&peer_id) {
                                    swarm.behaviour_mut().send_request(&peer_id,request);
                                    info!("request sent to peer: {peer_id:?}");
                                } else {
                                    info!("re dialing");
                                    swarm.dial(target_multi_addr).expect("failed to re dial");
                                    swarm.behaviour_mut().send_request(&peer_id,request);
                                    info!("request sent to peer: {peer_id:?}");
                                }
                            },
                            Some(NetworkCommand::Dial {target_multi_addr,target_peer_id}) => {
                                // check first if the peer communication is already connected
                                if swarm.is_connected(&target_peer_id){
                                    info!("peer already connected: {target_peer_id}")
                                }else{
                                    info!("dialing peer: {target_peer_id} ");
                                    swarm.dial(target_multi_addr).expect("failed to dial");
                                }
                            },
                            None => {
                                info!("command channel closed");
                            },
                            _ => {}
                        }
                    }
                }
            })
        }) as Box<dyn FnMut()>);

        web_sys::window()
            .expect("windows not found")
            .set_interval_with_callback_and_timeout_and_arguments_0(
                callback.as_ref().unchecked_ref(),
                100, // 100 ms interval
            )
            .map_err(|err| anyhow!("failed to set windows interval: {err:?}"))?;

        callback.forget();
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
        let dial_command = NetworkCommand::Dial {
            target_multi_addr: target_url.clone(),
            target_peer_id: peer_id.clone(),
        };

        self.p2p_command_tx
            .send(dial_command)
            .await
            .map_err(|err| anyhow!("failed to send dial command; {err}"))?;

        Ok(())
    }

    // close the connection to the peer_id
    pub async fn disconnect_from_peer_id(&mut self, peer_id: &PeerId) -> Result<(), anyhow::Error> {
        let close_command = NetworkCommand::Close {
            peer_id: peer_id.clone(),
        };

        self.p2p_command_tx.send(close_command);
        Ok(())
    }

   
    pub async fn wasm_send_request(
        &mut self,
        request: Rc<RefCell<TxStateMachine>>,
        target_peer_id: PeerId,
        target_multi_addr: Multiaddr,
    ) -> Result<(), Error> {
        let req = request.borrow().clone();
        let req_command = NetworkCommand::WasmSendRequest {
            request: req,
            peer_id: target_peer_id,
            target_multi_addr,
        };

        self.p2p_command_tx
            .send(req_command)
            .await
            .map_err(|err| anyhow!("failed to send req command; {err}"))?;
        trace!(target: "p2p","\nsending request command to the swarm thread ");
        Ok(())
    }


    pub async fn wasm_send_response(
        &mut self,
        outbound_id: u64,
        response: Rc<RefCell<TxStateMachine>>,
    ) -> Result<(), anyhow::Error> {
        let txn_state = response.borrow().clone();

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
}

