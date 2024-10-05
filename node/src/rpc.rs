// this layer should only be about user interaction
// receive tx requests
// pre processing
// send to tx processing layer
// NGIX for ssl

extern crate alloc;
use crate::cryptography::verify_public_bytes;
use alloc::sync::Arc;
use anyhow::anyhow;
use db::DbWorker;
use jsonrpsee::core::Serialize;
use jsonrpsee::core::__reexports::serde_json;
use jsonrpsee::{
    core::{async_trait, RpcResult, SubscriptionResult},
    proc_macros::rpc,
    PendingSubscriptionSink, SubscriptionMessage,
};
use log::{debug, info, warn};
use primitives::data_structure::{
    AirtableResponse, ChainSupported, Discovery, Fields, PeerRecord, Record, Token, TxStateMachine,
    TxStatus, UserAccount,
};
use reqwest::{ClientBuilder, Url};
use sp_core::{Blake2Hasher, Hasher};
use tinyrand::{Rand, Xorshift};
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::Mutex;

const AIRTABLE_SECRET: &'static str =
    "98c01b1e015d124f9317ee1c9dd1eb2deda439ef28e3d6e0975798a4a19f4768";
const AIRTABLE_CLIENT_ID: &'static str = "82c3deff-d786-465c-b456-b24c92c2c42f";
const AIRTABLE_TOKEN: &'static str =
    "patk0xLAgM5lDfRnF.33307d75c85fdf2118d71025aa11eee60a87f9dc51ad876f56013054c1492540";
const BASE_ID: &'static str = "appP1AoGmxoh2EmDI";
const TABLE_ID: &'static str = "tblWKDAWkSieIHsO8";
const AIRTABLE_URL: &'static str = "https://api.airtable.com/v0/";

// minimal airtable client
pub struct Airtable {
    client: reqwest::Client,
}

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
            Err(anyhow!("server or client error"))?
        }
        let body = resp.bytes().await?;
        let json_value = serde_json::from_slice::<&serde_json::value::RawValue>(&*body)?;
        //
        let record: AirtableResponse = serde_json::from_str(json_value.get())?;

        let mut peers: Vec<Discovery> = vec![];

        record.records.iter().cloned().for_each(|record| {
            let mut accounts: Vec<String> = vec![];

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
                peer_id: record
                    .fields
                    .peer_id
                    .unwrap_or("0x0000000000000000".to_string()),
                multi_addr: record
                    .fields
                    .multi_addr
                    .unwrap_or("ip4/1.1.1/tcp/1000/p2p/0x000000".to_string()),
                account_ids: accounts,
            };
            peers.push(disc)
        });

        Ok(peers)
    }

    pub async fn create_peer(&self, record: Fields) -> Result<(), anyhow::Error> {
        let url = Url::parse(AIRTABLE_URL)?;
        let list_record_url = url.join(&(BASE_ID.to_string() + "/" + TABLE_ID))?;

        let resp = self
            .client
            .patch(list_record_url)
            .json::<Fields>(&record.into())
            .send()
            .await?;
        if resp.status().is_server_error() || resp.status().is_client_error() {
            Err(anyhow!("server or client error"))?
        }
        if resp.status().is_success() {
            info!("succesfully created peer in airtable");
        }
        Ok(())
    }

    pub async fn update_peer() -> Result<(), anyhow::Error> {
        Ok(())
    }
}

/// Trait
#[rpc(server, client)]
pub trait TransactionRpc {
    /// register user profile, generate node peer id and push the profile for vane discovery server
    /// params: `name`,`vec![(address, networkId)]`
    #[method(name = "register")]
    async fn register_vane_web3(
        &self,
        name: Vec<u8>,
        account_id: Vec<u8>,
        network: ChainSupported,
    ) -> RpcResult<()>;

    /// add crypto address account
    /// params: `vec![(address, networkId)]`
    #[method(name = "addAccount")]
    async fn add_account(&self, name: Vec<u8>, accounts: Vec<(Vec<u8>, Vec<u8>)>) -> RpcResult<()>;

    /// initiate tx to be verified recv address and network choice
    /// params: `sender address`,`receiver_address`, `amount`, `Optional networkId`
    #[method(name = "sendTx")]
    async fn initiate_transaction(
        &self,
        sender: String,
        receiver: String,
        amount: u128,
        token: Token,
        network: ChainSupported,
    ) -> RpcResult<()>;

    /// confirm sender signifying agreeing all tx state after verification and this will trigger actual submission
    #[method(name = "senderConfirm")]
    async fn sender_confirm(&self, tx: TxStateMachine) -> RpcResult<()>;

    /// watch tx update stream
    #[subscription(name ="subscribeTxUpdates",item = TxStateMachine )]
    async fn watch_tx_update(&self) -> SubscriptionResult;

    /// receiver confirmation on address and ownership of account ( network ) signifying correct token to the network choice
    #[method(name = "recvConfirm")]
    async fn receiver_confirm(&self, tx: TxStateMachine) -> RpcResult<()>;
}

/// handling tx submission & tx confirmation & tx simulation interactions
/// a first layer a user interact with and submits the tx to processing layer
pub struct TransactionRpcWorker {
    pub db_worker: Arc<Mutex<DbWorker>>,
    /// central server to get peer data
    pub airtable_client: Arc<Mutex<Airtable>>,
    /// peer url for p2p protocol
    pub url: String,
    /// receiving end of tx , updating state of tx to end user
    pub receiver_channel: Arc<Mutex<Receiver<Arc<Mutex<TxStateMachine>>>>>,
    /// sender channel to send self sending tx-state-machine
    pub sender_channel: Mutex<Sender<Arc<Mutex<TxStateMachine>>>>,
}

impl TransactionRpcWorker {
    pub async fn new(
        recv_channel: Arc<Mutex<Receiver<Arc<Mutex<TxStateMachine>>>>>,
        sender_channel: Sender<Arc<Mutex<TxStateMachine>>>,
    ) -> Result<Self, anyhow::Error> {
        // fetch to the db, if not then set one
        let port = Xorshift::default().next_lim_u16(u16::MAX - 100);
        let airtable_client = Airtable::new().await?;
        let db_worker = DbWorker::initialize_db_client("./../db/dev.db").await?;
        let url = format!("ip4/127.0.0.1:{}", port);
        Ok(Self {
            db_worker: Arc::new(Mutex::new(db_worker)),
            airtable_client: Arc::new(Mutex::new(airtable_client)),
            url,
            receiver_channel: recv_channel,
            sender_channel: Mutex::new(sender_channel),
        })
    }

    /// first dry tx, returns the projected fees
    pub async fn dry_run_tx(
        network: ChainSupported,
        sender: String,
        recv: String,
        token: Token,
        amount: u64,
    ) -> Result<u64, anyhow::Error> {
        let fees = match network {
            ChainSupported::Polkadot => {
                todo!()
            }
            ChainSupported::Ethereum => {
                todo!()
            }
            ChainSupported::Bnb => {
                todo!()
            }
            ChainSupported::Solana => {
                todo!()
            }
        };
        Ok(fees)
    }
}

#[async_trait]
impl TransactionRpcServer for TransactionRpcWorker {
    async fn register_vane_web3(
        &self,
        name: Vec<u8>,
        account_id: Vec<u8>,
        network: ChainSupported,
    ) -> RpcResult<()> {
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
        let self_peer_id = libp2p::identity::Keypair::generate_ed25519();
        let peer_account = PeerRecord {
            peer_address: self_peer_id
                .public()
                .to_peer_id()
                .to_base58()
                .as_bytes()
                .to_vec(),
            accountId1: Some(account_id),
            accountId2: None,
            accountId3: None,
            accountId4: None,
            multi_addr: self.url.to_string().as_bytes().to_vec(),
            keypair: Some(
                self_peer_id
                    .to_protobuf_encoding()
                    .map_err(|_| anyhow!("failed to encode keypair"))?,
            ),
        };
        self.db_worker
            .lock()
            .await
            .update_user_peerId_accounts(peer_account)
            .await?;

        // let field: Fields = peer_account.clone().into();
        //
        // self.airtable_client.lock().await.create_peer(field).await?;

        Ok(())
    }

    async fn add_account(&self, name: Vec<u8>, accounts: Vec<(Vec<u8>, Vec<u8>)>) -> RpcResult<()> {
        todo!()
    }

    async fn initiate_transaction(
        &self,
        sender: String,
        receiver: String,
        amount: u128,
        token: Token,
        network: ChainSupported,
    ) -> RpcResult<()> {
        if let (Ok(net_sender), Ok(net_recv)) = (
            verify_public_bytes(sender.as_str(), token, network),
            verify_public_bytes(receiver.as_str(), token, network),
        ) {
            if net_sender != net_recv {
                Err(anyhow!("sender and receiver should be same network"))?
            }
            // construct the tx
            let mut sender_recv = sender.as_bytes().to_vec();
            sender_recv.extend_from_slice(receiver.as_bytes());
            let multi_addr = Blake2Hasher::hash(&sender_recv[..]);

            let tx_state_machine = TxStateMachine {
                sender_address: sender.as_bytes().to_vec(),
                receiver_address: receiver.as_bytes().to_vec(),
                multi_id: multi_addr,
                signature: None,
                network: net_sender,
                status: TxStatus::default(),
                amount,
                signed_call_payload: None,
                call_payload: None,
            };

            // dry run the tx

            //let fees = self::dry_run_tx().map_err(|err|anyhow!("{}",err))?;

            // propagate the tx to lower layer
            let sender_channel = self.sender_channel.lock().await;

            let sender = sender_channel.clone();
            sender
                .send(Arc::from(Mutex::new(tx_state_machine)))
                .await
                .map_err(|_| anyhow!("failed to send initial tx state to sender channel"))?;
        } else {
            Err(anyhow!(
                "sender and receiver should be correct accounts for the specified token"
            ))?
        }
        Ok(())
    }

    async fn sender_confirm(&self, tx: TxStateMachine) -> RpcResult<()> {
        let sender_channel = self.sender_channel.lock().await;

        let sender = sender_channel.clone();
        sender.send(Arc::from(Mutex::new(tx))).await.map_err(|_| {
            anyhow!("failed to send sender confirmation tx state to sender-channel")
        })?;
        Ok(())
    }

    async fn receiver_confirm(&self, tx: TxStateMachine) -> RpcResult<()> {
        let sender_channel = self.sender_channel.lock().await;

        let sender = sender_channel.clone();
        sender
            .send(Arc::from(Mutex::new(tx)))
            .await
            .map_err(|_| anyhow!("failed to send recv confirmation tx state to sender channel"))?;
        Ok(())
    }

    async fn watch_tx_update(
        &self,
        subscription_sink: PendingSubscriptionSink,
    ) -> SubscriptionResult {
        let sink = subscription_sink
            .accept()
            .await
            .map_err(|_| anyhow!("failed to accept rpc ws channel"))?;
        while let Some(tx_update) = self.receiver_channel.lock().await.recv().await {
            let tx: TxStateMachine = tx_update.lock().await.clone();

            let subscription_msg = SubscriptionMessage::from_json(&tx)
                .map_err(|_| anyhow!("failed to convert tx update to json"))?;
            sink.send(subscription_msg)
                .await
                .map_err(|_| anyhow!("failed to send msg to rpc ws channel"))?;
        }
        Ok(())
    }
}
