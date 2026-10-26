/** A completion flag cannot substitute for continuous duration, bounded storage or ownership. */
import { expect, it } from 'vitest';
import { summarizeNativeSoak, type NativeSoakReceipt } from './soak-summary.ts';

/** Small valid smoke witness; intentionally insufficient for six-hour acceptance. */
function smoke(): NativeSoakReceipt {
  const storage = {completedStep: 0, snakes: 65, snakeCapacity: 128, snakeLimit: 128,
    bodyPoints: 650, bodyCapacity: 2048, bodyLimit: 4096, pellets: 10, pelletCapacity: 100, pelletLimit: 100,
    brains: 55, brainCapacity: 55, lifeSlots: 55, lifeSlotCapacity: 55, recentLifeEnds: 0, recentLifeEndCapacity: 256,
    transactionLifeEnds: 0, transactionLifeEndCapacity: 55, lives: 55, deaths: 0, respawns: 0,
    failedPlacements: 0, pendingSeconds: 0, recurrentElements: 880, weightElements: 1_377_310,
    admittedStateBytes: 1_500_000_000, placementCandidates: 0, placementGeometryChecks: 0, currentRssBytes: 1024};
  return {complete: true, gate: 'continuous-native-soak-v1', requestedSteps: 600, completedStep: 600,
    measuredSteps: 600, warmupSteps: 0, sixHourEquivalent: false, sourceSha256: 'a'.repeat(64), continuationSha256: 'b'.repeat(64),
    stepTiming: {samples: 600, buckets: [600, ...Array<number>(24).fill(0)]},
    snapshots: [storage, {...storage, completedStep: 600, recentLifeEnds: 60, lives: 115, deaths: 60, respawns: 60}],
    shadows: Array.from({length: 4}, (_, slot) => ({samples: 10, activationSequence: slot + 1})),
    capture: {observed: Array<number>(10).fill(10), drained: Array<number>(10).fill(9),
      droppedSamples: 10, droppedChunks: 1, chunks: 9, productionWatermark: 10, releasedThrough: 10,
      ownershipReconciled: true, countLimitedDroppedChunks: 1, byteLimitedDroppedChunks: 0,
      queuePeakChunks: 1, queueChunkLimit: 1, queuePeakBytes: 4096, drainEverySteps: 600}};
}

it('keeps short valid pressure smoke separate from the six-hour gate', () => {
  expect(summarizeNativeSoak(smoke())).toMatchObject({pressureObserved: true, repeatedDeaths: true, sixHourPressureSoak: false});
  const report = smoke(); report.sixHourEquivalent = true;
  expect(() => summarizeNativeSoak(report)).toThrow();
});
it('rejects partial authority, missing timings and falsely reconciled capture', () => {
  for (const mutate of [
    (report: NativeSoakReceipt) => {report.completedStep--;},
    (report: NativeSoakReceipt) => {report.stepTiming.buckets[0]!--;},
    (report: NativeSoakReceipt) => {report.capture.droppedSamples++;},
    (report: NativeSoakReceipt) => {report.capture.releasedThrough--;},
    (report: NativeSoakReceipt) => {report.shadows[3]!.samples--;}
  ]) {const report = smoke(); mutate(report); expect(() => summarizeNativeSoak(report)).toThrow();}
});
it('rejects historical record growth outside native limits or lifecycle bounds', () => {
  for (const mutate of [
    (report: NativeSoakReceipt) => {report.snapshots[1]!.snakes = 129;},
    (report: NativeSoakReceipt) => {report.snapshots[1]!.recentLifeEnds = 257;},
    (report: NativeSoakReceipt) => {report.snapshots[1]!.lifeSlots++;},
    (report: NativeSoakReceipt) => {report.snapshots[1]!.currentRssBytes = Number.NaN;}
  ]) {const report = smoke(); mutate(report); expect(() => summarizeNativeSoak(report)).toThrow();}
});

it('rejects hidden buffer growth, changing admission and impossible life accounting', () => {
  for (const mutate of [
    (report: NativeSoakReceipt) => {report.snapshots[1]!.brainCapacity++;},
    (report: NativeSoakReceipt) => {report.snapshots[1]!.recentLifeEndCapacity++;},
    (report: NativeSoakReceipt) => {report.snapshots[1]!.transactionLifeEndCapacity++;},
    (report: NativeSoakReceipt) => {report.snapshots[1]!.recurrentElements++;},
    (report: NativeSoakReceipt) => {report.snapshots[1]!.weightElements++;},
    (report: NativeSoakReceipt) => {report.snapshots[1]!.admittedStateBytes++;},
    (report: NativeSoakReceipt) => {report.snapshots[1]!.bodyLimit++;},
    (report: NativeSoakReceipt) => {report.snapshots[1]!.lives++;},
    (report: NativeSoakReceipt) => {report.snapshots[1]!.respawns = 61; report.snapshots[1]!.lives = 116;},
    (report: NativeSoakReceipt) => {report.snapshots[1]!.transactionLifeEnds = 56;},
    (report: NativeSoakReceipt) => {report.snapshots[1]!.pendingSeconds = Number.POSITIVE_INFINITY;},
    (report: NativeSoakReceipt) => {report.snapshots[1]!.failedPlacements = -1;}
  ]) {const report = smoke(); mutate(report); expect(() => summarizeNativeSoak(report)).toThrow();}
});
it('rejects a reported queue footprint or cadence beyond native capture admission', () => {
  for (const mutate of [
    (report: NativeSoakReceipt) => {report.capture.queuePeakBytes = 16 * 1024 * 1024 + 1;},
    (report: NativeSoakReceipt) => {report.capture.queueChunkLimit = 65_537;},
    (report: NativeSoakReceipt) => {report.capture.drainEverySteps = 0;},
    (report: NativeSoakReceipt) => {report.capture.drainEverySteps = 60_001;}
  ]) {const report = smoke(); mutate(report); expect(() => summarizeNativeSoak(report)).toThrow();}
});
