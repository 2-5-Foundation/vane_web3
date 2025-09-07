#!/bin/bash

# Build script for WASM package with host functions

set -e

echo "Building WASM package with host functions..."

# Build the WASM module
echo "Building WASM module..."
wasm-pack build --target web --out-dir pkg

# Add host functions import to the generated JS file
echo "Adding host functions import to vane_wasm_node.js..."
if [ -f pkg/vane_wasm_node.js ]; then
    # Create a temporary file with the import at the top
    echo "import { hostFunctions } from './main.ts';" > pkg/vane_wasm_node_temp.js
    cat pkg/vane_wasm_node.js >> pkg/vane_wasm_node_temp.js
    mv pkg/vane_wasm_node_temp.js pkg/vane_wasm_node.js
    echo "Host functions import added successfully"
else
    echo "Warning: vane_wasm_node.js not found"
fi

# Copy required files from host functions
echo "Copying host function files..."
cp host_functions/main.ts pkg/
cp host_functions/cryptography.ts pkg/
cp host_functions/networking.ts pkg/
cp host_functions/logging.ts pkg/

# Create index.ts if it doesn't exist
if [ ! -f pkg/index.ts ]; then
    echo "Creating index.ts..."
    cat > pkg/index.ts << 'EOF'
// Export the WASM module
export * from './wasm_node.js';

// Export the host functions
export * from './main.ts';

// Re-export the WASM module as default for convenience
export { default as WasmNode } from './wasm_node.js';

// Re-export host functions as default for convenience
export { default as hostFunctions } from './main.ts';
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
  'main.ts',
  'cryptography.ts',
  'networking.ts',
  'logging.ts'
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
