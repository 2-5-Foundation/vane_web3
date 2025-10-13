import { Buffer } from 'buffer';
import process from 'process';
import { webcrypto } from 'crypto';

// Set Buffer polyfill
if (!globalThis.Buffer) {
  (globalThis as any).Buffer = Buffer;
}

// Set process polyfill
if (!globalThis.process) {
  (globalThis as any).process = process;
}

// Set crypto polyfill only if not already available
if (!globalThis.crypto) {
  try {
    Object.defineProperty(globalThis, 'crypto', {
      value: webcrypto,
      writable: true,
      configurable: true
    });
  } catch (error) {
    // If we can't set crypto, it's likely already available
    console.warn('Could not set crypto polyfill:', error);
  }
}
