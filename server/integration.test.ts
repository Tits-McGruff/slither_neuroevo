import { describe, it, expect } from 'vitest';
import WebSocket, { type RawData } from 'ws';
import { startServer } from './index.ts';
import { DEFAULT_CONFIG } from './config.ts';
import { getSensorLayout } from '../src/protocol/sensors.ts';
import type { PopulationSnapshotPayload } from './persistence.ts';

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
    dbPath: ':memory:',
    inferenceBackend: 'js',
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
  it('rejects Protocol 1 with an explicit incompatibility error', async () => {
    const server = await startServerWithGuard();
    if (!server) return;
    const ws = new WebSocket(server.wsUrl);

    try {
      const result = await new Promise<{ message: string; code: number }>((resolve, reject) => {
        let message = '';
        const timeout = setTimeout(() => reject(new Error('timed out waiting for version rejection')), 5000);
        ws.on('error', reject);
        ws.on('message', (data: RawData) => {
          const parsed = parseJsonMessage(data);
          if (parsed?.['type'] === 'error' && typeof parsed['message'] === 'string') {
            message = parsed['message'];
          }
        });
        ws.on('close', (code: number) => {
          clearTimeout(timeout);
          resolve({ message, code });
        });
        ws.on('open', () => {
          ws.send(JSON.stringify({ type: 'hello', clientType: 'ui', version: 1 }));
        });
      });

      expect(result.code).toBe(1008);
      expect(result.message).toContain('server requires 2');
    } finally {
      ws.close();
      await server.close();
    }
  }, 10000);

  it('handshakes and streams frames', async () => {
    const server = await startServerWithGuard();
    if (!server) return;

    const ws = new WebSocket(server.wsUrl);
    ws.binaryType = 'arraybuffer';

    const seen = { welcome: false, frame: false };

    try {
      const result = new Promise<void>((resolve, reject) => {
        let settled = false;
        const timeout = setTimeout(() => {
          if (settled) return;
          settled = true;
          reject(new Error('timed out waiting for welcome/frame'));
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
        }, 10000);

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
          ws.send(JSON.stringify({ type: 'hello', clientType: 'bot', version: 2 }));
          ws.send(JSON.stringify({ type: 'join', mode: 'player', name: 'test-bot' }));
        });
      });

      await result;
    } finally {
      ws.close();
      await server.close();
    }

    expect(assignedId).toBeTruthy();
    const layout = getSensorLayout(16, 'v3');
    expect(sensorCount).toBe(layout.inputSize);
    expect(sensorOrder.length).toBe(layout.inputSize);
    expect(sensorOrder.slice(0, 7)).toEqual(layout.order.slice(0, 7));
  }, 20000);

  it('reports live config identity through dynamic HTTP getters and save payloads', async () => {
    const server = await startServerWithGuard();
    if (!server) return;
    const httpBase = `http://127.0.0.1:${server.port}`;
    let ws: WebSocket | null = null;

    try {
      const initialHealth = await fetch(`${httpBase}/health`).then(async response =>
        response.json() as Promise<{ configRevision: number; configHash: string }>);
      ws = new WebSocket(server.wsUrl);
      const socket = ws;
      const applied = await new Promise<Record<string, unknown>>((resolve, reject) => {
        const timeout = setTimeout(() => reject(new Error('timed out waiting for settings result')), 5000);
        socket.on('error', reject);
        socket.on('close', (code: number, reason: Buffer) => {
          clearTimeout(timeout);
          reject(new Error(`settings socket closed (${code}): ${reason.toString('utf8')}`));
        });
        socket.on('message', (data: RawData) => {
          const msg = parseJsonMessage(data);
          if (msg?.['type'] === 'welcome') {
            socket.send(JSON.stringify({ type: 'join', mode: 'spectator' }));
            socket.send(JSON.stringify({
              type: 'reset',
              settings: {
                snakeCount: 2,
                simSpeed: 1,
                hiddenLayers: 1,
                neurons1: 8,
                neurons2: 8,
                neurons3: 8,
                neurons4: 8,
                neurons5: 8
              },
              updates: [
                { path: 'baselineBots.count', value: 0 },
                { path: 'pelletCountTarget', value: 100 }
              ]
            }));
            setTimeout(() => {
              socket.send(JSON.stringify({
                type: 'settings',
                requestId: 'http-state',
                updates: [{ path: 'observer.zoomLerpFollow', value: 0.2 }]
              }));
            }, 100);
            return;
          }
          if (msg?.['type'] === 'error') {
            clearTimeout(timeout);
            reject(new Error(String(msg['message'])));
            return;
          }
          if (msg?.['type'] !== 'settingsApplied') return;
          clearTimeout(timeout);
          resolve(msg);
        });
        socket.on('open', () => {
          socket.send(JSON.stringify({ type: 'hello', clientType: 'ui', version: 2 }));
        });
      });
      const updatedHealth = await fetch(`${httpBase}/health`).then(async response =>
        response.json() as Promise<{ configRevision: number; configHash: string }>);
      const saveResponse = await fetch(`${httpBase}/api/save`, { method: 'POST' });
      expect(saveResponse.ok).toBe(true);
      const exported = await fetch(`${httpBase}/api/export/latest`).then(async response =>
        response.json() as Promise<PopulationSnapshotPayload>);

      expect(applied['configRevision']).toBe(2);
      expect(updatedHealth.configRevision).toBe(2);
      expect(updatedHealth.configHash).not.toBe(initialHealth.configHash);
      expect(exported.cfgHash).toBe(updatedHealth.configHash);
      expect(exported.worldSeed).toBe(
        (await fetch(`${httpBase}/health`).then(async response =>
          response.json() as Promise<{ run: { seed: number } }>)).run.seed
      );
      const metadataOnlySeed = (exported.worldSeed + 1) >>> 0;
      const imported = await fetch(`${httpBase}/api/import`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ ...exported, worldSeed: metadataOnlySeed })
      }).then(async response => {
        expect(response.ok).toBe(true);
        return response.json() as Promise<{
          importedWorldSeed: number;
          activeWorldSeed: number;
          seedApplied: boolean;
          seedDisposition: string;
        }>;
      });
      expect(imported).toMatchObject({
        importedWorldSeed: metadataOnlySeed,
        activeWorldSeed: exported.worldSeed,
        seedApplied: false,
        seedDisposition: expect.stringContaining('metadata-only')
      });
    } finally {
      ws?.close();
      await server.close();
    }
  }, 10000);
});
