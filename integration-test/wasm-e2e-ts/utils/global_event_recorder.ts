// test/utils/global_event_recorder.ts
export interface GlobalEvent {
    type: string;
    sourceNodeId: string;
    data: any;
  }
  
  export type GlobalEventHandler = (event: GlobalEvent) => void;
  
  export class GlobalEventRecorder {
    private static instance: GlobalEventRecorder;
  
    private events = new Map<string, GlobalEventHandler[]>();
    private ws: WebSocket | null = null;
    private queue: GlobalEvent[] = [];
    private isOpen = false;
    private connecting = false;
  
    private readonly HUB_URL = `ws://127.0.0.1:${(import.meta as any).env?.E2E_HUB_PORT || 17321}`;
  
    private constructor() { this.ensureConnected(); }
  
    static getInstance(): GlobalEventRecorder {
      if (!GlobalEventRecorder.instance) GlobalEventRecorder.instance = new GlobalEventRecorder();
      return GlobalEventRecorder.instance;
    }
  
    subscribe(eventType: string, handler: GlobalEventHandler): void {
      if (!this.events.has(eventType)) this.events.set(eventType, []);
      this.events.get(eventType)!.push(handler);
    }
  
    unsubscribe(eventType: string, handler: GlobalEventHandler): void {
      const list = this.events.get(eventType);
      if (!list) return;
      const i = list.indexOf(handler);
      if (i > -1) list.splice(i, 1);
    }
  
    emit(eventType: string, sourceNodeId: string, data: any): void {
      const ev: GlobalEvent = { type: eventType, sourceNodeId, data };
      // fire locally immediately
      this.dispatch(ev);
      // send to hub (or queue if not yet open)
      if (this.isOpen && this.ws?.readyState === this.ws?.OPEN) {
        this.ws?.send(JSON.stringify(ev));
      } else {
        this.queue.push(ev);
        this.ensureConnected();
      }
    }
  
    async waitForEvent(eventType: string, timeoutMs = 30000): Promise<GlobalEvent> {
      return new Promise((resolve, reject) => {
        const timer = setTimeout(() => {
          this.unsubscribe(eventType, handler);
          reject(new Error(`Timeout waiting for event '${eventType}' after ${timeoutMs}ms`));
        }, timeoutMs);
  
        const handler = (event: GlobalEvent) => {
          clearTimeout(timer);
          this.unsubscribe(eventType, handler);
          resolve(event);
        };
  
        this.subscribe(eventType, handler);
      });
    }
  
    async waitForEventFromNode(eventType: string, sourceNodeId: string, timeoutMs = 30000): Promise<GlobalEvent> {
      return new Promise((resolve, reject) => {
        const timer = setTimeout(() => {
          this.unsubscribe(eventType, handler);
          reject(new Error(`Timeout waiting for event '${eventType}' from '${sourceNodeId}' after ${timeoutMs}ms`));
        }, timeoutMs);
  
        const handler = (event: GlobalEvent) => {
          if (event.sourceNodeId === sourceNodeId) {
            clearTimeout(timer);
            this.unsubscribe(eventType, handler);
            resolve(event);
          }
        };
  
        this.subscribe(eventType, handler);
      });
    }
  
    reset(): void { this.events.clear(); }
  
    private ensureConnected() {
      if (this.connecting || this.isOpen) return;
      this.connecting = true;
  
      try {
        this.ws = new WebSocket(this.HUB_URL);
  
        this.ws.onopen = () => {
          this.isOpen = true;
          this.connecting = false;
          // flush queued
          for (const ev of this.queue.splice(0)) {
            this.ws!.send(JSON.stringify(ev));
          }
        };
  
        this.ws.onmessage = (e) => {
          try {
            const ev = JSON.parse(e.data as string) as GlobalEvent;
            if (!ev || !ev.type) return;
            this.dispatch(ev);
          } catch {}
        };
  
        this.ws.onclose = () => {
          this.isOpen = false;
          this.connecting = false;
          setTimeout(() => this.ensureConnected(), 200);
        };
  
        this.ws.onerror = () => {
          // hub may not be ready yet — close and retry
          try { this.ws?.close(); } catch {}
        };
      } catch {
        this.connecting = false;
        setTimeout(() => this.ensureConnected(), 200);
      }
    }
  
    private dispatch(event: GlobalEvent) {
      const handlers = this.events.get(event.type) || [];
      for (const h of handlers) {
        try { h(event); } catch (err) { console.error(`Error in handler for '${event.type}':`, err); }
      }
    }
  }
  
  export const globalEventRecorder = GlobalEventRecorder.getInstance();
  
  export const NODE_EVENTS = {
    NODE_STARTED: 'node_started',
    NODE_READY: 'node_ready',
    TRANSACTION_INITIATED: 'transaction_initiated',
    TRANSACTION_SENT: 'transaction_sent',
    TRANSACTION_RECEIVED: 'transaction_received',
    TRANSACTION_SUBMITTED: 'transaction_submitted',
    TRANSACTION_SUCCESS: 'transaction_success',
    TRANSACTION_FAILED: 'transaction_failed',
    SENDER_CONFIRMED: 'sender_confirmed',
    RECEIVER_CONFIRMED: 'receiver_confirmed',
    SENDER_CONFIRMATION_FAILED: 'sender_confirmation_failed',
    RECEIVER_CONFIRMATION_FAILED: 'receiver_confirmation_failed',
    RECEIVER_UNREGISTERED: 'receiver_unregistered',
    SENDER_FAILED: 'sender_failed',
    PEER_CONNECTED: 'peer_connected',
    PEER_DISCONNECTED: 'peer_disconnected',
    ERROR: 'error'
  } as const;
  