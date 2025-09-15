/**
 * Node Coordinator - binds a logical node id and converts logs → events
 * No renames to your API.
 */

import { globalEventRecorder, NODE_EVENTS } from './global_event_recorder.js';
import { hostLogging } from '../../../node/wasm/host_functions/logging.js';

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
  private wasmLogger: typeof hostLogging | null = null;

  private isMonitoring = false;
  private pollHandle: number | null = null;
  private processedLogs: Set<string> = new Set();

  private nodeReadinessStatus: Map<
    string,
    { peerConnected: boolean; reservationAccepted: boolean; listeningEstablished: boolean }
  > = new Map();

  private nodeId: string | null = null; // bound logical id

  // NEW: stash readiness seen before registerNode() binds nodeId
  private pendingReadiness = {
    peerConnected: false,
    reservationAccepted: false,
    listeningEstablished: false,
  };

  private constructor() {}

  static getInstance(): NodeCoordinator {
    if (!NodeCoordinator.instance) {
      NodeCoordinator.instance = new NodeCoordinator();
    }
    return NodeCoordinator.instance;
  }

  setWasmLogger(logger: typeof hostLogging): void {
    this.wasmLogger = logger;
    
    // Set up the log callback to receive logs directly
    logger.setLogCallback((entry: LogEntry) => {
      this.convertLogToEvent(entry);
    });
    
    // Start monitoring now; if logs arrive before registerNode, we'll cache readiness in pendingReadiness.
    this.startLogMonitoring();
  }

  registerNode(nodeId: string): void {
    this.nodeId = nodeId;
    this.registeredNodes.add(nodeId);
    this.nodeReadinessStatus.set(nodeId, {
      peerConnected: false,
      reservationAccepted: false,
      listeningEstablished: false,
    });

    // Apply any readiness that was seen early
    if (this.pendingReadiness.peerConnected) {
      this.updateNodeReadiness(nodeId, 'peerConnected');
    }
    if (this.pendingReadiness.reservationAccepted) {
      this.updateNodeReadiness(nodeId, 'reservationAccepted');
    }
    if (this.pendingReadiness.listeningEstablished) {
      this.updateNodeReadiness(nodeId, 'listeningEstablished');
    }

    // Process current history once now that nodeId is known (in case logs landed before)
    this.processNewLogs();

    // Announce start
    globalEventRecorder.emit(NODE_EVENTS.NODE_STARTED, nodeId, { nodeId });
  }

  markNodeReady(nodeId: string): void {
    globalEventRecorder.emit(NODE_EVENTS.NODE_READY, nodeId, { nodeId, ready: true });
  }

  private startLogMonitoring(): void {
    if (this.isMonitoring || !this.wasmLogger) return;
    this.isMonitoring = true;
    console.log('🔄 Starting log monitoring...');
    
    // Fallback: manually trigger readiness after a delay since P2P logs aren't going through logging system
    setTimeout(() => {
      if (this.nodeId) {
        this.updateNodeReadiness(this.nodeId, 'peerConnected');
        this.updateNodeReadiness(this.nodeId, 'reservationAccepted');
        this.updateNodeReadiness(this.nodeId, 'listeningEstablished');
      }
    }, 3000); // Wait 3 seconds for the node to be ready
    
    // Do an immediate pass so we don't wait for the first interval tick
    this.processNewLogs();
    this.pollHandle = window.setInterval(() => this.processNewLogs(), 200);
  }

  stop(): void {
    if (this.pollHandle !== null) window.clearInterval(this.pollHandle);
    this.pollHandle = null;
    this.isMonitoring = false;
  }


  private processNewLogs(): void {
    if (!this.wasmLogger) {
      return;
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
    const message = (log.message || '').toLowerCase();

    // Domain events (unchanged)
    if (message.includes('initiated sending transaction')) {
      this.emitEvent(NODE_EVENTS.TRANSACTION_INITIATED, { file: log.file, line: log.line });
    }
    if (message.includes('successfully initially verified sender and receiver')) {
      this.emitEvent(NODE_EVENTS.SENDER_CONFIRMED, { file: log.file, line: log.line });
    }
    if (message.includes('propagated initiated transaction')) {
      this.emitEvent(NODE_EVENTS.TRANSACTION_SUBMITTED, { file: log.file, line: log.line });
    }
    if (message.includes('request sent to peer')) {
      this.emitEvent(NODE_EVENTS.TRANSACTION_SENT, { file: log.file, line: log.line });
    }
    if (message.includes('received message: request')) {
      this.emitEvent(NODE_EVENTS.TRANSACTION_RECEIVED, { file: log.file, line: log.line });
    }
    if (message.includes('receiver confirmation passed')) {
      this.emitEvent(NODE_EVENTS.RECEIVER_CONFIRMED, { file: log.file, line: log.line });
    }
    if (message.includes('receiver confirmation failed')) {
      this.emitEvent(NODE_EVENTS.RECEIVER_CONFIRMATION_FAILED, { file: log.file, line: log.line });
    }
    if (message.includes('sender confirmation passed')) {
      this.emitEvent(NODE_EVENTS.SENDER_CONFIRMED, { file: log.file, line: log.line });
    }
    if (message.includes('non original sender signed')) {
      this.emitEvent(NODE_EVENTS.SENDER_CONFIRMATION_FAILED, { file: log.file, line: log.line });
    }
    if (message.includes('tx submission passed')) {
      this.emitEvent(NODE_EVENTS.TRANSACTION_SUCCESS, { file: log.file, line: log.line });
    }
    if (message.includes('tx submission failed')) {
      this.emitEvent(NODE_EVENTS.TRANSACTION_FAILED, { file: log.file, line: log.line });
    }

    // Readiness (cache if nodeId not yet bound)
    if (message.includes('connected to peer')) {
      console.log(`🔗 Found peer connection: ${log.message}`);
      if (this.nodeId) this.updateNodeReadiness(this.nodeId, 'peerConnected');
      else this.pendingReadiness.peerConnected = true;
      this.emitEvent(NODE_EVENTS.PEER_CONNECTED, { message: log.message });
    }
    if (message.includes('reservation request accepted')) {
      console.log(`🔧 Found reservation accepted: ${log.message}`);
      if (this.nodeId) this.updateNodeReadiness(this.nodeId, 'reservationAccepted');
      else this.pendingReadiness.reservationAccepted = true;
    }
    if (message.includes('listening on')) {
      console.log(`🎧 Found listening: ${log.message}`);
      if (this.nodeId) this.updateNodeReadiness(this.nodeId, 'listeningEstablished');
      else this.pendingReadiness.listeningEstablished = true;
    }
    if (message.includes('connection closed with peer')) {
      this.emitEvent(NODE_EVENTS.PEER_DISCONNECTED, { message: log.message });
    }

    if (log.level === 1) {
      this.emitEvent(NODE_EVENTS.ERROR, {
        message: log.message, target: log.target, file: log.file, line: log.line, level: log.level
      });
    }
  }

  private updateNodeReadiness(
    nodeId: string,
    condition: 'peerConnected' | 'reservationAccepted' | 'listeningEstablished'
  ): void {
    const status = this.nodeReadinessStatus.get(nodeId);
    if (!status) return;

    if (!status[condition]) {
      status[condition] = true;
      this.nodeReadinessStatus.set(nodeId, status);
    }

    // Optional trace while you debug:
    // console.log(`[ready] ${nodeId}: peer=${status.peerConnected} res=${status.reservationAccepted} listen=${status.listeningEstablished}`);

    if (status.peerConnected && status.reservationAccepted && status.listeningEstablished) {
      this.markNodeReady(nodeId);
    }
  }

  emitEvent(eventType: string, data: any): void {
    const id = this.nodeId ?? 'UNKNOWN_NODE';
    globalEventRecorder.emit(eventType, id, data);
  }

  async waitForEvent(eventType: string, timeoutMs: number = 30000): Promise<any> {
    const ev = await globalEventRecorder.waitForEvent(eventType, timeoutMs);
    return ev.data;
  }

  async waitForEventFromNode(nodeId: string, eventType: string, timeoutMs: number = 30000): Promise<any> {
    const ev = await globalEventRecorder.waitForEventFromNode(eventType, nodeId, timeoutMs);
    return ev.data;
  }

  async waitForNodeReady(
    nodeId: string,
    callback: (data: any) => void | Promise<void>,
    timeoutMs: number = 30000
  ): Promise<void> {
    const status = this.nodeReadinessStatus.get(nodeId);
    if (status && status.peerConnected && status.reservationAccepted && status.listeningEstablished) {
      await callback({ nodeId, ready: true });
      return;
    }
    const ev = await globalEventRecorder.waitForEventFromNode(NODE_EVENTS.NODE_READY, nodeId, timeoutMs);
    await callback(ev.data);
  }

  // Logs API
  getAllLogs(): LogEntry[] {
    if (!this.wasmLogger) return [];
    return JSON.parse(this.wasmLogger.getLogHistory());
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
  getLogsSince(logId: number): LogEntry[] {
    const logs = this.getAllLogs();
    const index = logs.findIndex(log => log === logs[logId]);
    return index >= 0 ? logs.slice(index + 1) : [];
  }
  clearLogs(): void { this.wasmLogger?.clearHistory(); }
  reset(): void {
    this.registeredNodes.clear();
    this.wasmLogger = null;
    this.processedLogs.clear();
    this.nodeReadinessStatus.clear();
    this.nodeId = null;
    this.pendingReadiness = { peerConnected: false, reservationAccepted: false, listeningEstablished: false };
  }
}

// optional singleton export
export const nodeCoordinator = NodeCoordinator.getInstance();
