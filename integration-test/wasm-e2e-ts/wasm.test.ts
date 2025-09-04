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
  let walletClient1: TestClient | null = null;
  let walletClient2: TestClient | null = null;
  let wasm_client_address1: string | undefined = undefined;
  let wasm_client_address2: string | undefined = undefined;
  let wasmNodeInstance1: WasmNodeInstance | null = null;
  let wasmNodeInstance2: WasmNodeInstance | null = null;
  
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

    walletClient1 = createTestClient({
      account: privateKeyToAccount('0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80'), 
      chain: foundry,
      mode: 'anvil',
      transport: http(),
    })

    walletClient2 = createTestClient({
      account: privateKeyToAccount('0x8da4ef21b864d2cc526dbdb2a120bd2874c36c9d0a1fb7f8c63d7f7a8b41de8f'), 
      chain: foundry,
      mode: 'anvil',
      transport: http(),
    })
    
    wasm_client_address1 = walletClient1.account?.address;
    wasm_client_address2 = walletClient2.account?.address;
    console.log(`🔑 Created test wallet: ${wasm_client_address1}`);    
    console.log(`🔑 Created test wallet: ${wasm_client_address2}`);    

    wasmNodeInstance1 = startWasmNode(relayInfo.multiAddr, wasm_client_address1!, "Ethereum", false);
    await wasmNodeInstance1.promise;

    // wasmNodeInstance2 = startWasmNode(relayInfo.multiAddr, wasm_client_address2!, "Ethereum", false);
    // await wasmNodeInstance2.promise;

    // Give extra time to observe the reservation process
    await new Promise(resolve => setTimeout(resolve, 150000));
    console.log('✅ P2P observation period completed');
    
  },100000)

  afterAll(async () => {
    if (wasmNodeInstance1) {
      console.log('🛑 Stopping WASM node...');
      wasmNodeInstance1.stop();
    }
    // if (wasmNodeInstance2) {
    //   console.log('🛑 Stopping WASM node...');
    //   wasmNodeInstance2.stop();
    // }
    
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