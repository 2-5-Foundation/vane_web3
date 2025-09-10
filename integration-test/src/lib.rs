// use std::fs;
// use walkdir::WalkDir;

// use alloy::signers::SignerSync;
// use alloy::signers::{local::PrivateKeySigner, Signer};
// use alloy_primitives::{keccak256, B256};
// use codec::{Decode, Encode};
// use db::LocalDbWorker;
// use jsonrpsee::core::client::ClientT;
// use jsonrpsee::http_client::HttpClient;
// use libp2p::{Multiaddr, PeerId};
// use native_node::p2p::{BoxStream, P2pWorker};
// use native_node::MainServiceWorker;
// use primitives::data_structure::{ChainSupported, PeerRecord, ETH_SIG_MSG_PREFIX};
// use simplelog::{
//     ColorChoice, CombinedLogger, Config, ConfigBuilder, LevelFilter, TermLogger, TerminalMode,
//     WriteLogger,
// };
// use std::fs::File;
// use std::sync::Arc;
// use tokio::sync::Mutex;

// fn log_setup() -> Result<(), anyhow::Error> {
//     let config = ConfigBuilder::new()
//         .set_target_level(LevelFilter::Error)
//         .set_location_level(LevelFilter::Info) // This enables file and line logging
//         .set_thread_level(LevelFilter::Off)
//         .set_time_level(LevelFilter::Off)
//         .build();

//     CombinedLogger::init(vec![
//         TermLogger::new(
//             LevelFilter::Info,
//             config.clone(), // Use the custom config
//             TerminalMode::Mixed,
//             ColorChoice::Auto,
//         ),
//         WriteLogger::new(
//             LevelFilter::Info,
//             config, // Use the same custom config
//             File::create("vane-test.log").unwrap(),
//         ),
//     ])?;
//     Ok(())
// }

// fn delete_unnecessary_test_files() -> Result<(), anyhow::Error> {
//     // Get current directory
//     let binding = std::env::current_dir().unwrap();
//     let current_dir = binding.parent().unwrap();

//     // Navigate to db directory
//     let search_dir = current_dir.join("db");

//     let patterns = [".db"];

//     for entry in WalkDir::new(search_dir).into_iter() {
//         match entry {
//             Ok(entry) => {
//                 let path = entry.path();

//                 if patterns
//                     .iter()
//                     .any(|pattern| path.to_string_lossy().to_string().contains(pattern))
//                 {
//                     if let Err(e) = fs::remove_file(path) {
//                         println!("Error deleting file {:?}: {}", path, e);
//                     } else {
//                         println!("Deleted: {:?}", path);
//                     }
//                 }
//             }
//             Err(e) => println!("Error accessing entry: {}", e),
//         }
//     }

//     Ok(())
// }

// #[cfg(feature = "e2e")]
// mod e2e_tests {
//     use super::*;
//     use crate::log_setup;
//     use alloy::signers::k256::ecdsa::SigningKey;
//     use alloy::signers::k256::FieldBytes;
//     use alloy::signers::local::LocalSigner;
//     use alloy_primitives::{hex, Keccak256};
//     use anyhow::{anyhow, Error};
//     use db::db::new_client_with_url;
//     use jsonrpsee::core::client::{Client, Subscription, SubscriptionClientT};
//     use jsonrpsee::core::params::ArrayParams;
//     use jsonrpsee::http_client::HttpClientBuilder;
//     use jsonrpsee::ws_client::WsClientBuilder;
//     use jsonrpsee::{rpc_params, SubscriptionMessage};
//     use libp2p::futures::StreamExt;
//     use libp2p::request_response::Message;
//     use log::{error, info};
//     use node_native::MainServiceWorker;
//     use primitives::data_structure::{
//         AccountInfo, AirtableRequestBody, Fields, PostRecord, SwarmMessage, TxStateMachine,
//         TxStatus,
//     };
//     use rand::Rng;
//     use std::hash::{DefaultHasher, Hash, Hasher};
//     use std::sync::Arc;

//     // having 2 peers; peer 1 sends a tx-state-machine message to peer 2
//     // and peer2 respond a modified version of tx-state-machine.
//     // and the vice-versa
//     #[tokio::test]
//     #[ignore]
//     async fn p2p_test() -> Result<(), anyhow::Error> {
//         let _ = log_setup();

//         // ========================================================================//
//         // Test state structure
//         struct TestState {
//             sent_msg: TxStateMachine,
//             response_msg: TxStateMachine,
//         }

//         // Create shared state using Arc
//         let test_state = Arc::new(TestState {
//             sent_msg: TxStateMachine::default(),
//             response_msg: TxStateMachine {
//                 amount: 1000,
//                 ..Default::default()
//             },
//         });
//         // ========================================================================//

//         let main_worker_1 = MainServiceWorker::e2e_new(3000, "../db/test1.db").await?;
//         let main_worker_2 = MainServiceWorker::e2e_new(4000, "../db/test2.db").await?;

//         let cloned_main_worker_1 = main_worker_1.clone();
//         let cloned_main_worker_2 = main_worker_2.clone();

//         let _worker_handle_1 = tokio::spawn(MainServiceWorker::e2e_run(cloned_main_worker_1));
//         let _worker_handle_2 = tokio::spawn(MainServiceWorker::e2e_run(cloned_main_worker_2));

//         // wait for proper initialization
//         tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

//         // ========================================================================//
//         let worker_1 = main_worker_1.clone();
//         let p2p_worker_1 = Arc::clone(&worker_1.p2p_worker);

//         // Clone worker_2 for the spawned task
//         let worker_2_for_task = main_worker_2.clone();
//         let p2p_worker_2 = Arc::clone(&worker_2_for_task.p2p_worker);
//         // Keep a separate clone for later use
//         let worker_2_for_later = main_worker_2.clone();

//         let state_1 = test_state.clone();
//         let state_2 = test_state.clone();

//         // ========================= listening to swarm ===========================//
//         let (sender_channel_1, mut recv_channel_1) = tokio::sync::mpsc::channel(256);
//         let (sender_channel_2, mut recv_channel_2) = tokio::sync::mpsc::channel(256);

//         let _swarm_task_1 = tokio::spawn(async move {
//             p2p_worker_1
//                 .lock()
//                 .await
//                 .start_swarm(sender_channel_1)
//                 .await
//         });

//         let _swarm_task_2 = tokio::spawn(async move {
//             p2p_worker_2
//                 .lock()
//                 .await
//                 .start_swarm(sender_channel_2)
//                 .await
//         });

//         // ========================================================================//

//         tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

//         let swarm_1 = tokio::spawn(async move {
//             println!("swarm 1 here ");
//             while let Some(event) = recv_channel_1.recv().await {
//                 println!("swarm 1 here inner ");
//                 match event {
//                     Ok(SwarmMessage::Request { .. }) => {
//                         info!("Worker 1 received request");
//                     }
//                     Ok(SwarmMessage::Response { data, outbound_id }) => {
//                         let received_response: TxStateMachine =
//                             Decode::decode(&mut &data[..]).unwrap();
//                         assert_eq!(received_response, state_1.response_msg);
//                         assert_eq!(1, 2);
//                     }
//                     Err(e) => error!("Worker 1 error: {}", e),
//                     _ => {}
//                 }
//             }
//             Ok::<(), Error>(())
//         });

//         let swarm_2 = tokio::spawn(async move {
//             println!("swarm 2 here ");
//             while let Some(event) = recv_channel_2.recv().await {
//                 println!("jello");
//                 match event {
//                     Ok(SwarmMessage::Request { data, inbound_id }) => {
//                         println!("received a req: {data:?}");
//                         let mut req_id_hash = DefaultHasher::default();
//                         inbound_id.hash(&mut req_id_hash);
//                         let req_id_hash = req_id_hash.finish();

//                         worker_2_for_task
//                             .p2p_network_service
//                             .lock()
//                             .await
//                             .send_response(
//                                 req_id_hash,
//                                 Arc::new(Mutex::new(state_2.response_msg.clone())),
//                             )
//                             .await?;
//                     }
//                     Ok(SwarmMessage::Response { data, outbound_id }) => {
//                         // nothing for now
//                     }
//                     Err(e) => error!("Worker 1 error: {}", e),
//                     _ => {}
//                 }
//             }
//             Ok::<(), Error>(())
//         });

//         // ========================================================================//
//         // Now use the separate clone for later operations
//         let peer_id_2 = worker_2_for_later.p2p_worker.lock().await.node_id;
//         let multi_addr_2 = worker_2_for_later.p2p_worker.lock().await.url.clone();

//         main_worker_1
//             .p2p_network_service
//             .lock()
//             .await
//             .dial_to_peer_id(multi_addr_2.clone(), &peer_id_2)
//             .await?;

//         tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

//         main_worker_1
//             .p2p_network_service
//             .lock()
//             .await
//             .send_request(
//                 Arc::new(Mutex::new(test_state.sent_msg.clone())),
//                 peer_id_2,
//                 multi_addr_2,
//             )
//             .await?;

//         // ========================================================================//

//         Ok(())
//     }
//     // Retry connection with backoff
//     async fn connect_with_retry(url: &str, max_attempts: u32) -> Result<Client, Error> {
//         let mut attempts = 0;
//         loop {
//             match WsClientBuilder::default().build(url).await {
//                 Ok(client) => return Ok(client),
//                 Err(e) => {
//                     attempts += 1;
//                     if attempts >= max_attempts {
//                         return Err(anyhow!("error failed to connect caused by; {e:?}"));
//                     }
//                     println!("Connection attempt {} failed, retrying in 1s...", attempts);
//                     tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
//                 }
//             }
//         }
//     }
//     #[tokio::test]
//     async fn transaction_full_cycle_test() -> Result<(), anyhow::Error> {
//         let _ = log_setup();

//         // ============================================================================
//         // Wallets
//         // --------------------------------- Wallet 1 ------------------------------------------- //
//         // testing 2 phantom wallet
//         //0xb82e9d2e1d6fe235859ae5ea619486d084d87ad8d48fb96f0923cd93ae39b4ac
//         let testing2_priv_key =
//             hex::decode(&"ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80")
//                 .unwrap();
//         let signing_key_1 =
//             SigningKey::from_bytes(<&FieldBytes>::from(&testing2_priv_key[..])).unwrap();
//         let wallet_1 = LocalSigner::from_signing_key(signing_key_1);

//         // --------------------------------- Wallet 2 ------------------------------------------- //
//         // meme coin phantom wallet
//         //0x30ec8bace0712e3f653bf986cbceae3fd017ceccd7636097687d12143b298f01
//         let meme_priv_key =
//             hex::decode(&"59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d")
//                 .unwrap();
//         let signing_key_2 =
//             SigningKey::from_bytes(<&FieldBytes>::from(&meme_priv_key[..])).unwrap();
//         let wallet_2 = LocalSigner::from_signing_key(signing_key_2);

//         let network_id: String = ChainSupported::Ethereum.into();

//         let port = rand::thread_rng().gen_range(0..=u16::MAX);

//         // ----------------------------------------------------------------------------------------//
//         // ============================================================================
//         let main_worker_1 = MainServiceWorker::e2e_new(port, "../db/test2.db").await?;
//         let main_worker_2 = MainServiceWorker::e2e_new(port + 80, "../db/test3.db").await?;
//         let airtable_client = main_worker_1
//             .tx_rpc_worker
//             .lock()
//             .await
//             .clone()
//             .airtable_client
//             .lock()
//             .await
//             .clone();

//         let cloned_worker_1 = main_worker_1.clone();
//         let worker_handle_1 =
//             tokio::spawn(async move { MainServiceWorker::e2e_run(cloned_worker_1).await });
//         let cloned_worker_2 = main_worker_2.clone();
//         let worker_handle_2 =
//             tokio::spawn(async move { MainServiceWorker::e2e_run(cloned_worker_2).await });

//         // ============================================================================

//         // rpc 1 (register)
//         let rpc_url_1 = main_worker_1.tx_rpc_worker.lock().await.rpc_url.clone();
//         let full_url_1 = format!("ws://{rpc_url_1}");

//         // rpc 2 (register)
//         let rpc_url_2 = main_worker_2.tx_rpc_worker.lock().await.rpc_url.clone();
//         let full_url_2 = format!("ws://{rpc_url_2}");

//         // ============================================================================
//         // Create RPC client and wait for it to be ready for 1
//         let rpc_client_1 = connect_with_retry(full_url_1.as_str(), 2).await?;

//         let rpc_client_2 = connect_with_retry(full_url_2.as_str(), 2).await?;

//         // ============================================================================

//         if rpc_client_1.is_connected() {
//             info!("ws client 1 connected")
//         }

//         if rpc_client_2.is_connected() {
//             info!("ws client 2 connected")
//         }

//         let mut register_params_1 = ArrayParams::new();
//         register_params_1.insert("Lukamba").unwrap();
//         register_params_1
//             .insert(wallet_1.address().to_string())
//             .unwrap();
//         register_params_1.insert(network_id.clone()).unwrap();
//         register_params_1.insert("ws://bla.900").unwrap();

//         let mut register_params_2 = ArrayParams::new();
//         register_params_2.insert("Haji").unwrap();
//         register_params_2
//             .insert(wallet_2.address().to_string())
//             .unwrap();
//         register_params_2.insert(network_id).unwrap();
//         register_params_2.insert("ws:nju.890").unwrap();

//         // ============================================================================

//         tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

//         let _res_register_1 = rpc_client_1
//             .request::<(), _>("register", register_params_1.clone())
//             .await
//             .expect("failed to call rpc register_1");

//         let _res_register_2 = rpc_client_2
//             .request::<(), _>("register", register_params_2.clone())
//             .await
//             .expect("failed to call rpc register_2");

//         // ============================================================================
//         tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

//         // subscribe to watch transactions
//         let ws_client_1 = connect_with_retry(full_url_1.as_str(), 2).await?;
//         let ws_client_2 = connect_with_retry(full_url_2.as_str(), 2).await?;

//         if !ws_client_1.is_connected() && !ws_client_2.is_connected() {
//             error!("ws client 1 or 2 not connected yet");
//         } else {
//             info!("ws client 1 and 2 connected");
//         }
//         // Create subscription
//         let mut subscription_2: Subscription<TxStateMachine> = ws_client_2
//             .subscribe::<TxStateMachine, _>(
//                 "subscribeTxUpdates",   // Must match the name in #[subscription(name = "...")]
//                 rpc_params!(),          // No params needed
//                 "unsubscribeTxUpdates", // Unsubscribe method is auto-generated as "unsubscribe" + method name
//             )
//             .await?;

//         let mut subscription_1: Subscription<TxStateMachine> = ws_client_1
//             .subscribe::<TxStateMachine, _>(
//                 "subscribeTxUpdates",   // Must match the name in #[subscription(name = "...")]
//                 rpc_params!(),          // No params needed
//                 "unsubscribeTxUpdates", // Unsubscribe method is auto-generated as "unsubscribe" + method name
//             )
//             .await?;

//         // Handle subscription in a task
//         let cloned_wallet_2 = wallet_2.clone();
//         let sub_handle_2 = tokio::spawn(async move {
//             println!("\n inside sub handle 2 \n");
//             while let Some(tx_update) = subscription_2.next().await {
//                 match tx_update {
//                     Ok(mut tx_state) => {
//                         // Handle your TxStateMachine update here
//                         match tx_state.status {
//                             TxStatus::Genesis => {
//                                 let msg = tx_state.clone().receiver_address;
//                                 let msg_len = msg.len().to_string();
//                                 let signable_msg = format!("{ETH_SIG_MSG_PREFIX}{msg_len}{msg}");
//                                 let pre_hash = keccak256(&signable_msg.as_bytes()[..]);

//                                 let sig = cloned_wallet_2
//                                     .sign_hash_sync(&pre_hash)
//                                     .expect("recv failed to sign msg");

//                                 let vec_sig = hex::encode(Vec::<u8>::from(sig));
//                                 println!("recv signature \n {} \n end ----", vec_sig);

//                                 tx_state.recv_signature = Some(Vec::from(sig));
//                                 // receiver confirm
//                                 ws_client_2
//                                     .request::<(), _>(
//                                         "receiverConfirm",
//                                         rpc_params!(tx_state.clone()),
//                                     )
//                                     .await
//                                     .expect("failed to confirm recv");
//                             }
//                             _ => panic!("in receiver's Tx State is invalid"),
//                         }
//                     }
//                     Err(e) => {
//                         error!("Subscription error: {}", e);
//                         break;
//                     }
//                 }
//             }
//         });

//         let cloned_wallet_1 = wallet_1.clone();
//         let sub_handle_1 = tokio::spawn(async move {
//             println!("inside sub handle 1 \n");
//             while let Some(tx_update) = subscription_1.next().await {
//                 match tx_update {
//                     Ok(mut tx_state) => {
//                         // Handle your TxStateMachine update here
//                         match tx_state.status {
//                             TxStatus::RecvAddrConfirmationPassed => {
//                                 let call_payload_pre_hash =
//                                     B256::new(tx_state.call_payload.unwrap());
//                                 let sig = cloned_wallet_1
//                                     .sign_hash_sync(&call_payload_pre_hash)
//                                     .expect("recv failed to sign msg");

//                                 tx_state.signed_call_payload = Some(Vec::from(sig));

//                                 // receiver confirm
//                                 ws_client_1
//                                     .request::<(), _>(
//                                         "senderConfirm",
//                                         rpc_params!(tx_state.clone()),
//                                     )
//                                     .await
//                                     .expect("failed to confirm sender");
//                             }
//                             TxStatus::FailedToSubmitTxn(msg) => {
//                                 println!("in handle 1: {msg:?}")
//                             }
//                             TxStatus::TxSubmissionPassed(hash) => {
//                                 println!("trx submitted hash: {hash:?}")
//                             }
//                             _ => panic!("in sender's side txStatus is invalid"),
//                         }
//                     }
//                     Err(e) => {
//                         error!("Subscription error: {}", e);
//                         break;
//                     }
//                 }
//             }
//         });

//         // ============================================================================
//         tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
//         println!("\n before initiating transactions \n");

//         // rpc_1 (initiate transaction)
//         let amount: u128 = 1000000000000000000000;
//         let mut tx_params = ArrayParams::new();
//         tx_params.insert(wallet_1.address().to_string()).unwrap();
//         tx_params.insert(wallet_2.address().to_string()).unwrap();
//         tx_params.insert(amount).unwrap();
//         tx_params.insert("Eth".to_string()).unwrap();
//         tx_params.insert("Ethereum".to_string()).unwrap();

//         let _res_txn = rpc_client_1
//             .request::<(), _>("initiateTransaction", tx_params)
//             .await?;

//         // put timeout for the test
//         tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;

//         // TODO attest all storage changes and effect
//         // 1. Success Tx recorded in DB
//         // 2. Tx success value in DB
//         // 3. Balances changes
//         // 4. Revenue recorded
//         // clean up
//         airtable_client.delete_all().await?;
//         delete_unnecessary_test_files()?;
//         Ok(())
//     }

//     #[tokio::test]
//     async fn recv_not_registered_error_works() -> Result<(), anyhow::Error> {
//         let _ = log_setup();

//         // ============================================================================
//         // Wallets
//         let wallet_1 = PrivateKeySigner::random();
//         let wallet_2 = PrivateKeySigner::random();

//         let network_id: String = ChainSupported::Ethereum.into();

//         // ============================================================================
//         let main_worker_1 = MainServiceWorker::e2e_new(3000, "../db/test2.db").await?;
//         let airtable_client = main_worker_1
//             .tx_rpc_worker
//             .lock()
//             .await
//             .clone()
//             .airtable_client
//             .lock()
//             .await
//             .clone();

//         let cloned_worker_1 = main_worker_1.clone();
//         let worker_handle_1 =
//             tokio::spawn(async move { MainServiceWorker::e2e_run(cloned_worker_1).await });

//         // ============================================================================

//         // rpc 1 (register)
//         let rpc_url_1 = main_worker_1.tx_rpc_worker.lock().await.rpc_url.clone();
//         let full_url_1 = format!("ws://{rpc_url_1}");

//         // ============================================================================
//         // Create RPC client and wait for it to be ready for 1
//         let rpc_client_1 = connect_with_retry(full_url_1.as_str(), 2).await?;

//         // ============================================================================

//         if rpc_client_1.is_connected() {
//             info!("ws client 1 connected")
//         }

//         let mut register_params_1 = ArrayParams::new();
//         register_params_1.insert("Lukamba").unwrap();
//         register_params_1
//             .insert(wallet_1.address().to_string())
//             .unwrap();
//         register_params_1.insert(network_id.clone()).unwrap();

//         // ============================================================================

//         tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

//         let _res_register_1 = rpc_client_1
//             .request::<(), _>("register", register_params_1.clone())
//             .await
//             .expect("failed to call rpc register_1");

//         // ============================================================================
//         tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

//         // subscribe to watch transactions
//         let ws_client_1 = connect_with_retry(full_url_1.as_str(), 2).await?;

//         if !ws_client_1.is_connected() {
//             error!("ws client 1 not connected yet");
//         } else {
//             info!("ws client 1 connected");
//         }

//         let mut subscription_1: Subscription<TxStateMachine> = ws_client_1
//             .subscribe::<TxStateMachine, _>(
//                 "subscribeTxUpdates",   // Must match the name in #[subscription(name = "...")]
//                 rpc_params!(),          // No params needed
//                 "unsubscribeTxUpdates", // Unsubscribe method is auto-generated as "unsubscribe" + method name
//             )
//             .await?;

//         let cloned_wallet_1 = wallet_1.clone();
//         let sub_handle_1 = tokio::spawn(async move {
//             println!("inside sub handle 1 \n");
//             while let Some(tx_update) = subscription_1.next().await {
//                 match tx_update {
//                     Ok(mut tx_state) => {
//                         println!("\n in sub_handle 1 watching tx: {tx_state:?} \n");
//                         // Handle your TxStateMachine update here
//                         match tx_state.status {
//                             TxStatus::RecvAddrConfirmationPassed => {
//                                 let call_payload_pre_hash =
//                                     B256::new(tx_state.call_payload.unwrap());
//                                 let sig = cloned_wallet_1
//                                     .sign_hash_sync(&call_payload_pre_hash)
//                                     .expect("recv failed to sign msg");

//                                 tx_state.signed_call_payload = Some(Vec::from(sig));

//                                 // receiver confirm
//                                 ws_client_1
//                                     .request::<(), _>(
//                                         "senderConfirm",
//                                         rpc_params!(tx_state.clone()),
//                                     )
//                                     .await
//                                     .expect("failed to confirm sender");
//                             }
//                             TxStatus::ReceiverNotRegistered => {
//                                 println!(" Recv not registered: {tx_state:?}")
//                             }
//                             _ => panic!("in sender's side txStatus is invalid"),
//                         }
//                     }
//                     Err(e) => {
//                         error!("Subscription error: {}", e);
//                         break;
//                     }
//                 }
//             }
//         });

//         // ============================================================================
//         tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
//         println!("\n before initiating transactions \n");

//         // rpc_1 (initiate transaction)
//         let mut tx_params = ArrayParams::new();
//         tx_params.insert(wallet_1.address().to_string()).unwrap();
//         tx_params.insert(wallet_2.address().to_string()).unwrap();
//         tx_params.insert(100_000).unwrap();
//         tx_params.insert("Eth".to_string()).unwrap();
//         tx_params.insert("Ethereum".to_string()).unwrap();

//         let _res_txn = rpc_client_1
//             .request::<(), _>("initiateTransaction", tx_params)
//             .await?;

//         // put timeout for the test
//         tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
//         // clean up
//         airtable_client.delete_all().await?;
//         Ok(())
//     }

//     // user creating an account, and sending a wrong eth address transaction reverts
//     #[tokio::test]
//     async fn user_flow_eth_wrong_address_reverts() -> Result<(), anyhow::Error> {
//         // TODO
//         // 1. Wrong address signing
//         // 2. Sender gets notification on wrong address
//         // 3. Sender cancels transaction
//         // 4. Attest no Balance changes
//         // 5. Attest Db recorded Failed Tx and Failed Tx value
//         Ok(())
//     }

//     #[tokio::test]
//     async fn revenue_eth_works() -> Result<(), anyhow::Error> {
//         Ok(())
//     }
// }
