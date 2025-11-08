import { 
  createPublicClient, 
  http, 
  parseEther, 
  parseUnits,
  keccak256,
  recoverPublicKey,
  type Address,
  type TransactionRequest,
  type Chain,
  type PublicClient,
  type Hash,
  type Transaction,
  type Hex,
  serializeTransaction,
  parseSignature,
  parseTransaction,
  recoverAddress,
  encodeFunctionData,
  getContract,
  erc20Abi,
  bytesToHex,
  formatEther,
  hexToBytes
} from 'viem';

import type { TransactionSerializedEIP1559 } from 'viem';

import { 
  mainnet, bsc
} from 'viem/chains';
import {
  Connection as SolanaConnection,
  SystemProgram,
  LAMPORTS_PER_SOL,
  PublicKey,
  VersionedTransaction,
  TransactionMessage,
  VersionedMessage,
} from "@solana/web3.js";
import {
  getAssociatedTokenAddress, createTransferCheckedInstruction,
  createAssociatedTokenAccountInstruction,getMint,
  TOKEN_PROGRAM_ID, TOKEN_2022_PROGRAM_ID
} from '@solana/spl-token';

import { TronWeb } from 'tronweb';


import bs58 from 'bs58';

import { ChainSupported, type TxStateMachine, type Token } from '../../primitives';


export type UnsignedEip1559 = {
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

export type UnsignedLegacy = {
  to: Address;
  value: bigint;
  chainId: number;
  nonce: number;
  gas: bigint;
  gasPrice: bigint;
  data?: Hex;
  type: 'legacy';
};


// Toggle Anvil (local) vs Live RPC via env. Works with Vite/Bun.
const USE_ANVIL = (typeof process !== 'undefined' && (process.env?.VITE_USE_ANVIL === 'true'))
  || (typeof import.meta !== 'undefined' && (import.meta as any).env?.VITE_USE_ANVIL === 'true');

// In live mode, route RPC calls through Next.js API so the server component holds the API key.
// We use an absolute path from the app root: "/api/transaction".
// In local Anvil mode, we use the local JSON-RPC endpoint.
const pickRpc = (nextApiRoute: string, chain: ChainSupported): string => {
  if (USE_ANVIL) {
    if (chain === ChainSupported.Ethereum) return 'http://127.0.0.1:8545';
    if (chain === ChainSupported.Bnb) return 'http://127.0.0.1:8555';
  }
  // Ensure it starts with a leading slash for Next.js API routes
  return nextApiRoute.startsWith('/') ? nextApiRoute : `/${nextApiRoute}`;
};

const CHAIN_CONFIGS: Record<ChainSupported.Ethereum | ChainSupported.Bnb, {
  chain: Chain;
  rpcUrl: string;
  chainId: number;
}> = {
  [ChainSupported.Ethereum]: {
    chain: mainnet,
    // Proxy via Next.js API route in live mode to get prepared chain data
    rpcUrl: pickRpc('/api/create-tx/prepare-evm', ChainSupported.Ethereum),
    chainId: USE_ANVIL ? 31337 : 1
  },
  [ChainSupported.Bnb]: {
    chain: bsc,
    // Proxy via Next.js API route in live mode to get prepared chain data
    rpcUrl: pickRpc('/api/create-tx/prepare-bsc', ChainSupported.Bnb),
    chainId: USE_ANVIL ? 2192 : 56
  }
};

function getChainFamily(chain: ChainSupported): 'ethereum' | 'bsc' | 'solana' | 'polkadot' | 'base' | 'optimism' | 'arbitrum' | 'polygon' | 'unknown' {
  if (chain === ChainSupported.Ethereum) return 'ethereum';
  if (chain === ChainSupported.Bnb) return 'bsc';
  if (chain === ChainSupported.Solana) return 'solana';
  if (chain === ChainSupported.Polkadot) return 'polkadot';
  if (chain === ChainSupported.Base) return 'base';
  if (chain === ChainSupported.Optimism) return 'optimism';
  if (chain === ChainSupported.Arbitrum) return 'arbitrum';
  if (chain === ChainSupported.Polygon) return 'polygon';
  return 'unknown';
}


export function reconstructSignedTransaction(unsignedBytes: Uint8Array, signatureBytes: Uint8Array): `0x${string}` {
  const unsignedHex = bytesToHex(unsignedBytes);
  const unsignedTx = parseTransaction(unsignedHex);

  const signatureHex = bytesToHex(signatureBytes);
  const signature = parseSignature(signatureHex);

  // viem computes the correct v / y-parity when serializing with a parsed signature
  return serializeTransaction(unsignedTx as any, signature);
}


export const hostNetworking = {
  async submitTx(tx: TxStateMachine): Promise<Uint8Array> {
    console.log('Submitting transaction...');
    try {
      const family = getChainFamily(tx.senderAddressNetwork);
      if (family === 'unknown') {
        throw new Error(`Unsupported chain: ${tx.senderAddressNetwork}`);
      }
      
      // Ensure we have signing artifacts
      if (!tx.signedCallPayload) throw new Error("No signed call payload found - transaction must be signed first");
      if (!tx.callPayload) throw new Error("No call payload found - transaction must be created first");

      if (family === 'ethereum') {
        // Local Anvil: send directly via viem
        if (USE_ANVIL) {
          const chainConfig = CHAIN_CONFIGS[tx.senderAddressNetwork as keyof typeof CHAIN_CONFIGS];
          const publicClient = createPublicClient({ chain: chainConfig.chain, transport: http(chainConfig.rpcUrl) });
          const serializedTxBytes = 'ethereum' in tx.callPayload! ? tx.callPayload.ethereum.callPayload[1] : null;
          if (!serializedTxBytes) throw new Error('Invalid call payload for Ethereum transaction');
          const signatureBytes = tx.signedCallPayload;
          const signedTransactionHex = reconstructSignedTransaction(new Uint8Array(serializedTxBytes), new Uint8Array(signatureBytes));
          const hash = await publicClient.sendRawTransaction({ serializedTransaction: signedTransactionHex });
          // Ensure it's mined before returning (Anvil auto-mines, but wait for safety)
          await publicClient.waitForTransactionReceipt({ hash });
          const hashBytes = hexToBytes(hash);
          return hashBytes;
        }

        // Live mode: send txStateMachine to next API route handler
        const resp = await fetch('/api/submit-tx/submit-evm', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          credentials: 'same-origin',
          body: JSON.stringify({ tx: toWire(tx) }),
        })

        if (!resp.ok) throw new Error(`API submitTx failed: ${resp.status}`);
        const data = await resp.json();
        const hashHex: string = data?.hash;
        if (!hashHex || typeof hashHex !== 'string') throw new Error('Invalid hash from API');
        return hexToBytes(hashHex as Hex);
      }

      if (family === 'bsc') {
        // Local Anvil: send directly via viem
        if (USE_ANVIL) {
          const chainConfig = CHAIN_CONFIGS[tx.senderAddressNetwork as keyof typeof CHAIN_CONFIGS];
          const publicClient = createPublicClient({ chain: bsc, transport: http(chainConfig.rpcUrl) });
          const serializedTxBytes = 'bnb' in tx.callPayload! ? tx.callPayload.bnb.callPayload[1] : null;
          if (!serializedTxBytes) throw new Error('Invalid call payload for BSC transaction');
          const signatureBytes = tx.signedCallPayload;
          const signedTransactionHex = reconstructSignedTransaction(new Uint8Array(serializedTxBytes), new Uint8Array(signatureBytes));
          const hash = await publicClient.sendRawTransaction({ serializedTransaction: signedTransactionHex });
          // Wait for receipt to ensure balance updates are visible
          await publicClient.waitForTransactionReceipt({ hash });
          const hashBytes = hexToBytes(hash);
          return hashBytes;
        }

        // Live mode: send txStateMachine to next API route handler
        const resp = await fetch('/api/submit-tx/submit-bsc', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          credentials: 'same-origin',
          body: JSON.stringify({ tx: toWire(tx) }),
        })

        if (!resp.ok) throw new Error(`API submitTx failed: ${resp.status}`);
        const data = await resp.json();
        const hashHex: string = data?.hash;
        if (!hashHex || typeof hashHex !== 'string') throw new Error('Invalid hash from API');
        return hexToBytes(hashHex as Hex);
      }
  
      if (family === 'polkadot') {
        throw new Error('Polkadot submission not implemented in host functions yet');
      }
  
      if (family === 'solana') {     
        if (USE_ANVIL) {
          const connection = new SolanaConnection("http://localhost:8899", "confirmed");
          const rawSignatureBytes = new Uint8Array(tx.signedCallPayload!);

          if (!('solana' in tx.callPayload!)) throw new Error('Invalid call payload for Solana transaction');
          const vmsg = VersionedMessage.deserialize(new Uint8Array(tx.callPayload.solana.callPayload));
          const lastValidBlockHeight = tx.callPayload.solana.latestBlockHeight;
          let vtx  = new VersionedTransaction(vmsg);
          if (rawSignatureBytes.length != 64) {
            throw new Error('ed25519 signature must be 64 bytes');
          };

          vtx.addSignature(new PublicKey(tx.senderAddress), rawSignatureBytes);

          const sig = await connection.sendRawTransaction(vtx.serialize(), {maxRetries: 10});
          const result = await connection.confirmTransaction(
            {
              signature: sig,
              blockhash: vmsg.recentBlockhash,
              lastValidBlockHeight: lastValidBlockHeight,
            },
            'finalized'
          );
          if (!result.value) {
            throw new Error('Transaction failed in confirmation');
          }

          return bs58.decode(sig);
        }

        const resp = await fetch("/api/submit-tx/submit-solana", {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          credentials: 'same-origin',
          body: JSON.stringify({ tx: toWire(tx) })
        });
        
        if (!resp.ok) throw new Error(`API submitTx failed: ${resp.status}`);
        const data = await resp.json();
        const hashHex: string = data?.hash;
        if (!hashHex || typeof hashHex !== 'string') throw new Error('Invalid hash from API');
        return bs58.decode(hashHex);
      }
  
  
      throw new Error(`Unhandled chain family: ${family}`);
      
    } catch (error) {
      console.error(`Failed to submit transaction for ${tx.senderAddressNetwork}:`, error);
      throw error;
    }
  },

  async createTx(tx: TxStateMachine): Promise<TxStateMachine> {
    
    try {
      const family = getChainFamily(tx.senderAddressNetwork);
      if (family === 'unknown') {
        throw new Error(`Unsupported chain: ${tx.senderAddressNetwork}`);
      }
      
      if (family === 'ethereum' || family === 'base' || family === 'optimism' || family === 'arbitrum' || family === 'polygon') {
        // Local Anvil: build fields on client
        if (USE_ANVIL) {
          return await createTestTxEVM(tx);
        }

        // Live mode: get prepared chain data from server, then construct tx locally
        const chainConfig = CHAIN_CONFIGS[tx.senderAddressNetwork as keyof typeof CHAIN_CONFIGS];
        const resp = await fetch(chainConfig.rpcUrl, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          credentials: 'same-origin',
          body: JSON.stringify({ tx: toWire(tx) }),
        })
        if (!resp.ok) throw new Error(`API prepareCreateTx failed: ${resp.status}`);
        const data = await resp.json();
        return fromWire(data?.prepared)
      }  

      if (family === 'bsc') {
        // Local Anvil: build fields on client
        if (USE_ANVIL) {
          return await createTestTxBSC(tx);
        }

        // Live mode: get prepared chain data from server, then construct tx locally
        const chainConfig = CHAIN_CONFIGS[tx.senderAddressNetwork as keyof typeof CHAIN_CONFIGS];
        const resp = await fetch(chainConfig.rpcUrl, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          credentials: 'same-origin',
          body: JSON.stringify({ tx: toWire(tx) }),
        })
        if (!resp.ok) throw new Error(`API prepareCreateTx failed: ${resp.status}`);
        const data = await resp.json();
        return fromWire(data?.prepared)
      }

      if (family === 'solana') {
        if (USE_ANVIL) {
          return await createTestTxSolana(tx);
        }
        // Live mode: get prepared chain data from server, then construct tx locally
        // Get latest blockhash
        const resp = await fetch("/api/create-tx/prepare-solana", {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          credentials: 'same-origin',
          body: JSON.stringify({ tx: toWire(tx) })
        });

        if (!resp.ok) throw new Error(`API prepareCreateTx failed: ${resp.status}`);
        const data = await resp.json();
        return fromWire(data?.prepared)
      }

      throw new Error(`Unhandled chain family: ${family}`);

    } catch (error) {
      console.error(`Failed to create transaction for ${tx.senderAddressNetwork}:`, error);
      throw error;
    }
  },
};

// ===== Family-specific createTx implementations =====
export async function createTestTxEVM(tx: TxStateMachine): Promise<TxStateMachine> {

  // construct an unsigned ethereum transaction
  const chainConfig = CHAIN_CONFIGS[tx.senderAddressNetwork as keyof typeof CHAIN_CONFIGS];
  const publicClient = createPublicClient({ 
    chain: chainConfig.chain, 
    transport: http(chainConfig.rpcUrl)
  });

  const sender = tx.senderAddress as Address;
  const receiver = tx.receiverAddress as Address;

  const nonce = await publicClient.getTransactionCount({ address: sender });

  // Determine if this is a native token or ERC20 token
  const isNativeToken = isNativeEthereumToken(tx.token);
  
  let transactionData: {
    to: Address;
    value: bigint;
    data: Hex;
  };

  if (isNativeToken) {
    // Native token transfer (ETH)
    transactionData = {
      to: receiver,
      value: tx.amount,
      data: '0x'
    };
  } else {
    // ERC20 token transfer
    const tokenInfo = await getTokenAddress(tx.token, tx.senderAddressNetwork);
    if (!tokenInfo) {
      throw new Error(`Invalid ERC20 token address for ${JSON.stringify(tx.token)}`);
    }
    const {address: tokenAddress, decimal} = tokenInfo;

   
    const data = encodeFunctionData({
      abi: erc20Abi,
      functionName: 'transfer',
      args: [receiver, tx.amount]
    });

    transactionData = {
      to: tokenAddress as Address,
      value: 0n, // No ETH value for ERC20 transfers
      data
    };
  }

  const gas = await publicClient.estimateGas({ 
    account: sender, 
    to: transactionData.to, 
    value: transactionData.value,
    data: transactionData.data
  });

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

  let feesInEth = formatEther(gas * maxFeePerGas);

  const fields: UnsignedEip1559 = {
    to: transactionData.to,
    value: transactionData.value,
    chainId: chainConfig.chainId,
    nonce,
    gas,
    maxFeePerGas,
    maxPriorityFeePerGas,
    data: transactionData.data,
    accessList: [],
    type: 'eip1559',
  };

  // This is the **EIP-2718 unsigned payload** (should start with 0x02)
  const signingPayload = serializeTransaction(fields) as Hex;
  if (!signingPayload.startsWith('0x02')) throw new Error('Expected 0x02 typed payload');

  const digest = keccak256(signingPayload) as Hex;

  const updated: TxStateMachine = {
    ...tx,
    feesAmount: Number(feesInEth),
    callPayload: {
      ethereum: {
        ethUnsignedTxFields: fields,
        callPayload: [
          // digest as bytes (32)
          Array.from(hexToBytes(digest)),
          // unsigned payload bytes (what you hashed)
          Array.from(hexToBytes(signingPayload)),
        ]
      }
    }
  };

  return updated;
}

// ===== BSC-specific createTx implementation =====
export async function createTestTxBSC(tx: TxStateMachine): Promise<TxStateMachine> {
  const chainConfig = CHAIN_CONFIGS[tx.senderAddressNetwork as keyof typeof CHAIN_CONFIGS];
  const publicClient = createPublicClient({ 
    chain: bsc, 
    transport: http(chainConfig.rpcUrl)
  });

  const sender = tx.senderAddress as Address;
  const receiver = tx.receiverAddress as Address;

  const nonce = await publicClient.getTransactionCount({ address: sender });

  // Determine if this is a native token or BEP20 token
  const isNativeToken = isNativeBSCToken(tx.token);
  
  let transactionData: {
    to: Address;
    value: bigint;
    data: Hex;
  };

  if (isNativeToken) {
    // Native token transfer (BNB)
    transactionData = {
      to: receiver,
      value: tx.amount,
      data: '0x'
    };
  } else {
    // BEP20 token transfer (reuse Ethereum helper)
    const tokenInfo = await getTokenAddress(tx.token, tx.senderAddressNetwork);
    if (!tokenInfo) {
      throw new Error(`Invalid BEP20 token address for ${JSON.stringify(tx.token)}`);
    }
    const {address: tokenAddress, decimal} = tokenInfo;
   
    const data = encodeFunctionData({
      abi: erc20Abi,
      functionName: 'transfer',
      args: [receiver, tx.amount]
    });

    transactionData = {
      to: tokenAddress as Address,
      value: 0n, // No BNB value for BEP20 transfers
      data
    };
  }

  const gas = await publicClient.estimateGas({ 
    account: sender, 
    to: transactionData.to, 
    value: transactionData.value,
    data: transactionData.data
  });

  // BSC uses legacy gas pricing, not EIP-1559
  const gasPrice = await publicClient.getGasPrice();

  const feesInBNB = formatEther(gas * gasPrice);


  const fields: UnsignedLegacy = {
    to: transactionData.to,
    value: transactionData.value,
    chainId: chainConfig.chainId,
    nonce,
    gas,
    gasPrice,
    data: transactionData.data,
    type: 'legacy',
  };

  // This is the **legacy unsigned payload** (should start with 0x)
  const signingPayload = serializeTransaction(fields) as Hex;
  if (!signingPayload.startsWith('0x')) throw new Error('Expected 0x legacy payload');

  const digest = keccak256(signingPayload) as Hex;

  
  const updated: TxStateMachine = {
    ...tx,
    feesAmount: Number(feesInBNB),
    callPayload: {
      bnb: {
        bnbLegacyTxFields: fields,
        callPayload: [
          // digest as bytes (32)
          Array.from(hexToBytes(digest)),
          // unsigned payload bytes (what you hashed)
          Array.from(hexToBytes(signingPayload)),
        ]
      }
    }
  };

  return updated;
}

// ===== Solana-specific createTx implementation =====
export async function createTestTxSolana(tx: TxStateMachine): Promise<TxStateMachine> {
  const connection = new SolanaConnection("http://localhost:8899", "confirmed");
  const { blockhash, lastValidBlockHeight } = await connection.getLatestBlockhash('finalized');
  // Determine if this is native SOL or SPL token
  const isNativeSol = isNativeSolToken(tx.token);
  
  if (isNativeSol) {
    const from = new PublicKey(tx.senderAddress);
    const to   = new PublicKey(tx.receiverAddress);
  
    // Build the transfer instruction
    const lamports = tx.amount;
    
    // Safety check for lamports precision
    if (lamports > BigInt(Number.MAX_SAFE_INTEGER)) {
      throw new Error('lamports exceed JS safe integer range');
    }
  
    const transferIx = SystemProgram.transfer({
      fromPubkey: from,
      toPubkey: to,
      lamports: Number(lamports),
    });
  
    // Compile to a v0 message
    const msgV0 = new TransactionMessage({
      payerKey: from,
      recentBlockhash: blockhash,
      instructions: [transferIx],
    }).compileToV0Message();
  
    const unsigned = new VersionedTransaction(msgV0);
    const messageBytes = unsigned.message.serialize();

    const fee = await connection.getFeeForMessage(unsigned.message, 'confirmed');
    const feesInSol = fee?.value ? fee.value / LAMPORTS_PER_SOL : 0;
    
    const updated: TxStateMachine = {
      ...tx,
      feesAmount: Number(feesInSol),
      callPayload: {
        solana: {
          callPayload: Array.from(messageBytes),
          latestBlockHeight: lastValidBlockHeight,
        }
      },
    };
  
    return updated;

  } else {
    // SPL token transfer
    const tokenInfo = await getTokenAddress(tx.token, tx.senderAddressNetwork);
    if (!tokenInfo) {
      throw new Error(`Invalid SPL token address for ${JSON.stringify(tx.token)}`);
    }
    const {address: tokenAddress, decimal} = tokenInfo;

    const mint = new PublicKey(tokenAddress as string);
    const info = await connection.getAccountInfo(mint);
    if (!info) throw new Error('Mint not found');
    const programId = info.owner.equals(TOKEN_2022_PROGRAM_ID) ? TOKEN_2022_PROGRAM_ID : TOKEN_PROGRAM_ID;
    
    const fromAta = await getAssociatedTokenAddress(mint, new PublicKey(tx.senderAddress), false, programId);
    const toAta = await getAssociatedTokenAddress(mint, new PublicKey(tx.receiverAddress), false, programId);
    const ixs = [];
    const toInfo = await connection.getAccountInfo(toAta);

    if (!toInfo) {
      ixs.push(
        createAssociatedTokenAccountInstruction(
          new PublicKey(tx.senderAddress),
          toAta,
          new PublicKey(tx.receiverAddress),
          mint,
          programId
        )
      );
    }


    ixs.push(
      createTransferCheckedInstruction(
        fromAta, new PublicKey(tokenAddress), toAta, new PublicKey(tx.senderAddress), tx.amount, decimal, [], programId
      )
    );
   
    const msg = new TransactionMessage({
      payerKey: new PublicKey(tx.senderAddress),
      recentBlockhash: blockhash,
      instructions: ixs,
    }).compileToV0Message();
    
    const unsigned = new VersionedTransaction(msg);
    
    const messageBytes = unsigned.message.serialize();

    const fee = await connection.getFeeForMessage(unsigned.message, 'confirmed');
    const feesInSol = fee?.value ? fee.value / LAMPORTS_PER_SOL : 0;
    
    const updated: TxStateMachine = {
      ...tx,
      feesAmount: Number(feesInSol),
      callPayload: {
        solana: {
          callPayload: Array.from(messageBytes),
          latestBlockHeight: lastValidBlockHeight,
        }
      },
    };
    return updated;
  }
}



// Helper function to determine if a token is a native Ethereum token
function isNativeEthereumToken(token: Token): boolean {
  if ('Ethereum' in token) {
    return token.Ethereum === 'ETH';
  }
  return false;
}

function isNativeBSCToken(token: Token): boolean {
  if ('Bnb' in token) {
    return token.Bnb === 'BNB';
  }
  return false;
}

// Helper function to get token contract address and validate it's a valid ERC20 contract
async function getTokenAddress(token: Token, network: ChainSupported): Promise<{address:string, decimal:number} | null> {
  // Support EVM-compatible networks: Ethereum & BSC (no on-chain validation)
  const isEthereum = network === ChainSupported.Ethereum;
  const isBsc = network === ChainSupported.Bnb;
  if (!isEthereum && !isBsc) return null;
  if ('Ethereum' in token && typeof token.Ethereum === 'object' && 'ERC20' in token.Ethereum) {
    return {
      address: token.Ethereum.ERC20.address,
      decimal: token.Ethereum.ERC20.decimals
    };
  }

  if ('Bnb' in token && typeof token.Bnb === 'object' && 'BEP20' in token.Bnb) {
    return {
      address:token.Bnb.BEP20.address,
      decimal:token.Bnb.BEP20.decimals
    };
  }

  if ('Solana' in token && typeof token.Solana === 'object' && 'SPL' in token.Solana) {
    return {
      address: token.Solana.SPL.address,
      decimal: token.Solana.SPL.decimals
    };
  }

  return null;
}



// Helper function to determine if a token is native SOL
function isNativeSolToken(token: Token): boolean {
  if ('Solana' in token) {
    return token.Solana === 'SOL';
  }
  return false;
}


// helper function to ensure TxStateMachine array types are converted back to correct types
export function toWire(tx: TxStateMachine): any {
  return {
    ...tx,
    amount: tx.amount.toString(),
    // Convert Uint8Array fields to regular arrays
    multiId: tx.multiId ? Array.from(tx.multiId) : null,
    recvSignature: tx.recvSignature ? Array.from(tx.recvSignature) : null,
    signedCallPayload: tx.signedCallPayload ? Array.from(tx.signedCallPayload) : null,
    // Convert callPayload arrays with type safety
    callPayload: tx.callPayload ? (() => {
      if ('ethereum' in tx.callPayload) {
        const f = tx.callPayload.ethereum.ethUnsignedTxFields;
        return {
          ethereum: {
            ethUnsignedTxFields: {
              ...f,
              // Convert all bigints to strings for JSON serialization
              value: f.value.toString(),
              gas: f.gas.toString(),
              maxFeePerGas: f.maxFeePerGas.toString(),
              maxPriorityFeePerGas: f.maxPriorityFeePerGas.toString(),
            },
            callPayload: tx.callPayload.ethereum.callPayload ? [
              Array.from(tx.callPayload.ethereum.callPayload[0]),
              Array.from(tx.callPayload.ethereum.callPayload[1])
            ] : null
          }
        };
      } else if ('bnb' in tx.callPayload) {
        const f = tx.callPayload.bnb.bnbLegacyTxFields;
        return {
          bnb: {
            bnbLegacyTxFields: {
              ...f,
              // Convert all bigints to strings for JSON serialization
              value: f.value.toString(),
              gas: f.gas.toString(),
              gasPrice: f.gasPrice.toString(),
            },
            callPayload: tx.callPayload.bnb.callPayload ? [
              Array.from(tx.callPayload.bnb.callPayload[0]),
              Array.from(tx.callPayload.bnb.callPayload[1])
            ] : null
          }
        };
      } else if ('solana' in tx.callPayload) {
        return {
          solana: {
            ...tx.callPayload.solana,
            callPayload: tx.callPayload.solana.callPayload ? Array.from(tx.callPayload.solana.callPayload) : null
          }
        };
      }
      return tx.callPayload;
    })() : null
  };
}

export function fromWire(wireTx: any): TxStateMachine {
  return {
    ...wireTx,
    amount: BigInt(wireTx.amount),
    // Convert arrays back to Uint8Array fields
    multiId: wireTx.multiId ? new Uint8Array(wireTx.multiId) : null,
    recvSignature: wireTx.recvSignature ? new Uint8Array(wireTx.recvSignature) : null,
    signedCallPayload: wireTx.signedCallPayload ? new Uint8Array(wireTx.signedCallPayload) : null,
    // Convert callPayload arrays back with type safety
    callPayload: wireTx.callPayload ? (() => {
      if ('ethereum' in wireTx.callPayload) {
        const f = wireTx.callPayload.ethereum.ethUnsignedTxFields;
        return {
          ethereum: {
            ethUnsignedTxFields: {
              ...f,
              // Convert strings back to bigints
              value: BigInt(f.value),
              gas: BigInt(f.gas),
              maxFeePerGas: BigInt(f.maxFeePerGas),
              maxPriorityFeePerGas: BigInt(f.maxPriorityFeePerGas),
            },
            callPayload: wireTx.callPayload.ethereum.callPayload ? [
              new Uint8Array(wireTx.callPayload.ethereum.callPayload[0]),
              new Uint8Array(wireTx.callPayload.ethereum.callPayload[1])
            ] : null
          }
        };
      } else if ('bnb' in wireTx.callPayload) {
        const f = wireTx.callPayload.bnb.bnbLegacyTxFields;
        return {
          bnb: {
            bnbLegacyTxFields: {
              ...f,
              // Convert strings back to bigints
              value: BigInt(f.value),
              gas: BigInt(f.gas),
              gasPrice: BigInt(f.gasPrice),
            },
            callPayload: wireTx.callPayload.bnb.callPayload ? [
              new Uint8Array(wireTx.callPayload.bnb.callPayload[0]),
              new Uint8Array(wireTx.callPayload.bnb.callPayload[1])
            ] : null
          }
        };
      } else if ('solana' in wireTx.callPayload) {
        return {
          solana: {
            ...wireTx.callPayload.solana,
            callPayload: wireTx.callPayload.solana.callPayload ? new Uint8Array(wireTx.callPayload.solana.callPayload) : null
          }
        };
      }
      return wireTx.callPayload;
    })() : null
  } as TxStateMachine;
}