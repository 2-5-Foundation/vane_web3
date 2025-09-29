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

const pickRpc = (liveUrl: string): string => USE_ANVIL ? 'http://127.0.0.1:8545' : liveUrl;

const CHAIN_CONFIGS: Record<ChainSupported.Ethereum, {
  chain: Chain;
  rpcUrl: string;
  chainId: number;
}> = {
  [ChainSupported.Ethereum]: {
    chain: mainnet,
    rpcUrl: pickRpc('https://eth-mainnet.g.alchemy.com/v2/demo'),
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
      
      if (!tx.signedCallPayload) {
        throw new Error("No signed call payload found - transaction must be signed first");
      }
      
      if (!tx.callPayload) {
        throw new Error("No call payload found - transaction must be created first");
      }
      
      if (family === 'ethereum') {
        console.log('Submitting Ethereum transaction...');
        const chainConfig = CHAIN_CONFIGS[tx.senderAddressNetwork as keyof typeof CHAIN_CONFIGS];
        const publicClient = createPublicClient({
          chain: mainnet,
          transport: http(chainConfig.rpcUrl)
        });
      
        const senderAddress = tx.senderAddress as Address;
        const [signatureHashBytes, serializedTxBytes] = tx.callPayload;
        const signatureBytes = tx.signedCallPayload;
           
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
        return await createTxEthereum(tx);
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
    const tokenAddress = token.Ethereum.ERC20;
    
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
