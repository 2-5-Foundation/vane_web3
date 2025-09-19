export enum ChainSupported {
    Ethereum = "Ethereum",
    Bnb = "Bnb",
    Polygon = "Polygon",
    Arbitrum = "Arbitrum",
    Polkadot = "Polkadot",
    Solana = "Solana",
    Tron = "Tron"
   }
   
// For status that contains data
interface TxStatusData {
    FailedToSubmitTxn: string;
    TxSubmissionPassed: { hash: Uint8Array };
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
    | { type: "Reverted" }

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

export interface TxStateMachine {

    senderAddress: string;
    receiverAddress: string;
    multiId: number[];
    recvSignature?: Uint8Array;
    network: ChainSupported;
    token: string;
    status: TxStatus;
    amount: bigint; // For u128
    signedCallPayload?: Uint8Array; // signed tx
    callPayload?: [Uint8Array, Uint8Array] | null; // [unsigned tx hash (32 bytes), raw unsigned transaction bytes]
    inboundReqId?: number; // u64
    outboundReqId?: number; // u64
    txNonce: number;
    codeword: string;
    ethUnsignedTxFields?: UnsignedEip1559 | null;
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
      network: ChainSupported,
      token: string,
      amount: bigint,
      codeword: string
    ): TxStateMachineManager {
      return new TxStateMachineManager({
        senderAddress,
        receiverAddress,
        multiId: [], // Generate hash of sender+receiver
        network,
        token,
        status: {type: "Genesis"},
        amount,
        txNonce: 0,
        codeword
      });
    }
}

export interface AccountProfile {
    accounts: {address: string, network: string}[];
    peer_id: string;
    multi_addr: string;
    rpc: string;
}


