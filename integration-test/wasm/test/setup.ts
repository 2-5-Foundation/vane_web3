import { beforeAll } from 'vitest'
import { execSync } from 'child_process'
import { existsSync } from 'fs'
import { resolve } from 'path'

// Browser API polyfills for WASM/OPFS compatibility
function setupBrowserAPIs() {
  console.log('🌐 Setting up browser API polyfills...')
  
  // Ensure window object exists (should be provided by jsdom)
  if (typeof window === 'undefined') {
    throw new Error('Window object not available - jsdom environment not properly configured')
  }

  // Setup navigator.storage for OPFS
  if (!window.navigator) {
    (window as any).navigator = {}
  }

  // Mock Origin Private File System (OPFS) APIs
  if (!window.navigator.storage) {
    // Mock WritableFileStream for OPFS
    const mockWritableFileStream = {
      write: async (data: any) => {},
      write_at_cursor_pos: async (data: any) => {},
      close: async () => {},
      abort: async () => {},
    }

    // Mock FileHandle for OPFS
    const mockFileHandle = {
      createWritable: async () => mockWritableFileStream,
      createWritableWithOptions: async (options: any) => mockWritableFileStream,
      read: async () => new Uint8Array(0), // Return empty file initially
      getFile: async () => new File([], 'mock-file'),
    }

    // Mock DirectoryHandle for OPFS  
    const mockDirectoryHandle = {
      getFileHandle: async (name: string) => mockFileHandle,
      getFileHandleWithOptions: async (name: string, options: any) => mockFileHandle,
      getDirectoryHandle: async (name: string) => mockDirectoryHandle,
      removeEntry: async (name: string) => {},
      keys: async function* () {},
      values: async function* () {},
      entries: async function* () {},
    }

    // Mock StorageManager with OPFS support
    ;(window.navigator as any).storage = {
      getDirectory: async () => mockDirectoryHandle,
      estimate: async () => ({ quota: 1000000, usage: 0 }),
      persist: async () => true,
      persisted: async () => true,
    }

    console.log('📁 OPFS APIs mocked successfully')
  }

  // Mock global window reference for WASM
  if (typeof globalThis !== 'undefined') {
    ;(globalThis as any).window = window
  }

  console.log('✅ Browser API polyfills configured')
}

beforeAll(async () => {
  console.log('🧪 Setting up WASM integration tests...')
  
  // Setup browser environment first
  setupBrowserAPIs()
  
  // Check if WASM file exists, if not build it
  const wasmPath = resolve(__dirname, '../../../../node/wasm/pkg/wasm_node_bg.wasm')
  
  if (!existsSync(wasmPath)) {
    console.log('🔨 WASM file not found, building...')
    try {
      execSync('bun run build:wasm', { 
        cwd: __dirname, 
        stdio: 'inherit' 
      })
      console.log('✅ WASM build completed')
    } catch (error) {
      console.error('❌ Failed to build WASM:', error)
      throw error
    }
  } else {
    console.log('✅ WASM file already exists')
  }
  
  console.log('🚀 Test setup complete!')
})
