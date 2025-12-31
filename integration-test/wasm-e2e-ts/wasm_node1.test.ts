
// Buffer polyfill for browser environment
import { Buffer } from 'buffer';
(globalThis as any).Buffer ??= Buffer;

import { describe, test, expect, beforeAll, afterAll } from 'vitest';
import {
  initializeNode,
  setLogLevel,
  LogLevel,
  onLog,
  initiateTransaction,
  senderConfirm,
  revertTransaction,
  fetchPendingTxUpdates,
  exportStorage,
  receiverConfirm,
  watchP2pNotifications,
  unsubscribeWatchP2pNotifications,
  addAccount
} from '../../node/wasm/vane_lib/api.js';
import {
  TxStateMachine,
  TxStateMachineManager,
  UnsignedEip1559,
  StorageExport,
  StorageExportManager,
  TokenManager,
  ChainSupported,
  BackendEvent,
} from '../../node/wasm/vane_lib/primitives.js';
import {
  loadRelayNodeInfo,
  waitForWasmInitialization,
  getWallets,
  getSplTokenBalance,
  waitForReceiverHandledEvent
} from './utils/wasm_utils.js';
import { NODE_EVENTS, NodeCoordinator } from './utils/node_coordinator.js';
import { hexToBytes, bytesToHex, TestClient, WalletActions, parseTransaction, PublicActions, formatEther, createTestClient, http, walletActions, publicActions, keccak256, ByteArray, SignTransactionParameters } from 'viem';
import { sign, serializeSignature, privateKeyToAccount } from 'viem/accounts';
import {
  mnemonicGenerate,
  mnemonicValidate,
  mnemonicToMiniSecret,
  ed25519PairFromSeed
} from '@polkadot/util-crypto';
import { bsc, mint, mode } from 'viem/chains';
import {
  Connection as SolanaConnection,
  LAMPORTS_PER_SOL,
  Keypair,
  PublicKey,
} from "@solana/web3.js";

import nacl from "tweetnacl";
import bs58 from 'bs58'
// Token creation imports removed - will be done via CLI


// using vane_lib as a library


// Helper function to remove data field from BackendEvent for cleaner logging
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

describe('WASM NODE & RELAY NODE INTERACTIONS (Sender)', () => {
  let relayInfo: any | null = null;
  let nodeCoordinator: NodeCoordinator;
  let walletClient: TestClient & WalletActions & PublicActions;
  let walletClient2: TestClient & WalletActions & PublicActions;
  
  let solanaClient: SolanaConnection;
  let solWasmWallet: Keypair;
  let solWasmWallet2: Keypair;
  let solWasmWalletAddress: string;
  let solWasmWalletAddress2: string;
 

  let wasm_client_address: string;
  let wasm_client_address2: string;
  let privkey: string;
  let privkey2: string;
  let libp2pKey: string;
  const receiver_client_address: string = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
  const wrong_receiver_client_address: string = "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC";


  beforeAll(async () => {
    // Relay info
    try {
      relayInfo = await loadRelayNodeInfo();
    } catch (error) {
      console.error('❌ Failed to load relay node info:', error);
      throw error;
    }

    // Solana Client & wallets
    solanaClient = new SolanaConnection("http://localhost:8899", "confirmed");
    solWasmWallet = Keypair.fromSeed(hexToBytes("0xf9aebb1cad8f89ede2475ad9977ff3960e4eb1bb0c9ceb793d1e61d85cf6a502"));
    solWasmWallet2 = Keypair.fromSeed(hexToBytes("0x52e6cab5a778bc471b8d6fde1107ec0837a0bf19a783615490b7c469fc7c1800"));
    solWasmWalletAddress = solWasmWallet.publicKey.toBase58();
    solWasmWalletAddress2 = solWasmWallet2.publicKey.toBase58();

    console.log('🔑 SOL WASM WALLET ADDRESS', solWasmWalletAddress);
    console.log('🔑 SOL WASM WALLET ADDRESS 2', solWasmWalletAddress2);

    const { blockhash, lastValidBlockHeight } = await solanaClient.getLatestBlockhash('confirmed');

    const signature = await solanaClient.requestAirdrop(solWasmWallet.publicKey, 1000 * LAMPORTS_PER_SOL);

    // // 3) Confirm using the *strategy* object
    await solanaClient.confirmTransaction(
      { signature, blockhash, lastValidBlockHeight },
      'finalized'
    );

    const { blockhash: blockHash2, lastValidBlockHeight: lastValidBlockHeight2 } = await solanaClient.getLatestBlockhash('confirmed');

    const signature2 = await solanaClient.requestAirdrop(solWasmWallet2.publicKey, 1000 * LAMPORTS_PER_SOL);

    // await solanaClient.confirmTransaction(
    //   { signature: signature2, blockhash: blockHash2, lastValidBlockHeight: lastValidBlockHeight2 },
    //   'confirmed'
    // );
    
    // Wallet / address
    walletClient = getWallets()[0][0] as TestClient & WalletActions & PublicActions;
    privkey = getWallets()[0][1];
    wasm_client_address = walletClient.account!.address;
    walletClient2 = getWallets()[3][0] as TestClient & WalletActions & PublicActions;
    privkey2 = getWallets()[3][1];
    wasm_client_address2 = walletClient2.account!.address;
    console.log('🔑 WASM_CLIENT_ADDRESS', wasm_client_address);


    // Set up logging
    setLogLevel(LogLevel.Debug);
    // onLog((level, message) => {
    //   console.log(`[${LogLevel[level]}] ${message}`);
    // });
    await waitForWasmInitialization();

    // Coordinator bound to SENDER_NODE
    nodeCoordinator = NodeCoordinator.getInstance();
    nodeCoordinator.registerNode('SENDER_NODE', wasm_client_address);

    

    // Initialize WASM node using vane_lib
    try {
      await initializeNode({
        relayMultiAddr: relayInfo.multiAddr,
        account: wasm_client_address,
        network: ChainSupported.Ethereum,
        live: false,
        self_node: false,
        logLevel: LogLevel.Debug,
      });
    } catch (error) {
      console.error('❌ Failed to initialize WASM node:', error);
      throw error;
    }

    // Register backend event watcher early to observe JSON-RPC backend events
    watchP2pNotifications((event: BackendEvent) => {
      logBackendEventWithoutData(event);
    });

   

    // arbitrary wait for receiver node to be ready ( we dont do cross test events yet)
    await new Promise(resolve => setTimeout(resolve, 5000));
  });

  // test('should successfully initiate and confirm a transaction and submit it to the network', async () => {
  //   console.log(" \n \n TEST CASE 1: should successfully initiate and confirm a transaction and submit it to the network");
  //   const senderBalanceBefore = parseFloat(formatEther(await walletClient.getBalance({address: wasm_client_address as `0x${string}`})));

  //   const ethToken = TokenManager.createNativeToken(ChainSupported.Ethereum);
    
  //   await initiateTransaction(
  //     wasm_client_address,
  //     receiver_client_address,
  //     BigInt(10) * BigInt(10 ** 18),
  //     ethToken,
  //     'Maji',
  //     ChainSupported.Ethereum,
  //     ChainSupported.Ethereum
  //   );

  //   await nodeCoordinator.waitForEvent(NODE_EVENTS.SENDER_RECEIVED_RESPONSE, async () => {
  //     console.log('👂 SENDER_RECEIVED_RESPONSE');
  //     const txUpdates: TxStateMachine[] = await fetchPendingTxUpdates();
  //     const tx = txUpdates[0]; // latest tx
  //      // Skip if we already have a signature
  //      if (tx.signedCallPayload) {
  //       console.log('🔑 TX ALREADY SIGNED', tx.status);
  //       return;
  //     }

  //     if(tx.receiverAddress !== receiver_client_address) {
  //       return;
  //     }
      
  //     // Only process when receiver has confirmed
  //     const isRecvConfirmed = tx.status.type === 'RecvAddrConfirmationPassed';
      
  //     if (!isRecvConfirmed) {
  //       console.log('🔑 RECV NOT CONFIRMED', tx.status);
  //       return;
  //     }
      
      
  //     console.log("Processing - receiver confirmed, signing transaction");

  //     if (!walletClient) throw new Error('walletClient not initialized');
  //     if (!walletClient.account) throw new Error('walletClient account not available');
  //     if (!tx.callPayload) {
  //       throw new Error('No call payload found');
  //     }
  //     if (!tx.callPayload || !('ethereum' in tx.callPayload) || !tx.callPayload.ethereum.ethUnsignedTxFields) {
  //       throw new Error('No unsigned transaction fields found');
  //     }

  //     const account = walletClient.account!;
  //     if (!account.signMessage) {
  //       throw new Error('Account signMessage function not available');
  //     }
  //     if (!tx.callPayload || !('ethereum' in tx.callPayload)) {
  //       throw new Error('No Ethereum call payload found');
  //     }
  //     const [txHash, txBytes] = tx.callPayload.ethereum.callPayload;
  //     const txSignature =  await sign({ hash: bytesToHex(txHash as unknown as ByteArray), privateKey: privkey as `0x${string}` });
      
  //     const txManager = new TxStateMachineManager(tx);
  //     txManager.setSignedCallPayload(Array.from(hexToBytes(serializeSignature(txSignature))));
  //     const updatedTx = txManager.getTx();
  //     console.log('🔑 TX UPDATED', updatedTx.status);
  //     await senderConfirm(updatedTx);
  //   },120000);

  //   // Wait for either transaction submission success or failure
  //   try {
  //     await Promise.race([
  //       nodeCoordinator.waitForEvent(NODE_EVENTS.TRANSACTION_SUBMITTED_PASSED, async (data) => {
  //         console.log('✅ Transaction submitted successfully:', data);
  //         const tx: TxStateMachine[] = await fetchPendingTxUpdates();
  //         expect(tx).toBeDefined();
          
  //         expect(tx[0].status.type).toEqual("TxSubmissionPassed");
          
  //         return;
  //       }, 60000),
  //       nodeCoordinator.waitForEvent(NODE_EVENTS.TRANSACTION_SUBMITTED_FAILED, (data) => {
  //         console.log('❌ Transaction submission failed:', data);
  //       }, 60000)
  //     ]);
  //   } catch (error) {
  //     console.error('⏰ Timeout waiting for transaction submission result:', error);
  //     throw error;
  //   }

  //   // assert the balance changes
  //   const senderBalanceAfter = parseFloat(formatEther(await walletClient.getBalance({address: wasm_client_address as `0x${string}`})));
  //   console.log('🔑 SENDER BALANCE BEFORE', senderBalanceBefore);
  //   console.log('🔑 SENDER BALANCE AFTER', senderBalanceAfter);
  //   const balanceChange = Math.ceil(senderBalanceBefore) - Math.ceil(senderBalanceAfter);
  //   expect(balanceChange).toEqual(10);


  //   // assert storage updates
  //   const storage:StorageExport = await exportStorage() as StorageExport;

  //   const storageManager = new StorageExportManager(storage);
  //   const metrics = storageManager.getSummary();  
  //   console.log('🔑 STORAGE METRICS', metrics);
    
   
  // });



  test("should succesfully revert and cancel transaction even if malicious node confirm the transaction", async () => {
    console.log(" \n \n TEST CASE 3: should succesfully revert and cancel transaction if wrong address is confirmed by receiver");
    const receiverBalanceBefore = parseFloat(formatEther(await walletClient.getBalance({address: wasm_client_address as `0x${string}`})));
    const _intendedReceiverAddress = receiver_client_address;
    const ethToken = TokenManager.createNativeToken(ChainSupported.Ethereum);
    
     await initiateTransaction(
      wasm_client_address,
      wrong_receiver_client_address,
      BigInt(10) * BigInt(10 ** 18),
      ethToken,
      'Wrong',
      ChainSupported.Ethereum,
      ChainSupported.Ethereum
    );

    await waitForReceiverHandledEvent(wrong_receiver_client_address);

    await new Promise(resolve => setTimeout(resolve, 40000));

    function getEventData(event: BackendEvent): number[] | null {
      if ('SenderRequestReceived' in event) {
        return event.SenderRequestReceived.data;
      } else if ('SenderRequestHandled' in event) {
        return event.SenderRequestHandled.data;
      } else if ('SenderConfirmed' in event) {
        return event.SenderConfirmed.data;
      } else if ('SenderReverted' in event) {
        return event.SenderReverted.data;
      } else if ('ReceiverResponseReceived' in event) {
        return event.ReceiverResponseReceived.data;
      } else if ('ReceiverResponseHandled' in event) {
        return event.ReceiverResponseHandled.data;
      } else if ('DataExpired' in event) {
        return event.DataExpired.data;
      } else if ('TxSubmitted' in event) {
        return event.TxSubmitted.data;
      }
      return null;
    }

    function deserializeEventData(event: BackendEvent): TxStateMachine | null {
      const data = getEventData(event);
      
      if (!data) {
        return null;
      }
      
      try {
        const jsonString = Buffer.from(data).toString('utf-8');
        const parsed = JSON.parse(jsonString);
        
        return {
          ...parsed,
          inboundReqId: typeof parsed.inboundReqId === 'number' ? parsed.inboundReqId : null,
          outboundReqId: typeof parsed.outboundReqId === 'number' ? parsed.outboundReqId : null,
          amount: BigInt(parsed.amount),
          callPayload: parsed.callPayload
            ? (() => {
                if ('ethereum' in parsed.callPayload) {
                  const fields = parsed.callPayload.ethereum.ethUnsignedTxFields;
                  return {
                    ethereum: {
                      ...parsed.callPayload.ethereum,
                      ethUnsignedTxFields: {
                        ...fields,
                        value: BigInt(fields.value),
                        gas: BigInt(fields.gas),
                        maxFeePerGas: BigInt(fields.maxFeePerGas),
                        maxPriorityFeePerGas: BigInt(fields.maxPriorityFeePerGas),
                      },
                    },
                  };
                }
                
                if ('bnb' in parsed.callPayload) {
                  const fields = parsed.callPayload.bnb.bnbLegacyTxFields;
                  return {
                    bnb: {
                      ...parsed.callPayload.bnb,
                      bnbLegacyTxFields: {
                        ...fields,
                        value: BigInt(fields.value),
                        gas: BigInt(fields.gas),
                        gasPrice: BigInt(fields.gasPrice),
                      },
                    },
                  };
                }
                
                return parsed.callPayload;
              })()
            : null,
        } as TxStateMachine;
      } catch (error) {
        console.error("Failed to deserialize event data:", error);
        return null;
      }
    }

    watchP2pNotifications((event: BackendEvent) => {
      console.log("NOTI EVENT: ", event);
      
      const data = getEventData(event);
      if (data) {
        const deserializedTx = deserializeEventData(event);
        if (deserializedTx) {
          console.log("DESERIALIZED TX: ", deserializedTx);
        }
      }
    });


    console.log('👂 SENDER_RECEIVED_RESPONSE');
    const txUpdates: TxStateMachine[] = await fetchPendingTxUpdates();
    const senderReceivedTx = txUpdates[0]; 
    console.log('🔑 MALICIOUS TX CODEWORD', senderReceivedTx.codeWord);
    if (senderReceivedTx.codeWord !== 'Wrong') {
      return;
    }
    console.log('🔑 WRONG ADDRESS TX UPDATED', senderReceivedTx.status, senderReceivedTx.codeWord);
    // check if the transaction is reverted
    if (senderReceivedTx.status.type === 'Reverted') {
        await revertTransaction(senderReceivedTx);
    }else{
      // revert the transaction with a reason
      await revertTransaction(senderReceivedTx, "Intended receiver not met");
    }
      
    
    // const tx:TxStateMachine[] = await fetchPendingTxUpdates();
    // expect(tx).toBeDefined();
    // const latestTx = tx;
    // console.log('🔑 LATEST TX MAL', latestTx);

    await new Promise(resolve => setTimeout(resolve, 15000));

    const newTx:TxStateMachine[] = await fetchPendingTxUpdates();
    expect(newTx).toBeDefined();
    const newLatestTx = newTx;
    console.log('🔑 NEW LATEST TX MAL', newLatestTx);
  
  });

  // test("should be able to successfully revert even when wrong address is selected by sender", async () => {
  //   console.log(" \n \n TEST CASE 4: should be able to successfully revert even when wrong address is selected by sender");
    
  //   const ethToken = TokenManager.createNativeToken(ChainSupported.Ethereum);
    
  //   let returnedTx: TxStateMachine;
  //   try {
  //     returnedTx = await initiateTransaction(
  //       wasm_client_address,
  //       wrong_receiver_client_address,
  //       BigInt(10),
  //       ethToken,
  //       'mistaken',
  //       ChainSupported.Ethereum,
  //       ChainSupported.Ethereum
  //     ) as unknown as TxStateMachine;
  //   } catch (e) {
  //     console.error('initiateTransaction failed', e);
  //     throw e;
  //   }

  //   await revertTransaction(returnedTx, "Changed my mind");

  //   const tx:TxStateMachine[] = await fetchPendingTxUpdates();
  //   expect(tx).toBeDefined();
  //   const latestTx = tx[0];
  //    const s = latestTx.status as any;
  //    const isReverted =
  //      (typeof s === 'string' && s === 'Reverted') ||
  //      (typeof s === 'object' && ('Reverted' in s));
  //    expect(isReverted).toBe(true);
  //    if (typeof s === 'object' && ('Reverted' in s)) {
  //      expect(s.Reverted).toBe('Changed my mind');
  //    }
  //    expect(latestTx.codeWord).toBe('mistaken');
  //    console.log("asserted reverted transaction, midway");

  //    await new Promise(resolve => setTimeout(resolve, 10000));
  //    // show the storage
  //    const storage:StorageExport = await exportStorage() as StorageExport;
  //    console.log('🔑 STORAGE FOR REVERTED TRANSACTION', storage);
  //    const storageManager = new StorageExportManager(storage);
  //    const metrics = storageManager.getSummary();
  //    console.log('🔑 STORAGE METRICS FOR REVERTED TRANSACTION SUMMARY', metrics);
     

  // });

  // test("should successfully send ERC20 token transaction", async () => {
  //   await new Promise(resolve => setTimeout(resolve, 5000));
  //   console.log(" \n \n TEST CASE 5: should successfully send ERC20 token transaction");
  //   const ethERC20Token = TokenManager.createERC20Token(ChainSupported.Ethereum,"ERC20Mock",'0x5FbDB2315678afecb367f032d93F642f64180aa3');
  //   await initiateTransaction(
  //     wasm_client_address,
  //     receiver_client_address,
  //     BigInt(100),
  //     ethERC20Token,
  //     'ERC20Testing',
  //     ChainSupported.Ethereum,
  //     ChainSupported.Ethereum
  //   );

  //   await new Promise(resolve => setTimeout(resolve, 20000));

  //   await nodeCoordinator.waitForEvent(NODE_EVENTS.SENDER_RECEIVED_RESPONSE, async () => {
  //     console.log('👂 SENDER_RECEIVED_RESPONSE');
  //     const txUpdates: TxStateMachine[] = await fetchPendingTxUpdates();
  //     const tx = txUpdates[0]; // latest tx
  //     console.log('🔑 TX', tx);
  //      // Skip if we already have a signatures
  //      if (tx.signedCallPayload) {
  //       return;
  //     }
  //     if (tx.codeWord !== 'ERC20Testing') {
  //       return;
  //     }
      
  //     // Only process when receiver has confirmed
  //     const isRecvConfirmed = (typeof tx.status === 'string' && tx.status === 'RecvAddrConfirmationPassed') ||
  //                           (typeof tx.status === 'object' && 'RecvAddrConfirmationPassed' in tx.status);
      
  //     if (!isRecvConfirmed) {
  //       return;
  //     }
      
  //     console.log("Processing - receiver confirmed, signing transaction ERC20 TOKEN");

  //     if (!walletClient) throw new Error('walletClient not initialized');
  //     if (!walletClient.account) throw new Error('walletClient account not available');
  //     if (!tx.callPayload) {
  //       throw new Error('No call payload found');
  //     }
  //     if (!tx.callPayload || !('ethereum' in tx.callPayload) || !tx.callPayload.ethereum.ethUnsignedTxFields) {
  //       throw new Error('No unsigned transaction fields found');
  //     }

  //     const account = walletClient.account!;
  //     if (!account.signMessage) {
  //       throw new Error('Account signMessage function not available');
  //     }
  //     if (!tx.callPayload || !('ethereum' in tx.callPayload)) {
  //       throw new Error('No Ethereum call payload found');
  //     }
  //     const [txHash, txBytes] = tx.callPayload.ethereum.callPayload;
  //     const txSignature =  await sign({ hash: bytesToHex(txHash), privateKey: privkey as `0x${string}` });
      
  //     const txManager = new TxStateMachineManager(tx);
  //     txManager.setSignedCallPayload(hexToBytes(serializeSignature(txSignature)));
  //     const updatedTx = txManager.getTx();
  //     console.log('🔑 TX UPDATED ERC20 TOKEN', updatedTx.status);
  //     await senderConfirm(updatedTx);
  //   },120000);

  //   // Wait for either transaction submission success or failure
  //   try {
  //     await Promise.race([
  //       nodeCoordinator.waitForEvent(NODE_EVENTS.TRANSACTION_SUBMITTED_PASSED, async (data) => {
  //         console.log('✅ Transaction submitted successfully:', data);
  //         const tx: TxStateMachine[] = await fetchPendingTxUpdates();
  //         expect(tx).toBeDefined();
          
  //         // Convert BigInt to string for JSON serialization
  //         const txWithStringBigInts = JSON.parse(JSON.stringify(tx[tx.length - 1], (key, value) =>
  //           typeof value === 'bigint' ? value.toString() : value
  //         ));
          
  //         // Assert that the transaction was successfully submitted
  //         expect(txWithStringBigInts.status).toHaveProperty('TxSubmissionPassed');
  //         expect(txWithStringBigInts.status.TxSubmissionPassed).toBeDefined();
  //         expect(Array.isArray(txWithStringBigInts.status.TxSubmissionPassed)).toBe(true);
          
  //         return;
  //       }, 60000),
  //       nodeCoordinator.waitForEvent(NODE_EVENTS.TRANSACTION_SUBMITTED_FAILED, (data) => {
  //         console.log('❌ Transaction submission failed:', data);
  //       }, 60000)
  //     ]);
  //   } catch (error) {
  //     console.error('⏰ Timeout waiting for transaction submission result:', error);
  //     throw error;
  //   }

  //   await new Promise(resolve => setTimeout(resolve, 15000));
    
  // });

//   // the tsStatus must be either TxSubmissionPassed or FailedToSubmitTxn
//   if(updatedLatestTx.status.type !== "TxSubmissionPassed" && updatedLatestTx.status.type !== "FailedToSubmitTxn"){
//     throw new Error('Tx Status must be either TxSubmissionPassed or FailedToSubmitTxn');
//   }

//   if(updatedLatestTx.status.type === "TxSubmissionPassed"){
//     const senderBalanceAfter = parseFloat(formatEther(await walletClient.getBalance({address: wasm_client_address as `0x${string}`})));
//     const balanceChange = Math.ceil(senderBalanceBefore)-Math.ceil(senderBalanceAfter);
//     expect(balanceChange).toEqual(10);
//   }
  
//   console.log("Storage ----->", storage);
// });

// test("should successfully send to EVM chain and confirm", async () => {
//   console.log(" \n \n TEST CASE 7: should successfully send to EVM chain and confirm (BNB)");
//   const [walletClient1,privkey1] = [createTestClient({
//     account: privateKeyToAccount('0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80'), 
//     chain: bsc,
//     mode: 'anvil',
//     transport: http('http://127.0.0.1:8555'),
//   }).extend(walletActions).extend(publicActions),'0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80']

//   // Fetch BNB balance
//   const balanceBefore = await walletClient1.getBalance({
//     address: walletClient1.account.address
//   });

//   await addAccount(wasm_client_address2, ChainSupported.Bnb);

//   const bnbToken = TokenManager.createNativeToken(ChainSupported.Bnb);
//   await initiateTransaction(
//     wasm_client_address,
//     wasm_client_address2,
//     BigInt(10),
//     bnbToken,
//     'Maji',
//     ChainSupported.Bnb,
//     ChainSupported.Bnb
//   )

//   await new Promise(resolve => setTimeout(resolve, 5000));

//   const receiverReceivedTx: TxStateMachine[] = await fetchPendingTxUpdates();
//   const latestTx = receiverReceivedTx[0];
//   if (!walletClient2) throw new Error('walletClient not initialized');
//   const recvAccount = walletClient2.account!;
//   // @ts-ignore
//   const signature = await recvAccount.signMessage({ message: latestTx.receiverAddress });
//   const recvTxManager = new TxStateMachineManager(latestTx);
//   recvTxManager.setReceiverSignature(hexToBytes(signature as `0x${string}`));
//   const recvUpdatedTx = recvTxManager.getTx();
//   await receiverConfirm(recvUpdatedTx);

//   await new Promise(resolve => setTimeout(resolve, 5000));
//   const senderPendingTx: TxStateMachine[] = await fetchPendingTxUpdates();
//   const senderPendinglatestTx = senderPendingTx[0];

//   if (!walletClient) throw new Error('walletClient not initialized');
//   if (!walletClient.account) throw new Error('walletClient account not available');
//   if (!senderPendinglatestTx.callPayload) {
//     throw new Error('No call payload found');
//   }
//   if (!senderPendinglatestTx.callPayload || !('bnb' in senderPendinglatestTx.callPayload) || !senderPendinglatestTx.callPayload.bnb.bnbLegacyTxFields) {
//     throw new Error('No unsigned transaction fields found');
//   }

//   const account = walletClient.account!;
//   if (!account.signMessage) {
//     throw new Error('Account signMessage function not available');
//   }
//   if (!senderPendinglatestTx.callPayload || !('bnb' in senderPendinglatestTx.callPayload)) {
//     throw new Error('No BNB call payload found');
//   }
//   const [txHash, txBytes] = senderPendinglatestTx.callPayload.bnb.callPayload;
//   const txSignature =  await sign({ hash: bytesToHex(txHash), privateKey: privkey as `0x${string}` });
  
//   const txManager = new TxStateMachineManager(senderPendinglatestTx);
//   txManager.setSignedCallPayload(hexToBytes(serializeSignature(txSignature)));
//   const updatedTx = txManager.getTx();
//   console.log('🔑 TX UPDATED', updatedTx.status);
//   await senderConfirm(updatedTx);

//   await new Promise(resolve => setTimeout(resolve, 2000));
//   const balanceAfter = await walletClient1.getBalance({
//     address: walletClient1.account.address
//   });
//   console.log('🔑 BALANCE BEFORE', formatEther(balanceBefore));
//   console.log('🔑 BALANCE AFTER', formatEther(balanceAfter));
//   const balanceChange = Math.ceil(Number(formatEther(balanceBefore))) - Math.ceil(Number(formatEther(balanceAfter)));
//   console.log('🔑 BALANCE CHANGE', balanceChange);
// })

// test("should successfully send BEP20 token transaction", async () => {
//   console.log(" \n \n TEST CASE 8: should successfully send BEP20 token transaction");
//    let BnbTokenAddress: string;
//    try {
//      const response = await fetch('/bnb_token.txt');
//      if (!response.ok) {
//        throw new Error(`HTTP ${response.status}: ${response.statusText}`);
//      }
//      BnbTokenAddress = (await response.text()).trim();
//      console.log('🪙 Using BNB Chain BEP20 token from fund-evm.sh:', BnbTokenAddress);
//    } catch (error) {
//      console.error('❌ Failed to read bnb_token.txt, falling back to hardcoded address:', error);
//      // Fallback: use a hardcoded address (this should not happen if fund-evm.sh ran successfully)
//      BnbTokenAddress = '0x5FbDB2315678afecb367f032d93F642f64180aa3';
//    }
  
//   // fetch the BNB token balance of the sender
//   const bnbBEP20Token = TokenManager.createBEP20Token("ERC20Mock", BnbTokenAddress);
//   await initiateTransaction(
//     wasm_client_address,
//     wasm_client_address2,
//     BigInt(10),
//     bnbBEP20Token,
//     'BEP20Testing',
//     ChainSupported.Bnb,
//     ChainSupported.Bnb
//   );

//   await new Promise(resolve => setTimeout(resolve, 5000));

//   const receiverReceivedTx: TxStateMachine[] = await fetchPendingTxUpdates();
//   const latestTx = receiverReceivedTx[0];
//   if (!walletClient2) throw new Error('walletClient not initialized');
//   const recvAccount = walletClient2.account!;
//   // @ts-ignore
//   const signature = await recvAccount.signMessage({ message: latestTx.receiverAddress });
//   const recvTxManager = new TxStateMachineManager(latestTx);
//   recvTxManager.setReceiverSignature(hexToBytes(signature as `0x${string}`));
//   const recvUpdatedTx = recvTxManager.getTx();
//   await receiverConfirm(recvUpdatedTx);

//   await new Promise(resolve => setTimeout(resolve, 5000));

//   const senderPendingTx: TxStateMachine[] = await fetchPendingTxUpdates();
//   const senderPendinglatestTx = senderPendingTx[0];

//   if (!walletClient) throw new Error('walletClient not initialized');
//   if (!walletClient.account) throw new Error('walletClient account not available');
//   if (!senderPendinglatestTx.callPayload) {
//     throw new Error('No call payload found');
//   }
//   if (!senderPendinglatestTx.callPayload || !('bnb' in senderPendinglatestTx.callPayload) || !senderPendinglatestTx.callPayload.bnb.bnbLegacyTxFields) {
//     throw new Error('No unsigned transaction fields found');
//   }

//   const account = walletClient.account!;
//   if (!account.signMessage) {
//     throw new Error('Account signMessage function not available');
//   }
//   if (!senderPendinglatestTx.callPayload || !('bnb' in senderPendinglatestTx.callPayload)) {
//     throw new Error('No BNB call payload found');
//   }
//   const [txHash, txBytes] = senderPendinglatestTx.callPayload.bnb.callPayload;
//   const txSignature =  await sign({ hash: bytesToHex(txHash), privateKey: privkey as `0x${string}` });
  
//   const txManager = new TxStateMachineManager(senderPendinglatestTx);
//   txManager.setSignedCallPayload(hexToBytes(serializeSignature(txSignature)));
//   const updatedTx = txManager.getTx();
//   console.log('🔑 TX UPDATED', updatedTx.status);
//   await senderConfirm(updatedTx);

//   await new Promise(resolve => setTimeout(resolve, 5000));

// })

test.skip("should successfully send to Solana chain and confirm", async () => {
  console.log(" \n \n TEST CASE 9: should successfully send to Solana chain and confirm");


  const solanaToken = TokenManager.createNativeToken(ChainSupported.Solana);
  const lamports = await solanaClient.getBalance(solWasmWallet.publicKey, 'confirmed');
  const solBalanceBefore = lamports / LAMPORTS_PER_SOL;
  console.log('🔑 SOL BALANCE BEFORE', solBalanceBefore);
 

  initiateTransaction(
    solWasmWalletAddress,
    solWasmWalletAddress2,
    BigInt(10) * BigInt(10 ** 9),
    solanaToken,
    'SOLANA',
    ChainSupported.Solana,
    ChainSupported.Solana
  )

  // Wait for ReceiverResponseHandled backend event with matching address
  waitForReceiverHandledEvent(solWasmWalletAddress2);

  // Wait a bit for the cache to be updated with call_payload after the backend event
  await new Promise(resolve => setTimeout(resolve, 15000));

  const senderPendingTx: TxStateMachine[] = await fetchPendingTxUpdates();
  const senderPendinglatestTx = senderPendingTx[0];
  
  if (!senderPendinglatestTx.callPayload || !('solana' in senderPendinglatestTx.callPayload)) {
    throw new Error('No Solana call payload found');
  }
  const callPayload = senderPendinglatestTx.callPayload.solana.callPayload;
  const txSignature = nacl.sign.detached(new Uint8Array(callPayload), solWasmWallet.secretKey);
  if(txSignature.length !== 64) {
    throw new Error('txSignature length is not 64');
  }
  
  const txManager = new TxStateMachineManager(senderPendinglatestTx);
  txManager.setSignedCallPayload(Array.from(txSignature));
  const updatedTx = txManager.getTx();
  await senderConfirm(updatedTx);

  await new Promise(resolve => setTimeout(resolve, 15000));
  
  const lamportsAfter = await solanaClient.getBalance(solWasmWallet.publicKey, 'confirmed');
  const solBalanceAfter = lamportsAfter / LAMPORTS_PER_SOL;
  console.log('🔑 SOL BALANCE AFTER', solBalanceAfter);
  const balanceChange = Math.ceil(solBalanceBefore) - Math.ceil(solBalanceAfter);
  expect(balanceChange).toEqual(10);

})

//  test("should successfully send SPL token to Solana chain and confirm", async () => {
//   console.log(" \n \n TEST CASE 10: should successfully send SPL token to Solana chain and confirm");

//   // Read the token mint from the file created by fund-solana.sh (served via HTTP)
//   let tokenMint: PublicKey;
//   try {
//     const response = await fetch('/token_mint.txt');
//     if (!response.ok) {
//       throw new Error(`HTTP ${response.status}: ${response.statusText}`);
//     }
//     const tokenMintAddress = await response.text();
//     tokenMint = new PublicKey(tokenMintAddress.trim());
//     console.log('🪙 Using existing token mint from fund-solana.sh:', tokenMint.toBase58());
//   } catch (error) {
//     console.error('❌ Failed to read token_mint.txt, falling back to generating new token:', error);
//     throw error;
//   }

//   const splToken = TokenManager.createSPLToken('SPLTEST', tokenMint!.toBase58());

//   // fetch the SPL token balance of the sender
//   const senderSplTokenBalanceBefore = await getSplTokenBalance(solanaClient, tokenMint, solWasmWallet.publicKey);

//   try {
//     await addAccount(solWasmWalletAddress2, ChainSupported.Solana);
//   } catch (e) {
//     console.error('addAccount failed', e);
//     throw e;
//   }
  
//   await initiateTransaction(
//     solWasmWalletAddress,
//     solWasmWalletAddress2,
//     BigInt(10),
//     splToken,
//     'SPLToken',
//     ChainSupported.Solana,
//     ChainSupported.Solana
//   )

//   await new Promise(resolve => setTimeout(resolve, 5000));
//   const receiverReceivedTx: TxStateMachine[] = await fetchPendingTxUpdates();
//   const latestTx = receiverReceivedTx[0];
//   const msgBytes = new TextEncoder().encode(latestTx.receiverAddress);
//   const signature = nacl.sign.detached(msgBytes, solWasmWallet2.secretKey);

//   const recvTxManager = new TxStateMachineManager(latestTx);
//   recvTxManager.setReceiverSignature(signature);
//   const recvUpdatedTx = recvTxManager.getTx();
//   await receiverConfirm(recvUpdatedTx);

//   await new Promise(resolve => setTimeout(resolve, 5000));
  
//   const senderPendingTx: TxStateMachine[] = await fetchPendingTxUpdates();
//   const senderPendinglatestTx = senderPendingTx[0];
//   if (!senderPendinglatestTx.callPayload || !('solana' in senderPendinglatestTx.callPayload)) {
//     throw new Error('No Solana call payload found');
//   }
//   const senderCallPayload = senderPendinglatestTx.callPayload.solana.callPayload;
//   const senderTxSignature = nacl.sign.detached(new Uint8Array(senderCallPayload), solWasmWallet.secretKey);
//   const senderTxManager = new TxStateMachineManager(senderPendinglatestTx);
//   senderTxManager.setSignedCallPayload(senderTxSignature);
//   const senderUpdatedTx = senderTxManager.getTx();
//   await senderConfirm(senderUpdatedTx);

//   await new Promise(resolve => setTimeout(resolve, 8000));

//   const senderSplTokenBalanceAfter = await getSplTokenBalance(solanaClient, tokenMint, solWasmWallet.publicKey);
//   const balanceChange = senderSplTokenBalanceBefore.uiAmount - senderSplTokenBalanceAfter.uiAmount;
//   expect(balanceChange).toEqual(10);

  
//   const storage:StorageExport = await exportStorage() as StorageExport;
//   console.log('🔑 EXPORT STORAGE', storage);
 
//  })

  // test("should succesfully revert and cancel transaction if wrong network is selected by sender", async () => {
  //   // i think this should be static test no need for recev end as the transaction wont even initiate
  //   // and later on we can see the cross chain shenanigans
  // });

  // test("should successfully revert and cancel transaction if sender tries to wrongfully send cross chain transaction", async () => {
    
  // });

  // test("should successfully send cross chain transaction", async () => {
    
  // });

  // test("should successfully handle fees conversion via intents", async () => {
    
  // });

  // test("should be able to collect fees from different chains based on transactions", async () => {
    
  // });

  afterAll(() => {
    nodeCoordinator?.stop();
  });
});
