import { describe, expect, it } from 'vitest';
import type { Genome } from '../src/mlp.ts';
import type { World } from '../src/world.ts';
import {
  AuthoritativeStateCaptureError,
  captureAuthoritativeWorldDigest,
  findFirstAuthoritativeWorldDivergence
} from './test/authoritativeWorldDigest.ts';

/** Test-suite label. */
const SUITE = 'Phase 0 authoritative World digest';

/** Options for the explicit hand-built World fixture. */
interface FixtureOptions {
  /** Reverse unordered collections and graph nodes. */
  reverseOrder?: boolean;
  /** Store negative zero at population weight index one. */
  negativeZeroWeight?: boolean;
  /** Second GRU hidden-state value. */
  recurrentValue?: number;
  /** Change only observer/render metadata. */
  observerVariant?: boolean;
}

/**
 * Build an explicit World-shaped component fixture without construction RNG.
 * @param options - Fixture variations.
 * @returns Hand-built World shape.
 */
function makeWorldFixture(options: FixtureOptions = {}): World {
  const negativeZero = options.negativeZeroWeight ?? true;
  const recurrentValue = options.recurrentValue ?? 0.25;
  const populationGenome = {
    archKey: 'fixture-graph',
    brainType: 'gru',
    fitness: 12.5,
    weights: new Float32Array([1, negativeZero ? -0 : 0, -2.5])
  } as Genome;
  const baselineGenome = {
    archKey: 'fixture-baseline',
    brainType: 'scripted',
    fitness: 0,
    weights: new Float32Array(0)
  } as Genome;
  const graphNodes = [
    { id: 'memory', type: 'GRU', gru: { h: new Float32Array([-0.5, recurrentValue]) } },
    { id: 'head', type: 'Dense' }
  ];
  if (options.reverseOrder) graphNodes.reverse();

  const populationSnake = {
    id: 1,
    populationSlot: 0,
    color: options.observerVariant ? 'observer-a' : 'observer-b',
    x: 10.25,
    y: -20.5,
    dir: 0.75,
    radius: 5.5,
    speed: 42.25,
    boost: 1,
    alive: true,
    foodEaten: 3,
    age: 4.5,
    killScore: 2.25,
    pointsScore: 18.75,
    prevPointsScore: 17.5,
    targetLen: 24.25,
    points: [{ x: 10.25, y: -20.5 }, { x: 8.5, y: -21.25 }],
    genome: populationGenome,
    brain: { nodes: graphNodes },
    turnInput: -0.125,
    boostInput: 0.875,
    _ctrlAcc: 0.01,
    _hasAct: 1,
    _lastControlExternal: false,
    lastSensors: new Float32Array([options.observerVariant ? 99 : -99]),
    lastOutputs: new Float32Array([options.observerVariant ? 88 : -88]),
    controlMode: 'neural',
    baselineBotIndex: null,
    skin: options.observerVariant ? 99 : 0
  };
  const baselineSnake = {
    id: 200001,
    color: options.observerVariant ? 'observer-c' : 'observer-d',
    x: -4,
    y: 8,
    dir: -0.5,
    radius: 4,
    speed: 30,
    boost: 0,
    alive: false,
    foodEaten: 1,
    age: 2,
    killScore: 0,
    pointsScore: 4,
    prevPointsScore: 4,
    targetLen: 16,
    points: [{ x: -4, y: 8 }],
    genome: baselineGenome,
    brain: { nodes: [] },
    turnInput: 0.25,
    boostInput: 0,
    _ctrlAcc: 0.02,
    _hasAct: 1,
    _lastControlExternal: false,
    lastSensors: new Float32Array([options.observerVariant ? 77 : -77]),
    lastOutputs: new Float32Array([options.observerVariant ? 66 : -66]),
    controlMode: 'external-only',
    baselineBotIndex: 0,
    skin: options.observerVariant ? 7 : 0
  };
  const pellets = [
    { x: 3.5, y: -7.25, v: 1.5, kind: 'ambient', color: 'ignored-a', colorId: 4 },
    { x: -1.25, y: 2.75, v: 3, kind: 'boost', color: 'ignored-b', colorId: 8 }
  ];
  const snakes = [populationSnake, baselineSnake];
  if (options.reverseOrder) {
    pellets.reverse();
    snakes.reverse();
  }

  return {
    archKey: 'fixture-graph',
    generation: 7,
    generationTime: 12.25,
    tickId: 42,
    _pelletSpawnAcc: 0.375,
    simSpeed: 1,
    bestPointsThisGen: 18.75,
    bestPointsSnakeId: 1,
    _nextExternalSnakeId: 100004,
    _nextBaselineBotId: 200003,
    settings: {
      snakeCount: 1,
      simSpeed: 1,
      hiddenLayers: 1,
      neurons1: 4,
      neurons2: 3,
      neurons3: 2,
      neurons4: 1,
      neurons5: 1,
      worldRadius: 500,
      collision: {
        substepMaxDt: 0.006,
        skipSegments: 1,
        hitScale: 0.82,
        cellSize: 70,
        neighborRange: 1
      }
    },
    population: [populationGenome],
    snakes,
    baselineBots: [baselineSnake],
    pellets,
    botManager: {
      botSeeds: [123],
      botStates: ['avoid'],
      botStateTimers: [0.5],
      botWanderAngles: [-0.25],
      botWanderTimers: [0.75],
      botActions: [{ turn: 0.125, boost: 1 }],
      botSnakeIds: [200001],
      respawnTimers: [2.5],
      controllerDisabled: false
    },
    cameraX: options.observerVariant ? 999 : -999,
    cameraY: options.observerVariant ? 888 : -888,
    zoom: options.observerVariant ? 4 : 0.25,
    focusSnake: options.observerVariant ? baselineSnake : populationSnake,
    _focusCooldown: options.observerVariant ? 100 : 0,
    viewMode: options.observerVariant ? 'follow' : 'overview',
    particles: { observerOnly: options.observerVariant },
    fitnessHistory: options.observerVariant ? [{ observerOnly: true }] : [],
    _lastHoFEntry: options.observerVariant ? { observerOnly: true } : null,
    runId: options.observerVariant ? 'run-b' : 'run-a',
    timestamp: options.observerVariant ? 999999 : 1
  } as unknown as World;
}

/**
 * Capture and return the helper's structured validation error.
 * @param mutate - Fixture mutation that introduces invalid state.
 * @returns Capture error.
 */
function captureFailure(mutate: (world: World) => void): AuthoritativeStateCaptureError {
  const world = makeWorldFixture();
  mutate(world);
  try {
    captureAuthoritativeWorldDigest(world);
  } catch (error) {
    if (error instanceof AuthoritativeStateCaptureError) return error;
    throw error;
  }
  throw new Error('Expected authoritative-state capture to fail.');
}

describe(SUITE, () => {
  it('canonicalizes durable identities and excludes observer-only metadata', () => {
    const expected = captureAuthoritativeWorldDigest(makeWorldFixture());
    const actual = captureAuthoritativeWorldDigest(makeWorldFixture({
      reverseOrder: true,
      observerVariant: true
    }));

    expect(actual.digest).toBe(expected.digest);
    expect(findFirstAuthoritativeWorldDivergence(expected, actual)).toBeNull();
    expect(expected.algorithm).toBe('sha256');
    expect(expected.version).toBe(1);
    const paths = expected.entries.map((entry) => entry.path);
    expect(paths).toEqual([...paths].sort());
    expect(new Set(paths).size).toBe(paths.length);
    for (const forbidden of [
      'camera', 'focus', 'viewMode', 'particles', 'color', 'skin',
      'lastSensors', 'lastOutputs', 'runId', 'sessionId', 'timestamp',
      'fitnessHistory', 'HoF'
    ]) {
      expect(paths.some((path) => path.includes(forbidden))).toBe(false);
    }
  });

  it('uses raw Float32 bits and reports the first population divergence', () => {
    const expected = captureAuthoritativeWorldDigest(makeWorldFixture({ negativeZeroWeight: true }));
    const actual = captureAuthoritativeWorldDigest(makeWorldFixture({ negativeZeroWeight: false }));
    const difference = findFirstAuthoritativeWorldDivergence(expected, actual);

    expect(actual.digest).not.toBe(expected.digest);
    expect(difference).toMatchObject({
      path: '10.population.slot=000000000000.weights[1]',
      expected: 'float32-bits:0x80000000',
      actual: 'float32-bits:0x00000000',
      expectedTick: 42,
      actualTick: 42,
      populationSlot: 0,
      brainType: 'gru',
      brainNode: null
    });
    expect(difference?.message).toContain('slot=0 brain=gru node=n/a');
  });

  it('reports recurrent brain-node context at first divergence', () => {
    const expected = captureAuthoritativeWorldDigest(makeWorldFixture({ recurrentValue: 0.25 }));
    const actual = captureAuthoritativeWorldDigest(makeWorldFixture({ recurrentValue: 0.5 }));
    const difference = findFirstAuthoritativeWorldDivergence(expected, actual);

    expect(difference).toMatchObject({
      path: '20.snakes.population=000000000000.brain.node=memory.h[1]',
      expected: 'float32-bits:0x3e800000',
      actual: 'float32-bits:0x3f000000',
      populationSlot: 0,
      brainType: 'gru',
      brainNode: 'memory'
    });
    expect(difference?.message).toContain('tick=42/42');
  });

  it('rejects non-finite Float32 weights instead of normalizing them', () => {
    const error = captureFailure((world) => {
      world.population[0]!.weights[2] = Number.NaN;
    });

    expect(error.path).toBe('10.population.slot=000000000000.weights[2]');
    expect(error.message).toContain('non-finite value NaN');
  });

  it('rejects non-finite continuous World values instead of normalizing them', () => {
    const error = captureFailure((world) => {
      world.pellets[0]!.x = Number.POSITIVE_INFINITY;
    });

    expect(error.path).toBe('30.pellets.source[0].x');
    expect(error.message).toContain('non-finite value Infinity');
  });
});
