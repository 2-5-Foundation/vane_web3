//! All data structure related to transaction processing and updating
extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;
use anyhow::Error;
use codec::{Decode, Encode};
use core::hash::{Hash, Hasher};
use libp2p::request_response::{InboundRequestId, OutboundRequestId, ResponseChannel};
use libp2p::{Multiaddr, PeerId};
use serde::de::Error as SerdeError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use sp_core::{blake2_256, keccak_256, sha2_256};
use twox_hash::XxHash64;
#[cfg(feature = "wasm")]
use wasm_bindgen::{JsError, JsValue};

use dotenv::dotenv;
// Ethereum signature preimage prefix according to EIP-191
// keccak256("\x19Ethereum Signed Message:\n" + len(message) + message))
pub const ETH_SIG_MSG_PREFIX: &str = "\x19Ethereum Signed Message:\n";

/// tx state
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, Encode, Decode)]
pub enum TxStatus {
    /// initial state,
    Genesis,
    /// if receiver just confirmed
    RecvAddrConfirmed,
    /// if receiver address confirmation has passed
    RecvAddrConfirmationPassed,
    /// if receiver chain network has been confirmed , used in tx simulation
    NetConfirmed,
    /// if the sender has confirmed, last stage and the txn is being submitted
    SenderConfirmed,
    /// if non-original sender tries to sign
    SenderConfirmationfailed,
    /// if receiver failed to verify
    RecvAddrFailed,
    /// if transaction failed to be submitted due to some reasons
    FailedToSubmitTxn(String),
    /// if submission passed (tx-hash)
    TxSubmissionPassed([u8; 32]),
    /// if the receiver has not registered to vane yet
    ReceiverNotRegistered,
    /// if the transaction is reverted
    Reverted,
}

impl Default for TxStatus {
    fn default() -> Self {
        Self::Genesis
    }
}

fn serialize_u64_as_string<S>(value: &Option<u64>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match value {
        Some(v) => serializer.serialize_str(&v.to_string()),
        None => serializer.serialize_none(),
    }
}
fn deserialize_u64_flexible<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: Deserializer<'de>,
{
    // First try as string
    let value = Value::deserialize(deserializer)?;

    match value {
        // Handle direct number
        Value::Number(n) => {
            if let Some(num) = n.as_u64() {
                Ok(Some(num))
            } else {
                Err(D::Error::custom("Invalid number format for u64"))
            }
        }
        // Handle string (both normal and hex)
        Value::String(s) => {
            if let Some(stripped) = s.strip_prefix("0x") {
                // Handle hex string
                u64::from_str_radix(stripped, 16)
                    .map(Some)
                    .map_err(D::Error::custom)
            } else {
                // Handle decimal string
                s.parse::<u64>().map(Some).map_err(D::Error::custom)
            }
        }
        Value::Null => Ok(None),
        _ => Err(D::Error::custom("Expected string, number, or null")),
    }
}

fn string_serialize<S, T>(value: &Option<T>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    T: serde::Serialize,
{
    match value {
        Some(val) => {
            let json_str = serde_json::to_string(val).map_err(serde::ser::Error::custom)?;
            serializer.serialize_str(&json_str)
        }
        None => serializer.serialize_none(),
    }
}

/// Deserialize a string into `AccountInfo`
fn string_deserialize<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let s: Option<String> = Option::deserialize(deserializer)?;
    match s {
        Some(json_str) => {
            let parsed = serde_json::from_str(&json_str).map_err(serde::de::Error::custom)?;
            Ok(Some(parsed))
        }
        None => Ok(None),
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct WasmDhtResponse {
    pub peer_id: Option<PeerId>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct WasmDhtRequest {
    pub key: String,
}

/// Transaction data structure state machine, passed in rpc and p2p swarm
#[derive(Clone, Default, PartialEq, Debug, Deserialize, Serialize, Encode, Decode)]
pub struct TxStateMachine {
    #[serde(rename = "senderAddress")]
    pub sender_address: String,
    #[serde(rename = "receiverAddress")]
    pub receiver_address: String,
    /// hashed sender and receiver address to bind the addresses while sending
    #[serde(rename = "multiId")]
    pub multi_id: [u8; 32],
    /// signature of the receiver id (Signature)
    #[serde(rename = "recvSignature")]
    pub recv_signature: Option<Vec<u8>>,
    /// chain network
    pub network: ChainSupported,
    /// token
    pub token: Token,
    /// State Machine status
    pub status: TxStatus,
    /// code word
    #[serde(rename = "codeWord")]
    pub code_word: String,
    /// amount to be sent
    pub amount: u128,
    /// signed call payload (signed hash of the transaction)
    #[serde(rename = "signedCallPayload")]
    pub signed_call_payload: Option<Vec<u8>>,
    /// call payload (hash of transaction)
    #[serde(rename = "callPayload")]
    pub call_payload: Option<[u8; 32]>,
    // /// used for simplifying tx identification
    // pub code_word: String,
    // pub sender_name: String,
    /// Inbound Request id for p2p
    #[serde(rename = "inboundReqId")]
    #[serde(serialize_with = "serialize_u64_as_string")]
    #[serde(deserialize_with = "deserialize_u64_flexible")]
    pub inbound_req_id: Option<u64>,
    /// Outbound Request id for p2p
    #[serde(rename = "outboundReqId")]
    #[serde(serialize_with = "serialize_u64_as_string")]
    #[serde(deserialize_with = "deserialize_u64_flexible")]
    pub outbound_req_id: Option<u64>,
    /// stores the current nonce of the transaction per vane not the nonce for the blockchain network
    #[serde(rename = "txNonce")]
    pub tx_nonce: u32,
}

#[cfg(feature = "wasm")]
impl TxStateMachine {
    pub fn from_js_value_unconditional(value: JsValue) -> Result<Self, JsError> {
        let tx_state_machine: TxStateMachine = serde_wasm_bindgen::from_value(value)
            .map_err(|e| JsError::new(&format!("Failed to deserialize TxStateMachine: {:?}", e)))?;
        Ok(tx_state_machine)
    }
}

impl TxStateMachine {
    pub fn recv_confirmation_passed(&mut self) {
        self.status = TxStatus::RecvAddrConfirmationPassed
    }
    pub fn recv_confirmation_failed(&mut self) {
        self.status = TxStatus::RecvAddrFailed
    }
    pub fn recv_confirmed(&mut self) {
        self.status = TxStatus::RecvAddrConfirmed
    }
    pub fn sender_confirmation(&mut self) {
        self.status = TxStatus::SenderConfirmed
    }
    pub fn sender_confirmation_failed(&mut self) {
        self.status = TxStatus::SenderConfirmationfailed
    }
    pub fn tx_submission_failed(&mut self, reason: String) {
        self.status = TxStatus::FailedToSubmitTxn(reason)
    }
    pub fn tx_submission_passed(&mut self, tx_hash: [u8; 32]) {
        self.status = TxStatus::TxSubmissionPassed(tx_hash)
    }
    pub fn net_confirmed(&mut self) {
        self.status = TxStatus::NetConfirmed
    }
    pub fn recv_not_registered(&mut self) {
        self.status = TxStatus::ReceiverNotRegistered
    }
    pub fn increment_nonce(&mut self) {
        self.tx_nonce += 1
    }
    pub fn get_tx_hash(&self) -> [u8; 32] {
        match self.network {
            ChainSupported::Polkadot => blake2_256(&self.encode()[..]),
            ChainSupported::Solana => sha2_256(&self.encode()[..]),
            // EVM
            _ => keccak_256(&self.encode()[..]),
        }
    }
}

// helper for hashing p2p swarm request ids
pub trait HashId: Hash {
    fn get_hash_id(&self) -> u64 {
        let mut req_id_hash = XxHash64::default();
        self.hash(&mut req_id_hash);
        req_id_hash.finish()
    }
}

impl HashId for OutboundRequestId {}
impl HashId for InboundRequestId {}

// ================================================================================= //

#[derive(Debug)]
pub enum NetworkCommand {
    SendRequest {
        request: Vec<u8>,
        peer_id: PeerId,
        target_multi_addr: Multiaddr,
    },
    SendResponse {
        response: Vec<u8>,
        channel: ResponseChannel<Result<Vec<u8>, Error>>,
    },
    WasmSendRequest {
        request: TxStateMachine,
        peer_id: PeerId,
        target_multi_addr: Multiaddr,
    },
    WasmSendResponse {
        response: Result<TxStateMachine, String>,
        channel: ResponseChannel<Result<TxStateMachine, String>>,
    },
    Dial {
        target_multi_addr: Multiaddr,
        target_peer_id: PeerId,
    },
    Close {
        peer_id: PeerId,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum SwarmMessage {
    Request {
        data: Vec<u8>,
        inbound_id: InboundRequestId,
    },
    Response {
        data: Vec<u8>,
        outbound_id: OutboundRequestId,
    },
    WasmRequest {
        data: TxStateMachine,
        inbound_id: InboundRequestId,
    },
    WasmResponse {
        data: TxStateMachine,
        outbound_id: OutboundRequestId,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct UserMetrics {
    pub user_account: UserAccount,
    pub total_success_txns: Vec<DbTxStateMachine>,
    pub total_failed_txns: Vec<DbTxStateMachine>,
    pub saved_target_peers: (Vec<String>, String),
}

/// Transaction data structure to store in the db
#[derive(Clone, Debug, Deserialize, Serialize, Encode, Decode)]
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

impl Default for Token {
    fn default() -> Self {
        Self::Eth
    }
}

impl From<Token> for String {
    fn from(value: Token) -> Self {
        match value {
            Token::Dot => "Dot".to_string(),
            Token::Bnb => "Bnb".to_string(),
            Token::Sol => "Sol".to_string(),
            Token::Eth => "Eth".to_string(),
            Token::UsdtSol => "UsdtSol".to_string(),
            Token::UsdcSol => "UsdcSol".to_string(),
            Token::UsdtEth => "UsdtEth".to_string(),
            Token::UsdcEth => "UsdcEth".to_string(),
            Token::UsdtDot => "UsdtDot".to_string(),
        }
    }
}

impl From<&str> for Token {
    fn from(value: &str) -> Self {
        match value {
            "Dot" => Token::Dot,
            "Bnb" => Token::Bnb,
            "Sol" => Token::Sol,
            "Eth" => Token::Eth,
            "UsdtSol" => Token::UsdtSol,
            "UsdcSol" => Token::UsdcSol,
            "UsdtEth" => Token::UsdtEth,
            "UsdcEth" => Token::UsdcEth,
            "UsdtDot" => Token::UsdtDot,
            _ => unreachable!(),
        }
    }
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

impl From<ChainSupported> for String {
    fn from(value: ChainSupported) -> Self {
        match value {
            ChainSupported::Polkadot => "Polkadot".to_string(),
            ChainSupported::Ethereum => "Ethereum".to_string(),
            ChainSupported::Bnb => "Bnb".to_string(),
            ChainSupported::Solana => "Solana".to_string(),
        }
    }
}

impl From<&str> for ChainSupported {
    fn from(value: &str) -> Self {
        println!("value: {value}");
        match value {
            "Polkadot" => ChainSupported::Polkadot,
            "Ethereum" => ChainSupported::Ethereum,
            "Bnb" => ChainSupported::Bnb,
            "Solana" => ChainSupported::Solana,
            _ => ChainSupported::Ethereum,
        }
    }
}

impl ChainSupported {
    // Method to get the URL based on the network type
    pub fn url(&self) -> String {
        {
            // Load .env file if it exists
            dotenv().ok();
        }

        match self {
            ChainSupported::Polkadot => std::env::var("POLKADOT_RPC_URL")
                .unwrap_or_else(|_| "wss://polkadot-rpc.dwellir.com".to_string()),
            ChainSupported::Ethereum => std::env::var("ETHEREUM_RPC_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:8545".to_string()),
            ChainSupported::Bnb => std::env::var("BNB_RPC_URL")
                .unwrap_or_else(|_| "https://bsc-dataseed.binance.org/".to_string()),
            ChainSupported::Solana => std::env::var("SOLANA_RPC_URL")
                .unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".to_string()),
        }
    }
}

impl std::fmt::Display for ChainSupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", String::from(*self))
    }
}

/// User account
#[derive(Clone, Eq, Debug, PartialEq, Deserialize, Serialize, Encode, Decode)]
pub struct UserAccount {
    pub multi_addr: String,
    pub accounts: Vec<(String, ChainSupported)>,
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

//  --------------------------- REMOTE DB ------------------------------------------------------------ //

// ----------------------- DB related ---------------------------------------------------------- //
#[derive(Serialize, Deserialize, Encode, Decode)]
pub struct Ports {
    pub rpc: u16,
    pub p_2_p_port: u16,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Encode, Decode)]
pub struct AccountInfo {
    pub account: String,
    pub network: ChainSupported,
}
/// db interface
#[allow(async_fn_in_trait)]
pub trait DbWorkerInterface: Sized {
    async fn initialize_db_client(file_url: &str) -> Result<Self, anyhow::Error>;

    async fn set_user_account(&self, user: UserAccount) -> Result<(), anyhow::Error>;
    async fn update_user_account(
        &self,
        account_id: String,
        network: ChainSupported,
    ) -> Result<UserAccount, anyhow::Error>;

    async fn get_nonce(&self) -> Result<u32, anyhow::Error>;

    // get all related network id accounts
    async fn get_user_account(&self) -> Result<UserAccount, anyhow::Error>;

    async fn update_success_tx(&self, tx_state: DbTxStateMachine) -> Result<(), anyhow::Error>;

    async fn update_failed_tx(&self, tx_state: DbTxStateMachine) -> Result<(), anyhow::Error>;
    async fn get_failed_txs(&self) -> Result<Vec<DbTxStateMachine>, anyhow::Error>;

    async fn get_total_value_success(&self) -> Result<u64, anyhow::Error>;
    async fn get_total_value_failed(&self) -> Result<u64, anyhow::Error>;

    // record the user of this app

    async fn get_success_txs(&self) -> Result<Vec<DbTxStateMachine>, anyhow::Error>;

    async fn increment_nonce(&self) -> Result<(), anyhow::Error>;

    // saved peers interacted with
    async fn record_saved_user_peers(
        &self,
        acc_id: String,
        multi_addr: String,
    ) -> Result<(), anyhow::Error>;

    // get saved peers
    async fn get_saved_user_peers(&self, account_id: String) -> Result<String, anyhow::Error>;

    // get all saved peers
    async fn get_all_saved_peers(&self) -> Result<(Vec<String>, String), anyhow::Error>;

    // delete a specific saved peer
    async fn delete_saved_peer(&self, peer_id: &str) -> Result<(), anyhow::Error>;
}

/// Node error reporting structure
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NodeError {
    pub timestamp: u64,
    pub error_type: String, // "network", "database", "execution", "rpc"
    pub message: String,
    pub details: Option<String>,
}
