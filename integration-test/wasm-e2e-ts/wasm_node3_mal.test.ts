import { describe, test, expect, beforeAll, afterAll, it } from 'vitest'
import { hostFunctions } from '../../node/wasm/host_functions/main.js'
import init, * as wasmModule from '../../node/wasm/pkg/vane_wasm_node.js';
import { logWasmExports, waitForWasmInitialization, setupWasmLogging, loadRelayNodeInfo, RelayNodeInfo, startWasmNode, WasmNodeInstance, getWallets } from './utils/wasm_utils.js';
import { TestClient } from 'viem'

// THE THIRD NODE IS THE MALICIOUS NODE THAT WILL BE TESTED

describe('WASM NODE & RELAY NODE INTERACTIONS', () => {
  let relayInfo: RelayNodeInfo | null = null;
  let walletClient: TestClient | null = null;
  let wasm_client_address: string | undefined = undefined;
  let sender_client_address = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
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

    walletClient = getWallets()[2];
    wasm_client_address = walletClient!.account!.address;
    console.log("3 wasm client address", wasm_client_address);

    // Initialize WASM module
    // const wasmUrl = '../../node/wasm/pkg/wasm_node.js';
    try {
        await init();
        console.log('✅ WASM module initialized');
        setupWasmLogging();
        logWasmExports();
        await waitForWasmInitialization();
  
      } catch (error) {
        console.error('❌ Failed to initialize WASM module:', error);
      }
  
      wasmNodeInstance = startWasmNode(relayInfo.multiAddr, wasm_client_address!, "Ethereum", false);
      await wasmNodeInstance.promise;
      await new Promise(resolve => setTimeout(resolve, 11000));
      console.log('✅ WASM node started successfully');
  
  })

  test('should initialize malicious WASM node successfully', async () => {
    expect(wasm_client_address).toBeDefined();
    expect(relayInfo).toBeDefined();
    console.log('✅ Malicious node initialization test passed');
    
    // Keep the node connected for other nodes to interact with it
    console.log('⏳ Keeping malicious WASM node connected for other nodes to interact...');
    await new Promise(resolve => setTimeout(resolve, 60000));
})


  afterAll(async () => {
    console.log('🧹 Cleaning up Malicious WASM Node 3...');
  })
})