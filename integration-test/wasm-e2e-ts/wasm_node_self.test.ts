// Buffer polyfill for browser environment
import { Buffer } from 'buffer';
(globalThis as any).Buffer ??= Buffer;

import { describe, test, expect, beforeAll, afterAll } from 'vitest';
import {
  initializeNode,
  setLogLevel,
  LogLevel,
  initiateTransaction,
  senderConfirm,
  receiverConfirm,
  fetchPendingTxUpdates,
  exportStorage,
  addAccount,
  revertTransaction,
} from '../../node/wasm/vane_lib/api.js';
import {
  TxStateMachine,
  TxStateMachineManager,
  StorageExport,
  TokenManager,
  ChainSupported,
} from '../../node/wasm/vane_lib/primitives.js';
import { loadRelayNodeInfo, waitForWasmInitialization, getWallets } from './utils/wasm_utils.js';
import { hexToBytes, bytesToHex, TestClient, WalletActions, PublicActions, formatEther, ByteArray } from 'viem';
import { sign, serializeSignature } from 'viem/accounts';
import { nodeCoordinator } from './utils/node_coordinator.js';

describe('WASM NODE SELF TRANSACTIONS', () => {
  let relayInfo: any | null = null;
  let walletClient: TestClient & WalletActions & PublicActions;
  let walletClient2: TestClient & WalletActions & PublicActions;
  let wasm_client_address: string;
  let wasm_client_address2: string;
  let privkey: string;

  beforeAll(async () => {
    relayInfo = await loadRelayNodeInfo();

    walletClient = getWallets()[0][0] as TestClient & WalletActions & PublicActions;
    privkey = getWallets()[0][1];
    wasm_client_address = walletClient.account!.address;

    walletClient2 = getWallets()[3][0] as TestClient & WalletActions & PublicActions;
    wasm_client_address2 = walletClient2.account!.address;

    setLogLevel(LogLevel.Debug);
    await waitForWasmInitialization();

    await initializeNode({
      relayMultiAddr: relayInfo.multiAddr,
      account: wasm_client_address,
      network: 'Ethereum',
      live: false,
      self_node: true,
      logLevel: LogLevel.Debug,
    });

    // allow node to finish bootstrapping
    await new Promise((resolve) => setTimeout(resolve, 5000));
  });

  afterAll(() => {
    // no-op placeholder for future cleanup if needed
    nodeCoordinator.stop();
  });

  test('should successfuly send transaction and confirm to self', async () => {
    console.log(' \n \n TEST CASE: should successfuly send transaction and confirm to self');
    const senderBalanceBefore = parseFloat(
      formatEther(await walletClient.getBalance({ address: wasm_client_address as `0x${string}` })),
    );

    const ethToken = TokenManager.createNativeToken(ChainSupported.Ethereum);
    await addAccount(wasm_client_address2, ChainSupported.Ethereum);
    const storage = (await exportStorage()) as StorageExport;
    expect(storage.user_account?.accounts.length).toEqual(2);

    initiateTransaction(
      wasm_client_address,
      wasm_client_address2,
      BigInt(10) * BigInt(10 ** 18),
      ethToken,
      'Maji',
      ChainSupported.Ethereum,
      ChainSupported.Ethereum,
    );

    await new Promise((resolve) => setTimeout(resolve, 5000));

    const receiverReceivedTx = (await fetchPendingTxUpdates()) as TxStateMachine[];
    const latestTx = receiverReceivedTx[0];

    if (latestTx.receiverAddress !== wasm_client_address2) {
      return;
    }

    if (!walletClient2) throw new Error('walletClient not initialized');
    const recvAccount = walletClient2.account!;
    // @ts-ignore
    const signature = await recvAccount.signMessage({ message: latestTx.receiverAddress });
    const recvTxManager = new TxStateMachineManager(latestTx);
    recvTxManager.setReceiverSignature(Array.from(hexToBytes(signature as `0x${string}`)));
    const recvUpdatedTx = recvTxManager.getTx();
    await receiverConfirm(recvUpdatedTx);

    await new Promise((resolve) => setTimeout(resolve, 5000));
    const senderPendingTx = (await fetchPendingTxUpdates()) as TxStateMachine[];
    const senderPendinglatestTx = senderPendingTx[0];

    if (senderPendinglatestTx.receiverAddress !== wasm_client_address2) {
      return;
    }

    if (!walletClient) throw new Error('walletClient not initialized');
    if (!walletClient.account) throw new Error('walletClient account not available');
    if (!senderPendinglatestTx.callPayload) {
      throw new Error('No call payload found');
    }
    if (
      !senderPendinglatestTx.callPayload ||
      !('ethereum' in senderPendinglatestTx.callPayload) ||
      !senderPendinglatestTx.callPayload.ethereum.ethUnsignedTxFields
    ) {
      throw new Error('No unsigned transaction fields found');
    }

    const account = walletClient.account!;
    if (!account.signMessage) {
      throw new Error('Account signMessage function not available');
    }
    if (!senderPendinglatestTx.callPayload || !('ethereum' in senderPendinglatestTx.callPayload)) {
      throw new Error('No Ethereum call payload found');
    }
    const [txHash] = senderPendinglatestTx.callPayload.ethereum.callPayload;
    const txSignature = await sign({
      hash: bytesToHex(txHash as unknown as ByteArray),
      privateKey: privkey as `0x${string}`,
    });

    const txManager = new TxStateMachineManager(senderPendinglatestTx);
    txManager.setSignedCallPayload(Array.from(hexToBytes(serializeSignature(txSignature))));
    const updatedTx = txManager.getTx();
    console.log('🔑 TX UPDATED', updatedTx.status);
    await senderConfirm(updatedTx);

    await new Promise((resolve) => setTimeout(resolve, 2000));

    const senderBalanceAfter = parseFloat(
      formatEther(await walletClient.getBalance({ address: wasm_client_address as `0x${string}` })),
    );
    const balanceChange = Math.ceil(senderBalanceBefore) - Math.ceil(senderBalanceAfter);
    expect(balanceChange).toEqual(10);
  });

  test("should succesfuly revert and cancel transaction when sending to self", async () => {
    console.log(' \n \n TEST CASE: should succesfuly revert and cancel transaction when sending to self');
    const senderBalanceBefore = parseFloat(
      formatEther(await walletClient.getBalance({ address: wasm_client_address as `0x${string}` })),
    );
    
    const ethToken = TokenManager.createNativeToken(ChainSupported.Ethereum);
    await addAccount(wasm_client_address2, ChainSupported.Ethereum);
    const storage = (await exportStorage()) as StorageExport;
    expect(storage.user_account?.accounts.length).toEqual(2);

    initiateTransaction(
      wasm_client_address,
      wasm_client_address2,
      BigInt(10) * BigInt(10 ** 18),
      ethToken,
      'Maji',
      ChainSupported.Ethereum,
      ChainSupported.Ethereum,
    );

    const receiverReceivedTx = (await fetchPendingTxUpdates()) as TxStateMachine[];
    const latestTx = receiverReceivedTx[0];

    await revertTransaction(latestTx);
    
    await new Promise(resolve => setTimeout(resolve, 10000));

    const newTx:TxStateMachine[] = await fetchPendingTxUpdates();
    expect(newTx).toBeDefined();
    const newLatestTx = newTx;
    console.log('🔑 SELF NODE REVERTED TX', newLatestTx);
  });
});

