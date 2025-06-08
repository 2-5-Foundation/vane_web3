import { config } from 'dotenv';
import { createClient } from 'redis';
import { spawn } from 'bun';
import ngrok from 'ngrok';
import path from 'path';
import crypto from 'crypto';
import { hostname } from 'os';

// Simple logger implementation for Bun
const logger = {
    info: (...args) => console.log(new Date().toISOString(), '[INFO]', ...args),
    error: (...args) => console.error(new Date().toISOString(), '[ERROR]', ...args),
    warn: (...args) => console.warn(new Date().toISOString(), '[WARN]', ...args)
};

// Load environment variables
config({ path: path.join(import.meta.dirname, '..', '.env') });

const {
    REDIS_URL,
    REDIS_PASSWORD,
    REDIS_HOST
} = process.env;

// Configure Redis
const redisClient = createClient({
    url: REDIS_URL,
    host: REDIS_HOST,
    password: REDIS_PASSWORD
});

redisClient.on('error', (err) => logger.error('Redis Client Error:', err));

// Track running containers
const runningContainers = new Map();

class VaneMonitor {
    constructor() {
        this.validateConfig();
    }

    validateConfig() {
        if (!REDIS_URL || !REDIS_PASSWORD) {
            throw new Error('Missing required Redis environment variables');
        }
    }

    async cleanup(containerId, hash) {
        try {
            // Stop and remove Docker container
            if (containerId) {
                await Bun.spawn(['docker', 'stop', containerId]).exited;
                await Bun.spawn(['docker', 'rm', containerId]).exited;
            }

            // Clean up Redis entries
            if (hash) {
                await redisClient.hDel('ACCOUNT_PROFILE', hash);
                await redisClient.hDel('QUEUE', hash);
                // Get all addresses from ACCOUNT_LINK that point to this hash
                const addresses = await redisClient.hGetAll('ACCOUNT_LINK');
                for (const [address, addrHash] of Object.entries(addresses)) {
                    if (addrHash === hash) {
                        await redisClient.hDel('ACCOUNT_LINK', address);
                    }
                }
            }
        } catch (error) {
            logger.error('Error during cleanup:', error);
        }
    }

    async startDockerInstance(accounts, hash) {
        try {
            // Generate unique container name
            const randomId = Math.floor(Math.random() * (887689 - 787 + 23) + 49152);
            const containerName = `vane-${hash}-${randomId}`;
            // Generate random port (avoiding well-known ports)
            const rpcPort = Math.floor(Math.random() * (65535 - 49152 + 1) + 49152);

            // Start Docker container with the specified port
            logger.info('spawning container with params:', {
                containerName,
                rpcPort,
                hash
            });

            const dockerCommand = [
                'docker', 'run', '-d',
                '--name', containerName,
                '--network=bridge',
                '-p', `${rpcPort}:${rpcPort}`,
                'vane_web3_app',
                '--port', rpcPort.toString(),
                '--redis-url', REDIS_URL,
                '--redis-host', REDIS_HOST,
                '--account-profile-hash', hash,
                '--accounts', accounts.map(acc => `${acc.address}:${acc.network}`).join(',')
            ];
            
            const proc = spawn(dockerCommand, {
                stderr: 'pipe',
                stdout: 'pipe'
            });

            // Get the output from the process
            const stderr = await new Response(proc.stderr).text();
            if (stderr) {
                logger.error('Docker stderr:', stderr);
                throw new Error(`Docker error: ${stderr}`);
            }

            const stdout = await new Response(proc.stdout).text();
            logger.info('Docker stdout:', stdout);

            await Bun.sleep(5000);

            const containerId = stdout.trim();
            logger.info(`containerId: ${containerId}`);
            logger.info('Started Docker container', {
                containerId,
                hash,
                containerName,
                rpcPort
            });

            return {
                id: containerId,
                rpcPort
            };

        } catch (error) {
            logger.error('Error in startDockerInstance', { error, hash });
            throw error;
        }
    }

    async extractRpcUrl(containerId, timeout = 30000) {
        const startTime = Date.now();

        while (Date.now() - startTime < timeout) {
            try {
                const proc = Bun.spawn(['docker', 'exec', containerId.id, 'cat', '/app/vane.log']);
                const logs = await new Response(proc.stdout).text();

                // Extract RPC URL
                const rpcMatch = logs.match(/\[INFO\] listening to rpc url: ([0-9.]+:[0-9]+)/);
                // Extract P2P URL which contains both peer ID and multi-address
                const p2pMatch = logs.match(/\[INFO\] listening to p2p url: ([a-zA-Z0-9\/\.:]+)/);

                if (rpcMatch && p2pMatch) {
                    const p2pUrl = p2pMatch[1];
                    // Extract peer ID from p2p URL (after "p2p/")
                    const peerId = p2pUrl.split('p2p/')[1];
                    
                    return {
                        rpcUrl: rpcMatch[1],
                        peerId,
                        multiAddr: p2pUrl
                    };
                }
            } catch (error) {
                logger.error('Error reading file from container:', error);
            }

            await Bun.sleep(1000);
        }

        throw new Error('Failed to extract required information within timeout period');
    }

    async createNgrokTunnel(port) {
        try {
            const url = await ngrok.connect({
                addr: port,
                proto: 'http'
            });
            logger.info('Created ngrok tunnel', { port, url });
            return url.replace('https://', '');
        } catch (error) {
            logger.error('Failed to create ngrok tunnel', { error, port });
            throw error;
        }
    }

    async processQueueEntry(hash, accounts) {
        let containerId = null;
        try {
            // Start Docker container
            containerId = await this.startDockerInstance(accounts, hash);
            await Bun.sleep(3000);
            
            // Get RPC URL and create ngrok tunnel
            const { rpcUrl, peerId, multiAddr } = await this.extractRpcUrl(containerId);
            const ngrokUrl = await this.createNgrokTunnel(containerId.rpcPort);

            // Construct RedisAccountProfile
            const accountProfile = {
                peerId,
                multiAddr,
                accounts: accounts,
                rpc: ngrokUrl
            };

            // Store in Redis
            await redisClient.hSet('ACCOUNT_PROFILE', hash, JSON.stringify(accountProfile));

            // Store container info
            runningContainers.set(hash, {
                containerId,
                rpcUrl,
                publicRpcUrl: ngrokUrl,
                addresses: accounts,
                startTime: Date.now()
            });

            logger.info('Successfully processed queue entry', { hash, accounts, rpcUrl });
            return ngrokUrl;
        } catch (error) {
            logger.error('Failed to process queue entry', { error, hash, accounts });
            await this.cleanup(containerId?.id, hash);
            throw error;
        }
    }

    async monitorQueue() {
        logger.info('Starting VaneWeb3 monitor');

        while (true) {
            try {
                // Get all entries from QUEUE
                const queueEntries = await redisClient.hGetAll('QUEUE');
                
                for (const [hash, accounts] of Object.entries(queueEntries)) {
                    try {
                        logger.info('Processing queue entry', { hash, accounts });
                        
                        // json decode accounts
                        /** @type {{address: string, network: string}[]} */
                        const parsedAccounts = JSON.parse(accounts);
                        
                        await this.processQueueEntry(hash, parsedAccounts);
                        
                        // Remove from queue after successful processing
                        await redisClient.hDel('QUEUE', hash);
                        
                        logger.info('Successfully processed and removed from queue', { hash });
                    } catch (error) {
                        logger.error('Error processing queue entry', { error, hash });
                        await this.cleanup(null, hash);
                    }
                }

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
    monitor.monitorQueue();
} catch (error) {
    logger.error('Failed to start monitor', { error });
    process.exit(1);
}