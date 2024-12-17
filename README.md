# vane_web3

A full sovereign custodian implementation of risk-free transaction sending for web3 users.

### What are we solving?
  - Losing funds due wrong address input ( a huge pain currently in web3 as the action is not reversible after sending the transaction ).

  - Losings funds due wrong network selection while sending the transaction.

At some point the address can be correct but the choice of the network can result to loss of funds

### Our Solution
vane act as a safety net for web3 users.

- Receiver address confirmation

- Transaction execution simulation

- Receiver account ownership confirmation after transaction execution and network simulation.

As this is crucial to make sure that you control account provided ( receiver ) in X network/blockchain.

After all confirmation, vane will route and submit the transaction to X address to the Y network/blokchain.

## Technical design

vane_web3 is designed to be decentralized and locally run by users having complete control.

In itself is not a wallet, but can work with any type of wallet as it acts as an extension safety layer for web3.

### Technical components
1. **DbWorker**
    
    - database worker is responsible for locally storing, user addresses, tx records, and peers that user interacted with
2. P2PWorker
    
    - responsible for sending pinging tx to receiver for confirmation on address and network
3. TxProcessingWorker

    - signature verification, creating tx payload to be signed, simulating tx, submitting tx
4. RpcWorker

    - interface where users interact with, signing tx, initiating tx, sender confirmation, receiver confirmation
5. TelemetryWorker

    - recording total volume of tx per user and reverted tx volume and number of peers(receivers) interacted with
6. MainServiceWorker

    - orchestrating all workers and having a single run function to spawn all workers and control the flow of passing tx state machine processing updates
7. Remote Db (Airtable)

    - a naive solution to dht ( peer discovery mechanism) as new users for vane will register their peer id along with account addresses
   to remote airtable server as a means to be discovered by other peers.

## Features
1. Vane provides a comprehensive safety net for Web3 users by ensuring receiver address confirmation, transaction execution simulation, and ownership verification of the receiver's account, thereby preventing losses from incorrect addresses and network selections before routing transactions to the intended destination.

2. Batching transactions, reducing fees drastically.
3. Turn any wallet into a smart account abstraction wallet


## HOW TO RUN & TEST
1. Install Rust
```
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

2. Compile & Run
```
cargo build --release
```

```
./target/release -p app
or 
./target/release -p app --db-url "url"
```

3. Test

```
cargo test --package integration-test --lib e2e_tests::transaction_processing_test -- --exact
```

## CONTRIBUTE & GET PAID

1. tip issue  - 15 - 20 usd
2. medium issue - 30 - 60 usd
3. hard issue - 70 - 150 usd
