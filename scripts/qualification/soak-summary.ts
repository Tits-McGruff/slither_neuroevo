/** Validate continuous native soak receipts without reconstructing simulation state. */
import assert from 'node:assert/strict';

/** Only fixed scalar resource witnesses are accepted from the native report. */
interface Storage {
  completedStep: number;
  snakes: number; snakeCapacity: number; snakeLimit: number;
  bodyPoints: number; bodyCapacity: number; bodyLimit: number;
  pellets: number; pelletCapacity: number; pelletLimit: number;
  brains: number; brainCapacity: number;
  lifeSlots: number; lifeSlotCapacity: number; recentLifeEnds: number; recentLifeEndCapacity: number;
  transactionLifeEnds: number; transactionLifeEndCapacity: number;
  lives: number; deaths: number; respawns: number; failedPlacements: number; pendingSeconds: number;
  recurrentElements: number; weightElements: number; admittedStateBytes: number;
  placementCandidates: number; placementGeometryChecks: number; currentRssBytes: number;
}
/** Independent report checks bind duration, diagnostic ownership and resource snapshots. */
export interface NativeSoakReceipt {
  complete: boolean; gate: string; requestedSteps: number; completedStep: number;
  measuredSteps: number; warmupSteps: number; sixHourEquivalent: boolean;
  sourceSha256: string; continuationSha256: string;
  stepTiming: {samples: number; buckets: number[]};
  snapshots: Storage[];
  shadows: Array<{samples: number; activationSequence: number}>;
  capture: {observed: number[]; drained: number[]; droppedSamples: number; droppedChunks: number;
    chunks: number; productionWatermark: number; releasedThrough: number; ownershipReconciled: boolean;
    countLimitedDroppedChunks: number; byteLimitedDroppedChunks: number;
    queuePeakChunks: number; queueChunkLimit: number; queuePeakBytes: number; drainEverySteps: number};
}

/** Preserve failed or partial coverage; a short smoke can never establish six-hour acceptance. */
export function summarizeNativeSoak(report: NativeSoakReceipt) {
  assert.equal(report.gate, 'continuous-native-soak-v1'); assert.equal(report.complete, true);
  assert.match(report.sourceSha256, /^[0-9a-f]{64}$/u);
  assert.match(report.continuationSha256, /^[0-9a-f]{64}$/u);
  /** Reject rounded, negative or nonfinite scalar counters before arithmetic. */
  const count = (value: number): number => {assert(Number.isSafeInteger(value) && value >= 0); return value;};
  for (const value of [report.requestedSteps, report.completedStep, report.measuredSteps, report.warmupSteps]) count(value);
  assert(report.measuredSteps > 0 && report.requestedSteps <= 1_296_000);
  assert.equal(report.completedStep, report.requestedSteps);
  assert.equal(report.requestedSteps, report.measuredSteps + report.warmupSteps);
  assert.equal(report.sixHourEquivalent, report.measuredSteps === 1_296_000);
  assert.equal(report.stepTiming.samples, report.measuredSteps); assert.equal(report.stepTiming.buckets.length, 25);
  assert.equal(report.stepTiming.buckets.reduce((sum, value) => sum + count(value), 0), report.measuredSteps);
  const capture = report.capture; assert.equal(capture.observed.length, 10); assert.equal(capture.drained.length, 10);
  for (const [slot, value] of capture.observed.entries()) {
    assert(count(value) <= report.measuredSteps && count(capture.drained[slot]!) <= value);
  }
  const observed = capture.observed.reduce((sum, value) => sum + count(value), 0);
  const drained = capture.drained.reduce((sum, value) => sum + count(value), 0);
  for (const value of [capture.droppedSamples, capture.droppedChunks, capture.chunks, capture.productionWatermark,
    capture.releasedThrough, capture.countLimitedDroppedChunks, capture.byteLimitedDroppedChunks,
    capture.queuePeakChunks, capture.queueChunkLimit, capture.queuePeakBytes, capture.drainEverySteps]) count(value);
  assert(observed > 0); assert.equal(capture.ownershipReconciled, true);
  assert.equal(observed, drained + capture.droppedSamples);
  assert.equal(capture.productionWatermark, capture.chunks + capture.droppedChunks);
  assert(capture.productionWatermark <= observed);
  assert.equal(capture.releasedThrough, capture.productionWatermark);
  assert.equal(capture.countLimitedDroppedChunks + capture.byteLimitedDroppedChunks, capture.droppedChunks);
  assert(capture.queuePeakChunks <= capture.queueChunkLimit && capture.queueChunkLimit > 0 && capture.queueChunkLimit <= 65_536);
  assert(capture.queuePeakBytes <= 16 * 1024 * 1024);
  assert(capture.drainEverySteps > 0 && capture.drainEverySteps <= 60_000);
  assert.equal(report.shadows.length, 4); assert(capture.observed[0]! > 0);
  for (const [index, shadow] of report.shadows.entries()) {
    assert.equal(shadow.activationSequence, index + 1); assert.equal(shadow.samples, capture.observed[0]);
  }
  assert(report.snapshots.length >= 2 && report.snapshots.length <= 362);
  assert.equal(report.snapshots[0]!.completedStep, 0);
  assert.equal(report.snapshots.at(-1)!.completedStep, report.completedStep);
  let previous: Storage | undefined;
  for (const snapshot of report.snapshots) {
    for (const value of [snapshot.completedStep, snapshot.snakes, snapshot.snakeCapacity, snapshot.snakeLimit,
      snapshot.bodyPoints, snapshot.bodyCapacity, snapshot.bodyLimit, snapshot.pellets, snapshot.pelletCapacity,
      snapshot.pelletLimit, snapshot.brains, snapshot.brainCapacity, snapshot.lifeSlots, snapshot.lifeSlotCapacity,
      snapshot.recentLifeEnds, snapshot.recentLifeEndCapacity, snapshot.transactionLifeEnds, snapshot.transactionLifeEndCapacity,
      snapshot.lives, snapshot.deaths, snapshot.respawns, snapshot.failedPlacements,
      snapshot.recurrentElements, snapshot.weightElements, snapshot.admittedStateBytes,
      snapshot.placementCandidates, snapshot.placementGeometryChecks, snapshot.currentRssBytes]) count(value);
    assert(snapshot.lifeSlots > 0 && snapshot.lifeSlots <= 300 && snapshot.lifeSlotCapacity >= snapshot.lifeSlots);
    assert(snapshot.recentLifeEnds <= snapshot.recentLifeEndCapacity && snapshot.recentLifeEndCapacity <= 256);
    assert(snapshot.transactionLifeEnds <= snapshot.transactionLifeEndCapacity && snapshot.transactionLifeEnds <= snapshot.lifeSlots);
    assert.equal(snapshot.brains, snapshot.lifeSlots); assert(snapshot.brainCapacity >= snapshot.brains);
    assert(snapshot.currentRssBytes > 0 && snapshot.admittedStateBytes > 0 && snapshot.weightElements > 0);
    assert(Number.isFinite(snapshot.pendingSeconds) && snapshot.pendingSeconds >= 0);
    assert.equal(snapshot.lives, snapshot.lifeSlots + snapshot.respawns);
    assert(snapshot.respawns <= snapshot.deaths && snapshot.deaths <= snapshot.lives);
    for (const [used, capacity, limit] of [[snapshot.snakes, snapshot.snakeCapacity, snapshot.snakeLimit],
      [snapshot.bodyPoints, snapshot.bodyCapacity, snapshot.bodyLimit], [snapshot.pellets, snapshot.pelletCapacity, snapshot.pelletLimit]] as const) {
      assert(used <= capacity && used <= limit);
    }
    if (previous) {
      assert(snapshot.completedStep > previous.completedStep && snapshot.completedStep - previous.completedStep <= 3600);
      assert(snapshot.deaths >= previous.deaths && snapshot.respawns >= previous.respawns &&
        snapshot.failedPlacements >= previous.failedPlacements && snapshot.pendingSeconds >= previous.pendingSeconds);
      // This harness never changes generation, population, graph or retained brains. Those
      // buffers and admitted bounds must stay fixed even while physical lives are replaced.
      for (const key of ['lifeSlots', 'lifeSlotCapacity', 'brains', 'brainCapacity', 'recentLifeEndCapacity',
        'transactionLifeEndCapacity', 'recurrentElements', 'weightElements', 'admittedStateBytes',
        'snakeLimit', 'bodyLimit', 'pelletLimit'] as const) assert.equal(snapshot[key], previous[key], key);
    }
    previous = snapshot;
  }
  const last = report.snapshots.at(-1)!;
  return {measuredSimulatedHours: report.measuredSteps / 216_000, observed, drained,
    pressureObserved: capture.droppedChunks > 0, repeatedDeaths: last.deaths > last.lifeSlots,
    peakSampledResidentBytes: Math.max(...report.snapshots.map(snapshot => snapshot.currentRssBytes)),
    sixHourPressureSoak: report.sixHourEquivalent && capture.droppedChunks > 0 && last.deaths > last.lifeSlots};
}
