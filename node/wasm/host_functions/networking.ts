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
  type Transaction
} from 'viem';
import { 
  mainnet, 
  polygon, 
  bsc, 
  arbitrum 
} from 'viem/chains';
import { ChainSupported, type TxStateMachine } from './primitives';


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
    chainId: 1
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


export const hostNetworking = {
  async submitTx(tx: TxStateMachine): Promise<Uint8Array> {
    
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
        const chainConfig = CHAIN_CONFIGS[tx.network as keyof typeof CHAIN_CONFIGS];
        const publicClient = createPublicClientForChain(chainConfig.chain, chainConfig.rpcUrl);
      
      const senderAddress = tx.senderAddress as Address;
      
      const [signatureHashBytes, serializedTxBytes] = tx.callPayload;
      
      const serializedTxHex = '0x' + Array.from(serializedTxBytes as unknown as number[])
        .map((b: number) => b.toString(16).padStart(2, '0'))
        .join('');
      
      const signatureBytes = tx.signedCallPayload;
      
      const signatureHex = '0x' + Array.from(signatureBytes)
        .map(b => b.toString(16).padStart(2, '0'))
        .join('');
      
      try {
        const recoveredPublicKey = await recoverPublicKey({
          hash: '0x' + Array.from(signatureHashBytes as unknown as number[]).map((b: number) => b.toString(16).padStart(2, '0')).join('') as `0x${string}`,
          signature: signatureHex as `0x${string}`
        });
        
        const recoveredAddress = '0x' + keccak256(recoveredPublicKey.slice(1) as `0x${string}`).slice(-40);
        
        if (recoveredAddress.toLowerCase() !== senderAddress.toLowerCase()) {
          throw new Error(`Signature verification failed: recovered address ${recoveredAddress} does not match sender address ${senderAddress}`);
        }
        console.log('✅ Signature verification passed!');
        
      } catch (error) {
        throw new Error(`Invalid signature - transaction cannot be submitted: ${error}`);
      }
      
      const signedTransactionHex = serializedTxHex + Array.from(signatureBytes)
        .map(b => b.toString(16).padStart(2, '0'))
        .join('');
      
      console.log('Submitting signed transaction to blockchain...');
      const hash = await publicClient.sendRawTransaction({
        serializedTransaction: signedTransactionHex as `0x${string}`
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
    console.log("Creating transaction:", tx);
    
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
async function createTxEvm(tx: TxStateMachine): Promise<TxStateMachine> {
  const chainConfig = CHAIN_CONFIGS[tx.network as keyof typeof CHAIN_CONFIGS];
  const publicClient = createPublicClientForChain(chainConfig.chain, chainConfig.rpcUrl);

  const senderAddress = tx.senderAddress as Address;
  const receiverAddress = tx.receiverAddress as Address;
  const value = parseEther(tx.amount.toString());

  const nonce = await publicClient.getTransactionCount({ address: senderAddress });

  const gasEstimate = await publicClient.estimateGas({
    account: senderAddress,
    to: receiverAddress,
    value: value
  });

  let maxFeePerGas: bigint;
  let maxPriorityFeePerGas: bigint;
  try {
    const feeData = await publicClient.estimateFeesPerGas();
    maxFeePerGas = feeData.maxFeePerGas!;
    maxPriorityFeePerGas = feeData.maxPriorityFeePerGas!;
  } catch (error) {
    const gasPrice = await publicClient.getGasPrice();
    maxFeePerGas = gasPrice;
    maxPriorityFeePerGas = gasPrice;
  }

  let transactionRequest: TransactionRequest;
  switch (tx.network) {
    case ChainSupported.Ethereum:
    case ChainSupported.Polygon:
    case ChainSupported.Arbitrum:
    case ChainSupported.Bnb:
      transactionRequest = {
        to: receiverAddress,
        value: value,
        chainId: chainConfig.chainId,
        nonce: nonce,
        gas: gasEstimate,
        maxFeePerGas: maxFeePerGas,
        maxPriorityFeePerGas: maxPriorityFeePerGas,
        type: 'eip1559'
      } as TransactionRequest;
      break;
    default:
      throw new Error(`Unsupported EVM chain: ${tx.network}`);
  }

  const serializedTx = serializeEthTransaction({
    to: receiverAddress,
    value: value,
    chainId: chainConfig.chainId,
    nonce: nonce,
    gas: gasEstimate,
    maxFeePerGas: maxFeePerGas,
    maxPriorityFeePerGas: maxPriorityFeePerGas,
    type: 'eip1559'
  });

  const signatureHash = keccak256(serializedTx);
  const hashBytes = new Uint8Array(
    signatureHash.slice(2).match(/.{1,2}/g)!.map(byte => parseInt(byte, 16))
  );

  const serializedBytes = new Uint8Array(
    serializedTx.slice(2).match(/.{1,2}/g)!.map(byte => parseInt(byte, 16))
  );

  const updated: TxStateMachine = {
    ...tx,
    callPayload: [hashBytes, serializedBytes]
  };

  return updated;
}

async function createTxPolkadot(_tx: TxStateMachine): Promise<TxStateMachine> {
  throw new Error('Polkadot transaction creation not implemented in host functions yet');
}

async function createTxSolana(_tx: TxStateMachine): Promise<TxStateMachine> {
  throw new Error('Solana transaction creation not implemented in host functions yet');
}