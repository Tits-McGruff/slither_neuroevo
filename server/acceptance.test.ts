import { it, expect } from 'vitest';
import WebSocket, { type RawData } from 'ws';
import { startServer } from './index.ts';
import { DEFAULT_CONFIG } from './config.ts';
import { describeNetworkSuite } from './test/networkSuites.ts';

/**
 * Parses WS text payloads into JSON objects when possible.
 * @param data - Raw websocket payload.
 * @returns Parsed JSON object or null on failure.
 */
function parseJsonMessage(data: RawData): Record<string, unknown> | null {
  const text =
    typeof data === 'string'
      ? data
      : Buffer.isBuffer(data)
        ? data.toString('utf8')
        : data instanceof ArrayBuffer
          ? Buffer.from(data).toString('utf8')
          : String(data ?? '');
  try {
    const parsed = JSON.parse(text) as unknown;
    if (!parsed || typeof parsed !== 'object') return null;
    return parsed as Record<string, unknown>;
  } catch {
    return null;
  }
}

describeNetworkSuite('acceptance: join and play flow', () => {
  it('assigns a snake and accepts actions', async () => {
    const server = await startServer({
      ...DEFAULT_CONFIG,
      port: 0,
      dbPath: ':memory:',
      resume: 'fresh',
      inferenceBackend: 'js',
      logLevel: 'error'
    });

    const ws = new WebSocket(server.wsUrl);
    ws.binaryType = 'arraybuffer';
    let assignedId: number | null = null;
    let sawSensor = false;

    try {
      const result = new Promise<void>((resolve, reject) => {
        let settled = false;
        const timeout = setTimeout(() => {
          if (settled) return;
          settled = true;
          reject(new Error('timeout'));
        }, 12000);

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

        ws.on('message', (data: RawData, isBinary: boolean) => {
          if (isBinary) return;
          const msg = parseJsonMessage(data);
          if (!msg) return;
          if (msg['type'] === 'assign') {
            assignedId = typeof msg['snakeId'] === 'number' ? msg['snakeId'] : null;
          }
          if (msg['type'] === 'sensors') {
            sawSensor = true;
            if (assignedId) {
              ws.send(JSON.stringify({
                type: 'action',
                tick: typeof msg['tick'] === 'number' ? msg['tick'] : 0,
                snakeId: assignedId,
                turn: 0.2,
                boost: 0
              }));
            }
            if (settled) return;
            settled = true;
            clearTimeout(timeout);
            resolve();
          }
        });

        ws.on('open', () => {
          ws.send(JSON.stringify({ type: 'hello', clientType: 'ui', version: 2 }));
          ws.send(JSON.stringify({ type: 'join', mode: 'player', name: 'acceptance' }));
        });
      });

      await result;
    } finally {
      if (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CLOSING) {
        ws.close();
      }
      await server.close();
    }

    expect(assignedId).toBeTruthy();
    expect(sawSensor).toBe(true);
  }, 20000);
});
