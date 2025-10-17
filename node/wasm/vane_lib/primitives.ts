export enum ChainSupported {
    Ethereum = "Ethereum",
    Polkadot = "Polkadot",
    Bnb = "Bnb",
    Solana = "Solana",
    Tron = "Tron",
    Optimism = "Optimism",
    Arbitrum = "Arbitrum",
    Polygon = "Polygon",
    Base = "Base",
    Bitcoin = "Bitcoin"
}

/** Ethereum ecosystem tokens */
export enum EthereumToken {
    ETH = "ETH",
    ERC20 = "ERC20"
}

/** ERC20 token with name and contract address */
export interface ERC20Token {
    name: string;
    address: string;
}

/** BNB Smart Chain ecosystem tokens */
export enum BnbToken {
    BNB = "BNB",
    BEP20 = "BEP20"
}

/** BEP20 token with name and contract address */
export interface BEP20Token {
    name: string;
    address: string;
}

/** Polkadot ecosystem tokens */
export enum PolkadotToken {
    DOT = "DOT",
    Asset = "Asset"
}

/** Polkadot asset with name and asset ID */
export interface PolkadotAsset {
    name: string;
    id: string;
}

/** Solana ecosystem tokens */
export enum SolanaToken {
    SOL = "SOL",
    SPL = "SPL"
}

/** SPL token with name and mint address */
export interface SPLToken {
    name: string;
    address: string;
}

/** TRON ecosystem tokens */
export enum TronToken {
    TRX = "TRX",
    TRC20 = "TRC20"
}

/** TRC20 token with name and contract address */
export interface TRC20Token {
    name: string;
    address: string;
}

/** Optimism ecosystem tokens */
export enum OptimismToken {
    ETH = "ETH",
    ERC20 = "ERC20"
}

/** Arbitrum ecosystem tokens */
export enum ArbitrumToken {
    ETH = "ETH",
    ERC20 = "ERC20"
}

/** Polygon ecosystem tokens */
export enum PolygonToken {
    POL = "POL",
    ERC20 = "ERC20"
}

/** Base ecosystem tokens */
export enum BaseToken {
    ETH = "ETH",
    ERC20 = "ERC20"
}

/** Bitcoin ecosystem tokens */
export enum BitcoinToken {
    BTC = "BTC"
}

/** Supported tokens (flexible) */
export type Token = 
    | { Ethereum: EthereumToken | { ERC20: ERC20Token } }
    | { Bnb: BnbToken | { BEP20: BEP20Token } }
    | { Polkadot: PolkadotToken | { Asset: PolkadotAsset } }
    | { Solana: SolanaToken | { SPL: SPLToken } }
    | { Tron: TronToken | { TRC20: TRC20Token } }
    | { Optimism: OptimismToken | { ERC20: ERC20Token } }
    | { Arbitrum: ArbitrumToken | { ERC20: ERC20Token } }
    | { Polygon: PolygonToken | { ERC20: ERC20Token } }
    | { Base: BaseToken | { ERC20: ERC20Token } }
    | { Bitcoin: BitcoinToken }

/**
 * Token Manager - Utility for creating and managing tokens
 */
export class TokenManager {
  /**
   * Create a native token for a specific chain
   */
  static createNativeToken(chain: ChainSupported): Token {
    switch (chain) {
      case ChainSupported.Ethereum:
        return { Ethereum: EthereumToken.ETH };
      case ChainSupported.Bnb:
        return { Bnb: BnbToken.BNB };
      case ChainSupported.Polkadot:
        return { Polkadot: PolkadotToken.DOT };
      case ChainSupported.Solana:
        return { Solana: SolanaToken.SOL };
      case ChainSupported.Tron:
        return { Tron: TronToken.TRX };
      case ChainSupported.Optimism:
        return { Optimism: OptimismToken.ETH };
      case ChainSupported.Arbitrum:
        return { Arbitrum: ArbitrumToken.ETH };
      case ChainSupported.Polygon:
        return { Polygon: PolygonToken.POL };
      case ChainSupported.Base:
        return { Base: BaseToken.ETH };
      case ChainSupported.Bitcoin:
        return { Bitcoin: BitcoinToken.BTC };
      default:
        throw new Error(`Unsupported chain: ${chain}`);
    }
  }

  /**
   * Create an ERC-20 token for Ethereum-compatible chains
   */
  static createERC20Token(chain: ChainSupported, name: string, address: string): Token {
    const erc20Token: ERC20Token = { name, address };
    switch (chain) {
      case ChainSupported.Ethereum:
        return { Ethereum: { ERC20: erc20Token } };
      case ChainSupported.Optimism:
        return { Optimism: { ERC20: erc20Token } };
      case ChainSupported.Arbitrum:
        return { Arbitrum: { ERC20: erc20Token } };
      case ChainSupported.Polygon:
        return { Polygon: { ERC20: erc20Token } };
      case ChainSupported.Base:
        return { Base: { ERC20: erc20Token } };
      default:
        throw new Error(`ERC-20 tokens not supported on ${chain}`);
    }
  }

  /**
   * Create a BEP-20 token for BNB Smart Chain
   */
  static createBEP20Token(name: string, address: string): Token {
    const bep20Token: BEP20Token = { name, address };
    return { Bnb: { BEP20: bep20Token } };
  }

  /**
   * Create an SPL token for Solana
   */
  static createSPLToken(name: string, address: string): Token {
    const splToken: SPLToken = { name, address };
    return { Solana: { SPL: splToken } };
  }

  /**
   * Create a TRC-20 token for TRON
   */
  static createTRC20Token(name: string, address: string): Token {
    const trc20Token: TRC20Token = { name, address };
    return { Tron: { TRC20: trc20Token } };
  }

  /**
   * Create a Polkadot asset
   */
  static createPolkadotAsset(name: string, id: string): Token {
    const asset: PolkadotAsset = { name, id };
    return { Polkadot: { Asset: asset } };
  }

  /**
   * Get the chain from a token
   */
  static getChainFromToken(token: Token): ChainSupported {
    if ('Ethereum' in token) return ChainSupported.Ethereum;
    if ('Bnb' in token) return ChainSupported.Bnb;
    if ('Polkadot' in token) return ChainSupported.Polkadot;
    if ('Solana' in token) return ChainSupported.Solana;
    if ('Tron' in token) return ChainSupported.Tron;
    if ('Optimism' in token) return ChainSupported.Optimism;
    if ('Arbitrum' in token) return ChainSupported.Arbitrum;
    if ('Polygon' in token) return ChainSupported.Polygon;
    if ('Base' in token) return ChainSupported.Base;
    if ('Bitcoin' in token) return ChainSupported.Bitcoin;
    throw new Error(`Unknown token type: ${JSON.stringify(token)}`);
  }

  /**
   * Get a human-readable string representation of the token
   */
  static getTokenString(token: Token): string {
    const chain = this.getChainFromToken(token);
    
    if ('Ethereum' in token && token.Ethereum === EthereumToken.ETH) {
      return "Ethereum:ETH";
    }
    if ('Bnb' in token && token.Bnb === BnbToken.BNB) {
      return "BNB:BNB";
    }
    if ('Polkadot' in token && token.Polkadot === PolkadotToken.DOT) {
      return "Polkadot:DOT";
    }
    if ('Solana' in token && token.Solana === SolanaToken.SOL) {
      return "Solana:SOL";
    }
    if ('Tron' in token && token.Tron === TronToken.TRX) {
      return "TRON:TRX";
    }
    if ('Optimism' in token && token.Optimism === OptimismToken.ETH) {
      return "Optimism:ETH";
    }
    if ('Arbitrum' in token && token.Arbitrum === ArbitrumToken.ETH) {
      return "Arbitrum:ETH";
    }
    if ('Polygon' in token && token.Polygon === PolygonToken.POL) {
      return "Polygon:POL";
    }
    if ('Base' in token && token.Base === BaseToken.ETH) {
      return "Base:ETH";
    }
    if ('Bitcoin' in token && token.Bitcoin === BitcoinToken.BTC) {
      return "Bitcoin:BTC";
    }

    // Handle ecosystem tokens
    if ('Ethereum' in token && typeof token.Ethereum === 'object' && 'ERC20' in token.Ethereum) {
      return `Ethereum:${token.Ethereum.ERC20.name} (${token.Ethereum.ERC20.address})`;
    }
    if ('Bnb' in token && typeof token.Bnb === 'object' && 'BEP20' in token.Bnb) {
      return `BNB:${token.Bnb.BEP20.name} (${token.Bnb.BEP20.address})`;
    }
    if ('Solana' in token && typeof token.Solana === 'object' && 'SPL' in token.Solana) {
      return `Solana:${token.Solana.SPL.name} (${token.Solana.SPL.address})`;
    }
    if ('Tron' in token && typeof token.Tron === 'object' && 'TRC20' in token.Tron) {
      return `TRON:${token.Tron.TRC20.name} (${token.Tron.TRC20.address})`;
    }
    if ('Polkadot' in token && typeof token.Polkadot === 'object' && 'Asset' in token.Polkadot) {
      return `Polkadot:${token.Polkadot.Asset.name} (${token.Polkadot.Asset.id})`;
    }
    if ('Optimism' in token && typeof token.Optimism === 'object' && 'ERC20' in token.Optimism) {
      return `Optimism:${token.Optimism.ERC20.name} (${token.Optimism.ERC20.address})`;
    }
    if ('Arbitrum' in token && typeof token.Arbitrum === 'object' && 'ERC20' in token.Arbitrum) {
      return `Arbitrum:${token.Arbitrum.ERC20.name} (${token.Arbitrum.ERC20.address})`;
    }
    if ('Polygon' in token && typeof token.Polygon === 'object' && 'ERC20' in token.Polygon) {
      return `Polygon:${token.Polygon.ERC20.name} (${token.Polygon.ERC20.address})`;
    }
    if ('Base' in token && typeof token.Base === 'object' && 'ERC20' in token.Base) {
      return `Base:${token.Base.ERC20.name} (${token.Base.ERC20.address})`;
    }

    return `${chain}:Unknown`;
  }

  /**
   * Parse a token string back to a Token object
   */
  static parseTokenString(tokenString: string): Token {
    const [chain, symbol] = tokenString.split(':');
    
    switch (chain) {
      case "Ethereum":
        if (symbol === "ETH") {
          return { Ethereum: EthereumToken.ETH };
        }
        // For ERC20 tokens, expect format: "USDC (0x123...)" or just "USDC"
        const erc20Match = symbol.match(/^(.+?)\s*\((.+)\)$/);
        if (erc20Match) {
          const [, name, address] = erc20Match;
          return { Ethereum: { ERC20: { name: name.trim(), address: address.trim() } } };
        }
        // Fallback: treat as name only, address will need to be resolved elsewhere
        return { Ethereum: { ERC20: { name: symbol, address: "" } } };
      
      case "BNB":
        if (symbol === "BNB") {
          return { Bnb: BnbToken.BNB };
        }
        // For BEP20 tokens, expect format: "USDC (0x123...)" or just "USDC"
        const bep20Match = symbol.match(/^(.+?)\s*\((.+)\)$/);
        if (bep20Match) {
          const [, name, address] = bep20Match;
          return { Bnb: { BEP20: { name: name.trim(), address: address.trim() } } };
        }
        return { Bnb: { BEP20: { name: symbol, address: "" } } };
      
      case "Polkadot":
        if (symbol === "DOT") {
          return { Polkadot: PolkadotToken.DOT };
        }
        // For Polkadot assets, expect format: "USDC (1984)" or just "USDC"
        const assetMatch = symbol.match(/^(.+?)\s*\((.+)\)$/);
        if (assetMatch) {
          const [, name, id] = assetMatch;
          return { Polkadot: { Asset: { name: name.trim(), id: id.trim() } } };
        }
        return { Polkadot: { Asset: { name: symbol, id: "" } } };
      
      case "Solana":
        if (symbol === "SOL") {
          return { Solana: SolanaToken.SOL };
        }
        // For SPL tokens, expect format: "USDC (0x123...)" or just "USDC"
        const splMatch = symbol.match(/^(.+?)\s*\((.+)\)$/);
        if (splMatch) {
          const [, name, address] = splMatch;
          return { Solana: { SPL: { name: name.trim(), address: address.trim() } } };
        }
        return { Solana: { SPL: { name: symbol, address: "" } } };
      
      case "TRON":
        if (symbol === "TRX") {
          return { Tron: TronToken.TRX };
        }
        // For TRC20 tokens, expect format: "USDC (0x123...)" or just "USDC"
        const trc20Match = symbol.match(/^(.+?)\s*\((.+)\)$/);
        if (trc20Match) {
          const [, name, address] = trc20Match;
          return { Tron: { TRC20: { name: name.trim(), address: address.trim() } } };
        }
        return { Tron: { TRC20: { name: symbol, address: "" } } };
      
      case "Optimism":
        if (symbol === "ETH") {
          return { Optimism: OptimismToken.ETH };
        }
        // For ERC20 tokens, expect format: "USDC (0x123...)" or just "USDC"
        const optimismErc20Match = symbol.match(/^(.+?)\s*\((.+)\)$/);
        if (optimismErc20Match) {
          const [, name, address] = optimismErc20Match;
          return { Optimism: { ERC20: { name: name.trim(), address: address.trim() } } };
        }
        return { Optimism: { ERC20: { name: symbol, address: "" } } };
      
      case "Arbitrum":
        if (symbol === "ETH") {
          return { Arbitrum: ArbitrumToken.ETH };
        }
        // For ERC20 tokens, expect format: "USDC (0x123...)" or just "USDC"
        const arbitrumErc20Match = symbol.match(/^(.+?)\s*\((.+)\)$/);
        if (arbitrumErc20Match) {
          const [, name, address] = arbitrumErc20Match;
          return { Arbitrum: { ERC20: { name: name.trim(), address: address.trim() } } };
        }
        return { Arbitrum: { ERC20: { name: symbol, address: "" } } };
      
      case "Polygon":
        if (symbol === "POL") {
          return { Polygon: PolygonToken.POL };
        }
        // For ERC20 tokens, expect format: "USDC (0x123...)" or just "USDC"
        const polygonErc20Match = symbol.match(/^(.+?)\s*\((.+)\)$/);
        if (polygonErc20Match) {
          const [, name, address] = polygonErc20Match;
          return { Polygon: { ERC20: { name: name.trim(), address: address.trim() } } };
        }
        return { Polygon: { ERC20: { name: symbol, address: "" } } };
      
      case "Base":
        if (symbol === "ETH") {
          return { Base: BaseToken.ETH };
        }
        // For ERC20 tokens, expect format: "USDC (0x123...)" or just "USDC"
        const baseErc20Match = symbol.match(/^(.+?)\s*\((.+)\)$/);
        if (baseErc20Match) {
          const [, name, address] = baseErc20Match;
          return { Base: { ERC20: { name: name.trim(), address: address.trim() } } };
        }
        return { Base: { ERC20: { name: symbol, address: "" } } };
      
      case "Bitcoin":
        if (symbol === "BTC") {
          return { Bitcoin: BitcoinToken.BTC };
        }
        throw new Error("Bitcoin only supports BTC token");
      
      default:
        throw new Error(`Unknown chain: ${chain}`);
    }
  }

  /**
   * Get all available native tokens
   */
  static getAllNativeTokens(): Token[] {
    return [
      this.createNativeToken(ChainSupported.Ethereum),
      this.createNativeToken(ChainSupported.Bnb),
      this.createNativeToken(ChainSupported.Polkadot),
      this.createNativeToken(ChainSupported.Solana),
      this.createNativeToken(ChainSupported.Tron),
      this.createNativeToken(ChainSupported.Optimism),
      this.createNativeToken(ChainSupported.Arbitrum),
      this.createNativeToken(ChainSupported.Polygon),
      this.createNativeToken(ChainSupported.Base),
      this.createNativeToken(ChainSupported.Bitcoin),
    ];
  }

  /**
   * Get all available chains
   */
  static getAllChains(): ChainSupported[] {
    return [
      ChainSupported.Ethereum,
      ChainSupported.Bnb,
      ChainSupported.Polkadot,
      ChainSupported.Solana,
      ChainSupported.Tron,
      ChainSupported.Optimism,
      ChainSupported.Arbitrum,
      ChainSupported.Polygon,
      ChainSupported.Base,
      ChainSupported.Bitcoin,
    ];
  }
}
   
// For status that contains data
interface TxStatusData {
    FailedToSubmitTxn: string;
    TxError: string;
    TxSubmissionPassed: { hash: Uint8Array };
    Reverted: string;
}
   
export type TxStatus = 
    | { type: "Genesis" }
    | { type: "RecvAddrConfirmed" }
    | { type: "RecvAddrConfirmationPassed" }
    | { type: "NetConfirmed" }
    | { type: "SenderConfirmed" }
    | { type: "SenderConfirmationfailed" }
    | { type: "RecvAddrFailed" }
    | { type: "FailedToSubmitTxn", data: TxStatusData["FailedToSubmitTxn"] }
    | { type: "TxError", data: TxStatusData["TxError"] }
    | { type: "TxSubmissionPassed", data: TxStatusData["TxSubmissionPassed"] }
    | { type: "ReceiverNotRegistered" }
    | { type: "Reverted", data: TxStatusData["Reverted"] }

export interface UnsignedEip1559 {
    to: string;
    value: bigint;
    chainId: number;
    nonce: number;
    gas: bigint;
    maxFeePerGas: bigint;
    maxPriorityFeePerGas: bigint;
    data?: string;
    accessList?: any[];
    type: 'eip1559';
}

// BNB legacy unsigned transaction (no EIP-1559)
export interface UnsignedBnbLegacy {
    to: string;
    value: bigint;
    chainId: number;
    nonce: number;
    gas: bigint;
    gasPrice: bigint;
    data?: string;
    type: 'legacy';
}


export type ChainTransactionType = 
    | {
        ethereum: {
            ethUnsignedTxFields: UnsignedEip1559;
            callPayload: [Uint8Array, Uint8Array];
        };
    }
    | {
        solana: {
            callPayload: Uint8Array;
            latestBlockHeight: number;
        };
    }
    | {
        bnb: {
            callPayload: [Uint8Array, Uint8Array];
            bnbLegacyTxFields: UnsignedBnbLegacy;
        };
    };

/** Transaction data structure state machine, passed in rpc and p2p swarm */
export interface TxStateMachine {
    senderAddress: string;
    senderPublicKey: string | null;
    receiverPublicKey: string | null;
    receiverAddress: string;
    /** Hashed sender and receiver address to bind the addresses while sending */
    multiId: number[]; // [u8; 32] in Rust -> number[] in TS
    /** Signature of the receiver id */
    recvSignature?: Uint8Array;
    /** Token type */
    token: Token;
    /** State Machine status */
    status: TxStatus;
    /** Code word */
    codeWord: string;
    /** Amount to be sent */
    amount: bigint; // u128 in Rust -> bigint in TS
    /** Fees amount */
    feesAmount: number; // u8 in Rust -> number in TS
    /** Signed call payload (signed hash of the transaction) */
    signedCallPayload?: Uint8Array;
    /** Call payload (hash of transaction and raw transaction bytes) */
    callPayload?: ChainTransactionType | null;
    /** Inbound Request id for p2p */
    inboundReqId?: number; // Option<u64> in Rust -> number | undefined in TS
    /** Outbound Request id for p2p */
    outboundReqId?: number; // Option<u64> in Rust -> number | undefined in TS
    /** Stores the current nonce of the transaction per vane not the nonce for the blockchain network */
    txNonce: number; // u32 in Rust -> number in TS
    /** Monotonic version for conflict/race resolution across async boundaries */
    txVersion: number; // u32 in Rust -> number in TS
    /** Sender address network */
    senderAddressNetwork: ChainSupported;
    /** Receiver address network */
    receiverAddressNetwork: ChainSupported;
}

export class TxStateMachineManager {
    private tx: TxStateMachine;
   
    constructor(tx: TxStateMachine) {
      this.tx = tx;
    }
   
    setReceiverSignature(signature: Uint8Array): void {
      this.tx.recvSignature = signature;
    }
   
    setCallPayload(payload: ChainTransactionType | null): void {
      this.tx.callPayload = payload;
    }
   
    setSignedCallPayload(payload: Uint8Array): void {
      this.tx.signedCallPayload = payload;
    }
    setRevertedReason(reason: string): void {
      this.tx.status = {type: "Reverted", data: reason};
    }
   
    updateStatus(status: TxStatus): void {
      this.tx.status = status;
    }
   
    setRequestIds(inbound?: number, outbound?: number): void {
      if (inbound) this.tx.inboundReqId = inbound;
      if (outbound) this.tx.outboundReqId = outbound;
    }
   
    // Utility methods
    isSignedByReceiver(): boolean {
      return !!this.tx.recvSignature;
    }
   
    hasCallPayload(): boolean {
      return !!this.tx.callPayload;
    }
   
    // Getters
    getTx(): TxStateMachine {
      return {...this.tx};
    }
   
    // Create new instance
    static create(
      senderAddress: string,
      receiverAddress: string,
      senderNetwork: ChainSupported,
      receiverNetwork: ChainSupported,
      token: Token,
      amount: bigint,
      codeWord: string,
      senderPublicKey: string | null,
      receiverPublicKey: string | null,
      feesAmount: number = 0
    ): TxStateMachineManager {
      return new TxStateMachineManager({
        senderAddress,
        senderPublicKey,
        receiverPublicKey,
        receiverAddress,
        multiId: [], // Generate hash of sender+receiver
        token,
        status: {type: "Genesis"},
        amount,
        feesAmount,
        txNonce: 0,
        txVersion: 0,
        codeWord,
        senderAddressNetwork: senderNetwork,
        receiverAddressNetwork: receiverNetwork
      });
    }
}

export interface AccountProfile {
    accounts: {address: string, network: string}[];
    peer_id: string;
    multi_addr: string;
    rpc: string;
}

// ===================== Database Storage Export Types ===================== //

/** User account structure containing multi-address and associated accounts */
export interface UserAccount {
    /** User's libp2p multi-address */
    multi_addr: string;
    /** Array of account addresses paired with their respective chains */
    accounts: [string, ChainSupported][];
}

/** Transaction data structure as stored in the database */
export interface DbTxStateMachine {
    /** Transaction hash based on the chain's hashing algorithm */
    tx_hash: number[]; // Vec<u8> in Rust -> number[] in TS
    /** Amount sent in the transaction */
    amount: bigint; // u128 in Rust -> bigint in TS
    /** Sender address */
    sender: string;
    /** Receiver address */
    receiver: string;
    /** Sender address network */
    sender_network: ChainSupported;
    /** Receiver address network */
    receiver_network: ChainSupported;
    /** Whether the transaction was successful */
    success: boolean;
}

/** Information about a saved peer */
export interface SavedPeerInfo {
    /** The peer's multi-address (e.g., "/ip4/127.0.0.1/tcp/8080/p2p/12D3KooWPeer1") */
    peer_id: string;
    /** All account IDs associated with this peer */
    account_ids: string[];
}

/** 
 * Complete database storage export structure
 * Contains all data from the database using getter methods
 */
export interface StorageExport {
    /** User account information (multi-address and associated chain accounts) */
    user_account?: UserAccount;
    
    /** Current nonce value for transaction ordering */
    nonce: number; // u32 in Rust -> number in TS
    
    /** All successful transactions */
    success_transactions: DbTxStateMachine[];
    
    /** All failed transactions */
    failed_transactions: DbTxStateMachine[];
    
    /** Total value of all successful transactions (in wei/smallest unit) */
    total_value_success: number; // u64 in Rust -> number in TS
    
    /** Total value of all failed transactions (in wei/smallest unit) */
    total_value_failed: number; // u64 in Rust -> number in TS
    
    /** 
     * Multiple saved peers, each with their own account IDs
     * Example with 2 separate peers, each having 2 addresses:
     * [
     *   { 
     *     peer_id: "/ip4/127.0.0.1/tcp/8080/p2p/12D3KooWPeer1", 
     *     account_ids: ["0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266", "0x70997970C51812dc3A010C7d01b50e0d17dc79C8"] 
     *   },
     *   { 
     *     peer_id: "/ip4/192.168.1.100/tcp/8080/p2p/12D3KooWPeer2", 
     *     account_ids: ["0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC", "0x90F79bf6EB2c4f870365E785982E1f101E45bF15"] 
     *   }
     * ]
     */
    all_saved_peers: SavedPeerInfo[];
}

/** Node connection status information */
export interface NodeConnectionStatus {
    relay_connected: boolean;
    peer_id: string;
    relay_address: string;
    connection_uptime_seconds?: number;
    last_connection_change?: number; // Unix timestamp
}

/** User metrics structure */
export interface UserMetrics {
    user_account: UserAccount;
    total_success_txns: DbTxStateMachine[];
    total_failed_txns: DbTxStateMachine[];
    saved_target_peers: [string[], string]; // (Vec<String>, String) in Rust -> [string[], string] in TS
}

/**
 * Helper class for working with StorageExport data
 */
export class StorageExportManager {
    private storage: StorageExport;

    constructor(storage: StorageExport) {
        this.storage = storage;
    }

    /**
     * Get the total number of transactions (successful + failed)
     */
    getTotalTransactionCount(): number {
        return this.storage.success_transactions.length + this.storage.failed_transactions.length;
    }

    /**
     * Get the success rate as a percentage
     */
    getSuccessRate(): number {
        const total = this.getTotalTransactionCount();
        if (total === 0) return 0;
        return (this.storage.success_transactions.length / total) * 100;
    }

    /**
     * Get all unique networks from transactions (both sender and receiver networks)
     */
    getNetworksUsed(): ChainSupported[] {
        const networks = new Set<ChainSupported>();
        [...this.storage.success_transactions, ...this.storage.failed_transactions]
            .forEach(tx => {
                networks.add(tx.sender_network);
                networks.add(tx.receiver_network);
            });
        return Array.from(networks);
    }

    /**
     * Get transactions by network (matches either sender or receiver network)
     */
    getTransactionsByNetwork(network: ChainSupported): {
        successful: DbTxStateMachine[];
        failed: DbTxStateMachine[];
    } {
        return {
            successful: this.storage.success_transactions.filter(tx => 
                tx.sender_network === network || tx.receiver_network === network),
            failed: this.storage.failed_transactions.filter(tx => 
                tx.sender_network === network || tx.receiver_network === network),
        };
    }

    /**
     * Get transactions by sender address
     */
    getTransactionsBySender(senderAddress: string): {
        successful: DbTxStateMachine[];
        failed: DbTxStateMachine[];
    } {
        return {
            successful: this.storage.success_transactions.filter(tx => tx.sender === senderAddress),
            failed: this.storage.failed_transactions.filter(tx => tx.sender === senderAddress),
        };
    }

    /**
     * Get transactions by receiver address
     */
    getTransactionsByReceiver(receiverAddress: string): {
        successful: DbTxStateMachine[];
        failed: DbTxStateMachine[];
    } {
        return {
            successful: this.storage.success_transactions.filter(tx => tx.receiver === receiverAddress),
            failed: this.storage.failed_transactions.filter(tx => tx.receiver === receiverAddress),
        };
    }

    /**
     * Get all account IDs from saved peers
     */
    getAllAccountIds(): string[] {
        return this.storage.all_saved_peers.flatMap(peer => peer.account_ids);
    }

    /**
     * Find peer by account ID
     */
    findPeerByAccountId(accountId: string): SavedPeerInfo | undefined {
        return this.storage.all_saved_peers.find(peer => 
            peer.account_ids.includes(accountId)
        );
    }

    /**
     * Get storage export data
     */
    getStorage(): StorageExport {
        return { ...this.storage };
    }
    
    /**
     * Get largest transaction failed amount
     */
    getLargestFailedTransactionAmount(): number {
        return this.storage.failed_transactions.reduce((max, tx) => {
            return Math.max(max, Number(tx.amount));
        }, 0);
    }

    /**
     * Create a human-readable summary
     */
    getSummary(): {
        totalTransactions: number;
        successfulTransactions: number;
        failedTransactions: number;
        largestFailedTransactionAmount: number;
        successRate: string;
        totalValueSuccess: number;
        totalValueFailed: number;
        networksUsed: ChainSupported[];
        peersCount: number;
        accountsCount: number;
        currentNonce: number;
    } {
        return {
            totalTransactions: this.getTotalTransactionCount(),
            successfulTransactions: this.storage.success_transactions.length,
            largestFailedTransactionAmount: this.getLargestFailedTransactionAmount(),
            failedTransactions: this.storage.failed_transactions.length,
            successRate: `${this.getSuccessRate().toFixed(2)}%`,
            totalValueSuccess: this.storage.total_value_success,
            totalValueFailed: this.storage.total_value_failed,
            networksUsed: this.getNetworksUsed(),
            peersCount: this.storage.all_saved_peers.length,
            accountsCount: this.getAllAccountIds().length,
            currentNonce: this.storage.nonce,
        };
    }
}