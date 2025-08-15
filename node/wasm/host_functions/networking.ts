import { 
  createPublicClient, 
  http, 
  parseEther, 
  keccak256,
  serializeTransaction,
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


const CHAIN_CONFIGS: Record<ChainSupported.Ethereum | ChainSupported.Polygon | ChainSupported.Bnb | ChainSupported.Arbitrum, {
  chain: Chain;
  rpcUrl: string;
  chainId: number;
}> = {
  [ChainSupported.Ethereum]: {
    chain: mainnet,
    rpcUrl: 'https://eth-mainnet.g.alchemy.com/v2/demo',
    chainId: 1
  },
  [ChainSupported.Polygon]: {
    chain: polygon,
    rpcUrl: 'https://polygon-rpc.com', 
    chainId: 137
  },
  [ChainSupported.Bnb]: {
    chain: bsc,
    rpcUrl: 'https://bsc-dataseed.binance.org', 
    chainId: 56
  },
  [ChainSupported.Arbitrum]: {
    chain: arbitrum,
    rpcUrl: 'https://arb1.arbitrum.io/rpc', 
    chainId: 42161
  }
};

const createPublicClientForChain = (chain: Chain, rpcUrl: string): PublicClient => {
  return createPublicClient({
    chain,
    transport: http(rpcUrl)
  });
};

function isSupportedEVMChain(chain: ChainSupported): chain is ChainSupported.Ethereum | ChainSupported.Polygon | ChainSupported.Bnb | ChainSupported.Arbitrum {
  return chain === ChainSupported.Ethereum || 
         chain === ChainSupported.Polygon || 
         chain === ChainSupported.Bnb || 
         chain === ChainSupported.Arbitrum;
}


export const hostNetworking = {
  async submitTx(tx: TxStateMachine): Promise<Uint8Array> {
    
    try {
      if (!isSupportedEVMChain(tx.network)) {
        throw new Error(`Unsupported EVM chain: ${tx.network}`);
      }
      
      if (!tx.signedCallPayload) {
        throw new Error("No signed call payload found - transaction must be signed first");
      }
      
      if (!tx.callPayload) {
        throw new Error("No call payload found - transaction must be created first");
      }
      
      const chainConfig = CHAIN_CONFIGS[tx.network];
      const publicClient = createPublicClientForChain(chainConfig.chain, chainConfig.rpcUrl);
      
      const senderAddress = tx.senderAddress as Address;
      
      const signatureHashBytes = tx.callPayload.slice(0, 32);
      
      const serializedTxBytes = tx.callPayload.slice(32);
      
      const serializedTxHex = '0x' + Array.from(serializedTxBytes)
        .map(b => b.toString(16).padStart(2, '0'))
        .join('');
      
      const signatureBytes = tx.signedCallPayload;
      
      const signatureHex = '0x' + Array.from(signatureBytes)
        .map(b => b.toString(16).padStart(2, '0'))
        .join('');
      
      try {
        const recoveredPublicKey = await recoverPublicKey({
          hash: '0x' + Array.from(signatureHashBytes).map(b => b.toString(16).padStart(2, '0')).join('') as `0x${string}`,
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
      
    } catch (error) {
      console.error(`Failed to submit transaction for ${tx.network}:`, error);
      throw error;
    }
  },

  async createTx(tx: TxStateMachine): Promise<[Uint8Array, Uint8Array]> {
    console.log("Creating transaction:", tx);
    
    try {
      // Check if the chain is supported for EVM transactions
      if (!isSupportedEVMChain(tx.network)) {
        throw new Error(`Unsupported EVM chain: ${tx.network}`);
      }
      
      const chainConfig = CHAIN_CONFIGS[tx.network];

      const publicClient = createPublicClientForChain(chainConfig.chain, chainConfig.rpcUrl);
      
      // Parse addresses and amount
      const senderAddress = tx.senderAddress as Address;
      const receiverAddress = tx.receiverAddress as Address;
      const value = parseEther(tx.amount.toString());

      const nonce = await publicClient.getTransactionCount({
        address: senderAddress
      });

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
          throw new Error(`Unsupported chain: ${tx.network}`);
      }

      const serializedTx = serializeTransaction({
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
      
      return [hashBytes, serializedBytes];

    } catch (error) {
      console.error(`Failed to create transaction for ${tx.network}:`, error);
      throw error;
    }
  },
};