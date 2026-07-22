import { describe, expect, it } from 'vitest';
import { DEFAULT_CONFIG } from './config.ts';
import { startServer } from './index.ts';
import type { InferenceModeRecord } from './inferenceMode.ts';
import type { Logger } from './logger.ts';
import type { SimulationFaultStatus } from './simServer.ts';
import type { SchedulerDiagnostics } from '../src/sim/SimCore.ts';

/** Test suite label for native/diagnostic inference-mode integration. */
const SUITE = 'Phase 3 inference-mode diagnostics';

/** Minimal shape returned by the health endpoint. */
interface HealthResponse {
  /** Health indicator. */
  ok: boolean;
  /** Current server tick. */
  tick: number;
  /** Connected client count. */
  clients: number;
  /** Current inference-mode record. */
  inferenceMode: InferenceModeRecord;
  /** Current fixed-step scheduling measurements. */
  scheduler: SchedulerDiagnostics;
  /** Current authoritative simulation fault state. */
  fault: SimulationFaultStatus;
}

/** Captured structured logger entry. */
interface CapturedLog {
  /** Logger module label. */
  module: string;
  /** Logger message payload. */
  message: string;
}

/**
 * Create a logger that records informational output for assertions.
 * @param entries - Destination array for captured entries.
 * @returns Logger implementation used by the server.
 */
function createCapturingLogger(entries: CapturedLog[]): Logger {
  return {
    debug: () => {},
    info: (module, message) => entries.push({ module, message }),
    warn: () => {},
    error: () => {}
  };
}

describe(SUITE, () => {
  it('reports explicit serial JS diagnostic mode without loading native', async () => {
    const logs: CapturedLog[] = [];
    const server = await startServer({
      ...DEFAULT_CONFIG,
      port: 0,
      dbPath: ':memory:',
      logLevel: 'error',
      seed: 424242,
      mtEnabled: false,
      inferenceBackend: 'js'
    }, createCapturingLogger(logs));

    try {
      const response = await fetch(`http://127.0.0.1:${server.port}/health`);
      expect(response.status).toBe(200);
      const health = await response.json() as HealthResponse;
      expect(health.ok).toBe(true);
      expect(health.tick).toBeGreaterThanOrEqual(0);
      expect(health.clients).toBe(0);
      expect(health.inferenceMode).toMatchObject({
        requestedBackend: 'js',
        activeBackend: 'js',
        requestedMt: false,
        activeWorkerCount: 0,
        poolEpoch: null,
        weightEpoch: null,
        seed: 424242,
        nativeAddonBuildIdentifier: null
      });
      expect(health.inferenceMode.graphKey.length).toBeGreaterThan(0);
      expect(health.inferenceMode.parameterCount).toBeGreaterThan(0);
      expect(health.inferenceMode.nativeAddonStatus).toBe('unavailable');
      expect(health.scheduler).toMatchObject({
        requestedMultiplier: 1,
        droppedSimulationSeconds: 0,
        maxStepsPerPump: 120
      });
      expect(health.scheduler.achievedMultiplier).toBeGreaterThanOrEqual(0);
      expect(health.fault).toEqual({ faulted: false, reason: null, tick: null });

      const modeLog = logs.find(entry => entry.module === 'inference-mode');
      expect(modeLog).toBeDefined();
      expect(JSON.parse(modeLog?.message ?? 'null')).toEqual(health.inferenceMode);
    } finally {
      await server.close();
    }
  });

  it('loads and reports native for a normal single-threaded server', async () => {
    const logs: CapturedLog[] = [];
    const server = await startServer({
      ...DEFAULT_CONFIG,
      port: 0,
      dbPath: ':memory:',
      logLevel: 'error',
      seed: 424242,
      mtEnabled: false,
      inferenceBackend: 'native'
    }, createCapturingLogger(logs));

    try {
      const response = await fetch(`http://127.0.0.1:${server.port}/health`);
      expect(response.status).toBe(200);
      const health = await response.json() as HealthResponse;
      expect(health.inferenceMode).toMatchObject({
        requestedBackend: 'native',
        activeBackend: 'native',
        requestedMt: false,
        activeWorkerCount: 0,
        nativeAddonStatus: 'ready'
      });
      expect(health.inferenceMode.nativeAddonBuildIdentifier).toMatch(
        /^slither_native\/0\.1\.0\+[0-9a-f]{12}\.[0-9a-f]{16}$/u
      );
      const modeLog = logs.find(entry => entry.module === 'inference-mode');
      expect(modeLog).toBeDefined();
      expect(JSON.parse(modeLog?.message ?? 'null')).toEqual(health.inferenceMode);
    } finally {
      await server.close();
    }
  });
});
