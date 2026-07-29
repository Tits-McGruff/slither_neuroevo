/** Contract tests for the bounded Stage 2 P5/P6 external-control measurement runner. */

import { afterEach, describe, expect, it } from 'vitest';
import { resetCFGToDefaults } from '../../src/config.ts';
import {
  externalControlComposition,
  installExternalControlScenario,
  p5Composition,
  p6Composition,
  parseOptions,
  schedulerDelta
} from './external-control-baseline.ts';

/** Build the minimal scheduler health shape accepted by the delta helper. */
function health(
  tick: number,
  completedSteps: number,
  wallSeconds: number,
  simulatedSeconds: number,
  droppedSimulationSeconds: number,
  requestedMultiplier = 4
): Parameters<typeof schedulerDelta>[0] {
  return {
    tick,
    clients: 1,
    scheduler: {
      requestedMultiplier,
      achievedMultiplier: wallSeconds > 0 ? simulatedSeconds / wallSeconds : 0,
      completedSteps,
      wallSeconds,
      simulatedSeconds,
      droppedSimulationSeconds
    },
    collisionGrid: {},
    outbound: {},
    fault: { faulted: false, reason: null, tick: null },
    inferenceMode: {},
    persistence: {
      checkpointEveryGenerations: 1,
      lastDurableSnapshotId: 1,
      lastDurableGeneration: 1,
      inMemoryGeneration: 1
    }
  };
}

afterEach(() => {
  resetCFGToDefaults();
});

describe('Stage 2 P5/P6 external-control runner', () => {
  it('accepts only the explicitly measured scheduler multipliers and viewer modes', () => {
    expect(parseOptions([])).toMatchObject({
      profile: 'p5',
      scenario: 'P0',
      simSpeed: 1,
      viewer: true,
      checkpointEvery: 1_000_000,
      playerHz: 30
    });
    expect(parseOptions([
      '--profile', 'p6', '--sim-speed', '12', '--viewer', 'on', '--checkpoint-every', '0'
    ])).toMatchObject({ profile: 'p6', simSpeed: 12, viewer: true, checkpointEvery: 0 });
    expect(parseOptions(['--profile', 'p6'])).toMatchObject({
      checkpointEvery: 1,
      viewer: false
    });
    expect(() => parseOptions([
      '--profile', 'p6', '--checkpoint-every', '2'
    ])).toThrow('1 for the primary matrix or 0 for an explicit diagnostic');
    expect(() => parseOptions(['--sim-speed', '3'])).toThrow('1, 2, 4, 8, or 12');
    expect(() => parseOptions(['--viewer', 'true'])).toThrow('on or off');
    expect(() => parseOptions(['--viewer', 'off'])).toThrow('P5 compatibility');
    expect(() => parseOptions(['--checkpoint-every', '-1'])).toThrow('0 to 1000000');
  });

  it('keeps P5 compatibility and P6 isolation as distinct socket compositions', () => {
    expect(p5Composition()).toEqual({
      botControllers: 1,
      uiPlayers: 1,
      uiSpectators: 1,
      totalSockets: 3
    });
    expect(p6Composition(false)).toEqual({
      botControllers: 1,
      uiPlayers: 0,
      uiSpectators: 0,
      totalSockets: 1
    });
    expect(p6Composition(true)).toEqual({
      botControllers: 1,
      uiPlayers: 0,
      uiSpectators: 1,
      totalSockets: 2
    });
    expect(externalControlComposition({ profile: 'p5', viewer: false })).toEqual(p5Composition());
    expect(externalControlComposition({ profile: 'p6', viewer: false })).toEqual(p6Composition(false));
  });

  it('subtracts scheduler counters including dropped debt and rejects inconsistent steps', () => {
    const delta = schedulerDelta(
      health(100, 100, 10, 10 / 6, 0.25),
      health(220, 220, 15, 22 / 6, 1.0),
      4,
      60
    );
    expect(delta).toMatchObject({
      tick: 120,
      completedSteps: 120,
      wallSeconds: 5,
      droppedSimulationSeconds: 0.75
    });
    expect(delta.simulatedSeconds).toBeCloseTo(2, 12);
    expect(delta.achievedMultiplier).toBeCloseTo(0.4, 12);
    expect(delta.achievedToRequestedRatio).toBeCloseTo(0.1, 12);
    expect(() => schedulerDelta(
      health(1, 1, 1, 1 / 60, 0),
      health(2, 3, 2, 3 / 60, 0),
      4,
      60
    )).toThrow('tick/completed-step mismatch');
  });

  it('accepts ordinary floating-point accumulation across a 10-minute 12x run', () => {
    const completedSteps = 432_000;
    let accumulatedSimulatedSeconds = 0;
    for (let step = 0; step < completedSteps; step++) {
      accumulatedSimulatedSeconds += 1 / 60;
    }

    const delta = schedulerDelta(
      health(0, 0, 0, 0, 0, 12),
      health(
        completedSteps,
        completedSteps,
        600,
        accumulatedSimulatedSeconds,
        0,
        12
      ),
      12,
      60
    );

    expect(delta.simulatedSeconds).toBeCloseTo(7_200, 7);
    expect(delta.achievedMultiplier).toBeCloseTo(12, 9);
    expect(delta.achievedToRequestedRatio).toBeCloseTo(1, 9);
  });

  it('sets simSpeed before the scenario can be passed to a run-start World', () => {
    const scenario = installExternalControlScenario('P1', 8);
    expect(scenario.settings.simSpeed).toBe(8);
    expect(scenario.settings.snakeCount).toBe(300);
  });
});
