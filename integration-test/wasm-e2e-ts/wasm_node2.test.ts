import { describe, test, expect, beforeAll, afterAll, it } from 'vitest'
import { hostFunctions } from '../../node/wasm/host_functions/main.js'
import init, * as wasmModule from '../../node/wasm/vane_lib/pkg/vane_wasm_node.js';
import { logWasmExports, waitForWasmInitialization, setupWasmLogging, loadRelayNodeInfo, RelayNodeInfo, startWasmNode, WasmNodeInstance, getWallets } from './utils/wasm_utils.js';
import { TestClient,LocalAccount, WalletActions, WalletClient, WalletClientConfig, hexToBytes, formatEther, PublicActions } from 'viem'
import { NODE_EVENTS, NodeCoordinator } from './utils/node_coordinator.js'
import { PublicInterfaceWorkerJs } from '../../node/wasm/vane_lib/pkg/vane_wasm_node.js';
import { TxStateMachine, TxStateMachineManager } from '../../node/wasm/vane_lib/primitives.js';

// THE SECOND NODE TEST IS THE SAME AS THE FIRST NODE TEST BUT WITH A DIFFERENT WALLET


describe('WASM NODE & RELAY NODE INTERACTIONS', () => {
  let relayInfo: RelayNodeInfo | null = null;
  let walletClient: TestClient & WalletActions & PublicActions;
  let wasm_client_address: string | undefined = undefined;
  let privkey: string | undefined = undefined;
  let sender_client_address = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
  let wasmNodeInstance: WasmNodeInstance | null = null;
  let nodeCoordinator: NodeCoordinator;
  let wasmLogger: any | null = null;
  
  beforeAll(async () => {
    try {
      relayInfo = await loadRelayNodeInfo();
    } catch (error) {
      throw error;
    }

    walletClient = getWallets()[1][0];
    privkey = getWallets()[1][1];
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
      nodeCoordinator.registerNode('RECEIVER_NODE');
      nodeCoordinator.setWasmLogger(hostFunctions.hostLogging.getLogInstance());

      // Start WASM node
      wasmNodeInstance = startWasmNode(relayInfo!.multiAddr, wasm_client_address!, "Ethereum", false);

      await wasmNodeInstance.promise;

      // Wait until receiver node is connected to a peer
      await nodeCoordinator.waitForEvent(
        NODE_EVENTS.PEER_CONNECTED,
        async () => { console.log('👂 RECEIVER_NODE READY'); }
      );
  })

  it("it should receive a transaction and confirm it successfully",async() => {
    console.log(" \n \n TEST CASE: it should receive a transaction and confirm it successfully (RECEIVER_NODE)");
    const receiverBalanceBefore = parseFloat(formatEther(await walletClient.getBalance({address: wasm_client_address as `0x${string}`})));
   
    await nodeCoordinator.waitForEvent(
        NODE_EVENTS.TRANSACTION_RECEIVED,
        async () => {
         console.log('👂 TRANSACTION_RECEIVED');
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
            console.log('🔑 UPDATED TX', updatedTx);
            await vaneWasm?.receiverConfirm(updatedTx);
          });
           
           // Fetch current pending transactions
           const receivedTx = await vaneWasm?.fetchPendingTxUpdates();
           expect(receivedTx.length).toBeGreaterThan(0);
         });
       },
        60000
      );

    await nodeCoordinator.waitForEvent(
      NODE_EVENTS.P2P_SENT_TO_EVENT, async () => { 
          // abritray wait for the sender node to submit the transaction
          await new Promise(resolve => setTimeout(resolve, 10000));
          console.log('sender finished its job and disconnected');
          const receiverBalanceAfter = parseFloat(formatEther(await walletClient.getBalance({address: wasm_client_address as `0x${string}`})));
          const balanceChange = Math.ceil(receiverBalanceAfter)-Math.ceil(receiverBalanceBefore);
          expect(balanceChange).toEqual(10);
       }
    );
    
  })

  afterAll(() => {
    nodeCoordinator.stop();
  })
})