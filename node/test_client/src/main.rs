use anyhow::Result;
use clap::Parser;
use libp2p::futures::StreamExt;
use libp2p::kad::Record;
use libp2p::swarm::derive_prelude::NetworkBehaviour;
use libp2p::{
    core::{
        multiaddr::{Multiaddr, Protocol},
        transport::OrTransport,
    },
    identity,
    kad::store::{MemoryStore, MemoryStoreConfig},
    kad::{Behaviour as DhtBehaviour, Config as KademliaConfig},
    noise, ping, relay,
    swarm::SwarmEvent,
    yamux, PeerId, Swarm,
};
use libp2p::{StreamProtocol, Transport};
use tracing::{error, info, warn, Level};

#[derive(Debug, Parser)]
#[command(name = "test_client")]
struct Opts {
    /// The relay address to connect to
    #[arg(long)]
    relay_address: String,
}

#[derive(NetworkBehaviour)]
struct Behaviour {
    relay_client: relay::client::Behaviour,
    ping: ping::Behaviour,
    dht: DhtBehaviour<MemoryStore>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_max_level(Level::INFO).init();

    let opts = Opts::parse();

    info!("🚀 Starting test client...");

    // Generate a keypair
    let keypair = identity::Keypair::generate_ed25519();
    let peer_id = PeerId::from(keypair.public());
    info!("🔑 Generated peer ID: {}", &peer_id);

    // Parse relay address
    let relay_multi_addr: Multiaddr = opts.relay_address.parse()?;
    info!("🔗 Relay address: {}", relay_multi_addr);

    // Create circuit address (relay + p2p-circuit)
    let circuit_addr = relay_multi_addr.clone().with(Protocol::P2pCircuit);
    info!("🔄 Circuit address: {}", circuit_addr);

    // Create relay client behavior
    let (relay_transport, relay_behaviour) = relay::client::new(peer_id.clone());

    let authenticated_relay = relay_transport
        .upgrade(libp2p::core::transport::upgrade::Version::V1)
        .authenticate(noise::Config::new(&keypair)?)
        .multiplex(yamux::Config::default())
        .boxed();

    // Create WebSocket transport for direct connections
    let websocket_transport = libp2p::websocket::Config::new(libp2p::tcp::tokio::Transport::new(
        libp2p::tcp::Config::default(),
    ))
    .upgrade(libp2p::core::transport::upgrade::Version::V1)
    .authenticate(noise::Config::new(&keypair)?)
    .multiplex(yamux::Config::default())
    .boxed();

    // Combine both transports
    let combined_transport = OrTransport::new(authenticated_relay, websocket_transport)
        .map(|either, _| match either {
            libp2p::futures::future::Either::Left((peer, muxer)) => (peer, muxer),
            libp2p::futures::future::Either::Right((peer, muxer)) => (peer, muxer),
        })
        .boxed();

    let dht_behaviour = DhtBehaviour::with_config(
        peer_id.clone(),
        MemoryStore::with_config(
            peer_id.clone(),
            MemoryStoreConfig {
                max_records: 10_000,         // Same as relay
                max_value_bytes: 2048,       // Increase this
                max_providers_per_key: 1024, // Keep this
                max_provided_keys: 100,      // Increase this
            },
        ),
        KademliaConfig::new(StreamProtocol::new("/vane_dht_protocol")),
    );

    // Create swarm with combined transport
    let mut swarm = Swarm::new(
        combined_transport,
        Behaviour {
            relay_client: relay_behaviour,
            ping: ping::Behaviour::new(ping::Config::new()),
            dht: dht_behaviour,
        },
        peer_id,
        libp2p::swarm::Config::with_tokio_executor()
            .with_idle_connection_timeout(std::time::Duration::from_secs(300)),
    );

    info!("📡 Swarm created successfully");

    // Step 1: Dial the relay node
    info!("📞 Dialing relay node...");
    swarm.dial(relay_multi_addr.clone())?;

    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
    // Step 2: Listen on circuit address (this should trigger reservation)
    info!("🎧 Listening on circuit address...");
    match swarm.listen_on(circuit_addr.clone()) {
        Ok(_) => info!("✅ Successfully started listening on circuit"),
        Err(e) => {
            error!("❌ Failed to listen on circuit: {}", e);
            return Err(e.into());
        }
    }

    //dht
    let dht = &mut swarm.behaviour_mut().dht;
    dht.add_address(&peer_id, relay_multi_addr.clone());

    let record = Record::new(peer_id.to_bytes().to_vec(), b"test_value".to_vec());
    info!("Hellooo");
    dht.put_record(record.clone(), libp2p::kad::Quorum::One)?;
    dht.start_providing(record.clone().key)?;

    // Event loop
    info!("🔄 Starting event loop...");
    loop {
        match swarm.select_next_some().await {
            SwarmEvent::NewListenAddr { address, .. } => {
                info!("🎧 New listen address: {}", address);
            }
            SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                info!("🟢 Connection established with: {}", peer_id);
            }
            SwarmEvent::ConnectionClosed { peer_id, cause, .. } => {
                warn!("🔌 Connection closed with {}: {:?}", peer_id, cause);
            }
            SwarmEvent::Dialing { peer_id, .. } => {
                info!(
                    "📞 Dialing peer: {}",
                    peer_id
                        .map(|p| p.to_string())
                        .unwrap_or_else(|| "unknown".to_string())
                );
            }
            SwarmEvent::Behaviour(BehaviourEvent::RelayClient(relay_event)) => match relay_event {
                relay::client::Event::ReservationReqAccepted { relay_peer_id, .. } => {
                    info!(
                        "✅ Reservation request ACCEPTED by relay: {}",
                        relay_peer_id
                    );
                }
                relay::client::Event::OutboundCircuitEstablished { relay_peer_id, .. } => {
                    info!(
                        "🔄 Outbound circuit established with relay: {}",
                        relay_peer_id
                    );
                }
                relay::client::Event::InboundCircuitEstablished { src_peer_id, .. } => {
                    info!("🔄 Inbound circuit established from: {}", src_peer_id);
                }
            },
            SwarmEvent::Behaviour(BehaviourEvent::Ping(_)) => {
                // Ignore ping events
            }
            event => {
                info!("📨 Other swarm event: {:?}", event);
            }
        }
    }
}
