import type {
  TxStateMachine,
  Token,
  ChainSupported,
  StorageExport,
  UserMetrics,
  AccountProfile,
  BackendEvent,
} from "./primitives";

// WASM bindings (ESM)
import initWasm, { start_vane_web3, PublicInterfaceWorkerJs } from "./pkg/vane_wasm_node.js";

// Logging host helpers exposed to WASM
import { hostLogging, LogLevel } from "./pkg/host_functions/logging";

type InitOptions = {
  sig: Uint8Array;
  relayMultiAddr: string;
  account: string;
  network: string;
  self_node: boolean;
  storage?: StorageExport;
  live?: boolean;
  logLevel?: LogLevel | number;
};

let wasmInitialized: boolean = false;
let nodeWorker: PublicInterfaceWorkerJs | null = null;

async function ensureWasmInitialized(): Promise<void> {
  if (!wasmInitialized) {
    await initWasm();
    wasmInitialized = true;
  }
}

/**
 * Initialize the Vane Web3 node with the specified options
 * @param options - Configuration options for the node
 * @returns Promise that resolves to the initialized worker interface
 */
export async function initializeNode(options: InitOptions): Promise<PublicInterfaceWorkerJs> {
  const { sig, relayMultiAddr, account, network, live = false, self_node, logLevel, storage } = options;

  await ensureWasmInitialized();

  if (typeof logLevel !== "undefined") {
    hostLogging.setLogLevel(Number(logLevel));
  }

  nodeWorker = await start_vane_web3(sig, relayMultiAddr, account, network, self_node, live, storage);
  return nodeWorker;
}

export function setLogLevel(level: LogLevel | number): void {
  hostLogging.setLogLevel(Number(level));
}

type LogCallback = Parameters<typeof hostLogging.setLogCallback>[0];
export function onLog(callback: LogCallback): void {
  hostLogging.setLogCallback(callback);
}

// Type for transaction update callback
export type TxUpdateCallback = (tx: TxStateMachine) => void;

// Type for backend event notification callback
export type BackendEventCallback = (event: BackendEvent) => void;

// Type for revert transaction reason
export type RevertReason = string | null | undefined;

// Type for amount parameter (supports both number and bigint)
export type Amount = number | bigint;

export { LogLevel };

function requireWorker(): PublicInterfaceWorkerJs {
  if (!nodeWorker) throw new Error("Vane WASM node is not initialized. Call initializeNode() first.");
  return nodeWorker;
}


export function isInitialized(): boolean {
  return nodeWorker !== null;
}

/**
 * Initiate a new transaction between sender and receiver
 * @param sender - Sender's address
 * @param receiver - Receiver's address  
 * @param amount - Amount to send (number or bigint)
 * @param token - Token type to send
 * @param codeWord - Unique code word for the transaction
 * @param sender_network - Network of the sender
 * @param receiver_network - Network of the receiver
 * @returns Promise that resolves to the transaction state machine
 */
export async function initiateTransaction(
  sig: Uint8Array,
  sender: string,
  receiver: string,
  amount: Amount,
  token: Token,
  codeWord: string,
  sender_network: ChainSupported,
  receiver_network: ChainSupported,
): Promise<TxStateMachine> {
  const amt = typeof amount === "bigint" ? amount : BigInt(amount);
  const res = await requireWorker().initiateTransaction(
    sig,
    sender,
    receiver,
    amt,
    token,
    codeWord,
    sender_network,
    receiver_network,
  );
  return res as TxStateMachine;
}

export async function senderConfirm(sig: Uint8Array, tx: TxStateMachine): Promise<void> {
  await requireWorker().senderConfirm(sig,tx);
}

export async function receiverConfirm(sig: Uint8Array, tx: TxStateMachine): Promise<void> {
  await requireWorker().receiverConfirm(sig,tx);
}

/**
 * Verify transaction call payload integrity
 * @param tx - Transaction state machine to verify
 * @returns Promise that resolves when verification succeeds, rejects on failure
 */
export async function verifyTxCallPayload(tx: TxStateMachine): Promise<void> {
  await requireWorker().verifyTxCallPayload(tx);
}

export async function revertTransaction(sig: Uint8Array, tx: TxStateMachine, reason?: RevertReason): Promise<void> {
  await requireWorker().revertTransaction(sig, tx, reason ?? null);
}

export async function watchTxUpdates(callback: TxUpdateCallback): Promise<void> {
  await requireWorker().watchTxUpdates(callback);
}

export async function watchP2pNotifications(callback: BackendEventCallback): Promise<void> {
  await requireWorker().watchP2pNotifications(callback);
}

export async function unsubscribeWatchTxUpdates(): Promise<void> {
  await requireWorker().unsubscribeWatchTxUpdates();
}

export async function unsubscribeWatchP2pNotifications(): Promise<void> {
  await requireWorker().unsubscribeWatchP2pNotifications();
}

export async function fetchPendingTxUpdates(sig: Uint8Array): Promise<TxStateMachine[]> {
  const res = await requireWorker().fetchPendingTxUpdates(sig);
  return res as TxStateMachine[];
}

export async function exportStorage(): Promise<StorageExport> {
  const res = await requireWorker().exportStorage();
  return res as StorageExport;
}

/**
 * Add an account to the node
 * @param accountId - Account address/identifier
 * @param network - Network/chain the account belongs to
 * @returns Promise that resolves when the account is added
 */
export async function addAccount(accountId: string, network: ChainSupported): Promise<void> {
  await requireWorker().addAccount(accountId, network);
}

export async function clearRevertedFromCache(): Promise<void> {
  await requireWorker().clearRevertedFromCache();
}

export async function clearFinalizedFromCache(): Promise<void> {
  await requireWorker().clearFinalizedFromCache();
}

export async function clearCache(): Promise<void> {
  await requireWorker().clearCache();
}

export async function deleteTxInCache(tx: TxStateMachine): Promise<void> {
  await requireWorker().deleteTxInCache(tx);
}

export async function resetNode(): Promise<void> {
  const w = nodeWorker;
  nodeWorker = null; // 🔑 detach immediately, no matter what

  if (!w) return;

  // Best-effort cleanup — MUST NOT throw
  try { await w.unsubscribeWatchTxUpdates(); } catch {}
  try { await w.unsubscribeWatchP2pNotifications(); } catch {}
  try { await w.clearCache(); } catch {}
}
export function getWorker(): PublicInterfaceWorkerJs | null {
  return nodeWorker;
}

// Default export singleton for ergonomic frontend usage
const VaneWeb3 = {
  initializeNode,
  isInitialized,
  setLogLevel,
  onLog,
  LogLevel,
  initiateTransaction,
  senderConfirm,
  receiverConfirm,
  verifyTxCallPayload,
  revertTransaction,
  watchTxUpdates,
  watchP2pNotifications,
  unsubscribeWatchTxUpdates,
  unsubscribeWatchP2pNotifications,
  fetchPendingTxUpdates,
  exportStorage,
  addAccount,
  clearRevertedFromCache,
  clearFinalizedFromCache,
  deleteTxInCache,
  getWorker,
};

export default VaneWeb3;