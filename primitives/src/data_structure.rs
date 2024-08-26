//! All data structure related to transaction processing and updating
extern crate alloc;
use alloc::{sync::Arc, vec::Vec};
use codec::{Decode, Encode};
use libp2p::request_response::OutboundRequestId;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::Sender;
/// The idea is similar to how future executor tasks are able to progress and have channels to send
/// themselves
#[derive(Clone)]
pub struct TxStateMachine {
    /// Sender channel to propagate itself
    pub sender_channel: Sender<Arc<TxStateMachine>>,
    pub sender_address: Vec<u8>,
    pub receiver_address: Vec<u8>,
    // hashed sender and receiver address to bind the addresses while sending
    pub multi_id: Vec<u8>,
    //signature of the receiver id
    pub signature: Option<Vec<u8>>,
    // chain network
    pub network: Option<ChainSupported>,
    /// State Machine main params
    // if address has been confirmed
    pub addr_confirmed: bool,
    // if chain network has been confirmed
    pub net_confirmed: bool,
    // if address was able to be resolved automatically
    pub address_resolved: Option<ChainSupported>,
    // amount to be sent
    pub amount: u64,
    // signed call payload
    pub signed_call_payload: Option<Vec<u8>>,
    // call payload
    pub call_payload: Option<Vec<u8>>,
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
    pub fn url(&self) -> &'static str {
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
    pub id: OutboundRequestId,
    pub request: Request,
}

#[derive(Debug, Clone, Decode, Encode)]
pub struct Request {
    pub sender: Vec<u8>,
    pub receiver: Vec<u8>,
    pub amount: u64,
    pub network: ChainSupported,
    pub msg: Vec<u8>,
}

#[derive(Debug, Clone, Decode, Encode)]
pub struct Response {
    pub sender: Vec<u8>,
    pub receiver: Vec<u8>,
    pub response: Vec<u8>,
    pub sent_request_hash: Vec<u8>,
    pub msg: Vec<u8>,
    pub signature: Vec<u8>,
}

// Tx processing section

pub const POLKADOT_DOT: [u8; 32] = [
    234, 159, 151, 149, 77, 136, 90, 255, 210, 65, 183, 86, 160, 52, 93, 187, 226, 81, 189, 199,
    97, 83, 41, 247, 149, 89, 46, 0, 155, 194, 206, 55,
];
pub const POLKADOT_USDT: [u8; 32] = [
    234, 159, 151, 149, 77, 136, 90, 255, 210, 65, 183, 86, 160, 52, 93, 187, 226, 81, 189, 199,
    97, 83, 41, 247, 149, 89, 46, 0, 155, 194, 206, 55,
];
pub const ETHEREUM_ERC20: [u8; 20] = [
    105, 31, 184, 40, 43, 197, 168, 133, 138, 155, 238, 38, 186, 119, 226, 154, 136, 115, 130, 82,
];
pub const SOLANA: [u8; 44] = [
    65, 104, 117, 102, 100, 98, 65, 51, 49, 116, 77, 120, 49, 115, 100, 103, 106, 116, 113, 75,
    105, 115, 78, 85, 78, 72, 76, 89, 115, 52, 104, 118, 115, 67, 119, 90, 89, 81, 57, 89, 109,
    120, 84, 86,
];
pub const BEP20: [u8; 20] = [
    168, 67, 211, 99, 66, 69, 233, 17, 113, 99, 2, 94, 99, 58, 184, 246, 198, 102, 225, 111,
];
