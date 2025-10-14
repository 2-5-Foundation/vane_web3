import { 
  createPublicClient, 
  http, 
  parseEther, 
  parseUnits,
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
  parseSignature,
  parseTransaction,
  recoverAddress,
  encodeFunctionData,
  getContract,
  erc20Abi,
  bytesToHex
} from 'viem';

import type { TransactionSerializedEIP1559 } from 'viem';

import { 
  mainnet, bsc
} from 'viem/chains';
import {
  Connection as SolanaConnection,
  Transaction as SolanaTransaction,
  SystemProgram,
  LAMPORTS_PER_SOL,
  Message,
  PublicKey,
  VersionedTransaction,
  TransactionMessage,
  VersionedMessage,
} from "@solana/web3.js";
import {
  getAssociatedTokenAddress, createTransferCheckedInstruction, transfer,
  createAssociatedTokenAccountInstruction,getMint,
  TOKEN_PROGRAM_ID, TOKEN_2022_PROGRAM_ID
} from '@solana/spl-token';

import { ChainSupported, type TxStateMachine, type Token } from '../../primitives';


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

type UnsignedLegacy = {
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
    rpcUrl: pickRpc('/api/prepare-evm', ChainSupported.Ethereum),
    chainId: USE_ANVIL ? 31337 : 1
  },
  [ChainSupported.Bnb]: {
    chain: bsc,
    // Proxy via Next.js API route in live mode to get prepared chain data
    rpcUrl: pickRpc('/api/prepare-bsc', ChainSupported.Bnb),
    chainId: USE_ANVIL ? 56 : 56
  }
};

function getChainFamily(chain: ChainSupported): 'ethereum' | 'bsc' | 'solana' | 'polkadot' | 'unknown' {
  if (chain === ChainSupported.Ethereum) return 'ethereum';
  if (chain === ChainSupported.Bnb) return 'bsc';
  if (chain === ChainSupported.Solana) return 'solana';
  if (chain === ChainSupported.Polkadot) return 'polkadot';
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
          const publicClient = createPublicClient({ chain: mainnet, transport: http(chainConfig.rpcUrl) });
          const serializedTxBytes = 'ethereum' in tx.callPayload! ? tx.callPayload.ethereum.callPayload[1] : null;
          if (!serializedTxBytes) throw new Error('Invalid call payload for Ethereum transaction');
          const signatureBytes = tx.signedCallPayload;
          const signedTransactionHex = reconstructSignedTransaction(serializedTxBytes, signatureBytes);
          const hash = await publicClient.sendRawTransaction({ serializedTransaction: signedTransactionHex });
          const hashBytes = new Uint8Array(hash.slice(2).match(/.{1,2}/g)!.map((byte: string) => parseInt(byte, 16)));
          return hashBytes;
        }

        // Live mode: send txStateMachine to next API route handler
        const resp = await fetch('/api/tx/submit-evm', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          credentials: 'same-origin',
          body: JSON.stringify({ tx: { ...tx, amount: tx.amount.toString() } }),
        })

        if (!resp.ok) throw new Error(`API submitTx failed: ${resp.status}`);
        const data = await resp.json();
        const hashHex: string = data?.hash;
        if (!hashHex || typeof hashHex !== 'string') throw new Error('Invalid hash from API');
        return new Uint8Array(hashHex.slice(2).match(/.{1,2}/g)!.map((b: string) => parseInt(b, 16)));
      }

      if (family === 'bsc') {
        // Local Anvil: send directly via viem
        if (USE_ANVIL) {
          const chainConfig = CHAIN_CONFIGS[tx.senderAddressNetwork as keyof typeof CHAIN_CONFIGS];
          const publicClient = createPublicClient({ chain: bsc, transport: http(chainConfig.rpcUrl) });
          const serializedTxBytes = 'bnb' in tx.callPayload! ? tx.callPayload.bnb.callPayload[1] : null;
          if (!serializedTxBytes) throw new Error('Invalid call payload for Ethereum transaction');
          const signatureBytes = tx.signedCallPayload;
          const signedTransactionHex = reconstructSignedTransaction(serializedTxBytes, signatureBytes);
          const hash = await publicClient.sendRawTransaction({ serializedTransaction: signedTransactionHex });
          const hashBytes = new Uint8Array(hash.slice(2).match(/.{1,2}/g)!.map((byte: string) => parseInt(byte, 16)));
          return hashBytes;
        }

        // Live mode: send txStateMachine to next API route handler
        const resp = await fetch('/api/tx/submit-bsc', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          credentials: 'same-origin',
          body: JSON.stringify({ tx: { ...tx, amount: tx.amount.toString() } }),
        })

        if (!resp.ok) throw new Error(`API submitTx failed: ${resp.status}`);
        const data = await resp.json();
        const hashHex: string = data?.hash;
        if (!hashHex || typeof hashHex !== 'string') throw new Error('Invalid hash from API');
        return new Uint8Array(hashHex.slice(2).match(/.{1,2}/g)!.map((b: string) => parseInt(b, 16)));
      }
  
      if (family === 'polkadot') {
        throw new Error('Polkadot submission not implemented in host functions yet');
      }
  
      if (family === 'solana') {     
        if (USE_ANVIL) {
          const connection = new SolanaConnection("http://localhost:8899", "confirmed");
          const rawSignatureBytes = new Uint8Array(tx.signedCallPayload!);

          if (!('solana' in tx.callPayload!)) throw new Error('Invalid call payload for Solana transaction');
          const vmsg = VersionedMessage.deserialize(tx.callPayload.solana.callPayload);
          const lastValidBlockHeight = tx.callPayload.solana.latestBlockHeight;
          let vtx  = new VersionedTransaction(vmsg);
          if (rawSignatureBytes.length != 64) {
            throw new Error('ed25519 signature must be 64 bytes');
          };

          vtx.addSignature(new PublicKey(tx.senderAddress), rawSignatureBytes);

          const sig = await connection.sendRawTransaction(vtx.serialize());
          const result = await connection.confirmTransaction(
            {
              signature: sig,
              blockhash: vmsg.recentBlockhash,
              lastValidBlockHeight: lastValidBlockHeight,
            },
            'confirmed'
          );
          if (!result.value) {
            throw new Error('Transaction failed in confirmation');
          }

          return new Uint8Array(sig.slice(2).match(/.{1,2}/g)!.map((byte: string) => parseInt(byte, 16)));
        }

        const resp = await fetch("api/tx/submit-solana", {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          credentials: 'same-origin',
          body: JSON.stringify({ tx: { ...tx, amount: tx.amount.toString() } })
        });
        
        if (!resp.ok) throw new Error(`API submitTx failed: ${resp.status}`);
        const data = await resp.json();
        const hashHex: string = data?.hash;
        if (!hashHex || typeof hashHex !== 'string') throw new Error('Invalid hash from API');
        return new Uint8Array(hashHex.slice(2).match(/.{1,2}/g)!.map((b: string) => parseInt(b, 16)));
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
      
      if (family === 'ethereum') {
        // Local Anvil: build fields on client
        if (USE_ANVIL) {
          return await createTestTxEthereum(tx);
        }

        // Live mode: get prepared chain data from server, then construct tx locally
        const chainConfig = CHAIN_CONFIGS[tx.senderAddressNetwork as keyof typeof CHAIN_CONFIGS];
        const resp = await fetch(chainConfig.rpcUrl, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          credentials: 'same-origin',
          body: JSON.stringify({ tx: { ...tx, amount: tx.amount.toString() } }),
        })
        if (!resp.ok) throw new Error(`API prepareCreateTx failed: ${resp.status}`);
        const data = await resp.json();
        const prepared = data?.prepared as PreparedEthParams | undefined;
        if (!prepared) throw new Error('Invalid prepared params from API');
        return await createTxEthereumWithParams(tx, prepared);
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
          body: JSON.stringify({ tx: { ...tx, amount: tx.amount.toString() } }),
        })
        if (!resp.ok) throw new Error(`API prepareCreateTx failed: ${resp.status}`);
        const data = await resp.json();
        const prepared = data?.prepared as PreparedBSCParams | undefined;
        if (!prepared) throw new Error('Invalid prepared params from API');
        return await createTxBSCWithParams(tx, prepared);
      }

      if (family === 'solana') {
        if (USE_ANVIL) {
          return await createTestTxSolana(tx);
        }
        // Live mode: get prepared chain data from server, then construct tx locally
        // Get latest blockhash
        const resp = await fetch("api/tx/prepare-solana", {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          credentials: 'same-origin',
          body: JSON.stringify({ tx: { ...tx, amount: tx.amount.toString() } })
        });

        if (!resp.ok) throw new Error(`API prepareCreateTx failed: ${resp.status}`);
        const data = await resp.json();
        const { blockhash, lastValidBlockHeight } = data?.prepared as { blockhash: string, lastValidBlockHeight: number };
        return await createTxSolanaWithParams(tx, { blockhash, lastValidBlockHeight });
      }

      throw new Error(`Unhandled chain family: ${family}`);

    } catch (error) {
      console.error(`Failed to create transaction for ${tx.senderAddressNetwork}:`, error);
      throw error;
    }
  },
};

// ===== Family-specific createTx implementations =====
export async function createTestTxEthereum(tx: TxStateMachine): Promise<TxStateMachine> {
  const chainConfig = CHAIN_CONFIGS[tx.senderAddressNetwork as keyof typeof CHAIN_CONFIGS];
  const publicClient = createPublicClient({ 
    chain: mainnet, 
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
    const value = parseEther(tx.amount.toString());
    transactionData = {
      to: receiver,
      value,
      data: '0x'
    };
  } else {
    // ERC20 token transfer
    const tokenAddress = await getTokenAddress(tx.token, tx.senderAddressNetwork);
    if (!tokenAddress) {
      throw new Error(`Invalid ERC20 token address for ${JSON.stringify(tx.token)}`);
    }

    const decimals = await publicClient.readContract({
      address: tokenAddress as `0x${string}`,
      abi: erc20Abi,
      functionName: 'decimals',
    });
  
    const value = parseUnits(String(tx.amount), decimals);

    const data = encodeFunctionData({
      abi: erc20Abi,
      functionName: 'transfer',
      args: [receiver, value]
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
    callPayload: {
      ethereum: {
        ethUnsignedTxFields: fields,
        callPayload: [
          // digest as bytes (32)
          new Uint8Array(digest.slice(2).match(/.{1,2}/g)!.map((b) => parseInt(b, 16))),
          // unsigned payload bytes (what you hashed)
          new Uint8Array(signingPayload.slice(2).match(/.{1,2}/g)!.map((b) => parseInt(b, 16))),
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
  const amount = BigInt(tx.amount);

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
    const value = parseEther(tx.amount.toString());
    transactionData = {
      to: receiver,
      value,
      data: '0x'
    };
  } else {
    // BEP20 token transfer (reuse Ethereum helper)
    const tokenAddress = await getTokenAddress(tx.token, tx.senderAddressNetwork);
    if (!tokenAddress) {
      throw new Error(`Invalid BEP20 token address for ${JSON.stringify(tx.token)}`);
    }

    const decimals = await publicClient.readContract({
      address: tokenAddress as `0x${string}`,
      abi: erc20Abi,
      functionName: 'decimals',
    });
  
    const value = parseUnits(String(tx.amount), decimals);
   
    const data = encodeFunctionData({
      abi: erc20Abi,
      functionName: 'transfer',
      args: [receiver, value]
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

  // Convert legacy fields to EIP-1559 format for storage compatibility
  const eip1559Fields: UnsignedEip1559 = {
    to: fields.to,
    value: fields.value,
    chainId: fields.chainId,
    nonce: fields.nonce,
    gas: fields.gas,
    maxFeePerGas: fields.gasPrice, // Use gasPrice as maxFeePerGas for BSC
    maxPriorityFeePerGas: fields.gasPrice, // Use gasPrice as maxPriorityFeePerGas for BSC
    data: fields.data,
    accessList: [],
    type: 'eip1559',
  };

  const updated: TxStateMachine = {
    ...tx,
    callPayload: {
      bnb: {
        bnbLegacyTxFields: eip1559Fields,
        callPayload: [
          // digest as bytes (32)
          new Uint8Array(digest.slice(2).match(/.{1,2}/g)!.map((b) => parseInt(b, 16))),
          // unsigned payload bytes (what you hashed)
          new Uint8Array(signingPayload.slice(2).match(/.{1,2}/g)!.map((b) => parseInt(b, 16))),
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
    const lamports =
      typeof tx.amount === 'bigint'
        ? tx.amount * BigInt(LAMPORTS_PER_SOL)
        : Math.round(Number(tx.amount) * LAMPORTS_PER_SOL);
  
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
    
    const updated: TxStateMachine = {
      ...tx,
      callPayload: {
        solana: {
          callPayload: messageBytes as Uint8Array,
          latestBlockHeight: lastValidBlockHeight,
        }
      },
    };
  
    return updated;

  } else {
    // SPL token transfer
    const tokenAddress  = await getSPLTokenAddress(tx.token, tx.senderAddressNetwork);
    if (!tokenAddress) {
      throw new Error(`Invalid SPL token address for ${JSON.stringify(tx.token)}`);
    }
    const mint = new PublicKey(tokenAddress);
    const connection = new SolanaConnection("http://localhost:8899", 'confirmed');
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

    const minInfo = await getMint(connection, mint, 'confirmed',programId);
    const decimals = minInfo.decimals;
    
    ixs.push(
      createTransferCheckedInstruction(
        fromAta, mint, toAta, new PublicKey(tx.senderAddress), tx.amount * (10n ** BigInt(decimals)), decimals, [], programId
      )
    );
   
    const msg = new TransactionMessage({
      payerKey: new PublicKey(tx.senderAddress),
      recentBlockhash: blockhash,
      instructions: ixs,
    }).compileToV0Message();
    
    const unsigned = new VersionedTransaction(msg);
    
    const messageBytes = unsigned.message.serialize();
    
    const updated: TxStateMachine = {
      ...tx,
      callPayload: {
        solana: {
          callPayload: new Uint8Array(messageBytes),
          latestBlockHeight: lastValidBlockHeight,
        }
      },
    };
    return updated;
  }
}

// ===== Helper for live mode: construct tx using server-prepared chain data =====
export type PreparedEthParams = {
  nonce: number;
  gas: bigint;
  maxFeePerGas: bigint;
  maxPriorityFeePerGas: bigint;
  tokenAddress?: string | null; // required if ERC20
  tokenDecimals?: number | null
};

export type PreparedBSCParams = {
  nonce: number;
  gas: bigint;
  gasPrice: bigint;
  tokenAddress?: string | null; // required if BEP20
  tokenDecimals?: number | null; // required if BEP20
};

async function createTxEthereumWithParams(tx: TxStateMachine, params: PreparedEthParams): Promise<TxStateMachine> {
  const chainConfig = CHAIN_CONFIGS[tx.senderAddressNetwork as keyof typeof CHAIN_CONFIGS];

  const sender = tx.senderAddress as Address;
  const receiver = tx.receiverAddress as Address;

  // Determine if this is a native token or ERC20 token
  const isNativeToken = isNativeEthereumToken(tx.token);

  let transactionData: {
    to: Address;
    value: bigint;
    data: Hex;
  };

  if (isNativeToken) {
    const value = parseEther(tx.amount.toString());
    transactionData = { to: receiver, value, data: '0x' };
  } else {
    const tokenAddress = params.tokenAddress;
    if (!tokenAddress) throw new Error('Missing tokenAddress for ERC20 transfer');
    
    // Use decimals provided by server
    const decimals = params.tokenDecimals!
    const tokenAmount = parseUnits(tx.amount.toString(), decimals);
    
    const data = encodeFunctionData({
      abi: erc20Abi,
      functionName: 'transfer',
      args: [receiver, tokenAmount]
    });
    transactionData = { to: tokenAddress as Address, value: 0n, data };
  }

  const fields: UnsignedEip1559 = {
    to: transactionData.to,
    value: transactionData.value,
    chainId: chainConfig.chainId,
    nonce: params.nonce,
    gas: params.gas,
    maxFeePerGas: params.maxFeePerGas,
    maxPriorityFeePerGas: params.maxPriorityFeePerGas,
    data: transactionData.data,
    accessList: [],
    type: 'eip1559',
  };

  const signingPayload = serializeTransaction(fields) as Hex;
  if (!signingPayload.startsWith('0x02')) throw new Error('Expected 0x02 typed payload');
  const digest = keccak256(signingPayload) as Hex;

  const updated: TxStateMachine = {
    ...tx,
    callPayload: {
      ethereum: {
        ethUnsignedTxFields: fields,
        callPayload: [
          new Uint8Array(digest.slice(2).match(/.{1,2}/g)!.map((b) => parseInt(b, 16))),
          new Uint8Array(signingPayload.slice(2).match(/.{1,2}/g)!.map((b) => parseInt(b, 16))),
        ]
      }
    }
  };

  return updated;
}

async function createTxSolanaWithParams(tx: TxStateMachine, params: { blockhash: string, lastValidBlockHeight: number }): Promise<TxStateMachine> {
  
  let unsignedTx = new SolanaTransaction().add(
    SystemProgram.transfer({
      fromPubkey: new PublicKey(tx.senderAddress),
      toPubkey: new PublicKey(tx.receiverAddress),
      lamports: Number(tx.amount) * LAMPORTS_PER_SOL
    })
  );

  unsignedTx.recentBlockhash = params.blockhash;
  unsignedTx.feePayer = new PublicKey(tx.senderAddress);
  const bufferNeedToSign = unsignedTx.serializeMessage();
  const unsignedTxBytes = new Uint8Array(bufferNeedToSign.buffer, bufferNeedToSign.byteOffset, bufferNeedToSign.byteLength);
  const updated: TxStateMachine = {
    ...tx,
    callPayload: {
      solana: {
        callPayload: unsignedTxBytes,
        latestBlockHeight: params.lastValidBlockHeight,
      }
    },
  };
  return updated;

}

async function createTxBSCWithParams(tx: TxStateMachine, params: PreparedBSCParams): Promise<TxStateMachine> {
  const chainConfig = CHAIN_CONFIGS[tx.senderAddressNetwork as keyof typeof CHAIN_CONFIGS];

  const receiver = tx.receiverAddress as Address;

  // Determine if this is a native token or BEP20 token
  const isNativeToken = isNativeBSCToken(tx.token);

  let transactionData: {
    to: Address;
    value: bigint;
    data: Hex;
  };

  if (isNativeToken) {
    const value = parseEther(tx.amount.toString());
    transactionData = { to: receiver, value, data: '0x' };
  } else {
    const tokenAddress = params.tokenAddress;
    if (!tokenAddress) throw new Error('Missing tokenAddress for BEP20 transfer');
    
    // Use decimals provided by server
    const decimals = params.tokenDecimals!;
    const tokenAmount = parseUnits(tx.amount.toString(), decimals);
    
    const data = encodeFunctionData({
      abi: erc20Abi,
      functionName: 'transfer',
      args: [receiver, tokenAmount]
    });
    transactionData = { to: tokenAddress as Address, value: 0n, data };
  }

  const fields: UnsignedLegacy = {
    to: transactionData.to,
    value: transactionData.value,
    chainId: chainConfig.chainId,
    nonce: params.nonce,
    gas: params.gas,
    gasPrice: params.gasPrice,
    data: transactionData.data,
    type: 'legacy',
  };

  const signingPayload = serializeTransaction(fields) as Hex;
  if (!signingPayload.startsWith('0x')) throw new Error('Expected 0x legacy payload');
  const digest = keccak256(signingPayload) as Hex;

  // Convert legacy fields to EIP-1559 format for storage compatibility
  const eip1559Fields: UnsignedEip1559 = {
    to: fields.to,
    value: fields.value,
    chainId: fields.chainId,
    nonce: fields.nonce,
    gas: fields.gas,
    maxFeePerGas: fields.gasPrice, // Use gasPrice as maxFeePerGas for BSC
    maxPriorityFeePerGas: fields.gasPrice, // Use gasPrice as maxPriorityFeePerGas for BSC
    data: fields.data,
    accessList: [],
    type: 'eip1559',
  };

  const updated: TxStateMachine = {
    ...tx,
    callPayload: {
      bnb: {
        bnbLegacyTxFields: eip1559Fields,
        callPayload: [
          new Uint8Array(digest.slice(2).match(/.{1,2}/g)!.map((b) => parseInt(b, 16))),
          new Uint8Array(signingPayload.slice(2).match(/.{1,2}/g)!.map((b) => parseInt(b, 16))),
        ]
      }
    }
  };

  return updated;
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
async function getTokenAddress(token: Token, network: ChainSupported): Promise<string | null> {
  // Support EVM-compatible networks: Ethereum & BSC (no on-chain validation)
  const isEthereum = network === ChainSupported.Ethereum;
  const isBsc = network === ChainSupported.Bnb;
  if (!isEthereum && !isBsc) return null;

  if ('Ethereum' in token && typeof token.Ethereum === 'object' && 'ERC20' in token.Ethereum) {
    return token.Ethereum.ERC20.address || null;
  }

  if ('Bnb' in token && typeof token.Bnb === 'object' && 'BEP20' in token.Bnb) {
    return token.Bnb.BEP20.address || null;
  }

  return null;
}

async function getSPLTokenAddress(token: Token, network: ChainSupported): Promise<string | null> {
  // Support Solana network only
  const isSolana = network === ChainSupported.Solana;
  if (!isSolana) return null;

  if ('Solana' in token && typeof token.Solana === 'object' && 'SPL' in token.Solana) {
    return token.Solana.SPL.address || null;
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
