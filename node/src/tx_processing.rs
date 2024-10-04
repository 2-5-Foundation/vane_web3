// receives tx from rpc
// Tx State machine updating
// Db updates
// Send and receive tx update events from p2p swarm
// send to designated chain network

extern crate alloc;

use alloc::sync::Arc;
use alloy::consensus::{SignableTransaction, TxEip7702};
use alloy::hex;
use alloy::network::TransactionBuilder;
use alloy::primitives::private::alloy_rlp::{Decodable, Encodable};
use alloy::primitives::ruint::aliases::U256;
use alloy::primitives::Address;
use alloy::providers::{Provider, ProviderBuilder, ReqwestProvider};
use anyhow::anyhow;
use codec;
use libp2p::futures::TryFutureExt;
use primitives::data_structure::{ChainSupported, Token, TxStateMachine};
use sp_core::{
    ecdsa::{Public as EcdsaPublic, Signature as EcdsaSignature},
    ed25519::{Public as EdPublic, Signature as EdSignature},
    keccak_256,
    sr25519::{Pair as SrPair, Public as SrPublic, Signature as SrSignature},
};
use sp_core::{ByteArray, H256};
use sp_runtime::traits::Verify;
use std::collections::BTreeMap;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::Mutex;

// use solana_client::rpc_client::RpcClient;

/// handling tx processing, updating tx state machine, updating db and tx chain simulation processing
/// & tx submission to specified and confirmed chain
pub struct TxProcessingWorker {
    /// for receiving queued to be processed tx
    pub tx_state_machine_receiver: Arc<Mutex<Receiver<Arc<Mutex<TxStateMachine>>>>>,
    /// In-memory Db for tx processing at any stage
    tx_staging: Arc<Mutex<BTreeMap<H256, TxStateMachine>>>,
    /// In-memory Db for to be confirmed tx on sender
    pub sender_tx_pending: Arc<Mutex<Vec<TxStateMachine>>>,
    /// In-memory Db for to be confirmed tx on receiver
    pub receiver_tx_pending: Arc<Mutex<Vec<TxStateMachine>>>,
    // /// substrate client
    // sub_client: OnlineClient<PolkadotConfig>,
    /// ethereum & bnb client
    evm_client: ReqwestProvider,
    // solana_client: RpcClient
}

impl TxProcessingWorker {
    pub async fn new(
        recv_channel: Arc<Mutex<Receiver<Arc<Mutex<TxStateMachine>>>>>,
        chain_networks: (ChainSupported, ChainSupported, ChainSupported),
    ) -> Result<Self, anyhow::Error> {
        let (_solana, eth, bnb) = chain_networks;
        //let polkadot_url = polkadot.url();
        let eth_url = eth.url();
        let bnb_url = bnb.url().to_string();

        // let sub_client = OnlineClient::from_url(polkadot_url)
        //     .await
        //     .map_err(|_| anyhow!("failed to connect polkadot url"))?;
        let eth_rpc_url = eth_url.parse()?;
        // Create a provider with the HTTP transport using the `reqwest` crate.
        let provider = ProviderBuilder::new().on_http(eth_rpc_url);

        Ok(Self {
            tx_state_machine_receiver: recv_channel,
            tx_staging: Arc::new(Default::default()),
            sender_tx_pending: Arc::new(Default::default()),
            receiver_tx_pending: Arc::new(Default::default()),
            //sub_client,
            evm_client: provider,
        })
    }
    /// cryptographically verify the receiver address, validity and address ownership on receiver's end
    async fn validate_receiver_address(&mut self, tx: TxStateMachine) -> Result<(), anyhow::Error> {
        let network = tx.network;
        let signature = tx.signature.ok_or(anyhow!("receiver didnt signed"))?;
        let msg = tx.receiver_address.clone();

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
            ChainSupported::Ethereum | ChainSupported::Bnb => {
                let ec_receiver_public = EcdsaPublic::from_slice(&tx.receiver_address[..])
                    .map_err(|_| anyhow!("failed to convert ecdsa recv addr bytes"))?;
                let hashed_msg = keccak_256(&msg[..]);
                let sig = EcdsaSignature::from_slice(&signature[..])
                    .map_err(|_| anyhow!("failed to convert Ecdsa_signature"))?;
                if sig.verify(&hashed_msg[..], &ec_receiver_public) {
                    let addr = sig
                        .recover(hashed_msg)
                        .ok_or(anyhow!("cannot recover addr"))?;
                    if addr == ec_receiver_public {
                        Ok::<(), anyhow::Error>(())?
                    } else {
                        Err(anyhow!("addr recovery equality failed hence recv failed"))?
                    }
                } else {
                    Err(anyhow!(
                        "ec signature verification failed hence recv failed"
                    ))?
                }
            }
            ChainSupported::Solana => {
                let ed_receiver_public = EdPublic::from_slice(&tx.receiver_address[..])
                    .map_err(|_| anyhow!("failed to convert ed25519 recv addr bytes"))?;
                let sig = EdSignature::from_slice(&signature[..])
                    .map_err(|_| anyhow!("failed to convert ed25519_signature"))?;

                if sig.verify(&msg[..], &ed_receiver_public) {
                    Ok::<(), anyhow::Error>(())?
                } else {
                    Err(anyhow!(
                        "ed25519 signature verification failed hence recv failed"
                    ))?
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// simulate the recipient blockchain network for mitigating errors resulting to wrong network selection
    async fn sim_confirm_network(&mut self, tx: TxStateMachine) -> Result<(), anyhow::Error> {
        Ok(())
    }

    /// create the tx to be signed by externally owned account
    pub async fn create_tx(&mut self, tx: TxStateMachine) -> Result<Vec<u8>, anyhow::Error> {
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
                let to_address = Address::from_slice(&tx.receiver_address);
                let value = U256::from(tx.amount);

                let tx_builder = alloy::rpc::types::TransactionRequest::default()
                    .with_to(to_address)
                    .with_value(value)
                    .build_unsigned()
                    .map_err(|err| {
                        anyhow!("cannot build unsigned tx to be signed by EOA; caused by: {err:?}")
                    })?;

                let mut encoded_tx = Vec::<u8>::new();
                tx_builder
                    .eip7702()
                    .ok_or(anyhow!("failed to convert to EIP 7702"))?
                    .encode(&mut encoded_tx);

                encoded_tx
            }

            ChainSupported::Bnb => {
                let to_address = Address::from_slice(&tx.receiver_address);
                let value = U256::from(tx.amount);

                let tx_builder = alloy::rpc::types::TransactionRequest::default()
                    .with_to(to_address)
                    .with_value(value)
                    .with_chain_id(56)
                    .build_unsigned()
                    .map_err(|err| {
                        anyhow!("cannot build unsigned tx to be signed by EOA; caused by: {err:?}")
                    })?;

                let mut encoded_tx = Vec::<u8>::new();
                tx_builder
                    .eip7702()
                    .ok_or(anyhow!("failed to convert to EIP 7702"))?
                    .encode(&mut encoded_tx);

                encoded_tx
            }

            ChainSupported::Solana => {
                todo!()
            }
            _ => {
                todo!()
            }
        };
        Ok(to_signed_bytes)
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
                let tx_payload = tx
                    .call_payload
                    .ok_or(anyhow!("call payload not found"))?;
                let decoded_tx = TxEip7702::decode(&mut &tx_payload[..]).map_err(|err| {
                    anyhow!("failed to decode eth EIP7702 tx payload; caused by: {err:?}")
                })?;

                let signed_tx = decoded_tx.into_signed(
                    signature
                        .as_slice()
                        .try_into()
                        .map_err(|err| anyhow!("failed to decode tx siganture"))?,
                );
                let mut encoded_signed_tx = vec![];
                signed_tx.tx().encode_with_signature(
                    signed_tx.signature(),
                    &mut encoded_signed_tx,
                    false,
                );

                let receipt = self
                    .evm_client
                    .send_raw_transaction(&encoded_signed_tx)
                    .await
                    .map_err(|err| anyhow!("failed to submit eth raw tx"))?
                    .tx_hash()
                    .clone();

                receipt
                    .to_vec()
                    .try_into()
                    .map_err(|err| anyhow!("failed to convert to 32 bytes array"))?
            }
            ChainSupported::Bnb => {
                todo!()
            }
            ChainSupported::Solana => {
                todo!()
            }
        };
        Ok(block_hash)
    }
}
