import { defineConfig } from 'vitest/config';
import { nodePolyfills } from 'vite-plugin-node-polyfills';
import { resolve } from 'path';

export default defineConfig({
  // Vite config (Vitest will use this)
  plugins: [
    nodePolyfills({
      protocolImports: true,
    }),
  ],
  server: {
    fs: {
      allow: [
        '.',                               // current dir
        '../../node/wasm/pkg',             // WASM pkg
        '../../node/wasm/vane_lib',        // vane_lib
      ],
    },
    hmr: false,  // Disable HMR to prevent reloads during tests
  },
  assetsInclude: ['**/*.wasm'],
  define: {
    global: 'globalThis',
  },
  resolve: {
    alias: {
      buffer: 'buffer',
      process: 'process/browser',
    },
  },
  optimizeDeps: {
    include: [
      'buffer',
      'process',
      'crypto',
      'vite-plugin-node-polyfills/shims/buffer',
      'vite-plugin-node-polyfills/shims/global',
      'vite-plugin-node-polyfills/shims/process',
      '@solana/web3.js',
      '@solana/spl-token',
      '@solana/spl-token-metadata',
    ],
  },

  // Vitest config
  test: {
    hookTimeout: 900000,
    testTimeout: 900000,
    setupFiles: ['./vitest.setup.ts'],   // <-- add this file
    browser: {
      enabled: true,
      provider: 'playwright',
      headless: true,
      instances: [{ 
        browser: 'chromium'
      }],
    },
  },
});
