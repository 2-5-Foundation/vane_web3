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
  exportStorage
} from '../../node/wasm/vane_lib/api.js';
import {
  TxStateMachine,
  TxStateMachineManager,
  UnsignedEip1559,
  StorageExport,
  StorageExportManager,
  TokenManager,
  ChainSupported
} from '../../node/wasm/vane_lib/primitives.js';
import {
  loadRelayNodeInfo,
  waitForWasmInitialization,
  getWallets,
} from './utils/wasm_utils.js';
import { NODE_EVENTS, NodeCoordinator } from './utils/node_coordinator.js';
import { hexToBytes, bytesToHex, TestClient, WalletActions, parseTransaction, PublicActions, formatEther } from 'viem';
import { sign, serializeSignature } from 'viem/accounts';

// using vane_lib as a library


describe('WASM NODE & RELAY NODE INTERACTIONS (Sender)', () => {
  let relayInfo: any | null = null;
  let nodeCoordinator: NodeCoordinator;
  let walletClient: TestClient & WalletActions & PublicActions;
  let wasm_client_address: string;
  let privkey: string;
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

    // Wallet / address
    walletClient = getWallets()[0][0] as TestClient & WalletActions & PublicActions;
    privkey = getWallets()[0][1];
    wasm_client_address = walletClient.account!.address;
    console.log('🔑 WASM_CLIENT_ADDRESS', wasm_client_address);

    // Set up logging
    setLogLevel(LogLevel.Debug);
    // onLog((level, message) => {
    //   console.log(`[${LogLevel[level]}] ${message}`);
    // });
    await waitForWasmInitialization();

    // Coordinator bound to SENDER_NODE
    nodeCoordinator = NodeCoordinator.getInstance();
    nodeCoordinator.registerNode('SENDER_NODE');

    // Initialize WASM node using vane_lib
    await initializeNode({
      relayMultiAddr: relayInfo.multiAddr,
      account: wasm_client_address,
      network: "Ethereum",
      live: false,
      logLevel: LogLevel.Debug
    });

    await nodeCoordinator.waitForEvent(NODE_EVENTS.PEER_CONNECTED, async () => {
      console.log('✅ SENDER_NODE READY');
    });
    // arbitrary wait for receiver node to be ready ( we dont do cross test events yet)
    await new Promise(resolve => setTimeout(resolve, 5000));
  });

  test('should successfully initiate and confirm a transaction and submit it to the network', async () => {
    console.log(" \n \n TEST CASE 1: should successfully initiate and confirm a transaction and submit it to the network");
    const senderBalanceBefore = parseFloat(formatEther(await walletClient.getBalance({address: wasm_client_address as `0x${string}`})));

    const ethToken = TokenManager.createNativeToken(ChainSupported.Ethereum);
    
    await initiateTransaction(
      wasm_client_address,
      receiver_client_address,
      BigInt(10),
      ethToken,
      'Maji',
      ChainSupported.Ethereum,
      ChainSupported.Ethereum
    );

    await nodeCoordinator.waitForEvent(NODE_EVENTS.SENDER_RECEIVED_RESPONSE, async () => {
      console.log('👂 SENDER_RECEIVED_RESPONSE');
      const txUpdates: TxStateMachine[] = await fetchPendingTxUpdates();
      const tx = txUpdates[0]; // latest tx
       // Skip if we already have a signature
       if (tx.signedCallPayload) {
        return;
      }
      
      // Only process when receiver has confirmed
      const isRecvConfirmed = (typeof tx.status === 'string' && tx.status === 'RecvAddrConfirmationPassed') ||
                            (typeof tx.status === 'object' && 'RecvAddrConfirmationPassed' in tx.status);
      
      if (!isRecvConfirmed) {
        return;
      }
      
      console.log("Processing - receiver confirmed, signing transaction");

      if (!walletClient) throw new Error('walletClient not initialized');
      if (!walletClient.account) throw new Error('walletClient account not available');
      if (!tx.callPayload) {
        throw new Error('No call payload found');
      }
      if (!tx.ethUnsignedTxFields) {
        throw new Error('No unsigned transaction fields found');
      }

      const account = walletClient.account!;
      if (!account.signMessage) {
        throw new Error('Account signMessage function not available');
      }
      const [txHash, txBytes] = tx.callPayload!;
      const txSignature =  await sign({ hash: bytesToHex(txHash), privateKey: privkey as `0x${string}` });
      
      const txManager = new TxStateMachineManager(tx);
      txManager.setSignedCallPayload(hexToBytes(serializeSignature(txSignature)));
      const updatedTx = txManager.getTx();
      console.log('🔑 TX UPDATED', updatedTx.status);
      await senderConfirm(updatedTx);
    },120000);

    // Wait for either transaction submission success or failure
    try {
      await Promise.race([
        nodeCoordinator.waitForEvent(NODE_EVENTS.TRANSACTION_SUBMITTED_PASSED, async (data) => {
          console.log('✅ Transaction submitted successfully:', data);
          const tx: TxStateMachine[] = await fetchPendingTxUpdates();
          expect(tx).toBeDefined();
          
          // Convert BigInt to string for JSON serialization
          const txWithStringBigInts = JSON.parse(JSON.stringify(tx[tx.length - 1], (key, value) =>
            typeof value === 'bigint' ? value.toString() : value
          ));
          
          // Assert that the transaction was successfully submitted
          expect(txWithStringBigInts.status).toHaveProperty('TxSubmissionPassed');
          expect(txWithStringBigInts.status.TxSubmissionPassed).toBeDefined();
          expect(Array.isArray(txWithStringBigInts.status.TxSubmissionPassed)).toBe(true);
          
          return;
        }, 60000),
        nodeCoordinator.waitForEvent(NODE_EVENTS.TRANSACTION_SUBMITTED_FAILED, (data) => {
          console.log('❌ Transaction submission failed:', data);
        }, 60000)
      ]);
    } catch (error) {
      console.error('⏰ Timeout waiting for transaction submission result:', error);
      throw error;
    }

    // assert the balance changes
    const senderBalanceAfter = parseFloat(formatEther(await walletClient.getBalance({address: wasm_client_address as `0x${string}`})));
    console.log('🔑 SENDER BALANCE BEFORE', senderBalanceBefore);
    console.log('🔑 SENDER BALANCE AFTER', senderBalanceAfter);
    const balanceChange = Math.ceil(senderBalanceBefore) - Math.ceil(senderBalanceAfter);
    expect(balanceChange).toEqual(10);

    // assert storage updates
    const storage:StorageExport = await exportStorage() as StorageExport;
    const _totalTxAndBalanceChange = (10000 - senderBalanceAfter)/10;

    const storageManager = new StorageExportManager(storage);
    const metrics = storageManager.getSummary();
    console.log('🔑 STORAGE METRICS', metrics);
    expect(metrics.totalTransactions).toEqual(1);
    expect(metrics.successfulTransactions).toEqual(1);
    expect(metrics.failedTransactions).toEqual(0);
    expect(metrics.successRate).toEqual('100.00%');
    expect(metrics.totalValueSuccess).toEqual(10);
    expect(metrics.totalValueFailed).toEqual(0);
    expect(metrics.peersCount).toEqual(1);
    expect(metrics.accountsCount).toEqual(1);
    expect(metrics.currentNonce).toEqual(1);
    // assert metrics
   
  });

  test("should notify if the receiver is not registered", async () => {
    console.log(" \n \n TEST CASE 2: should notify if the receiver is not registered");
    const ethToken = TokenManager.createNativeToken(ChainSupported.Ethereum);
    
    await initiateTransaction(
      wasm_client_address,
      '0xa0Ee7A142d267C1f36714E4a8F75612F20a79720',
      BigInt(10),
      ethToken,
      'Maji',
      ChainSupported.Ethereum,
      ChainSupported.Ethereum
    );

    await nodeCoordinator.waitForEvent(NODE_EVENTS.RECEIVER_NOT_REGISTERED, async () => {
      console.log('👂 RECEIVER_NOT_REGISTERED EVENT');
      const tx: TxStateMachine[] = await fetchPendingTxUpdates();
      expect(tx).toBeDefined();
      expect(tx[0].status).toBe('ReceiverNotRegistered');
      console.log('🔑 asserted receiver not registered', tx[0].status);
    },70000);
      
  });

  test("should succesfully revert and cancel transaction even if malicious node confirm the transaction", async () => {
    console.log(" \n \n TEST CASE 3: should succesfully revert and cancel transaction if wrong address is confirmed by receiver");
    const receiverBalanceBefore = parseFloat(formatEther(await walletClient.getBalance({address: wasm_client_address as `0x${string}`})));
    const _intendedReceiverAddress = receiver_client_address;
    const ethToken = TokenManager.createNativeToken(ChainSupported.Ethereum);
    
     await initiateTransaction(
      wasm_client_address,
      wrong_receiver_client_address,
      BigInt(10),
      ethToken,
      'Wrong',
      ChainSupported.Ethereum,
      ChainSupported.Ethereum
    );

    await nodeCoordinator.waitForEvent(NODE_EVENTS.SENDER_RECEIVED_RESPONSE, async () => {
      console.log('👂 SENDER_RECEIVED_RESPONSE');
      const txUpdates: TxStateMachine[] = await fetchPendingTxUpdates();
      const latestTx = txUpdates[0]; 
      if (latestTx.codeWord !== 'Wrong') {
        return;
      }
      console.log('🔑 WRONG ADDRESS TX UPDATED', latestTx.status, latestTx.codeWord);
      console.log("The intended receiver did not receive the transaction notification, hence wrong receover confirmation");
      // check if the transaction is reverted
      if (latestTx.status.type === 'Reverted') {
          await revertTransaction(latestTx);
      }else{
        // revert the transaction with a reason
        await revertTransaction(latestTx, "Intended receiver not met");
      }
      
    },60000);

    
    await new Promise(resolve => setTimeout(resolve, 12000));

    const tx:TxStateMachine[] = await fetchPendingTxUpdates();
    expect(tx).toBeDefined();
    const latestTx = tx[0];
    if(latestTx.codeWord === 'Wrong'){
      if (latestTx.status.type === 'Reverted') {
        await revertTransaction(latestTx);
      }else{
        let txManager = new TxStateMachineManager(latestTx);
        txManager.setRevertedReason("Intended receiver not met");
        const updatedTx = txManager.getTx();
        await revertTransaction(updatedTx);
      }
    }
    await new Promise(resolve => setTimeout(resolve, 5000));
    const tx2:TxStateMachine[] = await fetchPendingTxUpdates();
    expect(tx2).toBeDefined();
    const latestTx2 = tx2[0];
    const s = latestTx2.status as any;
    const isReverted =
      (typeof s === 'string' && s === 'Reverted') ||
      (typeof s === 'object' && (s?.type === 'Reverted' || 'Reverted' in s));
    expect(isReverted).toBe(true);
    const receiverBalanceAfter = parseFloat(formatEther(await walletClient.getBalance({address: wasm_client_address as `0x${string}`})));
    const balanceChange = Math.ceil(receiverBalanceAfter)-Math.ceil(receiverBalanceBefore);
    expect(balanceChange).toEqual(0);
    // assert storage updates
    const storage:StorageExport = await exportStorage() as StorageExport;
    const storageManager = new StorageExportManager(storage);
    const metrics = storageManager.getSummary();
    console.log('🔑 STORAGE METRICS', metrics);
    expect(metrics.totalTransactions).toEqual(2);
    expect(metrics.successfulTransactions).toEqual(1);
    expect(metrics.failedTransactions).toEqual(1);
    expect(metrics.successRate).toEqual('50.00%');
    expect(metrics.totalValueSuccess).toEqual(10);
    expect(metrics.totalValueFailed).toEqual(10);
    expect(metrics.peersCount).toEqual(1);
    expect(metrics.accountsCount).toEqual(1);
    expect(metrics.currentNonce).toEqual(3);
    // assert metrics
  
  });

  test("should be able to successfully revert even when wrong address is selected by sender", async () => {
    console.log(" \n \n TEST CASE 4: should be able to successfully revert even when wrong address is selected by sender");
    
    const ethToken = TokenManager.createNativeToken(ChainSupported.Ethereum);
    
    let returnedTx: TxStateMachine;
    try {
      returnedTx = await initiateTransaction(
        wasm_client_address,
        wrong_receiver_client_address,
        BigInt(10),
        ethToken,
        'mistaken',
        ChainSupported.Ethereum,
        ChainSupported.Ethereum
      ) as unknown as TxStateMachine;
    } catch (e) {
      console.error('initiateTransaction failed', e);
      throw e;
    }

    await revertTransaction(returnedTx, "Changed my mind");

    const tx:TxStateMachine[] = await fetchPendingTxUpdates();
    expect(tx).toBeDefined();
    const latestTx = tx[0];
     const s = latestTx.status as any;
     const isReverted =
       (typeof s === 'string' && s === 'Reverted') ||
       (typeof s === 'object' && ('Reverted' in s));
     expect(isReverted).toBe(true);
     if (typeof s === 'object' && ('Reverted' in s)) {
       expect(s.Reverted).toBe('Changed my mind');
     }
     expect(latestTx.codeWord).toBe('mistaken');
     console.log("asserted reverted transaction, midway");

  });

  test("should successfully send ERC20 token transaction", async () => {
    await new Promise(resolve => setTimeout(resolve, 5000));
    console.log(" \n \n TEST CASE 5: should successfully send ERC20 token transaction");
    const ethERC20Token = TokenManager.createERC20Token(ChainSupported.Ethereum, '0x5FbDB2315678afecb367f032d93F642f64180aa3');
    await initiateTransaction(
      wasm_client_address,
      receiver_client_address,
      BigInt(100),
      ethERC20Token,
      'ERC20Testing',
      ChainSupported.Ethereum,
      ChainSupported.Ethereum
    );

    await new Promise(resolve => setTimeout(resolve, 20000));

    await nodeCoordinator.waitForEvent(NODE_EVENTS.SENDER_RECEIVED_RESPONSE, async () => {
      console.log('👂 SENDER_RECEIVED_RESPONSE');
      const txUpdates: TxStateMachine[] = await fetchPendingTxUpdates();
      const tx = txUpdates[0]; // latest tx
       // Skip if we already have a signature
       if (tx.signedCallPayload) {
        return;
      }
      if (tx.codeWord !== 'ERC20Testing') {
        return;
      }
      
      // Only process when receiver has confirmed
      const isRecvConfirmed = (typeof tx.status === 'string' && tx.status === 'RecvAddrConfirmationPassed') ||
                            (typeof tx.status === 'object' && 'RecvAddrConfirmationPassed' in tx.status);
      
      if (!isRecvConfirmed) {
        return;
      }
      
      console.log("Processing - receiver confirmed, signing transaction ERC20 TOKEN");

      if (!walletClient) throw new Error('walletClient not initialized');
      if (!walletClient.account) throw new Error('walletClient account not available');
      if (!tx.callPayload) {
        throw new Error('No call payload found');
      }
      if (!tx.ethUnsignedTxFields) {
        throw new Error('No unsigned transaction fields found');
      }

      const account = walletClient.account!;
      if (!account.signMessage) {
        throw new Error('Account signMessage function not available');
      }
      const [txHash, txBytes] = tx.callPayload!;
      const txSignature =  await sign({ hash: bytesToHex(txHash), privateKey: privkey as `0x${string}` });
      
      const txManager = new TxStateMachineManager(tx);
      txManager.setSignedCallPayload(hexToBytes(serializeSignature(txSignature)));
      const updatedTx = txManager.getTx();
      console.log('🔑 TX UPDATED ERC20 TOKEN', updatedTx.status);
      await senderConfirm(updatedTx);
    },120000);

    // Wait for either transaction submission success or failure
    try {
      await Promise.race([
        nodeCoordinator.waitForEvent(NODE_EVENTS.TRANSACTION_SUBMITTED_PASSED, async (data) => {
          console.log('✅ Transaction submitted successfully:', data);
          const tx: TxStateMachine[] = await fetchPendingTxUpdates();
          expect(tx).toBeDefined();
          
          // Convert BigInt to string for JSON serialization
          const txWithStringBigInts = JSON.parse(JSON.stringify(tx[tx.length - 1], (key, value) =>
            typeof value === 'bigint' ? value.toString() : value
          ));
          
          // Assert that the transaction was successfully submitted
          expect(txWithStringBigInts.status).toHaveProperty('TxSubmissionPassed');
          expect(txWithStringBigInts.status.TxSubmissionPassed).toBeDefined();
          expect(Array.isArray(txWithStringBigInts.status.TxSubmissionPassed)).toBe(true);
          
          return;
        }, 60000),
        nodeCoordinator.waitForEvent(NODE_EVENTS.TRANSACTION_SUBMITTED_FAILED, (data) => {
          console.log('❌ Transaction submission failed:', data);
        }, 60000)
      ]);
    } catch (error) {
      console.error('⏰ Timeout waiting for transaction submission result:', error);
      throw error;
    }

    
  });

  test("should succesfully revert and cancel transaction if wrong network is selected by sender", async () => {
    // i think this should be static test no need for recev end as the transaction wont even initiate
    // and later on we can see the cross chain shenanigans
  });

  test("should successfully revert and cancel transaction if sender tries to wrongfully send cross chain transaction", async () => {
    
  });

  test("should successfully send cross chain transaction", async () => {
    
  });

  test("should successfully handle fees conversion via intents", async () => {
    
  });

  test("should be able to collect fees from different chains based on transactions", async () => {
    
  });

  afterAll(() => {
    nodeCoordinator?.stop();
  });
});
