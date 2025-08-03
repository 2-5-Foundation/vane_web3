import { config } from 'dotenv';
import { createClient } from 'redis';
import { spawn } from 'bun';
import path from 'path';
import fs from 'fs';
import { parse, stringify } from 'yaml';
import { encodeAddress } from '@polkadot/util-crypto';
import WebSocket from 'ws';
import crypto from 'crypto';

// Simple logger
const logger = {
    info: (...args) => console.log(new Date().toISOString(), '[INFO]', ...args),
    error: (...args) => console.error(new Date().toISOString(), '[ERROR]', ...args),
    warn: (...args) => console.warn(new Date().toISOString(), '[WARN]', ...args)
};

// Load environment variables
config({ path: path.join(import.meta.dirname, '..', '.env') });

const {
    REDIS_URL,
    CLOUDFLARED_TUNNEL_NAME,
    CLOUDFLARED_TUNNEL_CREDENTIALS_FILE
} = process.env;

class VaneMonitor {
    constructor() {
        this.validateConfig();
        this.configPath = path.join(import.meta.dirname, 'cloudflare.config.yaml');
        this.redisClient = null;
        this.runningContainers = new Map();
        this.keepAliveInterval = null;
    }

    validateConfig() {
        if (!REDIS_URL || !CLOUDFLARED_TUNNEL_NAME || !CLOUDFLARED_TUNNEL_CREDENTIALS_FILE) {
            throw new Error('Missing required environment variables');
        }
        console.log(REDIS_URL)
    }

    async initRedis() {
        this.redisClient = createClient({ url: REDIS_URL });
        
        // Suppress routine connection logs, only log actual errors
        this.redisClient.on('error', (err) => {
            if (err.code !== 'ECONNREFUSED' && err.code !== 'ENOTFOUND') {
                logger.error('Redis Client Error:', err);
            }
        });
        this.redisClient.on('disconnect', () => logger.info('Redis disconnected (will reconnect automatically)'));
        this.redisClient.on('reconnecting', () => logger.info('Redis reconnecting...'));
        
        await this.redisClient.connect();
        logger.info('Connected to Redis');
        
        // Start minimal keep-alive
        this.startKeepAlive();
    }

    startKeepAlive() {
        this.keepAliveInterval = setInterval(async () => {
            try {
                if (this.redisClient?.isOpen) {
                    await this.redisClient.ping();
                }
            } catch (error) {
                // Ignore ping errors - connection will be handled by ensureRedisConnection
            }
        }, 60000); // 60 seconds
    }

    stopKeepAlive() {
        if (this.keepAliveInterval) {
            clearInterval(this.keepAliveInterval);
            this.keepAliveInterval = null;
        }
    }

    // Ensure Redis connection is active
    async ensureRedisConnection(context = 'general') {
        try {
            if (!this.redisClient.isOpen) {
                logger.warn(`Redis connection lost during ${context}, attempting to reconnect...`);
                await this.redisClient.connect();
                logger.info('Redis reconnected successfully');
            }
            
            // Test the connection with a ping
            await this.redisClient.ping();
            return true;
        } catch (error) {
            // Always log connection failures when processing entries
            if (context === 'entry-processing') {
                logger.error(`Redis connection failed during ${context}:`, error);
            }
            
            // Try to recreate the client if connection is completely broken
            try {
                if (this.redisClient) {
                    await this.redisClient.disconnect().catch(() => {});
                }
                
                this.redisClient = createClient({ url: REDIS_URL });
                this.redisClient.on('error', (err) => {
                    if (err.code !== 'ECONNREFUSED' && err.code !== 'ENOTFOUND') {
                        logger.error('Redis Client Error:', err);
                    }
                });
                this.redisClient.on('disconnect', () => logger.info('Redis disconnected (will reconnect automatically)'));
                this.redisClient.on('reconnecting', () => logger.info('Redis reconnecting...'));
                
                await this.redisClient.connect();
                await this.redisClient.ping();
                
                logger.info(`Redis client recreated and connected during ${context}`);
                return true;
            } catch (recreateError) {
                logger.error(`Failed to recreate Redis connection during ${context}:`, recreateError);
                return false;
            }
        }
    }

    // Generate a valid hostname from hash using Polkadot encoding
    generateHostname(hash) {
        try {
            // Convert hash to bytes (remove 0x prefix)
            const hashBytes = new Uint8Array(Buffer.from(hash.replace('0x', ''), 'hex'));
            
            // Use Polkadot's encodeAddress to get a clean, URL-safe string
            // Using prefix 0 for Polkadot mainnet format
            const encoded = encodeAddress(hashBytes, 0).toLowerCase();
            
            // Create hostname - encoded address is already URL-safe
            const hostname = `${encoded}.vaneweb3.com`;
            
            logger.info('Generated hostname:', { hash, encoded, hostname });
            return hostname;
        } catch (error) {
            logger.error('Failed to generate hostname with encodeAddress, falling back to simple method:', error);
            
            // Fallback to simple method if encodeAddress fails
            const cleanHash = hash.replace('0x', '').substring(0, 12).toLowerCase();
            return `vane-${cleanHash}.vaneweb3.com`;
        }
    }

    // Test WebSocket connection to verify setup is working
    async testWebSocket(hostname, timeoutMs = 15000) {
        return new Promise((resolve) => {
            logger.info(`Testing WebSocket connection to: ${hostname}`);
            
            const ws = new WebSocket(`wss://${hostname}`);
            const timeout = setTimeout(() => {
                ws.close();
                logger.error(`WebSocket connection timeout for ${hostname}`);
                resolve(false);
            }, timeoutMs);

            ws.onopen = () => {
                clearTimeout(timeout);
                ws.close();
                logger.info(`WebSocket connection PASSED for ${hostname}`);
                resolve(true);
            };

            ws.onerror = (error) => {
                clearTimeout(timeout);
                ws.close();
                logger.error(`WebSocket connection FAILED for ${hostname}:`, error.message);
                resolve(false);
            };
        });
    }

    // Start Docker container
    async startDockerContainer(accounts, hash) {
        const randomId = Math.floor(Math.random() * 100000) + 10000;
        const containerName = `vane-${hash.substring(2, 8)}-${randomId}`;
        const rpcPort = Math.floor(Math.random() * (65535 - 49152 + 1) + 49152);

        logger.info('Starting container:', { containerName, rpcPort, hash });

        const dockerCommand = [
            'docker', 'run', '-d',
            '--name', containerName,
            '--network=bridge',
            '-p', `${rpcPort}:${rpcPort}`,
            'vane_web3_app',
            '--port', rpcPort.toString(),
            '--redis-url', REDIS_URL,
            '--account-profile-hash', hash,
            '--accounts', `"${accounts.map(acc => `${acc.address}:${acc.network}`).join(',')}"`
        ];

        const proc = spawn(dockerCommand, {
            stderr: 'pipe',
            stdout: 'pipe'
        });

        const stderr = await new Response(proc.stderr).text();
        if (stderr) {
            throw new Error(`Docker error: ${stderr}`);
        }

        const stdout = await new Response(proc.stdout).text();
        const containerId = stdout.trim();

        // Wait longer for container and application to be fully ready
        logger.info('Waiting for container and application to fully start...');
        await Bun.sleep(10000); // Increased from 3s to 10s

        logger.info('Container started:', { containerId, rpcPort });
        return { containerId, rpcPort };
    }

    // Extract P2P info from container logs
    async extractP2PInfo(containerId, timeoutMs = 30000) {
        const startTime = Date.now();

        while (Date.now() - startTime < timeoutMs) {
            try {
                const proc = Bun.spawn(['docker', 'exec', containerId, 'cat', '/app/vane.log']);
                const logs = await new Response(proc.stdout).text();

                const p2pMatch = logs.match(/\[INFO\] listening to p2p url: ([a-zA-Z0-9\/\.:]+)/);
                if (p2pMatch) {
                    const p2pUrl = p2pMatch[1];
                    const peerId = p2pUrl.split('p2p/')[1];
                    return { peerId, multiAddr: p2pUrl };
                }
            } catch (error) {
                logger.error('Error reading container logs:', error);
            }
            await Bun.sleep(1000);
        }

        throw new Error('Failed to extract P2P info within timeout');
    }

    // Setup Cloudflare DNS
    async setupCloudflare(hostname, port) {
        try {
            // Create/update config
            let config = {
                tunnel: CLOUDFLARED_TUNNEL_NAME,
                'credentials-file': CLOUDFLARED_TUNNEL_CREDENTIALS_FILE,
                ingress: []
            };

            if (fs.existsSync(this.configPath)) {
                const existingConfig = parse(fs.readFileSync(this.configPath, 'utf8'));
                config.ingress = existingConfig.ingress || [];
            }

            // Remove existing entry for this hostname
            config.ingress = config.ingress.filter(entry => entry.hostname !== hostname);
            
            // Add new entry
            config.ingress.push({
                hostname: hostname,
                service: `ws://localhost:${port}`
            });

            // Ensure catch-all rule is last
            config.ingress = config.ingress.filter(entry => entry.service !== 'http_status:404');
            config.ingress.push({ service: 'http_status:404' });

            // Write config
            fs.writeFileSync(this.configPath, stringify(config));
            logger.info('Updated Cloudflare config:', { hostname, port });

            // Route DNS
            await this.routeDNS(hostname);
            
            // Restart cloudflared
            await this.restartCloudflared();

            return true;
        } catch (error) {
            logger.error('Failed to setup Cloudflare:', error);
            throw error;
        }
    }

    async routeDNS(hostname) {
        const proc = await Bun.spawn(['cloudflared', 'tunnel', 'route', 'dns', CLOUDFLARED_TUNNEL_NAME, hostname]);
        const exitCode = await proc.exited;
        
        if (exitCode !== 0) {
            throw new Error(`Failed to route DNS for ${hostname}`);
        }
        
        logger.info('DNS routed successfully:', hostname);
    }

    async restartCloudflared() {
        try {
            // Kill existing process
            await Bun.spawn(['pkill', 'cloudflared']).exited;
            await Bun.sleep(2000); // Wait for process to fully terminate

            // Start new process
            const proc = Bun.spawn(['cloudflared', 'tunnel', '--config', this.configPath, 'run'], {
                stdout: 'ignore',
                stderr: 'ignore'
            });

            // Give it time to start and establish tunnel connections
            logger.info('Waiting for cloudflared to start and establish connections...');
            await Bun.sleep(20000); // 20 seconds total

            // Verify it's running
            const checkProc = await Bun.spawn(['pgrep', 'cloudflared']);
            const isRunning = await checkProc.exited === 0;

            if (!isRunning) {
                throw new Error('Cloudflared failed to start');
            }

            logger.info('Cloudflared restarted successfully');
            
        } catch (error) {
            logger.error('Failed to restart cloudflared:', error);
            throw error;
        }
    }

    // Test the complete setup with DNS propagation wait
    async testSetup(hostname) {
        logger.info('Testing setup for:', hostname);
        
        // Wait for DNS propagation first
        logger.info('Waiting for DNS propagation...');
        await Bun.sleep(60000); // Wait 1 minute for DNS to propagate
        
        // Quick WebSocket connection test - 10 seconds total
        const maxAttempts = 2;
        const waitTime = 5000; // 5 seconds between attempts
        
        for (let attempt = 1; attempt <= maxAttempts; attempt++) {
            logger.info(`WebSocket test attempt ${attempt}/${maxAttempts} for ${hostname}`);
            
            if (await this.testWebSocket(hostname)) {
                logger.info('Setup test PASSED for:', hostname);
                return true;
            }
            
            if (attempt < maxAttempts) {
                logger.info(`WebSocket connection failed, waiting ${waitTime/1000}s before retry...`);
                await Bun.sleep(waitTime);
            }
        }

        // Total time: 1 minute + ~10 seconds
        logger.error(`WebSocket connection test failed after ${maxAttempts} attempts (after 1 minute DNS wait)`);
        throw new Error(`WebSocket connection test failed after ${maxAttempts} attempts for ${hostname}`);
    }

    // Cleanup resources
    async cleanup(containerId, hostname, hash) {
        try {
            if (containerId) {
                await Bun.spawn(['docker', 'stop', containerId]).exited;
                await Bun.spawn(['docker', 'rm', containerId]).exited;
                logger.info('Container cleaned up:', containerId);
            }

            if (hostname) {
                // Remove DNS route
                try {
                    await Bun.spawn(['cloudflared', 'tunnel', 'route', 'dns', 'delete', hostname]).exited;
                    logger.info('DNS route cleaned up:', hostname);
                } catch (error) {
                    logger.warn('DNS cleanup failed:', error.message);
                }

                // Remove from config
                if (fs.existsSync(this.configPath)) {
                    const config = parse(fs.readFileSync(this.configPath, 'utf8'));
                    config.ingress = config.ingress.filter(entry => entry.hostname !== hostname);
                    fs.writeFileSync(this.configPath, stringify(config));
                    logger.info('Config cleaned up for:', hostname);
                }
            }

            this.runningContainers.delete(hash);
        } catch (error) {
            logger.error('Cleanup error:', error);
        }
    }

    // Process a single queue entry
    async processQueueEntry(hash, accounts) {
        const startTime = Date.now();
        let containerId = null;
        let hostname = null;

        try {
            logger.info('Processing queue entry:', { hash, accounts });

            // Generate valid hostname
            hostname = this.generateHostname(hash);
            logger.info('Generated hostname:', hostname);

            // Start Docker container
            const { containerId: cId, rpcPort } = await this.startDockerContainer(accounts, hash);
            containerId = cId;

            // Extract P2P info
            const { peerId, multiAddr } = await this.extractP2PInfo(containerId);

            // Setup Cloudflare
            await this.setupCloudflare(hostname, rpcPort);

            // Test the complete setup
            await this.testSetup(hostname);

            // Ensure Redis connection before storing data - specify context for proper error logging
            if (!(await this.ensureRedisConnection('entry-processing'))) {
                throw new Error('Failed to establish Redis connection for storing data');
            }

            // Store in Redis
            const accountProfile = {
                peerId,
                multiAddr,
                accounts,
                rpc: hostname
            };

            await this.redisClient.hSet('ACCOUNT_PROFILE', hash, JSON.stringify(accountProfile));
            
            // Only remove from QUEUE on success
            await this.redisClient.hDel('QUEUE', hash);
            logger.info('Removed entry from QUEUE after successful processing:', hash);

            // Track container
            this.runningContainers.set(hash, {
                containerId,
                hostname,
                accounts,
                startTime: Date.now()
            });

            const totalTime = Date.now() - startTime;
            logger.info('Successfully processed queue entry:', { hash, hostname, totalTimeMs: totalTime, totalTimeSeconds: (totalTime / 1000).toFixed(2) });
            return hostname;

        } catch (error) {
            const totalTime = Date.now() - startTime;
            logger.error('Failed to process queue entry:', { error: error.message, hash, totalTimeMs: totalTime, totalTimeSeconds: (totalTime / 1000).toFixed(2) });
            
            // On failure, remove from ACCOUNT_PROFILE if it exists
            if (await this.ensureRedisConnection('entry-processing')) {
                const exists = await this.redisClient.hExists('ACCOUNT_PROFILE', hash);
                if (exists) {
                    await this.redisClient.hDel('ACCOUNT_PROFILE', hash);
                    logger.info('Removed failed entry from ACCOUNT_PROFILE:', hash);
                }
            }
            
            await this.cleanup(containerId, hostname, hash);
            throw error;
        }
    }

    // Main monitoring loop - Enhanced error handling
    async monitorQueue() {
        logger.info('Starting VaneWeb3 monitor');

        while (true) {
            try {
                // Ensure Redis connection before checking queue
                if (!(await this.ensureRedisConnection('monitoring'))) {
                    logger.warn('Redis connection failed during monitoring, retrying in 5 seconds...');
                    await Bun.sleep(5000); // Increased delay
                    continue;
                }

                const queueEntries = await this.redisClient.hGetAll('QUEUE');
                
                if (Object.keys(queueEntries).length > 0) {
                    logger.info(`Found ${Object.keys(queueEntries).length} entries in queue`);
                }
                
                for (const [hash, accountsJson] of Object.entries(queueEntries)) {
                    try {
                        const accounts = JSON.parse(accountsJson);
                        await this.processQueueEntry(hash, accounts);
                    } catch (error) {
                        logger.error('Error processing entry:', { hash, error: error.message });
                    }
                }

                await Bun.sleep(2000);
            } catch (error) {
                // Check if it's a connection error
                if (error.code === 'ECONNREFUSED' || error.message.includes('connection')) {
                    logger.info('Connection issue in monitor loop, will retry...');
                } else {
                    logger.error('Monitor loop error:', error);
                }
                await Bun.sleep(5000);
            }
        }
    }

    async start() {
        await this.initRedis();
        await this.monitorQueue();
    }
}

// Start the monitor
async function start() {
    try {
        const monitor = new VaneMonitor();
        await monitor.start();
    } catch (error) {
        logger.error('Failed to start monitor:', error);
        process.exit(1);
    }
}

start();