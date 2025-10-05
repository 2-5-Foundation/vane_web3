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
  recoverAddress,
  encodeFunctionData,
  getContract
} from 'viem';

import type { TransactionSerializedEIP1559 } from 'viem';

import { 
  mainnet
} from 'viem/chains';
import { ChainSupported, type TxStateMachine, type Token } from '../../primitives';

// ERC20 ABI for token transfers and validation
const ERC20_ABI = [
  {
    name: 'transfer',
    type: 'function',
    stateMutability: 'nonpayable',
    inputs: [
      { name: 'to', type: 'address' },
      { name: 'amount', type: 'uint256' }
    ],
    outputs: [{ name: '', type: 'bool' }]
  },
  {
    name: 'balanceOf',
    type: 'function',
    stateMutability: 'view',
    inputs: [
      { name: 'account', type: 'address' }
    ],
    outputs: [{ name: '', type: 'uint256' }]
  },
  {
    name: 'totalSupply',
    type: 'function',
    stateMutability: 'view',
    inputs: [],
    outputs: [{ name: '', type: 'uint256' }]
  }
] as const;

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

// In live mode, route RPC calls through Next.js API so the server component holds the API key.
// We use an absolute path from the app root: "/api/transaction".
// In local Anvil mode, we use the local JSON-RPC endpoint.
const pickRpc = (nextApiRoute: string): string => {
  if (USE_ANVIL) return 'http://127.0.0.1:8545';
  // Ensure it starts with a leading slash for Next.js API routes
  return nextApiRoute.startsWith('/') ? nextApiRoute : `/${nextApiRoute}`;
};

const CHAIN_CONFIGS: Record<ChainSupported.Ethereum, {
  chain: Chain;
  rpcUrl: string;
  chainId: number;
}> = {
  [ChainSupported.Ethereum]: {
    chain: mainnet,
    // Proxy via Next.js API route in live mode. Example Next route: /api/transactions
    // Your Next.js server should proxy JSON-RPC to your provider using server-side API keys.
    rpcUrl: pickRpc('/api/prepare-evm'),
    chainId: USE_ANVIL ? 31337 : 1
  }
};

function getChainFamily(chain: ChainSupported): 'ethereum' | 'unknown' {
  if (chain === ChainSupported.Ethereum) return 'ethereum';
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
        const chainConfig = CHAIN_CONFIGS[tx.senderAddressNetwork as keyof typeof CHAIN_CONFIGS];
        // Local Anvil: send directly via viem
        if (USE_ANVIL) {
          const publicClient = createPublicClient({ chain: mainnet, transport: http(chainConfig.rpcUrl) });
          const [_, serializedTxBytes] = tx.callPayload;
          const signatureBytes = tx.signedCallPayload;
          const signedTransactionHex = reconstructSignedTransaction(serializedTxBytes, signatureBytes);
          const hash = await publicClient.sendRawTransaction({ serializedTransaction: signedTransactionHex });
          const hashBytes = new Uint8Array(hash.slice(2).match(/.{1,2}/g)!.map((byte: string) => parseInt(byte, 16)));
          return hashBytes;
        }

        // Live mode: proxy to Next.js API route; server holds API keys and public client
        const resp = await fetch('/api/tx/submit-evm', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          credentials: 'same-origin',
          body: JSON.stringify({ tx: { ...tx, amount: tx.amount.toString() } }),
        })

        if (!resp.ok) throw new Error(`Proxy submitTx failed: ${resp.status}`);
        const data = await resp.json();
        const hashHex: string = data?.hash;
        if (!hashHex || typeof hashHex !== 'string') throw new Error('Invalid hash from proxy');
        return new Uint8Array(hashHex.slice(2).match(/.{1,2}/g)!.map((b: string) => parseInt(b, 16)));
      }
  
      if (family === 'polkadot') {
        throw new Error('Polkadot submission not implemented in host functions yet');
      }
  
      if (family === 'solana') {
        throw new Error('Solana submission not implemented in host functions yet');
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
        if (USE_ANVIL) return await createTxEthereum(tx);

        // Live mode: ask server for prepared chain data (nonce, gas, fees, tokenAddress)
        const chainConfig = CHAIN_CONFIGS[tx.senderAddressNetwork as keyof typeof CHAIN_CONFIGS];
        const resp = await fetch(chainConfig.rpcUrl, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          credentials: 'same-origin',
          body: JSON.stringify({ tx: { ...tx, amount: tx.amount.toString() } }),
        })
        if (!resp.ok) throw new Error(`Proxy prepareCreateTxEthereum failed: ${resp.status}`);
        const data = await resp.json();
        const prepared = data?.prepared as PreparedEthParams | undefined;
        if (!prepared) throw new Error('Invalid prepared params from proxy');
        return await createTxEthereumWithParams(tx, prepared);
      }

      throw new Error(`Unhandled chain family: ${family}`);

    } catch (error) {
      console.error(`Failed to create transaction for ${tx.senderAddressNetwork}:`, error);
      throw error;
    }
  },
};

// ===== Family-specific createTx implementations =====
export async function createTxEthereum(tx: TxStateMachine): Promise<TxStateMachine> {
  const chainConfig = CHAIN_CONFIGS[tx.senderAddressNetwork as keyof typeof CHAIN_CONFIGS];
  const publicClient = createPublicClient({ 
    chain: mainnet, 
    transport: http(chainConfig.rpcUrl)
  });

  const sender = tx.senderAddress as Address;
  const receiver = tx.receiverAddress as Address;
  const amount = BigInt(tx.amount);

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
    const tokenAddress = await getTokenAddress(tx.token, tx.senderAddressNetwork, publicClient);
    if (!tokenAddress) {
      throw new Error(`Invalid ERC20 token address for ${JSON.stringify(tx.token)}`);
    }

    // Convert amount to token's smallest unit (most ERC20 tokens use 18 decimals)
    const tokenAmount = amount * BigInt(10 ** 18);

    const data = encodeFunctionData({
      abi: ERC20_ABI,
      functionName: 'transfer',
      args: [receiver, tokenAmount]
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

// ===== Helper for live mode: construct tx using server-prepared chain data =====
type PreparedEthParams = {
  nonce: number;
  gas: bigint;
  maxFeePerGas: bigint;
  maxPriorityFeePerGas: bigint;
  tokenAddress?: string | null; // required if ERC20
};

async function createTxEthereumWithParams(tx: TxStateMachine, params: PreparedEthParams): Promise<TxStateMachine> {
  const chainConfig = CHAIN_CONFIGS[tx.senderAddressNetwork as keyof typeof CHAIN_CONFIGS];

  const sender = tx.senderAddress as Address;
  const receiver = tx.receiverAddress as Address;
  const amount = BigInt(tx.amount);

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
    // Assume 18 decimals for now; server could pre-scale if needed
    const tokenAmount = amount * BigInt(10 ** 18);
    const data = encodeFunctionData({
      abi: ERC20_ABI,
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
    callPayload: [
      new Uint8Array(digest.slice(2).match(/.{1,2}/g)!.map((b) => parseInt(b, 16))),
      new Uint8Array(signingPayload.slice(2).match(/.{1,2}/g)!.map((b) => parseInt(b, 16))),
    ],
    ethUnsignedTxFields: fields
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

// Helper function to get token contract address and validate it's a valid ERC20 contract
async function getTokenAddress(token: Token, network: ChainSupported, publicClient: any): Promise<string | null> {
  // Only support Ethereum mainnet ERC20 tokens
  if (network !== ChainSupported.Ethereum) {
    return null;
  }

  // Extract token address from Ethereum token object
  if ('Ethereum' in token && typeof token.Ethereum === 'object' && 'ERC20' in token.Ethereum) {
    const tokenAddress = token.Ethereum.ERC20.address;
    
    // Validate that the address is a valid ERC20 contract
    const isValidERC20 = await validateERC20Contract(tokenAddress as Address, publicClient);
    if (isValidERC20) {
      return tokenAddress;
    }
  }

  return null;
}

// Helper function to validate if an address is a valid ERC20 contract
async function validateERC20Contract(tokenAddress: Address, publicClient: any): Promise<boolean> {
  try {
    // Ensure there is bytecode at the address (indicates a contract exists)
    const code = await publicClient.getCode({ address: tokenAddress });
    return !!code;
  } catch (error) {
    console.warn(`Token address ${tokenAddress} failed ERC20 validation:`, error);
    return false;
  }
}
