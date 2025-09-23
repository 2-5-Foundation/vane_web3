// import type { TxStateMachine } from "./primitives";
// import { toHex } from "viem";

// const HEARTBEAT_INTERVAL = 45000; // Less frequent heartbeat
// const REQUEST_TIMEOUT = 30000;
// const MAX_RECONNECT_ATTEMPTS = 5;
// const RECONNECT_DELAY = 2000; // Fixed 2 second delay

// export interface VaneClientRpc {
//   register(name: string, accountId: string, network: string): Promise<void>;
//   initiateTransaction(sender: string, receiver: string, amount: number, token: string, network: string, codeword: string): Promise<void>;
//   senderConfirm(tx: TxStateMachine): Promise<void>;
//   receiverConfirm(tx: TxStateMachine): Promise<void>;
//   revertTransaction(tx: TxStateMachine): Promise<void>;
//   watchTxUpdates(callback: (tx: TxStateMachine) => void): Promise<() => Promise<void>>;
//   fetchPendingTxUpdates: () => Promise<TxStateMachine[]>;
//   disconnect(): void;
//   isConnected(): boolean;
// }

// interface PendingRequest<T = unknown> {
//   resolve: (value: T) => void;
//   reject: (error: Error) => void;
//   timeout: ReturnType<typeof setTimeout>;
// }

// export const createVaneClient = async (url: string): Promise<VaneClientRpc> => {
//   if (!url) {
//     throw new Error('WebSocket URL is required');
//   }

//   let socket: WebSocket;
//   let heartbeatInterval: ReturnType<typeof setInterval> | null = null;
//   let txUpdateCallback: ((tx: TxStateMachine) => void) | null = null;
//   let isCleanClose = false;
//   let reconnectAttempts = 0;
//   let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
//   let isReconnecting = false;
  
//   const pendingRequests = new Map<number, PendingRequest<unknown>>();
//   let requestId = 1;

//   const cleanup = (shouldReconnect = false) => {
//     if (!shouldReconnect) {
//       isCleanClose = true;
//     }
    
//     if (heartbeatInterval) {
//       clearInterval(heartbeatInterval);
//       heartbeatInterval = null;
//     }
    
//     if (reconnectTimer) {
//       clearTimeout(reconnectTimer);
//       reconnectTimer = null;
//     }
    
//     if (!shouldReconnect) {
//       pendingRequests.forEach(({ reject, timeout }) => {
//         clearTimeout(timeout);
//         reject(new Error('Connection closed'));
//       });
//       pendingRequests.clear();
//     }
    
//     if (socket && socket.readyState === WebSocket.OPEN) {
//       socket.close(1000, 'Client disconnect');
//     }
//   };

//   const isSocketReady = (): boolean => {
//     return socket && socket.readyState === WebSocket.OPEN;
//   };

//   const reconnect = async (): Promise<void> => {
//     if (isCleanClose || isReconnecting) {
//       return;
//     }

//     if (reconnectAttempts >= MAX_RECONNECT_ATTEMPTS) {
//       console.error(`Max reconnection attempts (${MAX_RECONNECT_ATTEMPTS}) reached. Giving up.`);
//       return;
//     }

//     isReconnecting = true;
//     reconnectAttempts++;
    
//     console.info(`Attempting to reconnect (${reconnectAttempts}/${MAX_RECONNECT_ATTEMPTS}) in ${RECONNECT_DELAY}ms...`);
    
//     reconnectTimer = setTimeout(async () => {
//       try {
//         await connect();
//         console.info('Reconnection successful!');
//         reconnectAttempts = 0;
//         isReconnecting = false;
        
//         // Re-subscribe to tx updates if we had a callback
//         if (txUpdateCallback) {
//           console.info('Re-subscribing to transaction updates...');
//           await sendRequest("subscribeTxUpdates", []);
//         }
//       } catch (error) {
//         console.warn(`Reconnection attempt ${reconnectAttempts} failed:`, error);
//         isReconnecting = false;
//         // Try again
//         reconnect();
//       }
//     }, RECONNECT_DELAY);
//   };

//   // Simple heartbeat - use proper JSON-RPC format
//   const startHeartbeat = () => {
//     if (heartbeatInterval) {
//       clearInterval(heartbeatInterval);
//     }
    
//     heartbeatInterval = setInterval(() => {
//       if (isSocketReady()) {
//         try {
//           // Use a proper JSON-RPC ping instead of simple JSON
//           const pingMessage = JSON.stringify({
//             jsonrpc: "2.0",
//             method: "ping",
//             params: [],
//             id: requestId++
//           });
//           socket.send(pingMessage);
//           console.log('Heartbeat sent');
//         } catch (error) {
//           console.warn('Heartbeat failed (connection may be closing naturally):', error);
//         }
//       }
//     }, HEARTBEAT_INTERVAL);
//   };

//   const onOpen = () => {
//     console.info('WebSocket connected and stable');
//     reconnectAttempts = 0; // Reset on successful connection
//     startHeartbeat();
//   };

//   const onClose = (event: CloseEvent) => {
//     console.info(`WebSocket closed: ${event.code} - ${event.reason}`);
    
//     if (heartbeatInterval) {
//       clearInterval(heartbeatInterval);
//       heartbeatInterval = null;
//     }
    
//     // Only attempt reconnection if it wasn't a clean close
//     if (!isCleanClose) {
//       console.warn('Connection lost unexpectedly, attempting to reconnect...');
//       reconnect();
//     }
//   };

//   const onError = (event: Event) => {
//     console.warn('WebSocket error:', event);
//     // Let the close handler decide about reconnection
//   };

//   const onMessage = (event: MessageEvent) => {
//     try {
//       console.log("event.data", event.data)
//       const data = JSON.parse(event.data);
      
//       // LOG EVERYTHING - Let's see what we're actually getting
//       console.log('=== RAW WEBSOCKET MESSAGE ===');
//       console.log('Raw event.data:', event.data);
//       console.log('Parsed data:', JSON.stringify(data, null, 2));
//       console.log('Message type:', typeof data);
//       console.log('Has method?', data.method);
//       console.log('Has params?', data.params);
//       console.log('Has id?', data.id);
//       console.log('Has result?', data.result);
//       console.log('Has error?', data.error);
//       console.log('================================');
      
//       // Handle subscription notifications
//       // Format: { "jsonrpc": "2.0", "method": "subscription", "params": { "subscription": "id", "result": TxStateMachine } }
//       if (data.method === "subscription" && data.params && txUpdateCallback) {
//         console.log('🔔 SUBSCRIPTION NOTIFICATION DETECTED');
//         console.log('Subscription params:', JSON.stringify(data.params, null, 2));
//         if (data.params.result) {
//           console.log('📦 Calling txUpdateCallback with result:', JSON.stringify(data.params.result, null, 2));
//           txUpdateCallback(data.params.result);
//         } else if (data.params.subscription && data.params.result !== undefined) {
//           console.log('📦 Calling txUpdateCallback with subscription result:', JSON.stringify(data.params.result, null, 2));
//           txUpdateCallback(data.params.result);
//         } else {
//           console.log('❌ Subscription notification but no valid result found');
//         }
//         return;
//       }
      
//       // Handle direct method notifications (alternative format)
//       if (data.method === "subscribeTxUpdates" && data.params && txUpdateCallback) {
//         console.log('🔔 DIRECT subscribeTxUpdates NOTIFICATION DETECTED');
//         console.log('Direct notification params:', JSON.stringify(data.params, null, 2));
//         console.log('📦 Calling txUpdateCallback with params:', JSON.stringify(data.params, null, 2));
//         txUpdateCallback(data.params);
//         return;
//       }
      
//       // Handle RPC responses (including subscription setup and fetchPendingTxUpdates)
//       if (data.id !== undefined && pendingRequests.has(data.id)) {
//         console.log('🎯 RPC RESPONSE DETECTED');
//         console.log('Response ID:', data.id);
//         console.log('Response result:', JSON.stringify(data.result, null, 2));
//         console.log('Response error:', data.error);
        
//         const request = pendingRequests.get(data.id)!;
//         pendingRequests.delete(data.id);
//         clearTimeout(request.timeout);
        
//         if (data.error) {
//           console.error('❌ RPC Error:', data.error);
//           request.reject(new Error(data.error.message || JSON.stringify(data.error)));
//         } else {
//           console.log('✅ RPC Success - resolving with result:', JSON.stringify(data.result, null, 2));
//           request.resolve(data.result);
//         }
//         return;
//       }
      
//       // Log unhandled messages for debugging
//       console.warn('⚠️ UNHANDLED WEBSOCKET MESSAGE:');
//       console.warn('Full message:', JSON.stringify(data, null, 2));
//       console.warn('Current txUpdateCallback exists?', !!txUpdateCallback);
//       console.warn('Current pendingRequests:', Array.from(pendingRequests.keys()));
      
//     } catch (error) {
//       console.error("💥 ERROR PARSING WEBSOCKET MESSAGE:", error);
//       console.log("🔍 Raw message that failed to parse:", event.data);
//       console.log("🔍 Raw message type:", typeof event.data);
//       console.log("🔍 Raw message length:", event.data?.length);
//     }
//   };

//   const connect = async (): Promise<void> => {
//     return new Promise((resolve, reject) => {
//       console.info('Establishing WebSocket connection...');
      
//       socket = new WebSocket(url);
      
//       const connectionTimeout = setTimeout(() => {
//         socket.close();
//         reject(new Error('Connection timeout'));
//       }, 10000);

//       const handleOpen = () => {
//         clearTimeout(connectionTimeout);
//         console.info('WebSocket handshake completed successfully');
//         resolve();
//       };

//       const handleError = () => {
//         clearTimeout(connectionTimeout);
//         reject(new Error('WebSocket connection failed'));
//       };

//       socket.addEventListener('open', handleOpen, { once: true });
//       socket.addEventListener('error', handleError, { once: true });
      
//       // Set up persistent listeners
//       socket.addEventListener('open', onOpen);
//       socket.addEventListener('close', onClose);
//       socket.addEventListener('error', onError);
//       socket.addEventListener('message', onMessage);
//     });
//   };

//   const sendRequest = async <T = void>(method: string, params: unknown[]): Promise<T> => {
//     // Wait for connection if we're reconnecting
//     if (isReconnecting) {
//       await new Promise((resolve) => {
//         const checkConnection = () => {
//           if (isSocketReady()) {
//             resolve(void 0);
//           } else if (!isReconnecting) {
//             resolve(void 0); // Give up if not reconnecting anymore
//           } else {
//             setTimeout(checkConnection, 100);
//           }
//         };
//         checkConnection();
//       });
//     }

//     if (!isSocketReady()) {
//       throw new Error('WebSocket not connected and reconnection failed');
//     }

//     const id = requestId++;
    
//     // Construct the JSON-RPC message carefully
//     const rpcMessage = {
//       jsonrpc: "2.0",
//       method: method,
//       params: params,
//       id: id
//     };

//     // Convert to JSON with proper bigint handling
//     const message = JSON.stringify(rpcMessage, (key, value) => {
//       if (typeof value === 'bigint') {
//         return toHex(value);
//       }
//       return value;
//     });

//     // DEBUG: Check what's at position 271
//     console.log('=== DEBUG REQUEST ===');
//     console.log('Full message length:', message.length);
//     if (message.length >= 271) {
//         console.log('Character at position 271:', message.charAt(270)); // 0-indexed
//         console.log('Context around position 271:', message.slice(260, 280));
//     }
//     console.log('Full message:', message);
//     console.log('Params being sent:', JSON.stringify(params, (key, value) => {
//         if (typeof value === 'bigint') return `BigInt(${value})`;
//         if (value instanceof Uint8Array) return `Uint8Array[${value.length}]`;
//         return value;
//     }, 2));

//     console.log('Sending WebSocket request:', rpcMessage);
//     console.log('Raw JSON being sent:', message);

//     return new Promise<T>((resolve, reject) => {
//       const timeout = setTimeout(() => {
//         pendingRequests.delete(id);
//         reject(new Error(`Request timeout: ${method}`));
//       }, REQUEST_TIMEOUT);

//       pendingRequests.set(id, { resolve, reject, timeout });

//       try {
//         // Ensure the socket is still ready before sending
//         if (!isSocketReady()) {
//           throw new Error('WebSocket connection lost');
//         }
        
//         socket.send(message);
//         console.log('Message sent successfully');
//       } catch (error) {
//         console.error('Error sending message:', error);
//         clearTimeout(timeout);
//         pendingRequests.delete(id);
//         reject(error);
//       }
//     });
//   };

//   // Initial connection - simple and clean
//   await connect();
//   console.info('WebSocket client ready and stable');

//   return {
//     register: (name: string, accountId: string, network: string) =>
//       sendRequest("register", [name, accountId, network]),

//     initiateTransaction: (sender: string, receiver: string, amount: number, token: string, network: string, codeword: string) =>
//       sendRequest("initiateTransaction", [sender, receiver, amount, token, network, codeword]),

//     senderConfirm: (tx: TxStateMachine) => {
//         const serializedTx = {
//           ...tx,
//           // Convert Uint8Array to number arrays for Rust server
//           multiId: tx.multiId instanceof Uint8Array ? Array.from(tx.multiId) : tx.multiId,
//           recvSignature: tx.recvSignature ? 
//             (tx.recvSignature instanceof Uint8Array ? Array.from(tx.recvSignature) : tx.recvSignature) 
//             : undefined,
//           signedCallPayload: tx.signedCallPayload ? 
//             (tx.signedCallPayload instanceof Uint8Array ? Array.from(tx.signedCallPayload) : tx.signedCallPayload) 
//             : undefined,
//           callPayload: tx.callPayload ? 
//             (tx.callPayload instanceof Uint8Array ? Array.from(tx.callPayload) : tx.callPayload) 
//             : undefined,
//         };
//         return sendRequest("senderConfirm", [serializedTx]);
//     },

//     receiverConfirm: (tx: TxStateMachine) => {
//         const serializedTx = {
//           ...tx,
//           // Convert Uint8Array to number arrays for Rust server
//           multiId: tx.multiId instanceof Uint8Array ? Array.from(tx.multiId) : tx.multiId,
//           recvSignature: tx.recvSignature ? 
//             (tx.recvSignature instanceof Uint8Array ? Array.from(tx.recvSignature) : tx.recvSignature) 
//             : undefined,
//           signedCallPayload: tx.signedCallPayload ? 
//             (tx.signedCallPayload instanceof Uint8Array ? Array.from(tx.signedCallPayload) : tx.signedCallPayload) 
//             : undefined,
//           callPayload: tx.callPayload ? 
//             (tx.callPayload instanceof Uint8Array ? Array.from(tx.callPayload) : tx.callPayload) 
//             : undefined,
//         };
//         return sendRequest("receiverConfirm", [serializedTx]);
//     },

//     revertTransaction: (tx: TxStateMachine) =>
//       sendRequest("revertTransaction", [tx]),

//     watchTxUpdates: async (callback: (tx: TxStateMachine) => void) => {
//       console.log('🎬 STARTING watchTxUpdates');
//       console.log('Setting up txUpdateCallback...');
//       txUpdateCallback = callback;
      
//       console.log('📡 Sending subscribeTxUpdates request...');
//       const result = await sendRequest("subscribeTxUpdates", []);
//       console.log('📡 subscribeTxUpdates response:', JSON.stringify(result, null, 2));
      
//       return async () => {
//         console.log('🛑 Unsubscribing from watchTxUpdates');
//         txUpdateCallback = null;
//       };
//     },

//     fetchPendingTxUpdates: async () => {
//       console.log('📋 STARTING fetchPendingTxUpdates');
//       const result = await sendRequest<TxStateMachine[]>("fetchPendingTxUpdates", []);
//       console.log('📋 fetchPendingTxUpdates response:', JSON.stringify(result, null, 2));
//       console.log('📋 fetchPendingTxUpdates result type:', typeof result);
//       console.log('📋 fetchPendingTxUpdates is array?', Array.isArray(result));
//       if (Array.isArray(result)) {
//         console.log('📋 fetchPendingTxUpdates array length:', result.length);
//         result.forEach((tx, index) => {
//           console.log(`📋 Transaction ${index}:`, JSON.stringify(tx, null, 2));
//         });
//       }
//       return result;
//     },

//     isConnected: () => isSocketReady(),

//     disconnect: () => cleanup(false)
//   };
// };

// export const initializeWebSocket = async (url: string) => {
//   if (!url) {
//     throw new Error("WebSocket URL is required");
//   }

//   return await createVaneClient(url);
// };