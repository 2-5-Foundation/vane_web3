//! All data structure related to transaction processing and updating
extern crate alloc;
use alloc::{sync::Arc, vec::Vec};
use codec::{Decode, Encode};
use libp2p::request_response::{InboundRequestId, OutboundRequestId};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::Sender;
/// The idea is similar to how future executor tasks are able to progress and have channels to send
/// themselves
#[derive(Clone)]
pub struct TxStateMachine {
    /// Sender channel to propagate itself
    sender_channel: Sender<Arc<TxStateMachine>>,
    sender_address: Vec<u8>,
    receiver_address: Vec<u8>,
    network: Option<ChainSupported>,
    /// State Machine main params
    // if address has been confirmed
    addr_confirmed: bool,
    // if chain network has been confirmed
    net_confirmed: bool,
    // if address was able to be resolved automatically
    address_resolved: Option<ChainSupported>,
    // amount to be sent
    amount: u64,
}

/// Transaction data structure to store in the db
#[derive(Clone, Deserialize, Serialize)]
pub struct DbTxStateMachine {
    // Tx hash based on the chain hashing algorithm
    pub tx_hash: Vec<u8>,
    // amount to be sent
    pub amount: u64,
    // chain network
    pub network: ChainSupported,
    // status
    pub success: bool,
}

/// Supported blockchain networks along with rpc provider url
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, Encode, Decode)]
pub enum ChainSupported {
    Polkadot,
    Ethereum,
    Bnb,
    Solana,
}

impl From<ChainSupported> for &'static str {
    fn from(value: ChainSupported) -> Self {
        match value {
            ChainSupported::Polkadot => "Polkadot",
            ChainSupported::Ethereum => "Ethereum",
            ChainSupported::Bnb => "Bnb",
            ChainSupported::Solana => "Solana",
        }
    }
}

impl ChainSupported {
    // Associated constants representing network URLs or other constants
    const POLKADOT_URL: &'static str = "wss://polkadot-rpc.dwellir.com";
    const ETHEREUM_URL: &'static str = "https://mainnet.infura.io/v3/YOUR_INFURA_PROJECT_ID";
    const BNB_URL: &'static str = "https://bsc-dataseed.binance.org/";
    const SOLANA_URL: &'static str = "https://api.mainnet-beta.solana.com";

    // Method to get the URL based on the network type
    fn url(&self) -> &'static str {
        match self {
            ChainSupported::Polkadot => Self::POLKADOT_URL,
            ChainSupported::Ethereum => Self::ETHEREUM_URL,
            ChainSupported::Bnb => Self::BNB_URL,
            ChainSupported::Solana => Self::SOLANA_URL,
        }
    }
}

/// User account
#[derive(Clone, Eq, PartialEq, Deserialize, Serialize, Encode, Decode)]
pub struct UserAccount {
    pub user_name: Vec<u8>,
    pub account_id: Vec<u8>,
    pub network: ChainSupported,
}

/// Vane peer record
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, Encode, Decode)]
pub struct PeerRecord {
    pub peer_address: Vec<u8>, // this should be just account address and it will be converted to libp2p::PeerId,
    pub accountId1: Vec<u8>,
    pub accountId2: Option<Vec<u8>>,
    pub accountId3: Option<Vec<u8>>,
    pub accountId4: Option<Vec<u8>>,
    pub multi_addr: Vec<u8>,
    pub keypair: Option<Vec<u8>>, // encrypted
}

/// p2p config
pub struct p2pConfig {}


pub struct OuterRequest {
    pub id:OutboundRequestId,
    pub request: Request
}

#[derive(Debug,Clone,Decode,Encode)]
pub struct Request {
    pub sender: Vec<u8>,
    pub receiver: Vec<u8>,
    pub amount: u64,
    pub network: ChainSupported,
    pub msg: Vec<u8>,
}

#[derive(Debug,Clone,Decode,Encode)]
pub struct Response {
    pub sender: Vec<u8>,
    pub receiver: Vec<u8>,
    pub response: Vec<u8>,
    pub sent_request_hash: Vec<u8>,
    pub msg: Vec<u8>,
    pub signature: Vec<u8>,
}
