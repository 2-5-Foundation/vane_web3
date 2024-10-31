//! All data structure related to transaction processing and updating
extern crate alloc;
use alloc::vec::Vec;
use anyhow::Error;
use codec::{Decode, Encode};
use libp2p::request_response::{InboundRequestId, OutboundRequestId, ResponseChannel};
use libp2p::PeerId;
use serde::{Deserialize, Serialize};
use sp_core::H256;
use std::str::FromStr;
use tokio::sync::MutexGuard;

/// tx state
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, Encode, Decode)]
pub enum TxStatus {
    /// initial state,
    Genesis,
    /// if receiver address has been confirmed
    AddrConfirmed,
    /// if receiver chain network has been confirmed , used in tx simulation
    NetConfirmed,
    /// if the sender has confirmed, last stage and the txn is being submitted
    SenderConfirmed,
}
impl Default for TxStatus {
    fn default() -> Self {
        Self::Genesis
    }
}
/// Transaction data structure state machine, passed in rpc and p2p swarm
#[derive(Clone, Default, PartialEq, Debug, Deserialize, Serialize, Encode, Decode)]
pub struct TxStateMachine {
    pub sender_address: Vec<u8>,
    pub receiver_address: Vec<u8>,
    /// hashed sender and receiver address to bind the addresses while sending
    pub multi_id: sp_core::H256,
    /// signature of the receiver id
    pub signature: Option<Vec<u8>>,
    /// chain network
    pub network: ChainSupported,
    /// State Machine main params
    pub status: TxStatus,
    /// amount to be sent
    pub amount: u128,
    /// signed call payload
    pub signed_call_payload: Option<Vec<u8>>,
    /// call payload
    pub call_payload: Option<Vec<u8>>,
    /// Inbound Request id for p2p
    pub indbound_req_id: Option<u64>,
    /// Outbound Request id for p2p
    pub outbound_req_id: Option<u64>,
}

/// helper for solving passing `MutexGuard<TxStateMachine>`
/// as encoding that type doesnt work
pub fn new_tx_state_from_mutex(tx: MutexGuard<TxStateMachine>) -> TxStateMachine {
    TxStateMachine {
        sender_address: tx.sender_address.clone(),
        receiver_address: tx.receiver_address.clone(),
        multi_id: tx.multi_id,
        signature: tx.signature.clone(),
        network: tx.network,
        status: tx.status.clone(),
        amount: tx.amount.clone(),
        signed_call_payload: tx.signed_call_payload.clone(),
        call_payload: tx.call_payload.clone(),
        indbound_req_id: tx.indbound_req_id,
        outbound_req_id: tx.outbound_req_id,
    }
}

pub enum NetworkCommand {
    SendRequest {
        request: Vec<u8>,
        peer_id: PeerId,
    },
    SendResponse {
        response: Vec<u8>,
        channel: ResponseChannel<Result<Vec<u8>, Error>>,
    },
    Dial {
        peer_id: PeerId,
    },
}

#[derive(Clone)]
pub enum SwarmMessage {
    Request {
        data: Vec<u8>,
        inbound_id: InboundRequestId,
    },
    Response {
        data: Vec<u8>,
        outbound_id: OutboundRequestId,
    },
}

/// Transaction data structure to store in the db
#[derive(Clone, Deserialize, Serialize)]
pub struct DbTxStateMachine {
    // Tx hash based on the chain hashing algorithm
    pub tx_hash: Vec<u8>,
    // amount to be sent
    pub amount: u128,
    // chain network
    pub network: ChainSupported,
    // status
    pub success: bool,
}

/// Supported tokens
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize, Encode, Decode)]
pub enum Token {
    Dot,
    Bnb,
    Sol,
    Eth,
    UsdtSol,
    UsdcSol,
    UsdtEth,
    UsdcEth,
    UsdtDot,
}

impl From<Token> for ChainSupported {
    fn from(value: Token) -> Self {
        match value {
            Token::Dot | Token::UsdtDot => ChainSupported::Polkadot,
            Token::Bnb => ChainSupported::Bnb,
            Token::Sol | Token::UsdcSol | Token::UsdtSol => ChainSupported::Solana,
            Token::Eth | Token::UsdtEth | Token::UsdcEth => ChainSupported::Ethereum,
        }
    }
}

/// Supported blockchain networks along with rpc provider url
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, Encode, Decode, Copy)]
pub enum ChainSupported {
    Polkadot,
    Ethereum,
    Bnb,
    Solana,
}

impl Default for ChainSupported {
    fn default() -> Self {
        ChainSupported::Polkadot
    }
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
    pub peer_id: Option<String>, // this should be just account address and it will be converted to libp2p::PeerId,
    pub account_id1: Option<Vec<u8>>,
    pub account_id2: Option<Vec<u8>>,
    pub account_id3: Option<Vec<u8>>,
    pub account_id4: Option<Vec<u8>>,
    pub multi_addr: Option<String>,
    pub keypair: Option<Vec<u8>>, // encrypted
}

/// p2p config
pub struct P2pConfig {}

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

// airtable db or peer discovery
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Discovery {
    pub peer_id: Option<String>,
    pub multi_addr: Option<String>,
    pub account_ids: Vec<String>,
}

impl From<Discovery> for PeerRecord {
    fn from(value: Discovery) -> Self {
        let mut acc = vec![];
        if let Some(addr) = value.account_ids.get(0) {
            let acc_to_vec_id = addr.as_bytes().to_vec();
            acc.push(acc_to_vec_id)
        }
        if let Some(addr) = value.account_ids.get(1) {
            let acc_to_vec_id = addr.as_bytes().to_vec();
            acc.push(acc_to_vec_id)
        }
        if let Some(addr) = value.account_ids.get(2) {
            let acc_to_vec_id = addr.as_bytes().to_vec();
            acc.push(acc_to_vec_id)
        }
        if let Some(addr) = value.account_ids.get(3) {
            let acc_to_vec_id = addr.as_bytes().to_vec();
            acc.push(acc_to_vec_id)
        }

        Self {
            peer_id: value.peer_id,
            account_id1: acc.get(0).map(|x| x.clone()),
            account_id2: acc.get(1).map(|x| x.clone()),
            account_id3: acc.get(2).map(|x| x.clone()),
            account_id4: acc.get(3).map(|x| x.clone()),
            multi_addr: value.multi_addr,
            keypair: None,
        }
    }
}

// to destructure returned json from db
#[derive(Debug, Serialize, Clone, Deserialize)]
pub struct AirtableResponse {
    pub records: Vec<Record>,
}
#[derive(Debug, Serialize, Clone, Deserialize)]
pub struct Record {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    #[serde(rename = "createdTime")]
    pub created_time: String,
    pub fields: Fields,
}

#[derive(Debug, Serialize, Clone, Deserialize)]
pub struct Fields {
    #[serde(rename = "portId", default)]
    pub multi_addr: Option<String>,
    #[serde(rename = "peerId", default)]
    pub peer_id: Option<String>,
    #[serde(rename = "accountId1", default)]
    pub account_id1: Option<String>,
    #[serde(rename = "accountId2", default)]
    pub account_id2: Option<String>,
    #[serde(rename = "accountId3", default)]
    pub account_id3: Option<String>,
    #[serde(rename = "accountId4", default)]
    pub account_id4: Option<String>,
}

impl From<PeerRecord> for Fields {
    fn from(value: PeerRecord) -> Self {
        let multi_addr = value.multi_addr;
        let peer_id = value.peer_id;

        // TODO convert all accounts
        Self {
            multi_addr,
            peer_id,
            account_id1: Some(
                String::from_utf8(value.account_id1.expect("no account id 1 from peer record"))
                    .unwrap(),
            ),
            account_id2: None,
            account_id3: None,
            account_id4: None,
        }
    }
}
