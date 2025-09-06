import { describe, test, expect, beforeAll, afterAll, it } from 'vitest'
import { hostFunctions } from '../../node/wasm/host_functions/main.js'
import init, * as wasmModule from '../../node/wasm/pkg/wasm_node.js';
import { logWasmExports, waitForWasmInitialization, setupWasmLogging, loadRelayNodeInfo, RelayNodeInfo, startWasmNode, WasmNodeInstance, getWallets } from './utils/wasm_utils.js';
import { TestClient } from 'viem'


describe('WASM NODE & RELAY NODE INTERACTIONS', () => {
  let relayInfo: RelayNodeInfo | null = null;
  let walletClient: TestClient | null = null;
  let wasm_client_address: string | undefined = undefined;
  let receiver_client_address = "0x63FaC9201494f0bd17B9892B9fae4d52fe3BD377"
  let wasmNodeInstance: WasmNodeInstance | null = null;
 
 
  
  beforeAll(async () => {
    try {
      relayInfo = await loadRelayNodeInfo();
      console.log('✅ Relay node info loaded');
    } catch (error) {
      console.error('❌ Failed to load relay node info:', error);
      console.error('💡 Make sure to start the relay node first: bun run start-relay');
      throw error;
    }

    walletClient = getWallets()[0];
    wasm_client_address = walletClient!.account!.address;
    console.log("1 wasm client address", wasm_client_address);

    // Initialize WASM module
    //const wasmUrl = '../../node/wasm/pkg/wasm_node_bg.wasm';
    await init();
    console.log('✅ WASM module initialized');
  })

  test('should initialize WASM node successfully', async () => {
    expect(wasm_client_address).toBeDefined();
    expect(relayInfo).toBeDefined();
    console.log('✅ Basic initialization test passed');
  })

  afterAll(async () => {
    console.log('🧹 Cleaning up WASM Node 1...');
  })
})