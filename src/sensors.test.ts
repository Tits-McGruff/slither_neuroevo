import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { buildSensors } from './sensors.ts';
import { CFG } from './config.ts';

describe('sensors.ts', () => {
  /** Snapshot of default bubble bin count for cleanup. */
  const originalBins = CFG.sense.bubbleBins;
  /** Snapshot of default boost threshold for cleanup. */
  const originalMinBoost = CFG.boost.minPointsToBoost;

  beforeEach(() => {
    CFG.sense.bubbleBins = 8;
    CFG.boost.minPointsToBoost = 5;
  });

  afterEach(() => {
    CFG.sense.bubbleBins = originalBins;
    CFG.boost.minPointsToBoost = originalMinBoost;
  });

  /**
   * Builds a minimal snake stub with optional overrides.
   * @param overrides - Additional fields to merge into the stub.
   * @returns Snake-like object for sensor tests.
   */
  function makeSnake(overrides: Record<string, unknown> = {}) {
    return {
      x: 0,
      y: 0,
      dir: 0,
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
    { pellets = [], bestPointsThisGen = 1 }: { pellets?: Array<{ x: number; y: number; v: number }>; bestPointsThisGen?: number } = {}
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
      pelletGrid: { cellSize, map },
      _collGrid: {}
    };
  }

  it('buildSensors returns the expected buffer length', () => {
    const snake = makeSnake();
    const world = makeWorld();
    const expected = 5 + 3 * CFG.sense.bubbleBins;
    const out = new Float32Array(expected);

    const sensors = buildSensors(world, snake, out);

    expect(sensors).toBe(out);
    expect(sensors.length).toBe(expected);
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

    const sensors = buildSensors(world, snake);
    const foodOffset = 5;

    expect(sensors[foodOffset]).toBeGreaterThan(-1);
  });

  it('reports clear hazard and wall bins when nothing is nearby', () => {
    const snake = makeSnake();
    const world = makeWorld();
    const sensors = buildSensors(world, snake);
    const bins = CFG.sense.bubbleBins;
    const hazardOffset = 5 + bins;
    const wallOffset = hazardOffset + bins;

    for (let i = 0; i < bins; i++) {
      expect(sensors[hazardOffset + i]).toBeCloseTo(1);
      expect(sensors[wallOffset + i]).toBeCloseTo(1);
    }
  });
});
