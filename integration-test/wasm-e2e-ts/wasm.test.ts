  import { describe, test, expect, beforeAll, afterAll, it } from 'vitest'

describe('WASM NODE & RELAY NODE INTERACTIONS', () => {
  beforeAll(() => {
     expect(window).toBeDefined()  
     expect(document).toBeDefined()
     expect(WebAssembly).toBeDefined()
     expect(window.fetch).toBeDefined()
     expect(window.WebSocket).toBeDefined()
     expect(window.crypto).toBeDefined()
     expect(window.localStorage).toBeDefined()
  })

  afterAll(() => {
    console.log('After all tests')
  })

  it('set up wasm with host functions work', () => {
   
  })
})