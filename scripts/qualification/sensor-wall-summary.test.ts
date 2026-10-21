import { describe, expect, it } from 'vitest';
import { summarizeSensorWall, type SensorWallRead, type SensorGenerationBarrier } from './sensorWallSummary.ts';

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


/** One independently specified native finish and durable successor, including time spent at the barrier. */
function generationWindow(): {reads: SensorWallRead[]; barriers: SensorGenerationBarrier[]} {
  const reads = [read(1000, 60), read(20000, 1199), read(22000, 1201), read(61002, 3600)];
  for (const entry of reads.slice(2)) {entry.health.worldEpoch = hex(3); entry.health.generation = hex(2);}
  return {reads, barriers: [{beforeReadIndex: 1, afterReadIndex: 2, effectiveStep: hex(1200),
    checkpoint: {runId: 'qualified-run', boundaryKind: 'generation', generation: hex(2), completedStep: hex(1200), logicalRootSha256: 'a'.repeat(64)},
    terminal: {generation: hex(1), completedStep: hex(1200), reason: 'manual', population: 55, actualSeconds: 20}}]};
}

describe('durable generation wall-window accounting', () => {
  it('includes checkpoint wall time and requires an exact native/SQLite join for every authority change', () => {
    const {reads, barriers} = generationWindow();
    expect(() => summarizeSensorWall(reads, 60)).toThrow(/authority/);
    const result = summarizeSensorWall(reads, 60, {runId: 'qualified-run', barriers});
    expect(result.progress.deltaSteps).toBe('3540');
    expect(result.progress.minimumWallSeconds).toBeCloseTo(60.001);
    expect(result.meetsRealtimeGate).toBe(true);
    reads[3]!.health.completedStep = hex(3300); reads[3]!.health.schedulerCompletedSteps = hex(3300);
    reads[3]!.health.stepTimingSamples = hex(3300); reads[3]!.health.stepTimingBucketCounts = [hex(3300), hex(0), hex(0)];
    expect(summarizeSensorWall(reads, 60, {runId: 'qualified-run', barriers}).meetsRealtimeGate).toBe(false);
  });
  it('joins multiple ordered generations without subtracting either durability pause from wall time', () => {
    const {reads, barriers} = generationWindow();
    reads.splice(3, 1, read(40000, 2279), read(42000, 2281), read(61002, 3480));
    reads[3]!.health.worldEpoch = hex(3); reads[3]!.health.generation = hex(2);
    for (const entry of reads.slice(4)) {entry.health.worldEpoch = hex(5); entry.health.generation = hex(3);}
    barriers.push({beforeReadIndex: 3, afterReadIndex: 4, effectiveStep: hex(2280),
      checkpoint: {runId: 'qualified-run', boundaryKind: 'generation', generation: hex(3), completedStep: hex(2280), logicalRootSha256: 'b'.repeat(64)},
      terminal: {generation: hex(2), completedStep: hex(2280), reason: 'manual', population: 55, actualSeconds: 18}});
    const result = summarizeSensorWall(reads, 60, {runId: 'qualified-run', barriers});
    expect(result.progress.deltaSteps).toBe('3420');
    expect(result.meetsRealtimeGate).toBe(false);
    expect(() => summarizeSensorWall(reads, 60, {runId: 'qualified-run', barriers: [...barriers].reverse()})).toThrow(/unordered/);
  });
  it('rejects changed runs, forged chronology, missing sidecars and falsely claimed barriers', () => {
    const mutations: Array<(barrier: SensorGenerationBarrier) => void> = [
      barrier => {barrier.checkpoint.runId = 'other-run';},
      barrier => {barrier.checkpoint.generation = hex(3);},
      barrier => {barrier.checkpoint.completedStep = hex(1199);},
      barrier => {barrier.checkpoint.logicalRootSha256 = 'invalid';},
      barrier => {barrier.terminal.generation = hex(2);},
      barrier => {barrier.terminal.completedStep = hex(1201);},
      barrier => {barrier.terminal.reason = 'early';},
      barrier => {barrier.terminal.actualSeconds = Number.NaN;},
      barrier => {barrier.terminal.population = 54;},
      barrier => {barrier.afterReadIndex = 3;}
    ];
    for (const mutate of mutations) {
      const {reads, barriers} = generationWindow(); mutate(barriers[0]!);
      expect(() => summarizeSensorWall(reads, 60, {runId: 'qualified-run', barriers})).toThrow(/generation/);
    }
    const {reads, barriers} = generationWindow();
    expect(() => summarizeSensorWall(reads, 60, {runId: 'qualified-run', barriers: []})).toThrow(/proof/);
    expect(() => summarizeSensorWall(reads, 60, {runId: 'qualified-run', barriers: [...barriers, ...barriers]})).toThrow(/unordered/);
    reads[2]!.health.worldEpoch = hex(1);
    expect(() => summarizeSensorWall(reads, 60, {runId: 'qualified-run', barriers})).toThrow(/chronology/);
  });
  it('rejects unreported changes after a valid barrier and regressed cumulative counters', () => {
    const {reads, barriers} = generationWindow();
    reads[3]!.health.generation = hex(3); reads[3]!.health.worldEpoch = hex(5);
    expect(() => summarizeSensorWall(reads, 60, {runId: 'qualified-run', barriers})).toThrow(/authority/);
    reads[3]!.health.generation = hex(2); reads[3]!.health.worldEpoch = hex(3);
    reads[3]!.health.schedulerCompletedSteps = hex(1000);
    expect(() => summarizeSensorWall(reads, 60, {runId: 'qualified-run', barriers})).toThrow(/regressed/);
  });
});
