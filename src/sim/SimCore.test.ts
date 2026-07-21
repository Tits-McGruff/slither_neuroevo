import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { CFG, resetCFGToDefaults } from '../config.ts';
import type { World } from '../world.ts';
import { SimCore } from './SimCore.ts';

/** Phase 1 scheduler regression suite label. */
const SUITE = 'SimCore fixed-step scheduler';

/** Deterministic state advanced by the hand-owned scheduled World. */
interface ScheduledState {
  /** Ordered fixed deltas received by World. */
  stepDts: number[];
  /** Ordered tick ids assigned to World. */
  tickIds: number[];
  /** Deterministic recurrence used for exact grouping comparisons. */
  value: number;
}

/** SimCore plus its hand-owned deterministic World state. */
interface ScheduledCoreFixture {
  /** Production scheduler under test. */
  core: SimCore;
  /** State mutated only by successful fixed World steps. */
  state: ScheduledState;
}

beforeEach(() => {
  resetCFGToDefaults();
  CFG.baselineBots.count = 0;
  CFG.pelletCountTarget = 0;
});

afterEach(() => {
  resetCFGToDefaults();
});

/**
 * Build a production SimCore with a deterministic hand-owned World step.
 * @param simSpeed - Requested scheduling multiplier.
 * @param maxStepsPerPump - Optional pump cap.
 * @returns Scheduler fixture.
 */
function buildScheduledCore(
  simSpeed: number,
  maxStepsPerPump = 120
): ScheduledCoreFixture {
  const core = new SimCore({
    settings: { snakeCount: 1, simSpeed },
    tickRateHz: 60,
    maxStepsPerPump
  });
  const state: ScheduledState = { stepDts: [], tickIds: [], value: 0 };
  const world = {
    simSpeed,
    generation: 1,
    fitnessHistory: [],
    tickId: 0,
    step: async (
      baseDt: number,
      _viewW: number,
      _viewH: number,
      _controllers: unknown,
      tickId: number
    ) => {
      state.stepDts.push(baseDt);
      state.tickIds.push(tickId);
      state.value = Math.fround((state.value + tickId) * 1.0001);
      world.tickId = tickId;
    }
  } as unknown as World;
  core.world = world;
  core.lastGeneration = world.generation;
  core.lastHistoryLen = world.fitnessHistory.length;
  return { core, state };
}

/**
 * Pump one scheduler at a steady 60 Hz for one wall-clock second.
 * @param core - Scheduler to pump.
 */
async function pumpOneSecond(core: SimCore): Promise<void> {
  for (let pump = 0; pump < 60; pump++) {
    await core.update(1 / 60);
  }
}

describe(SUITE, () => {
  it.each([
    { speed: 0.1, expectedSteps: 6 },
    { speed: 1, expectedSteps: 60 },
    { speed: 12, expectedSteps: 720 }
  ])('requests and achieves $speed x using whole fixed steps', async ({ speed, expectedSteps }) => {
    const { core, state } = buildScheduledCore(speed);

    await pumpOneSecond(core);

    const diagnostics = core.getSchedulerDiagnostics();
    expect(core.tickId).toBe(expectedSteps);
    expect(state.stepDts).toHaveLength(expectedSteps);
    expect(state.stepDts.every((dt) => dt === core.fixedDt)).toBe(true);
    expect(diagnostics.requestedMultiplier).toBe(speed);
    expect(diagnostics.achievedMultiplier).toBeCloseTo(speed, 10);
    expect(diagnostics.droppedSimulationSeconds).toBe(0);
  });

  it('produces exact state for 100 singly pumped or 12x-grouped steps', async () => {
    const single = buildScheduledCore(1);
    const grouped = buildScheduledCore(12);
    for (let step = 0; step < 100; step++) {
      await single.core.update(single.core.fixedDt);
    }
    for (let pump = 0; pump < 8; pump++) {
      await grouped.core.update(grouped.core.fixedDt);
    }
    await grouped.core.update((4 * grouped.core.fixedDt) / 12);

    expect(single.core.tickId).toBe(100);
    expect(grouped.core.tickId).toBe(100);
    expect(grouped.state).toEqual(single.state);
  });

  it('scheduler jitter changes pump grouping but never the fixed delta or result', async () => {
    const steady = buildScheduledCore(1);
    const jittered = buildScheduledCore(1);
    for (let step = 0; step < 100; step++) {
      await steady.core.update(steady.core.fixedDt);
    }
    for (let pair = 0; pair < 50; pair++) {
      await jittered.core.update(jittered.core.fixedDt * 0.4);
      await jittered.core.update(jittered.core.fixedDt * 1.6);
    }

    expect(jittered.core.tickId).toBe(100);
    expect(jittered.state).toEqual(steady.state);
  });

  it('caps whole steps and reports discarded simulation-time debt honestly', async () => {
    const { core } = buildScheduledCore(500, 12);

    await core.update(1 / 60);

    const diagnostics = core.getSchedulerDiagnostics();
    expect(core.tickId).toBe(12);
    expect(diagnostics.requestedMultiplier).toBe(500);
    expect(diagnostics.achievedMultiplier).toBeCloseTo(12, 10);
    expect(diagnostics.droppedSimulationSecondsThisPump).toBeCloseTo(
      488 * core.fixedDt,
      10
    );
    expect(diagnostics.pendingSimulationSeconds).toBe(0);
  });

  it('does not publish an incremented tick when a fixed World step rejects', async () => {
    const { core } = buildScheduledCore(1);
    core.world.step = async () => {
      throw new Error('fixed-step failure');
    };

    await expect(core.update(core.fixedDt)).rejects.toThrow('fixed-step failure');

    expect(core.tickId).toBe(0);
    expect(core.world.tickId).toBe(0);
    expect(core.getSchedulerDiagnostics().completedSteps).toBe(0);
  });
});
