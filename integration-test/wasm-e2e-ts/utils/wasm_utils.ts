import init, * as wasmModule from '../../../node/wasm/vane_lib/pkg/vane_wasm_node.js';
import { hostFunctions } from '../../../node/wasm/host_functions/main';
import { createTestClient, http, TestClient,walletActions, publicActions, WalletActions, PublicActions } from 'viem';
import { foundry } from 'viem/chains';
import { privateKeyToAccount } from 'viem/accounts';
import { PublicInterfaceWorkerJs } from '../../../node/wasm/vane_lib/pkg/vane_wasm_node.js';
import { logger } from '../../../node/wasm/host_functions/logging.js';
import { getAssociatedTokenAddress, getMint, TOKEN_2022_PROGRAM_ID, TOKEN_PROGRAM_ID } from '@solana/spl-token';
import {
  Connection as SolanaConnection,
  LAMPORTS_PER_SOL,
  Keypair,
  PublicKey,
} from "@solana/web3.js";
import { BackendEvent } from '../../../node/wasm/vane_lib/primitives.js';
import { watchP2pNotifications } from '../../../node/wasm/vane_lib/api.js';

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

  const logBackendEventWithoutData = (event: BackendEvent) => {
    if ('SenderRequestReceived' in event) {
      console.log('🔑 BACKEND EVENT', { SenderRequestReceived: { address: event.SenderRequestReceived.address } });
    } else if ('SenderRequestHandled' in event) {
      console.log('🔑 BACKEND EVENT', { SenderRequestHandled: { address: event.SenderRequestHandled.address } });
    } else if ('ReceiverResponseReceived' in event) {
      console.log('🔑 BACKEND EVENT', { ReceiverResponseReceived: { address: event.ReceiverResponseReceived.address } });
    } else if ('ReceiverResponseHandled' in event) {
      console.log('🔑 BACKEND EVENT', { ReceiverResponseHandled: { address: event.ReceiverResponseHandled.address } });
    } else if ('PeerDisconnected' in event) {
      console.log('🔑 BACKEND EVENT', event);
    } else if ('DataExpired' in event) {
      console.log('🔑 BACKEND EVENT', { DataExpired: { multi_id: event.DataExpired.multi_id } });
    }
  };

export async function waitForReceiverHandledEvent(receiverAddress: string) {
  await new Promise<void>((resolve, reject) => {
    let resolved = false;
    const timeout = setTimeout(() => {
      if (!resolved) {
        resolved = true;
        reject(new Error('Timeout waiting for ReceiverResponseHandled event'));
      }
    }, 60000);

    // Save the original handler and chain it with our specific handler
    const originalHandler = (event: BackendEvent) => {
      logBackendEventWithoutData(event);
    };

    const eventHandler = (event: BackendEvent) => {
      // Call original handler first
      originalHandler(event);
      
      // Check for our specific event
      if (!resolved && 'ReceiverResponseHandled' in event && event.ReceiverResponseHandled) {
        const { address } = event.ReceiverResponseHandled;
        if (address === receiverAddress) {
          resolved = true;
          clearTimeout(timeout);
          resolve();
        }
      }
    };

    watchP2pNotifications(eventHandler);
  });
  
}  

export interface RelayNodeInfo {
  multiAddr: string;
}

export interface WasmNodeInstance {
  promise: Promise<PublicInterfaceWorkerJs | null>;
  isRunning: boolean;
  stop: () => void;
}

export async function loadRelayNodeInfo(): Promise<RelayNodeInfo> {
  return {
    multiAddr: "ws://127.0.0.1:9947"
  };
}

export async function getSplTokenBalance(
  connection: SolanaConnection,
  tokenAddress: PublicKey,   // the mint (a.k.a. token address)
  owner: PublicKey           // the wallet whose balance you want
) {
  // 1) Which token program owns this mint?
  const mintInfo = await connection.getAccountInfo(tokenAddress);
  if (!mintInfo) throw new Error('Mint not found');
  const programId = mintInfo.owner.equals(TOKEN_2022_PROGRAM_ID)
    ? TOKEN_2022_PROGRAM_ID
    : TOKEN_PROGRAM_ID;

  // 2) Derive the owner’s ATA for this mint
  const ata = await getAssociatedTokenAddress(tokenAddress, owner, false, programId);

  // 3) If ATA doesn’t exist → balance is 0
  const ataInfo = await connection.getAccountInfo(ata);
  if (!ataInfo) {
    // Also useful: pull decimals so you can format 0 properly
    const mint = await getMint(connection, tokenAddress, 'confirmed', programId);
    return {
      ata,
      amountRaw: 0n,
      decimals: mint.decimals,
      uiAmount: 0,
      uiAmountString: '0',
      exists: false,
    };
  }

  // 4) Read the balance
  const bal = await connection.getTokenAccountBalance(ata, 'confirmed');
  // bal.value.amount is a decimal-string of raw units; use BigInt for precision
  return {
    ata,
    amountRaw: BigInt(bal.value.amount),
    decimals: bal.value.decimals,
    uiAmount: bal.value.uiAmount ?? Number(bal.value.uiAmountString),
    uiAmountString: bal.value.uiAmountString,
    exists: true,
  };
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

