import { describe, test, expect, beforeAll, afterAll, it } from 'vitest'
import {
  initializeNode,
  setLogLevel,
  LogLevel,
  onLog,
  receiverConfirm,
  watchTxUpdates,
  fetchPendingTxUpdates
} from '../../node/wasm/vane_lib/api.js';
import {
  TxStateMachine,
  TxStateMachineManager,
  TokenManager,
  ChainSupported
} from '../../node/wasm/vane_lib/primitives.js';
import { logWasmExports, waitForWasmInitialization, setupWasmLogging, loadRelayNodeInfo, RelayNodeInfo, getWallets } from './utils/wasm_utils.js';
import { TestClient,LocalAccount, WalletActions, WalletClient, WalletClientConfig, hexToBytes, formatEther, PublicActions, bytesToHex } from 'viem'
import { NODE_EVENTS, NodeCoordinator } from './utils/node_coordinator.js'
import {
  mnemonicGenerate,
  mnemonicValidate,
  mnemonicToMiniSecret,
  ed25519PairFromSeed
} from '@polkadot/util-crypto';

// THE THIRD NODE IS THE MALICIOUS NODE THAT WILL BE TESTED

describe('WASM NODE & RELAY NODE INTERACTIONS', () => {
  let relayInfo: RelayNodeInfo | null = null;
  let walletClient: TestClient & WalletActions & PublicActions;
  let wasm_client_address: string | undefined = undefined;
  let privkey: string | undefined = undefined;
  let libp2pKey: string;
  let sender_client_address = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
  let receiver_client_address = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8"
  let nodeCoordinator: NodeCoordinator;

  
  beforeAll(async () => {
    try {
      relayInfo = await loadRelayNodeInfo();
    } catch (error) {
      throw error;
    }

    walletClient = getWallets()[2][0];
    privkey = getWallets()[2][1];
    wasm_client_address = walletClient!.account!.address;
    console.log('🔑 WASM_CLIENT_ADDRESS', wasm_client_address);

    try {
        // Set up logging
        setLogLevel(LogLevel.Debug);
        // onLog((level, message) => {
        //   console.log(`[${LogLevel[level]}] ${message}`);
        // });
        await waitForWasmInitialization();
      } catch (error) {
        throw error;
      }

      // Coordinator bound to MALICIOUS_NODE
      nodeCoordinator = NodeCoordinator.getInstance();
      nodeCoordinator.registerNode('MALICIOUS_NODE');

      // generate a libp2p key
      const mnemonic = mnemonicGenerate();
      const miniSecret = mnemonicToMiniSecret(mnemonic);
      libp2pKey = bytesToHex(miniSecret);

      // Initialize WASM node using vane_lib
      await initializeNode({
        relayMultiAddr: relayInfo!.multiAddr,
        account: wasm_client_address!,
        network: "Ethereum",
        libp2pKey: libp2pKey,
        live: false,
        logLevel: LogLevel.Debug
      });

      // Wait until malicious node is connected to a peer
      await nodeCoordinator.waitForEvent(
        NODE_EVENTS.PEER_CONNECTED,
        async () => { console.log('👂 MALICIOUS_NODE READY'); }
      );
  })

  it("it should receive a transaction and confirm it successfully",async() => {
    console.log(" \n \n TEST CASE: it should receive a transaction and confirm it successfully (MALICIOUS_NODE)");
    const receiverBalanceBefore = parseFloat(formatEther(await walletClient.getBalance({address: wasm_client_address as `0x${string}`})));
    

    await nodeCoordinator.waitForEvent(
        NODE_EVENTS.MALICIOUS_NODE_RESPONSE,
        async () => {
         console.log('👂 TRANSACTION_RECEIVED ON MALICIOUS_NODE');
         // fetch pending transactions
         const pendingTx = await fetchPendingTxUpdates();
         if (pendingTx.length === 0) {
          console.log('🔑 No pending transactions found');
          return;
         }
         const tx = pendingTx[0];
         if (!walletClient) throw new Error('walletClient not initialized');
         const account = walletClient.account!;
         // @ts-ignore
         const signature = await account.signMessage({ message: tx.receiverAddress });
         const txManager = new TxStateMachineManager(tx);
         txManager.setReceiverSignature(hexToBytes(signature as `0x${string}`));
         const updatedTx = txManager.getTx();
         console.log('🔑 Malicious node confirmed the transaction');
         await receiverConfirm(updatedTx);
       },
        350000
      );

      
  })

  afterAll(() => {
    nodeCoordinator.stop();
  })
})