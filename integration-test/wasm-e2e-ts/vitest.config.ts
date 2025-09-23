import { defineConfig } from 'vitest/config'
import { resolve } from 'path'

export default defineConfig({
  server: {
    fs: {
      // Allow serving files from the WASM pkg directory
      allow: [
        // Current directory
        '.',
        // WASM package directory
        '../../node/wasm/pkg',
        // vane_lib package directory
        '../../node/wasm/vane_lib'
      ]
    }
  },
  assetsInclude: ['**/*.wasm'],
  test: {
    hookTimeout: 900000,
    testTimeout: 900000,
    browser: {
      enabled: true,
      provider: 'playwright',
      headless: true,
      instances: [
        {
          browser: 'chromium'
        }
      ]
    }
  }
})
