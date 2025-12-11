/* tslint:disable */
/* eslint-disable */
export function start_vane_web3(relay_node_multi_addr: string, account: string, network: string, self_node: boolean, live: boolean, storage: any): Promise<PublicInterfaceWorkerJs>;
export class PublicInterfaceWorkerJs {
  private constructor();
  free(): void;
  initiateTransaction(sender: string, receiver: string, amount: bigint, token: any, code_word: string, sender_network: any, receiver_network: any): Promise<any>;
  senderConfirm(tx: any): Promise<void>;
  watchTxUpdates(callback: Function): Promise<void>;
  unsubscribeWatchTxUpdates(): void;
  watchP2pNotifications(callback: Function): Promise<void>;
  unsubscribeWatchP2pNotifications(): void;
  fetchPendingTxUpdates(): Promise<any>;
  addAccount(account_id: string, network: any): Promise<void>;
  receiverConfirm(tx: any): Promise<void>;
  revertTransaction(tx: any, reason?: string | null): Promise<void>;
  exportStorage(): Promise<any>;
  deleteTxInCache(tx: any): void;
  clearRevertedFromCache(): void;
  clearFinalizedFromCache(): void;
}
export class RequestArguments {
  private constructor();
  free(): void;
  readonly method: string;
  readonly params: Array<any>;
}

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
  readonly memory: WebAssembly.Memory;
  readonly start_vane_web3: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: any) => any;
  readonly __wbg_publicinterfaceworkerjs_free: (a: number, b: number) => void;
  readonly publicinterfaceworkerjs_initiateTransaction: (a: number, b: number, c: number, d: number, e: number, f: bigint, g: bigint, h: any, i: number, j: number, k: any, l: any) => any;
  readonly publicinterfaceworkerjs_senderConfirm: (a: number, b: any) => any;
  readonly publicinterfaceworkerjs_watchTxUpdates: (a: number, b: any) => any;
  readonly publicinterfaceworkerjs_unsubscribeWatchTxUpdates: (a: number) => [number, number];
  readonly publicinterfaceworkerjs_watchP2pNotifications: (a: number, b: any) => any;
  readonly publicinterfaceworkerjs_unsubscribeWatchP2pNotifications: (a: number) => [number, number];
  readonly publicinterfaceworkerjs_fetchPendingTxUpdates: (a: number) => any;
  readonly publicinterfaceworkerjs_addAccount: (a: number, b: number, c: number, d: any) => any;
  readonly publicinterfaceworkerjs_receiverConfirm: (a: number, b: any) => any;
  readonly publicinterfaceworkerjs_revertTransaction: (a: number, b: any, c: number, d: number) => any;
  readonly publicinterfaceworkerjs_exportStorage: (a: number) => any;
  readonly publicinterfaceworkerjs_deleteTxInCache: (a: number, b: any) => [number, number];
  readonly publicinterfaceworkerjs_clearRevertedFromCache: (a: number) => [number, number];
  readonly publicinterfaceworkerjs_clearFinalizedFromCache: (a: number) => [number, number];
  readonly __wbg_requestarguments_free: (a: number, b: number) => void;
  readonly requestarguments_method: (a: number) => [number, number];
  readonly requestarguments_params: (a: number) => any;
  readonly __wbindgen_malloc: (a: number, b: number) => number;
  readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
  readonly __wbindgen_exn_store: (a: number) => void;
  readonly __externref_table_alloc: () => number;
  readonly __wbindgen_export_4: WebAssembly.Table;
  readonly __wbindgen_free: (a: number, b: number, c: number) => void;
  readonly __wbindgen_export_6: WebAssembly.Table;
  readonly __externref_table_dealloc: (a: number) => void;
  readonly closure568_externref_shim: (a: number, b: number, c: any) => void;
  readonly _dyn_core__ops__function__FnMut_____Output___R_as_wasm_bindgen__closure__WasmClosure___describe__invoke__h1a79a3c1169870fb: (a: number, b: number) => void;
  readonly _dyn_core__ops__function__FnMut_____Output___R_as_wasm_bindgen__closure__WasmClosure___describe__invoke__hff5c85c2ca770dbc: (a: number, b: number) => void;
  readonly closure770_externref_shim: (a: number, b: number, c: any) => void;
  readonly _dyn_core__ops__function__FnMut_____Output___R_as_wasm_bindgen__closure__WasmClosure___describe__invoke__h7876d40aeac1bca2: (a: number, b: number) => void;
  readonly closure851_externref_shim: (a: number, b: number, c: any, d: any) => void;
  readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;
/**
* Instantiates the given `module`, which can either be bytes or
* a precompiled `WebAssembly.Module`.
*
* @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
*
* @returns {InitOutput}
*/
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
* If `module_or_path` is {RequestInfo} or {URL}, makes a request and
* for everything else, calls `WebAssembly.instantiate` directly.
*
* @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
*
* @returns {Promise<InitOutput>}
*/
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
