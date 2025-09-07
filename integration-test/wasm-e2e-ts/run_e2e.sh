#!/bin/bash

# Script to start WASM nodes 1, 2, and 3 (malicious) in separate terminals
# Each node will run in its own terminal window

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo -e "${BLUE}🚀 Starting WASM Nodes in Separate Terminals${NC}"
echo -e "${YELLOW}This will open 3 new terminal windows for the WASM nodes${NC}"
echo ""

# Get the directory where this script is located
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# Build steps
echo -e "${BLUE}🔨 Building required components...${NC}"

# Step 1: Build WASM node
echo -e "${YELLOW}Step 1: Building WASM node...${NC}"
cd ../../node/wasm

# Compile to wasm32-unknown-unknown
echo -e "${YELLOW}  Compiling to wasm32-unknown-unknown...${NC}"
cargo build --target wasm32-unknown-unknown --release

if [ $? -ne 0 ]; then
    echo -e "${RED}❌ Failed to compile WASM node to wasm32-unknown-unknown${NC}"
    exit 1
fi

# Build with wasm-pack
echo -e "${YELLOW}  Building with wasm-pack...${NC}"
wasm-pack build --target web --out-dir pkg

if [ $? -ne 0 ]; then
    echo -e "${RED}❌ Failed to build WASM package with wasm-pack${NC}"
    exit 1
fi

echo -e "${GREEN}✅ WASM node built successfully${NC}"

# Step 2: Build relay node
echo -e "${YELLOW}Step 2: Building relay node...${NC}"
cd ../../

# Build the relay node binary
echo -e "${YELLOW}  Compiling vane_web3_app...${NC}"
cargo build --release

if [ $? -ne 0 ]; then
    echo -e "${RED}❌ Failed to compile relay node${NC}"
    exit 1
fi

echo -e "${GREEN}✅ Relay node built successfully${NC}"

# Return to test directory
cd integration-test/wasm-e2e-ts
echo -e "${GREEN}✅ All components built successfully${NC}"
echo ""

# Function to start a node in a new terminal
start_node() {
    local node_name=$1
    local color=$2
    
    echo -e "${color}Starting ${node_name}...${NC}"
    
    # Create a temporary script for this node
    local node_id=$(echo "${node_name}" | tr '[:upper:]' '[:lower:]' | tr ' ' '_' | tr -d '()')
    local temp_script="/tmp/start_${node_id}.sh"
    cat > "$temp_script" << EOF
#!/bin/bash
cd "$SCRIPT_DIR"
echo -e "${color}=== ${node_name} Terminal ===${NC}"
echo -e "${color}Starting in 2 seconds...${NC}"
sleep 2

# Run the specific test file based on node name
if [[ "${node_name}" == "WASM Node 1" ]]; then
    bunx vitest run wasm_node1.test.ts --reporter=verbose
elif [[ "${node_name}" == "WASM Node 2" ]]; then
    bunx vitest run wasm_node2.test.ts --reporter=verbose
elif [[ "${node_name}" == "WASM Node 3 (Malicious)" ]]; then
    bunx vitest run wasm_node3_mal.test.ts --reporter=verbose
fi

echo -e "${color}${node_name} finished. Press any key to close this terminal...${NC}"
read -n 1
EOF

    chmod +x "$temp_script"
    
    # Open new terminal and run the script
    if [[ "$OSTYPE" == "darwin"* ]]; then
        # macOS - Use Terminal app
        osascript -e "tell application \"Terminal\" to do script \"$temp_script\""
    elif [[ "$OSTYPE" == "linux-gnu"* ]]; then
        # Linux
        gnome-terminal -- bash -c "$temp_script; exec bash" 2>/dev/null || \
        xterm -e "bash $temp_script" 2>/dev/null || \
        konsole -e "bash $temp_script" 2>/dev/null || \
        echo "Could not open terminal. Please run: $temp_script"
    else
        echo "Unsupported OS. Please run: $temp_script"
    fi
}

# Check if relay node is running
echo -e "${YELLOW}Checking if relay node is running...${NC}"
if ! pgrep -f "start-relay" > /dev/null; then
    echo -e "${RED}⚠️  Relay node is not running!${NC}"
    echo -e "${YELLOW}Starting relay node first...${NC}"
    
    # Start relay node in background
    bun run start-relay &
    RELAY_PID=$!
    
    # Wait for relay node to start
    echo -e "${YELLOW}Waiting for relay node to initialize (10 seconds)...${NC}"
    sleep 10
    
    echo -e "${GREEN}✅ Relay node started (PID: $RELAY_PID)${NC}"
else
    echo -e "${GREEN}✅ Relay node is already running${NC}"
fi

echo ""
echo -e "${BLUE}Starting WASM nodes...${NC}"

# Start Node 1
start_node "WASM Node 1" "$GREEN"

# Start Node 2
start_node "WASM Node 2" "$BLUE"

# Start Malicious Node 3
start_node "WASM Node 3 (Malicious)" "$RED"

echo ""
echo -e "${GREEN}✅ All WASM nodes have been started in separate terminals!${NC}"
echo -e "${YELLOW}Each terminal will show the logs for its respective node.${NC}"
echo ""
echo -e "${BLUE}To stop all nodes:${NC}"
echo -e "1. Close the terminal windows manually"
echo -e "2. Or run: pkill -f 'vitest run'"
echo -e "3. To stop relay node: pkill -f 'start-relay'"
echo ""

# Keep this script running to show status
echo -e "${YELLOW}Press Ctrl+C to exit this script (nodes will continue running)${NC}"
while true; do
    sleep 5
    # Check if any vitest processes are still running
    if ! pgrep -f "vitest run" > /dev/null; then
        echo -e "${YELLOW}All WASM node processes have finished.${NC}"
        break
    fi
done
