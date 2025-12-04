import { describe, test, expect, beforeAll, afterAll, it } from 'vitest'
import {
  initializeNode,
  setLogLevel,
  LogLevel,
  onLog,
  receiverConfirm,
  watchTxUpdates,
  fetchPendingTxUpdates,
  addAccount,
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
import nacl from "tweetnacl";
import {
  Connection as SolanaConnection,
  LAMPORTS_PER_SOL,
  Keypair,
  PublicKey,
} from "@solana/web3.js";

// THE SECOND NODE TEST IS THE SAME AS THE FIRST NODE TEST BUT WITH A DIFFERENT WALLET


describe('WASM NODE & RELAY NODE INTERACTIONS', () => {
  let relayInfo: RelayNodeInfo | null = null;
  let walletClient: TestClient & WalletActions & PublicActions;
  let wasm_client_address: string | undefined = undefined;
  let solWasmWallet2: Keypair;
  let solWasmWalletAddress2: string;
  let privkey: string | undefined = undefined;
  let libp2pKey: string;
  let sender_client_address = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
  let nodeCoordinator: NodeCoordinator;
  
  beforeAll(async () => {
    try {
      relayInfo = await loadRelayNodeInfo();
    } catch (error) {
      throw error;
    }

    walletClient = getWallets()[1][0];
    privkey = getWallets()[1][1];
    wasm_client_address = walletClient!.account!.address;
    console.log('🔑 WASM_CLIENT_ADDRESS', wasm_client_address);

    solWasmWallet2 = Keypair.fromSeed(hexToBytes("0x52e6cab5a778bc471b8d6fde1107ec0837a0bf19a783615490b7c469fc7c1800"));
    solWasmWalletAddress2 = solWasmWallet2.publicKey.toBase58();

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

      // Coordinator bound to RECEIVER_NODE
      nodeCoordinator = NodeCoordinator.getInstance();
      nodeCoordinator.registerNode('RECEIVER_NODE', wasm_client_address!);

      // Initialize WASM node using vane_lib
      await initializeNode({
        relayMultiAddr: relayInfo!.multiAddr,
        account: solWasmWalletAddress2,
        network: ChainSupported.Solana,
        self_node: false,
        live: false,
        logLevel: LogLevel.Debug
      });

  
  })

  // it("it should receive a ETH transaction and confirm it successfully",async() => {
  //   console.log(" \n \n TEST CASE 1: it should receive a transaction and confirm it successfully (RECEIVER_NODE)");
  //   const receiverBalanceBefore = parseFloat(formatEther(await walletClient.getBalance({address: wasm_client_address as `0x${string}`})));
   
  //   await nodeCoordinator.waitForEvent(
  //       NODE_EVENTS.TRANSACTION_RECEIVED,
  //       async () => {
  //        console.log('👂 TRANSACTION_RECEIVED');
  //        const receivedTx: TxStateMachine[] = await fetchPendingTxUpdates();
  //        const latestTx = receivedTx[0];
  //        if (!walletClient) throw new Error('walletClient not initialized');
  //        const account = walletClient.account!;
  //        // @ts-ignore
  //        const signature = await account.signMessage({ message: latestTx.receiverAddress });
  //        const txManager = new TxStateMachineManager(latestTx);
  //        txManager.setReceiverSignature(Array.from(hexToBytes(signature as `0x${string}`)));
  //        const updatedTx = txManager.getTx();
  //        await receiverConfirm(updatedTx);
  //      },
  //       60000
  //     );

  //   await nodeCoordinator.waitForEvent(
  //     NODE_EVENTS.P2P_SENT_TO_EVENT, async () => { 
  //         // abritray wait for the sender node to submit the transaction
  //         await new Promise(resolve => setTimeout(resolve, 10000));
  //         console.log('sender finished its job and disconnected');
  //         const receiverBalanceAfter = parseFloat(formatEther(await walletClient.getBalance({address: wasm_client_address as `0x${string}`})));
  //         const balanceChange = Math.ceil(receiverBalanceAfter)-Math.ceil(receiverBalanceBefore);
  //         expect(balanceChange).toEqual(10);
  //      },
  //      60000
  //   );
    
  // })

  // test("should successfully receive ERC20 token transaction", async () => {
  //   await new Promise(resolve => setTimeout(resolve, 15000));
  //   console.log(" \n \n TEST CASE 2: should successfully receive ERC20 token transaction and confirm it (RECEIVER_NODE)");
  //   await nodeCoordinator.waitForEvent(
  //     NODE_EVENTS.TRANSACTION_RECEIVED,
  //     async () => {
  //      console.log('👂 TRANSACTION_RECEIVED ERC20 TOKEN');
  //      await watchTxUpdates(async (tx: TxStateMachine) => {
  //        if(tx.codeWord !== 'ERC20Testing') {return;}else{
  //        console.log('🔑 WATCHING TX ERC20 TOKEN', tx.codeWord);
  //        if (!walletClient) throw new Error('walletClient not initialized');
  //        const account = walletClient.account!;
  //        // @ts-ignore
  //        const signature = await account.signMessage({ message: tx.receiverAddress });
  //        const txManager = new TxStateMachineManager(tx);
  //        txManager.setReceiverSignature(hexToBytes(signature as `0x${string}`));
  //        const updatedTx = txManager.getTx();
  //        await receiverConfirm(updatedTx);
  //        }
  //      });
  //    },
  //     60000
  //   );
  //   await new Promise(resolve => setTimeout(resolve, 30000));
  // });

  test("should successfully receive SOLANA token transaction", async () => {
    console.log(" \n \n TEST CASE 2: should successfully receive SOLANA token transaction and confirm it (RECEIVER_NODE)");
   

  
    await new Promise(resolve => setTimeout(resolve, 30000));

    console.log('👂 TRANSACTION_RECEIVED SOLANA TOKEN');
      const receiverReceivedTx: TxStateMachine[] = await fetchPendingTxUpdates();
      const latestTx = receiverReceivedTx[0];
      const msgBytes = new TextEncoder().encode(latestTx.receiverAddress);
      const signature = nacl.sign.detached(msgBytes, solWasmWallet2.secretKey);

      const recvTxManager = new TxStateMachineManager(latestTx);
      recvTxManager.setReceiverSignature(Array.from(signature));
      const recvUpdatedTx = recvTxManager.getTx();
      await receiverConfirm(recvUpdatedTx);
    
  })


  afterAll(() => {
    nodeCoordinator.stop();
  })
})