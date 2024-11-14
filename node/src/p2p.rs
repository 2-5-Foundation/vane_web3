use anyhow::{anyhow, Error};
use core::pin::Pin;
use core::str::FromStr;
use log::{debug, error, info, trace};
use std::collections::{HashMap, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io;
use std::sync::Arc;
use std::time::Duration;
// peer discovery
// app to app communication (i.e sending the tx to be verified by the receiver) and back
use crate::rpc::Airtable;
use codec::Encode;
use db::DbWorker;
use libp2p::futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, Stream};
use libp2p::request_response::{Behaviour, Event, InboundRequestId, Message, OutboundRequestId};
use libp2p::request_response::{Codec, ProtocolSupport, ResponseChannel};
use libp2p::swarm::SwarmEvent;
use libp2p::{Multiaddr, PeerId, Swarm, SwarmBuilder};
use local_ip_address::local_ip;
use primitives::data_structure::{
    new_tx_state_from_mutex, AirtableRequestBody, Fields, PeerRecord,
};
use primitives::data_structure::{NetworkCommand, SwarmMessage, TxStateMachine};
use sp_core::H256;
use tokio::select;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::{Mutex, MutexGuard};
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;

pub type BoxStream<I> = Pin<Box<dyn Stream<Item = Result<I, anyhow::Error>>>>;

#[derive(Debug, Clone)]
#[doc(hidden)] // Needs to be public in order to satisfy the Rust compiler.
pub struct GenericCodec {
    max_request_size: u64,
    max_response_size: u64,
}

impl Default for GenericCodec {
    fn default() -> Self {
        Self {
            max_request_size: u64::MAX,
            max_response_size: u64::MAX,
        }
    }
}

#[async_trait::async_trait]
impl Codec for GenericCodec {
    type Protocol = &'static str;
    type Request = Vec<u8>;
    type Response = Result<Vec<u8>, anyhow::Error>;

    async fn read_request<T>(
        &mut self,
        _: &Self::Protocol,
        mut io: &mut T,
    ) -> io::Result<Self::Request>
    where
        T: AsyncRead + Unpin + Send,
    {
        // Read the length.
        let length = unsigned_varint::aio::read_usize(&mut io)
            .await
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?;
        if length > usize::try_from(self.max_request_size).unwrap_or(usize::MAX) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "Request size exceeds limit: {} > {}",
                    length, self.max_request_size
                ),
            ));
        }

        // Read the payload.
        let mut buffer = vec![0; length];
        io.read_exact(&mut buffer).await?;
        Ok(buffer)
    }

    async fn read_response<T>(
        &mut self,
        _: &Self::Protocol,
        mut io: &mut T,
    ) -> io::Result<Self::Response>
    where
        T: AsyncRead + Unpin + Send,
    {
        // Note that this function returns a `Result<Result<...>>`. Returning an `Err` is
        // considered as a protocol error and will result in the entire connection being closed.
        // Returning `Ok(Err(_))` signifies that a response has successfully been fetched, and
        // that this response is an error.

        // Read the length.
        let length = match unsigned_varint::aio::read_usize(&mut io).await {
            Ok(l) => l,
            Err(unsigned_varint::io::ReadError::Io(err))
                if matches!(err.kind(), io::ErrorKind::UnexpectedEof) =>
            {
                return Ok(Err(anyhow!("failed to get response length")))
            }
            Err(err) => return Err(io::Error::new(io::ErrorKind::InvalidInput, err)),
        };

        if length > usize::try_from(self.max_response_size).unwrap_or(usize::MAX) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "Response size exceeds limit: {} > {}",
                    length, self.max_response_size
                ),
            ));
        }

        // Read the payload.
        let mut buffer = vec![0; length];
        io.read_exact(&mut buffer).await?;
        Ok(Ok(buffer))
    }

    async fn write_request<T>(
        &mut self,
        _: &Self::Protocol,
        io: &mut T,
        req: Self::Request,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        // TODO: check the length?
        // Write the length.
        {
            let mut buffer = unsigned_varint::encode::usize_buffer();
            io.write_all(unsigned_varint::encode::usize(req.len(), &mut buffer))
                .await?;
        }

        // Write the payload.
        io.write_all(&req).await?;

        io.close().await?;
        Ok(())
    }

    async fn write_response<T>(
        &mut self,
        _: &Self::Protocol,
        io: &mut T,
        res: Self::Response,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        // If `res` is an `Err`, we jump to closing the substream without writing anything on it.
        if let Ok(res) = res {
            // TODO: check the length?
            // Write the length.
            {
                let mut buffer = unsigned_varint::encode::usize_buffer();
                io.write_all(unsigned_varint::encode::usize(res.len(), &mut buffer))
                    .await?;
            }

            // Write the payload.
            io.write_all(&res).await?;
        }

        io.close().await?;
        Ok(())
    }
}

/// handling connection with other peer ( recipients ) of txs
/// and tx passing to receivers and senders

type BlockStream<T> = Pin<Box<dyn Stream<Item = Result<T, anyhow::Error>> + Send>>;
type BlockStreamRes<T> = Result<BlockStream<T>, anyhow::Error>;

#[derive(Clone)]
pub struct P2pWorker {
    pub node_id: PeerId,
    pub swarm: Arc<Mutex<Swarm<Behaviour<GenericCodec>>>>,
    pub url: Multiaddr,
    // for receiving network commands
    pub p2p_command_recv: Arc<Mutex<Receiver<NetworkCommand>>>,
    // for pending requests to be replied, along with the response channel <InboundRequestId, Channel>
    pub pending_request: Arc<Mutex<HashMap<u64, ResponseChannel<Result<Vec<u8>, Error>>>>>,
    // for storing current ongoing request data
    pub current_req: VecDeque<SwarmMessage>,
}

impl P2pWorker {
    /// generate new ed25519 keypair for node identity and register the peer record in  the db
    pub async fn new(
        airtable_client: Arc<Mutex<Airtable>>,
        db_worker: Arc<Mutex<DbWorker>>,
        port: u16,
        command_recv_channel: Receiver<NetworkCommand>,
    ) -> Result<Self, Error> {
        let self_peer_id = libp2p::identity::Keypair::generate_ed25519();
        let peer_id = self_peer_id.public().to_peer_id().to_base58();
        let mut p2p_url = String::new();

        let local_ip = local_ip()
            .map_err(|err| anyhow!("failed to get local ip address; caused by: {err}"))?;

        if local_ip.is_ipv4() {
            p2p_url = format!(
                "/ip4/{}/tcp/{}/p2p/{}",
                local_ip.to_string(),
                port - 541,
                peer_id
            );
        } else {
            p2p_url = format!(
                "/ip6/{}/tcp/{}/p2p/{}",
                local_ip.to_string(),
                port - 541,
                peer_id
            );
        }

        info!("listening to p2p url: {p2p_url}");
        let mut user_peer_id = PeerRecord {
            record_id: "".to_string(),
            peer_id: Some(peer_id),
            account_id1: None,
            account_id2: None,
            account_id3: None,
            account_id4: None,
            multi_addr: Some(p2p_url),
            keypair: Some(
                self_peer_id
                    .to_protobuf_encoding()
                    .map_err(|_| anyhow!("failed to encode keypair"))?,
            ),
        };

        let field: Fields = user_peer_id.clone().into();
        let req_body = AirtableRequestBody::new(field);
        let record_data = airtable_client.lock().await.create_peer(req_body).await?;

        // store in the local db and airtable db
        user_peer_id.record_id = record_data.id;
        db_worker
            .lock()
            .await
            .record_user_peer_id(user_peer_id.clone())
            .await?;

        let url = user_peer_id.multi_addr.unwrap();
        let multi_addr: Multiaddr = url
            .parse()
            .map_err(|err| anyhow!("failed to parse multi addr, caused by: {err}"))?;

        let peer_id: PeerId = PeerId::from_str(&user_peer_id.peer_id.unwrap())
            .map_err(|err| anyhow!("failed to convert PeerId, caused by: {err}"))?;

        let secret_bytes = user_peer_id.keypair.ok_or(anyhow!("keyPair is not set"))?;
        let keypair = libp2p::identity::Keypair::from_protobuf_encoding(&secret_bytes[..])
            .map_err(|_| anyhow!("failed to decode keypair ed25519"))?;

        let request_response_config = libp2p::request_response::Config::default()
            .with_request_timeout(tokio::time::Duration::from_secs(600)); // 10 minutes waiting time for a response

        let behaviour = Behaviour::new(
            vec![("/vane-web3/1.0.0", ProtocolSupport::Full)].into_iter(),
            request_response_config,
        );

        let swarm = SwarmBuilder::with_existing_identity(keypair)
            .with_tokio()
            .with_tcp(
                libp2p::tcp::Config::default(),
                libp2p::tls::Config::new,
                libp2p::yamux::Config::default,
            )?
            .with_behaviour(|_| behaviour)?
            .with_swarm_config(|cfg| {
                cfg.with_idle_connection_timeout(tokio::time::Duration::from_secs(6000))
            })
            .build();

        Ok(Self {
            node_id: peer_id,
            swarm: Arc::new(Mutex::new(swarm)),
            url: multi_addr,
            p2p_command_recv: Arc::new(Mutex::new(command_recv_channel)),
            pending_request: Default::default(),
            current_req: Default::default(),
        })
    }

    pub async fn handle_swarm_events(
        pending_request: Arc<Mutex<HashMap<u64, ResponseChannel<Result<Vec<u8>, Error>>>>>,
        events: SwarmEvent<Event<Vec<u8>, Result<Vec<u8>, Error>>>,
        sender: Sender<Result<SwarmMessage, Error>>,
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
                            let req_msg = SwarmMessage::Request {
                                data: request,
                                inbound_id: request_id,
                            };

                            let mut req_id_hash = DefaultHasher::default();
                            request_id.hash(&mut req_id_hash);
                            let req_id_hash = req_id_hash.finish();

                            pending_request.lock().await.insert(req_id_hash, channel);

                            if let Err(e) = sender.send(Ok(req_msg)).await {
                                error!("Failed to send message: {}", e);
                            }
                            info!(target: "p2p","propagating txn request msg to main service worker");
                        }
                        Message::Response {
                            response,
                            request_id,
                        } => {
                            if let Ok(data) = response {
                                let resp_msg = SwarmMessage::Response {
                                    data,
                                    outbound_id: request_id,
                                };
                                if let Err(e) = sender.send(Ok(resp_msg)).await {
                                    error!("Failed to send message: {}", e);
                                }
                                info!(target: "p2p","propagating txn response msg to main service worker");
                            }
                        }
                    }
                }
                Event::OutboundFailure {
                    error,
                    peer,
                    request_id,
                } => {
                    error!(target:"p2p","outbound error: {error:?} peerId: {peer}  request id: {request_id}")
                }
                Event::InboundFailure { error, .. } => {
                    error!("inbound error: {error}")
                }
                Event::ResponseSent { .. } => {
                    info!(target: "p2p","response sent")
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
        sender_channel: Sender<Result<SwarmMessage, Error>>,
    ) -> Result<(), Error> {
        let multi_addr = &self.url;
        let _listening_id = self.swarm.lock().await.listen_on(multi_addr.clone())?;
        trace!(target:"p2p","listening to: {:?}",multi_addr);

        let sender = sender_channel;
        let mut swarm = self.swarm.lock().await;
        let mut p2p_command_recv = self.p2p_command_recv.lock().await;

        loop {
            // Create futures before select to ensure they're polled fairly
            let next_event = swarm.next();
            let next_command = p2p_command_recv.recv();

            select! {
                event = next_event => {

                    if let Some(event) = event {
                        Self::handle_swarm_events(self.clone().pending_request, event, sender.clone()).await
                    } else {
                        info!("no current swarm event")
                    }

                },
                cmd = next_command => {

                    match cmd {
                        Some(NetworkCommand::SendResponse {response,channel}) => {
                            if channel.is_open() {
                                swarm.behaviour_mut().send_response(channel,Ok(response))
                                    .map_err(|err|anyhow!("failed to send response; {err:?}"))?;
                            } else {
                                error!("response channel is closed");
                            }
                        },
                        Some(NetworkCommand::SendRequest {request,peer_id,target_multi_addr}) => {
                            if swarm.is_connected(&peer_id) {
                                swarm.behaviour_mut().send_request(&peer_id,request);
                                info!("request sent to peer: {peer_id:?}");
                            } else {
                                info!("re dialing");
                                swarm.dial(target_multi_addr).map_err(|err|anyhow!("failed to re dial: {err}"))?;
                                swarm.behaviour_mut().send_request(&peer_id,request);
                                info!("request sent to peer: {peer_id:?}");
                            }
                        },
                        Some(NetworkCommand::Dial {target_multi_addr,target_peer_id}) => {
                            // check first if the peer communication is already connected
                            if swarm.is_connected(&target_peer_id){
                                info!("peer already connected: {target_peer_id}")
                            }else{
                                swarm.dial(target_multi_addr).map_err(|err|anyhow!("failed to dial: {err}"))?;
                                info!("dialing peer: {target_peer_id} ")
                            }
                        },
                        None => {
                            info!("command channel closed");
                        }
                    }
                }
            }

            // Optional: Add a small delay to prevent tight loop
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

#[derive(Clone)]
pub struct P2pNetworkService {
    // for sending p2p network commands
    pub p2p_command_tx: Arc<Sender<NetworkCommand>>,
    // p2p worker instance
    pub p2p_worker: P2pWorker,
}

impl P2pNetworkService {
    pub fn new(
        p2p_command_tx: Arc<Sender<NetworkCommand>>,
        p2p_worker: P2pWorker,
    ) -> Result<Self, Error> {
        Ok(Self {
            p2p_command_tx,
            p2p_worker,
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

    pub async fn send_request(
        &mut self,
        request: Arc<Mutex<TxStateMachine>>,
        target_peer_id: PeerId,
        target_multi_addr: Multiaddr,
    ) -> Result<(), Error> {
        let request = request.lock().await;
        let encoded_req = request.encode();
        let req_command = NetworkCommand::SendRequest {
            request: encoded_req,
            peer_id: target_peer_id,
            target_multi_addr,
        };

        self.p2p_command_tx
            .send(req_command)
            .await
            .map_err(|err| anyhow!("failed to send req command; {err}"))?;
        info!(target: "p2p","sending request command to the swarm thread ");
        Ok(())
    }

    pub async fn send_response(
        &mut self,
        outbound_id: u64,
        response: Arc<Mutex<TxStateMachine>>,
    ) -> Result<(), anyhow::Error> {
        let txn = response.lock().await;
        let txn_state = new_tx_state_from_mutex(txn);
        let encoded_resp = txn_state.encode();
        let channel = self
            .clone()
            .p2p_worker
            .pending_request
            .lock_owned()
            .await
            .remove(&outbound_id)
            .ok_or(anyhow!("failed to get response channel"))?;

        let resp_command = NetworkCommand::SendResponse {
            response: encoded_resp,
            channel,
        };
        self.p2p_command_tx.send(resp_command).await?;
        info!(target: "p2p","sending response command");

        Ok(())
    }
}
