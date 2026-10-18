import { describe, expect, it } from 'vitest';
import { summarizeSensorWall, type SensorWallRead } from './sensorWallSummary.ts';

/** Exact counters for an independently specified 60-second native window. */
const hex = (value: number | bigint): string => BigInt(value).toString(16).padStart(16, '0');
/** Construct scalar observations with a known histogram and clock bracket. */
function read(at: number, steps: number, slow = false): SensorWallRead {
  return {beforeMs: at, afterMs: at + 1, health: {worldEpoch: hex(1), generation: hex(1),
    calculationWorkers: 5, lifecycle: 'running', completedStep: hex(steps), schedulerCompletedSteps: hex(steps),
    schedulerDroppedWallMicros: hex(0), schedulerOverloaded: false,
    stepTimingSamples: hex(steps), stepTimingHistogramConsistent: true,
    stepTimingBucketUpperMicros: [hex(16000), hex(20000), 'ffffffffffffffff'],
    stepTimingBucketCounts: slow ? [hex(0), hex(steps), hex(0)] : [hex(steps), hex(0), hex(0)]}};
}

describe('real-clock v4 qualification accounting', () => {
  it('uses conservative clock brackets and exact counter deltas at 1x', () => {
    const result = summarizeSensorWall([read(1000, 60), read(61002, 3660)], 60);
    expect(result.progress.deltaSteps).toBe('3600');
    expect(result.progress.minimumWallSeconds).toBeCloseTo(60.001);
    expect(result.progress.minimumSimulatedWallRatio).toBeLessThan(1);
    expect(result.meetsMeasuredGates).toBe(true);
  });
  it('keeps actual throughput separate from the existing p99 timing criterion', () => {
    const result = summarizeSensorWall([read(1000, 60, true), read(61002, 3660, true)], 60);
    expect(result.meetsRealtimeGate).toBe(true);
    expect(result.meetsExistingStepTimingGate).toBe(false);
    expect(result.meetsMeasuredGates).toBe(false);
  });
  it('records insufficient progress, debt and intermediate overload as failed measurements', () => {
    expect(summarizeSensorWall([read(1000, 60), read(61002, 3500)], 60).meetsRealtimeGate).toBe(false);
    const debt = read(61002, 3660); debt.health.schedulerDroppedWallMicros = hex(1);
    expect(summarizeSensorWall([read(1000, 60), debt], 60).meetsRealtimeGate).toBe(false);
    const middle = read(31001, 1860); middle.health.schedulerOverloaded = true;
    expect(summarizeSensorWall([read(1000, 60), middle, read(61002, 3660)], 60).meetsRealtimeGate).toBe(false);
  });
  it('rejects short, regressed, changed-authority and partial-histogram evidence', () => {
    expect(() => summarizeSensorWall([read(1000, 60), read(60000, 3600)], 59)).toThrow(/60..600/);
    const epoch = read(61002, 3660); epoch.health.worldEpoch = hex(2);
    expect(() => summarizeSensorWall([read(1000, 60), epoch], 60)).toThrow(/authority/);
    expect(() => summarizeSensorWall([read(1000, 60), read(31001, 2000), read(61002, 1900)], 60)).toThrow(/regressed/);
    const partial = read(61002, 3660); partial.health.stepTimingHistogramConsistent = false;
    expect(() => summarizeSensorWall([read(1000, 60), partial], 60)).toThrow(/inconsistent/);
    partial.beforeMs = 31001; partial.afterMs = 31002;
    expect(() => summarizeSensorWall([read(1000, 60), partial, read(61002, 4000)], 60)).toThrow(/inconsistent/);
    expect(() => summarizeSensorWall([read(1000, 60), read(1001, 61)], 60)).toThrow(/overlapping/);
  });
});
