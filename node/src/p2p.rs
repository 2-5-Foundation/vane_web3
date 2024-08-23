use anyhow::anyhow;
use core::pin::Pin;
use std::collections::BTreeMap;
use log::{debug, error, info, trace, warn};
use std::io;
use std::sync::Arc;
use std::time::Duration;
// peer discovery
// app to app communication (i.e sending the tx to be verified by the receiver) and back
use crate::primitives::data_structure::{p2pConfig, PeerRecord};
use codec::Encode;
use libp2p::futures::{
    AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, Stream, TryStreamExt,
};
use libp2p::request_response::{Codec, InboundRequestId, OutboundRequestId, ProtocolSupport, ResponseChannel};
use libp2p::swarm::SwarmEvent;
use libp2p::{
    request_response::{Behaviour, Event, Message},
    swarm::NetworkBehaviour,
};
use libp2p::{Multiaddr, PeerId, Swarm, SwarmBuilder};
use tokio::sync::Mutex;
use tokio_stream::StreamExt;
use primitives::data_structure::{ChainSupported, OuterRequest, Request, Response};

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
pub struct P2pWorker {
    node_id: PeerId,
    swarm: Swarm<Behaviour<GenericCodec>>,
    url: Multiaddr,
    // storing pending requests to be replied
    pending_req: Arc<Mutex<BTreeMap<OutboundRequestId,Request>>>
}

impl P2pWorker {
    pub async fn new(user_peer_id: PeerRecord) -> Result<Self, anyhow::Error> {
        // the peer record keypair is already decrypted at this point
        let url = user_peer_id.multi_addr;
        let multi_addr: Multiaddr = core::str::from_utf8(&url[..]).map_err(|_|anyhow!("failed to convert bytes to str"))?
            .parse()
            .map_err(|_| anyhow!("failed to parse multi addr"))?;

        let peer_id: PeerId = PeerId::from_bytes(&user_peer_id.peer_address[..])?;

        let mut secret_bytes = user_peer_id.keypair.ok_or(anyhow!("KeyPair is not set"))?;
        let keypair = libp2p::identity::Keypair::from_protobuf_encoding(&secret_bytes[..])
            .map_err(|_| anyhow!("failed to decode keypair ed25519"))?;

        let behaviour = Behaviour::new(
            vec![("vane", ProtocolSupport::Full)].into_iter(),
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
            .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(u64::MAX)))

            .build();

        Ok(Self {
            node_id: peer_id,
            swarm,
            url: multi_addr,
            pending_req: Arc::new(Default::default()),
        })
    }

    // dialing the target peer_id
    pub async fn dial_to_peer_id(&mut self,target_url: Multiaddr,target_peer_id: PeerId) -> Result<(),anyhow::Error> {
        self.swarm
            .dial(target_url)
            .map_err(|_| anyhow!("failed to dial: {}", target_peer_id))?;
        Ok(())
    }

    // start the connection by listening to the address and then dialing and then listen to incoming messages events
    pub async fn start_swarm(
        &mut self
    ) -> Result<BoxStream<Message<Vec<u8>, Result<Vec<u8>,anyhow::Error>>,>, anyhow::Error> {
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
                        .send(Ok(message)).await
                        .map_err(|_| anyhow!("failed to send in the message event channel"))?
                    },
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
                    debug!("connection established: peer_id:{peer_id:?} endpoint:{endpoint:?} num_established:{num_established:?}")
                }
                SwarmEvent::IncomingConnection {
                    local_addr,
                    send_back_addr,
                    ..
                } => {
                    debug!("incoming connection: local_addr:{local_addr:?} send_back_addr:{send_back_addr:?}")
                }
                SwarmEvent::Dialing { peer_id, .. } => {
                    debug!("dialing to {peer_id:?}")
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
                    debug!("connection closed peer_id:{peer_id:?} endpoint:{endpoint:?} cause:{cause:?}")
                }
                SwarmEvent::IncomingConnectionError {error,..} => debug!("incoming connection error: {error:?}"),
                SwarmEvent::OutgoingConnectionError {error,..} => debug!("outgoing connection error: {error:?}"),
                SwarmEvent::ListenerClosed {reason,..} => debug!("listener closed: {reason:?}"),
                SwarmEvent::NewListenAddr {address,..} => debug!("new listener add: {address:?}"),
                SwarmEvent::ExpiredListenAddr {address,..} => debug!("expired listener add: {address:?}"),
                SwarmEvent::NewExternalAddrCandidate {address,..} => debug!("new external addr candidate: {address:?}"),
                _ => debug!("unhandled event")
            }
        }

        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(
            recv_channel,
        )))
    }

    pub async fn send_request(&mut self, request: Request, target_peer_id:PeerId) -> Result<(), anyhow::Error> {
        let encoded_req = request.encode();
        let outbound_req_id = self.swarm.behaviour_mut().send_request(&target_peer_id,encoded_req);
        info!(target: "p2p","sending request to :{target_peer_id:?} outbound_id: {outbound_req_id:?}");
        //record the outgoing request
        let mut pending_req =self.pending_req.lock().await;
        pending_req.insert(outbound_req_id,request);
        Ok(())
    }

    pub async fn send_response(
        &mut self,
        response_channel: ResponseChannel<Result<Vec<u8>,anyhow::Error>>,
        outer_request: OuterRequest,
        response: Response
    ) -> Result<(), anyhow::Error> {
        let encoded_resp =response.encode();
        self.swarm.behaviour_mut().send_response(response_channel,Ok(encoded_resp)).map_err(|_|anyhow!("failed sending response"))?;
        // delete the responded request
        let mut pending_req =self.pending_req.lock().await;
        pending_req.remove(&outer_request.id);
        Ok(())
    }

    pub async fn get_pending_req(&self) -> Option<Vec<OutboundRequestId>>{
        let pending_req =self.pending_req.lock().await;
        Some(pending_req.keys().into_iter().map(|k|*k).collect())
    }
}
