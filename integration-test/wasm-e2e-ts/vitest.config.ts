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
        '../../node/wasm/pkg'
      ]
    }
  },
  assetsInclude: ['**/*.wasm'],
  test: {
    hookTimeout:20000,
    browser: {
      enabled: true,
      provider: 'playwright',
      headless: true,
      instances: [
        {
          browser: 'chromium'
        }
      ],
      providerOptions: {
        playwright: {
          screenshot: 'off',
          video: 'off',
          trace: 'off'
        }
      }
    }
  }
})
