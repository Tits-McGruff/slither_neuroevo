/** Contract tests for the bounded Stage 2 P5/P6 external-control measurement runner. */

import { afterEach, describe, expect, it, vi } from 'vitest';
import { resetCFGToDefaults } from '../../src/config.ts';
import {
  externalControlComposition,
  installExternalControlScenario,
  p5Composition,
  p6Composition,
  parseOptions,
  readHealth,
  schedulerDelta,
  tickBoundaryPollDelayMs,
  viewerWarmupReadiness
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
    worldLoad: {
      committedTick: tick,
      generation: 1,
      generationTime: 0,
      populationGenomeCount: 0,
      totalSnakes: 0,
      aliveEvolvedPopulationSnakes: 0,
      aliveBaselineBots: 0,
      aliveExternallyOwnedSnakes: 0,
      aliveNeuralModeNonBaselineUnownedSnakes: 0,
      aliveOtherNonBaselineSnakes: 0,
      aliveTotalSnakes: 0,
      aliveBodyPointCount: 0,
      pelletCount: 0
    },
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
  vi.unstubAllGlobals();
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
      playerHz: 30,
      warmupMs: 2_000,
      durationMs: 15_000,
      warmupTick: null,
      measurementSteps: null
    });
    expect(parseOptions([
      '--profile', 'p6', '--sim-speed', '12', '--viewer', 'on', '--checkpoint-every', '0'
    ])).toMatchObject({ profile: 'p6', simSpeed: 12, viewer: true, checkpointEvery: 0 });
    expect(parseOptions(['--profile', 'p6'])).toMatchObject({
      checkpointEvery: 1,
      viewer: false,
      warmupMs: null,
      durationMs: null,
      warmupTick: 300,
      measurementSteps: 1_800
    });
    expect(parseOptions([
      '--profile', 'p6', '--warmup-tick', '600', '--measurement-steps', '3600'
    ])).toMatchObject({ warmupTick: 600, measurementSteps: 3_600 });
    expect(() => parseOptions([
      '--profile', 'p6', '--checkpoint-every', '2'
    ])).toThrow('1 for the primary matrix or 0 for an explicit diagnostic');
    expect(() => parseOptions(['--sim-speed', '3'])).toThrow('1, 2, 4, 8, or 12');
    expect(() => parseOptions(['--viewer', 'true'])).toThrow('on or off');
    expect(() => parseOptions(['--viewer', 'off'])).toThrow('P5 compatibility');
    expect(() => parseOptions(['--checkpoint-every', '-1'])).toThrow('0 to 1000000');
    expect(() => parseOptions([
      '--profile', 'p6', '--duration-ms', '1000'
    ])).toThrow('common polled tick target');
    expect(() => parseOptions([
      '--profile', 'p6', '--warmup-ms', '0'
    ])).toThrow('common polled tick target');
    expect(() => parseOptions([
      '--measurement-steps', '1800'
    ])).toThrow('P5 compatibility');
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

  it('does not let delayed P6 frame publication postpone the warm-up boundary', () => {
    expect(viewerWarmupReadiness('p6')).toBe('connected');
    expect(viewerWarmupReadiness('p5')).toBe('first-frame-and-stats');
  });

  it('paces authoritative-tick polling without busy polling or long overshoot sleeps', () => {
    expect(tickBoundaryPollDelayMs(1_800, 60, 1)).toBe(1_000);
    expect(tickBoundaryPollDelayMs(60, 60, 12)).toBeCloseTo(66.6666666667, 8);
    expect(tickBoundaryPollDelayMs(1, 60, 12)).toBe(25);
    expect(tickBoundaryPollDelayMs(Number.NaN, 0, Number.NaN)).toBe(25);
  });

  it('aborts a health request that exceeds its explicit wall-time bound', async () => {
    const fetchMock = vi.fn((_input: string | URL | Request, init?: RequestInit) =>
      new Promise<Response>((_resolve, reject) => {
        const signal = init?.signal;
        if (!signal) {
          reject(new Error('missing abort signal'));
          return;
        }
        const rejectAbort = () => reject(new Error('mock fetch aborted'));
        if (signal.aborted) {
          rejectAbort();
        } else {
          signal.addEventListener('abort', rejectAbort, { once: true });
        }
      })
    );
    vi.stubGlobal('fetch', fetchMock);

    await expect(
      readHealth({ port: 1 } as Parameters<typeof readHealth>[0], 5)
    ).rejects.toThrow('health request timed out');
    expect(fetchMock).toHaveBeenCalledOnce();
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

  it('keeps exact step accounting when both health samples occur inside one scheduler pump', () => {
    const delta = schedulerDelta(
      health(100, 100, 10, 100 / 60, 1),
      health(160, 160, 10, 160 / 60, 1),
      4,
      60
    );
    expect(delta).toMatchObject({
      tick: 60,
      completedSteps: 60,
      wallSeconds: 0,
      achievedMultiplier: null,
      achievedToRequestedRatio: null
    });
    expect(delta.simulatedSeconds).toBeCloseTo(1, 12);
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
