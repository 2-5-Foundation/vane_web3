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
use log::{debug, error, info, trace};
use lru::LruCache;
use reqwasm::http::{Request, RequestMode};
use serde_wasm_bindgen;
use sp_core::blake2_256;
use sp_runtime::{format, traits::Zero};
use tokio_with_wasm::alias::sync::{
    mpsc::{Receiver, Sender},
    {Mutex, MutexGuard},
};
use wasm_bindgen::prelude::*;

use crate::{
    cryptography::verify_public_bytes,
    p2p::{P2pNetworkService, WasmP2pWorker},
};

use primitives::data_structure::{
    AccountInfo, ChainSupported, DbTxStateMachine, DbWorkerInterface, Token, TxStateMachine,
    TxStatus, UserAccount, UserMetrics, SavedPeerInfo, StorageExport,
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
    pub lru_cache: Rc<RefCell<LruCache<u64, TxStateMachine>>>, // initial fees, after dry running tx initialy without optimization
    /// Flag to track if a watcher is already active to prevent multiple concurrent watchers
    pub watcher_active: Rc<RefCell<bool>>,
    /// List of registered callbacks for transaction updates
    pub tx_callbacks: Rc<RefCell<Vec<js_sys::Function>>>,
}

impl PublicInterfaceWorker {
    pub async fn new(
        db_worker: Rc<DbWorker>,
        p2p_worker: Rc<WasmP2pWorker>,
        p2p_network_service: Rc<P2pNetworkService>,
        rpc_recv_channel: Rc<RefCell<Receiver<TxStateMachine>>>,
        user_rpc_update_sender_channel: Rc<RefCell<Sender<TxStateMachine>>>,
        peer_id: PeerId,
        lru_cache: Rc<RefCell<LruCache<u64, TxStateMachine>>>,
    ) -> Result<Self, JsValue> {
        Ok(Self {
            db_worker,
            p2p_worker,
            p2p_network_service,
            rpc_receiver_channel: rpc_recv_channel,
            user_rpc_update_sender_channel,
            peer_id,
            lru_cache,
            watcher_active: Rc::new(RefCell::new(false)),
            tx_callbacks: Rc::new(RefCell::new(Vec::new())),
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

            // failure to this should stop the node and not continue
            let nonce = self
                .db_worker
                .get_nonce()
                .await
                .map_err(|e| JsError::new(&format!("{:?}", e)))?
                + 1;
            // update the db on nonce
            // failure to this should stop the node and not continue
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
                eth_unsigned_tx_fields: None,
            };

            // dry run the tx

            // propagate the tx to lower layer (Main service worker layer)
            let sender_channel = self.user_rpc_update_sender_channel.borrow_mut();

            let sender = sender_channel.clone();
            sender
                .send(tx_state_machine.clone())
                .await
                .map_err(|_| anyhow!("failed to send initial tx state to sender channel"))
                .map_err(|e| JsError::new(&format!("{:?}", e)))?;
            info!("propagated initiated transaction to tx handling layer")
        } else {
            error!("sender and receiver should be correct accounts for the specified token: sender:{:?}, receiver:{:?}, token:{:?}, network:{:?}", sender, receiver, token, network);  
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

    /// Register a callback for transaction updates. Multiple callbacks can be registered.
    /// The background watcher will be started automatically if it's not already running.
    pub async fn watch_tx_updates(&self, callback: &js_sys::Function) -> Result<(), JsError> {
        let callback = callback.clone();
        
        // Add the callback to our list
        self.tx_callbacks.borrow_mut().push(callback);
        
        // Start the background watcher if it's not already running
        if !*self.watcher_active.borrow() {
            self.start_background_watcher().await?;
        }
        
        Ok(())
    }

    /// Start the background watcher task (internal method)
    async fn start_background_watcher(&self) -> Result<(), JsError> {
        if *self.watcher_active.borrow() {
            return Ok(()); // Already running
        }

        // Mark watcher as active
        *self.watcher_active.borrow_mut() = true;

        let receiver_channel = self.rpc_receiver_channel.clone();
        let callbacks = self.tx_callbacks.clone();
        let watcher_active = self.watcher_active.clone();
        
        // Spawn a single background task to continuously poll for transaction updates
        wasm_bindgen_futures::spawn_local(async move {
            let mut receiver = receiver_channel.borrow_mut();
            
            loop {
                match receiver.recv().await {
                    Some(tx_update) => {
                        debug!("watch_tx_updates: {:?}", tx_update);
                        
                        // Convert transaction to JS value
                        let tx_js_value = serde_wasm_bindgen::to_value(&tx_update)
                            .unwrap_or_else(|_| JsValue::NULL);
                        
                        // Call all registered callbacks
                        let callbacks_guard = callbacks.borrow();
                        for callback in callbacks_guard.iter() {
                            let _ = callback.call1(&wasm_bindgen::JsValue::UNDEFINED, &tx_js_value);
                        }
                    }
                    None => {
                        info!("Transaction update channel closed");
                        // Mark watcher as inactive when done
                        *watcher_active.borrow_mut() = false;
                        break;
                    }
                }
            }
        });
        
        Ok(())
    }

    /// Unsubscribe from all transaction updates (stops watcher and clears all callbacks)
    pub fn unsubscribe_watch_tx_updates(&self) {
        *self.watcher_active.borrow_mut() = false;
        self.tx_callbacks.borrow_mut().clear();
        info!("Unsubscribed from all transaction updates");
    }

    pub async fn fetch_pending_tx_updates(&self) -> Result<JsValue, JsError> {
        let tx_updates = self
            .lru_cache
            .borrow()
            .iter()
            .map(|(_k, v)| v.clone())
            .collect::<Vec<TxStateMachine>>();
        debug!("lru: {tx_updates:?}");

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

    pub async fn revert_transaction(&self, tx: JsValue) -> Result<(), JsError> {
        let mut tx: TxStateMachine = TxStateMachine::from_js_value_unconditional(tx)?;
        
        match tx.status {
            TxStatus::Reverted(ref reason) => {
                let sender = self.user_rpc_update_sender_channel.borrow_mut();
                sender.send(tx).await.map_err(|_| anyhow!("failed to send revert transaction tx state to sender channel")).map_err(|e| JsError::new(&format!("{:?}", e)))?;
                Ok(())            }
            _ => {
                Err(anyhow!("Transaction is not in reverted state")).map_err(|e| JsError::new(&format!("{:?}", e)))
            }
        }
    }

    // export all storage
    pub async fn export_storage(&self) -> Result<JsValue, JsError> {
        // Call all getter methods and collect data
        let user_account = self.db_worker.get_user_account().await.ok();
        let nonce = self.db_worker.get_nonce().await.unwrap_or(0);
        let success_transactions = self.db_worker.get_success_txs().await.unwrap_or_default();
        let failed_transactions = self.db_worker.get_failed_txs().await.unwrap_or_default();
        let total_value_success = self.db_worker.get_total_value_success().await.unwrap_or(0);
        let total_value_failed = self.db_worker.get_total_value_failed().await.unwrap_or(0);
        
        // Get saved peers and convert to SavedPeerInfo format
        let all_saved_peers = if let Ok((account_ids, peer_multiaddr)) = self.db_worker.get_all_saved_peers().await {
            vec![SavedPeerInfo {
                peer_id: peer_multiaddr,
                account_ids,
            }]
        } else {
            Vec::new()
        };
        
        // Create the export structure
        let storage_export = StorageExport {
            user_account,
            nonce,
            success_transactions,
            failed_transactions,
            total_value_success,
            total_value_failed,
            all_saved_peers,
        };
        
        // Convert to JSON and return as JsValue
        serde_wasm_bindgen::to_value(&storage_export)
            .map_err(|e| JsError::new(&format!("Failed to serialize storage data: {}", e)))
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
    pub async fn watch_tx_updates(&self, callback: &js_sys::Function) -> Result<(), JsError> {
        self.inner.borrow().watch_tx_updates(callback).await?;
        Ok(())
    }

    #[wasm_bindgen(js_name = "unsubscribeWatchTxUpdates")]
    pub fn unsubscribe_watch_tx_updates(&self) {
        self.inner.borrow().unsubscribe_watch_tx_updates();
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

    #[wasm_bindgen(js_name = "revertTransaction")]
    pub async fn revert_transaction(&self, tx: JsValue) -> Result<(), JsError> {
        self.inner.borrow().revert_transaction(tx).await?;
        Ok(())
    }

    #[wasm_bindgen(js_name = "exportStorage")]
    pub async fn export_storage(&self) -> Result<JsValue, JsError> {
        let storage_export = self.inner.borrow().export_storage().await?;
        Ok(storage_export)
    }

    #[wasm_bindgen(js_name = "getMetrics")]
    pub async fn get_metrics(&self) -> Result<JsValue, JsError> {
        let metrics = self.inner.borrow().get_user_metrics().await?;
        Ok(metrics)
    }
}