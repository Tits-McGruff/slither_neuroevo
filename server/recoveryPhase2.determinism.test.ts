import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { CFG, resetCFGToDefaults } from '../src/config.ts';
import type { CoreSettings } from '../src/protocol/settings.ts';
import type { SimCore } from '../src/sim/SimCore.ts';
import type { ControllerRegistryLike, GenerationBoundaryState, World } from '../src/world.ts';
import { DEFAULT_CONFIG } from './config.ts';
import { createEntropySeed, createRunId, createSessionId } from './runIdentity.ts';
import { SimServer } from './simServer.ts';
import {
  captureAuthoritativeWorldDigest,
  findFirstAuthoritativeWorldDivergence
} from './test/authoritativeWorldDigest.ts';
import { createSeededWorldFixture } from './test/seededWorldFixture.ts';
import type { WsHub } from './wsHub.ts';

/** Phase 2 seeded production-World replay suite. */
const SUITE = 'recovery Phase 2 — authoritative seeded randomness';

/** Small production settings used by deterministic replay tests. */
const SETTINGS: Partial<CoreSettings> = {
  snakeCount: 5,
  simSpeed: 1,
  hiddenLayers: 1,
  neurons1: 8,
  neurons2: 6,
  neurons3: 5,
  neurons4: 4,
  neurons5: 3
};

/** Authoritative modules forbidden from reading the ambient global RNG. */
const AUTHORITATIVE_RANDOM_MODULES = [
  'src/world.ts',
  'src/snake.ts',
  'src/mlp.ts',
  'src/brains/ops.ts',
  'src/bots/baselineBots.ts',
  'src/sim/SimCore.ts',
  'server/index.ts',
  'server/simServer.ts'
] as const;
/** Number of discrete turn values in the deterministic action log. */
const ACTION_POSITION_COUNT = 9;

/** Compact initial-state shape used to name each required construction contract. */
interface InitialStateShape {
  /** Population weights in durable slot order. */
  genomes: number[][];
  /** Spawn geometry in durable snake order. */
  snakes: Array<{ id: number; x: number; y: number; dir: number }>;
  /** Ambient pellet values and positions in insertion order. */
  pellets: Array<{ x: number; y: number; value: number; kind: string }>;
  /** Initially selected observer focus id. */
  focusId: number | null;
}

/** Boundary observations captured synchronously inside the production hook. */
interface BoundaryObservation {
  /** Immutable boundary payload. */
  boundary: GenerationBoundaryState;
  /** Number of population genomes already assigned. */
  populationCount: number;
  /** Number of snakes present at the hook point. */
  snakeCount: number;
  /** Number of pellets present at the hook point. */
  pelletCount: number;
  /** Whether the payload exactly matched live RNG state at the hook point. */
  rngMatched: boolean;
}

beforeEach(() => {
  resetCFGToDefaults();
  CFG.baselineBots.count = 2;
  CFG.baselineBots.seed = 17;
  CFG.baselineBots.randomizeSeedPerGen = true;
  CFG.baselineBots.respawnDelay = 0.25;
  CFG.pelletCountTarget = 48;
  CFG.pelletSpawnPerSecond = 6;
  CFG.generationSeconds = 120;
});

afterEach(() => {
  resetCFGToDefaults();
});

/** Build one full production SimCore with a stable test identity. */
function makeCore(seed: number, runId = `run-${seed >>> 0}`): SimCore {
  return createSeededWorldFixture({ seed, runId, settings: SETTINGS });
}

/** Capture construction fields named explicitly by the Phase 2 gate. */
function captureInitialState(world: World): InitialStateShape {
  return {
    genomes: world.population.map(genome => Array.from(genome.weights)),
    snakes: world.snakes.map(snake => ({
      id: snake.id,
      x: snake.x,
      y: snake.y,
      dir: snake.dir
    })),
    pellets: world.pellets.map(pellet => ({
      x: pellet.x,
      y: pellet.y,
      value: pellet.v,
      kind: pellet.kind
    })),
    focusId: world.focusSnake?.id ?? null
  };
}

/** Create a deterministic tick-aligned action-log provider for snake one. */
function createActionLogController(): ControllerRegistryLike {
  return {
    isControlled: snakeId => snakeId === 1,
    getAction: (_snakeId, tickId) => ({
      turn: ((tickId % ACTION_POSITION_COUNT) - 4) / 4,
      boost: tickId % 19 === 0 ? 1 : 0
    }),
    publishSensors: () => undefined
  };
}

/** Advance a core by an exact number of fixed ticks. */
async function stepFixedTicks(
  core: SimCore,
  count: number,
  controllers?: ControllerRegistryLike
): Promise<void> {
  for (let tick = 0; tick < count; tick++) {
    await core.update(core.fixedDt, controllers);
  }
}

/** Assign deterministic fitness inputs before forcing a generation boundary. */
function prepareEvolution(world: World): void {
  for (let slot = 0; slot < world.population.length; slot++) {
    const snake = world.snakes[slot]!;
    snake.age = 12 + slot;
    snake.foodEaten = slot * 2;
    snake.killScore = slot % 3;
    snake.pointsScore = 5 + slot * 7;
  }
}

/** Strip observer-only continuation from an RNG bundle for isolation assertions. */
function captureGameplayStreams(world: World): object {
  const state = world.exportRngState();
  return {
    version: state.version,
    seed: state.seed,
    world: state.world,
    evolution: state.evolution,
    baselines: state.baselines
  };
}

describe(SUITE, () => {
  it('same seed creates identical genomes, snakes, pellets, focus, and digest', () => {
    const expected = makeCore(0x1234abcd);
    const actual = makeCore(0x1234abcd, 'different-metadata-run-id');

    expect(captureInitialState(actual.world)).toEqual(captureInitialState(expected.world));
    expect(captureAuthoritativeWorldDigest(actual.world).digest)
      .toBe(captureAuthoritativeWorldDigest(expected.world).digest);
  });

  it('different seeds diverge during normal production construction', () => {
    const expected = captureAuthoritativeWorldDigest(makeCore(100).world);
    const actual = captureAuthoritativeWorldDigest(makeCore(101).world);

    expect(actual.digest).not.toBe(expected.digest);
    expect(findFirstAuthoritativeWorldDivergence(expected, actual)).not.toBeNull();
  });

  it('same seed and action log replay identically for many fixed ticks', async () => {
    const expected = makeCore(0x31415926);
    const actual = makeCore(0x31415926, 'replay-copy');

    await stepFixedTicks(expected, 240, createActionLogController());
    await stepFixedTicks(actual, 240, createActionLogController());

    const expectedDigest = captureAuthoritativeWorldDigest(expected.world);
    const actualDigest = captureAuthoritativeWorldDigest(actual.world);
    expect(actualDigest.digest).toBe(expectedDigest.digest);
    expect(findFirstAuthoritativeWorldDivergence(expectedDigest, actualDigest)).toBeNull();
  });

  it('Reset keeps the seed, changes run identity, and reproduces generation one', async () => {
    const core = makeCore(0x10203040, 'before-reset');
    const initial = captureAuthoritativeWorldDigest(core.world);
    await stepFixedTicks(core, 30, createActionLogController());

    const identity = core.reset(SETTINGS, { runId: 'after-reset' });

    expect(identity).toEqual({ seed: 0x10203040, runId: 'after-reset' });
    expect(core.getRunIdentity()).toEqual(identity);
    expect(captureAuthoritativeWorldDigest(core.world).digest).toBe(initial.digest);
    expect(core.world.generation).toBe(1);
    expect(core.tickId).toBe(0);
  });

  it('keeps the prior world and identity when a future boundary hook rejects restart', () => {
    let rejectBoundary = false;
    const core = createSeededWorldFixture({
      seed: 0x10203040,
      runId: 'stable-run',
      settings: SETTINGS,
      onGenerationBoundary: () => {
        if (rejectBoundary) throw new Error('checkpoint rejected');
      }
    });
    const originalWorld = core.world;
    const originalIdentity = core.getRunIdentity();
    rejectBoundary = true;

    expect(() => core.reset(SETTINGS, { runId: 'rejected-run' }))
      .toThrow('checkpoint rejected');
    expect(core.world).toBe(originalWorld);
    expect(core.getRunIdentity()).toEqual(originalIdentity);
  });

  it('New Run exposes its new seed and creates divergent generation-one state', () => {
    const core = makeCore(0x11111111, 'before-new-run');
    const initial = captureAuthoritativeWorldDigest(core.world);

    const identity = core.newRun(SETTINGS, {
      seed: 0x22222222,
      runId: 'after-new-run'
    });

    expect(identity).toEqual({ seed: 0x22222222, runId: 'after-new-run' });
    expect(core.getRunIdentity()).toEqual(identity);
    expect(captureAuthoritativeWorldDigest(core.world).digest).not.toBe(initial.digest);
  });

  it('passes the server seed into SimCore/World and starts a new entropy lineage', () => {
    const wsHub = { sendJsonTo: () => undefined } as unknown as WsHub;
    const server = new SimServer(
      { ...DEFAULT_CONFIG, dbPath: ':memory:', logLevel: 'error' },
      wsHub,
      undefined,
      'phase-2-config',
      0x12345678,
      SETTINGS,
      'server-run-before'
    );
    const initial = captureAuthoritativeWorldDigest(server.getWorld());

    expect(server.getRunIdentity()).toEqual({
      seed: 0x12345678,
      runId: 'server-run-before'
    });
    expect(server.getWorld().seed).toBe(0x12345678);

    const identity = server.startNewRun();
    expect(identity.seed).not.toBe(0x12345678);
    expect(identity.runId).not.toBe('server-run-before');
    expect(server.getWorld().seed).toBe(identity.seed);
    expect(captureAuthoritativeWorldDigest(server.getWorld()).digest).not.toBe(initial.digest);
  });

  it('evolution selection, crossover, mutation, and checkpoint boundary reproduce', () => {
    const expected = makeCore(0x55667788);
    const actual = makeCore(0x55667788, 'evolution-copy');
    prepareEvolution(expected.world);
    prepareEvolution(actual.world);

    expected.world._endGeneration();
    actual.world._endGeneration();

    expect(expected.world.generation).toBe(2);
    expect(actual.world.population.map(genome => Array.from(genome.weights)))
      .toEqual(expected.world.population.map(genome => Array.from(genome.weights)));
    expect(captureAuthoritativeWorldDigest(actual.world).digest)
      .toBe(captureAuthoritativeWorldDigest(expected.world).digest);
  });

  it('emits the generation checkpoint after population assignment and before construction draws', () => {
    const observations: BoundaryObservation[] = [];
    const core = createSeededWorldFixture({
      seed: 0x88776655,
      runId: 'boundary-run',
      settings: SETTINGS,
      onGenerationBoundary: (boundary, world) => {
        observations.push({
          boundary,
          populationCount: world.population.length,
          snakeCount: world.snakes.length,
          pelletCount: world.pellets.length,
          rngMatched: boundary.rng.world.stateHex === world.worldRng.exportState().stateHex
            && boundary.rng.evolution.stateHex === world.evolutionRng.exportState().stateHex
        });
      }
    });

    expect(observations).toHaveLength(1);
    expect(observations[0]).toMatchObject({
      populationCount: SETTINGS.snakeCount,
      snakeCount: 0,
      pelletCount: 0,
      rngMatched: true,
      boundary: { kind: 'run-start', generation: 1, seed: 0x88776655 }
    });
    prepareEvolution(core.world);
    core.world._endGeneration();
    expect(observations).toHaveLength(2);
    expect(observations[1]).toMatchObject({
      populationCount: SETTINGS.snakeCount,
      snakeCount: 0,
      pelletCount: 0,
      rngMatched: true,
      boundary: { kind: 'generation', generation: 2, seed: 0x88776655 }
    });
    expect(core.world.worldRng.exportState()).not.toEqual(observations[1]!.boundary.rng.world);
  });

  it('death and boost pellet placement reproduce despite cosmetic particle draws', () => {
    const deathExpected = makeCore(0xdeadbeef);
    const deathActual = makeCore(0xdeadbeef, 'death-copy');
    deathExpected.world.snakes[0]!.die(deathExpected.world);
    deathActual.world.snakes[0]!.die(deathActual.world);
    expect(captureAuthoritativeWorldDigest(deathActual.world).digest)
      .toBe(captureAuthoritativeWorldDigest(deathExpected.world).digest);

    const boostExpected = makeCore(0x0badf00d);
    const boostActual = makeCore(0x0badf00d, 'boost-copy');
    for (const world of [boostExpected.world, boostActual.world]) {
      const snake = world.snakes[0]!;
      while (snake.points.length < CFG.snakeMinLen + 4) {
        const tail = snake.points[snake.points.length - 1]!;
        snake.points.push({ x: tail.x - CFG.snakeSpacing, y: tail.y });
      }
      snake.pointsScore = 100;
      snake.targetLen = snake.points.length;
      expect(snake._applyBoostMassBurn(world, 1)).toBeGreaterThan(0);
      expect(world.pellets.some(pellet => pellet.kind === 'boost')).toBe(true);
    }
    expect(captureAuthoritativeWorldDigest(boostActual.world).digest)
      .toBe(captureAuthoritativeWorldDigest(boostExpected.world).digest);
  });

  it('cosmetic particle work cannot shift authoritative replay', async () => {
    const expected = makeCore(0xcafebabe);
    const actual = makeCore(0xcafebabe, 'cosmetic-copy');
    for (let burst = 0; burst < 40; burst++) {
      actual.world.particles.spawnBurst(burst, -burst, 'rgb(1,2,3)', 8, 2);
      actual.world.particles.spawnBoost(-burst, burst, burst / 10, 'rgb(4,5,6)');
    }

    await stepFixedTicks(expected, 60);
    await stepFixedTicks(actual, 60);

    expect(captureAuthoritativeWorldDigest(actual.world).digest)
      .toBe(captureAuthoritativeWorldDigest(expected.world).digest);
  });

  it('observer work and added bot slots do not shift gameplay or evolution streams', () => {
    const expected = makeCore(0xabcdef01);
    const observerBusy = makeCore(0xabcdef01, 'observer-copy');
    for (let pick = 0; pick < 100; pick++) observerBusy.world._pickAnyAlive();

    expect(captureGameplayStreams(observerBusy.world)).toEqual(captureGameplayStreams(expected.world));
    expect(observerBusy.world.exportRngState().observer)
      .not.toEqual(expected.world.exportRngState().observer);
    expect(captureAuthoritativeWorldDigest(observerBusy.world).digest)
      .toBe(captureAuthoritativeWorldDigest(expected.world).digest);

    CFG.baselineBots.count = 1;
    const oneBot = makeCore(0x13572468, 'one-bot');
    CFG.baselineBots.count = 4;
    const fourBots = makeCore(0x13572468, 'four-bots');
    expect(fourBots.world.exportRngState().world).toEqual(oneBot.world.exportRngState().world);
    expect(fourBots.world.exportRngState().evolution)
      .toEqual(oneBot.world.exportRngState().evolution);
    expect(fourBots.world.exportRngState().baselines).toHaveLength(4);
    expect(oneBot.world.exportRngState().baselines).toHaveLength(1);
  });

  it('exports and restores world, evolution, observer, bot, and allocator continuation', () => {
    const world = makeCore(0x2468ace0).world;
    const rngState = world.exportRngState();
    const allocatorState = world.exportAllocatorState();
    const expectedDraws = {
      world: world.worldRng.next(),
      evolution: world.evolutionRng.gaussian(),
      observer: world.observerRng.int(1000),
      baseline: world.botManager.prepareBotSpawn(0)()
    };
    const expectedContinuation = world.exportRngState();

    world.restoreRngState(rngState);
    const actualDraws = {
      world: world.worldRng.next(),
      evolution: world.evolutionRng.gaussian(),
      observer: world.observerRng.int(1000),
      baseline: world.botManager.prepareBotSpawn(0)()
    };

    expect(actualDraws).toEqual(expectedDraws);
    expect(world.exportRngState()).toEqual(expectedContinuation);
    world._nextExternalSnakeId += 50;
    world._nextBaselineBotId += 50;
    world._nextResurrectedSnakeId += 50;
    world.restoreAllocatorState(allocatorState);
    expect(world.exportAllocatorState()).toEqual(allocatorState);
  });

  it('allocates deterministic collision-safe resurrection ids', () => {
    const expected = makeCore(0x42424242);
    const actual = makeCore(0x42424242, 'allocator-copy');
    const expectedAllocator = expected.world.exportAllocatorState();
    const actualAllocator = actual.world.exportAllocatorState();
    const genome = expected.world.population[0]!.toJSON();
    const genomeCopy = actual.world.population[0]!.toJSON();

    const firstExpected = expected.world.resurrect(genome);
    const firstActual = actual.world.resurrect(genomeCopy);
    expected.world.restoreAllocatorState(expectedAllocator);
    actual.world.restoreAllocatorState(actualAllocator);
    const secondExpected = expected.world.resurrect(genome);
    const secondActual = actual.world.resurrect(genomeCopy);

    expect(firstExpected).toBe(firstActual);
    expect(secondExpected).toBe(secondActual);
    expect(secondExpected).toBe(firstExpected + 1);
    expect(new Set(expected.world.snakes.map(snake => snake.id)).size)
      .toBe(expected.world.snakes.length);
  });

  it('continues a durable baseline stream across death and respawn', async () => {
    const core = makeCore(0x77777777);
    const initial = core.world.exportRngState().baselines[0]!.rng;
    core.world.baselineBots[0]!.die(core.world);
    await stepFixedTicks(core, 40);
    const continued = core.world.exportRngState().baselines[0]!.rng;

    expect(core.world.baselineBots[0]!.alive).toBe(true);
    expect(continued).not.toEqual(initial);
  });

  it('canonical digest ignores incidental snake, pellet, and batch backing order', async () => {
    const core = makeCore(0x91919191);
    await stepFixedTicks(core, 1);
    const expected = captureAuthoritativeWorldDigest(core.world);

    core.world.snakes.reverse();
    core.world.pellets.reverse();
    core.world._controlBatch.indices.reverse();
    core.world._controlBatch.snakeIndices.reverse();
    const actual = captureAuthoritativeWorldDigest(core.world);

    expect(actual.digest).toBe(expected.digest);
    expect(findFirstAuthoritativeWorldDivergence(expected, actual)).toBeNull();
  });

  it('uses system entropy for unspecified seeds and independent run/session ids', () => {
    const seed = createEntropySeed(123);
    const runId = createRunId();
    const sessionId = createSessionId();

    expect(seed).not.toBe(123);
    expect(seed).toBeGreaterThanOrEqual(0);
    expect(seed).toBeLessThanOrEqual(0xffffffff);
    expect(runId).not.toBe(sessionId);
    expect(runId).toMatch(/^[0-9a-f-]{36}$/u);
    expect(sessionId).toMatch(/^[0-9a-f-]{36}$/u);
  });

  it('statically rejects ambient Math.random reads in authoritative modules', () => {
    for (const relativePath of AUTHORITATIVE_RANDOM_MODULES) {
      const source = readFileSync(resolve(process.cwd(), relativePath), 'utf8');
      expect(source, relativePath).not.toContain('Math.random');
    }
  });
});
