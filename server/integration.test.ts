import { it, expect } from 'vitest';
import WebSocket, { type RawData } from 'ws';
import { startServer } from './index.ts';
import { DEFAULT_CONFIG } from './config.ts';
import { getSensorLayout } from '../src/protocol/sensors.ts';
import type { PopulationSnapshotPayload } from './persistence.ts';
import { describeNetworkSuite } from './test/networkSuites.ts';

/** One alive snake decoded from the authoritative binary frame. */
interface SerializedSnake {
  /** Stable snake identifier. */
  id: number;
  /** Serialized head X coordinate. */
  x: number;
  /** Serialized head Y coordinate. */
  y: number;
}

/** Minimal binary-frame state required by network integration assertions. */
interface SerializedFrame {
  /** Authoritative world radius. */
  worldRadius: number;
  /** Authoritative camera zoom. */
  zoom: number;
  /** Alive snakes decoded from the compact frame. */
  snakes: SerializedSnake[];
}

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

/** Start one isolated JS server for network integration. */
async function startIntegrationServer() {
  return startServer({
    ...DEFAULT_CONFIG,
    port: 0,
    dbPath: ':memory:',
    resume: 'fresh',
    inferenceBackend: 'js',
    logLevel: 'error'
  });
}

/**
 * Decode the binary frame fields exercised by live settings and God Mode.
 * @param data - Raw binary WebSocket payload.
 * @returns Parsed frame, or null when the compact layout is malformed.
 */
function parseSerializedFrame(data: RawData): SerializedFrame | null {
  const bytes = Array.isArray(data)
    ? Buffer.concat(data)
    : data instanceof ArrayBuffer
      ? Buffer.from(data)
      : Buffer.from(data.buffer, data.byteOffset, data.byteLength);
  if (bytes.byteLength % Float32Array.BYTES_PER_ELEMENT !== 0) return null;
  const copy = Uint8Array.from(bytes);
  const frame = new Float32Array(copy.buffer);
  if (frame.length < 7) return null;

  const aliveCount = frame[2] ?? -1;
  if (!Number.isInteger(aliveCount) || aliveCount < 0) return null;
  const snakes: SerializedSnake[] = [];
  let pointer = 7;
  for (let index = 0; index < aliveCount; index++) {
    if (pointer + 8 > frame.length) return null;
    const pointCount = frame[pointer + 7] ?? -1;
    if (!Number.isInteger(pointCount) || pointCount < 0) return null;
    const nextPointer = pointer + 8 + pointCount * 2;
    if (nextPointer > frame.length) return null;
    snakes.push({
      id: frame[pointer] ?? -1,
      x: frame[pointer + 3] ?? 0,
      y: frame[pointer + 4] ?? 0
    });
    pointer = nextPointer;
  }
  return {
    worldRadius: frame[3] ?? 0,
    zoom: frame[6] ?? 0,
    snakes
  };
}

describeNetworkSuite('server integration', () => {
  it('rejects Protocol 1 with an explicit incompatibility error', async () => {
    const server = await startIntegrationServer();
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
    const server = await startIntegrationServer();

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
    const server = await startIntegrationServer();

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

  it('reclaims the same controller lease over a new socket within wall-time grace', async () => {
    const server = await startIntegrationServer();
    const sockets: WebSocket[] = [];
    try {
      const first = await new Promise<{ snakeId: number; resumeToken: string }>((resolve, reject) => {
        const ws = new WebSocket(server.wsUrl);
        sockets.push(ws);
        const timeout = setTimeout(() => reject(new Error('timed out waiting for first assignment')), 5000);
        ws.on('error', reject);
        ws.on('message', (data: RawData, isBinary: boolean) => {
          if (isBinary) return;
          const msg = parseJsonMessage(data);
          if (
            msg?.['type'] === 'assign' &&
            typeof msg['snakeId'] === 'number' &&
            typeof msg['resumeToken'] === 'string'
          ) {
            clearTimeout(timeout);
            resolve({ snakeId: msg['snakeId'], resumeToken: msg['resumeToken'] });
          }
        });
        ws.on('open', () => {
          ws.send(JSON.stringify({ type: 'hello', clientType: 'bot', version: 2 }));
          ws.send(JSON.stringify({ type: 'join', mode: 'player', name: 'reclaim-bot' }));
        });
      });
      await new Promise<void>((resolve) => {
        sockets[0]!.once('close', () => resolve());
        sockets[0]!.close();
      });

      const reclaimed = await new Promise<{
        resultSeen: boolean;
        snakeId: number;
        resumeToken: string;
      }>((resolve, reject) => {
        const ws = new WebSocket(server.wsUrl);
        sockets.push(ws);
        let resultSeen = false;
        const timeout = setTimeout(() => reject(new Error('timed out waiting for reclaim')), 5000);
        ws.on('error', reject);
        ws.on('message', (data: RawData, isBinary: boolean) => {
          if (isBinary) return;
          const msg = parseJsonMessage(data);
          if (msg?.['type'] === 'reclaimResult' && msg['reclaimed'] === true) {
            resultSeen = true;
          }
          if (
            msg?.['type'] === 'assign' &&
            msg['reclaimed'] === true &&
            typeof msg['snakeId'] === 'number' &&
            typeof msg['resumeToken'] === 'string'
          ) {
            clearTimeout(timeout);
            resolve({
              resultSeen,
              snakeId: msg['snakeId'],
              resumeToken: msg['resumeToken']
            });
          }
        });
        ws.on('open', () => {
          ws.send(JSON.stringify({ type: 'hello', clientType: 'bot', version: 2 }));
          ws.send(JSON.stringify({
            type: 'join',
            mode: 'player',
            name: 'reclaim-bot',
            resumeToken: first.resumeToken
          }));
        });
      });

      expect(reclaimed.resultSeen).toBe(true);
      expect(reclaimed.snakeId).toBe(first.snakeId);
      expect(reclaimed.resumeToken).not.toBe(first.resumeToken);
    } finally {
      for (const socket of sockets) socket.close();
      await server.close();
    }
  }, 20000);

  it('supports trusted-LAN CORS requests and preflight without treating CORS as authentication', async () => {
    const server = await startIntegrationServer();
    const httpBase = `http://127.0.0.1:${server.port}`;
    const lanOrigin = 'http://192.168.1.25:5173';

    try {
      const response = await fetch(`${httpBase}/health`, {
        headers: { Origin: lanOrigin }
      });
      expect(response.ok).toBe(true);
      expect(response.headers.get('access-control-allow-origin')).toBe(lanOrigin);
      expect(response.headers.get('access-control-allow-credentials')).toBe('true');
      expect(response.headers.get('vary')).toContain('Origin');

      const preflight = await fetch(`${httpBase}/api/save`, {
        method: 'OPTIONS',
        headers: {
          Origin: lanOrigin,
          'Access-Control-Request-Method': 'POST',
          'Access-Control-Request-Headers': 'content-type'
        }
      });
      expect(preflight.status).toBe(204);
      expect(preflight.headers.get('access-control-allow-origin')).toBe(lanOrigin);
      expect(preflight.headers.get('access-control-allow-methods')).toContain('POST');

      const untrustedOrigin = await fetch(`${httpBase}/health`, {
        headers: { Origin: 'https://example.com' }
      });
      expect(untrustedOrigin.headers.get('access-control-allow-origin')).toBe('*');
      expect(untrustedOrigin.headers.get('access-control-allow-credentials')).toBeNull();
    } finally {
      await server.close();
    }
  }, 10000);

  it('reports live config identity through dynamic HTTP getters and save payloads', async () => {
    const server = await startIntegrationServer();
    const httpBase = `http://127.0.0.1:${server.port}`;
    let ws: WebSocket | null = null;

    try {
      const initialHealth = await fetch(`${httpBase}/health`).then(async response =>
        response.json() as Promise<{ configRevision: number; configHash: string }>);
      ws = new WebSocket(server.wsUrl);
      ws.binaryType = 'arraybuffer';
      const socket = ws;
      const applied = await new Promise<Record<string, unknown>>((resolve, reject) => {
        let didRequestSettings = false;
        const timeout = setTimeout(() => reject(new Error('timed out waiting for settings result')), 15000);
        socket.on('error', reject);
        socket.on('close', (code: number, reason: Buffer) => {
          clearTimeout(timeout);
          reject(new Error(`settings socket closed (${code}): ${reason.toString('utf8')}`));
        });
        socket.on('message', (data: RawData, isBinary: boolean) => {
          if (isBinary) {
            const frame = parseSerializedFrame(data);
            if (frame?.snakes.length === 2 && !didRequestSettings) {
              didRequestSettings = true;
              socket.send(JSON.stringify({
                type: 'settings',
                requestId: 'http-state',
                updates: [{ path: 'observer.zoomLerpFollow', value: 0.2 }]
              }));
            }
            return;
          }
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
  }, 20000);

  it('applies live settings and God Mode through WebSocket into serialized frames', async () => {
    const server = await startIntegrationServer();
    const ws = new WebSocket(server.wsUrl);
    ws.binaryType = 'arraybuffer';
    let initialZoom = 0;
    let updatedZoom = 0;
    let targetSnakeId = -1;
    let targetX = 0;
    let targetY = 0;
    let movedDistance = Number.POSITIVE_INFINITY;
    let sawKilledFrame = false;

    try {
      await new Promise<void>((resolve, reject) => {
        let phase = 'welcome';
        let settled = false;
        const timeout = setTimeout(() => {
          if (settled) return;
          settled = true;
          reject(new Error(`timed out during WebSocket control phase: ${phase}`));
        }, 15000);
        const fail = (error: unknown): void => {
          if (settled) return;
          settled = true;
          clearTimeout(timeout);
          reject(error instanceof Error ? error : new Error(String(error)));
        };

        ws.on('error', fail);
        ws.on('close', (code: number, reason: Buffer) => {
          fail(new Error(`control socket closed (${code}): ${reason.toString('utf8')}`));
        });
        ws.on('message', (data: RawData, isBinary: boolean) => {
          if (settled) return;
          if (!isBinary) {
            const message = parseJsonMessage(data);
            if (message?.['type'] === 'error') {
              fail(new Error(String(message['message'])));
              return;
            }
            if (message?.['type'] === 'welcome' && phase === 'welcome') {
              phase = 'reset-frame';
              ws.send(JSON.stringify({ type: 'join', mode: 'spectator' }));
              ws.send(JSON.stringify({
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
              return;
            }
            if (message?.['type'] === 'settingsApplied' && phase === 'settings-result') {
              if (message['applied'] !== true) {
                fail(new Error(`live settings rejected: ${String(message['reason'])}`));
                return;
              }
              phase = 'settings-frame';
              return;
            }
            if (message?.['type'] === 'godModeResult' && phase === 'move-result') {
              if (message['requestId'] !== 'integration-move' || message['applied'] !== true) {
                fail(new Error(`God Mode move rejected: ${String(message['reason'])}`));
                return;
              }
              phase = 'move-frame';
              return;
            }
            if (message?.['type'] === 'godModeResult' && phase === 'kill-result') {
              if (message['requestId'] !== 'integration-kill' || message['applied'] !== true) {
                fail(new Error(`God Mode kill rejected: ${String(message['reason'])}`));
                return;
              }
              phase = 'kill-frame';
            }
            return;
          }

          const frame = parseSerializedFrame(data);
          if (!frame) {
            fail(new Error('received a malformed authoritative frame'));
            return;
          }
          if (phase === 'reset-frame' && frame.snakes.length === 2) {
            initialZoom = frame.zoom;
            phase = 'settings-result';
            ws.send(JSON.stringify({
              type: 'settings',
              requestId: 'integration-settings',
              updates: [
                { path: 'observer.overviewPadding', value: 1.8 },
                { path: 'observer.zoomLerpOverview', value: 1 }
              ]
            }));
            return;
          }
          if (phase === 'settings-frame' && frame.zoom < initialZoom - 0.01) {
            const snake = frame.snakes[0];
            if (!snake) {
              fail(new Error('no snake remained for God Mode integration'));
              return;
            }
            updatedZoom = frame.zoom;
            targetSnakeId = snake.id;
            const targetOffset = Math.min(500, frame.worldRadius * 0.25);
            targetX = snake.x >= 0 ? -targetOffset : targetOffset;
            targetY = snake.y >= 0 ? -targetOffset : targetOffset;
            phase = 'move-result';
            ws.send(JSON.stringify({
              type: 'godMode',
              requestId: 'integration-move',
              action: 'move',
              snakeId: targetSnakeId,
              x: targetX,
              y: targetY
            }));
            return;
          }
          if (phase === 'move-frame') {
            const moved = frame.snakes.find(snake => snake.id === targetSnakeId);
            if (!moved) return;
            movedDistance = Math.hypot(moved.x - targetX, moved.y - targetY);
            if (movedDistance > 25) return;
            phase = 'kill-result';
            ws.send(JSON.stringify({
              type: 'godMode',
              requestId: 'integration-kill',
              action: 'kill',
              snakeId: targetSnakeId
            }));
            return;
          }
          if (phase === 'kill-frame' && !frame.snakes.some(snake => snake.id === targetSnakeId)) {
            sawKilledFrame = true;
            settled = true;
            clearTimeout(timeout);
            resolve();
          }
        });
        ws.on('open', () => {
          ws.send(JSON.stringify({ type: 'hello', clientType: 'ui', version: 2 }));
        });
      });
    } finally {
      ws.close();
      await server.close();
    }

    expect(updatedZoom).toBeLessThan(initialZoom);
    expect(movedDistance).toBeLessThanOrEqual(25);
    expect(sawKilledFrame).toBe(true);
  }, 20000);
});
