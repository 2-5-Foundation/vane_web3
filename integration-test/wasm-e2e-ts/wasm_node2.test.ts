import { describe, test, expect, beforeAll, afterAll, it } from 'vitest'
import { hostFunctions } from '../../node/wasm/host_functions/main.js'
import init, * as wasmModule from '../../node/wasm/pkg/vane_wasm_node.js';
import { logWasmExports, waitForWasmInitialization, setupWasmLogging, loadRelayNodeInfo, RelayNodeInfo, startWasmNode, WasmNodeInstance, getWallets } from './utils/wasm_utils.js';
import { TestClient } from 'viem'

// THE SECOND NODE TEST IS THE SAME AS THE FIRST NODE TEST BUT WITH A DIFFERENT WALLET

describe('WASM NODE & RELAY NODE INTERACTIONS', () => {
  let relayInfo: RelayNodeInfo | null = null;
  let walletClient: TestClient | null = null;
  let wasm_client_address: string | undefined = undefined;
  let sender_client_address = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
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

    walletClient = getWallets()[1];
    wasm_client_address = walletClient!.account!.address;
    console.log("2 wasm client address", wasm_client_address);

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

  it("should assert",async() => {
    expect(1 + 1).toEqual(2)
    
    // Keep the node connected for other nodes to interact with it
    console.log('⏳ Keeping WASM node 2 connected for other nodes to interact...');
    await new Promise(resolve => setTimeout(resolve, 60000));
})

})