import { describe, it, expect } from 'vitest';
import WebSocket, { type RawData } from 'ws';
import { startServer } from './index.ts';
import { DEFAULT_CONFIG } from './config.ts';
import { getSensorLayout } from '../src/protocol/sensors.ts';

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

  let cleanup = () => { };
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

describe('server integration', () => {
  it('handshakes and streams frames', async () => {
    const server = await startServerWithGuard();
    if (!server) return;

    const ws = new WebSocket(server.wsUrl);
    ws.binaryType = 'arraybuffer';

    const seen = { welcome: false, frame: false };

    try {
      const result = new Promise<void>((resolve, reject) => {
        const timeout = setTimeout(() => {
          reject(new Error('timed out waiting for welcome/frame'));
        }, 4000);

        ws.on('error', (err: Error) => {
          clearTimeout(timeout);
          reject(err);
        });

        ws.on('message', (data: RawData, isBinary: boolean) => {
          if (!isBinary) {
            const text =
              typeof data === 'string'
                ? data
                : Buffer.isBuffer(data)
                  ? data.toString('utf8')
                  : data instanceof ArrayBuffer
                    ? Buffer.from(data).toString('utf8')
                    : String(data ?? '');
            try {
              const msg = JSON.parse(text) as { type?: string };
              if (msg.type === 'welcome') seen.welcome = true;
            } catch {
              // Ignore malformed control payloads.
            }
          } else {
            seen.frame = true;
          }

          if (seen.welcome && seen.frame) {
            clearTimeout(timeout);
            resolve();
          }
        });

        ws.on('open', () => {
          ws.send(JSON.stringify({ type: 'hello', clientType: 'ui', version: 1 }));
          ws.send(JSON.stringify({ type: 'join', mode: 'spectator' }));
        });
      });

      await result;
    } finally {
      ws.close();
      await server.close();
    }

    expect(seen.welcome).toBe(true);
    expect(seen.frame).toBe(true);
  }, 20000);

  it('assigns a player and streams sensors', async () => {
    const server = await startServerWithGuard();
    if (!server) return;

    const ws = new WebSocket(server.wsUrl);
    ws.binaryType = 'arraybuffer';
    let assignedId: number | null = null;
    let sensorCount = 0;
    let sensorOrder: string[] = [];

    try {
      const result = new Promise<void>((resolve, reject) => {
        const timeout = setTimeout(() => {
          reject(new Error('timed out waiting for assign/sensors'));
        }, 4000);

        ws.on('error', (err: Error) => {
          clearTimeout(timeout);
          reject(err);
        });

        ws.on('message', (data: RawData, isBinary: boolean) => {
          if (isBinary) return;
          const msg = parseJsonMessage(data);
          if (!msg) return;
          if (msg['type'] === 'welcome') {
            const spec = msg['sensorSpec'];
            if (spec && typeof spec === 'object') {
              sensorCount = typeof (spec as { sensorCount?: unknown })['sensorCount'] === 'number'
                ? (spec as { sensorCount?: number })['sensorCount'] ?? 0
                : 0;
              const order = (spec as { order?: unknown }).order;
              sensorOrder = Array.isArray(order) ? order.filter(item => typeof item === 'string') : [];
            }
          }
          if (msg['type'] === 'assign') {
            assignedId = typeof msg['snakeId'] === 'number' ? msg['snakeId'] : null;
            return;
          }
          if (msg['type'] === 'sensors') {
            if (!assignedId || msg['snakeId'] !== assignedId) {
              clearTimeout(timeout);
              reject(new Error('sensor snakeId mismatch'));
              return;
            }
            if (sensorCount && Array.isArray(msg['sensors']) && msg['sensors'].length !== sensorCount) {
              clearTimeout(timeout);
              reject(new Error('sensor length mismatch'));
              return;
            }
            clearTimeout(timeout);
            resolve();
          }
        });

        ws.on('open', () => {
          ws.send(JSON.stringify({ type: 'hello', clientType: 'bot', version: 1 }));
          ws.send(JSON.stringify({ type: 'join', mode: 'player', name: 'test-bot' }));
        });
      });

      await result;
    } finally {
      ws.close();
      await server.close();
    }

    expect(assignedId).toBeTruthy();
    const layout = getSensorLayout(16, 'v2');
    expect(sensorCount).toBe(layout.inputSize);
    expect(sensorOrder.length).toBe(layout.inputSize);
    expect(sensorOrder.slice(0, 7)).toEqual(layout.order.slice(0, 7));
  }, 20000);
});
