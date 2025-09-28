import type { TxStateMachine, Token, ChainSupported } from "./primitives";

// WASM bindings (ESM)
import initWasm, { start_vane_web3, PublicInterfaceWorkerJs } from "./pkg/vane_wasm_node.js";

// Logging host helpers exposed to WASM
import { hostLogging, LogLevel } from "./pkg/host_functions/logging";

type InitOptions = {
  relayMultiAddr: string;
  account: string;
  network: string;
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

export async function initializeNode(options: InitOptions): Promise<PublicInterfaceWorkerJs> {
  const { relayMultiAddr, account, network, live = false, logLevel } = options;

  await ensureWasmInitialized();

  if (typeof logLevel !== "undefined") {
    hostLogging.setLogLevel(Number(logLevel));
  }

  nodeWorker = await start_vane_web3(relayMultiAddr, account, network, live);
  return nodeWorker;
}

export function setLogLevel(level: LogLevel | number): void {
  hostLogging.setLogLevel(Number(level));
}

type LogCallback = Parameters<typeof hostLogging.setLogCallback>[0];
export function onLog(callback: LogCallback): void {
  hostLogging.setLogCallback(callback);
}

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

export async function initiateTransaction(
  sender: string,
  receiver: string,
  amount: number | bigint,
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

export async function revertTransaction(tx: TxStateMachine, reason?: string | null): Promise<void> {
  await requireWorker().revertTransaction(tx, reason ?? null);
}

export async function watchTxUpdates(callback: (tx: TxStateMachine) => void): Promise<void> {
  await requireWorker().watchTxUpdates(callback);
}

export function unsubscribeWatchTxUpdates(): void {
  requireWorker().unsubscribeWatchTxUpdates();
}

export async function fetchPendingTxUpdates(): Promise<TxStateMachine[]> {
  const res = await requireWorker().fetchPendingTxUpdates();
  return res as TxStateMachine[];
}

export async function exportStorage(): Promise<unknown> {
  return await requireWorker().exportStorage();
}

export async function getMetrics(): Promise<unknown> {
  return await requireWorker().getMetrics();
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
  unsubscribeWatchTxUpdates,
  fetchPendingTxUpdates,
  exportStorage,
  getMetrics,
  clearRevertedFromCache,
  clearFinalizedFromCache,
  getWorker,
};

export default VaneWeb3;