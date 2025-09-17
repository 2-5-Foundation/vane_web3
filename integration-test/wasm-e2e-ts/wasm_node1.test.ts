import { describe, test, expect, beforeAll, afterAll } from 'vitest';
import init from '../../node/wasm/vane_lib/pkg/vane_wasm_node.js';
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

describe('WASM NODE & RELAY NODE INTERACTIONS (Sender)', () => {
  let relayInfo: any;
  let nodeCoordinator: NodeCoordinator;
  let wasmNodeInstance: any;
  let walletClient: any;
  let wasm_client_address: string;
  let receiver_client_address: string = "0x63FaC9201494f0bd17B9892B9fae4d52fe3BD377";
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
    walletClient = getWallets()[0];
    wasm_client_address = walletClient.account.address;

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
    
    await wasmNodeInstance.promise.then((vaneWasm: any) => {
      return vaneWasm?.initiateTransaction(
        wasm_client_address,
        receiver_client_address,
        BigInt(1),
        'Eth',
        'Ethereum',
        'Maji'
      );
    });

    await new Promise(resolve => setTimeout(resolve, 60000));
   
  });

  afterAll(() => {
    nodeCoordinator?.stop();
  });
});
