// utils/global_setup.ts
/* Keep this as TypeScript; we'll compile it to JS and run with Node. */
/* eslint-disable @typescript-eslint/no-var-requires */

type GlobalEvent = { type: string; sourceNodeId: string; data: any };

// Use CJS require at runtime to avoid TS/ESM export quirks of "ws"
const { WebSocketServer } = require('ws') as { WebSocketServer: any };

const HOST = process.env.E2E_HUB_HOST ?? '127.0.0.1';
const PORT = Number(process.env.E2E_HUB_PORT ?? '17321');

// Cache: last event per (type::sourceNodeId) so late joiners catch up
const lastByKey = new Map<string, GlobalEvent>();
const keyOf = (e: GlobalEvent) => `${e.type}::${e.sourceNodeId}`;

let wss: any;
try {
  wss = new WebSocketServer({ host: HOST, port: PORT });
} catch (e: any) {
  if (e?.code === 'EADDRINUSE') {
    console.log(`[e2e-hub] Port ${PORT} already in use; assuming hub is running. Exiting.`);
    process.exit(0);
  }
  throw e;
}

wss.on('listening', () => {
  console.log(`[e2e-hub] Listening ws://${HOST}:${PORT}`);
});

wss.on('connection', (ws: any) => {
  // Replay cache to newcomer (so they see prior NODE_READY immediately)
  for (const ev of lastByKey.values()) {
    try { ws.send(JSON.stringify(ev)); } catch {}
  }

  ws.on('message', (raw: any) => {
    let ev: GlobalEvent | undefined;
    try {
      ev = JSON.parse(String(raw)) as GlobalEvent;
    } catch {
      return;
    }
    if (!ev || !ev.type) return;

    lastByKey.set(keyOf(ev), ev);

    // Fan out to everyone else
    for (const client of wss.clients) {
      if (client !== ws && client.readyState === 1) {
        try { client.send(JSON.stringify(ev)); } catch {}
      }
    }
  });
});

const shutdown = () => {
  try { wss?.close(() => process.exit(0)); } catch { process.exit(0); }
};
process.on('SIGINT', shutdown);
process.on('SIGTERM', shutdown);
