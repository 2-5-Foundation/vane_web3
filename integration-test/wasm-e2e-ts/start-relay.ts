#!/usr/bin/env bun

import { spawn, ChildProcess } from 'child_process';
import { writeFileSync, unlinkSync, existsSync } from 'fs';
import { join, resolve } from 'path';
import { fileURLToPath } from 'url';
import { dirname } from 'path';

/**
 * Script to start the relay node for testing
 * This runs outside the browser environment and can use Node.js APIs
 * Saves relay info to a JSON file for browser tests to consume
 */

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

const RELAY_HOST = process.env.RELAY_HOST || '127.0.0.1';
const RELAY_PORT = process.env.RELAY_PORT || '30333';
const RELAY_INFO_FILE = join(__dirname, 'relay-info.json');

interface RelayInfo {
  peerId: string;
  multiAddr: string;
  host: string;
  port: number;
  ready: boolean;
  timestamp: string;
}

function startRelayNode(): ChildProcess {
  console.log(`🚀 Starting relay node on ${RELAY_HOST}:${RELAY_PORT}`);
  
  // Adjust the path to the relay binary relative to this script
  const relayBinary = resolve(__dirname, '../../target/release/vane_web3_app');
  
  const relayProcess = spawn(relayBinary, [
    'relay-node',
    '--dns', RELAY_HOST,
    '--port', RELAY_PORT
  ], {
    stdio: ['pipe', 'pipe', 'pipe']
  });

  let localPeerId: string | null = null;
  let relayReady = false;

  relayProcess.stdout?.on('data', (data: Buffer) => {
    const output = data.toString().trim();
    console.log(`[Relay Node]: ${output}`);
    
    // Capture peer ID for reference
    const peerIdMatch = output.match(/local_peer_id=([A-Za-z0-9]+)/);
    if (peerIdMatch && !localPeerId) {
      localPeerId = peerIdMatch[1];
      const multiAddr = `/ip6/::1/tcp/${RELAY_PORT}/ws/p2p/${localPeerId}`;
      console.log(`🎯 Local Peer ID: ${localPeerId}`);
      console.log(`🔗 Relay MultiAddr: ${multiAddr}`);
      
      // Save relay info to file for browser tests
      const relayInfo: RelayInfo = {
        peerId: localPeerId,
        multiAddr: multiAddr,
        host: RELAY_HOST,
        port: parseInt(RELAY_PORT),
        ready: false,
        timestamp: new Date().toISOString()
      };
      
      try {
        writeFileSync(RELAY_INFO_FILE, JSON.stringify(relayInfo, null, 2));
        console.log(`💾 Relay info saved to ${RELAY_INFO_FILE}`);
      } catch (error) {
        console.error(`❌ Failed to save relay info: ${(error as Error).message}`);
      }
    }
    
    // Confirm relay is ready and update the file
    if (output.includes('NewListenAddr') && localPeerId && !relayReady) {
      relayReady = true;
      console.log(`✅ Relay node is ready and listening`);
      console.log(`📡 You can now run your browser tests`);
      
      // Update relay info file to mark as ready
      try {
        const relayInfo: RelayInfo = {
          peerId: localPeerId,
          multiAddr: `/ip6/::1/tcp/${RELAY_PORT}/ws/p2p/${localPeerId}`,
          host: RELAY_HOST,
          port: parseInt(RELAY_PORT),
          ready: true,
          timestamp: new Date().toISOString()
        };
        writeFileSync(RELAY_INFO_FILE, JSON.stringify(relayInfo, null, 2));
        console.log(`✅ Relay info updated - ready status: true`);
      } catch (error) {
        console.error(`❌ Failed to update relay info: ${(error as Error).message}`);
      }
    }
  });

  relayProcess.stderr?.on('data', (data: Buffer) => {
    console.error(`[Relay Node Error]: ${data.toString().trim()}`);
  });

  relayProcess.on('error', (error: Error) => {
    console.error('❌ Failed to start relay node:', error);
    process.exit(1);
  });

  relayProcess.on('exit', (code: number | null, signal: NodeJS.Signals | null) => {
    // Clean up relay info file
    try {
      if (existsSync(RELAY_INFO_FILE)) {
        unlinkSync(RELAY_INFO_FILE);
        console.log(`🗑️ Cleaned up ${RELAY_INFO_FILE}`);
      }
    } catch (error) {
      console.error(`❌ Failed to clean up relay info file: ${(error as Error).message}`);
    }

    if (code !== 0) {
      console.error(`❌ Relay node exited with code ${code}, signal ${signal}`);
      process.exit(1);
    } else {
      console.log('👋 Relay node stopped gracefully');
    }
  });

  // Handle shutdown gracefully
  const cleanup = () => {
    console.log('\n🛑 Stopping relay node...');
    relayProcess.kill('SIGTERM');
  };

  process.on('SIGINT', cleanup);
  process.on('SIGTERM', cleanup);

  return relayProcess;
}

// Start the relay node if this script is run directly
if (import.meta.url === `file://${process.argv[1]}`) {
  startRelayNode();
}

export { startRelayNode };
export type { RelayInfo };
