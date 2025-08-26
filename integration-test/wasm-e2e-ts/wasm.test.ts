import { describe, test, expect, beforeAll, afterAll, it } from 'vitest'
import { hostFunctions } from '../../node/wasm/host_functions/main'
import { init as wasmerInit } from '@wasmer/sdk';
import init, * as wasmModule from '../../node/wasm/pkg/wasm_node.js';
import { logWasmExports, waitForWasmInitialization, setupWasmLogging, loadRelayNodeInfo, RelayNodeInfo, startWasmNode, WasmNodeInstance } from './utils/wasm_utils';
import { createTestClient, createWalletClient, http, parseEther, TestClient, WalletClient } from 'viem'
import { foundry, mainnet, sepolia } from 'viem/chains'
import { privateKeyToAccount } from 'viem/accounts'

describe('WASM NODE & RELAY NODE INTERACTIONS', () => {
  let relayInfo: RelayNodeInfo | null = null;
  let walletClient: TestClient | null = null;
  let wasm_client_address: string | undefined = undefined;
  let wasmNodeInstance: WasmNodeInstance | null = null;
  
  beforeAll(async () => {
    
    (globalThis as any).hostFunctions = hostFunctions;
    
    try {
      await init();
      await waitForWasmInitialization();
      logWasmExports();
      setupWasmLogging();
      console.log('✅ WASM initialization completed');
    } catch (error) {
      console.error('❌ WASM initialization failed:', error);
      throw error;
    }

    try {
      relayInfo = await loadRelayNodeInfo();
      console.log('✅ Relay node info loaded');
    } catch (error) {
      console.error('❌ Failed to load relay node info:', error);
      console.error('💡 Make sure to start the relay node first: bun run start-relay');
      throw error;
    }

    walletClient = createTestClient({
      account: privateKeyToAccount('0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80'), 
      chain: foundry,
      mode: 'anvil',
      transport: http(),
    })
    
    wasm_client_address = walletClient.account?.address;
    console.log(`🔑 Created test wallet: ${wasm_client_address}`);    

    wasmNodeInstance = startWasmNode(relayInfo.multiAddr, wasm_client_address!, "Ethereum", false);
    await wasmNodeInstance.promise;

    // Give extra time to observe the reservation process
    await new Promise(resolve => setTimeout(resolve, 150000));
    console.log('✅ P2P observation period completed');
    
  },100000)

  afterAll(async () => {
    if (wasmNodeInstance) {
      console.log('🛑 Stopping WASM node...');
      wasmNodeInstance.stop();
    }
    
    // Note: Relay node is managed externally, so we don't stop it here
    // The relay node should be stopped manually or via the start-relay.js script
    
    await new Promise(resolve => setTimeout(resolve, 3000));
    console.log('✅ WASM node cleanup completed');
  })

  it('window APIs should be available', () => {
    expect(window).toBeDefined()  
    expect(document).toBeDefined()
    expect(WebAssembly).toBeDefined()
    expect(window.fetch).toBeDefined()
    expect(window.WebSocket).toBeDefined()
    expect(window.crypto).toBeDefined()
    expect(window.localStorage).toBeDefined()
  })

 
})