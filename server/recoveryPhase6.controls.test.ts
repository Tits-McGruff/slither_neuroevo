import { beforeEach, describe, expect, it } from 'vitest';
import { CFG, resetCFGToDefaults } from '../src/config.ts';
import type { SimCore } from '../src/sim/SimCore.ts';
import type { BatchInferenceRunner } from '../src/world.ts';
import type { ServerMessage, WelcomeMsg } from './protocol.ts';
import { DEFAULT_CONFIG } from './config.ts';
import { SimServer, coerceCoreSettings } from './simServer.ts';
import type { WsHub } from './wsHub.ts';

/** Direct server message captured for one requesting connection. */
interface DirectMessage {
  /** Target connection id. */
  connId: number;
  /** Structured response. */
  message: ServerMessage;
}

/** Observable fake hub state used by Phase 6 server tests. */
interface HubProbe {
  /** Direct requester-only responses. */
  direct: DirectMessage[];
  /** Broadcasts emitted to joined UI clients. */
  broadcasts: ServerMessage[];
  /** Broadcast stream observed by each fake joined client. */
  byClient: Map<number, ServerMessage[]>;
  /** Latest future-handshake patch. */
  welcomePatch: Partial<WelcomeMsg>;
}

/** Private SimServer surfaces intentionally exercised at deterministic boundaries. */
interface SimServerAccess {
  /** Unified core owned by the server. */
  core: SimCore;
  /** Boundary drain invoked by SimCore immediately before a fixed step. */
  drainPendingCommands: (stepId: number) => void;
}

/**
 * Build a fake hub that preserves broadcast order for two joined UI clients.
 * @returns Hub cast and observable probe.
 */
function buildHub(): { hub: WsHub; probe: HubProbe } {
  const probe: HubProbe = {
    direct: [],
    broadcasts: [],
    byClient: new Map([[1, []], [2, []]]),
    welcomePatch: {}
  };
  const hub = {
    sendJsonTo: (connId: number, message: ServerMessage) => {
      probe.direct.push({ connId, message });
    },
    broadcastJsonToUi: (message: ServerMessage) => {
      probe.broadcasts.push(message);
      for (const messages of probe.byClient.values()) messages.push(message);
    },
    updateWelcome: (patch: Partial<WelcomeMsg>) => {
      probe.welcomePatch = { ...probe.welcomePatch, ...patch };
    },
    updateSensorSpec: () => {},
    broadcastError: () => {},
    hasFrameRecipients: () => false,
    broadcastFrame: () => {},
    broadcastStats: () => {}
  } as unknown as WsHub;
  return { hub, probe };
}

/**
 * Build a small serial SimServer without starting its wall-clock loop.
 * @returns Server, private test access, and fake-hub observations.
 */
function buildServer(): { server: SimServer; access: SimServerAccess; probe: HubProbe } {
  const { hub, probe } = buildHub();
  const server = new SimServer(
    { ...DEFAULT_CONFIG, mtEnabled: false, inferenceBackend: 'js' },
    hub,
    undefined,
    '',
    1234,
    { snakeCount: 4, simSpeed: 1 },
    'phase6-run'
  );
  return { server, access: server as unknown as SimServerAccess, probe };
}

/**
 * Advance exactly one step at the speed active before the scheduler pump.
 * @param core - Core to advance.
 */
async function advanceOneStep(core: SimCore): Promise<void> {
  const speed = Math.max(0.01, core.world.simSpeed);
  await core.update(core.fixedDt / speed);
}

/**
 * Extract alive snake ids from the authoritative serialized frame layout.
 * @param frame - Serialized world frame.
 * @returns Alive ids in serialized order.
 */
function readSerializedSnakeIds(frame: Float32Array): number[] {
  const aliveCount = frame[2] ?? 0;
  const ids: number[] = [];
  let pointer = 7;
  for (let index = 0; index < aliveCount; index++) {
    ids.push(frame[pointer] ?? -1);
    const pointCount = frame[pointer + 7] ?? 0;
    pointer += 8 + pointCount * 2;
  }
  return ids;
}

/**
 * Find one serialized snake head in the authoritative frame layout.
 * @param frame - Serialized world frame.
 * @param snakeId - Snake id to locate.
 * @returns Serialized head coordinates, or null when absent.
 */
function readSerializedSnakeHead(
  frame: Float32Array,
  snakeId: number
): { x: number; y: number } | null {
  const aliveCount = frame[2] ?? 0;
  let pointer = 7;
  for (let index = 0; index < aliveCount; index++) {
    const id = frame[pointer] ?? -1;
    const pointCount = frame[pointer + 7] ?? 0;
    if (id === snakeId) {
      return { x: frame[pointer + 3] ?? 0, y: frame[pointer + 4] ?? 0 };
    }
    pointer += 8 + pointCount * 2;
  }
  return null;
}

describe('Phase 6 authoritative controls', () => {
  beforeEach(() => {
    resetCFGToDefaults();
    CFG.baselineBots.count = 0;
    CFG.pelletCountTarget = 50;
    CFG.pelletSpawnPerSecond = 0;
  });

  it('bounds reset-time core settings with shared metadata', () => {
    expect(coerceCoreSettings({
      snakeCount: 10000,
      simSpeed: -1,
      hiddenLayers: 99,
      neurons1: 10000
    })).toEqual({
      snakeCount: 300,
      simSpeed: 0.1,
      hiddenLayers: 5,
      neurons1: 256
    });
  });

  it('applies an atomic normalized batch once and rejects reset-required batches', async () => {
    const { server, access, probe } = buildServer();
    const initialHash = server.getConfigHash();
    server.handleSettings(1, {
      type: 'settings',
      requestId: 'batch-1',
      updates: [
        { path: 'simSpeed', value: 100 },
        { path: 'sense.maxPelletChecks', value: 123.6 }
      ]
    });
    await advanceOneStep(access.core);

    const applied = probe.broadcasts[0];
    expect(applied).toMatchObject({
      type: 'settingsApplied',
      requestId: 'batch-1',
      applied: true,
      configRevision: 1,
      sequence: 1,
      step: 1,
      updates: [
        { path: 'simSpeed', value: 12 },
        { path: 'sense.maxPelletChecks', value: 124 }
      ]
    });
    expect(access.core.world.simSpeed).toBe(12);
    expect(CFG.sense.maxPelletChecks).toBe(124);
    expect(server.getConfigHash()).not.toBe(initialHash);

    server.handleSettings(1, {
      type: 'settings',
      requestId: 'batch-2',
      updates: [
        { path: 'simSpeed', value: 2 },
        { path: 'collision.cellSize', value: 80 }
      ]
    });
    await advanceOneStep(access.core);

    expect(server.getConfigState().configRevision).toBe(1);
    expect(access.core.world.simSpeed).toBe(12);
    expect(probe.direct.at(-1)?.message).toMatchObject({
      type: 'settingsApplied',
      requestId: 'batch-2',
      applied: false,
      reason: 'setting requires reset: collision.cellSize'
    });
  });

  it('orders racing clients and broadcasts one convergent revision stream', async () => {
    const { server, access, probe } = buildServer();
    server.handleSettings(1, {
      type: 'settings',
      requestId: 'client-1',
      updates: [{ path: 'simSpeed', value: 2 }]
    });
    server.handleSettings(2, {
      type: 'settings',
      requestId: 'client-2',
      updates: [{ path: 'simSpeed', value: 3 }]
    });
    await advanceOneStep(access.core);

    expect(probe.broadcasts.map((message) => ({
      requestId: 'requestId' in message ? message.requestId : null,
      revision: 'configRevision' in message ? message.configRevision : null,
      sequence: 'sequence' in message ? message.sequence : null
    }))).toEqual([
      { requestId: 'client-1', revision: 1, sequence: 1 },
      { requestId: 'client-2', revision: 2, sequence: 2 }
    ]);
    expect(probe.byClient.get(1)).toEqual(probe.byClient.get(2));
    expect(access.core.world.simSpeed).toBe(3);
  });

  it('holds an update received during inference until the following boundary', async () => {
    const { server, access, probe } = buildServer();
    let releaseInference!: () => void;
    let reportStarted!: () => void;
    const inferenceStarted = new Promise<void>((resolve) => {
      reportStarted = resolve;
    });
    const inferenceGate = new Promise<void>((resolve) => {
      releaseInference = resolve;
    });
    let calls = 0;
    const delayedRunner: BatchInferenceRunner = {
      runBatch: async (_inputs, outputs) => {
        outputs.fill(0);
        calls += 1;
        if (calls === 1) {
          reportStarted();
          await inferenceGate;
        }
      }
    };
    access.core.brainPool = delayedRunner;

    const update = access.core.update(access.core.fixedDt * 2);
    await inferenceStarted;
    server.handleSettings(1, {
      type: 'settings',
      requestId: 'during-inference',
      updates: [{ path: 'simSpeed', value: 2 }]
    });
    expect(probe.broadcasts).toHaveLength(0);
    expect(access.core.world.simSpeed).toBe(1);
    releaseInference();
    await update;

    expect(probe.broadcasts[0]).toMatchObject({
      type: 'settingsApplied',
      requestId: 'during-inference',
      applied: true,
      step: 2
    });
    expect(access.core.world.simSpeed).toBe(2);
  });

  it('changes subsequent generation-time rate through authoritative sim speed', async () => {
    const { server, access } = buildServer();
    const start = access.core.world.generationTime;
    server.handleSettings(1, {
      type: 'settings',
      requestId: 'speed',
      updates: [{ path: 'simSpeed', value: 2 }]
    });
    await access.core.update(access.core.fixedDt);
    const afterBoundary = access.core.world.generationTime;
    await access.core.update(access.core.fixedDt);

    expect(afterBoundary - start).toBeCloseTo(access.core.fixedDt, 8);
    expect(access.core.world.generationTime - afterBoundary).toBeCloseTo(
      access.core.fixedDt * 2,
      8
    );
  });

  it('kills through normal death drops and moves every body point with valid spatial state', () => {
    const { server, access, probe } = buildServer();
    const world = access.core.world;
    const moved = world.snakes[0]!;
    const beforePoints = moved.points.map((point) => ({ ...point }));
    const beforeHead = { x: moved.x, y: moved.y };
    server.handleGodMode(1, {
      type: 'godMode',
      requestId: 'move',
      action: 'move',
      snakeId: moved.id,
      x: moved.x + 10,
      y: moved.y + 5
    });
    access.drainPendingCommands(1);

    const moveResult = probe.direct.at(-1)?.message;
    expect(moveResult).toMatchObject({ type: 'godModeResult', applied: true, step: 1 });
    const dx = moved.x - beforeHead.x;
    const dy = moved.y - beforeHead.y;
    moved.points.forEach((point, index) => {
      expect(point.x - beforePoints[index]!.x).toBeCloseTo(dx, 8);
      expect(point.y - beforePoints[index]!.y).toBeCloseTo(dy, 8);
      expect(Math.hypot(point.x, point.y)).toBeLessThanOrEqual(world.worldRadius - moved.radius + 1e-6);
    });
    const collisionIndex = Math.max(1, Math.floor(CFG.collision.skipSegments));
    const first = moved.points[collisionIndex - 1]!;
    const second = moved.points[collisionIndex]!;
    const hits: number[] = [];
    world._collGrid.query(
      (first.x + second.x) * 0.5,
      (first.y + second.y) * 0.5,
      (snake) => hits.push(snake.id)
    );
    expect(hits).toContain(moved.id);
    const serializedHead = readSerializedSnakeHead(access.core.serialize(), moved.id);
    expect(serializedHead?.x).toBeCloseTo(moved.x, 3);
    expect(serializedHead?.y).toBeCloseTo(moved.y, 3);

    const killed = world.snakes[1]!;
    const pelletsBefore = world.pellets.length;
    server.handleGodMode(1, {
      type: 'godMode',
      requestId: 'kill',
      action: 'kill',
      snakeId: killed.id
    });
    access.drainPendingCommands(1);

    expect(killed.alive).toBe(false);
    expect(world.pellets.length).toBeGreaterThan(pelletsBefore);
    expect(readSerializedSnakeIds(access.core.serialize())).not.toContain(killed.id);
  });

  it('rejects invalid God Mode targets and unsafe finite coordinates', () => {
    const { server, access, probe } = buildServer();
    server.handleGodMode(1, {
      type: 'godMode',
      requestId: 'missing',
      action: 'kill',
      snakeId: 999999999
    });
    server.handleGodMode(1, {
      type: 'godMode',
      requestId: 'unsafe',
      action: 'move',
      snakeId: access.core.world.snakes[0]!.id,
      x: 1e308,
      y: 1e308
    });
    access.drainPendingCommands(1);

    expect(probe.direct.slice(-2).map((entry) => entry.message)).toEqual([
      expect.objectContaining({ type: 'godModeResult', requestId: 'missing', applied: false }),
      expect.objectContaining({ type: 'godModeResult', requestId: 'unsafe', applied: false })
    ]);
  });

  it('rejects Protocol 2 New Run when durable persistence is unavailable', async () => {
    const { server, probe } = buildServer();
    const identityBefore = server.getRunIdentity();
    await server.handleNewRun(1, { type: 'newRun', requestId: 'new-run' });

    expect(probe.direct.at(-1)?.message).toMatchObject({
      type: 'newRunResult',
      requestId: 'new-run',
      applied: false,
      reason: expect.stringContaining('durable persistence')
    });
    expect(server.getRunIdentity()).toEqual(identityBefore);
  });
});
