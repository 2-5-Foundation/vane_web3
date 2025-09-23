export enum ChainSupported {
    Ethereum = "Ethereum",
    Polkadot = "Polkadot",
    Bnb = "Bnb",
    Solana = "Solana"
}

/** Supported tokens */
export enum Token {
    Dot = "Dot",
    Bnb = "Bnb", 
    Sol = "Sol",
    Eth = "Eth",
    UsdtSol = "UsdtSol",
    UsdcSol = "UsdcSol",
    UsdtEth = "UsdtEth",
    UsdcEth = "UsdcEth",
    UsdtDot = "UsdtDot"
}
   
// For status that contains data
interface TxStatusData {
    FailedToSubmitTxn: string;
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

/** Transaction data structure state machine, passed in rpc and p2p swarm */
export interface TxStateMachine {
    senderAddress: string;
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
    /** Signed call payload (signed hash of the transaction) */
    signedCallPayload?: Uint8Array;
    /** Call payload (hash of transaction and raw transaction bytes) */
    callPayload?: [Uint8Array, Uint8Array] | null; // ([u8; 32], Vec<u8>) in Rust
    /** Inbound Request id for p2p */
    inboundReqId?: number; // Option<u64> in Rust -> number | undefined in TS
    /** Outbound Request id for p2p */
    outboundReqId?: number; // Option<u64> in Rust -> number | undefined in TS
    /** Stores the current nonce of the transaction per vane not the nonce for the blockchain network */
    txNonce: number; // u32 in Rust -> number in TS
    /** Monotonic version for conflict/race resolution across async boundaries */
    txVersion: number; // u32 in Rust -> number in TS
    /** Unsigned transaction fields for EIP-1559 transactions */
    ethUnsignedTxFields?: UnsignedEip1559 | null;
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
   
    setCallPayload(payload: [Uint8Array, Uint8Array] | null): void {
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
      codeWord: string
    ): TxStateMachineManager {
      return new TxStateMachineManager({
        senderAddress,
        receiverAddress,
        multiId: [], // Generate hash of sender+receiver
        token,
        status: {type: "Genesis"},
        amount,
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
     * Create a human-readable summary
     */
    getSummary(): {
        totalTransactions: number;
        successfulTransactions: number;
        failedTransactions: number;
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