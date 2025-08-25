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

function setupWasmLogging() {
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
  
  async function waitForWasmInitialization(timeoutMs: number = 15000): Promise<void> {
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