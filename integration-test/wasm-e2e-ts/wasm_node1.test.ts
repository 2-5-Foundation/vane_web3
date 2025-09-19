import { describe, test, expect, beforeAll, afterAll } from 'vitest';
import init, { PublicInterfaceWorkerJs } from '../../node/wasm/vane_lib/pkg/vane_wasm_node.js';
import {
  loadRelayNodeInfo,
  setupWasmLogging,
  startWasmNode,
  waitForWasmInitialization,
  getWallets,
} from './utils/wasm_utils.js';
import { NODE_EVENTS, NodeCoordinator } from './utils/node_coordinator.js';
import hostFunctions from '../../node/wasm/host_functions/main.js';
import { logger, Logger } from '../../node/wasm/host_functions/logging.js';
import { TxStateMachine, TxStateMachineManager, UnsignedEip1559 } from '../../node/wasm/host_functions/primitives.js';
import { hexToBytes, bytesToHex, TestClient, WalletActions, parseTransaction, PublicActions, formatEther } from 'viem';
import { sign, serializeSignature } from 'viem/accounts';

describe('WASM NODE & RELAY NODE INTERACTIONS (Sender)', () => {
  let relayInfo: any;
  let nodeCoordinator: NodeCoordinator;
  let wasmNodeInstance: any;
  let walletClient: TestClient & WalletActions & PublicActions;
  let wasm_client_address: string;
  let privkey: string;
  let receiver_client_address: string = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
  let wasmLogger: any | null = null;


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

    // Init WASM + logging
    await init();
    hostFunctions.hostLogging.setLogLevel(hostFunctions.hostLogging.LogLevel.Debug);
    await waitForWasmInitialization();

    // Coordinator bound to SENDER_NODE
    nodeCoordinator = NodeCoordinator.getInstance();
    nodeCoordinator.registerNode('SENDER_NODE');
    nodeCoordinator.setWasmLogger(hostFunctions.hostLogging.getLogInstance());

    // Start WASM node
    wasmNodeInstance = startWasmNode(
      relayInfo.multiAddr,
      wasm_client_address,
      'Ethereum',
      false
    );
    await wasmNodeInstance.promise;

    await nodeCoordinator.waitForEvent(NODE_EVENTS.PEER_CONNECTED, async () => {
      console.log('✅ SENDER_NODE READY');
    });
    // arbitrary wait for receiver node to be ready ( we dont do cross test events yet)
    await new Promise(resolve => setTimeout(resolve, 5000));
  });

  test('should successfully initiate and confirm a transaction and submit it to the network', async () => {
    const senderBalanceBefore = parseFloat(formatEther(await walletClient.getBalance({address: wasm_client_address as `0x${string}`})));

    await wasmNodeInstance.promise.then((vaneWasm: any) => {
      return vaneWasm?.initiateTransaction(
        wasm_client_address,
        receiver_client_address,
        BigInt(10),
        'Eth',
        'Ethereum',
        'Maji'
      );
    });

    await nodeCoordinator.waitForEvent(NODE_EVENTS.SENDER_RECEIVED_RESPONSE, async () => {
      console.log('👂 SENDER_RECEIVED_RESPONSE');
      await wasmNodeInstance.promise.then((vaneWasm: PublicInterfaceWorkerJs | null) => {
        return vaneWasm?.watchTxUpdates(async (tx: TxStateMachine) => {
            
            // Skip if we already have a signature
            if (tx.signedCallPayload) {
              return;
            }
            
            // Only process when receiver has confirmed
            const isRecvConfirmed = (typeof tx.status === 'string' && tx.status === 'RecvAddrConfirmationPassed') ||
                                  (typeof tx.status === 'object' && 'RecvAddrConfirmationPassed' in tx.status);
            
            if (!isRecvConfirmed) {
              console.log("Skipping - not receiver confirmed yet");
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
            console.log('🔑 TX UPDATED');
            await vaneWasm?.senderConfirm(updatedTx);

        });
      });
    },120000);

    // Wait for either transaction submission success or failure
    try {
      await Promise.race([
        nodeCoordinator.waitForEvent(NODE_EVENTS.TRANSACTION_SUBMITTED_PASSED, async (data) => {
          console.log('✅ Transaction submitted successfully:', data);
          await wasmNodeInstance.promise.then(async (vaneWasm: PublicInterfaceWorkerJs | null) => {
             const tx: TxStateMachine[] = await vaneWasm?.fetchPendingTxUpdates();
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
          });
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
    const balanceChange = Math.ceil(senderBalanceBefore) - Math.ceil(senderBalanceAfter);
    expect(balanceChange).toEqual(10);

    // assert storage updates

    // assert metrics
   
  });

  afterAll(() => {
    nodeCoordinator?.stop();
  });
});
