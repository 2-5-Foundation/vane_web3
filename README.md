# vane_web3

A full sovereign custodian implementation of risk-free transaction sending for web3 businesses and degens

### What are we solving?

  - Losing funds due wrong address input ( a huge pain currently in web3 as the action is not reversible after sending the transaction ).

  - Losings funds due wrong network selection while sending the transaction.

At some point the address can be correct but the choice of the network can result to loss of funds

### Our Solution

## Key Features

### Transaction Safety Layer

- Provides a safety net for Web3 transactions
- Verifies receiver addresses before transactions
- Supports multiple chains (BNB, Ethereum, Solana)

### P2P Networking

- Uses libp2p for peer-to-peer communication
- Handles peer discovery and connection management
- Manages swarm events and message routing

### RPC Server

- Provides JSON-RPC interface for transaction operations
- Handles transaction state updates
- Manages user interactions

### Transaction Flow

1. Genesis state: Initial transaction creation
2. Receiver address confirmation
3. Network confirmation
4. Sender confirmation
5. Transaction submission to blockchain

### Special Features

- e2e feature for end-to-end testing
- Support for both WASM and native environments
- Redis integration for distributed state management
- Local database for peer information caching

### Error Handling

- Comprehensive error handling using anyhow
- Transaction state machine for tracking transaction status
- Graceful error recovery and logging

## Architecture Benefits

The code is designed to provide a secure layer for Web3 transactions by:

- Verifying receiver addresses
- Confirming transaction details with both parties
- Providing a safety net against wrong addresses or networks
- Supporting multiple blockchain networks
- Using P2P networking for secure communication

This architecture ensures that transactions are verified and confirmed by both parties before being submitted to the blockchain, reducing the risk of sending funds to wrong addresses or networks.
