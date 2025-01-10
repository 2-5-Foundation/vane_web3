import { config } from 'dotenv';
import Airtable from 'airtable';
import { spawn } from 'bun';
import ngrok from 'ngrok';

import { readFile } from 'fs/promises';

// Simple logger implementation for Bun
const logger = {
    info: (...args) => console.log(new Date().toISOString(), '[INFO]', ...args),
    error: (...args) => console.error(new Date().toISOString(), '[ERROR]', ...args),
    warn: (...args) => console.warn(new Date().toISOString(), '[WARN]', ...args)
};

// Load environment variables
config();

const {
    AIRTABLE_API_KEY,
    AIRTABLE_BASE_ID,
    AIRTABLE_TABLE_ID
} = process.env;


// Configure Airtable
const airtable = new Airtable({ apiKey: AIRTABLE_API_KEY }).base(AIRTABLE_BASE_ID);

// Track running containers
const runningContainers = new Map();

class VaneMonitor {
    constructor() {
        this.validateConfig();
    }

    validateConfig() {
        if (!AIRTABLE_API_KEY || !AIRTABLE_BASE_ID || !AIRTABLE_TABLE_ID) {
            throw new Error('Missing required environment variables');
        }
    }

    async startDockerInstance(twitter, ethAddress) {
        try {
            // Generate unique container name
            const containerName = `vane-${twitter.replace('@', '').toLowerCase()}`;

            // Generate random port (avoiding well-known ports)
            const rpcPort = Math.floor(Math.random() * (65535 - 49152 + 1) + 49152);

            // Start Docker container with the specified port
            const proc = spawn(['docker', 'run', '-d',
                '--name', containerName,
                '--network=bridge',
                '-p', `${rpcPort}:${rpcPort}`,    // Map our random port
                'vane_web3_app',
                '--port', rpcPort.toString()     // Pass port as argument
            ], {
                stderr: 'pipe',
                stdout: 'pipe'
            });

            const containerId = await new Response(proc.stdout).text();

            // Store the port with the container info
            const containerInfo = {
                id: containerId.trim(),
                rpcPort
            };

            logger.info('Started Docker container', {
                containerId: containerInfo.id,
                twitter,
                containerName,
                rpcPort
            });

            return containerInfo;

        } catch (error) {
            logger.error('Error in startDockerInstance', { error, twitter });
            throw error;
        }
    }

    async  extractRpcUrl(containerId, timeout = 30000) {
        const startTime = Date.now();

        while (Date.now() - startTime < timeout) {
            try {
                // Use docker exec to cat the file content
                const proc = Bun.spawn(['docker', 'exec', containerId.id, 'cat', '/app/vane.log']);
                const logs = await new Response(proc.stdout).text();


                // Adjusted regex to match the exact format in your log file
                const match = logs.match(/\[INFO\] listening to rpc url: ([0-9.]+:[0-9]+)/);

                if (match) {
                    return match[1];
                }

            } catch (error) {
                console.error('Error reading file from container:', error);
            }

            await Bun.sleep(1000);
        }

        throw new Error('Failed to extract RPC URL within timeout period');
    }

    async createNgrokTunnel(port) {
        try {
            const url = await ngrok.connect({
                addr: port,
                proto: 'http'
            });
            logger.info('Created ngrok tunnel', { port, url });
            return url.replace('https://', ''); // Remove protocol as we'll specify it later
        } catch (error) {
            logger.error('Failed to create ngrok tunnel', { error, port });
            throw error;
        }
    }

    async registerWithNode(rpcUrl, twitter, ethAddress) {
        try {
            const hostRpcUrl = rpcUrl.replace(/172\.[0-9]+\.[0-9]+\.[0-9]+/, 'localhost');
            const response = await fetch(`http://${hostRpcUrl}`, {
                method: 'POST',
                headers: {
                    'Content-Type': 'application/json',
                },
                body: JSON.stringify({
                    jsonrpc: '2.0',
                    id: 1,
                    method: 'register',
                    params: [
                        twitter,           // name parameter
                        ethAddress,       // account_id parameter
                        'Ethereum',       // network parameter
                        `wss://${rpcUrl}` // rpc parameter
                    ]
                })
            });

            const data = await response.json();

            if (data.error) {
                throw new Error(`RPC error: ${JSON.stringify(data.error)}`);
            }

            logger.info('Successfully registered with node', { twitter, ethAddress, rpcUrl });
            return data.result;
        } catch (error) {
            logger.error('Failed to register with node', { error, twitter, ethAddress });
            throw error;
        }
    }

    async processNewEntry(record) {
        const { Twitter, EthAddress } = record.fields;

        if (!Twitter || !EthAddress) {
            logger.warn('Missing required fields', { record });
            return;
        }

        try {
            // Start Docker container
            const containerId = await this.startDockerInstance(Twitter, EthAddress);

            // Wait for RPC endpoint to become available
            await Bun.sleep(3000)
            const rpcUrl = await this.extractRpcUrl(containerId);

            const ngrokUrl = await this.createNgrokTunnel(containerId.rpcPort);

            // Register with the node
            await this.registerWithNode(ngrokUrl, Twitter, EthAddress);

            // Store container info
            runningContainers.set(Twitter, {
                containerId,
                rpcUrl,
                publicRpcUrl: ngrokUrl,
                EthAddress,
                startTime: Date.now()
            });

            logger.info('Successfully processed new entry', { Twitter, EthAddress, rpcUrl });
        } catch (error) {
            logger.error('Failed to process new entry', { error, Twitter, EthAddress });
        }
    }

    async monitorTable() {
        logger.info('Starting Vane monitor');

        while (true) {
            try {
                // Fetch all records from Airtable
                const records = await airtable(AIRTABLE_TABLE_ID)
                    .select({
                        filterByFormula: 'AND({Twitter}, {EthAddress}, NOT({processed}))'
                    })
                    .all();

                // Process new records
                for (const record of records) {
                    await this.processNewEntry(record);

                    // Mark as processed in Airtable
                    await airtable(AIRTABLE_TABLE_ID).update(record.id, {
                        processed: true
                    });
                }

                // Wait before next check using Bun.sleep
                await Bun.sleep(30000);
            } catch (error) {
                logger.error('Error in monitor loop', { error });
                await Bun.sleep(60000);
            }
        }
    }
}

// Start the monitor
try {
    const monitor = new VaneMonitor();
    monitor.monitorTable();
} catch (error) {
    logger.error('Failed to start monitor', { error });
    process.exit(1);
}