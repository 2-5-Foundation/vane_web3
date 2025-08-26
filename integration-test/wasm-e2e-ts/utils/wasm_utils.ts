import init, * as wasmModule from '../../../node/wasm/pkg/wasm_node.js';
import { hostFunctions } from '../../../node/wasm/host_functions/main';


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
    console.log('🔧 Setting up WASM logging...');
    
    // Set debug level for comprehensive logging
    try {
      hostFunctions.hostLogging.setLogLevel(hostFunctions.hostLogging.LogLevel.Debug);
      console.log('✅ WASM logging level set to Debug');
      
      // Log a test message to verify logging works
      hostFunctions.hostLogging.log(
        hostFunctions.hostLogging.LogLevel.Info,
        "TestFramework",
        "WASM logging system initialized successfully",
        "test-module",
        "wasm-node.test.ts",
        1
      );
      
      console.log('✅ WASM logging system verified');
    } catch (error) {
      console.warn('⚠️ Failed to setup WASM logging:', error);
    }
  }
  
  export async function waitForWasmInitialization(timeoutMs: number = 15000): Promise<void> {
    console.log('⏳ Waiting for WASM initialization...');
    
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
  promise: Promise<any>;
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

export function startWasmNode(
  relayMultiAddr: string, 
  walletAddress: string, 
  network: string = "Ethereum", 
  live: boolean = false
): WasmNodeInstance {
  let isRunning = true;
  let abortController: AbortController | null = null;

  // Create an abort controller to handle cancellation
  abortController = new AbortController();

  const wasmPromise = wasmModule.start_vane_web3(relayMultiAddr, walletAddress, network, live)
    .then((result) => {
      console.log('✅ WASM node started successfully:', result);
      return result;
    })
    .catch((error) => {
      if (!abortController?.signal.aborted) {
        console.error('❌ WASM node failed to start:', error);
        isRunning = false;
        throw error;
      }
      console.log('🛑 WASM node startup cancelled');
      return null;
    });

  const stop = () => {
    if (isRunning && abortController) {
      console.log('🛑 Stopping WASM node...');
      isRunning = false;
      abortController.abort();
      
      // Note: The WASM module doesn't have a built-in stop function,
      // so we rely on the abort controller to cancel the promise.
      // In a production environment, you would want to implement
      // a proper shutdown mechanism in the Rust code.
    }
  };

  return {
    promise: wasmPromise,
    isRunning,
    stop
  };
}