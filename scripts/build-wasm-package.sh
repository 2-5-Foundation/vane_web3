#!/bin/bash

# Build script for WASM package with host functions

set -e

echo "Building WASM package with host functions..."

# Get the project root directory (parent of scripts directory)
PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$PROJECT_ROOT/node/wasm"

# Step 1: Compile to wasm32-unknown-unknown
echo "Step 1: Compiling to wasm32-unknown-unknown..."
cargo build --target wasm32-unknown-unknown --release

if [ $? -ne 0 ]; then
    echo "❌ Failed to compile WASM node to wasm32-unknown-unknown"
    exit 1
fi

echo "✅ WASM compilation successful"

# Step 2: Build the WASM package with wasm-pack
echo "Step 2: Building WASM package with wasm-pack..."
wasm-pack build --target web --out-dir pkg

if [ $? -ne 0 ]; then
    echo "❌ Failed to build WASM package with wasm-pack"
    exit 1
fi

echo "✅ WASM package built successfully"

# Add host functions import to the generated JS file
echo "Adding host functions import to vane_wasm_node.js..."
if [ -f pkg/vane_wasm_node.js ]; then
    # Create a temporary file with the import at the top
    echo "import { hostFunctions } from './host_functions/main.ts';" > pkg/vane_wasm_node_temp.js
    cat pkg/vane_wasm_node.js >> pkg/vane_wasm_node_temp.js
    mv pkg/vane_wasm_node_temp.js pkg/vane_wasm_node.js
    echo "Host functions import added successfully"
else
    echo "Warning: vane_wasm_node.js not found"
fi

# Copy the entire host_functions directory
echo "Copying host_functions directory..."
rm -rf pkg/host_functions
cp -R host_functions pkg/

# Move pkg directory into vane_lib FIRST
echo "Moving pkg directory into vane_lib..."
if [ -d "vane_lib" ]; then
    rm -rf vane_lib/pkg
    mv pkg vane_lib/
    echo "✅ pkg directory moved into vane_lib"
    
    # Now build vane_lib (after pkg is inside it)
    echo "Building vane_lib..."
    cd vane_lib
    if [ -f package.json ]; then
        echo "Building vane_lib..."
        bun install
        bun run build
        echo "✅ vane_lib built successfully"
        cd ..
    else
        echo "Warning: vane_lib package.json not found"
        cd ..
    fi
else
    echo "Warning: vane_lib directory not found"
fi

# Update vane_lib's package.json to include pkg in files
echo "Updating vane_lib package.json..."
if [ -f "vane_lib/package.json" ]; then
    node -e "
    const fs = require('fs');
    const pkg = JSON.parse(fs.readFileSync('vane_lib/package.json', 'utf8'));
    
    // Add files field with pkg included
    pkg.files = ['pkg', 'dist', 'main.ts', 'networking.ts', 'primitives.ts'];
    
    // Set main entry point
    pkg.main = 'main.ts';
    pkg.types = 'main.ts';
    
    fs.writeFileSync('vane_lib/package.json', JSON.stringify(pkg, null, 2));
    "
    echo "✅ vane_lib package.json updated"
else
    echo "Warning: vane_lib package.json not found"
fi

# Update vane_lib/pkg package.json to include only essential files
echo "Updating vane_lib/pkg package.json..."
if [ -f "vane_lib/pkg/package.json" ]; then
    node -e "
    const fs = require('fs');
    const pkg = JSON.parse(fs.readFileSync('vane_lib/pkg/package.json', 'utf8'));

    pkg.files = [
      'wasm_node_bg.wasm',
      'wasm_node.js',
      'wasm_node.d.ts',
      'vane_wasm_node.js',
      'host_functions'
    ];

    pkg.main = 'wasm_node.js';
    pkg.types = 'wasm_node.d.ts';

    // Add dependencies from host_functions if present
    try {
      const hostPkg = JSON.parse(fs.readFileSync('host_functions/package.json', 'utf8'));
      pkg.dependencies = { ...pkg.dependencies, ...hostPkg.dependencies };
    } catch (e) {
      // optional
    }

    fs.writeFileSync('vane_lib/pkg/package.json', JSON.stringify(pkg, null, 2));
    "
    echo "✅ vane_lib/pkg package.json updated"
else
    echo "Warning: vane_lib/pkg/package.json not found"
fi

echo "Package built successfully!"
echo "Final vane_lib structure:"
ls -la vane_lib/
echo ""
echo "vane_lib/pkg contents:"
ls -la vane_lib/pkg/
