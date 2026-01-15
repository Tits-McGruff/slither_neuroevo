import { describe, it, expect, vi, beforeEach, afterEach, type Mock } from 'vitest';
import { BaselineBotManager } from './baselineBots.ts';
import { getSensorLayout } from '../protocol/sensors.ts';
import { angleToCenteredBin } from '../sensors.ts';
import { CFG } from '../config.ts';
import type { World } from '../world.ts';
import type { Snake } from '../snake.ts';

describe('BaselineBotManager AI', () => {
  /** Snapshot of default sensor bin count for cleanup. */
  const originalBins = CFG.sense.bubbleBins;
  /** Snapshot of default sensor layout version for cleanup. */
  const originalLayoutVersion = CFG.sense.layoutVersion;

  let manager: BaselineBotManager;
  let mockWorld: World;
  let mockSnake: Snake;

  beforeEach(() => {
    CFG.sense.bubbleBins = 12;
    CFG.sense.layoutVersion = 'v2';
    manager = new BaselineBotManager({ count: 1, seed: 123, randomizeSeedPerGen: false });

    mockWorld = {
      baselineBots: [],
      snakes: []
    } as unknown as World;

    mockSnake = {
      id: 100,
      alive: true,
      pointsScore: 1000,
      radius: 10,
      dir: 0,
      head: vi.fn().mockReturnValue({ x: 0, y: 0 }),
      computeSensors: vi.fn(),
      length: vi.fn().mockReturnValue(20)
    } as unknown as Snake;

    mockWorld.baselineBots[0] = mockSnake;
    (mockWorld.snakes as unknown as Snake[]) = [mockSnake];

    manager.registerBot(0, 100);
  });

  afterEach(() => {
    CFG.sense.bubbleBins = originalBins;
    CFG.sense.layoutVersion = originalLayoutVersion;
  });

  /**
   * Create a sensor array with the expected layout:
   * - 5 global values
   * - food[bins], hazard[bins], wall[bins]
   */
  function makeSensors(bins = 12): Float32Array {
    const layout = getSensorLayout(bins, 'v2');
    return new Float32Array(layout.inputSize);
  }

  /**
   * Set a single bin in the sensor array.
   *
   * @param sensors - Sensor array.
   * @param bins - Number of bins.
   * @param binIdx - Bin index.
   * @param food - Food value in [-1, 1].
   * @param hazard - Hazard value in [-1, 1] (higher is safer).
   * @param wall - Wall value in [-1, 1] (higher is safer).
   */
  function setBin(
    sensors: Float32Array,
    bins: number,
    binIdx: number,
    food: number,
    hazard: number,
    wall: number
  ) {
    const layout = getSensorLayout(bins, 'v2');
    sensors[layout.offsets.food + binIdx] = food;
    sensors[layout.offsets.hazard + binIdx] = hazard;
    sensors[layout.offsets.wall + binIdx] = wall;
  }

  /**
   * Infer a chosen bin index from the action turn output.
   *
   * This works reliably when the bot is not roaming (roam adds wander noise).
   *
   * @param turn - Action turn in [-1, 1].
   * @param bins - Number of bins.
   */
  function inferBinFromTurn(turn: number, bins: number): number {
    const clamped = Math.max(-1, Math.min(1, turn));
    const targetAngle = clamped * (Math.PI / 2);
    return angleToCenteredBin(targetAngle, bins);
  }

  it('prefers safe clearance over clamped food (small bot in seek state)', () => {
    const bins = 12;
    const sensors = makeSensors(bins);

    // Default: no food, fully clear.
    for (let i = 0; i < bins; i++) setBin(sensors, bins, i, -1, 1, 1);

    // Ensure seek state by placing at least one food value above the seek trigger.
    setBin(sensors, bins, 2, 0.2, 1, 1);

    // Forward bin: high food but poor clearance (still above avoid trigger).
    // clearance = (-0.9 + 1.0) / 2 = 0.05
    setBin(sensors, bins, 0, 1.0, -0.9, 1.0);

    // A safe bin should be preferred over the risky forward bin.
    setBin(sensors, bins, 6, -1.0, 1.0, 1.0);

    (mockSnake.computeSensors as Mock).mockReturnValue(sensors);

    manager.update(mockWorld, 0.1, vi.fn());

    const action = manager.getActionForSnake(100);
    expect(action).not.toBeNull();

    const chosen = inferBinFromTurn(action!.turn, bins);
    expect(chosen).not.toBe(0);
  });

  it('penalizes vetoed clearance bins even if food is high', () => {
    const bins = 12;
    const sensors = makeSensors(bins);

    for (let i = 0; i < bins; i++) setBin(sensors, bins, i, -1, 1, 1);

    // Force seek state.
    setBin(sensors, bins, 2, 0.2, 1, 1);

    // Veto is based on clearance = avg(hazard, wall).
    // To guarantee veto: clearance < -0.5.
    setBin(sensors, bins, 0, 1.0, -1.0, -1.0);

    // A safe alternative.
    setBin(sensors, bins, 1, -1.0, 1.0, 1.0);

    (mockSnake.computeSensors as Mock).mockReturnValue(sensors);

    manager.update(mockWorld, 0.1, vi.fn());

    const action = manager.getActionForSnake(100);
    expect(action).not.toBeNull();

    const chosen = inferBinFromTurn(action!.turn, bins);
    expect(chosen).not.toBe(0);
  });

  it('falls back to best clearance when all bins are vetoed', () => {
    const bins = 12;
    const sensors = makeSensors(bins);

    // Make every bin vetoed: clearance < -0.5 everywhere.
    for (let i = 0; i < bins; i++) {
      setBin(sensors, bins, i, -1, -1, -1); // clearance = -1
    }

    // Force seek state (prevents roam wander)
    setBin(sensors, bins, 2, 0.2, -1, -1);

    // Pick a bin whose angle is within [-pi/2, pi/2] so turn does not saturate.
    const bestClearIdx = 3;

    // Make it the "least bad" clearance among vetoed bins:
    // clearance = (-0.6 + -0.6)/2 = -0.6 (still vetoed, but best)
    setBin(sensors, bins, bestClearIdx, -1, -0.6, -0.6);

    (mockSnake.computeSensors as Mock).mockReturnValue(sensors);

    manager.update(mockWorld, 0.1, vi.fn());

    const action = manager.getActionForSnake(100);
    expect(action).not.toBeNull();

    // Expected turn uses the same mapping as BaselineBotManager (and includes clamp).
    const TAU = Math.PI * 2;
    const binAngle = -Math.PI + (bestClearIdx / bins) * TAU;
    const expectedTurn = Math.max(-1, Math.min(1, binAngle / (Math.PI / 2)));

    expect(action!.turn).toBeCloseTo(expectedTurn, 6);
  });
});
