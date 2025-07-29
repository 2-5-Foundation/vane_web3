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

use anyhow::anyhow;
use core::str::FromStr;
use jsonrpsee::core::JsonValue;

use log::{info, trace};

use crate::cryptography::verify_public_bytes;
use primitives::data_structure::{
    AccountInfo, ChainSupported, Discovery, PeerRecord, Token, TxStateMachine, TxStatus,
    UserAccount,
};
use sp_runtime::traits::Zero;

use primitives::data_structure::{DbTxStateMachine, DbWorkerInterface};


pub use std_imports::*;

mod std_imports {
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
    pub use redis::Client as RedisClient;
    pub use reqwest::{ClientBuilder, Url};
    pub use sp_core::Blake2Hasher;
    pub use sp_core::Hasher;
    pub use tokio::sync::mpsc::{Receiver, Sender};
    pub use tokio::sync::{Mutex, MutexGuard};
}

/// Trait
#[rpc(server, client)]
pub trait TransactionRpc {
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

    /// Allowing the sender to revery/cancel the transaction
    /// params:
    ///
    /// - `tx_id`
    #[method(name = "revertTransaction")]
    async fn revert_transaction(&self, tx_state: TxStateMachine) -> RpcResult<()>;
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
#[derive(Clone)]
pub struct TransactionRpcWorker {
    /// local database worker
    pub db_worker: Arc<Mutex<LocalDbWorker>>,
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
    pub moka_cache: AsyncCache<u64, TxStateMachine>,
    // initial fees, after dry running tx initialy without optimization
}

impl TransactionRpcWorker {
    pub async fn new(
        redis_client: RedisClient,
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

    async fn revert_transaction(&self, tx_state: TxStateMachine) -> RpcResult<()> {
        // remove from cache
        if let None = self.moka_cache.remove(&tx_state.tx_nonce.into()).await {
            Err(anyhow!("tx not found in cache"))?
        }
        // update the db
        let tx_hash = tx_state.get_tx_hash().to_vec();
        let tx_db = DbTxStateMachine {
            tx_hash: tx_hash,
            amount: tx_state.amount,
            network: tx_state.network,
            success: false,
        };
        self.db_worker.lock().await.update_failed_tx(tx_db).await?;

        let sender_channel = self.user_rpc_update_sender_channel.lock().await;
        let sender = sender_channel.clone();
        sender
            .send(Arc::from(Mutex::new(tx_state)))
            .await
            .map_err(|_| {
                anyhow!("failed to send sender confirmation tx state to sender-channel")
            })?;

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
