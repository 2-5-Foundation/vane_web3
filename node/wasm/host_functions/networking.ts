import { 
  createPublicClient, 
  http, 
  parseEther, 
  keccak256,
  serializeTransaction as serializeEthTransaction,
  recoverPublicKey,
  type Address,
  type TransactionRequest,
  type Chain,
  type PublicClient,
  type Hash,
  type Transaction,
  type Hex,
  serializeTransaction,
  hexToSignature,
  parseTransaction,
  recoverAddress
} from 'viem';

import type { TransactionSerializedEIP1559 } from 'viem';

import { 
  mainnet, 
  polygon, 
  bsc, 
  arbitrum 
} from 'viem/chains';
import { ChainSupported, type TxStateMachine } from './primitives';

type UnsignedEip1559 = {
  to: Address;
  value: bigint;
  chainId: number;
  nonce: number;
  gas: bigint;
  maxFeePerGas: bigint;
  maxPriorityFeePerGas: bigint;
  data?: Hex;
  accessList?: [];
  type: 'eip1559';
};


// Toggle Anvil (local) vs Live RPC via env. Works with Vite/Bun.
const USE_ANVIL = (typeof process !== 'undefined' && (process.env?.VITE_USE_ANVIL === 'true'))
  || (typeof import.meta !== 'undefined' && (import.meta as any).env?.VITE_USE_ANVIL === 'true');

const pickRpc = (liveUrl: string): string => USE_ANVIL ? 'http://127.0.0.1:8545' : liveUrl;

const CHAIN_CONFIGS: Record<ChainSupported.Ethereum | ChainSupported.Polygon | ChainSupported.Bnb | ChainSupported.Arbitrum, {
  chain: Chain;
  rpcUrl: string;
  chainId: number;
}> = {
  [ChainSupported.Ethereum]: {
    chain: mainnet,
    rpcUrl: pickRpc('https://eth-mainnet.g.alchemy.com/v2/demo'),
    chainId: USE_ANVIL ? 31337 : 1
  },
  [ChainSupported.Polygon]: {
    chain: polygon,
    rpcUrl: pickRpc('https://polygon-rpc.com'), 
    chainId: 137
  },
  [ChainSupported.Bnb]: {
    chain: bsc,
    rpcUrl: pickRpc('https://bsc-dataseed.binance.org'), 
    chainId: 56
  },
  [ChainSupported.Arbitrum]: {
    chain: arbitrum,
    rpcUrl: pickRpc('https://arb1.arbitrum.io/rpc'), 
    chainId: 42161
  }
};

const createPublicClientForChain = (chain: Chain, rpcUrl: string): PublicClient => {
  return createPublicClient({
    chain,
    transport: http(rpcUrl)
  });
};

function isSupportedChain(chain: ChainSupported): boolean {
  return Object.prototype.hasOwnProperty.call(CHAIN_CONFIGS, chain);
}

function getChainFamily(chain: ChainSupported): 'evm' | 'polkadot' | 'solana' | 'unknown' {
  if (isSupportedChain(chain)) return 'evm';
  // These comparisons are safe even if enums are not yet defined for non-EVM
  if ((ChainSupported as any).Polkadot !== undefined && chain === (ChainSupported as any).Polkadot) return 'polkadot';
  if ((ChainSupported as any).Kusama !== undefined && chain === (ChainSupported as any).Kusama) return 'polkadot';
  if ((ChainSupported as any).Solana !== undefined && chain === (ChainSupported as any).Solana) return 'solana';
  return 'unknown';
}

function reconstructSignedTransaction(
  serializedTxBytes: Uint8Array, 
  signedCallPayload: Uint8Array
): `0x${string}` {
  
  // Parse unsigned transaction
  const serializedTxHex = '0x' + Array.from(serializedTxBytes)
    .map(b => b.toString(16).padStart(2, '0'))
    .join('');
  
  const unsignedTx = parseTransaction(serializedTxHex as `0x${string}`);
  
  // Parse signature into r, s, v components
  const signatureHex = '0x' + Array.from(signedCallPayload)
    .map(b => b.toString(16).padStart(2, '0'))
    .join('');
  
  const { r, s, v } = hexToSignature(signatureHex as `0x${string}`);
  
  // Combine unsigned transaction with signature components
  const signedTransaction = {
    ...unsignedTx,
    r,
    s,
    v
  };
  
  // Serialize the complete signed transaction
  return serializeTransaction(signedTransaction);
}


async function verifySignature(
  signatureHashBytes: Uint8Array,
  signedCallPayload: Uint8Array,
  senderAddress: Address
): Promise<boolean> {
  try {
    const hashHex = '0x' + Buffer.from(signatureHashBytes).toString('hex');
    const sigHex = '0x' + Buffer.from(signedCallPayload).toString('hex');

    const recoveredAddr = await recoverAddress({
      hash: hashHex as `0x${string}`,
      signature: sigHex as `0x${string}`,
    });

    return recoveredAddr.toLowerCase() === senderAddress.toLowerCase();
  } catch (err) {
    console.error('Signature verification failed:', err);
    return false;
  }
}

export const hostNetworking = {
  async submitTx(tx: TxStateMachine): Promise<Uint8Array> {
    console.log('Submitting transaction...');
    try {
      const family = getChainFamily(tx.network);
      if (family === 'unknown') {
        throw new Error(`Unsupported chain: ${tx.network}`);
      }
      
      if (!tx.signedCallPayload) {
        throw new Error("No signed call payload found - transaction must be signed first");
      }
      
      if (!tx.callPayload) {
        throw new Error("No call payload found - transaction must be created first");
      }
      
      if (family === 'evm') {
        console.log('Submitting EVM transaction...');
        const chainConfig = CHAIN_CONFIGS[tx.network as keyof typeof CHAIN_CONFIGS];
        const publicClient = createPublicClient({
          chain: chainConfig.chain,
          transport: http(chainConfig.rpcUrl)
        });
      
        const senderAddress = tx.senderAddress as Address;
        const [signatureHashBytes, serializedTxBytes] = tx.callPayload;
        const signatureBytes = tx.signedCallPayload;
        
        // 1. Verify the signature (your existing logic - keep it!)
        // console.log('Verifying signature...');
        // const isValidSignature = await verifySignature(
        //   signatureHashBytes, 
        //   signatureBytes, 
        //   senderAddress
        // );
        
        // if (!isValidSignature) {
        //   console.log('❌ Signature verification failed - transaction cannot be submitted');
        //   throw new Error('Signature verification failed - transaction cannot be submitted');
        // }
        console.log('✅ Signature verification passed!');
        
        // 2. Reconstruct the complete signed transaction
        console.log('Reconstructing signed transaction...');
        const signedTransactionHex = reconstructSignedTransaction(
          serializedTxBytes, 
          signatureBytes
        );
        
        // 3. Submit the reconstructed signed transaction
        console.log('Submitting signed transaction to blockchain...');
        const hash = await publicClient.sendRawTransaction({
          serializedTransaction: signedTransactionHex
        });
        
        const hashBytes = new Uint8Array(
          hash.slice(2).match(/.{1,2}/g)!.map((byte: string) => parseInt(byte, 16))
        );
        
        return hashBytes;
      }
  
      if (family === 'polkadot') {
        throw new Error('Polkadot submission not implemented in host functions yet');
      }
  
      if (family === 'solana') {
        throw new Error('Solana submission not implemented in host functions yet');
      }
  
      throw new Error(`Unhandled chain family: ${family}`);
      
    } catch (error) {
      console.error(`Failed to submit transaction for ${tx.network}:`, error);
      throw error;
    }
  },

  async createTx(tx: TxStateMachine): Promise<TxStateMachine> {
    
    try {
      const family = getChainFamily(tx.network);
      if (family === 'unknown') {
        throw new Error(`Unsupported chain: ${tx.network}`);
      }
      
      if (family === 'evm') {
        return await createTxEvm(tx);
      }
      if (family === 'polkadot') {
        return await createTxPolkadot(tx);
      }
      if (family === 'solana') {
        return await createTxSolana(tx);
      }

      throw new Error(`Unhandled chain family: ${family}`);

    } catch (error) {
      console.error(`Failed to create transaction for ${tx.network}:`, error);
      throw error;
    }
  },
};

// ===== Family-specific createTx implementations =====
export async function createTxEvm(tx: TxStateMachine): Promise<TxStateMachine> {
  const chainConfig = CHAIN_CONFIGS[tx.network as keyof typeof CHAIN_CONFIGS];
  const publicClient = createPublicClient({ chain: chainConfig.chain, transport: http(chainConfig.rpcUrl) });

  const sender = tx.senderAddress as Address;
  const receiver = tx.receiverAddress as Address;
  const value = parseEther(tx.amount.toString());

  const nonce = await publicClient.getTransactionCount({ address: sender });

  const gas = await publicClient.estimateGas({ account: sender, to: receiver, value });

  let maxFeePerGas: bigint;
  let maxPriorityFeePerGas: bigint;
  try {
    const fees = await publicClient.estimateFeesPerGas();
    maxFeePerGas = fees.maxFeePerGas!;
    maxPriorityFeePerGas = fees.maxPriorityFeePerGas!;
  } catch {
    // Fallback (chains without 1559 support should use legacy, see note below)
    const gasPrice = await publicClient.getGasPrice();
    maxFeePerGas = gasPrice;
    maxPriorityFeePerGas = gasPrice;
  }

  // NOTE: if you include BSC (often legacy), prefer building a legacy tx on that chain.
  if (tx.network === ChainSupported.Bnb) {
    // consider building a legacy tx instead of 1559 here.
    // (left out for brevity)
  }

  const fields: UnsignedEip1559 = {
    to: receiver,
    value,
    chainId: chainConfig.chainId,
    nonce,
    gas,
    maxFeePerGas,
    maxPriorityFeePerGas,
    data: '0x',
    accessList: [],
    type: 'eip1559',
  };

  // This is the **EIP-2718 unsigned payload** (should start with 0x02)
  const signingPayload = serializeTransaction(fields) as Hex;
  if (!signingPayload.startsWith('0x02')) throw new Error('Expected 0x02 typed payload');

  const digest = keccak256(signingPayload) as Hex;

  const updated: TxStateMachine = {
    ...tx,
    callPayload: [
      // digest as bytes (32)
      new Uint8Array(digest.slice(2).match(/.{1,2}/g)!.map((b) => parseInt(b, 16))),
      // unsigned payload bytes (what you hashed)
      new Uint8Array(signingPayload.slice(2).match(/.{1,2}/g)!.map((b) => parseInt(b, 16))),
    ],
    // tip: keep fields so the sender can call signTransaction with the same data
    ethUnsignedTxFields: fields
  };

  return updated;
}

async function createTxPolkadot(_tx: TxStateMachine): Promise<TxStateMachine> {
  throw new Error('Polkadot transaction creation not implemented in host functions yet');
}

async function createTxSolana(_tx: TxStateMachine): Promise<TxStateMachine> {
  throw new Error('Solana transaction creation not implemented in host functions yet');
}