// this layer should only be about user interaction
// receive tx requests
// pre processing
// send to tx processing layer
// NGIX for ssl

// ========================================
// TODO!
// Make sure that user cannot manually change the STATUS,RECEIVER AND SENDER ADDRESSES, AND NETWORK AND TOKEN ID
// hence hash all those values and everytime user updates, assert if they are intact
// ========================================

extern crate alloc;

use core::str::FromStr;
use anyhow::anyhow;
use jsonrpsee::core::JsonValue;

use log::{info, trace};

use crate::cryptography::verify_public_bytes;
use primitives::data_structure::{
    AccountInfo, AirtableRequestBody, AirtableResponse, ChainSupported, Discovery, Fields, PeerRecord, PostRecord, Record, Token, TxStateMachine, TxStatus, UserAccount
};
use sp_core::{blake2_256, H256};
use sp_runtime::traits::Zero;

use primitives::data_structure::DbWorkerInterface;

#[cfg(not(target_arch = "wasm32"))]
pub use std_imports::*;

#[cfg(not(target_arch = "wasm32"))]
mod std_imports {
    pub use crate::cryptography::verify_public_bytes;
    pub use alloc::sync::Arc;
    pub use db::LocalDbWorker;
    pub use jsonrpsee::core::Error;
    pub use jsonrpsee::{
        core::{async_trait, RpcResult, SubscriptionResult},
        proc_macros::rpc,
        PendingSubscriptionSink, SubscriptionMessage,
    };
    pub use libp2p::PeerId;
    pub use local_ip_address;
    pub use local_ip_address::local_ip;
    pub use moka::future::Cache as AsyncCache;
    pub use reqwest::{ClientBuilder, Url};
    pub use tokio::sync::mpsc::{Receiver, Sender};
    pub use tokio::sync::{Mutex, MutexGuard};
    pub use sp_core::Blake2Hasher;
    pub use sp_core::Hasher;
}

// -------------------- WASM CRATES IMPORT ------------------ //
#[cfg(target_arch = "wasm32")]
use rpc_wasm_imports::*;

#[cfg(target_arch = "wasm32")]
mod rpc_wasm_imports {
    pub use alloc::rc::Rc;
    pub use async_stream::stream;
    pub use core::cell::RefCell;
    pub use db_wasm::OpfsRedbWorker;
    pub use futures::StreamExt;
    pub use libp2p::PeerId;
    pub use lru::LruCache;
    pub use reqwasm::http::{Request, RequestMode};
    pub use tokio_with_wasm::sync::mpsc::{Receiver, Sender};
    pub use tokio_with_wasm::sync::{Mutex, MutexGuard};
    pub use wasm_bindgen::JsValue;
    pub use wasm_bindgen::prelude::wasm_bindgen;
}

// ----------------------------------------------------------- /
const AIRTABLE_TOKEN: &'static str =
    "patk0xLAgM5lDfRnF.33307d75c85fdf2118d71025aa11eee60a87f9dc51ad876f56013054c1492540";
const BASE_ID: &'static str = "appP1AoGmxoh2EmDI";
const TABLE_ID: &'static str = "tblWKDAWkSieIHsO8";
const AIRTABLE_URL: &'static str = "https://api.airtable.com/v0/";

// ----------------------------------- WASM -------------------------------- //

#[cfg(target_arch = "wasm32")]
#[derive(Clone)]
pub struct AirtableWasm;

#[cfg(target_arch = "wasm32")]
impl AirtableWasm {
    pub async fn new() -> Result<Self, anyhow::Error> {
        Ok(Self)
    }

    pub async fn list_all_peers(&self) -> Result<Vec<Discovery>, anyhow::Error> {
        let url = format!("{}/{}/{}", AIRTABLE_URL, BASE_ID, TABLE_ID);

        let resp = Request::get(&url)
            .header("Authorization", &format!("Bearer {}", AIRTABLE_TOKEN))
            .header("Content-Type", "application/json")
            .send()
            .await?;

        if !resp.ok() {
            return Err(anyhow!("server or client error listing peers"));
        }

        let json = resp.json::<AirtableResponse>().await?;
        let mut peers = Vec::new();

        for record in json.records {
            let mut accounts = Vec::new();
            if let Some(account_id1) = record.fields.account_id1 {
                accounts.push(account_id1);
            }
            if let Some(account_id2) = record.fields.account_id2 {
                accounts.push(account_id2);
            }
            if let Some(account_id3) = record.fields.account_id3 {
                accounts.push(account_id3);
            }
            if let Some(account_id4) = record.fields.account_id4 {
                accounts.push(account_id4);
            }

            peers.push(Discovery {
                id: record.id,
                peer_id: record.fields.peer_id,
                multi_addr: record.fields.multi_addr,
                account_ids: accounts,
            });
        }

        Ok(peers)
    }

    pub async fn create_peer(&self, record: AirtableRequestBody) -> Result<Record, anyhow::Error> {
        let url = format!("{}/{}/{}", AIRTABLE_URL, BASE_ID, "peer_discovery");

        let resp = Request::post(&url)
            .header("Authorization", &format!("Bearer {}", AIRTABLE_TOKEN))
            .header("Content-Type", "application/json")
            .body(serde_json::to_string(&record)?)
            .send()
            .await?;

        if !resp.ok() {
            return Err(anyhow!("Error creating peer: {}", resp.status()));
        }

        let json = resp.json::<AirtableResponse>().await?;
        Ok(json.records[0].clone())
    }

    pub async fn update_peer(
        &self,
        record: PostRecord,
        record_id: String,
    ) -> Result<Record, anyhow::Error> {
        let url = format!(
            "{}/{}/{}/{}",
            AIRTABLE_URL, BASE_ID, "peer_discovery", record_id
        );

        let patch_value = serde_json::json!({
            "fields": {
                "accountId1": record.fields.account_id1.unwrap()
            }
        });

        let resp = Request::patch(&url)
            .header("Authorization", &format!("Bearer {}", AIRTABLE_TOKEN))
            .header("Content-Type", "application/json")
            .body(serde_json::to_string(&patch_value)?)
            .send()
            .await?;

        if !resp.ok() {
            return Err(anyhow!("Error updating peer: {}", resp.status()));
        }

        let record = resp.json::<Record>().await?;
        Ok(record)
    }

    #[cfg(feature = "e2e")]
    pub async fn delete_all(&self) -> Result<(), anyhow::Error> {
        let base_url = format!("{}/{}/{}", AIRTABLE_URL, BASE_ID, "peer_discovery");

        let records = self.list_all_peers().await?;
        for chunk in records.chunks(10) {
            let mut url = base_url.clone();

            for record in chunk {
                url.push_str(&format!("&records[]={}", record.id));
            }

            let resp = Request::delete(&url)
                .header("Authorization", &format!("Bearer {}", AIRTABLE_TOKEN))
                .send()
                .await?;

            if !resp.ok() {
                return Err(anyhow!("Error deleting records: {}", resp.status()));
            }
        }
        Ok(())
    }
}

// ------------------------------------------------------------------------- //
// ------------------------------------- WASM ------------------------------------- //
#[cfg(target_arch = "wasm32")]
#[derive(Clone)]
pub struct PublicInterfaceWorker {
    /// local database worker
    pub db_worker: Rc<OpfsRedbWorker>,
    /// central server to get peer data
    pub airtable_client: Rc<AirtableWasm>,
    /// receiving end of transaction which will be polled in websocket , updating state of tx to end user
    pub rpc_receiver_channel: Rc<RefCell<Receiver<TxStateMachine>>>,
    /// sender channel when user updates the transaction state, propagating to main service worker
    pub user_rpc_update_sender_channel: Rc<RefCell<Sender<TxStateMachine>>>,
    /// P2p peerId
    pub peer_id: PeerId,
    // txn_counter
    // HashMap<txn_counter,Integrity hash>
    //// tx pending store
    pub lru_cache: RefCell<LruCache<u64, TxStateMachine>>, // initial fees, after dry running tx initialy without optimization
}

#[cfg(target_arch = "wasm32")]
impl PublicInterfaceWorker {
    pub async fn new(
        airtable_client: AirtableWasm,
        db_worker: Rc<OpfsRedbWorker>,
        rpc_recv_channel: Rc<RefCell<Receiver<TxStateMachine>>>,
        user_rpc_update_sender_channel: Rc<RefCell<Sender<TxStateMachine>>>,
        peer_id: PeerId,
        lru_cache: LruCache<u64, TxStateMachine>,
    ) -> Result<Self, JsValue> {
        Ok(Self {
            db_worker,
            airtable_client: Rc::new(airtable_client),
            rpc_receiver_channel: rpc_recv_channel,
            user_rpc_update_sender_channel,
            peer_id,
            lru_cache: RefCell::new(lru_cache),
        })
    }
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
impl PublicInterfaceWorker {

    #[wasm_bindgen]
    pub async fn register_vane_web3(
        &self,
        name: String,
        account_id: String,
        network: String,
    ) -> Result<(), JsValue> {
        // TODO verify the account id as it belongs to the registerer
        let network = network.as_str().into();
        let user_account = UserAccount {
            user_name: name,
            account_id: account_id.clone(),
            network,
        };

        self.db_worker.set_user_account(user_account).await.map_err(|e|JsonValue::from_str(e.into()))?;

        // NOTE: the peer-record is already registered, the following is only updating account details of the record
        // update: account address related to peer id
        // ========================================================================================//

        // fetch the record
        let record = self
            .db_worker
            .get_user_peer_id(None, Some(self.peer_id.to_string()))
            .await.map_err(|e|JsonValue::from_str(e.into()))?;

        let peer_account = PeerRecord {
            record_id: record.record_id.clone(),
            peer_id: None,
            account_id1: Some(account_id),
            account_id2: None,
            account_id3: None,
            account_id4: None,
            multi_addr: None,
            keypair: None,
        };
        info!("updated user peer record to be stored in local db");

        self.db_worker
            .update_user_peer_id_accounts(peer_account.clone())
            .await.map_err(|e|JsonValue::from_str(e.into()))?;

        // update to airtable
        let field: Fields = peer_account.into();
        let req_body = PostRecord::new(field);

        self.airtable_client
            .update_peer(req_body, record.record_id)
            .await.map_err(|e|JsonValue::from_str(e.into()))?;

        info!("updated airtable db with user peer id");

        Ok(())
    }

    #[wasm_bindgen]
    pub async fn initiate_transaction(
        &self,
        sender: String,
        receiver: String,
        amount: u128,
        token: String,
        network: String,
    ) -> Result<(), JsValue> {
        info!("initiated sending transaction");
        let token = token.as_str().into();

        let network = network.as_str().into();
        if let (Ok(net_sender), Ok(net_recv)) = (
            verify_public_bytes(sender.as_str(), token, network),
            verify_public_bytes(receiver.as_str(), token, network),
        ) {
            if net_sender != net_recv {
                Err(anyhow!("sender and receiver should be same network")).map_err(|e|JsonValue::from_str(e.into()))?;
            }

            info!("successfully initially verified sender and receiver and related network bytes");
            // construct the tx
            let mut sender_recv = sender.as_bytes().to_vec();
            sender_recv.extend_from_slice(receiver.as_bytes());
            let multi_addr = blake2_256(&sender_recv[..]);

            let mut nonce = 0;
            nonce = self.db_worker.get_nonce().await.map_err(|e|JsonValue::from_str(e.into()))? + 1;
            // update the db on nonce
            self.db_worker.increment_nonce().await.map_err(|e|JsonValue::from_str(e.into()))?;

            let tx_state_machine = TxStateMachine {
                sender_address: sender,
                receiver_address: receiver,
                multi_id: multi_addr,
                recv_signature: None,
                network: net_sender,
                status: TxStatus::default(),
                amount,
                signed_call_payload: None,
                call_payload: None,
                inbound_req_id: None,
                outbound_req_id: None,
                tx_nonce: nonce,
            };

            // dry run the tx

            // propagate the tx to lower layer (Main service worker layer)
            let sender_channel = self.user_rpc_update_sender_channel.borrow_mut();

            let sender = sender_channel.clone();
            sender
                .send(tx_state_machine)
                .await
                .map_err(|_| anyhow!("failed to send initial tx state to sender channel")).map_err(|e|JsonValue::from_str(e.into()))?;
            info!("propagated initiated transaction to tx handling layer")
        } else {
            Err(anyhow!(
                "sender and receiver should be correct accounts for the specified token"
            )).map_err(|e|JsonValue::from_str(e.into()))?;
        }
        Ok(())
    }

    #[wasm_bindgen]
    pub async fn sender_confirm(&self, mut tx: TxStateMachine) -> Result<(), JsValue> {
        let sender_channel = self.user_rpc_update_sender_channel.borrow_mut();
        if tx.signed_call_payload.is_none() && tx.status != TxStatus::RecvAddrConfirmationPassed {
            // return error as receiver hasnt confirmed yet or sender hasnt confirmed on his turn
            Err(anyhow!(
                "Wait for Receiver to confirm or sender should confirm".to_string(),
            )).map_err(|e|JsonValue::from_str(e.into()))?;
        } else {
            // remove from cache
            self.lru_cache.borrow_mut().demote(&tx.tx_nonce.into());
            // verify the tx-state-machine integrity
            // TODO
            // update the TxStatus to TxStatus::SenderConfirmed
            tx.sender_confirmation();
            let sender = sender_channel.clone();
            sender.send(tx).await.map_err(|_| {
                anyhow!("failed to send sender confirmation tx state to sender-channel")
            }).map_err(|e|JsonValue::from_str(e.into()))?;
        }
        Ok(())
    }

    async fn watch_tx_updates(&self) -> Result<(), JsValue> {
        Ok(())
    }

    #[wasm_bindgen]
    async fn fetch_pending_tx_updates(&self) -> Result<JsValue, JsValue> {
        let tx_updates = self
            .lru_cache.borrow()
            .iter()
            .map(|(_k, v)| v.clone())
            .collect::<Vec<TxStateMachine>>();
        println!("lru: {tx_updates:?}");

        JsValue::from_serde(&tx_updates)
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }

    #[wasm_bindgen]
    pub async fn receiver_confirm(&self, mut tx: TxStateMachine) -> Result<(), JsValue> {
        let sender_channel = self.user_rpc_update_sender_channel.borrow_mut();
        if tx.recv_signature.is_none() {
            // return error as we do not accept any other TxStatus at this api and the receiver should have signed for confirmation
            Err(anyhow!("Receiver did not confirm".to_string())).map_err(|e|JsonValue::from_str(e.into()))?
        } else {
            // remove from cache
            self.lru_cache.borrow_mut().demote(&tx.tx_nonce.into());
            // verify the tx-state-machine integrity
            // TODO
            // tx status to TxStatus::RecvAddrConfirmed
            tx.recv_confirmed();
            let sender = sender_channel.clone();
            sender.send(tx).await.map_err(|_| {
                anyhow!("failed to send recv confirmation tx state to sender channel")
            }).map_err(|e|JsonValue::from_str(e.into()))?;

            Ok(())
        }
    }
}

// ------------------------------------------------------------------------------------------ //

// minimal airtable client
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone)]
pub struct Airtable {
    client: reqwest::Client,
}

#[cfg(not(target_arch = "wasm32"))]
impl Airtable {
    pub async fn new() -> Result<Self, anyhow::Error> {
        let mut headers = reqwest::header::HeaderMap::new();
        let bt = format!("Bearer {}", AIRTABLE_TOKEN);
        let bearer = reqwest::header::HeaderValue::from_str(&bt)?;

        // Set the default headers.
        headers.append(reqwest::header::AUTHORIZATION, bearer);
        headers.append(
            reqwest::header::CONTENT_TYPE,
            reqwest::header::HeaderValue::from_static("application/json"),
        );

        let client_builder = ClientBuilder::new();
        let client = client_builder
            .default_headers(headers)
            .build()
            .map_err(|_| anyhow!("failed to build reqwest client"))?;

        Ok(Self { client })
    }

    pub async fn list_all_peers(&self) -> Result<Vec<Discovery>, anyhow::Error> {
        let url = Url::parse(AIRTABLE_URL)?;
        let list_record_url = url.join(&(BASE_ID.to_string() + "/" + TABLE_ID))?;

        let req = self.client.get(list_record_url).build()?;
        let resp = self.client.execute(req).await?;

        if resp.status().is_server_error() || resp.status().is_client_error() {
            Err(anyhow!("server or client error listing peers"))?
        }
        let body = resp.bytes().await?;
        let json_value = serde_json::from_slice::<&serde_json::value::RawValue>(&*body)?;
        let record: AirtableResponse = serde_json::from_str(json_value.get())?;

        let mut peers: Vec<Discovery> = vec![];

        record.records.iter().cloned().for_each(|record| {
            let mut accounts: Vec<AccountInfo> = vec![];

            if let Some(account_id1) = record.fields.account_id1.clone() {
                accounts.push(account_id1);
            }
            if let Some(account_id2) = record.fields.account_id2.clone() {
                accounts.push(account_id2);
            }
            if let Some(account_id3) = record.fields.account_id3.clone() {
                accounts.push(account_id3);
            }
            if let Some(account_id4) = record.fields.account_id4.clone() {
                accounts.push(account_id4);
            }

            // build the discovery object
            let disc = Discovery {
                id: record.id,
                peer_id: record.fields.peer_id,
                multi_addr: record.fields.multi_addr,
                account_ids: accounts,
                rpc: record.fields.rpc,
            };
            peers.push(disc)
        });

        Ok(peers)
    }

    pub async fn create_peer(&self, record: AirtableRequestBody) -> Result<Record, anyhow::Error> {
        let url = Url::parse(AIRTABLE_URL)?;
        let create_record_url = url.join(&(BASE_ID.to_string() + "/" + "peer_discovery"))?;
        
        println!("record: {:?}", serde_json::json!(record));
        
        let resp = self
            .client
            .post(create_record_url)
            .json::<AirtableRequestBody>(&record.into())
            .send()
            .await?;

        if resp.status().is_server_error() {
            Err(anyhow!("server, create peer: {}", resp.status()))?
        }
        if resp.status().is_client_error() {
            Err(anyhow!("client error, create peer: {}", resp.status()))?
        }
        if resp.status().is_success() {
            info!("succesfully created peer in airtable");
        }

        let resp_object = resp.json::<AirtableResponse>().await?;
        let resp = resp_object.records.first().unwrap().clone();
        Ok(resp)
    }

    // fetch a single peer
    pub async fn fetch_peer(&self, record_id: String) -> Result<Record, anyhow::Error>{
        let url = Url::parse(AIRTABLE_URL)?;
        let fetch_url = url.join(&(BASE_ID.to_string() + "/" + "peer_discovery" + "/" + record_id.as_str()))?;
        let resp = self.client.get(fetch_url).send().await?;
        let resp = resp.json::<Record>().await?;
        Ok(resp)
    }

    // a patch request
    pub async fn update_peer(
        &self,
        record: PostRecord,
        record_id: String,
    ) -> Result<Record, anyhow::Error> {
        let url = Url::parse(AIRTABLE_URL)?;
        let patch_record_url =
            url.join(&(BASE_ID.to_string() + "/" + "peer_discovery" + "/" + record_id.as_str()))?;

        let fields_value = serde_json::to_value(record.fields)
        .map_err(|e| anyhow!("Failed to serialize fields: {}", e))?;

        let patch_value = serde_json::json!({
        "fields": fields_value
        });

        let resp = self
            .client
            .patch(patch_record_url)
            .json(&patch_value)
            .send()
            .await?;

        if resp.status().is_server_error() {
            Err(anyhow!("server error, update peer"))?
        }
        if resp.status().is_client_error() {
            Err(anyhow!("client error, update peer"))?
        }
        if resp.status().is_success() {
            info!("succesfully updated peer in airtable");
        }

        let resp = resp.json::<Record>().await?;
        Ok(resp)
    }

    #[cfg(feature = "e2e")]
    pub async fn delete_all(&self) -> Result<(), anyhow::Error> {
        let url = Url::parse(AIRTABLE_URL)?;
        let delete_record_url = url.join(&(BASE_ID.to_string() + "/" + "peer_discovery"))?;

        // fetch all records
        let record_ids = self
            .list_all_peers()
            .await?
            .into_iter()
            .map(|discv| discv.id)
            .collect::<Vec<String>>();

        // Process records in batches of 10
        for chunk in record_ids.chunks(10) {
            // Reset query parameters for each batch
            let mut url = delete_record_url.clone();

            // Add each record ID as a query parameter
            for id in chunk {
                url.query_pairs_mut().append_pair("records[]", id);
            }

            let resp = self.client.delete(url).send().await?;

            if resp.status().is_server_error() {
                Err(anyhow!("server, delete record: {}", resp.status()))?
            }
            if resp.status().is_client_error() {
                Err(anyhow!("client error, delete records: {}", resp.status()))?
            }
            if resp.status().is_success() {
                info!("succesfully deleted records in airtable");
            }
        }
        Ok(())
    }
}

/// Trait
#[cfg(not(target_arch = "wasm32"))]
#[rpc(server, client)]
pub trait TransactionRpc {
    /// register user profile, generate node peer id and push the profile for vane discovery server
    /// params:
    ///
    ///  - `name`
    ///  - `accountId`
    ///  - `network`
    ///  - `rpc url`

    #[method(name = "register")]
    async fn register_vane_web3(
        &self,
        name: String,
        account_id: String,
        network: String,
        rpc: String
    ) -> RpcResult<()>;

    /// add crypto address account
    /// params:
    ///
    /// - `name`
    /// - `vec![(address, networkId)]`
    #[method(name = "addAccount")]
    async fn add_account(
        &self,
        name: String,
        accounts: Vec<(String, ChainSupported)>,
    ) -> RpcResult<()>;

    /// initiate tx to be verified recv address and network choice
    /// params:
    ///
    /// - `sender address`,
    /// - `receiver_address`,
    /// - `amount`,
    /// - `networkId`
    #[method(name = "initiateTransaction")]
    async fn initiate_transaction(
        &self,
        sender: String,
        receiver: String,
        amount: u128,
        token: String,
        network: String,
    ) -> RpcResult<()>;

    /// confirm sender signifying agreeing all tx state after verification and this will trigger actual submission
    #[method(name = "senderConfirm")]
    async fn sender_confirm(&self, tx: TxStateMachine) -> RpcResult<()>;

    /// watch tx update stream
    #[subscription(name ="subscribeTxUpdates",item = TxStateMachine )]
    async fn watch_tx_updates(&self) -> SubscriptionResult;

    /// fetch upstream pending tx-state-machine, works as an alternative to `subscribeTxUpdates`
    #[method(name = "fetchPendingTxUpdates")]
    async fn fetch_pending_tx_updates(&self) -> RpcResult<Vec<TxStateMachine>>;

    /// receiver confirmation on address and ownership of account ( network ) signifying correct token to the network choice
    #[method(name = "receiverConfirm")]
    async fn receiver_confirm(&self, tx: TxStateMachine) -> RpcResult<()>;
}

/// handling tx submission & tx confirmation & tx simulation interactions
/// a first layer a user interact with and submits the tx to processing layer
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone)]
pub struct TransactionRpcWorker {
    /// local database worker
    pub db_worker: Arc<Mutex<LocalDbWorker>>,
    /// central server to get peer data
    pub airtable_client: Arc<Mutex<Airtable>>,
    /// rpc server url
    pub rpc_url: String,
    /// receiving end of transaction which will be polled in websocket , updating state of tx to end user
    pub rpc_receiver_channel: Arc<Mutex<Receiver<TxStateMachine>>>,
    /// sender channel when user updates the transaction state, propagating to main service worker
    pub user_rpc_update_sender_channel: Arc<Mutex<Sender<Arc<Mutex<TxStateMachine>>>>>,
    /// P2p peerId
    pub peer_id: PeerId,
    // txn_counter
    // HashMap<txn_counter,Integrity hash>
    /// tx pending store
    pub moka_cache: AsyncCache<u64, TxStateMachine>, // initial fees, after dry running tx initialy without optimization
}

#[cfg(not(target_arch = "wasm32"))]
impl TransactionRpcWorker {
    pub async fn new(
        airtable_client: Airtable,
        db_worker: Arc<Mutex<LocalDbWorker>>,
        rpc_recv_channel: Arc<Mutex<Receiver<TxStateMachine>>>,
        user_rpc_update_sender_channel: Arc<Mutex<Sender<Arc<Mutex<TxStateMachine>>>>>,
        port: u16,
        peer_id: PeerId,
        moka_cache: AsyncCache<u64, TxStateMachine>,
    ) -> Result<Self, anyhow::Error> {
        let local_ip = local_ip()
            .map_err(|err| anyhow!("failed to get local ip address; caused by: {err}"))?;

        let mut rpc_url = String::new();

        if local_ip.is_ipv4() {
            rpc_url = format!("{}:{}", local_ip.to_string(), port);
        } else {
            rpc_url = format!("{}:{}", local_ip.to_string(), port);
        }
        Ok(Self {
            db_worker,
            airtable_client: Arc::new(Mutex::new(airtable_client)),
            rpc_url,
            rpc_receiver_channel: rpc_recv_channel,
            user_rpc_update_sender_channel,
            peer_id,
            moka_cache,
        })
    }

    /// first dry tx, returns the projected fees
    pub async fn dry_run_tx(
        network: ChainSupported,
        _sender: String,
        _recv: String,
        _token: Token,
        _amount: u64,
    ) -> Result<u64, anyhow::Error> {
        let _fees = match network {
            ChainSupported::Polkadot => {}
            ChainSupported::Ethereum => {}
            ChainSupported::Bnb => {}
            ChainSupported::Solana => {}
        };
        todo!()
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[async_trait]
impl TransactionRpcServer for TransactionRpcWorker {
    async fn register_vane_web3(
        &self,
        name: String,
        account_id: String,
        network: String,
        rpc: String
    ) -> RpcResult<()> {
        // TODO verify the account id as it belongs to the registerer
        let network = network.as_str().into();
        let user_account = UserAccount {
            user_name: name,
            account_id: account_id.clone(),
            network,
        };

        self.db_worker
            .lock()
            .await
            .set_user_account(user_account)
            .await?;
        // NOTE: the peer-record is already registered, the following is only updating account details of the record
        // update: account address related to peer id
        // ========================================================================================//
        // fetch the record
        let record = self
            .db_worker
            .lock()
            .await
            .get_user_peer_id(None, Some(self.peer_id.to_string()))
            .await?;

        let acc_info = AccountInfo {
            account: account_id,
            network
        };

        let peer_account = PeerRecord {
            record_id: record.record_id.clone(),
            peer_id: None,
            account_id1: Some(acc_info),
            account_id2: None,
            account_id3: None,
            account_id4: None,
            multi_addr: None,
            keypair: None,
        };

        info!("updated user peer record to be stored in local db");

        self.db_worker
            .lock()
            .await
            .update_user_peer_id_account_ids(peer_account.clone())
            .await?;
        
        Ok(())
    }

    async fn add_account(
        &self,
        _name: String,
        _accounts: Vec<(String, ChainSupported)>,
    ) -> RpcResult<()> {
        todo!()
    }

    async fn initiate_transaction(
        &self,
        sender: String,
        receiver: String,
        amount: u128,
        token: String,
        network: String,
    ) -> RpcResult<()> {
        info!("initiated sending transaction");
        let token = token.as_str().into();

        let network = network.as_str().into();
        if let (Ok(net_sender), Ok(net_recv)) = (
            verify_public_bytes(sender.as_str(), token, network),
            verify_public_bytes(receiver.as_str(), token, network),
        ) {
            if net_sender != net_recv {
                Err(anyhow!("sender and receiver should be same network"))?
            }

            info!("successfully initially verified sender and receiver and related network bytes");
            // construct the tx
            let mut sender_recv = sender.as_bytes().to_vec();
            sender_recv.extend_from_slice(receiver.as_bytes());
            let multi_addr = Blake2Hasher::hash(&sender_recv[..]);

            let mut nonce = 0;
            nonce = self.db_worker.lock().await.get_nonce().await? + 1;
            // update the db on nonce
            self.db_worker.lock().await.increment_nonce().await?;

            let tx_state_machine = TxStateMachine {
                sender_address: sender,
                receiver_address: receiver,
                multi_id: multi_addr.into(),
                recv_signature: None,
                network: net_sender,
                status: TxStatus::default(),
                amount,
                signed_call_payload: None,
                call_payload: None,
                inbound_req_id: None,
                outbound_req_id: None,
                tx_nonce: nonce,
            };

            // dry run the tx

            //let fees = self::dry_run_tx().map_err(|err|anyhow!("{}",err))?;

            // propagate the tx to lower layer (Main service worker layer)
            let sender_channel = self.user_rpc_update_sender_channel.lock().await;

            let sender = sender_channel.clone();
            sender
                .send(Arc::from(Mutex::new(tx_state_machine)))
                .await
                .map_err(|_| anyhow!("failed to send initial tx state to sender channel"))?;
            info!("propagated initiated transaction to tx handling layer")
        } else {
            Err(anyhow!(
                "sender and receiver should be correct accounts for the specified token"
            ))?
        }
        Ok(())
    }

    /// sender confirms by updating TxStatus to SenderConfirmed
    /// at this stage receiver should have confirmed and sender should also have confirmed
    /// sender cannot confirm if TxStatus is RecvAddrFailed
    async fn sender_confirm(&self, mut tx: TxStateMachine) -> RpcResult<()> {
        let sender_channel = self.user_rpc_update_sender_channel.lock().await;
        if tx.signed_call_payload.is_none() && tx.status != TxStatus::RecvAddrConfirmationPassed {
            // return error as receiver hasnt confirmed yet or sender hasnt confirmed on his turn
            Err(Error::Custom(
                "Wait for Receiver to confirm or sender should confirm".to_string(),
            ))?
        } else {
            // remove from cache
            self.moka_cache.remove(&tx.tx_nonce.into()).await;
            // verify the tx-state-machine integrity
            // TODO
            // update the TxStatus to TxStatus::SenderConfirmed
            tx.sender_confirmation();
            let sender = sender_channel.clone();
            sender.send(Arc::from(Mutex::new(tx))).await.map_err(|_| {
                anyhow!("failed to send sender confirmation tx state to sender-channel")
            })?;
        }
        Ok(())
    }

    /// receiver confirms by signing msg and updating TxStatus to RecvConfirmed
    async fn receiver_confirm(&self, mut tx: TxStateMachine) -> RpcResult<()> {
        let sender_channel = self.user_rpc_update_sender_channel.lock().await;
        if tx.recv_signature.is_none() {
            // return error as we do not accept any other TxStatus at this api and the receiver should have signed for confirmation
            Err(Error::Custom("Receiver did not confirm".to_string()))?
        } else {
            // remove from cache
            self.moka_cache.remove(&tx.tx_nonce.into()).await;
            // verify the tx-state-machine integrity
            // TODO
            // tx status to TxStatus::RecvAddrConfirmed
            tx.recv_confirmed();
            let sender = sender_channel.clone();
            sender.send(Arc::from(Mutex::new(tx))).await.map_err(|_| {
                anyhow!("failed to send recv confirmation tx state to sender channel")
            })?;
            Ok(())
        }
    }

    async fn watch_tx_updates(
        &self,
        subscription_sink: PendingSubscriptionSink,
    ) -> SubscriptionResult {
        let sink = subscription_sink
            .accept()
            .await
            .map_err(|_| anyhow!("failed to accept rpc ws channel"))?;
        while let Some(tx_update) = self.rpc_receiver_channel.lock().await.recv().await {
            trace!(target:"rpc","\n watching tx: {tx_update:?} \n");

            let subscription_msg = SubscriptionMessage::from_json(&tx_update)
                .map_err(|_| anyhow!("failed to convert tx update to json"))?;
            sink.send(subscription_msg)
                .await
                .map_err(|_| anyhow!("failed to send msg to rpc ws channel"))?;
        }
        Ok(())
    }

    async fn fetch_pending_tx_updates(&self) -> RpcResult<Vec<TxStateMachine>> {
        let tx_updates = self
            .moka_cache
            .iter()
            .map(|(_k, v)| v)
            .collect::<Vec<TxStateMachine>>();
        println!("moka: {tx_updates:?}");
        Ok(tx_updates)
    }
}

// -------------------------------------- WASM BINDGEN ----------------------------------------- //
