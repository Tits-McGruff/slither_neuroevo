import { describe, it, expect } from 'vitest';
import WebSocket, { type RawData } from 'ws';
import { startServer } from './index.ts';
import { DEFAULT_CONFIG } from './config.ts';

describe('system: server lifecycle', () => {
  it('boots, streams, and shuts down cleanly', async () => {
    const server = await startServer({
      ...DEFAULT_CONFIG,
      port: 0,
      dbPath: ':memory:',
      logLevel: 'error'
    });

    const ws = new WebSocket(server.wsUrl);
    ws.binaryType = 'arraybuffer';
    let sawFrame = false;

    try {
      const result = new Promise<void>((resolve, reject) => {
        let settled = false;
        const timeout = setTimeout(() => {
          if (settled) return;
          settled = true;
          reject(new Error('timeout'));
        }, 15000);

        ws.on('error', (err: Error) => {
          if (settled) return;
          settled = true;
          clearTimeout(timeout);
          reject(err);
        });

        ws.on('close', (code: number) => {
          if (settled) return;
          settled = true;
          clearTimeout(timeout);
          reject(new Error(`socket closed (${code})`));
        });

        ws.on('message', (_data: RawData, isBinary: boolean) => {
          if (isBinary) sawFrame = true;
          if (sawFrame) {
            if (settled) return;
            settled = true;
            clearTimeout(timeout);
            resolve();
          }
        });

        ws.on('open', () => {
          ws.send(JSON.stringify({ type: 'hello', clientType: 'ui', version: 2 }));
          ws.send(JSON.stringify({ type: 'join', mode: 'spectator' }));
        });
      });

      await result;
    } finally {
      if (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CLOSING) {
        ws.close();
      }
      await server.close();
    }

    expect(sawFrame).toBe(true);
  }, 20000);
});
