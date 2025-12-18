import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { buildSensors } from './sensors.js';
import { CFG } from './config.js';

describe('sensors.js', () => {
  const originalBins = CFG.sense.bubbleBins;
  const originalMinBoost = CFG.boost.minPointsToBoost;

  beforeEach(() => {
    CFG.sense.bubbleBins = 8;
    CFG.boost.minPointsToBoost = 5;
  });

  afterEach(() => {
    CFG.sense.bubbleBins = originalBins;
    CFG.boost.minPointsToBoost = originalMinBoost;
  });

  function makeSnake(overrides = {}) {
    return {
      x: 0,
      y: 0,
      dir: 0,
      pointsScore: 0,
      length: () => CFG.snakeStartLen,
      sizeNorm: () => 0,
      ...overrides
    };
  }

  function makeWorld({ pellets = [], bestPointsThisGen = 1 } = {}) {
    const cellSize = 120;
    const map = new Map();
    for (const pellet of pellets) {
      const cx = Math.floor(pellet.x / cellSize);
      const cy = Math.floor(pellet.y / cellSize);
      const key = `${cx},${cy}`;
      const arr = map.get(key) || [];
      arr.push(pellet);
      map.set(key, arr);
    }

    return {
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
