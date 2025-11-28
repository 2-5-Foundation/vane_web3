#!/bin/bash

# Script to start WASM nodes 1, 2, and 3 (malicious) in separate terminals
# Each node will run in its own terminal window

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
PINK='\033[1;35m'
NC='\033[0m' # No Color

# Default values
LIVE="${LIVE:-false}"

# Parse command line arguments
while [[ $# -gt 0 ]]; do
  case $1 in
    --live)
      LIVE="true"
      shift
      ;;
    -h|--help)
      echo "Usage: $0 [OPTIONS]"
      echo "Options:"
      echo "  --live               Use live relay node (vane-relay.vaneweb3.com)"
      echo "  -h, --help           Show this help message"
      exit 0
      ;;
    *)
      echo "Unknown option: $1"
      echo "Use --help for usage information"
      exit 1
      ;;
  esac
done

echo -e "${BLUE}🚀 Starting WASM Nodes in Separate Terminals${NC}"
if [[ "$LIVE" == "true" ]]; then
    echo -e "${YELLOW}🌐 LIVE MODE: Using production relay node${NC}"
else
    echo -e "${YELLOW}🏠 LOCAL MODE: Using local relay node${NC}"
fi
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

# Step 2: Start DHT service
echo -e "${YELLOW}Step 2: Starting DHT service...${NC}"
cd "$PROJECT_ROOT/node/wasm/host_functions"
echo -e "${BLUE}Starting DHT service on port 8787...${NC}"
bunx wrangler dev --port 8787 &
DHT_PID=$!

# Wait for DHT service to be ready
echo -e "${YELLOW}Waiting for DHT service to be ready...${NC}"
sleep 5

# Clear all keys in the DHT database
echo -e "${YELLOW}Clearing DHT database...${NC}"
cd "$PROJECT_ROOT/node/wasm/host_functions"

# List all keys in the KV store
echo -e "${YELLOW}Listing all keys in DHT database...${NC}"
KEYS_JSON=$(bunx wrangler kv key list --binding=KV --local --preview 2>/dev/null)

if [ $? -eq 0 ] && [ ! -z "$KEYS_JSON" ]; then
    echo -e "${YELLOW}Found keys in DHT database, deleting them...${NC}"
    
    # Extract key names from JSON and delete them one by one
    echo "$KEYS_JSON" | jq -r '.[].name' | while read -r key; do
        if [ ! -z "$key" ]; then
            echo -e "${YELLOW}Deleting key: $key${NC}"
            bunx wrangler kv key delete "$key" --binding=KV --local --preview > /dev/null 2>&1
        fi
    done
    
    echo -e "${GREEN}✅ DHT database cleared${NC}"
else
    echo -e "${YELLOW}No keys found in DHT database or error listing keys${NC}"
fi

echo -e "${GREEN}✅ DHT service started (PID: $DHT_PID)${NC}"

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
elif [[ "${node_name}" == "WASM Node Self" ]]; then
    VITE_USE_ANVIL=true bunx vitest run wasm_node_self.test.ts --reporter=verbose
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
        elif [[ "${node_name}" == "WASM Node Self" ]]; then
            # Pink background for Self Node
            osascript -e "
            tell application \"Terminal\"
                set newTab to do script \"$temp_script\"
                set current settings of newTab to settings set \"Pro\"
                tell newTab
                    set background color to {10240, 0, 10240}
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
        elif [[ "${node_name}" == "WASM Node Self" ]]; then
            gnome-terminal --tab --title="WASM Node Self" --profile="Pink" -- bash -c "$temp_script; exec bash" 2>/dev/null || \
            xterm -bg '#330033' -fg '#ffffff' -e "bash $temp_script" 2>/dev/null || \
            konsole --profile Pink -e "bash $temp_script" 2>/dev/null
        else
            gnome-terminal -- bash -c "$temp_script; exec bash" 2>/dev/null || \
            xterm -e "bash $temp_script" 2>/dev/null || \
            konsole -e "bash $temp_script" 2>/dev/null
        fi
    else
        echo "Unsupported OS. Please run: $temp_script"
    fi
}

# Start simple HTTP server to serve relay-info.json
echo -e "${YELLOW}Starting HTTP server to serve relay-info.json...${NC}"
cd "$PROJECT_ROOT/integration-test/wasm-e2e-ts"
python3 -m http.server 8080 &
HTTP_PID=$!

if [[ "$LIVE" == "true" ]]; then
    # For live mode, just create static relay-info.json - no local relay node needed
    echo -e "${YELLOW}🌐 LIVE MODE: Creating static relay-info.json for production relay...${NC}"
    cat > "relay-info.json" <<EOF
{
  "peerId": "12D3KooWEmBqLAznmxDVdhRZV434uyEbHEzgpy2Q4WJx5fHtK46C",
  "multiAddr": "/dns4/vane-relay.vaneweb3.com/tcp/30333/ws/p2p/12D3KooWEmBqLAznmxDVdhRZV434uyEbHEzgpy2Q4WJx5fHtK46C",
  "host": "vane-relay.vaneweb3.com",
  "port": 30333,
  "ready": true
}
EOF
    echo -e "${GREEN}✅ Static relay-info.json created for LIVE mode${NC}"
    echo -e "${GREEN}✅ Ready to connect to production relay at vane-relay.vaneweb3.com${NC}"
else
    # For local mode, check if relay node is running and start if needed
    echo -e "${YELLOW}🏠 LOCAL MODE: Checking if local relay node is running...${NC}"
    if ! pgrep -f "start-relay" > /dev/null; then
        echo -e "${RED}⚠️  Local relay node is not running!${NC}"
        echo -e "${YELLOW}🏠 Starting local relay node...${NC}"
        
        # Start local relay node in background (using defaults)
        bun run start-relay &
        RELAY_PID=$!
        
        # Wait for the actual relay node to generate relay-info.json
        echo -e "${YELLOW}🏠 Waiting for local relay node to generate relay-info.json...${NC}"
        
        # Wait up to 30 seconds for relay-info.json to be created by the relay node
        for i in {1..30}; do
            if [ -f "relay-info.json" ]; then
                echo -e "${GREEN}✅ Relay info file generated by local relay after ${i} seconds${NC}"
                break
            fi
            echo -e "${YELLOW}⏳ Waiting for local relay to generate relay-info.json... (${i}/30)${NC}"
            sleep 1
        done
        
        if [ ! -f "relay-info.json" ]; then
            echo -e "${RED}❌ Local relay node failed to generate relay-info.json after 30 seconds${NC}"
            echo -e "${RED}Killing relay node and HTTP server, exiting...${NC}"
            kill $RELAY_PID 2>/dev/null
            kill $HTTP_PID 2>/dev/null
            exit 1
        fi
        
        # Give additional time for relay to be fully ready
        echo -e "${YELLOW}Giving local relay node additional time to be fully ready...${NC}"
        sleep 5
        
        echo -e "${GREEN}✅ Local relay node started (PID: $RELAY_PID)${NC}"
    else
        echo -e "${GREEN}✅ Local relay node is already running${NC}"
        
        # Check if relay-info.json exists for local mode
        if [ ! -f "relay-info.json" ]; then
            echo -e "${YELLOW}⚠️  Local relay node is running but relay-info.json is missing${NC}"
            echo -e "${YELLOW}🏠 Local relay node should have generated relay-info.json. This might cause connection issues.${NC}"
            echo -e "${YELLOW}Consider restarting the local relay node.${NC}"
        else
            echo -e "${GREEN}✅ Using relay-info.json generated by local relay node${NC}"
        fi
    fi
fi

# Step 3: Fund Solana accounts
echo -e "${YELLOW}Step 3: Funding Solana accounts...${NC}"
cd "$PROJECT_ROOT"
# if [ -f "scripts/fund-solana.sh" ]; then
#     echo -e "${BLUE}Running fund-solana.sh to create and fund Solana accounts...${NC}"
#     chmod +x scripts/fund-solana.sh
#     if ./scripts/fund-solana.sh; then
#         echo -e "${GREEN}✅ Solana accounts funded successfully${NC}"
        
#         # Copy token mint file to test directory
#         if [ -f "token_mint.txt" ]; then
#             cp token_mint.txt integration-test/wasm-e2e-ts/
#             echo -e "${GREEN}✅ Token mint file copied to test directory${NC}"
#         fi
        
#         # Copy token info file to test directory
#         if [ -f "token_info.txt" ]; then
#             cp token_info.txt integration-test/wasm-e2e-ts/
#             echo -e "${GREEN}✅ Token info file copied to test directory${NC}"
#         fi
#     else
#         echo -e "${RED}❌ Failed to fund Solana accounts. Continuing anyway...${NC}"
#     fi
# else
#     echo -e "${RED}⚠️  fund-solana.sh not found. Solana accounts may not be properly funded.${NC}"
# fi

# Step 4: Fund EVM accounts (Ethereum and BNB Chain)
echo -e "${YELLOW}Step 4: Funding EVM accounts...${NC}"
cd "$PROJECT_ROOT"
if [ -f "scripts/fund-evm.sh" ]; then
    echo -e "${BLUE}Running fund-evm.sh to create and fund EVM tokens...${NC}"
    chmod +x scripts/fund-evm.sh
    if ./scripts/fund-evm.sh; then
        echo -e "${GREEN}✅ EVM tokens funded successfully${NC}"
        
        # Copy EVM token info file to test directory
        if [ -f "evm_token_info.txt" ]; then
            cp evm_token_info.txt integration-test/wasm-e2e-ts/
            echo -e "${GREEN}✅ EVM token info file copied to test directory${NC}"
        fi
        
        # Extract token addresses and create simple text files for HTTP serving
        if [ -f "evm_token_info.txt" ]; then
            echo -e "${BLUE}Extracting EVM token addresses...${NC}"
            
            # Extract BNB Chain token address
            BNB_TOKEN=$(grep "Token Address:" evm_token_info.txt | head -1 | awk '{print $3}')
            if [ ! -z "$BNB_TOKEN" ]; then
                echo "$BNB_TOKEN" > integration-test/wasm-e2e-ts/bnb_token.txt
                echo -e "${GREEN}✅ BNB Chain token address: $BNB_TOKEN${NC}"
            fi
            
            # Extract Ethereum token address
            ETH_TOKEN=$(grep "Token Address:" evm_token_info.txt | tail -1 | awk '{print $3}')
            if [ ! -z "$ETH_TOKEN" ]; then
                echo "$ETH_TOKEN" > integration-test/wasm-e2e-ts/eth_token.txt
                echo -e "${GREEN}✅ Ethereum token address: $ETH_TOKEN${NC}"
            fi
        fi
    else
        echo -e "${RED}❌ Failed to fund EVM accounts. Continuing anyway...${NC}"
    fi
else
    echo -e "${RED}⚠️  fund-evm.sh not found. EVM accounts may not be properly funded.${NC}"
fi

echo ""
echo -e "${BLUE}Starting WASM nodes...${NC}"



# Start Node 2
start_node "WASM Node 2" "$BLUE"

# # Add delay between node starts
# echo -e "${YELLOW}Waiting 3 seconds before starting next node...${NC}"
# sleep 1

# # Start Malicious Node 3
# start_node "WASM Node 3 (Malicious)" "$RED"

# # Add delay between node starts to prevent connection conflicts
# echo -e "${YELLOW}Waiting 3 seconds before starting next node...${NC}"
# sleep 5

# Start Self Node
start_node "WASM Node Self" "$PINK"

# Start Node 1
start_node "WASM Node 1" "$GREEN"

echo ""
echo -e "${GREEN}✅ All WASM nodes have been started in separate terminals!${NC}"
echo -e "${YELLOW}Each terminal will show the logs for its respective node.${NC}"
echo ""
echo -e "${BLUE}To stop all nodes:${NC}"
echo -e "1. Close the terminal windows manually"
echo -e "2. Or run: pkill -f 'vitest run'"
echo -e "3. To stop relay node: pkill -f 'start-relay'"
echo -e "4. To stop HTTP server: pkill -f 'http.server'"
echo -e "5. To stop DHT service: pkill -f 'wrangler dev'"
echo ""

# Cleanup function
cleanup() {
    echo -e "${YELLOW}Cleaning up...${NC}"
    if [ ! -z "$DHT_PID" ]; then
        echo -e "${YELLOW}Stopping DHT service (PID: $DHT_PID)...${NC}"
        kill $DHT_PID 2>/dev/null
    fi
    if [ ! -z "$HTTP_PID" ]; then
        echo -e "${YELLOW}Stopping HTTP server (PID: $HTTP_PID)...${NC}"
        kill $HTTP_PID 2>/dev/null
    fi
    if [ ! -z "$RELAY_PID" ]; then
        echo -e "${YELLOW}Stopping relay node (PID: $RELAY_PID)...${NC}"
        kill $RELAY_PID 2>/dev/null
    fi
    echo -e "${GREEN}✅ Cleanup completed${NC}"
    exit 0
}

# Set up cleanup trap
trap cleanup SIGINT SIGTERM

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
