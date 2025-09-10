import { describe, test, expect, beforeAll, afterAll, it } from 'vitest'
import { hostFunctions } from '../../node/wasm/host_functions/main.js'
import init, * as wasmModule from '../../node/wasm/pkg/vane_wasm_node.js';
import { logWasmExports, waitForWasmInitialization, setupWasmLogging, loadRelayNodeInfo, RelayNodeInfo, startWasmNode, WasmNodeInstance, getWallets } from './utils/wasm_utils.js';
import { TestClient } from 'viem'


describe('WASM NODE & RELAY NODE INTERACTIONS', () => {
  let relayInfo: RelayNodeInfo | null = null
  let walletClient: TestClient | null = null;
  let wasm_client_address: string | undefined = undefined;
  let receiver_client_address = "0x63FaC9201494f0bd17B9892B9fae4d52fe3BD377"
  let wasmNodeInstance: WasmNodeInstance | null = null;
 
 
  
  beforeAll(async () => {

    try {
      relayInfo = await loadRelayNodeInfo();
    } catch (error) {
      console.error('❌ Failed to load relay node info:', error);
      console.error('💡 Make sure to start the relay node first: bun run start-relay');
      throw error;
    }

    walletClient = getWallets()[0];
    wasm_client_address = walletClient!.account!.address;
    console.log("1 wasm client address", wasm_client_address);

    try {
      await init();
      setupWasmLogging();
      logWasmExports();
      await waitForWasmInitialization();

    } catch (error) {
      console.error('❌ Failed to initialize WASM module:', error);
    }

    wasmNodeInstance = startWasmNode(relayInfo.multiAddr, wasm_client_address!, "Ethereum", false);
    await wasmNodeInstance.promise;
    await new Promise(resolve => setTimeout(resolve, 20000));
    console.log('✅ WASM node started successfully');

  })

  test('should successfully send a transaction', async () => {
    expect(wasmNodeInstance).toBeDefined();
    await new Promise(resolve => setTimeout(resolve, 10000));
    await wasmNodeInstance?.promise.then(vaneWasm => {
      return vaneWasm?.initiateTransaction(wasm_client_address!, receiver_client_address, BigInt(1), "Eth", "Ethereum", "Maji");
    });

    // Keep the node connected for other nodes to interact with it
    console.log('⏳ Keeping WASM node connected for other nodes to interact...');
    await new Promise(resolve => setTimeout(resolve, 60000));
  })


  afterAll(async () => {
    console.log('🧹 Cleaning up WASM Node 1...');
  })
})