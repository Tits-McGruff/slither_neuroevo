import { describe, it } from 'vitest';
import { createServer, type Server as HttpServer } from 'node:http';
import { CFG, resetCFGToDefaults } from '../src/config.ts';
import { World } from '../src/world.ts';
import { WorldSerializer } from '../src/serializer.ts';
import { DEFAULT_CONFIG, type ServerConfig } from './config.ts';
import { SERIALIZER_VERSION, type WelcomeMsg } from './protocol.ts';
import { buildSensorSpec } from './sensorSpec.ts';
import { SimServer } from './simServer.ts';
import { WsHub } from './wsHub.ts';

/** Test suite label for MT parity checks. */
const SUITE = 'server MT parity';
/** Absolute tolerance for frame buffer comparisons. */
const FRAME_TOLERANCE = 1e-3;
/** Total ticks to compare for parity. */
const TICK_COUNT = 20;
/** Fixed seed used for deterministic RNGs. */
const TEST_SEED = 4242;

/** RNG function returning values in [0, 1). */
type RandomFn = () => number;

/** Settings payload for deterministic test worlds. */
interface TestWorldSettings {
  /** Total number of snakes to spawn. */
  snakeCount: number;
  /** Hidden layer count for the brain architecture. */
  hiddenLayers: number;
  /** Layer 1 neuron count. */
  neurons1: number;
  /** Layer 2 neuron count. */
  neurons2: number;
  /** Layer 3 neuron count. */
  neurons3: number;
  /** Layer 4 neuron count. */
  neurons4: number;
  /** Layer 5 neuron count. */
  neurons5: number;
  /** Simulation speed multiplier. */
  simSpeed: number;
}

/** Deterministic world settings for parity tests. */
const TEST_SETTINGS: TestWorldSettings = {
  snakeCount: 6,
  hiddenLayers: 1,
  neurons1: 8,
  neurons2: 8,
  neurons3: 8,
  neurons4: 8,
  neurons5: 8,
  simSpeed: 1
};

/** Wrapper for a WS hub and HTTP server pair. */
interface TestHub {
  /** HTTP server used to attach the WebSocket hub. */
  httpServer: HttpServer;
  /** WebSocket hub instance used by the sim server. */
  wsHub: WsHub;
}

/** Details recorded for the first buffer mismatch. */
interface BufferMismatch {
  /** Index in the buffer where divergence begins. */
  index: number;
  /** Value from the MT buffer. */
  left: number;
  /** Value from the JS buffer. */
  right: number;
  /** Absolute difference between values. */
  diff: number;
  /** True when the buffer lengths differ. */
  lengthMismatch: boolean;
}

/**
 * Create a deterministic RNG for repeatable test runs.
 * @param seed - Initial seed value.
 * @returns RNG function returning values in [0,1).
 */
function createSeededRandom(seed: number): RandomFn {
  let state = (seed >>> 0) || 1;
  return () => {
    state = (state * 1664525 + 1013904223) >>> 0;
    return state / 0x100000000;
  };
}

/**
 * Run a function with Math.random temporarily overridden.
 * @param rng - RNG function to use for Math.random.
 * @param fn - Function to run under the RNG override.
 * @returns Return value from the invoked function.
 */
function withSeededRandom<T>(rng: RandomFn, fn: () => T): T {
  const original = Math.random;
  Math.random = rng;
  try {
    return fn();
  } finally {
    Math.random = original;
  }
}

/**
 * Run an async function with Math.random temporarily overridden.
 * @param rng - RNG function to use for Math.random.
 * @param fn - Async function to run under the RNG override.
 * @returns Return value from the invoked function.
 */
async function withSeededRandomAsync<T>(rng: RandomFn, fn: () => Promise<T>): Promise<T> {
  const original = Math.random;
  Math.random = rng;
  try {
    return await fn();
  } finally {
    Math.random = original;
  }
}

/**
 * Create a WebSocket hub for sim server tests.
 * @param settings - World settings used to size the welcome payload.
 * @returns Hub wrapper with the attached HTTP server.
 */
async function createTestHub(settings: TestWorldSettings): Promise<TestHub> {
  const sensorSpec = buildSensorSpec();
  const sampleWorld = new World(settings);
  const frameByteLength = WorldSerializer.serialize(sampleWorld).byteLength;
  const welcome: WelcomeMsg = {
    type: 'welcome',
    sessionId: 'test-session',
    tickRate: DEFAULT_CONFIG.tickRateHz,
    worldSeed: TEST_SEED,
    cfgHash: 'test-config',
    sensorSpec,
    serializerVersion: SERIALIZER_VERSION,
    frameByteLength
  };

  const httpServer = createServer((_req, res) => {
    res.statusCode = 404;
    res.end();
  });
  const wsHub = new WsHub(httpServer, welcome);

  return { httpServer, wsHub };
}

/**
 * Close a test hub and its underlying server.
 * @param hub - Hub wrapper to close.
 */
async function closeTestHub(hub: TestHub): Promise<void> {
  hub.wsHub.closeAll();
  if (!hub.httpServer.listening) return;
  await new Promise<void>((resolve) => hub.httpServer.close(() => resolve()));
}

/**
 * Execute a single sim server tick by calling its private tick method.
 * @param server - SimServer instance to tick.
 * @param now - Timestamp to pass into the tick call.
 */
async function runServerTick(server: SimServer, now: number): Promise<void> {
  const runner = server as unknown as { tick: (now: number) => Promise<void> };
  await runner.tick(now);
}

/**
 * Read the MT active flag from a sim server instance.
 * @param server - SimServer instance to inspect.
 * @returns True when MT inference ran on the last tick.
 */
function getMtActive(server: SimServer): boolean {
  const state = server as unknown as { mtActive?: boolean };
  return state.mtActive === true;
}

/**
 * Stop a sim server and wait for its brain pool to terminate.
 * @param server - SimServer instance to stop.
 */
async function shutdownSimServer(server: SimServer): Promise<void> {
  server.stop();
  const state = server as unknown as { brainPool?: { shutdown: () => Promise<void> } | null };
  if (state.brainPool) {
    await state.brainPool.shutdown();
  }
}

/**
 * Find the first mismatch between two Float32 buffers.
 * @param left - MT buffer to compare.
 * @param right - JS buffer to compare.
 * @param tol - Absolute tolerance allowed per element.
 * @returns Mismatch details or null when all values match.
 */
function findBufferMismatch(
  left: Float32Array,
  right: Float32Array,
  tol: number
): BufferMismatch | null {
  if (left.length !== right.length) {
    const index = Math.min(left.length, right.length);
    const lv = left[index] ?? 0;
    const rv = right[index] ?? 0;
    return {
      index,
      left: lv,
      right: rv,
      diff: Math.abs(lv - rv),
      lengthMismatch: true
    };
  }
  for (let i = 0; i < left.length; i++) {
    const lv = left[i] ?? 0;
    const rv = right[i] ?? 0;
    const diff = Math.abs(lv - rv);
    if (diff > tol) {
      return {
        index: i,
        left: lv,
        right: rv,
        diff,
        lengthMismatch: false
      };
    }
  }
  return null;
}

describe(SUITE, () => {
  it('matches JS batch outputs over deterministic ticks', async () => {
    resetCFGToDefaults();
    const originalBaselineBots = CFG.baselineBots.count;
    const originalPelletTarget = CFG.pelletCountTarget;
    const originalBatchEnabled = CFG.brain.batchEnabled;
    const baselineBotsSeed = CFG.baselineBots.seed;
    const baselineBotsRandomized = CFG.baselineBots.randomizeSeedPerGen;

    let mtHub: TestHub | null = null;
    let jsHub: TestHub | null = null;
    let mtServer: SimServer | null = null;
    let jsServer: SimServer | null = null;

    try {
      CFG.baselineBots.count = 0;
      CFG.baselineBots.seed = 0;
      CFG.baselineBots.randomizeSeedPerGen = false;
      CFG.pelletCountTarget = 0;
      CFG.brain.batchEnabled = true;

      mtHub = await createTestHub(TEST_SETTINGS);
      jsHub = await createTestHub(TEST_SETTINGS);

      const mtConfig: ServerConfig = {
        ...DEFAULT_CONFIG,
        mtEnabled: true,
        mtWorkers: 1,
        logLevel: 'error',
        port: 0
      };
      const jsConfig: ServerConfig = {
        ...DEFAULT_CONFIG,
        mtEnabled: false,
        mtWorkers: 0,
        logLevel: 'error',
        port: 0
      };

      const mtRng = createSeededRandom(TEST_SEED);
      const jsRng = createSeededRandom(TEST_SEED);

      mtServer = withSeededRandom(mtRng, () =>
        new SimServer(mtConfig, mtHub!.wsHub, undefined, 'test-config', TEST_SEED, TEST_SETTINGS)
      );
      jsServer = withSeededRandom(jsRng, () =>
        new SimServer(jsConfig, jsHub!.wsHub, undefined, 'test-config', TEST_SEED, TEST_SETTINGS)
      );

      const stepMs = 1000 / DEFAULT_CONFIG.tickRateHz;
      let now = 0;

      for (let t = 0; t < TICK_COUNT; t++) {
        now += stepMs;
        await withSeededRandomAsync(mtRng, () => runServerTick(mtServer!, now));
        await withSeededRandomAsync(jsRng, () => runServerTick(jsServer!, now));

        if (t === 0 && !getMtActive(mtServer)) {
          throw new Error('MT pool was not active on the first tick');
        }

        const mtFrame = WorldSerializer.serialize(mtServer.getWorld());
        const jsFrame = WorldSerializer.serialize(jsServer.getWorld());
        const mismatch = findBufferMismatch(mtFrame, jsFrame, FRAME_TOLERANCE);
        if (mismatch) {
          const detail = mismatch.lengthMismatch
            ? `length mt=${mtFrame.length} js=${jsFrame.length}`
            : `diff=${mismatch.diff.toFixed(6)}`;
          throw new Error(
            `[mt parity] tick=${t} index=${mismatch.index} mt=${mismatch.left} js=${mismatch.right} ${detail}`
          );
        }
      }
    } finally {
      CFG.baselineBots.count = originalBaselineBots;
      CFG.baselineBots.seed = baselineBotsSeed;
      CFG.baselineBots.randomizeSeedPerGen = baselineBotsRandomized;
      CFG.pelletCountTarget = originalPelletTarget;
      CFG.brain.batchEnabled = originalBatchEnabled;
      resetCFGToDefaults();
      if (mtServer) await shutdownSimServer(mtServer);
      if (jsServer) await shutdownSimServer(jsServer);
      if (mtHub) await closeTestHub(mtHub);
      if (jsHub) await closeTestHub(jsHub);
    }
  }, 20000);
});
