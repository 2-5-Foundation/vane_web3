import { config } from 'dotenv';
import Airtable from 'airtable';
import { spawn } from 'bun';
import ngrok from 'ngrok';
import path from 'path';



// Simple logger implementation for Bun
const logger = {
    info: (...args) => console.log(new Date().toISOString(), '[INFO]', ...args),
    error: (...args) => console.error(new Date().toISOString(), '[ERROR]', ...args),
    warn: (...args) => console.warn(new Date().toISOString(), '[WARN]', ...args)
};

// Load environment variables
config({ path: path.join(import.meta.dirname, '..', '.env') });

const {
    AIRTABLE_TOKEN,
    BASE_ID,
    TABLE_ID,
} = process.env;


// Configure Airtable
const airtable = new Airtable({ apiKey: AIRTABLE_TOKEN }).base(BASE_ID);

// Track running containers
const runningContainers = new Map();

class VaneMonitor {
    constructor() {
        this.validateConfig();
    }

    validateConfig() {
        if (!AIRTABLE_TOKEN || !BASE_ID || !TABLE_ID) {
            throw new Error('Missing required environment variables');
        }
    }

    async startDockerInstance(social, airtable_record_id) {
        try {
            // Generate unique container name
            const containerName = `vane-${social.toLowerCase()}`;

            // Generate random port (avoiding well-known ports)
            const rpcPort = Math.floor(Math.random() * (65535 - 49152 + 1) + 49152);

            // Start Docker container with the specified port
            const proc = spawn(['docker', 'run', '-d',
                '--name', containerName,
                '--network=bridge',
                '-p', `${rpcPort}:${rpcPort}`,    // Map our random port
                'vane_web3_app',
                '--port', rpcPort.toString(),     // Pass port as argument
                '--airtable_record_id', airtable_record_id 
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
                social,
                containerName,
                rpcPort
            });

            return containerInfo;

        } catch (error) {
            logger.error('Error in startDockerInstance', { error, social });
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

    async registerWithNode(rpcUrl, social, address, network) {
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
                        social,           // name parameter
                        address,       // account_id parameter
                        network,       // network parameter
                        `wss://${rpcUrl}` // rpc parameter
                    ]
                })
            });

            const data = await response.json();

            if (data.error) {
                throw new Error(`RPC error: ${JSON.stringify(data.error)}`);
            }

            logger.info('Successfully registered with node', { social, address, network, rpcUrl });
            return data.result;
        } catch (error) {
            logger.error('Failed to register with node', { error, social, address, network });
            throw error;
        }
    }

    async processNewEntry(record) {
        const { accountId1, accountId2, accountId3, accountId4, peerId, multiAddr, rpc, social} = record.fields;
        const {address, network} = JSON.parse(accountId1);
        if (!accountId1 || !social) {
            logger.warn('Missing required fields', { record });
            return;
        }

        try {
            // Start Docker container
            const containerId = await this.startDockerInstance(social);

            // Wait for RPC endpoint to become available
            await Bun.sleep(3000)
            const rpcUrl = await this.extractRpcUrl(containerId);

            const ngrokUrl = await this.createNgrokTunnel(containerId.rpcPort);

            // Register with the node
            await this.registerWithNode(ngrokUrl, social, address, network);

            // Store container info
            runningContainers.set(social, {
                containerId,
                rpcUrl,
                publicRpcUrl: ngrokUrl,
                address,
                network,
                startTime: Date.now()
            });

            logger.info('Successfully processed new entry', { social, address, network, rpcUrl });
        } catch (error) {
            logger.error('Failed to process new entry', { error, social, address, network });
        }
    }

    async monitorTable() {
        logger.info('Starting VaneWeb3 monitor');

        while (true) {
            try {
                // Fetch all records from Airtable
                const records = await airtable(TABLE_ID)
                    .select({
                        filterByFormula: 'AND({accountId1}, {accountId2}, {accountId3}, {accountId4}, {peerId}, {multiAddr}, {rpc}, {social}, NOT({processed}))'
                    })
                    .all();

                // Process new records
                for (const record of records) {
                    await this.processNewEntry(record);

                    // Mark as processed in Airtable
                    await airtable(TABLE_ID).update(record.id, {
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