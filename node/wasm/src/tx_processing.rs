extern crate alloc;

use alloc::{collections::BTreeMap, rc::Rc, string::ToString, sync::Arc, vec::Vec};
use core::{cell::RefCell, str::FromStr};
use std::fmt::format;

use anyhow::anyhow;
use base58ck;
use log::{error, info};
use serde_wasm_bindgen::{from_value, to_value};
use wasm_bindgen::prelude::*;
use web3::{transports, Web3};

use alloy::primitives::{Address, Signature as EcdsaSignature, SignatureError, B256};
use k256::elliptic_curve::sec1::FromEncodedPoint;
use primitives::data_structure::{ChainSupported, ChainTransactionType, TxStateMachine, ETH_SIG_MSG_PREFIX};
use sp_core::{
    blake2_256, ecdsa as EthSignature,
    keccak_256, ByteArray, H256,
};
use base58::FromBase58;
use base58::ToBase58;

use ed25519_compact::{PublicKey as Ed25519PublicKey,Signature as Ed25519Signature};
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
    pub fn new() -> Result<Self, anyhow::Error> {
        Ok(Self {
            tx_staging: Rc::new(RefCell::new(Default::default())),
            sender_tx_pending: Rc::new(RefCell::new(Default::default())),
            receiver_tx_pending: Rc::new(RefCell::new(Default::default())),
        })
    }

    pub fn validate_receiver_and_sender_address(
        &self,
        tx: &TxStateMachine,
        who: &str,
    ) -> Result<(), anyhow::Error> {
        let (network, signature,msg, address) = if who == "Receiver" {
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

            // Extract appropriate message based on chain type
            // handle error worker should handle this panic
            let msg = match tx
                .call_payload
                .as_ref()
                .expect("unexpected error, call payload should be available")
            {
                ChainTransactionType::Ethereum { call_payload, .. } => {
                    // Ethereum needs the hash (first element of tuple)
                    call_payload.0.clone()
                }
                ChainTransactionType::Bnb { call_payload, .. } => {
                    // BNB needs the hash (first element of tuple)
                    call_payload.0.clone()
                }
                ChainTransactionType::Solana { call_payload, .. } => {
                    // Solana needs the raw transaction
                    call_payload.clone()
                }
            };
            let sender_address = tx.sender_address.clone();

            (network, signature, msg, sender_address)
        };
        match network {
            ChainSupported::Ethereum
            | ChainSupported::Bnb
            | ChainSupported::Arbitrum
            | ChainSupported::Optimism
            | ChainSupported::Polygon
            | ChainSupported::Base => self.verify_evm_signature(address, signature, msg, who)?,
            ChainSupported::Tron => self.verify_tron_signature(address, signature, msg, who)?,
            ChainSupported::Solana => self.verify_solana_signature(address, signature, msg, who)?,
            ChainSupported::Bitcoin => {
                self.verify_bitcoin_signature(address, signature, msg, who)?
            }
            ChainSupported::Polkadot => {
                self.verify_polkadot_signature(address, signature, msg, who)?
            }
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

    /// Verify signature for EVM-compatible chains (Ethereum, BNB, Arbitrum, Optimism, Polygon, Base)
    fn verify_evm_signature(
        &self,
        address: String,
        signature: Vec<u8>,
        msg: Vec<u8>,
        who: &str,
    ) -> Result<(), anyhow::Error> {
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

        match signature.recover_from_prehash(<&B256>::from(&hashed_msg)) {
            Ok(recovered_addr) => {
                // check if the recovered key is a point on the secp256k1 curve
                // and we are doing this check here instead of very_bytes is because we cant get ECDSA public key before
                let recv_addr = recovered_addr.clone();
                let encoded_point = recv_addr.to_encoded_point(false).to_bytes().to_vec();

                if !Self::is_on_curve_sec1(&encoded_point) {
                    Err(anyhow!(
                        "addresses verification failed: point not on curve: who: {who}"
                    ))?
                }
                let pub_key_hash = keccak_256(&encoded_point[1..]);
                let recovered_addr = Address::from_str(&format!(
                    "0x{}",
                    hex::encode(&pub_key_hash[pub_key_hash.len() - 20..])
                ))
                .map_err(|err| anyhow!("failed to convert pub key hash to address: {:?}", err))?;

                info!("recovered addr: {:?}:  who: {who}", recovered_addr);
                info!("address: {:?}:  who: {who}", address);

                if recovered_addr == address {
                    Ok(())
                } else {
                    Err(anyhow!(
                        "addr recovery equality failed hence account invalid"
                    ))?
                }
            }
            Err(err) => Err(anyhow!("ec signature verification failed: {err}"))?,
        }
    }

    /// Verify signature for TRON
    fn verify_tron_signature(
        &self,
        address: String,
        signature: Vec<u8>,
        msg: Vec<u8>,
        who: &str,
    ) -> Result<(), anyhow::Error> {
        info!("TRON signature verification for {who}: {address}");
        todo!("TRON signature verification not implemented yet")
    }

    /// Verify signature for Solana
    fn verify_solana_signature(
        &self,
        address: String,
        signature: Vec<u8>,
        msg: Vec<u8>,
        who: &str,
    ) -> Result<(), anyhow::Error> {
        
        let public_key = Ed25519PublicKey::from_slice(&address.from_base58().map_err(|_| anyhow!("invalid solana address"))?)?;
        let signature = Ed25519Signature::from_slice(&signature)?;
        if let Ok(_) = public_key.verify(msg.as_slice(), &signature) {
            Ok(())
        } else {
            Err(anyhow!("solana signature verification failed"))
        }
   
    }

    /// Verify signature for Bitcoin
    fn verify_bitcoin_signature(
        &self,
        address: String,
        signature: Vec<u8>,
        msg: Vec<u8>,
        who: &str,
    ) -> Result<(), anyhow::Error> {
        info!("Bitcoin signature verification for {who}: {address}");
        todo!("Bitcoin signature verification not implemented yet")
    }

    /// Verify signature for Polkadot
    fn verify_polkadot_signature(
        &self,
        address: String,
        signature: Vec<u8>,
        msg: Vec<u8>,
        who: &str,
    ) -> Result<(), anyhow::Error> {
        info!("Polkadot signature verification for {who}: {address}");
        todo!("Polkadot signature verification not implemented yet")
    }

    pub fn is_on_curve_sec1(sec1: &[u8]) -> bool {
        match k256::EncodedPoint::from_bytes(sec1) {
            Ok(ep) => bool::from(k256::AffinePoint::from_encoded_point(&ep).is_some()),
            Err(_) => false,
        }
    }
}
