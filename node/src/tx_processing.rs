// receives tx from rpc
// Tx State machine updating
// Db updates
// Send and receive tx update events from p2p swarm
// send to designated chain network

extern crate alloc;

use alloc::sync::Arc;
use alloy::consensus::{SignableTransaction, TxEip7702, TypedTransaction};
use alloy::network::TransactionBuilder;
use alloy::primitives::private::alloy_rlp::{Decodable, Encodable};
use alloy::primitives::{keccak256, U256};
use alloy::primitives::{Address, Signature as EcdsaSignature, Signature, SignatureError, B256};
use alloy::providers::{Provider, ProviderBuilder, ReqwestProvider};
use alloy::rpc::types::TransactionRequest;
use alloy::signers::k256::sha2::digest::Mac;
use anyhow::anyhow;
use core::str::FromStr;
use log::error;
use primitives::data_structure::{ChainSupported, TxStateMachine, ETH_SIG_MSG_PREFIX};
use sp_core::{
    ed25519::{Public as EdPublic, Signature as EdSignature},
    keccak_256, Blake2Hasher, Hasher,ecdsa as EthSignature
};
use sp_core::{ByteArray, H256};
use sp_runtime::traits::Verify;
use std::collections::BTreeMap;
use tokio::sync::mpsc::Receiver;
use tokio::sync::Mutex;

// use solana_client::rpc_client::RpcClient;

// ------------------------------------- WASM ------------------------------------- //
#[cfg(target_arch = "wasm32")]
use tx_wasm_imports::*;

#[cfg(target_arch = "wasm32")]
mod tx_wasm_imports {
    pub use web3::Web3;
    pub use web3::transports;
    pub use core::cell::RefCell;
    pub use alloc::rc::Rc;
}

/// handling tx processing, updating tx state machine, updating db and tx chain simulation processing
/// & tx submission to specified and confirmed chain
#[derive(Clone)]
pub struct TxProcessingWorker {
    /// In-memory Db for tx processing at any stage 
    tx_staging: Arc<Mutex<BTreeMap<H256, TxStateMachine>>>,
    /// In-memory Db for to be confirmed tx on sender
    pub sender_tx_pending: Arc<Mutex<Vec<TxStateMachine>>>,
    /// In-memory Db for to be confirmed tx on receiver
    pub receiver_tx_pending: Arc<Mutex<Vec<TxStateMachine>>>,
    // /// substrate client
    // sub_client: OnlineClient<PolkadotConfig>,
    /// ethereum & bnb client
    eth_client: ReqwestProvider,
    bnb_client: ReqwestProvider,
    // solana_client: RpcClient
}

#[cfg(target_arch = "wasm32")]
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
   /// ethereum & bnb client
   eth_client: Web3<web3::transports::Http>,
   bnb_client: Web3<web3::transports::Http>,
   // solana_client: RpcClient 
}

#[cfg(target_arch = "wasm32")]
impl WasmTxProcessingWorker{
    pub fn new(
        chain_networks: (ChainSupported, ChainSupported, ChainSupported),
    ) -> Result<Self, anyhow::Error> {
        let (_solana, eth, bnb) = chain_networks;
        let eth_url = eth.url();
        let bnb_url = bnb.url().to_string();

        let eth_transport = web3::transports::Http::new(&eth_url)
            .map_err(|err| anyhow!("eth transport error: {err}"))?;
        let eth_web3 = web3::Web3::new(eth_transport);

        let bnb_transport = web3::transports::Http::new(&bnb_url)
            .map_err(|err| anyhow!("bnb transport error: {err}"))?;
        let bnb_web3 = web3::Web3::new(bnb_transport);

        Ok(Self {
            tx_staging: Rc::new(RefCell::new(Default::default())),
            sender_tx_pending: Rc::new(RefCell::new(Default::default())),
            receiver_tx_pending: Rc::new(RefCell::new(Default::default())),
            eth_client: eth_web3,
            bnb_client: bnb_web3,
        })
    }

    pub fn validate_receiver_sender_address(
        &self,
        tx: &TxStateMachine,
        who: &str
    ) -> Result<(),anyhow::Error>{
        let (network, signature, msg, address) = if who == "Receiver" {
            println!("\n receiver address verification \n");

            let network = tx.network;
            let signature = tx
                .clone()
                .recv_signature
                .ok_or(anyhow!("receiver didnt signed"))?;

            let recv_address = tx.receiver_address.clone();
            let msg = tx.receiver_address.as_bytes().to_vec();

            (network, signature, msg, recv_address)
        } else {
            println!("\n sender address verification \n");
            // who == Sender
            let network = tx.network;
            let signature = tx
                .clone()
                .signed_call_payload
                .ok_or(anyhow!("original sender didnt signed"))?;

            let msg = tx
                .call_payload
                .expect("unexpected error, call payload should be available");
            let sender_address = tx.sender_address.clone();

            (network, signature, msg.to_vec(), sender_address)
        };
        match network {
            ChainSupported::Ethereum => {
                let address: Address = address.parse().expect("Invalid address");

                let hashed_msg = {
                    if who == "Receiver" {
                        let mut signable_msg = Vec::<u8>::new();
                        signable_msg.extend_from_slice(ETH_SIG_MSG_PREFIX.as_bytes());
                        signable_msg.extend_from_slice(msg.len().to_string().as_bytes());
                        signable_msg.extend_from_slice(msg.as_slice());

                        keccak_256(signable_msg.as_slice())
                    } else {
                        msg.try_into().unwrap()
                    }
                };
                let signature = EcdsaSignature::try_from(signature.as_slice())
                    .map_err(|err| anyhow!("failed to convert ecdsa signature"))?;

                match signature.recover_address_from_prehash(<&B256>::from(&hashed_msg)) {
                    Ok(recovered_addr) => {
                        println!(
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
            },
            _ => unreachable!()
        }
        Ok(())
    }

    pub fn validate_multi_id(&self, txn: &TxStateMachine) -> bool {
        let post_multi_id = {
            let mut sender_recv = txn.sender_address.as_bytes().to_vec();
            sender_recv.extend_from_slice(txn.receiver_address.as_bytes());
            Blake2Hasher::hash(&sender_recv[..])
        };

        post_multi_id == txn.multi_id
    }

    pub async fn submit_tx(&mut self, tx: TxStateMachine) -> Result<[u8; 32], anyhow::Error> {
        // TODO
        Ok([0u8;32])
    }

    pub async fn create_tx(&mut self, tx: &mut TxStateMachine) -> Result<(), anyhow::Error> {
        // TODO
        Ok(())
    }

}

impl TxProcessingWorker {
    pub async fn new(
        chain_networks: (ChainSupported, ChainSupported, ChainSupported),
    ) -> Result<Self, anyhow::Error> {
        let (_solana, eth, bnb) = chain_networks;
        //let polkadot_url = polkadot.url();
        let eth_url = eth.url();
        let bnb_url = bnb.url().to_string();

        // let sub_client = OnlineClient::from_url(polkadot_url)
        //     .await
        //     .map_err(|_| anyhow!("failed to connect polkadot url"))?;
        let eth_rpc_url = eth_url
            .parse()
            .map_err(|err| anyhow!("eth rpc parse error: {err}"))?;
        // Create a provider with the HTTP transport using the `reqwest` crate.
        let eth_provider = ProviderBuilder::new().on_http(eth_rpc_url);

        let bnb_rpc_url = bnb_url
            .parse()
            .map_err(|err| anyhow!("bnb rpc url parse error: {err}"))?;
        let bnb_provider = ProviderBuilder::new().on_http(bnb_rpc_url);

        Ok(Self {
            tx_staging: Arc::new(Default::default()),
            sender_tx_pending: Arc::new(Default::default()),
            receiver_tx_pending: Arc::new(Default::default()),
            //sub_client,
            eth_client: eth_provider,
            bnb_client: bnb_provider,
        })
    }
    /// cryptographically verify the receiver address, validity and address ownership on receiver's end
    pub fn validate_receiver_sender_address(
        &self,
        tx: &TxStateMachine,
        who: &str,
    ) -> Result<(), anyhow::Error> {
        let (network, signature, msg, address) = if who == "Receiver" {
            println!("\n receiver address verification \n");

            let network = tx.network;
            let signature = tx
                .clone()
                .recv_signature
                .ok_or(anyhow!("receiver didnt signed"))?;

            let recv_address = tx.receiver_address.clone();
            let msg = tx.receiver_address.as_bytes().to_vec();

            (network, signature, msg, recv_address)
        } else {
            println!("\n sender address verification \n");
            // who == Sender
            let network = tx.network;
            let signature = tx
                .clone()
                .signed_call_payload
                .ok_or(anyhow!("original sender didnt signed"))?;

            let msg = tx
                .call_payload
                .expect("unexpected error, call payload should be available");
            let sender_address = tx.sender_address.clone();

            (network, signature, msg.to_vec(), sender_address)
        };
        match network {
            ChainSupported::Polkadot => {
                // let sr_receiver_public = SrPublic::from_slice(&tx.data.receiver_address[..])
                //     .map_err(|_| anyhow!("failed to convert recv addr bytes"))?;
                // let sig = SrSignature::from_slice(&signature[..])
                //     .map_err(|_| anyhow!("failed to convert sr25519signature"))?;
                // if sig.verify(&msg[..], &sr_receiver_public) {
                //     Ok::<(), anyhow::Error>(())?
                // } else {
                //     Err(anyhow!(
                //         "sr signature verification failed hence recv failed"
                //     ))?
                // }
                todo!()
            }
            ChainSupported::Ethereum => {
                let address: Address = address.parse().expect("Invalid address");

                let hashed_msg = {
                    if who == "Receiver" {
                        let mut signable_msg = Vec::<u8>::new();
                        signable_msg.extend_from_slice(ETH_SIG_MSG_PREFIX.as_bytes());
                        signable_msg.extend_from_slice(msg.len().to_string().as_bytes());
                        signable_msg.extend_from_slice(msg.as_slice());

                        keccak_256(signable_msg.as_slice())
                    } else {
                        msg.try_into().unwrap()
                    }
                };
                let signature = EcdsaSignature::try_from(signature.as_slice())
                    .map_err(|err| anyhow!("failed to convert ecdsa signature"))?;

                match signature.recover_address_from_prehash(<&B256>::from(&hashed_msg)) {
                    Ok(recovered_addr) => {
                        println!(
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
            ChainSupported::Bnb => {
                todo!()
            }
            ChainSupported::Solana => {
                let ed_receiver_public = EdPublic::from_str(&tx.receiver_address)
                    .map_err(|_| anyhow!("failed to convert ed25519 recv addr bytes"))?;
                let sig = EdSignature::from_slice(&signature[..])
                    .map_err(|_| anyhow!("failed to convert ed25519_signature"))?;

                if sig.verify(msg.as_slice(), &ed_receiver_public) {
                    Ok::<(), anyhow::Error>(())?
                } else {
                    Err(anyhow!(
                        "ed25519 signature verification failed hence recv failed"
                    ))?
                }
            }
        }
        Ok(())
    }

    pub fn validate_multi_id(&self, txn: &TxStateMachine) -> bool {
        let post_multi_id = {
            let mut sender_recv = txn.sender_address.as_bytes().to_vec();
            sender_recv.extend_from_slice(txn.receiver_address.as_bytes());
            Blake2Hasher::hash(&sender_recv[..])
        };

        post_multi_id == txn.multi_id
    }

    /// simulate the recipient blockchain network for mitigating errors resulting to wrong network selection
    async fn sim_confirm_network(&mut self, _tx: TxStateMachine) -> Result<(), anyhow::Error> {
        Ok(())
    }

    /// create the tx to be signed by externally owned account
    pub async fn create_tx(&mut self, tx: &mut TxStateMachine) -> Result<(), anyhow::Error> {
        let network = tx.network;
        let to_signed_bytes = match network {
            ChainSupported::Polkadot => {
                // let transfer_value = dynamic::Value::primitive(U128(tx.data.amount as u128));
                // let to_address = dynamic::Value::from_bytes(tx.data.receiver_address);
                //
                // // construct a dynamic extrinsic payload
                // let extrinsic = dynamic(
                //     "Balances",
                //     "transferKeepAlive",
                //     Named(vec![
                //         ("dest".to_string(), to_address),
                //         ("value".to_string(), transfer_value),
                //     ]),
                // );
                // let ext_params = DefaultExtrinsicParamsBuilder::<PolkadotConfig>::new().build();
                // let partial_ext = self
                //     .sub_client
                //     .tx()
                //     .create_partial_signed_offline(&extrinsic, ext_params)
                //     .map_err(|err| anyhow!("failed to create partial ext; caused by: {err:?}"))?;
                // partial_ext.signer_payload()
                todo!()
            }

            ChainSupported::Ethereum => {
                let from_address: Address = tx.sender_address.parse().expect("Invalid address");
                let to_address: Address = tx.receiver_address.parse().expect("Invalid address");
                let value = U256::from(tx.amount);

                // TODO upgrade to EIP7702
                let tx_builder = TransactionRequest::default()
                    .with_from(from_address)
                    .with_to(to_address)
                    .with_value(value)
                    .with_nonce(0)
                    .with_chain_id(56)
                    .with_gas_limit(21_000)
                    .with_max_priority_fee_per_gas(1_000_000_000)
                    .with_max_fee_per_gas(20_000_000_000)
                    .build_unsigned()
                    .map_err(|err| {
                        anyhow!("cannot build unsigned tx to be signed by EOA; caused by: {err:?}")
                    })?;

                let signing_hash = tx_builder
                    .eip1559()
                    .ok_or(anyhow!("failed to convert to EIP 7702"))?
                    .signature_hash();

                tx.call_payload = Some(<[u8; 32]>::from(signing_hash));
            }

            ChainSupported::Bnb => {
                let to_address = Address::from_slice(&tx.receiver_address.as_bytes());
                let value = U256::from(tx.amount);

                let tx_builder = alloy::rpc::types::TransactionRequest::default()
                    .with_to(to_address)
                    .with_value(value)
                    .with_chain_id(56)
                    .build_unsigned()
                    .map_err(|err| {
                        anyhow!("cannot build unsigned tx to be signed by EOA; caused by: {err:?}")
                    })?;

                let signing_hash = tx_builder
                    .eip7702()
                    .ok_or(anyhow!("failed to convert to EIP 7702"))?
                    .signature_hash();

                tx.call_payload = Some(<[u8; 32]>::from(signing_hash));
            }

            ChainSupported::Solana => {
                todo!()
            }
        };
        Ok(())
    }

    /// submit the externally signed tx, returns tx hash
    pub async fn submit_tx(&mut self, tx: TxStateMachine) -> Result<[u8; 32], anyhow::Error> {
        let network = tx.network;

        let block_hash = match network {
            ChainSupported::Polkadot => {
                // let signature_payload = MultiSignature::Sr25519(<[u8; 64]>::from(
                //     SrSignature::from_slice(
                //         &tx.data
                //             .signed_call_payload
                //             .ok_or(anyhow!("call payload not signed"))?,
                //     )
                //     .map_err(|_| anyhow!("failed to convert sr signature"))?,
                // ));
                // let sender = MultiAddress::Address32(
                //     SrPublic::from_slice(&tx.data.sender_address)
                //         .map_err(|_| anyhow!("failed to convert acc id"))?
                //         .0,
                // );
                //
                // let transfer_value = dynamic::Value::primitive(U128(tx.data.amount as u128));
                // let extrinsic = dynamic(
                //     "Balances",
                //     "transferKeepAlive",
                //     Named(vec![("dest".to_string(), transfer_value)]),
                // );
                // let ext_params = DefaultExtrinsicParamsBuilder::<PolkadotConfig>::new().build();
                // let partial_ext = self
                //     .sub_client
                //     .tx()
                //     .create_partial_signed_offline(&extrinsic, ext_params)
                //     .map_err(|_| anyhow!("failed to create partial ext"))?;
                //
                // let submittable_extrinsic =
                //     partial_ext.sign_with_address_and_signature(&sender, &signature_payload.into());
                //
                // let tx_hash = submittable_extrinsic
                //     .submit_and_watch()
                //     .await?
                //     .wait_for_finalized()
                //     .await?
                //     .block_hash()
                //     .tx_hash();
                //
                // tx_hash
                //     .to_vec()
                //     .try_into()
                //     .map_err(|err| anyhow!("failed to convert to 32 bytes array"))
                todo!()
            }
            ChainSupported::Ethereum => {
                let signature = tx
                    .signed_call_payload
                    .ok_or(anyhow!("sender did not signed the tx payload"))?;
                let signature = Signature::try_from(signature.as_slice())
                    .map_err(|err| anyhow!("failed to parse signature: {err}"))?;

                let to_address: Address = tx.receiver_address.parse().expect("Invalid address");
                let value = U256::from(tx.amount);

                let tx_builder = TransactionRequest::default()
                    .with_to(to_address)
                    .with_value(value)
                    .with_chain_id(56)
                    .build_unsigned()
                    .map_err(|err| {
                        anyhow!("cannot build unsigned tx to be signed by EOA; caused by: {err:?}")
                    })?
                    .eip7702()
                    .ok_or(anyhow!("failed to convert txn to eip7702"))?
                    .clone();

                let signed_tx = tx_builder.into_signed(signature);

                let to_submit_tx: TransactionRequest = signed_tx.tx().clone().into();
                let receipt = self
                    .eth_client
                    .send_transaction(to_submit_tx)
                    .await
                    .map_err(|err| anyhow!("failed to submit eth raw tx; caused by :{err}"))?
                    .tx_hash()
                    .clone();

                receipt.to_vec().try_into().map_err(|err| {
                    anyhow!("failed to convert to 32 bytes array; caused by: {err:?}")
                })?
            }
            ChainSupported::Bnb => {
                todo!();
                let signature = tx
                    .signed_call_payload
                    .ok_or(anyhow!("sender did not signed the tx payload"))?;
                let tx_payload = tx.call_payload.ok_or(anyhow!("call payload not found"))?;
                let decoded_tx = TxEip7702::decode(&mut &tx_payload[..]).map_err(|err| {
                    anyhow!("failed to decode eth EIP7702 tx payload; caused by: {err:?}")
                })?;

                let signed_tx =
                    decoded_tx.into_signed(signature.as_slice().try_into().map_err(|err| {
                        anyhow!("failed to decode tx siganture; caused by: {err}")
                    })?);

                let mut encoded_signed_tx = vec![];
                signed_tx.tx().encode_with_signature(
                    signed_tx.signature(),
                    &mut encoded_signed_tx,
                    false,
                );

                let receipt = self
                    .bnb_client
                    .send_raw_transaction(&encoded_signed_tx)
                    .await
                    .map_err(|err| anyhow!("failed to submit eth raw tx; caused by: {err}"))?
                    .tx_hash()
                    .clone();

                receipt.to_vec().try_into().map_err(|err| {
                    anyhow!("failed to convert to 32 bytes array; caused by: {err:?}")
                })?
            }
            ChainSupported::Solana => {
                todo!()
            }
        };
        Ok(block_hash)
    }
}
