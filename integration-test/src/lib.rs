use codec::{Decode, Encode};
use db::DbWorker;
use jsonrpsee::core::client::ClientT;
use jsonrpsee::http_client::HttpClient;
use libp2p::{Multiaddr, PeerId};
use node::p2p::{BoxStream, P2pWorker};
use node::MainServiceWorker;
use primitives::data_structure::{ChainSupported, PeerRecord};
use simplelog::*;
use std::fs::File;
use std::sync::Arc;
use tokio::sync::Mutex;

fn log_setup() -> Result<(), anyhow::Error> {
    CombinedLogger::init(vec![
        TermLogger::new(
            LevelFilter::Info,
            Config::default(),
            TerminalMode::Mixed,
            ColorChoice::Auto,
        ),
        WriteLogger::new(
            LevelFilter::Info,
            Config::default(),
            File::create("vane-test.log").unwrap(),
        ),
    ])?;
    Ok(())
}

#[cfg(feature = "e2e")]
mod e2e_tests {
    use super::*;
    use crate::log_setup;
    use anyhow::{anyhow, Error};
    use db::db::new_client_with_url;
    use jsonrpsee::http_client::HttpClientBuilder;
    use jsonrpsee::rpc_params;
    use jsonrpsee::ws_client::WsClientBuilder;
    use libp2p::futures::StreamExt;
    use libp2p::request_response::Message;
    use log::{error, info};
    use node::rpc::Airtable;
    use node::MainServiceWorker;
    use primitives::data_structure::{
        AirtableRequestBody, Fields, PostRecord, SwarmMessage, TxStateMachine,
    };
    use std::sync::Arc;

    // having 2 peers; peer 1 sends a tx-state-machine message to peer 2
    // and peer2 respond a modified version of tx-state-machine.
    // and the vice-versa
    // #[tokio::test]
    // async fn p2p_test() -> Result<(), anyhow::Error> {
    //     log_setup();
    //
    //     let main_worker_1 = MainServiceWorker::e2e_new(3000, "../db/test1.db").await?;
    //     let main_worker_2 = MainServiceWorker::e2e_new(4000, "../db/test2.db").await?;
    //
    //     // Test state structure
    //     struct TestState {
    //         sent_msg: TxStateMachine,
    //         response_msg: TxStateMachine,
    //     }
    //
    //     // Create shared state using Arc
    //     let test_state = Arc::new(TestState {
    //         sent_msg: TxStateMachine::default(),
    //         response_msg: TxStateMachine {
    //             amount: 1000,
    //             ..Default::default()
    //         },
    //     });
    //
    //     // Spawn worker 1 task
    //     let worker_1 = main_worker_1.clone();
    //     let state_1 = test_state.clone();
    //
    //     let swarm_task_1 = tokio::spawn(async move {
    //         let mut swarm = worker_1.p2p_worker.lock().await.start_swarm().await?;
    //
    //         while let Some(event) = swarm.next().await {
    //             match event {
    //                 Ok(SwarmMessage::Request { .. }) => {
    //                     info!("Worker 1 received request");
    //                 }
    //                 Ok(SwarmMessage::Response { data, outbound_id }) => {
    //                     let received_response: TxStateMachine =
    //                         Decode::decode(&mut &data[..]).unwrap();
    //                     assert_eq!(received_response, state_1.response_msg);
    //                     return Ok(());
    //                 }
    //                 Err(e) => error!("Worker 1 error: {}", e),
    //             }
    //         }
    //         Ok::<_, anyhow::Error>(())
    //     });
    //
    //     // Spawn worker 2 task
    //     let worker_2 = main_worker_2.clone();
    //     let state_2 = test_state.clone();
    //
    //     let swarm_task_2 = tokio::spawn(async move {
    //         let mut swarm = worker_2.p2p_worker.lock().await.start_swarm().await?;
    //
    //         while let Some(event) = swarm.next().await {
    //             match event {
    //                 Ok(SwarmMessage::Request { data, inbound_id }) => {
    //                     println!("received a req: {data:?}");
    //                     // worker_2
    //                     //     .req_resp
    //                     //     .lock()
    //                     //     .await
    //                     //     .send_response(
    //                     //         channel,
    //                     //         Arc::new(Mutex::new(state_2.response_msg.clone())),
    //                     //     )
    //                     //     .await?;
    //                 }
    //                 Ok(SwarmMessage::Response { .. }) => {
    //                     info!("Worker 2 received response");
    //                 }
    //                 Err(e) => error!("Worker 2 error: {}", e),
    //             }
    //         }
    //         Ok::<_, anyhow::Error>(())
    //     });
    //
    //     // sending the request
    //     let peer_id_2 = main_worker_2.p2p_worker.lock().await.node_id;
    //     let multi_addr_2 = main_worker_2.p2p_worker.lock().await.url.clone();
    //
    //     main_worker_1
    //         .p2p_worker
    //         .lock()
    //         .await
    //         .dial_to_peer_id(multi_addr_2, peer_id_2)
    //         .await?;
    //
    //     main_worker_1
    //         .p2p_worker
    //         .lock()
    //         .await
    //         .send_request(Arc::new(Mutex::new(test_state.sent_msg.clone())), peer_id_2)
    //         .await?;
    //
    //     swarm_task_1.await??;
    //     swarm_task_2.await??;
    //
    //     Ok(())
    // }

    #[tokio::test]
    async fn rpc_test() -> Result<(), anyhow::Error> {
        log_setup();
        // main worker
        let main_worker_1 = MainServiceWorker::e2e_new(3000, "../db/test3.db").await?;
        main_worker_1.clone().e2e_run().await?;

        let rpc_url = main_worker_1.tx_rpc_worker.lock().await.rpc_url.clone();
        let full_url = format!("ws://{rpc_url}");
        let rpc_client = WsClientBuilder::default()
            .build(full_url.as_str())
            .await
            .expect("failed to intialize rpc ws client");

        // creating account
        if !rpc_client.is_connected() {
            info!("not connected to {full_url}")
        }

        let acc_id = alloy::primitives::Address::default().to_string();
        let network_id: String = ChainSupported::Ethereum.into();
        let params = rpc_params!(["Luka".to_string(), acc_id, network_id]);
        let res = rpc_client.request::<String, _>("register", params).await?;
        info!("request result: {res}");
        // initializing a transaction

        Ok(())
    }

    #[tokio::test]
    async fn airtable_test() -> Result<(), anyhow::Error> {
        log_setup();

        let client = Airtable::new().await?;
        let mut peer = Fields::default();
        let req_body = AirtableRequestBody::new(peer.clone());
        let record_data = client.create_peer(req_body).await?;
        assert_eq!(record_data.fields, peer.clone());

        // try updating
        peer.account_id1 = Some("4456".to_string());
        let new_req_body = PostRecord::new(peer.clone());
        let updated_record = client.update_peer(new_req_body, record_data.id).await?;
        // try fetching
        assert_eq!(updated_record.fields, peer);

        // delete all
        //client.delete_all().await?;

        Ok(())
    }

    #[tokio::test]
    async fn transaction_processing_test() -> Result<(), anyhow::Error> {
        Ok(())
    }

    #[tokio::test]
    async fn telemetry_test() -> Result<(), anyhow::Error> {
        Ok(())
    }

    // user creating an account, and sending a correct eth transaction works with recv and sender confirmation
    #[tokio::test]
    async fn user_flow_eth_works() -> Result<(), anyhow::Error> {
        Ok(())
    }

    // user creating an account, and sending a wrong eth address transaction reverts
    #[tokio::test]
    async fn user_flow_eth_wrong_address_reverts() -> Result<(), anyhow::Error> {
        Ok(())
    }

    #[tokio::test]
    async fn user_flow_erc20_works() -> Result<(), anyhow::Error> {
        Ok(())
    }

    #[tokio::test]
    async fn user_flow_bnb_works() -> Result<(), anyhow::Error> {
        Ok(())
    }

    #[tokio::test]
    async fn user_flow_bnb_wrong_network_reverts() -> Result<(), anyhow::Error> {
        Ok(())
    }
    #[tokio::test]
    async fn user_flow_bnb_reverts() -> Result<(), anyhow::Error> {
        Ok(())
    }

    #[tokio::test]
    async fn user_flow_brc20_works() -> Result<(), anyhow::Error> {
        Ok(())
    }

    #[tokio::test]
    async fn revenue_eth_works() -> Result<(), anyhow::Error> {
        Ok(())
    }

    #[tokio::test]
    async fn revenue_bnb_works() -> Result<(), anyhow::Error> {
        Ok(())
    }
}
