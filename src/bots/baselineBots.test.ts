import { describe, it, expect, vi, beforeEach, type Mock } from 'vitest';
import { BaselineBotManager } from './baselineBots.ts';
import type { World } from '../world.ts';
import type { Snake } from '../snake.ts';

describe('BaselineBotManager AI', () => {
  let manager: BaselineBotManager;
  let mockWorld: World;
  let mockSnake: Snake;

  beforeEach(() => {
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

  /**
   * Create a sensor array with the expected layout:
   * - 5 global values
   * - food[bins], hazard[bins], wall[bins]
   */
  function makeSensors(bins = 12): Float32Array {
    return new Float32Array(5 + 3 * bins);
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
    sensors[5 + binIdx] = food;
    sensors[5 + bins + binIdx] = hazard;
    sensors[5 + 2 * bins + binIdx] = wall;
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
    const TAU = Math.PI * 2;
    const targetAngle = turn * (Math.PI / 2);
    const a = targetAngle < 0 ? targetAngle + TAU : targetAngle;
    return Math.round((a / TAU) * bins) % bins;
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
      setBin(sensors, bins, i, -1, -1, -1);
    }

    // Force seek state (prevents roam wander).
    // This bin is still vetoed, but it ensures bestFood > 0.1.
    setBin(sensors, bins, 2, 0.2, -1, -1);

    // Make one bin the least bad clearance among vetoed bins.
    // clearance = (-0.6 + -0.6)/2 = -0.6 (still vetoed, but best).
    setBin(sensors, bins, 7, -1, -0.6, -0.6);

    (mockSnake.computeSensors as Mock).mockReturnValue(sensors);

    manager.update(mockWorld, 0.1, vi.fn());

    const action = manager.getActionForSnake(100);
    expect(action).not.toBeNull();

    const chosen = inferBinFromTurn(action!.turn, bins);
    expect(chosen).toBe(7);
  });
});
