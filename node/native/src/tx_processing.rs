use anyhow::anyhow;
use core::str::FromStr;
use log::error;
use primitives::data_structure::{ChainSupported, ETH_SIG_MSG_PREFIX, TxStateMachine};
use sp_core::{ByteArray, H256};
use sp_core::{
    blake2_256, ecdsa as EthSignature,
    ed25519::{Public as EdPublic, Signature as EdSignature},
    keccak_256,
};
use sp_runtime::traits::Verify;
use std::collections::BTreeMap;

// use solana_client::rpc_client::RpcClient;
pub use tx_std_imports::*;

mod tx_std_imports {
    pub use crate::rpc::Blake2Hasher;
    pub use alloy::consensus::{SignableTransaction, TxEip7702, TypedTransaction};
    pub use alloy::network::TransactionBuilder;
    pub use alloy::primitives::private::alloy_rlp::{Decodable, Encodable};
    pub use alloy::primitives::{
        Address, B256, Signature as EcdsaSignature, Signature, SignatureError,
    };
    pub use alloy::primitives::{U256, keccak256};
    pub use alloy::providers::{Provider, ProviderBuilder, ReqwestProvider};
    pub use alloy::rpc::types::TransactionRequest;
    pub use sp_core::Hasher;
    pub use std::sync::Arc;
    pub use tokio::sync::Mutex;
    pub use tokio::sync::mpsc::Receiver;
}

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
    pub eth_client: ReqwestProvider,
    pub bnb_client: ReqwestProvider,
    // solana_client: RpcClient
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
                    .map_err(|err| anyhow!("failed to convert ecdsa signature; {err:?}"))?;

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

        post_multi_id == H256(txn.multi_id)
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
                let sender_address = tx.sender_address.parse().expect("Invalid sender address");
                let to_address: Address =
                    tx.receiver_address.parse().expect("Invalid recv address");
                let value = U256::from(tx.amount);

                // Get nonce and fee estimates from Ethereum RPC
                let nonce = self
                    .eth_client
                    .get_transaction_count(sender_address)
                    .await
                    .map_err(|err| {
                        log::error!("Failed to get nonce from Ethereum RPC: {err}");
                        anyhow!(
                            "Ethereum RPC unavailable - cannot get nonce for transaction creation"
                        )
                    })?;

                let fee_estimate = self
                    .eth_client
                    .estimate_eip1559_fees(None)
                    .await
                    .map_err(|err| {
                        log::error!("Failed to get fee estimates from Ethereum RPC: {err}");
                        anyhow!("Ethereum RPC unavailable - cannot get fee estimates for transaction creation")
                    })?;

                let max_fee = fee_estimate.max_fee_per_gas;
                let priority_fee = fee_estimate.max_priority_fee_per_gas;

                // Set gas limit (either calculate or use fixed amount)
                let gas_limit: u128 = U256::from(21000)
                    .try_into()
                    .expect("failed to convert to u128"); // Basic ETH transfer gas limit

                let tx_builder = TransactionRequest::default()
                    .with_to(to_address)
                    .with_value(value)
                    .with_chain_id(56)
                    // Add required EIP-1559 fields
                    .with_nonce(nonce) // Get from eth_client.get_transaction_count
                    .with_gas_limit(gas_limit.into()) // Either calculate or set fixed amount
                    .with_max_fee_per_gas(max_fee) // Get from eth_client.estimate_eip1559_fees
                    .with_max_priority_fee_per_gas(priority_fee) // Also from estimate_fees
                    .build_unsigned()
                    .map_err(|err| {
                        anyhow!("cannot build tx to be signed by EOA; caused by: {err:?}")
                    })?;

                let signing_hash = tx_builder
                    .eip1559()
                    .ok_or(anyhow!("failed to convert to EIP 1559"))?
                    .signature_hash();

                tx.call_payload = Some(<[u8; 32]>::from(signing_hash));
            }

            ChainSupported::Bnb => {
                let to_address = Address::from_slice(&tx.receiver_address.as_bytes());
                let value = U256::from(tx.amount);

                let tx_builder = alloy::rpc::types::TransactionRequest::default()
                    .with_to(to_address)
                    .with_value(value)
                    .with_chain_id(31337)
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

                let sender_address = tx.sender_address.parse().expect("Invalid sender address");
                let to_address: Address =
                    tx.receiver_address.parse().expect("Invalid recv address");
                let value = U256::from(tx.amount);

                // Get nonce and fee estimates from Ethereum RPC
                let nonce = self
                    .eth_client
                    .get_transaction_count(sender_address)
                    .await
                    .map_err(|err| {
                        log::error!("Failed to get nonce from Ethereum RPC: {err}");
                        anyhow!(
                            "Ethereum RPC unavailable - cannot get nonce for transaction submission"
                        )
                    })?;

                let fee_estimate = self
                    .eth_client
                    .estimate_eip1559_fees(None)
                    .await
                    .map_err(|err| {
                        log::error!("Failed to get fee estimates from Ethereum RPC: {err}");
                        anyhow!("Ethereum RPC unavailable - cannot get fee estimates for transaction submission")
                    })?;

                let max_fee = fee_estimate.max_fee_per_gas;
                let priority_fee = fee_estimate.max_priority_fee_per_gas;

                // Set gas limit (either calculate or use fixed amount)
                let gas_limit: u128 = U256::from(21000)
                    .try_into()
                    .expect("failed to convert to u128"); // Basic ETH transfer gas limit

                let tx_builder = TransactionRequest::default()
                    .with_to(to_address)
                    .with_value(value)
                    .with_chain_id(31337)
                    // Add required EIP-1559 fields
                    .with_nonce(nonce) // Get from eth_client.get_transaction_count
                    .with_gas_limit(gas_limit.into()) // Either calculate or set fixed amount
                    .with_max_fee_per_gas(max_fee) // Get from eth_client.estimate_eip1559_fees
                    .with_max_priority_fee_per_gas(priority_fee) // Also from estimate_fees
                    .build_unsigned()
                    .map_err(|err| {
                        anyhow!("cannot build tx to be submitted; caused by: {err:?}")
                    })?;

                let signed_tx = tx_builder
                    .eip1559()
                    .expect("failed to convert to EIP1559")
                    .clone()
                    .into_signed(signature);

                let to_submit_tx: TransactionRequest = signed_tx.tx().clone().into();

                let receipt = self
                    .eth_client
                    .send_transaction(to_submit_tx)
                    .await
                    .map_err(|err| {
                        log::error!("Failed to submit transaction to Ethereum RPC: {err}");
                        anyhow!("Failed to submit transaction to Ethereum RPC: {err}")
                    })?
                    .tx_hash()
                    .clone();

                log::info!(target: "MainServiceWorker","transaction submitted successfully: {receipt:?}");

                receipt.to_vec().try_into().map_err(|err| {
                    anyhow!("failed to convert to 32 bytes array; caused by: {err:?}")
                })?
            }
            ChainSupported::Bnb => {
                // TODO
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
