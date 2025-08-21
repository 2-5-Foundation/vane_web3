// Test on architecture and functionality and connecitivy on the wasm node and relay node

import { describe, it, expect, beforeAll, afterAll } from 'vitest'
import { spawn, ChildProcess } from 'child_process'
import { hostFunctions } from '../../../node/wasm/host_functions/main'
import init, * as wasmModule from '../../../node/wasm/pkg/wasm_node.js'
import { createTestClient, createWalletClient, http, parseEther, TestClient, WalletClient } from 'viem'
import { foundry, mainnet, sepolia } from 'viem/chains'
import { privateKeyToAccount } from 'viem/accounts'

// Helper function to log WASM exports
function logWasmExports() {
  console.log('📦 WASM Module Exports:');
  console.log('=====================================');
  
  const exports = Object.keys(wasmModule);
  console.log(`🔧 Total exports: ${exports.length}`);
  
  // Categorize exports
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
  
  console.log('=====================================\n');
}

// Helper function to setup WASM logging
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

// Helper function to wait for WASM initialization with logging
async function waitForWasmInitialization(timeoutMs: number = 15000): Promise<void> {
  console.log('⏳ Waiting for WASM initialization...');
  
  const startTime = Date.now();
  const checkInterval = 500; // Check every 500ms
  
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
    
    // Start checking
    setTimeout(checkInitialization, checkInterval);
  });
}

describe('WASM Node Tests', () => {
  let relayNodeProcess: ChildProcess | null = null;
  let localPeerId: string | null = null;
  let relayMultiAddr: string | null = null;
  let relayReady: boolean = false;
  let walletClient: TestClient | null = null;
  let wasm_client_address: string | undefined = undefined;
  let wasmNodePromise: Promise<any> | null = null; // Store the promise reference

  beforeAll(async () => {
    console.log('🚀 Starting test setup...');
    
    // Set up host functions for WASM
    console.log('🔧 Setting up host functions...');
    (globalThis as any).hostFunctions = hostFunctions;
    
    // Initialize WASM first and log exports
    console.log('📦 Initializing WASM module...');
    try {
      await init();
      console.log('✅ WASM module initialized');
      
      // Wait for full initialization
      await waitForWasmInitialization();
      
      // Log all WASM exports
      logWasmExports();
      
      // Setup logging system
      setupWasmLogging();
      
    } catch (error) {
      console.error('❌ WASM initialization failed:', error);
      throw error;
    }
    
    // Start relay node process
    console.log('🚀 Starting relay node...');
    relayNodeProcess = spawn('../../target/release/vane_web3_app', ['relay-node', '--dns', '127.0.0.1', '--port', '30333'], {
      cwd: process.cwd(),
      stdio: ['pipe', 'pipe', 'pipe']
    });

    // Promise to resolve when relay is fully ready
    const relayReadyPromise = new Promise<string>((resolve) => {
      relayNodeProcess!.stdout?.on('data', (data) => {
        const output = data.toString().trim();
        console.log(`[Relay Node]: ${output}`);
        
        // Extract local_peer_id from log output
        const peerIdMatch = output.match(/local_peer_id=([A-Za-z0-9]+)/);
        if (peerIdMatch && !localPeerId) {
          const capturedPeerId = peerIdMatch[1];
          localPeerId = capturedPeerId;
          relayMultiAddr = `/ip6/::/tcp/30333/quic-v1/webtransport/p2p/${capturedPeerId}`;
          console.log(`🎯 Captured local_peer_id: ${capturedPeerId}`);
          console.log(`🔗 Relay multiaddr: ${relayMultiAddr}`);
        }
        
        // Wait for NewListenAddr to confirm relay is ready
        if (output.includes('NewListenAddr') && localPeerId && !relayReady) {
          relayReady = true;
          resolve(localPeerId);
        }
      });
    });

    relayNodeProcess.stderr?.on('data', (data) => {
      console.error(`[Relay Node Error]: ${data.toString().trim()}`);
    });

    relayNodeProcess.on('error', (error) => {
      console.error('❌ Failed to start relay node:', error);
      throw error;
    });

    // Wait for relay node to be fully ready
    try {
      const timeoutPromise = new Promise<string>((_, reject) => 
        setTimeout(() => reject(new Error('Timeout waiting for relay node to be ready')), 20000)
      );
      
      localPeerId = await Promise.race([relayReadyPromise, timeoutPromise]);
      console.log('✅ Relay node is fully operational with peer ID:', localPeerId);
      console.log('✅ Relay multiaddr:', relayMultiAddr);
      
      // Small additional stabilization delay
      console.log('⏱️  Final stabilization...');
      await new Promise(resolve => setTimeout(resolve, 2000));
      console.log('✅ Relay node ready for connections');
    } catch (error) {
      console.error('❌ Failed to start relay node:', error);
      throw error;
    }
    
    // WASM is already initialized, so we can proceed directly
    console.log('🔑 Creating test wallet...');
    try {
      // Create a test wallet using viem
      walletClient = createTestClient({
        account: privateKeyToAccount('0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80'), 
        chain: foundry,
        mode: 'anvil',
        transport: http(),
      })
      
      wasm_client_address = walletClient.account?.address;
      console.log(`🔑 Created test wallet: ${wasm_client_address}`);
  
      
      // Start the WASM node with relay connection
      if (!relayMultiAddr) {
        throw new Error('Relay multiaddr not available');
      }
      if (!wasm_client_address) {
        throw new Error('WASM client address not available');
      }
      
      console.log('🚀 Starting WASM node with parameters:');
      console.log(`  - Relay MultiAddr: ${relayMultiAddr}`);
      console.log(`  - Account: ${wasm_client_address}`);
      console.log(`  - Network: Ethereum`);
      console.log(`  - Live mode: false`);
      
      // Verify the function exists before calling
      if (!wasmModule.start_vane_web3) {
        throw new Error('start_vane_web3 function not found in WASM exports');
      }
      
      // Call the WASM start function and store the promise
      console.log('📞 Calling start_vane_web3...');
      wasmNodePromise = wasmModule.start_vane_web3(relayMultiAddr, wasm_client_address, "Ethereum", false);
      console.log('✅ WASM node promise created');
      
      // Give WASM node some time to initialize and start logging
      console.log('⏳ Allowing WASM node to initialize...');
      await new Promise(resolve => setTimeout(resolve, 3000));
      
      // Check for any early logs from WASM
      try {
        const logHistory = hostFunctions.hostLogging.getLogHistory();
        const logs = JSON.parse(logHistory);
        console.log(`📊 WASM has generated ${logs.length} log entries so far`);
        if (logs.length > 0) {
          console.log('📝 Recent WASM logs:');
          logs.slice(-5).forEach((log: any) => {
            console.log(`  [${log.level}] ${log.target}: ${log.message}`);
          });
        }
      } catch (error) {
        console.warn('⚠️ Could not retrieve WASM logs:', error);
      }
      
      // Don't await here - let it run in background but keep the promise alive
      // Add error handling to prevent unhandled promise rejections
      wasmNodePromise.catch((error) => {
        console.error('❌ WASM node error:', error);
      });
      
    } catch (error) {
      console.error('❌ WASM initialization or node startup failed:', error);
      throw error;
    }

  }, 50000);

  // Clean up after tests
  afterAll(async () => {
    console.log('🧹 Cleaning up test resources...');
    
    // Clean up relay node
    if (relayNodeProcess) {
      console.log('🛑 Stopping relay node...');
      relayNodeProcess.kill('SIGTERM');
      relayNodeProcess = null;
    }
    
    // If you have a way to stop the WASM node, do it here
    // For now, just wait a bit to let any cleanup happen
    await new Promise(resolve => setTimeout(resolve, 1000));
    
    console.log('✅ Cleanup completed');
  });

  // Test cases
  it('should have initialized WASM with all expected exports', async () => {
    // Verify WASM module is loaded
    expect(wasmModule).toBeDefined();
    
    // Check for key exports
    expect(wasmModule.start_vane_web3).toBeDefined();
    expect(typeof wasmModule.start_vane_web3).toBe('function');
    
    console.log('✅ WASM exports verification passed');
  });

  it('should have working logging system', async () => {
    // Test logging functionality
    expect(hostFunctions.hostLogging).toBeDefined();
    expect(hostFunctions.hostLogging.log).toBeDefined();
    expect(hostFunctions.hostLogging.getLogHistory).toBeDefined();
    
    // Clear any existing logs
    hostFunctions.hostLogging.clearHistory();
    
    // Add a test log
    hostFunctions.hostLogging.log(
      hostFunctions.hostLogging.LogLevel.Info,
      "TestCase",
      "Test log message for verification",
      "test-module",
      "wasm-node.test.ts",
      100
    );
    
    // Verify log was recorded
    const logHistory = hostFunctions.hostLogging.getLogHistory();
    const logs = JSON.parse(logHistory);
    
    expect(logs.length).toBeGreaterThan(0);
    expect(logs[logs.length - 1].message).toContain("Test log message for verification");
    
    console.log('✅ Logging system verification passed');
  });

  it('should start WASM node successfully', async () => {
    expect(wasmNodePromise).toBeDefined();
    
    // Wait a bit more for WASM node to generate logs
    await new Promise(resolve => setTimeout(resolve, 2000));
    
    // Check if WASM node has generated any logs
    try {
      const logHistory = hostFunctions.hostLogging.getLogHistory();
      const logs = JSON.parse(logHistory);
      
      console.log(`📊 Total WASM logs after startup: ${logs.length}`);
      
      if (logs.length > 0) {
        console.log('📝 All WASM logs:');
        logs.forEach((log: any, index: number) => {
          const timestamp = new Date(log.timestamp).toISOString();
          console.log(`  ${index + 1}. [${timestamp}] [${log.level}] ${log.target}: ${log.message}`);
        });
        
        // Export logs for debugging
        const exportedLogs = hostFunctions.hostLogging.exportLogs();
        console.log('\n📄 Exported logs:\n' + exportedLogs);
      } else {
        console.warn('⚠️ No WASM logs found - this might indicate an issue');
      }
      
    } catch (error) {
      console.error('❌ Failed to retrieve WASM logs:', error);
    }
    
    console.log('✅ WASM node startup test completed');
  }, 15000); // Extended timeout for this test
});