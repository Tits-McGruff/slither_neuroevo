import { describe, expect, it } from 'vitest';
import type { RustBackgroundDisplay, RustBackgroundHealth } from '../../src/protocol/rustBackground.ts';
import { ExperimentalRuntimeTelemetry } from './runtimeTelemetry.ts';

/** Encode a small exact counter in the native health representation. */
function hex(value: number): string {
  return BigInt(value).toString(16).padStart(16, '0');
}

/** Construct one complete native health sample for telemetry projection. */
function health(completedStep: number): RustBackgroundHealth {
  return {
    lifecycle: 'running', loopState: 'ready', worldEpoch: hex(1), generation: hex(1),
    completedStep: hex(completedStep), generationCheckpointPublished: false,
    generationPersistenceAcknowledged: false, pendingExternalDeliveries: hex(0),
    schedulerCompletedSteps: hex(completedStep), processedCommands: hex(3),
    stepTimingSamples: hex(4), stepTimingTotalMicros: hex(1_000),
    stepTimingMaxMicros: hex(600), stepTimingP95Micros: hex(750),
    stepTimingP99Micros: hex(1_000)
  };
}

/** One scalar display observation; no frame bytes are needed by telemetry. */
const DISPLAY: RustBackgroundDisplay = {
  sequence: hex(1), worldEpoch: hex(1), completedStep: hex(70), generation: hex(1),
  generationTime: 1, alivePopulation: 50, baselineBotsAlive: 0, baselineBotsTotal: 0,
  totalSnakes: 50, aliveSnakes: 50, pellets: 200, frameByteLength: 12_345
};

describe('experimental runtime telemetry', () => {
  it('projects exact native timings and bounded interface distributions without game state', () => {
    const telemetry = new ExperimentalRuntimeTelemetry(health(10), 1 / 60);
    try {
      telemetry.observeDisplay(DISPLAY);
      telemetry.observeDisplay({ ...DISPLAY, frameByteLength: 10_000 });
      telemetry.observeCheckpointBarrier(12);
      telemetry.observeCheckpointBarrier(80);
      telemetry.observeAction('player', 0.75);
      telemetry.observeAction('player', 9);
      telemetry.observeAction('reinforcementLearning', 4);
      telemetry.observeControllerLifecycle('player', 'freshAssignment', 33);
      telemetry.observeControllerLifecycle('reinforcementLearning', 'reclaim', 8);
      telemetry.observeControllerDisconnect('reinforcementLearning');

      const snapshot = telemetry.snapshot(health(70));
      expect(snapshot.authoritativeSteps).toBe(60);
      expect(snapshot.simulatedSeconds).toBe(1);
      expect(snapshot.simulatedWallRatio).toBeGreaterThan(0);
      expect(snapshot.step).toEqual({ samples: 4, meanMs: 0.25, p95Ms: 0.75,
        p99Ms: 1, maxMs: 0.6, percentilesAreUpperBounds: true });
      expect(snapshot.frame).toEqual({ latestBytes: 10_000, maximumObservedBytes: 12_345 });
      expect(snapshot.checkpointBarrier).toEqual({ samples: 2, meanMs: 46, p95Ms: 125, maxMs: 80 });
      expect(snapshot.playerAction).toEqual({ samples: 2, meanMs: 4.875, p95Ms: 16, maxMs: 9 });
      expect(snapshot.trainerAction).toEqual({ samples: 1, meanMs: 4, p95Ms: 4, maxMs: 4 });
      expect(snapshot.controllerLifecycle).toEqual({ samples: 2, meanMs: 20.5, p95Ms: 64, maxMs: 33 });
      expect(snapshot.controllerActivity).toEqual({
        player: { freshAssignments: 1, successfulReclaims: 0, appliedActions: 2, appliedDisconnects: 0 },
        trainer: { freshAssignments: 0, successfulReclaims: 1, appliedActions: 1, appliedDisconnects: 1 }
      });
      expect(snapshot.process.rssBytes).toBeGreaterThan(0);
      expect(snapshot.process.eventLoopDelayP95Ms).toBeGreaterThanOrEqual(0);
    } finally {
      telemetry.close();
    }
  });
});
