import { readFileSync } from 'node:fs';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { CFG, resetCFGToDefaults } from './config.ts';
import { POINTS_DELTA_SENSOR_DESCRIPTION } from './protocol/sensors.ts';
import { Pellet, type Snake } from './snake.ts';
import {
  World,
  type BatchInferenceRunner,
  type ControllerRegistryLike
} from './world.ts';

/** Phase 5 score-observation contract suite label. */
const SUITE = 'recovery Phase 5 — score-delta sensor semantics';
/** Fixed index of points_delta_norm in the v3 scalar layout. */
const POINTS_DELTA_INDEX = 8;
/** Production divisor used before clamping the score delta. */
const POINTS_DELTA_SCALE = 10;

/** World and population snake returned by the focused fixture. */
interface SensorWorldFixture {
  /** Deterministic production World. */
  world: World;
  /** Sole population snake in the World. */
  snake: Snake;
}

/** Delivery paths that must share one score-observation boundary. */
type DeliveryPath = 'external' | 'pooled' | 'serial';

beforeEach(() => {
  resetCFGToDefaults();
  CFG.baselineBots.count = 0;
  CFG.pelletCountTarget = 0;
  CFG.pelletSpawnPerSecond = 0;
  CFG.collision.substepMaxDt = 1;
  CFG.generationSeconds = 1000;
  CFG.observer.earlyEndAliveThreshold = -1;
  CFG.reward.pointsPerSecondAlive = 0;
});

afterEach(() => {
  resetCFGToDefaults();
});

/**
 * Create a one-snake production World with deterministic construction.
 * @param seed - Authoritative World seed.
 * @returns World and its sole population snake.
 */
function createSensorWorld(seed = 5105): SensorWorldFixture {
  const world = new World({ snakeCount: 1 }, { seed, inferenceBackend: 'js' });
  const snake = world.snakes[0];
  if (!snake) throw new Error('Phase 5 fixture requires one population snake');
  return { world, snake };
}

/**
 * Read the normalized score-delta channel from one observation.
 * @param sensors - Delivered sensor vector.
 * @returns Normalized score delta.
 */
function scoreDelta(sensors: Float32Array): number {
  return sensors[POINTS_DELTA_INDEX] ?? Number.NaN;
}

/**
 * Normalize prose for a source-contract/documentation equality assertion.
 * @param value - Source or Markdown prose.
 * @returns Prose without Markdown ticks or layout whitespace differences.
 */
function normalizeProse(value: string): string {
  return value.replaceAll('`', '').replace(/\s+/g, ' ').trim();
}

/**
 * Observe one score delta through a production delivery branch.
 * @param path - External, serial neural, or pooled neural delivery branch.
 * @returns Delivered normalized delta and committed marker.
 */
async function observeDeliveryPath(path: DeliveryPath): Promise<{
  delta: number;
  marker: number;
}> {
  const { world, snake } = createSensorWorld(6000 + path.length);
  snake.pointsScore = 3;
  let observed = Number.NaN;

  if (path === 'external') {
    const controllers: ControllerRegistryLike = {
      isControlled: snakeId => snakeId === snake.id,
      getAction: () => ({ turn: 0, boost: 0 }),
      publishSensors: (_snakeId, _tickId, sensors) => {
        observed = scoreDelta(sensors);
      }
    };
    await world.step(1 / 60, 800, 600, controllers, 1);
  } else if (path === 'serial') {
    snake.brain.forward = sensors => {
      observed = scoreDelta(sensors);
      return Float32Array.of(0, 0);
    };
    await world.step(1 / 60, 800, 600, undefined, 1);
  } else {
    const runner: BatchInferenceRunner = {
      runBatch: async (inputs, outputs, _indices, count, inputStride, outputStride) => {
        expect(count).toBe(1);
        observed = inputs[POINTS_DELTA_INDEX] ?? Number.NaN;
        outputs[0] = 0;
        if (outputStride > 1) outputs[1] = 0;
        expect(inputStride).toBe(CFG.brain.inSize);
      }
    };
    await world.step(1 / 60, 800, 600, undefined, 1, runner);
  }

  return { delta: observed, marker: snake.pointsAtLastSensorSample };
}

describe(SUITE, () => {
  it('SNS-001 defines the first sample as score change since construction', () => {
    const { world, snake } = createSensorWorld();
    snake.pointsScore = 2.5;

    const sensors = snake.sampleSensors(world);

    expect(scoreDelta(sensors)).toBeCloseTo(2.5 / POINTS_DELTA_SCALE, 6);
    expect(snake.pointsAtLastSensorSample).toBe(2.5);
  });

  it('keeps construction pure and commits the marker only for a delivered sample', () => {
    const { world, snake } = createSensorWorld();
    snake.pointsScore = 4;

    expect(scoreDelta(snake.computeSensors(world))).toBeCloseTo(0.4, 6);
    expect(scoreDelta(snake.computeSensors(world))).toBeCloseTo(0.4, 6);
    expect(snake.pointsAtLastSensorSample).toBe(0);
    expect(scoreDelta(snake.sampleSensors(world))).toBeCloseTo(0.4, 6);
    expect(snake.pointsAtLastSensorSample).toBe(4);
    expect(scoreDelta(snake.sampleSensors(world))).toBe(0);
  });

  it('reports survival reward once at the next delivered observation', () => {
    CFG.reward.pointsPerSecondAlive = 2;
    const { world, snake } = createSensorWorld();

    snake.prepareForStep(0.5);

    expect(scoreDelta(snake.sampleSensors(world))).toBeCloseTo(0.1, 6);
    expect(scoreDelta(snake.sampleSensors(world))).toBe(0);
  });

  it('reports food gained after one observation in the next observation', () => {
    CFG.reward.pointsPerFood = 3;
    const { world, snake } = createSensorWorld();
    snake.sampleSensors(world);
    world.addPellet(new Pellet(snake.x, snake.y, 2));

    snake.advance(world, 0);

    expect(snake.foodEaten).toBe(2);
    expect(scoreDelta(snake.sampleSensors(world))).toBeCloseTo(0.6, 6);
    expect(scoreDelta(snake.sampleSensors(world))).toBe(0);
  });

  it('reports kill reward in the next observation', () => {
    CFG.reward.pointsPerKill = 4;
    const { world, snake } = createSensorWorld();
    snake.sampleSensors(world);

    snake.killScore += 1;
    snake.pointsScore += CFG.reward.pointsPerKill;

    expect(scoreDelta(snake.sampleSensors(world))).toBeCloseTo(0.4, 6);
    expect(scoreDelta(snake.sampleSensors(world))).toBe(0);
  });

  it('reports boost spending as a negative delta', () => {
    CFG.snakeMinLen = 3;
    CFG.boost.pointsCostPerSecond = 2;
    CFG.boost.pointsCostSizeFactor = 0;
    CFG.boost.lenLossPerPoint = 0;
    const { world, snake } = createSensorWorld();
    snake.pointsScore = 20;
    snake.sampleSensors(world);
    snake.boostInput = 1;

    snake.advance(world, 0.5);

    expect(scoreDelta(snake.sampleSensors(world))).toBeCloseTo(-0.1, 6);
    expect(scoreDelta(snake.sampleSensors(world))).toBe(0);
  });

  it('accumulates score changes across skipped neural control intervals', async () => {
    const baseDt = 0.25;
    CFG.brain.controlDt = baseDt * 3;
    CFG.reward.pointsPerSecondAlive = 2;
    const { world, snake } = createSensorWorld();
    const observations: number[] = [];
    snake.brain.forward = sensors => {
      observations.push(scoreDelta(sensors));
      return Float32Array.of(0, 0);
    };

    await world.step(baseDt, 800, 600, undefined, 1);
    await world.step(baseDt, 800, 600, undefined, 2);
    await world.step(baseDt, 800, 600, undefined, 3);

    expect(observations).toHaveLength(2);
    expect(observations[0]).toBeCloseTo(0.05, 6);
    expect(observations[1]).toBeCloseTo(0.1, 6);
  });

  it('SNS-002 gives external, serial, and pooled delivery the same sample', async () => {
    const [external, serial, pooled] = await Promise.all([
      observeDeliveryPath('external'),
      observeDeliveryPath('serial'),
      observeDeliveryPath('pooled')
    ]);

    expect(external.delta).toBeCloseTo(0.3, 6);
    expect(serial.delta).toBeCloseTo(external.delta, 6);
    expect(pooled.delta).toBeCloseTo(external.delta, 6);
    expect([external.marker, serial.marker, pooled.marker]).toEqual([3, 3, 3]);
  });

  it('keeps the source sensor contract and API wording aligned', () => {
    const apiInstructions = readFileSync(
      new URL('../docs/API-instructions.md', import.meta.url),
      'utf8'
    );

    expect(normalizeProse(apiInstructions)).toContain(
      normalizeProse(POINTS_DELTA_SENSOR_DESCRIPTION)
    );
  });
});
