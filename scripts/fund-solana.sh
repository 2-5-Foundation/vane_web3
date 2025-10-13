#!/bin/bash

# Solana Token-2022 Setup Script
# This script creates a token, sets up accounts, and mints tokens

set -e  # Exit on error

echo "🚀 Solana Token-2022 Setup Script"
echo "=================================="

# Configuration
TOKEN_NAME="SPLTEST"
TOKEN_SYMBOL="SPLT"
TOKEN_URI=""
DECIMALS=6
MINT_AMOUNT=1500

# Your sender address
SENDER="8tGT3GGgnv3HUFiPoUtDpb8ebWbYKZt7ij9NJmiPkZWb"

# Set to local validator
echo "📡 Setting network to localhost..."
solana config set --url http://localhost:8899

# Generate keypairs
echo ""
echo "🔑 Generating keypairs..."
solana-keygen new --outfile payer.json --no-bip39-passphrase --force
solana-keygen new --outfile receiver.json --no-bip39-passphrase --force

# Get addresses
PAYER=$(solana-keygen pubkey payer.json)
RECEIVER=$(solana-keygen pubkey receiver.json)

echo "Payer:    $PAYER"
echo "Sender:   $SENDER"
echo "Receiver: $RECEIVER"

# Set payer as default
solana config set --keypair payer.json

# Fund accounts with SOL (local validator = unlimited)
echo ""
echo "💰 Funding accounts with SOL..."
solana airdrop 1000 $PAYER
solana airdrop 100 $SENDER
solana airdrop 50 $RECEIVER

# Create token
echo ""
echo "🪙 Creating Token-2022 with metadata..."
spl-token create-token \
  --program-id TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb \
  --decimals $DECIMALS \
  --enable-metadata > token_output.txt

# Extract mint address
MINT=$(grep "Address:" token_output.txt | awk '{print $2}')
echo "Token Mint: $MINT"

# Initialize metadata
echo ""
echo "🏷️  Initializing metadata..."
spl-token initialize-metadata $MINT "$TOKEN_NAME" "$TOKEN_SYMBOL" "$TOKEN_URI"

# Create token accounts
echo ""
echo "👛 Creating token accounts..."
SENDER_ATA_OUTPUT=$(spl-token create-account $MINT --owner $SENDER --fee-payer payer.json)
SENDER_ATA=$(echo "$SENDER_ATA_OUTPUT" | grep "Creating account" | awk '{print $3}')
echo "Sender token account: $SENDER_ATA"

RECEIVER_ATA_OUTPUT=$(spl-token create-account $MINT --owner $RECEIVER --fee-payer payer.json)
RECEIVER_ATA=$(echo "$RECEIVER_ATA_OUTPUT" | grep "Creating account" | awk '{print $3}')
echo "Receiver token account: $RECEIVER_ATA"

# Mint tokens to sender
echo ""
echo "💵 Minting $MINT_AMOUNT tokens to sender..."
spl-token mint $MINT $MINT_AMOUNT $SENDER_ATA --fee-payer payer.json

# Display balances
echo ""
echo "✅ Setup complete!"
echo "=================="
echo "Token Mint: $MINT"
echo "Sender Balance: $(spl-token balance $MINT --owner $SENDER 2>/dev/null || echo '0') $TOKEN_SYMBOL"
echo "Receiver Balance: $(spl-token balance $MINT --owner $RECEIVER 2>/dev/null || echo '0') $TOKEN_SYMBOL"
echo ""
echo "📄 Keypair files saved:"
echo "  - payer.json (fee payer)"
echo "  - sender.json"
echo "  - receiver.json"
echo ""
echo "🔧 To transfer tokens:"
echo "solana config set --keypair sender.json"
echo "spl-token transfer $MINT <AMOUNT> <RECEIVER_TOKEN_ACCOUNT> --fund-recipient"

# Save info to file
cat > token_info.txt <<EOF
Token Mint: $MINT
Token Name: $TOKEN_NAME
Token Symbol: $TOKEN_SYMBOL
Decimals: $DECIMALS

Payer Address: $PAYER
Sender Address: $SENDER
Receiver Address: $RECEIVER

Network: Localhost (http://localhost:8899)
Program: Token-2022 (TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb)
EOF

# Save just the token mint for easy reading by tests
echo "$MINT" > token_mint.txt

echo "💾 Token info saved to token_info.txt"
echo "🪙 Token mint saved to token_mint.txt: $MINT"