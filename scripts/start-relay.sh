#!/bin/bash

set -e

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RELAY_BIN="$PROJECT_ROOT/target/release/vane-web3-app"

# Cleanup function
cleanup() {
  echo "Stopping relay node..."
  if [ -n "${RELAY_PID:-}" ] && ps -p "$RELAY_PID" > /dev/null 2>&1; then
    kill "$RELAY_PID" 2>/dev/null || true
  fi
}

# Set up cleanup trap
trap cleanup INT TERM EXIT

echo "Building relay node..."
cargo build --release

echo "Starting relay node..."

# Start relay in background to capture output
"$RELAY_BIN" relay-node > /tmp/relay.log 2>&1 &
RELAY_PID=$!

# Show logs in foreground
tail -f /tmp/relay.log