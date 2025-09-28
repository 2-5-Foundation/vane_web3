#!/bin/bash

set -e

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RELAY_BIN="$PROJECT_ROOT/target/release/vane-web3-app"
RELAY_INFO_FILE="$PROJECT_ROOT/integration-test/wasm-e2e-ts/relay-info.json"

# Cleanup function
cleanup() {
  echo "Stopping relay node..."
  if [ -n "${RELAY_PID:-}" ] && ps -p "$RELAY_PID" > /dev/null 2>&1; then
    kill "$RELAY_PID" 2>/dev/null || true
  fi
  if [ -f "$RELAY_INFO_FILE" ]; then
    rm -f "$RELAY_INFO_FILE"
    echo "Cleaned up relay-info.json"
  fi
}

# Set up cleanup trap
trap cleanup INT TERM EXIT

echo "Building relay node..."
cargo build --release

echo "Starting relay node..."

# Clean up any existing relay info file to ensure fresh start
if [ -f "$RELAY_INFO_FILE" ]; then
    rm -f "$RELAY_INFO_FILE"
    echo "Cleaned up existing relay-info.json"
fi

# Create basic relay info file
cat > "$RELAY_INFO_FILE" <<EOF
{
  "peerId": "pending",
  "multiAddr": "pending",
  "host": "127.0.0.1",
  "port": 30333,
  "ready": false
}
EOF

echo "Relay info file created at $RELAY_INFO_FILE"

# Start relay in background to capture output
"$RELAY_BIN" relay-node > /tmp/relay.log 2>&1 &
RELAY_PID=$!

# Wait for peer ID and update file
echo "Waiting for peer ID..."
for i in {1..30}; do
  if grep -q "local_peer_id" /tmp/relay.log; then
    PEER_ID=$(grep "local_peer_id" /tmp/relay.log | head -1 | sed 's/.*local_peer_id=\([A-Za-z0-9]*\).*/\1/')
    MULTIADDR="/ip6/::1/tcp/30333/ws/p2p/${PEER_ID}"
    
    cat > "$RELAY_INFO_FILE" <<EOF
{
  "peerId": "${PEER_ID}",
  "multiAddr": "${MULTIADDR}",
  "host": "127.0.0.1",
  "port": 30333,
  "ready": true
}
EOF
    echo "Updated relay info with peer ID: $PEER_ID"
    break
  fi
  sleep 1
done

# Show logs in foreground
tail -f /tmp/relay.log