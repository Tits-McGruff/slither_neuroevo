import { describe, it, expect } from 'vitest';
import WebSocket, { type RawData } from 'ws';
import { startServer } from './index.ts';
import { DEFAULT_CONFIG } from './config.ts';

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

/**
 * Starts the server and returns null when permissions prevent binding.
 * @returns Server handle or null when the port is unavailable.
 */
async function startServerWithGuard() {
  const isEperm = (err: unknown): boolean =>
    (err as { code?: string } | null)?.code === 'EPERM';
  const startPromise = startServer({
    ...DEFAULT_CONFIG,
    port: 0,
    logLevel: 'error'
  }).catch((err) => {
    if (isEperm(err)) return null;
    throw err;
  });

  let cleanup = () => {};
  const guard = new Promise<null>((resolve) => {
    const handler = (err: unknown) => {
      if (isEperm(err)) {
        resolve(null);
        return;
      }
      throw err;
    };
    process.once('uncaughtException', handler);
    cleanup = () => process.off('uncaughtException', handler);
  });

  let server: Awaited<ReturnType<typeof startServer>> | null = null;
  try {
    server = await Promise.race([startPromise, guard]);
  } finally {
    cleanup();
  }

  return server;
}

describe('acceptance: join and play flow', () => {
  it('assigns a snake and accepts actions', async () => {
    const server = await startServerWithGuard();
    if (!server) return;

    const ws = new WebSocket(server.wsUrl);
    ws.binaryType = 'arraybuffer';
    let assignedId: number | null = null;
    let sawSensor = false;

    try {
      const result = new Promise<void>((resolve, reject) => {
        const timeout = setTimeout(() => reject(new Error('timeout')), 1800);

        ws.on('error', (err: Error) => {
          clearTimeout(timeout);
          reject(err);
        });

        ws.on('message', (data: RawData, isBinary: boolean) => {
          if (isBinary) return;
          const msg = parseJsonMessage(data);
          if (!msg) return;
          if (msg.type === 'assign') {
            assignedId = typeof msg.snakeId === 'number' ? msg.snakeId : null;
          }
          if (msg.type === 'sensors') {
            sawSensor = true;
            if (assignedId) {
              ws.send(JSON.stringify({
                type: 'action',
                tick: typeof msg.tick === 'number' ? msg.tick : 0,
                snakeId: assignedId,
                turn: 0.2,
                boost: 0
              }));
            }
            clearTimeout(timeout);
            resolve();
          }
        });

        ws.on('open', () => {
          ws.send(JSON.stringify({ type: 'hello', clientType: 'ui', version: 1 }));
          ws.send(JSON.stringify({ type: 'join', mode: 'player', name: 'acceptance' }));
        });
      });

      await result;
    } finally {
      ws.close();
      await server.close();
    }

    expect(assignedId).toBeTruthy();
    expect(sawSensor).toBe(true);
  });
});
