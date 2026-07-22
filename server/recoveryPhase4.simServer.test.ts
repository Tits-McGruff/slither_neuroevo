import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { CFG, resetCFGToDefaults } from '../src/config.ts';
import { prepareInferenceBackend } from '../src/brains/nativeBridge.ts';
import { graphKey } from '../src/brains/graph/compiler.ts';
import type { GraphSpec } from '../src/brains/graph/schema.ts';
import type { SimCore } from '../src/sim/SimCore.ts';
import type { StatsMsg, VizMsg } from './protocol.ts';
import { captureAuthoritativeWorldDigest } from './test/authoritativeWorldDigest.ts';
import { DEFAULT_CONFIG } from './config.ts';
import { BrainPool } from './brainPool.ts';
import { SimServer } from './simServer.ts';
import type { WsHub } from './wsHub.ts';

/** Phase 4 production integration suite label. */
const SUITE = 'recovery Phase 4 — SimServer canonical MT integration';
/** Servers awaiting worker cleanup if an assertion interrupts a test. */
const activeServers = new Set<SimServer>();

/** Observable WebSocket side effects used by server failure assertions. */
interface WsHubProbe {
  /** Structured failure broadcasts. */
  errors: string[];
  /** Successfully published binary frames. */
  frames: Float32Array[];
  /** Successfully published statistics messages. */
  stats: StatsMsg[];
}

/** Private production seams exercised by focused component tests. */
interface SimServerAccess {
  /** Unified core owned by the server. */
  core: SimCore;
  /** Canonical pool attached to the core. */
  brainPool: BrainPool | null;
  /** Generation whose weights are installed in the pool. */
  mtGeneration: number;
  /** Whether the timer loop may schedule more work. */
  running: boolean;
  /** Next loop deadline. */
  nextTickAt: number;
  /** Previous tick timestamp. */
  lastTickAt: number;
  /** Last successful frame timestamp. */
  lastFrameSentAt: number;
  /** Last successful stats timestamp. */
  lastStatsSentAt: number;
  /** Run one timer-loop iteration. */
  loop: () => Promise<void>;
  /** Run one production tick without scheduling another. */
  tick: (now: number) => Promise<void>;
  /** Build one current statistics payload. */
  buildStats: () => StatsMsg;
}

beforeEach(() => {
  resetCFGToDefaults();
});

afterEach(async () => {
  await Promise.all(Array.from(activeServers, async (server) => server.stop()));
  activeServers.clear();
  resetCFGToDefaults();
});

/**
 * Build a WebSocket hub stub and observable publication probe.
 * @param hasFrameRecipients - Whether production tick should attempt a frame.
 * @returns Stub hub and captured outbound events.
 */
function buildWsHub(hasFrameRecipients = false): { hub: WsHub; probe: WsHubProbe } {
  const probe: WsHubProbe = { errors: [], frames: [], stats: [] };
  const hub = {
    sendJsonTo: () => undefined,
    updateSensorSpec: () => undefined,
    hasFrameRecipients: () => hasFrameRecipients,
    broadcastFrame: (frame: Float32Array) => probe.frames.push(frame.slice()),
    broadcastStats: (stats: StatsMsg) => probe.stats.push(stats),
    broadcastError: (message: string) => probe.errors.push(message)
  } as unknown as WsHub;
  return { hub, probe };
}

/**
 * Create a deterministic server with an initialized canonical worker pool.
 * @param workerCount - Requested worker count.
 * @param backend - Immutable JS or native backend.
 * @param hasFrameRecipients - Whether frame publication is observable.
 * @returns Server, private focused access, and outbound probe.
 */
async function createMtServer(
  workerCount: number,
  backend: 'js' | 'native',
  hasFrameRecipients = false
): Promise<{ server: SimServer; access: SimServerAccess; probe: WsHubProbe }> {
  await prepareInferenceBackend(backend);
  const { hub, probe } = buildWsHub(hasFrameRecipients);
  const server = new SimServer(
    {
      ...DEFAULT_CONFIG,
      mtEnabled: true,
      mtWorkers: workerCount,
      inferenceBackend: backend,
      checkpointEveryGenerations: 0
    },
    hub,
    undefined,
    'phase4-config',
    0x4a3b2c1d,
    { snakeCount: 8, simSpeed: 1 },
    'phase4-run'
  );
  activeServers.add(server);
  await server.initMT();
  return {
    server,
    access: server as unknown as SimServerAccess,
    probe
  };
}

/**
 * Drive exact fixed steps through the SimCore instance attached by SimServer.
 * @param access - Focused server internals.
 * @param stepCount - Number of exact steps to commit.
 */
async function driveFixedSteps(access: SimServerAccess, stepCount: number): Promise<void> {
  for (let step = 0; step < stepCount; step++) {
    await access.core.update(access.core.fixedDt);
  }
}

/**
 * Yield until a worker visualization response is cached.
 * @param pool - Canonical pool receiving the response.
 */
async function waitForVisualization(pool: BrainPool): Promise<void> {
  for (let attempt = 0; attempt < 100; attempt++) {
    if (pool.getCachedVisualization()) return;
    await new Promise<void>((resolve) => setTimeout(resolve, 5));
  }
  throw new Error('timed out waiting for the focused visualization response');
}

/**
 * Build a non-default graph that preserves the active sensor/output contract.
 * @returns Valid reset-time architecture override.
 */
function buildResetGraph(): GraphSpec {
  return {
    type: 'graph',
    nodes: [
      { id: 'input', type: 'Input', outputSize: CFG.brain.inSize },
      {
        id: 'hidden',
        type: 'MLP',
        inputSize: CFG.brain.inSize,
        outputSize: 5,
        hiddenSizes: [7]
      },
      { id: 'output', type: 'Dense', inputSize: 5, outputSize: CFG.brain.outSize }
    ],
    edges: [
      { from: 'input', to: 'hidden' },
      { from: 'hidden', to: 'output' }
    ],
    outputs: [{ nodeId: 'output' }],
    outputSize: CFG.brain.outSize
  };
}

describe(SUITE, () => {
  it('produces the same native seeded run for 1, 2, and 4 worker requests', async () => {
    const digests: string[] = [];
    for (const workerCount of [1, 2, 4]) {
      const { server, access } = await createMtServer(workerCount, 'native');
      const pool = access.brainPool;
      if (!pool) throw new Error('expected a canonical pool');
      expect(Array.from(pool.weightsView ?? [])).toEqual(
        server.getWorld().population.flatMap((genome) => Array.from(genome.weights))
      );
      expect(server.getInferenceMode()).toMatchObject({
        requestedBackend: 'native',
        activeBackend: 'native',
        requestedMt: true,
        activeWorkerCount: pool.workerCount,
        poolEpoch: pool.poolEpoch,
        weightEpoch: 1
      });
      expect(pool.getWorkerStatuses().every((status) => status.activeBackend === 'native'))
        .toBe(true);

      await driveFixedSteps(access, 6);
      expect(server.mtActive).toBe(false);
      digests.push(captureAuthoritativeWorldDigest(server.getWorld()).digest);
      await server.stop();
      activeServers.delete(server);
    }
    expect(digests[1]).toBe(digests[0]);
    expect(digests[2]).toBe(digests[0]);
  });

  it('synchronizes generation weights between fixed steps in one scheduler pump', async () => {
    const { server, access } = await createMtServer(2, 'js');
    const pool = access.brainPool;
    if (!pool) throw new Error('expected a canonical pool');
    const initialGeneration = server.getWorld().generation;
    CFG.generationSeconds = access.core.fixedDt * 0.5;

    const completed = await access.core.update(access.core.fixedDt * 2);

    expect(completed).toBe(2);
    expect(server.getWorld().generation).toBe(initialGeneration + 2);
    expect(access.mtGeneration).toBe(server.getWorld().generation);
    expect(pool.weightEpoch).toBe(3);
    expect(pool.getWorkerStatuses().every((status) => status.weightEpoch === 3)).toBe(true);
    expect(Array.from(pool.weightsView ?? [])).toEqual(
      server.getWorld().population.flatMap((genome) => Array.from(genome.weights))
    );
  });

  it('rebuilds at explicit New Run and keeps unowned native snakes serial', async () => {
    const { server, access } = await createMtServer(2, 'native');
    const firstPoolEpoch = access.brainPool?.poolEpoch;
    const world = server.getWorld();
    const external = world.spawnExternalSnake();
    expect(external.populationSlot).toBeNull();
    expect(external.brain.inferenceBackend).toBe('native');
    const genome = world.population[0];
    if (!genome) throw new Error('expected a population genome');
    const resurrectedId = world.resurrect(genome.toJSON());
    const resurrected = world.snakes.find((snake) => snake.id === resurrectedId);
    expect(resurrected?.populationSlot).toBeNull();
    expect(resurrected?.brain.inferenceBackend).toBe('native');

    const identity = await server.startNewRun();
    expect(identity.runId).not.toBe('phase4-run');
    expect(access.brainPool?.poolEpoch).toBeGreaterThan(firstPoolEpoch ?? 0);
    expect(access.brainPool?.weightEpoch).toBe(1);
    expect(server.getFaultStatus().faulted).toBe(false);
  });

  it('rebuilds deliberately after reset architecture and successful import', async () => {
    const { server, access } = await createMtServer(2, 'js');
    const initialPoolEpoch = access.brainPool?.poolEpoch;
    const graphSpec = buildResetGraph();
    await server.handleReset(1, {
      type: 'reset',
      settings: { snakeCount: 8, simSpeed: 1 },
      graphSpec
    });
    expect(access.brainPool?.poolEpoch).toBeGreaterThan(initialPoolEpoch ?? 0);
    expect(access.brainPool?.specKey).toBe(graphKey(graphSpec));
    expect(access.brainPool?.weightEpoch).toBe(1);

    const resetPoolEpoch = access.brainPool?.poolEpoch;
    const imported = await server.importPopulation(server.getWorld().exportPopulation());
    expect(imported.ok).toBe(true);
    expect(access.brainPool?.poolEpoch).toBeGreaterThan(resetPoolEpoch ?? 0);
    expect(access.brainPool?.specKey).toBe(graphKey(graphSpec));
    expect(access.brainPool?.weightEpoch).toBe(1);
  });

  it('publishes tagged selected-brain visualization while MT is active', async () => {
    const { server, access } = await createMtServer(2, 'js');
    access.lastTickAt = 0;
    access.lastFrameSentAt = Number.POSITIVE_INFINITY;
    access.lastStatsSentAt = Number.POSITIVE_INFINITY;
    await access.tick(1_000);
    expect(server.mtActive).toBe(true);
    const vizMessage: VizMsg = { type: 'viz', enabled: true };
    server.handleViz(1, vizMessage);
    access.buildStats();
    const pool = access.brainPool;
    if (!pool) throw new Error('expected a canonical pool');
    await waitForVisualization(pool);

    const stats = access.buildStats();
    expect(stats.viz).toMatchObject({
      kind: 'graph',
      simulationStep: server.getTickId(),
      poolEpoch: pool.poolEpoch,
      weightEpoch: pool.weightEpoch
    });
    expect(stats.viz?.populationSlot).toBeGreaterThanOrEqual(0);
  });

  it('faults an in-flight step without frame, stats, fallback, or later stepping', async () => {
    const { server, access, probe } = await createMtServer(2, 'js', true);
    const pool = access.brainPool;
    if (!pool) throw new Error('expected a canonical pool');
    for (const worker of pool.workers) {
      worker.removeAllListeners('message');
    }
    access.running = true;
    access.nextTickAt = 0;
    access.lastTickAt = 0;
    access.lastFrameSentAt = 0;
    access.lastStatsSentAt = 0;
    const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    try {
      const loop = access.loop();
      await new Promise<void>((resolve) => setImmediate(resolve));
      access.running = false;
      pool.workers[0]?.emit('error', new Error('phase4 production worker crash'));
      await loop;
    } finally {
      errorSpy.mockRestore();
      warnSpy.mockRestore();
    }

    expect(server.getTickId()).toBe(0);
    expect(server.getFaultStatus()).toEqual({
      faulted: true,
      reason: 'phase4 production worker crash',
      tick: 0
    });
    expect(pool.status).toBe('failed');
    expect(access.core.brainPool).toBe(pool);
    const failedPoolEpoch = pool.poolEpoch;
    expect(probe.frames).toHaveLength(0);
    expect(probe.stats).toHaveLength(0);
    expect(probe.errors).toEqual([
      'simulation faulted at tick 0: phase4 production worker crash'
    ]);

    await access.tick(2_000);
    expect(server.getTickId()).toBe(0);
    expect(probe.frames).toHaveLength(0);
    expect(probe.stats).toHaveLength(0);

    await server.startNewRun();
    expect(server.getFaultStatus()).toEqual({ faulted: false, reason: null, tick: null });
    expect(access.brainPool?.status).toBe('ready');
    expect(access.brainPool?.poolEpoch).toBeGreaterThan(failedPoolEpoch ?? 0);
    access.lastTickAt = 0;
    await access.tick(3_000);
    expect(server.getTickId()).toBe(1);
  });
});
