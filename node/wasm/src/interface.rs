extern crate alloc;

use alloc::{
    rc::Rc,
    string::{String, ToString},
    vec::Vec,
};
use core::{cell::RefCell, fmt, str::FromStr};

use anyhow::anyhow;
use async_stream::stream;
use db_wasm::{DbWorker, OpfsRedbWorker};
use futures::StreamExt;
use libp2p::{Multiaddr, PeerId};
use log::{info, trace};
use lru::LruCache;
use reqwasm::http::{Request, RequestMode};
use serde_wasm_bindgen;
use sp_core::blake2_256;
use sp_runtime::{format, traits::Zero};
use tokio_with_wasm::alias::sync::{
    mpsc::{Receiver, Sender},
    {Mutex, MutexGuard},
};
use wasm_bindgen::{prelude::wasm_bindgen, JsError, JsValue};

use crate::{
    cryptography::verify_public_bytes,
    p2p::{P2pNetworkService, WasmP2pWorker},
};

use primitives::data_structure::{
    AccountInfo, ChainSupported, DbTxStateMachine, DbWorkerInterface, Token, TxStateMachine,
    TxStatus, UserAccount, UserMetrics,
};

#[derive(Clone)]
pub struct PublicInterfaceWorker {
    /// local database worker
    pub db_worker: Rc<DbWorker>,
    // p2pworker
    pub p2p_worker: Rc<WasmP2pWorker>,
    /// p2p network service
    pub p2p_network_service: Rc<P2pNetworkService>,
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

impl PublicInterfaceWorker {
    pub async fn new(
        db_worker: Rc<DbWorker>,
        p2p_worker: Rc<WasmP2pWorker>,
        p2p_network_service: Rc<P2pNetworkService>,
        rpc_recv_channel: Rc<RefCell<Receiver<TxStateMachine>>>,
        user_rpc_update_sender_channel: Rc<RefCell<Sender<TxStateMachine>>>,
        peer_id: PeerId,
        lru_cache: LruCache<u64, TxStateMachine>,
    ) -> Result<Self, JsValue> {
        Ok(Self {
            db_worker,
            p2p_worker,
            p2p_network_service,
            rpc_receiver_channel: rpc_recv_channel,
            user_rpc_update_sender_channel,
            peer_id,
            lru_cache: RefCell::new(lru_cache),
        })
    }
}

impl PublicInterfaceWorker {
    pub async fn add_account(&self, account_id: String, network: String) -> Result<(), JsError> {
        let network = network.as_str().into();

        let user_account = self
            .db_worker
            .update_user_account(account_id.clone(), network)
            .await
            .map_err(|e| JsError::new(&format!("{:?}", e)))?;

        self.p2p_network_service
            .add_account_to_dht(account_id, user_account.multi_addr)
            .await
            .map_err(|e| JsError::new(&format!("{:?}", e)))?;

        Ok(())
    }

    pub async fn initiate_transaction(
        &self,
        sender: String,
        receiver: String,
        amount: u128,
        token: String,
        network: String,
        code_word: String,
    ) -> Result<(), JsError> {
        info!("initiated sending transaction");
        let token = token.as_str().into();

        let network = network.as_str().into();
        if let (Ok(net_sender), Ok(net_recv)) = (
            verify_public_bytes(sender.as_str(), token, network),
            verify_public_bytes(receiver.as_str(), token, network),
        ) {
            if net_sender != net_recv {
                Err(anyhow!("sender and receiver should be same network"))
                    .map_err(|e| JsError::new(&format!("{:?}", e)))?;
            }

            info!("successfully initially verified sender and receiver and related network bytes");
            // construct the tx
            let mut sender_recv = sender.as_bytes().to_vec();
            sender_recv.extend_from_slice(receiver.as_bytes());
            let multi_addr = blake2_256(&sender_recv[..]);

            let nonce = self
                .db_worker
                .get_nonce()
                .await
                .map_err(|e| JsError::new(&format!("{:?}", e)))?
                + 1;
            // update the db on nonce
            self.db_worker
                .increment_nonce()
                .await
                .map_err(|e| JsError::new(&format!("{:?}", e)))?;

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
                token,
                code_word,
            };

            // dry run the tx

            // propagate the tx to lower layer (Main service worker layer)
            let sender_channel = self.user_rpc_update_sender_channel.borrow_mut();

            let sender = sender_channel.clone();
            sender
                .send(tx_state_machine)
                .await
                .map_err(|_| anyhow!("failed to send initial tx state to sender channel"))
                .map_err(|e| JsError::new(&format!("{:?}", e)))?;
            info!("propagated initiated transaction to tx handling layer")
        } else {
            Err(anyhow!(
                "sender and receiver should be correct accounts for the specified token"
            ))
            .map_err(|e| JsError::new(&format!("{:?}", e)))?;
        }
        Ok(())
    }

    pub async fn sender_confirm(&self, tx: JsValue) -> Result<(), JsError> {
        let mut tx: TxStateMachine = TxStateMachine::from_js_value_unconditional(tx)?;
        let sender_channel = self.user_rpc_update_sender_channel.borrow_mut();
        if tx.signed_call_payload.is_none() && tx.status != TxStatus::RecvAddrConfirmationPassed {
            // return error as receiver hasnt confirmed yet or sender hasnt confirmed on his turn
            Err(anyhow!(
                "Wait for Receiver to confirm or sender should confirm".to_string(),
            ))
            .map_err(|e| JsError::new(&format!("{:?}", e)))?;
        } else {
            // remove from cache
            self.lru_cache.borrow_mut().demote(&tx.tx_nonce.into());
            // verify the tx-state-machine integrity
            // TODO
            // update the TxStatus to TxStatus::SenderConfirmed
            tx.sender_confirmation();
            let sender = sender_channel.clone();
            sender
                .send(tx)
                .await
                .map_err(|_| {
                    anyhow!("failed to send sender confirmation tx state to sender-channel")
                })
                .map_err(|e| JsError::new(&format!("{:?}", e)))?;
        }
        Ok(())
    }

    pub async fn watch_tx_updates(&self) -> Result<(), JsError> {
        Ok(())
    }

    pub async fn fetch_pending_tx_updates(&self) -> Result<JsValue, JsError> {
        let tx_updates = self
            .lru_cache
            .borrow()
            .iter()
            .map(|(_k, v)| v.clone())
            .collect::<Vec<TxStateMachine>>();
        info!("lru: {tx_updates:?}");

        serde_wasm_bindgen::to_value(&tx_updates)
            .map_err(|e| JsError::new(&format!("Serialization error: {:?}", e)))
    }

    pub async fn receiver_confirm(&self, tx: JsValue) -> Result<(), JsError> {
        let mut tx: TxStateMachine = TxStateMachine::from_js_value_unconditional(tx)?;
        let sender_channel = self.user_rpc_update_sender_channel.borrow_mut();
        if tx.recv_signature.is_none() {
            // return error as we do not accept any other TxStatus at this api and the receiver should have signed for confirmation
            Err(anyhow!("Receiver did not confirm".to_string()))
                .map_err(|e| JsError::new(&format!("{:?}", e)))?
        } else {
            // remove from cache
            self.lru_cache.borrow_mut().demote(&tx.tx_nonce.into());
            // verify the tx-state-machine integrity
            // TODO
            // tx status to TxStatus::RecvAddrConfirmed
            tx.recv_confirmed();
            let sender = sender_channel.clone();
            sender
                .send(tx)
                .await
                .map_err(|_| anyhow!("failed to send recv confirmation tx state to sender channel"))
                .map_err(|e| JsError::new(&format!("{:?}", e)))?;

            Ok(())
        }
    }

    // user metrics
    pub async fn get_user_metrics(&self) -> Result<JsValue, JsError> {
        let user_account = self
            .db_worker
            .get_user_account()
            .await
            .map_err(|e| JsError::new(&format!("{:?}", e)))?;
        let total_success_txns = self
            .db_worker
            .get_success_txs()
            .await
            .map_err(|e| JsError::new(&format!("{:?}", e)))?;
        let total_failed_txns = self
            .db_worker
            .get_failed_txs()
            .await
            .map_err(|e| JsError::new(&format!("{:?}", e)))?;
        let saved_target_peers = self
            .db_worker
            .get_all_saved_peers()
            .await
            .map_err(|e| JsError::new(&format!("{:?}", e)))?;

        let user_metrics = UserMetrics {
            user_account,
            total_success_txns,
            total_failed_txns,
            saved_target_peers,
        };
        serde_wasm_bindgen::to_value(&user_metrics)
            .map_err(|e| JsError::new(&format!("Serialization error: {:?}", e)))
    }
}

// =================== The interface =================== //

#[wasm_bindgen]
pub struct PublicInterfaceWorkerJs {
    inner: Rc<RefCell<PublicInterfaceWorker>>,
}

impl PublicInterfaceWorkerJs {
    pub fn new(inner: Rc<RefCell<PublicInterfaceWorker>>) -> Self {
        Self { inner }
    }
}

#[wasm_bindgen]
impl PublicInterfaceWorkerJs {
    #[wasm_bindgen(js_name = "addAccount")]
    pub async fn add_account(&self, account_id: String, network: String) -> Result<(), JsError> {
        self.inner.borrow().add_account(account_id, network).await?;
        Ok(())
    }

    #[wasm_bindgen(js_name = "initiateTransaction")]
    pub async fn initiate_transaction(
        &self,
        sender: String,
        receiver: String,
        amount: u128,
        token: String,
        network: String,
        code_word: String,
    ) -> Result<(), JsError> {
        self.inner
            .borrow()
            .initiate_transaction(sender, receiver, amount, token, network, code_word)
            .await?;
        Ok(())
    }

    #[wasm_bindgen(js_name = "senderConfirm")]
    pub async fn sender_confirm(&self, tx: JsValue) -> Result<(), JsError> {
        self.inner.borrow().sender_confirm(tx).await?;
        Ok(())
    }

    #[wasm_bindgen(js_name = "watchTxUpdates")]
    pub async fn watch_tx_updates(&self) -> Result<(), JsError> {
        self.inner.borrow().watch_tx_updates().await?;
        Ok(())
    }

    #[wasm_bindgen(js_name = "fetchPendingTxUpdates")]
    pub async fn fetch_pending_tx_updates(&self) -> Result<JsValue, JsError> {
        let tx_updates = self.inner.borrow().fetch_pending_tx_updates().await?;
        Ok(tx_updates)
    }

    #[wasm_bindgen(js_name = "receiverConfirm")]
    pub async fn receiver_confirm(&self, tx: JsValue) -> Result<(), JsError> {
        self.inner.borrow().receiver_confirm(tx).await?;
        Ok(())
    }

    #[wasm_bindgen(js_name = "getMetrics")]
    pub async fn get_metrics(&self) -> Result<JsValue, JsError> {
        let metrics = self.inner.borrow().get_user_metrics().await?;
        Ok(metrics)
    }
}
