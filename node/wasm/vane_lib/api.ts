import type {
  TxStateMachine,
  Token,
  ChainSupported,
  NodeConnectionStatus,
  StorageExport,
  UserMetrics,
  AccountProfile,
  P2pEventResult,
} from "./primitives";

// WASM bindings (ESM)
import initWasm, { start_vane_web3, PublicInterfaceWorkerJs } from "./pkg/vane_wasm_node.js";

// Logging host helpers exposed to WASM
import { hostLogging, LogLevel } from "./pkg/host_functions/logging";

type InitOptions = {
  relayMultiAddr: string;
  account: string;
  network: string;
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
  const { relayMultiAddr, account, network, live = false, logLevel, storage } = options;

  await ensureWasmInitialized();

  if (typeof logLevel !== "undefined") {
    hostLogging.setLogLevel(Number(logLevel));
  }

  nodeWorker = await start_vane_web3(relayMultiAddr, account, network, live, storage);
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

// Type for p2p notification callback
export type P2pNotificationCallback = (event: P2pEventResult) => void;

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

export async function addAccount(accountId: string, network: string): Promise<void> {
  return await requireWorker().addAccount(accountId, network);
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

export async function senderConfirm(tx: TxStateMachine): Promise<void> {
  await requireWorker().senderConfirm(tx);
}

export async function receiverConfirm(tx: TxStateMachine): Promise<void> {
  await requireWorker().receiverConfirm(tx);
}

export async function revertTransaction(tx: TxStateMachine, reason?: RevertReason): Promise<void> {
  await requireWorker().revertTransaction(tx, reason ?? null);
}

export async function watchTxUpdates(callback: TxUpdateCallback): Promise<void> {
  await requireWorker().watchTxUpdates(callback);
}

export async function watchP2pNotifications(callback: P2pNotificationCallback): Promise<void> {
  await requireWorker().watchP2pNotifications(callback);
}

export function unsubscribeWatchTxUpdates(): void {
  requireWorker().unsubscribeWatchTxUpdates();
}

export function unsubscribeWatchP2pNotifications(): void {
  requireWorker().unsubscribeWatchP2pNotifications();
}

export async function fetchPendingTxUpdates(): Promise<TxStateMachine[]> {
  const res = await requireWorker().fetchPendingTxUpdates();
  return res as TxStateMachine[];
}

export async function exportStorage(): Promise<StorageExport> {
  const res = await requireWorker().exportStorage();
  return res as StorageExport;
}

/**
 * Get the current node connection status
 * @returns Promise that resolves to the node connection status
 */
export async function getNodeConnection(): Promise<NodeConnectionStatus> {
  const res = await requireWorker().getNodeConnectionStatus();
  return res as NodeConnectionStatus;
}

export function clearRevertedFromCache(): void {
  requireWorker().clearRevertedFromCache();
}

export function clearFinalizedFromCache(): void {
  requireWorker().clearFinalizedFromCache();
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
  addAccount,
  initiateTransaction,
  senderConfirm,
  receiverConfirm,
  revertTransaction,
  watchTxUpdates,
  watchP2pNotifications,
  unsubscribeWatchTxUpdates,
  unsubscribeWatchP2pNotifications,
  fetchPendingTxUpdates,
  exportStorage,
  getNodeConnection,
  clearRevertedFromCache,
  clearFinalizedFromCache,
  getWorker,
};

export default VaneWeb3;