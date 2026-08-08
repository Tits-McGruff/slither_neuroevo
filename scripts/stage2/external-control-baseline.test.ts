/** Contract tests for the bounded Stage 2 P5/P6 external-control measurement runner. */

import os from 'node:os';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { resetCFGToDefaults } from '../../src/config.ts';
import {
  externalControlComposition,
  includeMemoryUsageInPeaks,
  installExternalControlScenario,
  p5Composition,
  p6Composition,
  p7Composition,
  parseOptions,
  readHealth,
  rssSlopeBytesPerMinute,
  runExternalControlBaseline,
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
      measurementSteps: null,
      evidenceEnvironment: 'development'
    });
    expect(parseOptions(['--environment', 'owner-target-vm'])).toMatchObject({
      evidenceEnvironment: 'owner-target-vm'
    });
    expect(() => parseOptions(['--environment', 'similar-machine'])).toThrow(
      'development or owner-target-vm'
    );
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
    expect(() => parseOptions([
      '--profile', 'p5', '--sample-every-ms', '1000'
    ])).toThrow('P7-only');
    expect(() => parseOptions([
      '--profile', 'p6', '--p7-test-short'
    ])).toThrow('P7-only');
    expect(parseOptions([
      '--profile', 'p7', '--p7-test-short', '--duration-ms', '3000', '--warmup-ms', '1000',
      '--reconnect-every-ms', '1000', '--manual-save-every-ms', '1000'
    ])).toMatchObject({
      profile: 'p7',
      scenario: 'P0',
      checkpointEvery: 1,
      viewer: true,
      p7TestOnlyShort: true
    });
    expect(() => parseOptions(['--profile', 'p7', '--duration-ms', '3000'])).toThrow('test-only');
    expect(() => parseOptions(['--profile', 'p7', '--scenario', 'P1'])).toThrow('P0-only');
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
    expect(p7Composition()).toEqual(p5Composition());
    expect(externalControlComposition({ profile: 'p7', viewer: true })).toEqual(p7Composition());
  });

  it('rejects a false owner-target declaration before starting the server', async () => {
    vi.spyOn(os, 'hostname').mockReturnValue('not-oxygen');
    await expect(runExternalControlBaseline(parseOptions([
      '--environment', 'owner-target-vm'
    ]))).rejects.toThrow('--environment owner-target-vm did not match');
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

  it('calculates a post-warmup RSS slope from bounded scalar samples only', () => {
    expect(rssSlopeBytesPerMinute([
      { elapsedMs: 0, rssBytes: 100 },
      { elapsedMs: 600_000, rssBytes: 200 },
      { elapsedMs: 1_200_000, rssBytes: 300 }
    ], 600_000)).toBeCloseTo(10, 12);
    expect(rssSlopeBytesPerMinute([{ elapsedMs: 600_000, rssBytes: 200 }], 600_000)).toBeNull();
  });

  it('includes a final memory observation when it exceeds the initial and periodic peaks', () => {
    const peaks = {
      peakRssBytes: 0,
      peakHeapUsedBytes: 0,
      peakExternalBytes: 0
    };
    includeMemoryUsageInPeaks(peaks, { rss: 100, heapUsed: 80, external: 30 });
    includeMemoryUsageInPeaks(peaks, { rss: 120, heapUsed: 90, external: 40 });
    includeMemoryUsageInPeaks(peaks, { rss: 150, heapUsed: 110, external: 60 });

    expect(peaks).toEqual({
      peakRssBytes: 150,
      peakHeapUsedBytes: 110,
      peakExternalBytes: 60
    });
  });

  it('reports endpoint-inclusive peaks and sampling limits from the shared P5/P6 path', async () => {
    const artifact = await runExternalControlBaseline(parseOptions([
      '--profile', 'p6', '--warmup-tick', '1', '--measurement-steps', '60',
      '--checkpoint-every', '0'
    ])) as {
      result: {
        memory: {
          before: NodeJS.MemoryUsage;
          after: NodeJS.MemoryUsage;
          peakRssBytes: number;
          peakHeapUsedBytes: number;
          peakExternalBytes: number;
          peakSampleCadenceMs: number;
          peakSampleCaveat: string;
        };
      };
    };
    const memory = artifact.result.memory;

    expect(memory.peakSampleCadenceMs).toBe(50);
    expect(memory.peakSampleCaveat).toContain('can still miss shorter spikes');
    for (const observation of [memory.before, memory.after]) {
      expect(memory.peakRssBytes).toBeGreaterThanOrEqual(observation.rss);
      expect(memory.peakHeapUsedBytes).toBeGreaterThanOrEqual(observation.heapUsed);
      expect(memory.peakExternalBytes).toBeGreaterThanOrEqual(observation.external);
    }
  }, 30_000);

  it('runs an explicit short P7 diagnostic with viewer, save, reclaim, and bounded samples', async () => {
    const capturedErrors: unknown[][] = [];
    const errorSpy = vi.spyOn(console, 'error').mockImplementation((...args: unknown[]) => {
      capturedErrors.push(args);
    });
    let artifact: {
      outcome: string;
      result: {
        samples: Array<{ memory: NodeJS.MemoryUsage }>;
        saves: unknown[];
        reconnects: Array<{
          resultSeen: boolean;
          assignmentReclaimed: boolean;
          sameSnake: boolean;
          rotatedToken: boolean;
        }>;
        sampleCountBound: number;
        fullP7SoakEligible: boolean;
        finalStorage: { snapshotCount: number; genomeRowCount: number } | null;
        controllers: { viewerFramesReceived: number; viewerStatsReceived: number };
        memoryBefore: NodeJS.MemoryUsage | null;
        memoryAfter: NodeJS.MemoryUsage;
        sampledMemoryPeaks: {
          rssBytes: number;
          heapUsedBytes: number;
          externalBytes: number;
        };
      };
    };
    try {
      artifact = await runExternalControlBaseline(parseOptions([
        '--profile', 'p7', '--p7-test-short', '--duration-ms', '3200', '--warmup-ms', '1000',
        '--sample-every-ms', '1000', '--reconnect-every-ms', '1000', '--manual-save-every-ms', '1000'
      ])) as typeof artifact;
    } finally {
      errorSpy.mockRestore();
    }
    expect(capturedErrors.every(args => (
      args[0] === '[ws.reliable_send_failed]' &&
      typeof args[1] === 'object' &&
      args[1] !== null &&
      'reason' in args[1] &&
      args[1].reason === 'socket is not open'
    ))).toBe(true);
    expect(artifact.outcome).toBe('completed');
    expect(artifact.result.fullP7SoakEligible).toBe(false);
    expect(artifact.result.samples.length).toBeLessThanOrEqual(artifact.result.sampleCountBound);
    expect(artifact.result.saves.length).toBeGreaterThan(0);
    expect(artifact.result.reconnects.length).toBeGreaterThan(0);
    expect(artifact.result.reconnects.every(reconnect => (
      reconnect.resultSeen &&
      reconnect.assignmentReclaimed &&
      reconnect.sameSnake &&
      reconnect.rotatedToken
    ))).toBe(true);
    expect(artifact.result.finalStorage?.snapshotCount).toBeGreaterThan(1);
    expect(artifact.result.finalStorage?.genomeRowCount).toBeGreaterThan(55);
    expect(artifact.result.controllers.viewerFramesReceived).toBeGreaterThan(0);
    expect(artifact.result.controllers.viewerStatsReceived).toBeGreaterThan(0);
    if (artifact.result.memoryBefore === null) {
      throw new Error('completed P7 diagnostic did not retain its initial memory observation');
    }
    const retainedMemory = [
      artifact.result.memoryBefore,
      artifact.result.memoryAfter,
      ...artifact.result.samples.map(sample => sample.memory)
    ];
    for (const observation of retainedMemory) {
      expect(artifact.result.sampledMemoryPeaks.rssBytes).toBeGreaterThanOrEqual(observation.rss);
      expect(artifact.result.sampledMemoryPeaks.heapUsedBytes).toBeGreaterThanOrEqual(
        observation.heapUsed
      );
      expect(artifact.result.sampledMemoryPeaks.externalBytes).toBeGreaterThanOrEqual(
        observation.external
      );
    }
  }, 30_000);
});
