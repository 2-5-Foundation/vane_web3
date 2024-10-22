use anyhow::anyhow;
use core::pin::Pin;
use core::str::FromStr;
use log::{debug, info, trace};
use std::io;
use std::sync::Arc;
use std::time::Duration;
// peer discovery
// app to app communication (i.e sending the tx to be verified by the receiver) and back
use codec::Encode;
use libp2p::futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, Stream};
use libp2p::request_response::{Behaviour, Event, Message};
use libp2p::request_response::{Codec, ProtocolSupport, ResponseChannel};
use libp2p::swarm::SwarmEvent;
use libp2p::{Multiaddr, PeerId, Swarm, SwarmBuilder};
use local_ip_address::local_ip;
use primitives::data_structure::TxStateMachine;
use primitives::data_structure::{new_tx_state_from_mutex, PeerRecord};
use tokio::sync::Mutex;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;
use db::DbWorker;

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

pub struct P2pWorker {
    node_id: PeerId,
    swarm: Swarm<Behaviour<GenericCodec>>,
    url: Multiaddr
}

impl P2pWorker {
    /// generate new ed25519 keypair for node identity and register the peer record in  the db
    pub async fn new(db_worker: Arc<Mutex<DbWorker>>,port:u16) -> Result<Self, anyhow::Error> {

        let self_peer_id = libp2p::identity::Keypair::generate_ed25519();
        let peer_id = self_peer_id.public().to_peer_id().to_base58();
        let mut p2p_url = String::new();

        let local_ip = local_ip()
            .map_err(|err| anyhow!("failed to get local ip address; caused by: {err}"))?;

        if local_ip.is_ipv4() {
            p2p_url = format!("/ip4/{}/tcp/{}/p2p/{}", local_ip.to_string(), port - 541,peer_id);
        } else {
            p2p_url = format!("/ip6/{}/tcp/{}/p2p/{}", local_ip.to_string(), port - 541,peer_id);
        }

        let user_peer_id = PeerRecord {
            peer_address: peer_id,
            account_id1: None,
            account_id2: None,
            account_id3: None,
            account_id4: None,
            multi_addr: p2p_url,
            keypair: Some(
                self_peer_id
                    .to_protobuf_encoding()
                    .map_err(|_| anyhow!("failed to encode keypair"))?,
            ),
        };

        db_worker
            .lock()
            .await
            .record_user_peer_id(user_peer_id.clone())
            .await?;

        let url = user_peer_id.multi_addr;
        let multi_addr: Multiaddr = url
            .parse()
            .map_err(|err| anyhow!("failed to parse multi addr, caused by: {err}"))?;

        let peer_id: PeerId = PeerId::from_str(&user_peer_id.peer_address)
            .map_err(|err| anyhow!("failed to convert PeerId, caused by: {err}"))?;

        let secret_bytes = user_peer_id.keypair.ok_or(anyhow!("KeyPair is not set"))?;
        let keypair = libp2p::identity::Keypair::from_protobuf_encoding(&secret_bytes[..])
            .map_err(|_| anyhow!("failed to decode keypair ed25519"))?;

        let behaviour = Behaviour::new(
            vec![("vane web3", ProtocolSupport::Full)].into_iter(),
            Default::default(),
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
                cfg.with_idle_connection_timeout(Duration::from_secs(6000))
            })
            .build();

        Ok(Self {
            node_id: peer_id,
            swarm,
            url: multi_addr,
        })
    }

    // dialing the target peer_id
    pub async fn dial_to_peer_id(
        &mut self,
        target_url: Multiaddr,
        target_peer_id: PeerId,
    ) -> Result<(), anyhow::Error> {
        self.swarm
            .dial(target_url)
            .map_err(|_| anyhow!("failed to dial: {}", target_peer_id))?;
        Ok(())
    }

    // start the connection by listening to the address and then dialing and then listen to incoming messages events
    pub async fn start_swarm(
        &mut self,
    ) -> Result<BlockStream<Message<Vec<u8>, Result<Vec<u8>, anyhow::Error>>>, anyhow::Error> {
        let (sender_channel, recv_channel) = tokio::sync::mpsc::channel(256);

        let multi_addr = &self.url;

        let _listening_id = self.swarm.listen_on(multi_addr.clone())?;
        trace!(target:"p2p","listening to: {:?}",multi_addr);

        // listen to stream of incoming events
        while let Some(event) = self.swarm.next().await {
            match event {
                SwarmEvent::Behaviour(behaviour_event) => match behaviour_event {
                    Event::Message { message, .. } => {
                        info!(target: "p2p","message: {message:?}");
                        sender_channel
                            .send(Ok(message))
                            .await
                            .map_err(|_| anyhow!("failed to send in the message event channel"))?
                    }
                    Event::OutboundFailure { error, .. } => {
                        Err(anyhow!("outbound error: {error:?}"))?
                    }
                    Event::InboundFailure { error, .. } => {
                        Err(anyhow!("inbound error: {error:?}"))?
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
                    Err(anyhow!("listener error {error:?}"))?
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
                    debug!(target:"p2p","incoming connection error: {error:?}")
                }
                SwarmEvent::OutgoingConnectionError { error, .. } => {
                    debug!(target:"p2p","outgoing connection error: {error:?}")
                }
                SwarmEvent::ListenerClosed { reason, .. } => debug!("listener closed: {reason:?}"),
                SwarmEvent::NewListenAddr { address, .. } => {
                    info!(target:"p2p","new listener address: {address:?}")
                }
                SwarmEvent::ExpiredListenAddr { address, .. } => {
                    debug!(target:"p2p","expired listener add: {address:?}")
                }
                SwarmEvent::NewExternalAddrCandidate { address, .. } => {
                    info!(target:"p2p","new external addr candidate: {address:?}")
                }
                _ => debug!(target:"p2p","unhandled event"),
            }
        }

        Ok(Box::pin(ReceiverStream::new(recv_channel)) as BlockStream<_>)
    }

    pub async fn send_request(
        &mut self,
        request: Arc<Mutex<TxStateMachine>>,
        target_peer_id: PeerId,
    ) -> Result<(), anyhow::Error> {
        let request = request.lock().await;
        let encoded_req = request.encode();
        let outbound_req_id = self
            .swarm
            .behaviour_mut()
            .send_request(&target_peer_id, encoded_req);
        info!(target: "p2p","sending request to :{target_peer_id:?} outbound_id: {outbound_req_id:?}");
        Ok(())
    }

    pub async fn send_response(
        &mut self,
        response_channel: ResponseChannel<Result<Vec<u8>, anyhow::Error>>,
        response: Arc<Mutex<TxStateMachine>>,
    ) -> Result<(), anyhow::Error> {
        let txn = response.lock().await;
        let txn_state = new_tx_state_from_mutex(txn);
        let encoded_resp = txn_state.encode();
        self.swarm
            .behaviour_mut()
            .send_response(response_channel, Ok(encoded_resp))
            .map_err(|_| anyhow!("failed sending response"))?;
        Ok(())
    }
}
