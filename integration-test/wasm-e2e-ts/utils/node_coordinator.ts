/**
 * Node Coordinator - binds a logical node id and converts logs → events
 * No renames to your API.
 */

import { hostLogging } from '../../../node/wasm/host_functions/logging.js';

export enum NODE_EVENTS {
  NODE_STARTED = 'node_started',
  NODE_READY = 'node_ready',
  PEER_CONNECTED = 'peer_connected',
  PEER_DISCONNECTED = 'Connection closed with peer',
  RESERVATION_ACCEPTED = 'reservation_accepted',
  LISTENING_ESTABLISHED = 'listening_established',
  TRANSACTION_INITIATED = 'initiated sending transaction',
  SENDER_CONFIRMED = 'successfully initially verified sender and receiver',
  TRANSACTION_SUBMITTED = 'propagated initiated transaction',
  TRANSACTION_SUBMITTED_FAILED = 'tx submission failed',
  TRANSACTION_SUBMITTED_PASSED = 'tx submission passed',
  TRANSACTION_SENT = 'request sent to peer',
  TRANSACTION_RECEIVED = 'received message',
  RECEIVER_CONFIRMED = 'receiver confirmation passed',
  SENDER_RECEIVED_RESPONSE = 'propagating txn msg as response',
  RECEIVER_CONFIRMATION_FAILED = 'receiver confirmation failed',
  RECEIVER_NOT_REGISTERED = 'DHT returned no address',
  SENDER_CONFIRMATION_FAILED = 'non original sender signed',
  TRANSACTION_SUCCESS = 'tx submission passed',
  TRANSACTION_FAILED = 'tx submission failed',
  ERROR = 'error',
}

export interface LogEntry {
  level: number;
  target: string;
  message: string;
  module_path?: string;
  file?: string;
  line?: number;
}

export class NodeCoordinator {
  private static instance: NodeCoordinator;

  private registeredNodes: Set<string> = new Set();
  private wasmLogger: any = null;

  private isMonitoring = false;
  private pollHandle: number | null = null;
  private processedLogs: Set<string> = new Set();

  private nodeReadinessStatus: Map<
    string,
    { peerConnected: boolean; reservationAccepted: boolean; listeningEstablished: boolean }
  > = new Map();

  private nodeId: string | null = null;
  private pendingReadiness = {
    peerConnected: false,
    reservationAccepted: false,
    listeningEstablished: false,
  };

  // Event bus
  private handlers: Map<string, Set<(data: any) => void>> = new Map();
  private history: Map<string, any[]> = new Map();

  private constructor() {}

  static getInstance(): NodeCoordinator {
    if (!NodeCoordinator.instance) {
      NodeCoordinator.instance = new NodeCoordinator();
    }
    return NodeCoordinator.instance;
  }

  setWasmLogger(logger: any): void {
    this.wasmLogger = logger;

    // Directly set callback on instance
    this.wasmLogger.setLogCallback((entry: LogEntry) => {
      this.convertLogToEvent(entry);
    });

    this.startLogMonitoring();
  }

  private ensureLogger(): void {
    if (this.wasmLogger) return;
    try {
      const instance = (hostLogging as any).getLogInstance?.();
      if (instance) {
        this.setWasmLogger(instance);
      }
    } catch {}
  }

  registerNode(nodeId: string): void {
    this.ensureLogger();
    this.nodeId = nodeId;
    this.registeredNodes.add(nodeId);
    this.nodeReadinessStatus.set(nodeId, {
      peerConnected: false,
      reservationAccepted: false,
      listeningEstablished: false,
    });

    // Apply pending readiness
    if (this.pendingReadiness.peerConnected) {
      this.updateNodeReadiness(nodeId, 'peerConnected');
    }
    if (this.pendingReadiness.reservationAccepted) {
      this.updateNodeReadiness(nodeId, 'reservationAccepted');
    }
    if (this.pendingReadiness.listeningEstablished) {
      this.updateNodeReadiness(nodeId, 'listeningEstablished');
    }

    // Process backlog
    this.processNewLogs();

    this.emitEvent(NODE_EVENTS.NODE_STARTED, { nodeId });
  }

  markNodeReady(nodeId: string): void {
    this.emitEvent(NODE_EVENTS.NODE_READY, { nodeId, ready: true });
  }

  private startLogMonitoring(): void {
    if (this.isMonitoring || !this.wasmLogger) return;
    this.isMonitoring = true;
    console.log('🔄 Starting log monitoring...');

    setTimeout(() => {
      if (this.nodeId) {
        this.updateNodeReadiness(this.nodeId, 'peerConnected');
        this.updateNodeReadiness(this.nodeId, 'reservationAccepted');
        this.updateNodeReadiness(this.nodeId, 'listeningEstablished');
      }
    }, 3000);

    this.processNewLogs();
    this.pollHandle = (globalThis as any).setInterval(() => this.processNewLogs(), 200) as unknown as number;
  }

  stop(): void {
    if (this.pollHandle !== null) (globalThis as any).clearInterval(this.pollHandle);
    this.pollHandle = null;
    this.isMonitoring = false;
  }

  private processNewLogs(): void {
    if (!this.wasmLogger) {
      this.ensureLogger();
      if (!this.wasmLogger) return;
    }
    const allLogs = this.getAllLogs();

    for (const log of allLogs) {
      const key = `${log.target}-${log.message}-${log.line}`;
      if (this.processedLogs.has(key)) continue;
      this.processedLogs.add(key);
      this.convertLogToEvent(log);
    }
  }

  private convertLogToEvent(log: LogEntry): void {
    // Map log messages to events
    if (log.message.includes('Connected to peer')) {
      this.emitEvent(NODE_EVENTS.PEER_CONNECTED, { log });
    } else if (log.message.includes('Connection closed with peer') || 
               log.message.includes('peer disconnected') ||
               log.message.includes('connection closed') ||
               log.message.includes('disconnected from peer')) {
      this.emitEvent(NODE_EVENTS.PEER_DISCONNECTED, { log });
    } else if (log.message.includes('reservation request accepted')) {
      this.emitEvent(NODE_EVENTS.RESERVATION_ACCEPTED, { log });
    } else if (log.message.includes('Listening on:')) {
      this.emitEvent(NODE_EVENTS.LISTENING_ESTABLISHED, { log });
    } else if (log.message.includes('initiated sending transaction')) {
      this.emitEvent(NODE_EVENTS.TRANSACTION_INITIATED, { log });
    } else if (log.message.includes('received message')) {
      this.emitEvent(NODE_EVENTS.TRANSACTION_RECEIVED, { log });
    } else if (log.message.includes('propagating txn msg as response')) {
      this.emitEvent(NODE_EVENTS.SENDER_RECEIVED_RESPONSE, { log });
    } else if (log.message.includes('tx submission failed')) {
      this.emitEvent(NODE_EVENTS.TRANSACTION_SUBMITTED_FAILED, { log });
    } else if (log.message.includes('tx submission passed')) {
      this.emitEvent(NODE_EVENTS.TRANSACTION_SUBMITTED_PASSED, { log });
    } else if (log.message.includes('DHT returned no address')) {
      this.emitEvent(NODE_EVENTS.RECEIVER_NOT_REGISTERED, { log });
    } else if (log.message.includes('error')) {
      this.emitEvent(NODE_EVENTS.ERROR, { log });
    }
  }

  private updateNodeReadiness(nodeId: string, status: 'peerConnected' | 'reservationAccepted' | 'listeningEstablished'): void {
    const current = this.nodeReadinessStatus.get(nodeId);
    if (current) {
      current[status] = true;
      this.nodeReadinessStatus.set(nodeId, current);
      
      // Check if all conditions are met
      if (current.peerConnected && current.reservationAccepted && current.listeningEstablished) {
        this.markNodeReady(nodeId);
      }
    }
  }

  // Logs API — now direct instance methods
  getAllLogs(): LogEntry[] {
    if (!this.wasmLogger) return [];
    return this.wasmLogger.getLogHistory(); // ← no JSON.parse
  }

  getLogsByLevel(level: number): LogEntry[] { return this.getAllLogs().filter(l => l.level === level); }
  getLogsByTarget(target: string): LogEntry[] { return this.getAllLogs().filter(l => l.target === target); }
  getLogsByMessage(searchText: string): LogEntry[] { return this.getAllLogs().filter(l => l.message.toLowerCase().includes(searchText.toLowerCase())); }
  getLogsByFile(filename: string): LogEntry[] { return this.getAllLogs().filter(l => l.file && l.file.includes(filename)); }
  getErrorLogs(): LogEntry[] { return this.getLogsByLevel(1); }
  getDebugLogs(): LogEntry[] { return this.getLogsByLevel(4); }
  getTransactionLogs(): LogEntry[] {
    return this.getAllLogs().filter(l =>
      l.message.toLowerCase().includes('transaction') ||
      l.message.toLowerCase().includes('tx') ||
      l.target.toLowerCase().includes('transaction')
    );
  }
  getP2PLogs(): LogEntry[] {
    return this.getAllLogs().filter(l =>
      l.target.toLowerCase().includes('p2p') ||
      l.message.toLowerCase().includes('peer') ||
      l.message.toLowerCase().includes('connection') ||
      (l.file ?? '').includes('p2p')
    );
  }
  getLatestLogs(count: number = 10): LogEntry[] {
    const logs = this.getAllLogs();
    return logs.slice(-count);
  }
  clearLogs(): void { this.wasmLogger?.clearHistory(); }
  reset(): void {
    this.registeredNodes.clear();
    this.wasmLogger = null;
    this.processedLogs.clear();
    this.nodeReadinessStatus.clear();
    this.nodeId = null;
    this.pendingReadiness = { peerConnected: false, reservationAccepted: false, listeningEstablished: false };
    this.handlers.clear();
    this.history.clear();
  }

  // Event bus methods
  private emitEvent(eventType: string, data: any): void {
    // Store in history
    if (!this.history.has(eventType)) {
      this.history.set(eventType, []);
    }
    this.history.get(eventType)!.push(data);

    // Notify handlers
    const eventHandlers = this.handlers.get(eventType);
    if (eventHandlers) {
      eventHandlers.forEach(handler => handler(data));
    }
  }

  waitForEvent(eventType: string, callback: (data: any) => void, timeout: number = 30000): Promise<any> {
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        reject(new Error(`Timeout waiting for event: ${eventType}`));
      }, timeout);

      const handler = (data: any) => {
        clearTimeout(timer);
        this.handlers.get(eventType)?.delete(handler);
        callback(data);
        resolve(data);
      };

      // Check history first
      const eventHistory = this.history.get(eventType);
      if (eventHistory && eventHistory.length > 0) {
        const latestEvent = eventHistory[eventHistory.length - 1];
        clearTimeout(timer);
        callback(latestEvent);
        resolve(latestEvent);
        return;
      }

      // Subscribe to future events
      if (!this.handlers.has(eventType)) {
        this.handlers.set(eventType, new Set());
      }
      this.handlers.get(eventType)!.add(handler);
    });
  }

  waitForLog(pattern: string, timeout: number = 30000): Promise<LogEntry> {
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        reject(new Error(`Timeout waiting for log pattern: ${pattern}`));
      }, timeout);

      const checkLogs = () => {
        const logs = this.getAllLogs();
        const matchingLog = logs.find(log => log.message.includes(pattern));
        if (matchingLog) {
          clearTimeout(timer);
          resolve(matchingLog);
        }
      };

      // Check immediately
      checkLogs();

      // Poll for new logs
      const pollInterval = setInterval(() => {
        checkLogs();
      }, 100);

      // Clean up on timeout
      setTimeout(() => {
        clearInterval(pollInterval);
      }, timeout);
    });
  }

  // Convenience methods
  async waitForPeerConnected(timeout: number = 30000): Promise<any> {
    return this.waitForEvent(NODE_EVENTS.PEER_CONNECTED, () => {}, timeout);
  }


}

export const nodeCoordinator = NodeCoordinator.getInstance();
