#!/bin/bash

# EVM Token Setup Script (BEP20 & ERC20)
# Creates tokens on BNB Chain and Ethereum, then funds an account

set -e  # Exit on error

echo "🚀 EVM Token Setup Script (BEP20 & ERC20)"
echo "=========================================="

# Configuration
TOKEN_NAME="TestToken"
TOKEN_SYMBOL="TTK"
DECIMALS=18
INITIAL_SUPPLY="1000000"  # 1 million tokens
MINT_AMOUNT="1500"        # Amount to mint to account

# Network configs
BNB_RPC="http://127.0.0.1:8555"
ETH_RPC="http://127.0.0.1:8545"
TO_FUND="0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"

# ERC20 Token Contract (Solidity)
cat > Token.sol <<'EOF'
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract Token {
    string public name;
    string public symbol;
    uint8 public decimals;
    uint256 public totalSupply;
    
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;
    
    event Transfer(address indexed from, address indexed to, uint256 value);
    event Approval(address indexed owner, address indexed spender, uint256 value);
    
    constructor(string memory _name, string memory _symbol, uint8 _decimals, uint256 _initialSupply) {
        name = _name;
        symbol = _symbol;
        decimals = _decimals;
        totalSupply = _initialSupply * 10**uint256(_decimals);
        balanceOf[msg.sender] = totalSupply;
        emit Transfer(address(0), msg.sender, totalSupply);
    }
    
    function transfer(address _to, uint256 _value) public returns (bool success) {
        require(balanceOf[msg.sender] >= _value, "Insufficient balance");
        balanceOf[msg.sender] -= _value;
        balanceOf[_to] += _value;
        emit Transfer(msg.sender, _to, _value);
        return true;
    }
    
    function approve(address _spender, uint256 _value) public returns (bool success) {
        allowance[msg.sender][_spender] = _value;
        emit Approval(msg.sender, _spender, _value);
        return true;
    }
    
    function transferFrom(address _from, address _to, uint256 _value) public returns (bool success) {
        require(_value <= balanceOf[_from], "Insufficient balance");
        require(_value <= allowance[_from][msg.sender], "Insufficient allowance");
        balanceOf[_from] -= _value;
        balanceOf[_to] += _value;
        allowance[_from][msg.sender] -= _value;
        emit Transfer(_from, _to, _value);
        return true;
    }
    
    function mint(address _to, uint256 _amount) public {
        totalSupply += _amount;
        balanceOf[_to] += _amount;
        emit Transfer(address(0), _to, _amount);
    }
}
EOF

echo "📄 Token contract created"

# Check if foundry is installed
if ! command -v forge &> /dev/null; then
    echo "❌ Foundry not found. Please install it:"
    echo "curl -L https://foundry.paradigm.xyz | bash"
    echo "foundryup"
    exit 1
fi

echo ""
echo "🔨 Compiling contract..."
forge init --no-git --force temp_project 2>/dev/null || true
mv Token.sol temp_project/src/
cd temp_project
forge build

echo ""
echo "========================================"
echo "🟡 DEPLOYING TO BNB CHAIN (BSC)"
echo "========================================"

# Deploy to BNB Chain
echo "Deploying BEP20 token..."
BNB_DEPLOY_OUTPUT=$(forge create src/Token.sol:Token \
    --rpc-url $BNB_RPC \
    --private-key 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80 \
    --constructor-args "$TOKEN_NAME" "$TOKEN_SYMBOL" $DECIMALS $INITIAL_SUPPLY)

BNB_TOKEN=$(echo "$BNB_DEPLOY_OUTPUT" | grep "Deployed to:" | awk '{print $3}')
echo "✅ BEP20 Token deployed at: $BNB_TOKEN"

# Mint tokens to account on BNB Chain
echo "Minting $MINT_AMOUNT tokens to $TO_FUND on BNB Chain..."
MINT_AMOUNT_WEI=$(echo "$MINT_AMOUNT * 10^$DECIMALS" | bc)
cast send $BNB_TOKEN \
    "mint(address,uint256)" $TO_FUND $MINT_AMOUNT_WEI \
    --rpc-url $BNB_RPC \
    --private-key 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80

# Check balance on BNB Chain
BNB_BALANCE=$(cast call $BNB_TOKEN "balanceOf(address)(uint256)" $TO_FUND --rpc-url $BNB_RPC)
BNB_BALANCE_FORMATTED=$(echo "scale=2; $BNB_BALANCE / 10^$DECIMALS" | bc)
echo "✅ BNB Chain balance: $BNB_BALANCE_FORMATTED $TOKEN_SYMBOL"

echo ""
echo "========================================"
echo "🔵 DEPLOYING TO ETHEREUM"
echo "========================================"

# Deploy to Ethereum
echo "Deploying ERC20 token..."
ETH_DEPLOY_OUTPUT=$(forge create src/Token.sol:Token \
    --rpc-url $ETH_RPC \
    --private-key 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80 \
    --constructor-args "$TOKEN_NAME" "$TOKEN_SYMBOL" $DECIMALS $INITIAL_SUPPLY)

ETH_TOKEN=$(echo "$ETH_DEPLOY_OUTPUT" | grep "Deployed to:" | awk '{print $3}')
echo "✅ ERC20 Token deployed at: $ETH_TOKEN"

# Mint tokens to account on Ethereum
echo "Minting $MINT_AMOUNT tokens to $TO_FUND on Ethereum..."
cast send $ETH_TOKEN \
    "mint(address,uint256)" $TO_FUND $MINT_AMOUNT_WEI \
    --rpc-url $ETH_RPC \
    --private-key 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80

# Check balance on Ethereum
ETH_BALANCE=$(cast call $ETH_TOKEN "balanceOf(address)(uint256)" $TO_FUND --rpc-url $ETH_RPC)
ETH_BALANCE_FORMATTED=$(echo "scale=2; $ETH_BALANCE / 10^$DECIMALS" | bc)
echo "✅ Ethereum balance: $ETH_BALANCE_FORMATTED $TOKEN_SYMBOL"

# Cleanup
cd ..
rm -rf temp_project

# Save token info
cat > evm_token_info.txt <<EOF
Token Name: $TOKEN_NAME
Token Symbol: $TOKEN_SYMBOL
Decimals: $DECIMALS
Initial Supply: $INITIAL_SUPPLY

=== BNB CHAIN (BSC) ===
RPC: $BNB_RPC
Token Address: $BNB_TOKEN
Balance of $TO_FUND: $BNB_BALANCE_FORMATTED $TOKEN_SYMBOL

=== ETHEREUM ===
RPC: $ETH_RPC
Token Address: $ETH_TOKEN
Balance of $TO_FUND: $ETH_BALANCE_FORMATTED $TOKEN_SYMBOL

=== FUNDED ACCOUNT ===
Address: $TO_FUND
EOF

echo ""
echo "✅ SETUP COMPLETE!"
echo "=================="
echo ""
echo "🟡 BNB Chain (BEP20)"
echo "   Token: $BNB_TOKEN"
echo "   Balance: $BNB_BALANCE_FORMATTED $TOKEN_SYMBOL"
echo ""
echo "🔵 Ethereum (ERC20)"
echo "   Token: $ETH_TOKEN"
echo "   Balance: $ETH_BALANCE_FORMATTED $TOKEN_SYMBOL"
echo ""
echo "📝 Info saved to evm_token_info.txt"
echo ""
echo "🔧 To check balances:"
echo "cast call $BNB_TOKEN 'balanceOf(address)(uint256)' $TO_FUND --rpc-url $BNB_RPC"
echo "cast call $ETH_TOKEN 'balanceOf(address)(uint256)' $TO_FUND --rpc-url $ETH_RPC"