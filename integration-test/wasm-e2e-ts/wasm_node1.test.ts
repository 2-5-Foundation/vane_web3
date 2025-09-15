import { describe, test, expect, beforeAll, afterAll } from 'vitest';
import init from '../../node/wasm/pkg/vane_wasm_node.js';
import {
  loadRelayNodeInfo,
  setupWasmLogging,
  startWasmNode,
  waitForWasmInitialization,
  getWallets,
} from './utils/wasm_utils.js';
import { NodeCoordinator } from './utils/node_coordinator.js';
import { globalEventRecorder, NODE_EVENTS } from './utils/global_event_recorder.js';

describe('WASM NODE & RELAY NODE INTERACTIONS (Sender)', () => {
  let relayInfo: any;
  let wasmLogger: any;
  let nodeCoordinator: NodeCoordinator;
  let wasmNodeInstance: any;
  let walletClient: any;
  let wasm_client_address: string;

  beforeAll(async () => {
    // Relay info
    try {
      relayInfo = await loadRelayNodeInfo();
    } catch (error) {
      console.error('❌ Failed to load relay node info:', error);
      throw error;
    }

    // Wallet / address
    walletClient = getWallets()[0];
    wasm_client_address = walletClient.account.address;

    // Init WASM + logging
    await init();
    wasmLogger = setupWasmLogging();
    await waitForWasmInitialization();

    // Coordinator bound to SENDER_NODE
    nodeCoordinator = NodeCoordinator.getInstance();
    nodeCoordinator.registerNode('SENDER_NODE');
    nodeCoordinator.setWasmLogger(wasmLogger);

    // Start WASM node
    wasmNodeInstance = startWasmNode(
      relayInfo.multiAddr,
      wasm_client_address,
      'Ethereum',
      false
    );
    await wasmNodeInstance.promise;

    // Wait until *this* node is actually ready (sticky)
    await nodeCoordinator.waitForNodeReady(
      'SENDER_NODE',
      async () => {
        // Optional: custom announcement
        console.log('✅ SENDER_NODE is ready');
        nodeCoordinator.emitEvent('SENDER_NODE_READY', {
          nodeId: 'SENDER_NODE',
          ready: true,
          walletAddress: wasm_client_address,
        });
      }
    );
  });

  test('should successfully send a transaction', async () => {
    // Kick off the transaction (no arbitrary sleep)
    await wasmNodeInstance.promise.then((vaneWasm: any) => {
      return vaneWasm?.initiateTransaction(
        wasm_client_address,
        '0x63FaC9201494f0bd17B9892B9fae4d52fe3BD377',
        BigInt(1),
        'Eth',
        'Ethereum',
        'Maji'
      );
    });

    // Cross-file assertions via events (true coordination)
    // 1) Receiver actually got the request
    await globalEventRecorder.waitForEventFromNode(
      NODE_EVENTS.TRANSACTION_RECEIVED,
      'RECEIVER_NODE',
      60_000
    );

    // 2) Receiver confirmed (optional, but good to assert)
    await globalEventRecorder.waitForEventFromNode(
      NODE_EVENTS.RECEIVER_CONFIRMED,
      'RECEIVER_NODE',
      60_000
    );

    // 3) Sender submission success (if your logs emit this)
    const success = await globalEventRecorder.waitForEventFromNode(
      NODE_EVENTS.TRANSACTION_SUCCESS,
      'SENDER_NODE',
      60_000
    );
    expect(success).toBeTruthy();
  });

  afterAll(() => {
    nodeCoordinator.stop();
  });
});
