use db::DbWorker;
use libp2p::Multiaddr;
use node::p2p::{BoxStream, P2pWorker};
use node::primitives::data_structure::{ChainSupported, OuterRequest, PeerRecord, Request};
use subxt_signer;

use anyhow::{anyhow, Error};
use libp2p::futures::FutureExt;
use libp2p::futures::StreamExt;
use libp2p::request_response::Message;
use log::{error, info};
use simplelog::*;
use std::fs::File;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::block_in_place;

fn log_setup() -> Result<(), anyhow::Error> {
    CombinedLogger::init(vec![
        TermLogger::new(
            LevelFilter::Debug,
            Config::default(),
            TerminalMode::Mixed,
            ColorChoice::Auto,
        ),
        WriteLogger::new(
            LevelFilter::Debug,
            Config::default(),
            File::create("vane.log").unwrap(),
        ),
    ])
    .unwrap();
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn p2p_test() -> Result<(), anyhow::Error> {
    log_setup()?;

    let (sender1, mut recv1) = tokio::sync::mpsc::channel(256);

    let alice1 = subxt_signer::sr25519::dev::alice().public_key().0;
    let self_keypair = libp2p::identity::Keypair::generate_ed25519();
    let self_multi_addr: Multiaddr = "/ip4/127.0.0.1/tcp/20200".parse()?;

    let bob = subxt_signer::sr25519::dev::bob().public_key().0;
    let target_keypair = libp2p::identity::Keypair::generate_ed25519();
    let target_multi_addr: Multiaddr = "/ip4/127.0.0.1/tcp/5820".parse()?;

    let self_peer_record = PeerRecord {
        peer_address: self_keypair.public().to_peer_id().to_bytes(),
        accountId1: alice1.to_vec(),
        accountId2: None,
        accountId3: None,
        accountId4: None,
        multi_addr: self_multi_addr.to_string().as_bytes().to_vec(),
        keypair: Some(self_keypair.to_protobuf_encoding().unwrap()),
    };

    // store user record
    let target_peer_record = PeerRecord {
        peer_address: target_keypair.public().to_peer_id().to_bytes(),
        accountId1: bob.to_vec(),
        accountId2: None,
        accountId3: None,
        accountId4: None,
        multi_addr: target_multi_addr.to_string().as_bytes().to_vec(),
        keypair: Some(target_keypair.to_protobuf_encoding().unwrap()),
    };
    // store target record and this should be fetched from PeerStore

    let worker1 = P2pWorker::new(self_peer_record).await?;
    let worker2 = P2pWorker::new(target_peer_record).await?;

    let worker1 = Arc::new(Mutex::new(worker1));
    let worker2 = Arc::new(Mutex::new(worker2));

    let worker1_clone = Arc::clone(&worker1);
    let worker2_clone = Arc::clone(&worker2);

    block_in_place(|| {
        Handle::current().block_on(async move {
            let res = worker1_clone.lock().await.start_swarm().await;
            let sender_res = sender1.send(res).await;
        });
    });

    // let mut stream2 = None;
    block_in_place(|| {
        Handle::current().block_on(async move {
            let res = worker2_clone.lock().await.start_swarm().await;
            //stream2 = Some(res)
        });
    });

    info!("Helloo am here");
    info!("Heyyyooo");

    worker1
        .lock()
        .await
        .dial_to_peer_id(target_multi_addr, target_keypair.public().to_peer_id())
        .await?;

    let req = Request {
        sender: vec![],
        receiver: vec![],
        amount: 1000,
        network: ChainSupported::Polkadot,
        msg: vec![],
    };

    worker1
        .lock()
        .await
        .send_request(req, target_keypair.public().to_peer_id())
        .await?;

    while let Some(stream_next) = recv1.recv().await {
        match stream_next {
            Ok(mut stream_inner) => {
                while let Some(stream_inner_inner) = stream_inner.next().await {
                    match stream_inner_inner {
                        Ok(stream_msg) => {
                            info!("msg {stream_msg:?}")
                        }
                        Err(e) => {
                            error!("{e:?}")
                        }
                    }
                }
            }
            Err(err) => {
                error!("stream 1 next error: {err:?}")
            }
        }
    }

    Ok(())
}

#[tokio::test]
async fn rpc_test() -> Result<(), anyhow::Error> {
    Ok(())
}

#[tokio::test]
async fn telemetry_test() -> Result<(), anyhow::Error> {
    Ok(())
}

// send tx and confirm both sender and receiver successfully

// send tx and revert tx due to address error and network error
