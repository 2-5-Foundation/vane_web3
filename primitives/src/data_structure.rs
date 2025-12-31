//! All data structure related to transaction processing and updating
extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;
use anyhow::Error;
use codec::{Decode, Encode};
use core::hash::{Hash, Hasher};
use libp2p::request_response::{InboundRequestId, OutboundRequestId, ResponseChannel};
use libp2p::{Multiaddr, PeerId};
use serde::de::DeserializeOwned;
use serde::de::Error as SerdeError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use std::fmt::Debug;
use twox_hash::XxHash64;
#[cfg(feature = "wasm")]
use wasm_bindgen::{JsError, JsValue};

// Ethereum signature preimage prefix according to EIP-191
// keccak256("\x19Ethereum Signed Message:\n" + len(message) + message))
pub const ETH_SIG_MSG_PREFIX: &str = "\x19Ethereum Signed Message:\n";

/// DHT response structure for host function communication
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, Encode, Decode)]
pub struct DHTResponse {
    pub success: bool,
    pub value: Option<String>,
    pub error: Option<String>,
    pub random: u32,
}

/// Connection state for P2P relay connections
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Encode, Decode)]
pub enum ConnectionState {
    Connected(u64),    // Unix timestamp in seconds
    Disconnected(u64), // Unix timestamp in seconds
}

impl Default for ConnectionState {
    fn default() -> Self {
        #[cfg(target_arch = "wasm32")]
        {
            use js_sys::Date;
            let now = (Date::now() / 1000.0) as u64; // Convert from ms to seconds
            ConnectionState::Disconnected(now)
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            use std::time::{SystemTime, UNIX_EPOCH};
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            ConnectionState::Disconnected(now)
        }
    }
}

/// Node connection status information
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Encode, Decode)]
pub struct NodeConnectionStatus {
    pub relay_connected: bool,
    pub peer_id: String,
    pub relay_address: String,
    pub connection_uptime_seconds: Option<u64>,
    pub last_connection_change: Option<u64>, // Unix timestamp
}

/// tx state
#[derive(Clone, Debug, PartialEq, Serialize, Encode, Decode)]
#[serde(tag = "type", content = "data")]
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
    /// any errors related to the tx
    TxError(String),
    /// if submission passed (tx-hash)
    TxSubmissionPassed { hash: Vec<u8> },
    /// if the receiver has not registered to vane yet
    ReceiverNotRegistered,
    /// if the transaction is reverted
    Reverted(String),
}

impl Default for TxStatus {
    fn default() -> Self {
        Self::Genesis
    }
}

impl<'de> Deserialize<'de> for TxStatus {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;

        // Helper to map string tag -> variant
        fn from_tag<'a, E: SerdeError>(
            tag: &'a str,
            val: Option<&serde_json::Value>,
        ) -> Result<TxStatus, E> {
            match tag {
                "Genesis" => Ok(TxStatus::Genesis),
                "RecvAddrConfirmed" => Ok(TxStatus::RecvAddrConfirmed),
                "RecvAddrConfirmationPassed" => Ok(TxStatus::RecvAddrConfirmationPassed),
                "NetConfirmed" => Ok(TxStatus::NetConfirmed),
                "SenderConfirmed" => Ok(TxStatus::SenderConfirmed),
                "SenderConfirmationfailed" => Ok(TxStatus::SenderConfirmationfailed),
                "RecvAddrFailed" => Ok(TxStatus::RecvAddrFailed),
                "ReceiverNotRegistered" => Ok(TxStatus::ReceiverNotRegistered),
                "FailedToSubmitTxn" => {
                    let reason = val
                        .and_then(|v| v.as_str())
                        .unwrap_or("Submission failed")
                        .to_string();
                    Ok(TxStatus::FailedToSubmitTxn(reason))
                }
                "TxSubmissionPassed" => {
                    let hash = val
                        .and_then(|v| v.get("hash"))
                        .and_then(|h| h.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|b| b.as_u64().map(|n| n as u8))
                                .collect::<Vec<u8>>()
                        })
                        .unwrap_or_default();

                    Ok(TxStatus::TxSubmissionPassed { hash })
                }
                "TxError" => {
                    let reason = val
                        .and_then(|v| v.as_str())
                        .unwrap_or("Transaction error")
                        .to_string();
                    Ok(TxStatus::TxError(reason))
                }
                "Reverted" => {
                    let reason = val
                        .and_then(|v| v.as_str())
                        .unwrap_or("Intended receiver not met")
                        .to_string();
                    Ok(TxStatus::Reverted(reason))
                }
                other => Err(E::custom(format!("unknown TxStatus variant: {other}"))),
            }
        }

        match value {
            // Simple string variant name
            serde_json::Value::String(s) => from_tag::<D::Error>(&s, None),

            // Object forms: { Variant: value } or { type: Variant, data: X }
            serde_json::Value::Object(map) => {
                if let Some(t) = map.get("type").and_then(|v| v.as_str()) {
                    let val = map.get("data");
                    return from_tag::<D::Error>(t, val);
                }

                // Single-key map: { "Variant": value }
                if map.len() == 1 {
                    let (k, v) = map.iter().next().unwrap();
                    return from_tag::<D::Error>(k, Some(v));
                }

                Err(D::Error::custom("invalid object for TxStatus"))
            }

            // Array forms: ["Variant"], ["Variant", value]
            serde_json::Value::Array(arr) => {
                if arr.is_empty() {
                    return Err(D::Error::custom("empty array for TxStatus"));
                }
                let tag = arr[0]
                    .as_str()
                    .ok_or_else(|| D::Error::custom("first element must be string variant"))?;
                let val = if arr.len() > 1 { Some(&arr[1]) } else { None };
                from_tag::<D::Error>(tag, val)
            }

            _ => Err(D::Error::custom("invalid type for TxStatus")),
        }
    }
}

impl From<TxStatus> for String {
    fn from(status: TxStatus) -> Self {
        match status {
            TxStatus::RecvAddrFailed => "Receiver address failed".to_string(),
            _ => unimplemented!("not used"),
        }
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

fn serialize_u128_as_string<S>(value: &u128, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&value.to_string())
}

fn deserialize_u128_from_string<'de, D>(deserializer: D) -> Result<u128, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    match value {
        Value::String(s) => {
            if let Some(stripped) = s.strip_prefix("0x") {
                u128::from_str_radix(stripped, 16).map_err(D::Error::custom)
            } else {
                s.parse::<u128>().map_err(D::Error::custom)
            }
        }
        Value::Number(n) => {
            if let Some(num) = n.as_u64() {
                Ok(num as u128)
            } else {
                Err(D::Error::custom("number out of range for u128"))
            }
        }
        _ => Err(D::Error::custom("Expected string or number for u128")),
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

/// Unsigned EIP-1559 transaction fields
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, Encode, Decode)]
pub struct UnsignedEip1559 {
    pub to: String,
    #[serde(serialize_with = "serialize_u128_as_string")]
    #[serde(deserialize_with = "deserialize_u128_from_string")]
    pub value: u128,
    #[serde(rename = "chainId")]
    pub chain_id: u64,
    pub nonce: u64,
    pub gas: u64,
    #[serde(rename = "maxFeePerGas")]
    pub max_fee_per_gas: u64,
    #[serde(rename = "maxPriorityFeePerGas")]
    pub max_priority_fee_per_gas: u64,
    pub data: Option<String>,
    #[serde(rename = "accessList")]
    pub access_list: Option<Vec<()>>,
    #[serde(rename = "type")]
    pub tx_type: String, // "eip1559"
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, Encode, Decode)]
pub struct UnsignedBnbLegacy {
    pub to: String,
    #[serde(serialize_with = "serialize_u128_as_string")]
    #[serde(deserialize_with = "deserialize_u128_from_string")]
    pub value: u128,
    #[serde(rename = "chainId")]
    pub chain_id: u64,
    pub nonce: u64,
    pub gas: u64,
    #[serde(rename = "gasPrice")]
    pub gas_price: u64,
    #[serde(rename = "data")]
    pub data: Option<String>,
    #[serde(rename = "type")]
    pub tx_type: String, // "legacy"
}

/// Transaction data structure state machine, passed in rpc and p2p swarm
#[derive(Clone, Default, PartialEq, Debug, Deserialize, Serialize, Encode, Decode)]
pub struct TxStateMachine {
    #[serde(rename = "senderAddress")]
    pub sender_address: String,
    #[serde(rename = "senderPublicKey")]
    pub sender_public_key: Option<String>,
    #[serde(rename = "receiverAddress")]
    pub receiver_address: String,
    #[serde(rename = "receiverPublicKey")]
    pub receiver_public_key: Option<String>,
    /// hashed sender and receiver address to bind the addresses while sending
    #[serde(rename = "multiId")]
    pub multi_id: [u8; 32],
    /// signature of the receiver id (Signature)
    #[serde(rename = "recvSignature")]
    pub recv_signature: Option<Vec<u8>>,
    /// token
    pub token: Token,
    /// State Machine status
    pub status: TxStatus,
    /// code word
    #[serde(rename = "codeWord")]
    pub code_word: String,
    /// amount to be sent
    #[serde(serialize_with = "serialize_u128_as_string")]
    #[serde(deserialize_with = "deserialize_u128_from_string")]
    pub amount: u128,
    /// fees amount
    #[serde(rename = "feesAmount")]
    pub fees_amount: f32,
    /// signed call payload (signed hash of the transaction)
    #[serde(rename = "signedCallPayload")]
    pub signed_call_payload: Option<Vec<u8>>,
    /// call payload (hash of transaction and raw transaction bytes)
    #[serde(rename = "callPayload")]
    pub call_payload: Option<ChainTransactionType>,
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
    /// monotonic version for conflict/race resolution across async boundaries
    #[serde(rename = "txVersion")]
    pub tx_version: u32,
    /// sender address network
    #[serde(rename = "senderAddressNetwork")]
    pub sender_address_network: ChainSupported,
    /// receiver address network
    #[serde(rename = "receiverAddressNetwork")]
    pub receiver_address_network: ChainSupported,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Encode, Decode)]
pub enum ChainTransactionType {
    #[serde(rename = "ethereum")]
    Ethereum {
        #[serde(rename = "ethUnsignedTxFields")]
        eth_unsigned_tx_fields: UnsignedEip1559,
        #[serde(rename = "callPayload")]
        call_payload: (Vec<u8>, Vec<u8>),
    },

    #[serde(rename = "solana")]
    Solana {
        #[serde(rename = "callPayload")]
        call_payload: Vec<u8>,
        #[serde(rename = "latestBlockHeight")]
        latest_block_height: u64,
    },
    #[serde(rename = "bnb")]
    Bnb {
        #[serde(rename = "callPayload")]
        call_payload: (Vec<u8>, Vec<u8>),
        #[serde(rename = "bnbLegacyTxFields")]
        bnb_legacy_tx_fields: UnsignedBnbLegacy,
    },
}
pub trait TxStateMachineLike:
    Encode + Decode + Debug + Clone + PartialEq + Serialize + DeserializeOwned
{
}

impl<T> TxStateMachineLike for T where
    T: Encode + Decode + Debug + Clone + PartialEq + Serialize + DeserializeOwned
{
}

pub type SignatureType = Vec<u8>;
#[derive(Debug, Clone, PartialEq, Encode, Decode)]
pub struct VanePayload<D, E>
where
    D: TxStateMachineLike + DeserializeOwned, // Add explicit DeserializeOwned bound
    E: Encode + Decode + Debug + Clone + PartialEq + Serialize + DeserializeOwned,
{
    pub data: D,
    pub extra_data: E,
}

impl<D, E> VanePayload<D, E>
where
    D: TxStateMachineLike + DeserializeOwned,
    E: Encode + Decode + Debug + Clone + PartialEq + Serialize + DeserializeOwned,
{
    pub fn new(data: D, extra_data: E) -> Self {
        Self { data, extra_data }
    }
}

#[cfg(feature = "wasm")]
impl TxStateMachine {
    pub fn from_js_value_unconditional(value: JsValue) -> Result<Self, JsError> {
        let tx_state_machine: TxStateMachine = serde_wasm_bindgen::from_value(value)
            .map_err(|e| JsError::new(&format!("Failed to deserialize TxStateMachine: {:?}", e)))?;
        Ok(tx_state_machine)
    }
}

#[cfg(feature = "wasm")]
impl Token {
    pub fn from_js_value_unconditional(value: JsValue) -> Result<Self, JsError> {
        let token: Token = serde_wasm_bindgen::from_value(value)
            .map_err(|e| JsError::new(&format!("Failed to deserialize Token: {:?}", e)))?;
        Ok(token)
    }
}

#[cfg(feature = "wasm")]
impl ChainSupported {
    pub fn from_js_value_unconditional(value: JsValue) -> Result<Self, JsError> {
        let chain_supported: ChainSupported = serde_wasm_bindgen::from_value(value)
            .map_err(|e| JsError::new(&format!("Failed to deserialize ChainSupported: {:?}", e)))?;
        Ok(chain_supported)
    }
}

#[cfg(feature = "wasm")]
impl StorageExport {
    pub fn from_js_value_unconditional(value: JsValue) -> Result<Self, JsError> {
        let storage_export: StorageExport = serde_wasm_bindgen::from_value(value)
            .map_err(|e| JsError::new(&format!("Failed to deserialize StorageExport: {:?}", e)))?;
        Ok(storage_export)
    }
}

impl TxStateMachine {
    pub fn increment_version(&mut self) {
        self.tx_version = self.tx_version.saturating_add(1);
    }
    pub fn recv_confirmation_passed(&mut self) {
        self.status = TxStatus::RecvAddrConfirmationPassed
    }
    pub fn recv_confirmation_failed(&mut self) {
        let reason: String = TxStatus::RecvAddrFailed.into();
        self.status = TxStatus::Reverted(reason)
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
    pub fn tx_submission_passed(&mut self, tx_hash: Vec<u8>) {
        self.status = TxStatus::TxSubmissionPassed { hash: tx_hash }
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
    pub fn set_sender_public_key(&mut self, public_key: String) {
        self.sender_public_key = Some(public_key)
    }
    pub fn set_receiver_public_key(&mut self, public_key: String) {
        self.receiver_public_key = Some(public_key)
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

// Wrapper for ttl
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, Encode, Decode)]
pub struct TtlWrapper<T: Encode + Decode + Clone> {
    pub value: T,
    pub ttl: u32,
}

impl<T: Encode + Decode + Clone> TtlWrapper<T> {
    pub fn new(value: T, ttl: u32) -> Self {
        Self { value, ttl }
    }
    pub fn is_expired(&self, now: u32, time_to_live: u32) -> bool {
        self.ttl + time_to_live < now
    }
    pub fn update_value(&mut self, value: T) {
        self.value = value;
    }
    pub fn get_value(&self) -> &T {
        &self.value
    }
}
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
        request: VanePayload<TxStateMachine, SignatureType>,
    },
    WasmSendResponse {
        response: Result<VanePayload<TxStateMachine, SignatureType>, String>,
    },
    Dial {
        target_multi_addr: Multiaddr,
        target_peer_id: PeerId,
        oneshot_sender: tokio_with_wasm::alias::sync::oneshot::Sender<Result<(), anyhow::Error>>,
    },
    GetDhtPeer {
        target_acc_id: String,
        response_sender: tokio_with_wasm::alias::sync::oneshot::Sender<Result<u32, Error>>,
    },
    AddDhtAccount {
        account_id: String,
        value: String,
    },
    FetchPendingTransactions {
        sig: SignatureType,
        account_id: String,
    },
    RevertTransaction {
        sig: SignatureType,
        account_id: String,
        data: TxStateMachine,
    },
    ConfirmTransaction {
        sig: SignatureType,
        account_id: String,
        data: TxStateMachine,
    },
    TxSubmissionUpdate {
        sig: SignatureType,
        account_id: String,
        data: TxStateMachine,
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
    },
    WasmResponse {
        data: TxStateMachine,
    },
    PendingTransactionsFetched {
        address: String,
        transactions: Vec<TxStateMachine>,
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
    #[serde(serialize_with = "serialize_u128_as_string")]
    #[serde(deserialize_with = "deserialize_u128_from_string")]
    pub amount: u128,
    // token
    pub token: Token,
    // sender
    pub sender: String,
    // receiver
    pub receiver: String,
    // sender address network
    pub sender_network: ChainSupported,
    // receiver address network
    pub receiver_network: ChainSupported,
    // status
    pub success: bool,
}

/// Supported tokens (flexible)
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, Encode, Decode)]
pub enum Token {
    /// Ethereum ecosystem tokens
    Ethereum(EthereumToken),
    /// BNB Smart Chain ecosystem tokens  
    Bnb(BnbToken),
    /// Polkadot ecosystem tokens
    Polkadot(PolkadotToken),
    /// Solana ecosystem tokens
    Solana(SolanaToken),
    /// TRON ecosystem tokens
    Tron(TronToken),
    /// Optimism ecosystem tokens
    Optimism(OptimismToken),
    /// Arbitrum ecosystem tokens
    Arbitrum(ArbitrumToken),
    /// Polygon ecosystem tokens
    Polygon(PolygonToken),
    /// Base ecosystem tokens
    Base(BaseToken),
    /// Bitcoin ecosystem tokens
    Bitcoin(BitcoinToken),
}

/// Ethereum ecosystem tokens
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, Encode, Decode)]
pub enum EthereumToken {
    /// Native ETH
    ETH,
    /// ERC-20 tokens with name, contract address, and decimals
    ERC20 {
        name: String,
        address: String,
        decimals: u8,
    },
}

/// BNB Smart Chain ecosystem tokens
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, Encode, Decode)]
pub enum BnbToken {
    /// Native BNB
    BNB,
    /// BEP-20 tokens with name, contract address, and decimals
    BEP20 {
        name: String,
        address: String,
        decimals: u8,
    },
}

/// Polkadot ecosystem tokens
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, Encode, Decode)]
pub enum PolkadotToken {
    /// Native DOT
    DOT,
    /// Ecosystem tokens with name and asset ID
    Asset { name: String, id: String },
}

/// Solana ecosystem tokens
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, Encode, Decode)]
pub enum SolanaToken {
    /// Native SOL
    SOL,
    /// SPL tokens with name, mint address, and decimals
    SPL {
        name: String,
        address: String,
        decimals: u8,
    },
}

/// TRON ecosystem tokens
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, Encode, Decode)]
pub enum TronToken {
    /// Native TRX
    TRX,
    /// TRC-20 tokens with name, contract address, and decimals
    TRC20 {
        name: String,
        address: String,
        decimals: u8,
    },
}

/// Optimism ecosystem tokens
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, Encode, Decode)]
pub enum OptimismToken {
    /// Native ETH (on Optimism)
    ETH,
    /// ERC-20 tokens with name, contract address, and decimals
    ERC20 {
        name: String,
        address: String,
        decimals: u8,
    },
}

/// Arbitrum ecosystem tokens
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, Encode, Decode)]
pub enum ArbitrumToken {
    /// Native ETH (on Arbitrum)
    ETH,
    /// ERC-20 tokens with name, contract address, and decimals
    ERC20 {
        name: String,
        address: String,
        decimals: u8,
    },
}

/// Polygon ecosystem tokens
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, Encode, Decode)]
pub enum PolygonToken {
    /// Native POL
    POL,
    /// ERC-20 tokens with name, contract address, and decimals
    ERC20 {
        name: String,
        address: String,
        decimals: u8,
    },
}

/// Base ecosystem tokens
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, Encode, Decode)]
pub enum BaseToken {
    /// Native ETH (on Base)
    ETH,
    /// ERC-20 tokens with name, contract address, and decimals
    ERC20 {
        name: String,
        address: String,
        decimals: u8,
    },
}

/// Bitcoin ecosystem tokens
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, Encode, Decode)]
pub enum BitcoinToken {
    /// Native BTC (only token on Bitcoin)
    BTC,
}

impl Default for Token {
    fn default() -> Self {
        Token::Ethereum(EthereumToken::ETH)
    }
}

impl From<Token> for String {
    fn from(value: Token) -> Self {
        match value {
            Token::Ethereum(EthereumToken::ETH) => "Ethereum:ETH".to_string(),
            Token::Ethereum(EthereumToken::ERC20 {
                name,
                address,
                decimals: _,
            }) => {
                format!("Ethereum:{} ({})", name, address)
            }
            Token::Bnb(BnbToken::BNB) => "BNB:BNB".to_string(),
            Token::Bnb(BnbToken::BEP20 {
                name,
                address,
                decimals: _,
            }) => format!("BNB:{} ({})", name, address),
            Token::Polkadot(PolkadotToken::DOT) => "Polkadot:DOT".to_string(),
            Token::Polkadot(PolkadotToken::Asset { name, id }) => {
                format!("Polkadot:{} ({})", name, id)
            }
            Token::Solana(SolanaToken::SOL) => "Solana:SOL".to_string(),
            Token::Solana(SolanaToken::SPL {
                name,
                address,
                decimals: _,
            }) => {
                format!("Solana:{} ({})", name, address)
            }
            Token::Tron(TronToken::TRX) => "TRON:TRX".to_string(),
            Token::Tron(TronToken::TRC20 {
                name,
                address,
                decimals: _,
            }) => {
                format!("TRON:{} ({})", name, address)
            }
            Token::Optimism(OptimismToken::ETH) => "Optimism:ETH".to_string(),
            Token::Optimism(OptimismToken::ERC20 {
                name,
                address,
                decimals: _,
            }) => {
                format!("Optimism:{} ({})", name, address)
            }
            Token::Arbitrum(ArbitrumToken::ETH) => "Arbitrum:ETH".to_string(),
            Token::Arbitrum(ArbitrumToken::ERC20 {
                name,
                address,
                decimals: _,
            }) => {
                format!("Arbitrum:{} ({})", name, address)
            }
            Token::Polygon(PolygonToken::POL) => "Polygon:POL".to_string(),
            Token::Polygon(PolygonToken::ERC20 {
                name,
                address,
                decimals: _,
            }) => {
                format!("Polygon:{} ({})", name, address)
            }
            Token::Base(BaseToken::ETH) => "Base:ETH".to_string(),
            Token::Base(BaseToken::ERC20 {
                name,
                address,
                decimals: _,
            }) => {
                format!("Base:{} ({})", name, address)
            }
            Token::Bitcoin(BitcoinToken::BTC) => "Bitcoin:BTC".to_string(),
        }
    }
}

impl From<Token> for ChainSupported {
    fn from(value: Token) -> Self {
        match value {
            Token::Ethereum(_) => ChainSupported::Ethereum,
            Token::Bnb(_) => ChainSupported::Bnb,
            Token::Polkadot(_) => ChainSupported::Polkadot,
            Token::Solana(_) => ChainSupported::Solana,
            Token::Tron(_) => ChainSupported::Tron,
            Token::Optimism(_) => ChainSupported::Optimism,
            Token::Arbitrum(_) => ChainSupported::Arbitrum,
            Token::Polygon(_) => ChainSupported::Polygon,
            Token::Base(_) => ChainSupported::Base,
            Token::Bitcoin(_) => ChainSupported::Bitcoin,
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
    Tron,
    Optimism,
    Arbitrum,
    Polygon,
    Base,
    Bitcoin,
}

impl Default for ChainSupported {
    fn default() -> Self {
        ChainSupported::Ethereum
    }
}

impl From<ChainSupported> for String {
    fn from(value: ChainSupported) -> Self {
        match value {
            ChainSupported::Polkadot => "Polkadot".to_string(),
            ChainSupported::Ethereum => "Ethereum".to_string(),
            ChainSupported::Bnb => "Bnb".to_string(),
            ChainSupported::Solana => "Solana".to_string(),
            ChainSupported::Tron => "Tron".to_string(),
            ChainSupported::Optimism => "Optimism".to_string(),
            ChainSupported::Arbitrum => "Arbitrum".to_string(),
            ChainSupported::Polygon => "Polygon".to_string(),
            ChainSupported::Base => "Base".to_string(),
            ChainSupported::Bitcoin => "Bitcoin".to_string(),
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
            "Tron" => ChainSupported::Tron,
            "Optimism" => ChainSupported::Optimism,
            "Arbitrum" => ChainSupported::Arbitrum,
            "Polygon" => ChainSupported::Polygon,
            "Base" => ChainSupported::Base,
            "Bitcoin" => ChainSupported::Bitcoin,
            _ => ChainSupported::Ethereum,
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
    async fn set_nonce(&self, nonce: u32) -> Result<(), anyhow::Error>;

    // get all related network id accounts
    async fn get_user_account(&self) -> Result<UserAccount, anyhow::Error>;

    async fn update_success_tx(&self, tx_state: DbTxStateMachine) -> Result<(), anyhow::Error>;

    async fn update_failed_tx(&self, tx_state: DbTxStateMachine) -> Result<(), anyhow::Error>;
    async fn get_failed_txs(&self) -> Result<Vec<DbTxStateMachine>, anyhow::Error>;

    async fn get_total_value_success(&self) -> Result<u128, anyhow::Error>;
    async fn get_total_value_failed(&self) -> Result<u128, anyhow::Error>;

    // record the user of this app

    async fn get_success_txs(&self) -> Result<Vec<DbTxStateMachine>, anyhow::Error>;

    async fn increment_nonce(&self) -> Result<(), anyhow::Error>;

    /// Get all saved peers with their associated account IDs
    async fn get_all_saved_peers_info(&self) -> Result<Vec<SavedPeerInfo>, anyhow::Error>;

    /// Get saved peer ID by account ID
    async fn get_saved_user_peers(&self, account_id: String) -> Result<String, anyhow::Error>;

    /// Delete a specific saved peer by peer_id
    async fn delete_saved_peer(&self, peer_id: &str) -> Result<(), anyhow::Error>;

    /// Record a saved peer with account_id and multi_addr
    async fn record_saved_user_peers(
        &self,
        account_id: String,
        multi_addr: String,
    ) -> Result<(), anyhow::Error>;
}

/// Node error reporting structure
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NodeError {
    pub timestamp: u64,
    pub error_type: String, // "network", "database", "execution", "rpc"
    pub message: String,
    pub details: Option<String>,
}

/// Information about a saved peer
#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode)]
pub struct SavedPeerInfo {
    /// The peer's ID
    pub peer_id: String,
    /// The peer's multi-address
    pub multi_addr: String,
    /// All account IDs associated with this peer
    pub account_ids: Vec<String>,
}

/// Complete database storage export structure
/// Contains all data from the database using getter methods
#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode)]
pub struct StorageExport {
    /// User account information (multi-address and associated chain accounts)
    pub user_account: Option<UserAccount>,

    /// Current nonce value for transaction ordering
    pub nonce: u32,

    /// All successful transactions
    pub success_transactions: Vec<DbTxStateMachine>,

    /// All failed transactions  
    pub failed_transactions: Vec<DbTxStateMachine>,
}

// Backend events for JSON-RPC communication
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Encode, Decode)]
pub enum BackendEvent {
    SenderRequestReceived {
        address: String,
        data: Vec<u8>,
    },
    SenderRequestHandled {
        address: String,
        data: Vec<u8>,
    },
    SenderConfirmed {
        address: String,
        data: Vec<u8>,
    },
    SenderReverted {
        address: String,
        data: Vec<u8>,
    },
    ReceiverResponseReceived {
        address: String,
        data: Vec<u8>,
    },
    ReceiverResponseHandled {
        address: String,
        data: Vec<u8>,
    },
    PeerDisconnected {
        account_id: String,
    },
    DataExpired {
        multi_id: String,
        data: Vec<u8>,
    },
    PendingTransactionsFetched {
        address: String,
        transactions: Vec<TxStateMachine>,
    },
    TxSubmitted {
        address: String,
        data: Vec<u8>,
    },
}

impl BackendEvent {
    pub fn get_address(&self) -> String {
        match self {
            BackendEvent::SenderRequestReceived { address, .. } => address.clone(),
            BackendEvent::SenderRequestHandled { address, .. } => address.clone(),
            BackendEvent::SenderConfirmed { address, .. } => address.clone(),
            BackendEvent::SenderReverted { address, .. } => address.clone(),
            BackendEvent::ReceiverResponseReceived { address, .. } => address.clone(),
            BackendEvent::ReceiverResponseHandled { address, .. } => address.clone(),
            BackendEvent::PeerDisconnected { account_id, .. } => account_id.clone(),
            BackendEvent::DataExpired { multi_id, .. } => multi_id.clone(),
            BackendEvent::PendingTransactionsFetched { address, .. } => address.clone(),
            BackendEvent::TxSubmitted { address, .. } => address.clone(),
        }
    }

    pub fn get_data(&self) -> Vec<u8> {
        match self {
            BackendEvent::SenderRequestReceived { data, .. } => data.clone(),
            BackendEvent::SenderRequestHandled { data, .. } => data.clone(),
            BackendEvent::SenderConfirmed { data, .. } => data.clone(),
            BackendEvent::SenderReverted { data, .. } => data.clone(),
            BackendEvent::ReceiverResponseReceived { data, .. } => data.clone(),
            BackendEvent::ReceiverResponseHandled { data, .. } => data.clone(),
            BackendEvent::DataExpired { data, .. } => data.clone(),
            BackendEvent::TxSubmitted { data, .. } => data.clone(),
            _ => unimplemented!(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Encode, Decode)]
pub enum SystemNotification {
    PeerAdded {
        address: String,
        account_id: String,
        network: ChainSupported,
    },
    PeerRemoved {
        address: String,
    },
    RequestQueued {
        address: String,
    },
    RequestProcessed {
        address: String,
    },
}
