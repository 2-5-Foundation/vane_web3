import { describe, test, expect, beforeAll, afterAll, it } from 'vitest'
import { hostFunctions } from '../../node/wasm/host_functions/main.js'
import init, * as wasmModule from '../../node/wasm/vane_lib/pkg/vane_wasm_node.js';
import { logWasmExports, waitForWasmInitialization, setupWasmLogging, loadRelayNodeInfo, RelayNodeInfo, startWasmNode, WasmNodeInstance, getWallets } from './utils/wasm_utils.js';
import { TestClient,LocalAccount, WalletActions, WalletClient, WalletClientConfig, hexToBytes, formatEther, PublicActions } from 'viem'
import { NODE_EVENTS, NodeCoordinator } from './utils/node_coordinator.js'
import { PublicInterfaceWorkerJs } from '../../node/wasm/vane_lib/pkg/vane_wasm_node.js';
import { TxStateMachine, TxStateMachineManager } from '../../node/wasm/vane_lib/primitives.js';

// THE THIRD NODE IS THE MALICIOUS NODE THAT WILL BE TESTED

describe('WASM NODE & RELAY NODE INTERACTIONS', () => {
  let relayInfo: RelayNodeInfo | null = null;
  let walletClient: TestClient & WalletActions & PublicActions;
  let wasm_client_address: string | undefined = undefined;
  let privkey: string | undefined = undefined;
  let sender_client_address = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
  let receiver_client_address = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8"
  let wasmNodeInstance: WasmNodeInstance | null = null;
  let nodeCoordinator: NodeCoordinator;
  let wasmLogger: any | null = null;

  
  beforeAll(async () => {
    try {
      relayInfo = await loadRelayNodeInfo();
    } catch (error) {
      throw error;
    }

    walletClient = getWallets()[2][0];
    privkey = getWallets()[2][1];
    wasm_client_address = walletClient!.account!.address;

    try {
        await init();
        wasmLogger = setupWasmLogging();
        await waitForWasmInitialization();
      } catch (error) {
        throw error;
      }

      // Coordinator bound to RECEIVER_NODE
      nodeCoordinator = NodeCoordinator.getInstance();
      nodeCoordinator.registerNode('MALICIOUS_NODE');
      nodeCoordinator.setWasmLogger(hostFunctions.hostLogging.getLogInstance());

      // Start WASM node
      wasmNodeInstance = startWasmNode(relayInfo!.multiAddr, wasm_client_address!, "Ethereum", false);

      await wasmNodeInstance.promise;

      // Wait until receiver node is connected to a peer
      await nodeCoordinator.waitForEvent(
        NODE_EVENTS.PEER_CONNECTED,
        async () => { console.log('👂 MALICIOUS_NODE READY'); }
      );
  })

  it("it should receive a transaction and confirm it successfully",async() => {
    console.log(" \n \n TEST CASE: it should receive a transaction and confirm it successfully (MALICIOUS_NODE)");
    await nodeCoordinator.waitForEvent(
        NODE_EVENTS.TRANSACTION_RECEIVED,
        async () => {
         console.log('👂 TRANSACTION_RECEIVED ON MALICIOUS_NODE');
         await wasmNodeInstance?.promise.then(async (vaneWasm: PublicInterfaceWorkerJs | null) => {
           // Set up the transaction watcher (await the Promise)
           await vaneWasm?.watchTxUpdates(async (tx: TxStateMachine) => {
            if (!walletClient) throw new Error('walletClient not initialized');
            const account = walletClient.account!;
            // @ts-ignore
            const signature = await account.signMessage({ message: tx.receiverAddress });
            const txManager = new TxStateMachineManager(tx);
            txManager.setReceiverSignature(hexToBytes(signature as `0x${string}`));
            const updatedTx = txManager.getTx();
            console.log('🔑 Malicious node confirmed the transaction');
            await vaneWasm?.receiverConfirm(updatedTx);
          });
           
          // Fetch current pending transactions
           const receivedTx = await vaneWasm?.fetchPendingTxUpdates();
           expect(receivedTx.length).toBeGreaterThan(0);
         });
       },
        60000
      );

    // await new Promise(resolve => setTimeout(resolve, 60000));

  })

  afterAll(() => {
    nodeCoordinator.stop();
  })
})