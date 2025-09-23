<p align="center">
  <img src="https://github.com/user-attachments/assets/997cc05e-b56c-45e5-8d5c-42cdaec4a9c4" alt="Vane Logo" width="100" height="auto">
</p>


# vane_web3

A full sovereign custodian implementation of risk-free transaction sending for web3 users.

### What are we solving?

  - Losing funds due to wrong address input ( a huge pain currently in web3 as the action is irreversible after sending the transaction ).

  - Losings funds due to wrong network selection while sending the transaction.

At some point the address can be correct but the choice of the network can result to loss of funds.

### Our Solution

Vane acts as a safety net for web3 users.

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

----
DEMO POST

https://x.com/MrishoLukamba/status/1866162459800707165


[App:](https://wwww.vaneweb3.com)

Sentiments:

[LinkedIn](https://www.linkedin.com/posts/jake-edwards-27bb44155_context-httpslnkdineaf7iyik-in-the-ugcPost-7352443063161532416-vObV?utm_source=social_share_send&utm_medium=member_desktop_web&rcm=ACoAADJZPCsBacGm1-F9KIi-L8AC5ynOqG1i-PE)

[X](https://x.com/onukogufavour/status/1945121048459976760?s=46)


## Technical design


### Special Features


- e2e feature for end-to-end testing
- Support for both WASM and native environments
- Redis integration for distributed state management
- Local database for peer information caching

It in itself is not a wallet, but can work with any type of wallet as it acts as an extension safety layer for web3.


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


This architecture ensures that transactions are verified and confirmed by both parties before being submitted to the blockchain, removing the risk of sending funds to wrong addresses or networks.

## Features
1. Vane provides a comprehensive safety net for Web3 users by ensuring receiver address confirmation, transaction execution simulation, and ownership verification of the receiver's account, thereby preventing losses from incorrect addresses and network selections before routing transactions to the intended destination.

2. Batching transactions, reducing fees drastically.
3. Turn any wallet into a smart account abstraction wallet
   
---

![SCR-20241217-pikb](https://github.com/user-attachments/assets/f8c82fa4-2d2b-46d8-87bf-7c1a7f18cae1)


## Prerequisites

1. **Install Rust**
```bash
=======

## HOW TO RUN & TEST
1. Install Rust
```

curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

2. **Install Bun** (for TypeScript scripts)
```bash
curl -fsSL https://bun.sh/install | bash
```

3. **Install wasm-pack** (for WASM builds)
```bash
curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh
```

## Quick Start

### 1. Build Everything
```bash
# Build all components
bun run build

# Or build individually
bun run build-wasm    # Build WASM package
bun run build-relay   # Build relay node
```

### 2. Run Tests

#### Database Tests
```bash
bun run test:db
```

#### End-to-End Tests (WASM + Relay)
```bash
bun run test:e2e
```

#### Individual Components
```bash
# Start relay node only
bun run start-relay

# Run WASM tests only (requires relay node running)
cd integration-test/wasm-e2e-ts
bunx vitest run
```

### 3. Run Native Application
```bash
# Run the main application
./target/release/vane_web3_app

# Or with custom database URL
./target/release/vane_web3_app --db-url "your-database-url"
```

## Detailed Usage

### Scripts Overview

All build and test scripts are centralized in the `scripts/` directory:

- **`scripts/build-wasm-package.sh`** - Builds WASM package with host functions
- **`scripts/run_e2e.sh`** - Complete e2e test runner (builds + tests)
- **`scripts/start-relay.ts`** - Relay node starter with info file generation
- **`scripts/db_tests.sh`** - Database setup and testing

### WASM Browser Testing

The WASM node can be tested in a browser environment:

#### Architecture
- **Relay Node**: Runs as separate process (outside browser)
- **WASM Node**: Runs in browser via Vitest/Playwright
- **Communication**: Relay writes info to `relay-info.json`, browser reads it

#### Manual Testing (Recommended for Development)
```bash
# Terminal 1: Start relay node
bun run start-relay

# Terminal 2: Run browser tests
cd integration-test/wasm-e2e-ts
bunx vitest run
```

#### Automatic Testing
```bash
# Runs relay + tests automatically
cd integration-test/wasm-e2e-ts
bun run test:with-relay
```

### Environment Variables

- `RELAY_HOST` - Relay node host (default: 127.0.0.1)
- `RELAY_PORT` - Relay node port (default: 30333)

### Docker Usage

```bash
# Build the image
docker build -t vane_web3_app .

# Run
docker run vane_web3_app
```

### Development Workflow

1. **Make changes to Rust code**
2. **Build components**: `bun run build`
3. **Run tests**: `bun run test:e2e`
4. **Test individual components** as needed

### Project Structure

```
vane_web3/
├── scripts/                    # All build/test scripts
│   ├── build-wasm-package.sh   # WASM package builder
│   ├── run_e2e.sh             # E2E test runner
│   ├── start-relay.ts         # Relay node starter
│   └── db_tests.sh            # Database testing
├── node/
│   ├── native/                # Pure native P2P node
│   ├── relay/                 # Relay P2P node
│   └── wasm/                  # WebAssembly client
├── integration-test/
│   └── wasm-e2e-ts/           # Browser-based WASM tests
├── db/                        # Database and migrations
└── primitives/                # Shared data structures
```

----

## CONTRIBUTE & GET PAID

1. tip issue  - 15 - 20 usd
2. medium issue - 30 - 60 usd
3. hard issue - 70 - 150 usd

----

🛣️ Roadmap
Our development roadmap for the vane_web3 project:

1. Build vane_web3 core features and node ✅

   - Completed the initial implementation of the core transaction processing, peer-to-peer communication, and database management features.


2. Build for WASM target 🚧

   - Currently in progress, focusing on making the codebase compatible with WebAssembly for browser and WASI environments.

3. Web application 🚧

   - Developing the web-based user interface and integration with the WASM-compiled core library.

4. Mobile client ⏱️

   - Planned for the next phase, to provide a native mobile experience for users.
  
5. Docker container MVP setup ✅
   - Spawn new docker containers for each registered user, this enables easy setup and better UX.
  

<img width="1239" alt="Screenshot 2025-01-11 at 02 18 55" src="https://github.com/user-attachments/assets/4cf97bbf-233f-46d6-96ba-374c98fbf7cd" />


## The team is actively working on the browser client and mobile PWA

---

## 📚 Documentation

This README serves as the single source of truth for all project documentation. All build scripts, test instructions, and usage information have been consolidated here for easy reference.

### Quick Reference

| Command | Description |
|---------|-------------|
| `bun run build` | Build all components |
| `bun run test:e2e` | Run complete e2e tests |
| `bun run start-relay` | Start relay node |
| `bun run test:db` | Run database tests |
| `bun run build-wasm` | Build WASM package only |
| `bun run build-relay` | Build relay node only |

### Need Help?

- All scripts are in the `scripts/` directory
- Check the project structure section above for file organization
- Environment variables are documented in the "Environment Variables" section
- Docker usage is covered in the "Docker Usage" section

