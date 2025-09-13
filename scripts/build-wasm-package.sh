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

# Copy entire host functions directory
echo "Copying host_functions directory..."
rm -rf pkg/host_functions
cp -R host_functions pkg/host_functions

# Create index.ts if it doesn't exist
if [ ! -f pkg/index.ts ]; then
    echo "Creating index.ts..."
    cat > pkg/index.ts << 'EOF'
// Export the WASM module
export * from './wasm_node.js';

// Export the host functions
export * from './host_functions/main.ts';

// Re-export the WASM module as default for convenience
export { default as WasmNode } from './wasm_node.js';

// Re-export host functions as default for convenience
export { default as hostFunctions } from './host_functions/main.ts';
EOF
fi

# Update package.json to include only essential files
echo "Updating package.json..."
node -e "
const fs = require('fs');
const pkg = JSON.parse(fs.readFileSync('pkg/package.json', 'utf8'));

pkg.files = [
  'wasm_node_bg.wasm',
  'wasm_node.js', 
  'wasm_node.d.ts',
  'vane_wasm_node.js',
  'index.ts',
  'host_functions/**'
];

pkg.main = 'index.ts';

// Add dependencies from host_functions
const hostPkg = JSON.parse(fs.readFileSync('host_functions/package.json', 'utf8'));
pkg.dependencies = { ...pkg.dependencies, ...hostPkg.dependencies };

fs.writeFileSync('pkg/package.json', JSON.stringify(pkg, null, 2));
"

echo "Package built successfully!"
echo "Files included:"
ls -la pkg/
