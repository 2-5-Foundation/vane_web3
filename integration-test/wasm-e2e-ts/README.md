# WASM Node Browser Tests

This directory contains browser-based tests for the WASM node that interact with a relay node.

## Architecture

- **Relay Node**: Runs as a separate Node.js process (outside browser)
- **WASM Node**: Runs in the browser via Vitest
- **Communication**: Relay node writes its info to `relay-info.json`, browser tests read it

## Usage

### Option 1: Run tests with automatic relay startup

```bash
bun run test:with-relay
```

This will start the relay node, wait 5 seconds, run tests, then stop the relay node.

### Option 2: Manual control (recommended for development)

1. **Start the relay node** (in one terminal):
```bash
bun run start-relay
# or directly: bun run start-relay.ts
```

2. **Run the tests** (in another terminal):
```bash
bunx vitest run
```

3. **Stop the relay node** (Ctrl+C in the relay terminal)

## How it Works

1. **Relay Node Script** (`start-relay.ts`):
   - Spawns the relay node binary
   - Captures the real peer ID and multiaddr from stdout
   - Saves relay info to `relay-info.json`
   - Cleans up the file when stopped

2. **Browser Tests** (`wasm.test.ts`):
   - Uses `loadRelayNodeInfo()` to fetch relay details via HTTP
   - Starts the WASM node with the real multiaddr
   - Runs tests in browser environment

3. **WASM Utils** (`utils/wasm_utils.ts`):
   - Browser-compatible utilities (no `child_process`)
   - `loadRelayNodeInfo()` - loads relay details from JSON file
   - `startWasmNode()` - manages WASM node lifecycle with proper cleanup

## Files

- `start-relay.ts` - TypeScript script to start relay node and capture info
- `relay-info.json` - Auto-generated relay information (do not commit)
- `wasm.test.ts` - Browser tests for WASM node
- `utils/wasm_utils.ts` - Browser-compatible utilities
- `vitest.config.ts` - Vitest configuration for browser testing

## Environment Variables

- `RELAY_HOST` - Relay node host (default: 127.0.0.1)
- `RELAY_PORT` - Relay node port (default: 30333)

## Prerequisites

Make sure you have built the relay node binary:
```bash
# From project root
cargo build --release
```

The relay binary should be available at: `../../target/release/vane_web3_app`
