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

# Get the project root directory (parent of scripts directory)
PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$PROJECT_ROOT"

# Step 1: Build WASM node
echo -e "${YELLOW}Step 1: Building WASM node...${NC}"
if ! "$PROJECT_ROOT/scripts/build-wasm-package.sh"; then
    echo -e "${RED}❌ Failed to build WASM node. Exiting...${NC}"
    exit 1
fi

# Return to test directory
cd "$PROJECT_ROOT/integration-test/wasm-e2e-ts"
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
cd "$PROJECT_ROOT/integration-test/wasm-e2e-ts"
export VITE_USE_ANVIL=true
echo -e "${color}=== ${node_name} Terminal ===${NC}"
echo -e "${color}Starting in 2 seconds...${NC}"
sleep 2

# Run the specific test file based on node name
if [[ "${node_name}" == "WASM Node 1" ]]; then
    VITE_USE_ANVIL=true bunx vitest run wasm_node1.test.ts --reporter=verbose
elif [[ "${node_name}" == "WASM Node 2" ]]; then
    VITE_USE_ANVIL=true bunx vitest run wasm_node2.test.ts --reporter=verbose
elif [[ "${node_name}" == "WASM Node 3 (Malicious)" ]]; then
    VITE_USE_ANVIL=true bunx vitest run wasm_node3_mal.test.ts --reporter=verbose
fi

echo -e "${color}${node_name} finished. Press any key to close this terminal...${NC}"
read -n 1
EOF

    chmod +x "$temp_script"
    
    # Open new terminal and run the script
    if [[ "$OSTYPE" == "darwin"* ]]; then
        # macOS - Use Terminal app with different color schemes
        if [[ "${node_name}" == "WASM Node 1" ]]; then
            # Subtle green background for Node 1
            osascript -e "
            tell application \"Terminal\"
                set newTab to do script \"$temp_script\"
                set current settings of newTab to settings set \"Pro\"
                tell newTab
                    set background color to {0, 10240, 0}
                    set normal text color to {65535, 65535, 65535}
                end tell
            end tell"
        elif [[ "${node_name}" == "WASM Node 2" ]]; then
            # Subtle blue background for Node 2
            osascript -e "
            tell application \"Terminal\"
                set newTab to do script \"$temp_script\"
                set current settings of newTab to settings set \"Pro\"
                tell newTab
                    set background color to {0, 0, 10240}
                    set normal text color to {65535, 65535, 65535}
                end tell
            end tell"
        elif [[ "${node_name}" == "WASM Node 3 (Malicious)" ]]; then
            # Subtle red background for Malicious Node
            osascript -e "
            tell application \"Terminal\"
                set newTab to do script \"$temp_script\"
                set current settings of newTab to settings set \"Pro\"
                tell newTab
                    set background color to {10240, 0, 0}
                    set normal text color to {65535, 65535, 65535}
                end tell
            end tell"
        else
            # Default for other nodes
            osascript -e "tell application \"Terminal\" to do script \"$temp_script\""
        fi
    elif [[ "$OSTYPE" == "linux-gnu"* ]]; then
        # Linux - Set background colors using terminal-specific methods
        if [[ "${node_name}" == "WASM Node 1" ]]; then
            gnome-terminal --tab --title="WASM Node 1" --profile="Green" -- bash -c "$temp_script; exec bash" 2>/dev/null || \
            xterm -bg '#003300' -fg '#ffffff' -e "bash $temp_script" 2>/dev/null || \
            konsole --profile Green -e "bash $temp_script" 2>/dev/null
        elif [[ "${node_name}" == "WASM Node 2" ]]; then
            gnome-terminal --tab --title="WASM Node 2" --profile="Blue" -- bash -c "$temp_script; exec bash" 2>/dev/null || \
            xterm -bg '#000033' -fg '#ffffff' -e "bash $temp_script" 2>/dev/null || \
            konsole --profile Blue -e "bash $temp_script" 2>/dev/null
        elif [[ "${node_name}" == "WASM Node 3 (Malicious)" ]]; then
            gnome-terminal --tab --title="WASM Node 3 (Malicious)" --profile="Red" -- bash -c "$temp_script; exec bash" 2>/dev/null || \
            xterm -bg '#330000' -fg '#ffffff' -e "bash $temp_script" 2>/dev/null || \
            konsole --profile Red -e "bash $temp_script" 2>/dev/null
        else
            gnome-terminal -- bash -c "$temp_script; exec bash" 2>/dev/null || \
            xterm -e "bash $temp_script" 2>/dev/null || \
            konsole -e "bash $temp_script" 2>/dev/null
        fi
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
    cd "$PROJECT_ROOT/integration-test/wasm-e2e-ts"
    bun run start-relay &
    RELAY_PID=$!
    
    # Start simple HTTP server to serve relay-info.json
    echo -e "${YELLOW}Starting HTTP server to serve relay-info.json...${NC}"
    python3 -m http.server 8080 &
    HTTP_PID=$!
    
    # Wait for relay node to start and create relay-info.json
    echo -e "${YELLOW}Waiting for relay node to initialize and create relay-info.json...${NC}"
    
    # Wait up to 30 seconds for relay-info.json to be created
    for i in {1..30}; do
        if [ -f "relay-info.json" ]; then
            echo -e "${GREEN}✅ Relay info file found after ${i} seconds${NC}"
            break
        fi
        echo -e "${YELLOW}⏳ Waiting for relay-info.json... (${i}/30)${NC}"
        sleep 1
    done
    
    if [ ! -f "relay-info.json" ]; then
        echo -e "${RED}❌ Relay info file not created after 30 seconds${NC}"
        echo -e "${RED}Killing relay node and HTTP server, exiting...${NC}"
        kill $RELAY_PID 2>/dev/null
        kill $HTTP_PID 2>/dev/null
        exit 1
    fi
    
    # Give additional time for relay to be fully ready
    echo -e "${YELLOW}Giving relay node additional time to be fully ready...${NC}"
    sleep 5
    
    echo -e "${GREEN}✅ Relay node started (PID: $RELAY_PID)${NC}"
else
    echo -e "${GREEN}✅ Relay node is already running${NC}"
    
    # Check if relay-info.json exists
    if [ ! -f "relay-info.json" ]; then
        echo -e "${YELLOW}⚠️  Relay node is running but relay-info.json is missing${NC}"
        echo -e "${YELLOW}This might cause connection issues. Consider restarting the relay node.${NC}"
    fi
fi

echo ""
echo -e "${BLUE}Starting WASM nodes...${NC}"

# Start Node 1
start_node "WASM Node 1" "$GREEN"

# Add delay between node starts to prevent connection conflicts
echo -e "${YELLOW}Waiting 3 seconds before starting next node...${NC}"
sleep 3

# Start Node 2
start_node "WASM Node 2" "$BLUE"

# Add delay between node starts
# echo -e "${YELLOW}Waiting 3 seconds before starting next node...${NC}"
# sleep 3

# # Start Malicious Node 3
# start_node "WASM Node 3 (Malicious)" "$RED"

echo ""
echo -e "${GREEN}✅ All WASM nodes have been started in separate terminals!${NC}"
echo -e "${YELLOW}Each terminal will show the logs for its respective node.${NC}"
echo ""
echo -e "${BLUE}To stop all nodes:${NC}"
echo -e "1. Close the terminal windows manually"
echo -e "2. Or run: pkill -f 'vitest run'"
echo -e "3. To stop relay node: pkill -f 'start-relay'"
echo -e "4. To stop HTTP server: pkill -f 'http.server'"
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
