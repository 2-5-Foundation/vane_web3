// Import the WASM module
import init from './pkg/vane_wasm_node.js';

// Export everything from other files
// export * from './account_finance';
export * from './api';
export * from './primitives';
// export * from './wallet_engine';

// Re-export WASM module and initialization
export { init };

// Main initialization function that sets up everything
export async function initializeVaneNode() {
  await init();
  return init;
}