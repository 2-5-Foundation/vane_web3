extern crate alloc;

use alloc::{collections::BTreeMap, rc::Rc, string::ToString, sync::Arc, vec::Vec};
use core::{cell::RefCell, str::FromStr};

use anyhow::anyhow;
use log::{error, info};
use serde_wasm_bindgen::{from_value, to_value};
use wasm_bindgen::prelude::*;
use web3::{transports, Web3};

use alloy::primitives::{Address, Signature as EcdsaSignature, SignatureError, B256};
use primitives::data_structure::{ChainSupported, TxStateMachine, ETH_SIG_MSG_PREFIX};
use sp_core::{
    blake2_256, ecdsa as EthSignature,
    ed25519::{Public as EdPublic, Signature as EdSignature},
    keccak_256, ByteArray, H256,
};
use sp_runtime::traits::Verify;
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(catch, js_namespace = ["hostFunctions", "hostNetworking"], js_name = submitTx)]
    async fn submit_tx_js(tx: JsValue) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(catch, js_namespace = ["hostFunctions", "hostNetworking"], js_name = createTx)]
    async fn create_tx_js(tx: JsValue) -> Result<JsValue, JsValue>;

}

#[derive(Clone)]
pub struct WasmTxProcessingWorker {
    /// In-memory Db for tx processing at any stage
    tx_staging: Rc<RefCell<BTreeMap<H256, TxStateMachine>>>,
    /// In-memory Db for to be confirmed tx on sender
    pub sender_tx_pending: Rc<RefCell<Vec<TxStateMachine>>>,
    /// In-memory Db for to be confirmed tx on receiver
    pub receiver_tx_pending: Rc<RefCell<Vec<TxStateMachine>>>,
    // /// substrate client
    // sub_client: OnlineClient<PolkadotConfig>,
    //// ethereum & bnb client
    // eth_client: ReqwestProvider,
    // bnb_client: ReqwestProvider,
    // solana_client: RpcClient
}

impl WasmTxProcessingWorker {
    pub fn new(
        chain_networks: (ChainSupported, ChainSupported, ChainSupported),
    ) -> Result<Self, anyhow::Error> {
        let (_solana, eth, bnb) = chain_networks;
        let eth_url = eth.url();
        let bnb_url = bnb.url().to_string();

        // let eth_rpc_url = eth_url
        //     .parse()
        //     .map_err(|err| anyhow!("eth rpc parse error: {err}"))?;
        // // Create a provider with the HTTP transport using the `reqwest` crate.
        // let eth_provider = ProviderBuilder::new().on_http(eth_rpc_url);
        //
        // let bnb_rpc_url = bnb_url
        //     .parse()
        //     .map_err(|err| anyhow!("bnb rpc url parse error: {err}"))?;
        // let bnb_provider = ProviderBuilder::new().on_http(bnb_rpc_url);

        Ok(Self {
            tx_staging: Rc::new(RefCell::new(Default::default())),
            sender_tx_pending: Rc::new(RefCell::new(Default::default())),
            receiver_tx_pending: Rc::new(RefCell::new(Default::default())),
            // eth_client: eth_provider,
            // bnb_client: bnb_provider,
        })
    }

    pub fn validate_receiver_and_sender_address(
        &self,
        tx: &TxStateMachine,
        who: &str,
    ) -> Result<(), anyhow::Error> {
        let (network, signature, msg, address) = if who == "Receiver" {
            info!(target: "WasmTxProcessingWorker", "receiver address verification");

            let network = tx.receiver_address_network;
            let signature = tx
                .clone()
                .recv_signature
                .ok_or(anyhow!("receiver didnt signed"))?;

            let recv_address = tx.receiver_address.clone();
            let msg = tx.receiver_address.as_bytes().to_vec();

            (network, signature, msg, recv_address)
        } else {
            info!(target: "WasmTxProcessingWorker", "sender address verification");
            // who == Sender
            let network = tx.sender_address_network;
            let signature = tx
                .clone()
                .signed_call_payload
                .ok_or(anyhow!("original sender didnt signed"))?;

            // TODO: how to handle this panic
            let (msg_hash, _raw_tx) = tx
                .call_payload
                .as_ref()
                .expect("unexpected error, call payload should be available");
            let sender_address = tx.sender_address.clone();

            (network, signature, msg_hash.to_vec(), sender_address)
        };
        match network {
            ChainSupported::Ethereum => {
                let address: alloy::primitives::Address = address.parse().expect("Invalid address");

                let hashed_msg = {
                    if who == "Receiver" {
                        let mut signable_msg = Vec::<u8>::new();
                        signable_msg.extend_from_slice(ETH_SIG_MSG_PREFIX.as_bytes());
                        signable_msg.extend_from_slice(msg.len().to_string().as_bytes());
                        signable_msg.extend_from_slice(msg.as_slice());

                        keccak_256(signable_msg.as_slice())
                    } else {
                        //TODO: handle this panic as system shutdown or error reporting
                        msg.try_into().unwrap()
                    }
                };
                let signature = EcdsaSignature::try_from(signature.as_slice())
                    .map_err(|err| anyhow!("failed to convert ecdsa signature: {:?}", err))?;

                match signature.recover_address_from_prehash(<&B256>::from(&hashed_msg)) {
                    Ok(recovered_addr) => {
                        info!(
                            "recovered addr: {recovered_addr:?} == address: {address:?} ==== {:?}",
                            tx.status
                        );
                        if recovered_addr == address {
                            Ok::<(), anyhow::Error>(())?
                        } else {
                            Err(anyhow!(
                                "addr recovery equality failed hence account invalid"
                            ))?
                        }
                    }
                    Err(err) => Err(anyhow!("ec signature verification failed: {err}"))?,
                }
            }
            _ => unreachable!(),
        }
        Ok(())
    }

    pub fn validate_multi_id(&self, txn: &TxStateMachine) -> bool {
        let post_multi_id = {
            let mut sender_recv = txn.sender_address.as_bytes().to_vec();
            sender_recv.extend_from_slice(txn.receiver_address.as_bytes());
            blake2_256(&sender_recv[..])
        };

        post_multi_id == txn.multi_id
    }

    pub async fn submit_tx(&mut self, tx: TxStateMachine) -> Result<[u8; 32], anyhow::Error> {
        let tx_hash_result = unsafe {
            let tx_value =
                to_value(&tx).map_err(|e| anyhow!("failed to convert tx to js value: {:?}", e))?;
            submit_tx_js(tx_value).await
        };

        match tx_hash_result {
            Ok(res) => {
                let tx_hash = from_value::<[u8; 32]>(res)
                    .map_err(|e| anyhow!("failed to convert tx hash to bytes: {:?}", e))?;
                Ok(tx_hash)
            }
            Err(js_err) => {
                error!("submitTx JS error: {:?}", js_err);
                Err(anyhow!("submitTx failed: {:?}", js_err))
            }
        }
    }

    pub async fn create_tx(&mut self, tx: &mut TxStateMachine) -> Result<(), anyhow::Error> {
        let unsigned_tx_result = unsafe {
            let tx_value =
                to_value(&tx).map_err(|e| anyhow!("failed to convert tx to js value: {:?}", e))?;
            create_tx_js(tx_value).await
        };

        match unsigned_tx_result {
            Ok(res) => {
                // JS now returns the full TxStateMachine with callPayload set
                let updated_tx = from_value::<TxStateMachine>(res)
                    .map_err(|e| anyhow!("failed to convert JS TxStateMachine: {:?}", e))?;
                *tx = updated_tx;
                Ok(())
            }
            Err(js_err) => {
                error!("createTx JS error: {:?}", js_err);
                Err(anyhow!("createTx failed: {:?}", js_err))
            }
        }
    }
}
