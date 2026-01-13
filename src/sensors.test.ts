import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { angleToCenteredBin, buildSensors, computeSensorRadii } from './sensors.ts';
import { getSensorLayout } from './protocol/sensors.ts';
import { CFG, resetCFGToDefaults } from './config.ts';

describe('sensors.ts', () => {
  /** Snapshot of default bubble bin count for cleanup. */
  const originalBins = CFG.sense.bubbleBins;
  /** Snapshot of default boost threshold for cleanup. */
  const originalMinBoost = CFG.boost.minPointsToBoost;
  /** Snapshot of default sensor layout version for cleanup. */
  const originalLayoutVersion = CFG.sense.layoutVersion;
  /** Snapshot of default v2 near radius base for cleanup. */
  const originalRNearBase = CFG.sense.rNearBase;
  /** Snapshot of default v2 near radius scale for cleanup. */
  const originalRNearScale = CFG.sense.rNearScale;
  /** Snapshot of default v2 near radius min for cleanup. */
  const originalRNearMin = CFG.sense.rNearMin;
  /** Snapshot of default v2 near radius max for cleanup. */
  const originalRNearMax = CFG.sense.rNearMax;
  /** Snapshot of default v2 far radius base for cleanup. */
  const originalRFarBase = CFG.sense.rFarBase;
  /** Snapshot of default v2 far radius scale for cleanup. */
  const originalRFarScale = CFG.sense.rFarScale;
  /** Snapshot of default v2 far radius min for cleanup. */
  const originalRFarMin = CFG.sense.rFarMin;
  /** Snapshot of default v2 far radius max for cleanup. */
  const originalRFarMax = CFG.sense.rFarMax;
  /** Snapshot of default v2 food saturation base for cleanup. */
  const originalFoodKBase = CFG.sense.foodKBase;
  /** Snapshot of world radius for cleanup. */
  const originalWorldRadius = CFG.worldRadius;
  /** Snapshot of collision hit scale for cleanup. */
  const originalHitScale = CFG.collision.hitScale;

  beforeEach(() => {
    CFG.sense.bubbleBins = 8;
    CFG.sense.layoutVersion = 'v2';
    CFG.boost.minPointsToBoost = 5;
  });

  afterEach(() => {
    CFG.sense.bubbleBins = originalBins;
    CFG.boost.minPointsToBoost = originalMinBoost;
    CFG.sense.layoutVersion = originalLayoutVersion;
    CFG.sense.rNearBase = originalRNearBase;
    CFG.sense.rNearScale = originalRNearScale;
    CFG.sense.rNearMin = originalRNearMin;
    CFG.sense.rNearMax = originalRNearMax;
    CFG.sense.rFarBase = originalRFarBase;
    CFG.sense.rFarScale = originalRFarScale;
    CFG.sense.rFarMin = originalRFarMin;
    CFG.sense.rFarMax = originalRFarMax;
    CFG.sense.foodKBase = originalFoodKBase;
    CFG.worldRadius = originalWorldRadius;
    CFG.collision.hitScale = originalHitScale;
  });

  /**
   * Builds a minimal snake stub with optional overrides.
   * @param overrides - Additional fields to merge into the stub.
   * @returns Snake-like object for sensor tests.
   */
  function makeSnake(overrides: Record<string, unknown> = {}) {
    return {
      id: 1,
      x: 0,
      y: 0,
      dir: 0,
      speed: CFG.snakeBaseSpeed,
      boost: 0,
      pointsScore: 0,
      points: [],
      radius: CFG.snakeRadius,
      alive: true,
      length: () => CFG.snakeStartLen,
      sizeNorm: () => 0,
      ...overrides
    };
  }

  /**
   * Builds a world stub with pellet grid and fitness tracking fields.
   * @param options - World options such as pellets and best points.
   * @returns World-like object for sensor tests.
   */
  function makeWorld(
    {
      pellets = [],
      bestPointsThisGen = 1,
      snakes = []
    }: {
      pellets?: Array<{ x: number; y: number; v: number }>;
      bestPointsThisGen?: number;
      snakes?: ReturnType<typeof makeSnake>[];
    } = {}
  ) {
    const cellSize = 120;
    const map = new Map<string, Array<{ x: number; y: number; v: number }>>();
    for (const pellet of pellets) {
      const cx = Math.floor(pellet.x / cellSize);
      const cy = Math.floor(pellet.y / cellSize);
      const key = `${cx},${cy}`;
      const arr = map.get(key) || [];
      arr.push(pellet);
      map.set(key, arr);
    }

    return {
      pellets,
      bestPointsThisGen,
      snakes,
      pelletGrid: { cellSize, map },
      _collGrid: {}
    };
  }

  it('buildSensors returns the expected buffer length', () => {
    const snake = makeSnake();
    const world = makeWorld();
    const layout = getSensorLayout(CFG.sense.bubbleBins, CFG.sense.layoutVersion);
    const expected = layout.inputSize;
    const out = new Float32Array(expected);

    const sensors = buildSensors(world, snake, out);

    expect(sensors).toBe(out);
    expect(sensors.length).toBe(expected);
  });

  it('getSensorLayout matches the legacy layout contract', () => {
    const bins = 12;
    const layout = getSensorLayout(bins, 'legacy');

    expect(layout.scalarCount).toBe(5);
    expect(layout.channelCount).toBe(3);
    expect(layout.inputSize).toBe(5 + 3 * bins);
    expect(layout.offsets.food).toBe(5);
    expect(layout.offsets.hazard).toBe(5 + bins);
    expect(layout.offsets.wall).toBe(5 + 2 * bins);
    expect(layout.offsets.head).toBeNull();
    expect(layout.order.length).toBe(layout.inputSize);
  });

  it('encodes heading, size, boost margin, and points percentile', () => {
    const snake = makeSnake({
      dir: 0,
      pointsScore: 5,
      sizeNorm: () => 0.5
    });
    const world = makeWorld({ bestPointsThisGen: 5 });

    const sensors = buildSensors(world, snake);

    expect(sensors[0]).toBeCloseTo(0);
    expect(sensors[1]).toBeCloseTo(1);
    expect(sensors[2]).toBeCloseTo(0);
    expect(sensors[3]).toBeCloseTo(0);
    expect(sensors[4]).toBeCloseTo(1);
  });

  it('updates the food histogram for nearby pellets', () => {
    const snake = makeSnake({ pointsScore: 5 });
    const pellet = { x: 100, y: 0, v: CFG.foodValue };
    const world = makeWorld({ pellets: [pellet], bestPointsThisGen: 5 });
    const layout = getSensorLayout(CFG.sense.bubbleBins, CFG.sense.layoutVersion);

    const sensors = buildSensors(world, snake);
    const foodOffset = layout.offsets.food;
    const expectedBin = layout.layoutVersion === 'v2'
      ? angleToCenteredBin(0, layout.bins)
      : 0;

    expect(sensors[foodOffset + expectedBin]).toBeGreaterThan(-1);
  });

  it('reports clear hazard and wall bins when nothing is nearby', () => {
    const snake = makeSnake();
    const world = makeWorld();
    const sensors = buildSensors(world, snake);
    const layout = getSensorLayout(CFG.sense.bubbleBins, CFG.sense.layoutVersion);
    const bins = layout.bins;
    const hazardOffset = layout.offsets.hazard;
    const wallOffset = layout.offsets.wall;

    for (let i = 0; i < bins; i++) {
      expect(sensors[hazardOffset + i]).toBeCloseTo(1);
      expect(sensors[wallOffset + i]).toBeCloseTo(1);
    }
  });

  it('defaults to the v2 layout input size after reset', () => {
    resetCFGToDefaults();
    const layout = getSensorLayout(CFG.sense.bubbleBins, CFG.sense.layoutVersion);

    expect(CFG.sense.layoutVersion).toBe('v2');
    expect(CFG.brain.inSize).toBe(layout.inputSize);
  });

  /**
   * Configure deterministic v2 sensor values for unit tests.
   * @param bins - Bin count for v2 layouts.
   */
  function configureV2(bins: number): void {
    CFG.sense.layoutVersion = 'v2';
    CFG.sense.bubbleBins = bins;
    CFG.sense.rNearBase = 100;
    CFG.sense.rNearScale = 0;
    CFG.sense.rNearMin = 100;
    CFG.sense.rNearMax = 100;
    CFG.sense.rFarBase = 200;
    CFG.sense.rFarScale = 0;
    CFG.sense.rFarMin = 200;
    CFG.sense.rFarMax = 200;
    CFG.sense.foodKBase = 1.0;
  }

  /**
   * Find the index of the largest value within a bin window.
   * @param sensors - Sensor array.
   * @param offset - Channel offset.
   * @param bins - Bin count.
   * @returns Index of the maximum bin.
   */
  function findMaxBin(sensors: Float32Array, offset: number, bins: number): number {
    let bestIdx = 0;
    let bestVal = -Infinity;
    for (let i = 0; i < bins; i++) {
      const val = sensors[offset + i] ?? -Infinity;
      if (val > bestVal) {
        bestVal = val;
        bestIdx = i;
      }
    }
    return bestIdx;
  }

  it('computes v2 radii with monotonic clamps', () => {
    CFG.sense.layoutVersion = 'v2';
    CFG.sense.rNearBase = 150;
    CFG.sense.rNearScale = 80;
    CFG.sense.rNearMin = 120;
    CFG.sense.rNearMax = 400;
    CFG.sense.rFarBase = 240;
    CFG.sense.rFarScale = 160;
    CFG.sense.rFarMin = 220;
    CFG.sense.rFarMax = 700;

    const small = computeSensorRadii(0);
    const large = computeSensorRadii(1);

    expect(large.rNear).toBeGreaterThanOrEqual(small.rNear);
    expect(large.rFar).toBeGreaterThanOrEqual(small.rFar);
    expect(small.rFar).toBeGreaterThanOrEqual(small.rNear + 1);
    expect(large.rFar).toBeGreaterThanOrEqual(large.rNear + 1);
  });

  it('uses centered bin mapping for v2 food bins', () => {
    configureV2(16);
    const snake = makeSnake({ dir: 0 });
    const layout = getSensorLayout(CFG.sense.bubbleBins, 'v2');

    const pelletForward = { x: 80, y: 0, v: CFG.foodValue * 2 };
    const worldForward = makeWorld({ pellets: [pelletForward], bestPointsThisGen: 5, snakes: [snake] });
    const sensorsForward = buildSensors(worldForward, snake);
    const foodOffset = layout.offsets.food;
    const forwardIdx = angleToCenteredBin(0, layout.bins);
    expect(findMaxBin(sensorsForward, foodOffset, layout.bins)).toBe(forwardIdx);

    const pelletBack = { x: -80, y: 0.001, v: CFG.foodValue * 2 };
    const worldBack = makeWorld({ pellets: [pelletBack], bestPointsThisGen: 5, snakes: [snake] });
    const sensorsBack = buildSensors(worldBack, snake);
    const backAngle = Math.atan2(pelletBack.y, pelletBack.x);
    const backIdx = angleToCenteredBin(backAngle, layout.bins);
    expect(findMaxBin(sensorsBack, foodOffset, layout.bins)).toBe(backIdx);
  });

  it('matches hitScale when computing v2 hazard clearance', () => {
    configureV2(8);
    CFG.collision.hitScale = 1.0;
    const snake = makeSnake({ id: 1, radius: 10 });
    const other = makeSnake({
      id: 2,
      radius: 10,
      points: [{ x: 70, y: 0 }, { x: 70, y: 0 }]
    });
    const world = makeWorld({ pellets: [], bestPointsThisGen: 5, snakes: [snake, other] });
    world._collGrid = {
      cellSize: 500,
      map: new Map(),
      query: () => [{ s: other, i: 1 }]
    };

    const sensors = buildSensors(world, snake);
    const layout = getSensorLayout(CFG.sense.bubbleBins, 'v2');
    const hazardIdx = angleToCenteredBin(0, layout.bins);
    const hazardValue = sensors[layout.offsets.hazard + hazardIdx];

    expect(hazardValue).toBeCloseTo(0, 4);
  });

  it('normalizes v2 wall clearance by rNear', () => {
    configureV2(8);
    CFG.worldRadius = 100;
    const snake = makeSnake({ x: 80, y: 0, radius: 10, dir: 0 });
    const world = makeWorld({ pellets: [], bestPointsThisGen: 5, snakes: [snake] });
    const sensors = buildSensors(world, snake);
    const layout = getSensorLayout(CFG.sense.bubbleBins, 'v2');
    const wallIdx = angleToCenteredBin(0, layout.bins);
    const wallValue = sensors[layout.offsets.wall + wallIdx];

    expect(wallValue).toBeCloseTo(-0.8, 3);
  });

  it('keeps head pressure head-only in v2', () => {
    configureV2(12);
    const snake = makeSnake({ id: 1 });
    const other = makeSnake({
      id: 2,
      points: [
        { x: 50, y: 0 },
        { x: 0, y: 80 }
      ]
    });
    const world = makeWorld({ pellets: [], bestPointsThisGen: 5, snakes: [snake, other] });
    const sensors = buildSensors(world, snake);
    const layout = getSensorLayout(CFG.sense.bubbleBins, 'v2');
    const headOffset = layout.offsets.head!;
    const headIdx = angleToCenteredBin(0, layout.bins);
    const bodyIdx = angleToCenteredBin(Math.PI / 2, layout.bins);

    expect(sensors[headOffset + headIdx]).toBeLessThan(1);
    expect(sensors[headOffset + bodyIdx]).toBeCloseTo(1);
  });

  it('produces deterministic v2 food bins with fixed pellets', () => {
    configureV2(10);
    const snake = makeSnake({ id: 3 });
    const pellets = [
      { x: 30, y: 10, v: CFG.foodValue },
      { x: -40, y: -20, v: CFG.foodValue * 0.5 }
    ];
    const world = makeWorld({ pellets, bestPointsThisGen: 5, snakes: [snake] });

    const first = buildSensors(world, snake);
    const second = buildSensors(world, snake);

    expect(Array.from(second)).toEqual(Array.from(first));
  });
});
