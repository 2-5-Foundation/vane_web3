import init, * as wasmModule from '../../../node/wasm/vane_lib/pkg/vane_wasm_node.js';
import { hostFunctions } from '../../../node/wasm/host_functions/main';
import { createTestClient, http, TestClient,walletActions, publicActions, WalletActions, PublicActions } from 'viem';
import { foundry } from 'viem/chains';
import { privateKeyToAccount } from 'viem/accounts';
import { PublicInterfaceWorkerJs } from '../../../node/wasm/vane_lib/pkg/vane_wasm_node.js';
import { logger } from '../../../node/wasm/host_functions/logging.js';


export function logWasmExports() {
    console.log('📦 WASM Module Exports:');
    
    const exports = Object.keys(wasmModule);
    console.log(`🔧 Total exports: ${exports.length}`);
    
    const functions = [];
    const classes = [];
    const constants = [];
    const other = [];
    
    for (const exportName of exports) {
      const exportValue = (wasmModule as any)[exportName];
      const type = typeof exportValue;
      
      if (type === 'function') {
        functions.push(exportName);
      } else if (type === 'object' && exportValue?.constructor?.name) {
        classes.push(`${exportName} (${exportValue.constructor.name})`);
      } else if (type === 'number' || type === 'string' || type === 'boolean') {
        constants.push(`${exportName}: ${exportValue}`);
      } else {
        other.push(`${exportName} (${type})`);
      }
    }
    
    if (functions.length > 0) {
      console.log(`\n🔧 Functions (${functions.length}):`);
      functions.forEach(name => console.log(`  - ${name}`));
    }
    
    if (classes.length > 0) {
      console.log(`\n📦 Classes/Objects (${classes.length}):`);
      classes.forEach(name => console.log(`  - ${name}`));
    }
    
    if (constants.length > 0) {
      console.log(`\n📊 Constants (${constants.length}):`);
      constants.forEach(name => console.log(`  - ${name}`));
    }
    
    if (other.length > 0) {
      console.log(`\n❓ Other exports (${other.length}):`);
      other.forEach(name => console.log(`  - ${name}`));
    }
    console.log("xxxxxxxxxxxxxxxxxxxxxx END LOGGING WASM EXPORTS xxxxxxxxxxxxxxxxxxxxxxx")    
}

export function setupWasmLogging() {    
  try {
    hostFunctions.hostLogging.setLogLevel(
      hostFunctions.hostLogging.LogLevel.Debug
    );

    // ✅ always return the same logger instance used by hostLogging
    return hostFunctions.hostLogging.getLogInstance();
  } catch (error) {
    console.warn('⚠️ Failed to setup WASM logging:', error);
    return null;
  }
}
  
  export async function waitForWasmInitialization(timeoutMs: number = 15000): Promise<void> {    
    const startTime = Date.now();
    const checkInterval = 500;
    
    return new Promise((resolve, reject) => {
      const timeoutId = setTimeout(() => {
        reject(new Error(`WASM initialization timeout after ${timeoutMs}ms`));
      }, timeoutMs);
      
      const checkInitialization = () => {
        const elapsed = Date.now() - startTime;
        
        try {
          // Try to access WASM exports to verify it's ready
          if (wasmModule && typeof wasmModule.start_vane_web3 === 'function') {
            console.log(`✅ WASM fully initialized after ${elapsed}ms`);
            clearTimeout(timeoutId);
            resolve();
            return;
          }
        } catch (error) {
          // Still initializing, continue checking
        }
        
        if (elapsed < timeoutMs) {
          console.log(`⏳ Still initializing... (${elapsed}ms elapsed)`);
          setTimeout(checkInitialization, checkInterval);
        }
      };
      
      setTimeout(checkInitialization, checkInterval);
    });
  }


export interface RelayNodeInfo {
  peerId: string;
  multiAddr: string;
}

export interface WasmNodeInstance {
  promise: Promise<PublicInterfaceWorkerJs | null>;
  isRunning: boolean;
  stop: () => void;
}

/**
 * Reads relay node info from the file created by the start-relay.js script
 * This allows browser tests to get the real multiaddr without using child_process
 */
export async function loadRelayNodeInfo(timeoutMs: number = 10000): Promise<RelayNodeInfo> {
  console.log(`🔍 Loading relay node info from relay-info.json`);
  
  return new Promise<RelayNodeInfo>((resolve, reject) => {
    const timeoutId = setTimeout(() => {
      reject(new Error(`Relay info not available within ${timeoutMs}ms. Make sure to start the relay node first using: bun run start-relay`));
    }, timeoutMs);

    const checkRelayInfo = async () => {
      try {
        // In browser environment, we'll fetch the file via HTTP
        const response = await fetch('/relay-info.json');
        if (!response.ok) {
          throw new Error(`HTTP ${response.status}: ${response.statusText}`);
        }
        
        const relayInfo = await response.json();
        
        // Proceed as soon as essential fields are present; don't require ready: true
        if (relayInfo.peerId && relayInfo.multiAddr) {
          clearTimeout(timeoutId);
          console.log(`✅ Loaded relay node info`);
          console.log(`🎯 Peer ID: ${relayInfo.peerId}`);
          console.log(`🔗 MultiAddr: ${relayInfo.multiAddr}`);
          
          resolve({
            peerId: relayInfo.peerId,
            multiAddr: relayInfo.multiAddr
          });
        } else {
          // Relay not ready yet, check again
          setTimeout(checkRelayInfo, 500);
        }
      } catch (error) {
        // File not found or not ready, check again
        setTimeout(checkRelayInfo, 500);
      }
    };

    // Start checking immediately
    checkRelayInfo();
  });
}

export function getWallets(): [TestClient & WalletActions & PublicActions,string][] {
  const [walletClient1,privkey1] = [createTestClient({
    account: privateKeyToAccount('0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80'), 
    chain: foundry,
    mode: 'anvil',
    transport: http(),
  }).extend(walletActions).extend(publicActions),'0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80']

  const [walletClient2,privkey2] = [createTestClient({
    account: privateKeyToAccount('0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d'), 
    chain: foundry,
    mode: 'anvil',
    transport: http(),
  }).extend(walletActions).extend(publicActions),'0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d']

  const [walletClient3,privkey3] = [createTestClient({
    account: privateKeyToAccount('0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a'), 
    chain: foundry,
    mode: 'anvil',
    transport: http(),
  }).extend(walletActions).extend(publicActions),'0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a']
  
  const [walletClient4,privkey4] = [createTestClient({
    account: privateKeyToAccount('0x7c852118294e51e653712a81e05800f419141751be58f605c371e15141b007a6'), 
    chain: foundry,
    mode: 'anvil',
    transport: http(),
  }).extend(walletActions).extend(publicActions),'0x7c852118294e51e653712a81e05800f419141751be58f605c371e15141b007a6']
  
  return [[walletClient1,privkey1], [walletClient2,privkey2], [walletClient3,privkey3], [walletClient4,privkey4]];
}

