/** Validate coarse published-observation counts without sampling or interpreting vectors. */
import assert from 'node:assert/strict';
import {
  OBSERVATION_CONSUMERS, OBSERVATION_COUNT_KEYS,
  type ObservationConsumerCounts, type ObservationQualitySnapshot,
} from '../../src/protocol/observationQuality.ts';

/** Test-only fixed diagnostic endpoint; normal production handles do not expose it. */
export interface ObservationQualityNative {
  /** Read one coherent fixed-size scalar prefix from the authority metrics cache. */
  observationQualityForTest(): string;
}

/** Node's monotonic bracket identifies when an independent diagnostic prefix was read. */
export interface ObservationQualityRead {
  /** Lower end of the cache observation. */
  beforeMs: number;
  /** Upper end of the cache observation. */
  afterMs: number;
  /** Validated exact native prefix. */
  snapshot: ObservationQualitySnapshot;
}

/** Reject unknown fields, lossy numeric counters and saturated lifetime counters. */
function exactKeys(value: object, keys: readonly string[]): void {
  assert.deepEqual(Object.keys(value).sort(), [...keys].sort(), 'observation quality shape');
}

/** Read exact, unsaturated u64 counters; a saturated diagnostic cannot qualify a workload. */
function counter(value: unknown): bigint {
  assert(typeof value === 'string' && /^[0-9a-f]{16}$/u.test(value) && value !== 'f'.repeat(16),
    'invalid or saturated observation counter');
  return BigInt('0x' + value);
}

/** Validate internal set/count relationships in one coherent source-class record. */
function validateCounts(row: ObservationConsumerCounts): void {
  const counts = Object.fromEntries(OBSERVATION_COUNT_KEYS.map(key => [key, counter(row[key])])) as
    Record<typeof OBSERVATION_COUNT_KEYS[number], bigint>;
  assert(counts.v4Observations <= counts.observations);
  const incomplete = [counts.rayIncompleteObservations, counts.bodyCappedObservations, counts.pelletCappedObservations];
  assert(incomplete.every(value => value <= counts.querySaturatedObservations));
  assert(counts.querySaturatedObservations <= counts.v4Observations);
  assert(counts.querySaturatedObservations <= incomplete.reduce((sum, value) => sum + value, 0n));
  assert(counts.v4Observations > 0n || counts.rayChargedUnits === 0n);
}

/** Validate the whole bounded scalar payload before a caller can calculate a fraction. */
export function parseObservationQuality(text: string): ObservationQualitySnapshot {
  assert(Buffer.byteLength(text, 'utf8') <= 4096, 'observation quality byte bound');
  const data = JSON.parse(text) as ObservationQualitySnapshot;
  assert(data && typeof data === 'object');
  exactKeys(data, ['schemaVersion', 'runId', 'ordinaryPublishedSteps', 'worldEpoch', 'generation', 'completedStep', 'consumers']);
  assert.equal(data.schemaVersion, 1);
  assert(data.runId === null || (typeof data.runId === 'string' && data.runId.length > 0 && Buffer.byteLength(data.runId, 'utf8') <= 256));
  for (const key of ['ordinaryPublishedSteps', 'worldEpoch', 'generation', 'completedStep'] as const) counter(data[key]);
  assert(Array.isArray(data.consumers) && data.consumers.length === 3);
  for (const [index, row] of data.consumers.entries()) {
    assert(row && typeof row === 'object');
    exactKeys(row, ['consumerClass', ...OBSERVATION_COUNT_KEYS]);
    assert.equal(row.consumerClass, OBSERVATION_CONSUMERS[index]);
    validateCounts(row);
  }
  const steps = counter(data.ordinaryPublishedSteps);
  assert((data.runId === null) === (steps === 0n));
  assert(steps > 0n || data.consumers.every(row => counter(row.observations) === 0n));
  return data;
}

/** Copy only the cached diagnostic payload; this never advances or probes the world. */
export function readObservationQuality(native: ObservationQualityNative): ObservationQualityRead {
  const beforeMs = performance.now();
  const snapshot = parseObservationQuality(native.observationQualityForTest());
  return {beforeMs, afterMs: performance.now(), snapshot};
}

/** Subtract coherent prefixes and apply the strict one-percent test with integer arithmetic. */
export function summarizeObservationQuality(before: ObservationQualityRead, after: ObservationQualityRead) {
  assert(Number.isFinite(before.beforeMs) && Number.isFinite(before.afterMs) &&
    Number.isFinite(after.beforeMs) && Number.isFinite(after.afterMs) &&
    before.beforeMs <= before.afterMs && before.afterMs <= after.beforeMs && after.beforeMs <= after.afterMs);
  const initial = parseObservationQuality(JSON.stringify(before.snapshot));
  const final = parseObservationQuality(JSON.stringify(after.snapshot));
  assert(initial.runId === null || initial.runId === final.runId, 'observation quality run changed');
  for (const key of ['ordinaryPublishedSteps', 'worldEpoch', 'generation', 'completedStep'] as const) {
    assert(counter(final[key]) >= counter(initial[key]), 'observation quality prefix regressed');
  }
  const steps = counter(final.ordinaryPublishedSteps) - counter(initial.ordinaryPublishedSteps);
  const rows = final.consumers.map((row, index) => {
    const counts = Object.fromEntries(OBSERVATION_COUNT_KEYS.map(key => {
      const value = counter(row[key]) - counter(initial.consumers[index]![key]);
      assert(value >= 0n, 'observation quality counter regressed');
      return [key, value];
    })) as Record<typeof OBSERVATION_COUNT_KEYS[number], bigint>;
    const exact = Object.fromEntries(OBSERVATION_COUNT_KEYS.map(key => [key, counts[key].toString(16).padStart(16, '0')]));
    validateCounts({...exact, consumerClass: row.consumerClass} as ObservationConsumerCounts);
    return {consumerClass: row.consumerClass,
      counts: Object.fromEntries(OBSERVATION_COUNT_KEYS.map(key => [key, counts[key].toString()])),
      rayIncompleteFraction: counts.v4Observations ? Number(counts.rayIncompleteObservations) / Number(counts.v4Observations) : null,
      querySaturatedFraction: counts.v4Observations ? Number(counts.querySaturatedObservations) / Number(counts.v4Observations) : null,
      rayBelowOnePercent: counts.v4Observations ? counts.rayIncompleteObservations * 100n < counts.v4Observations : null,
      queryBelowOnePercent: counts.v4Observations ? counts.querySaturatedObservations * 100n < counts.v4Observations : null};
  });
  assert(steps > 0n || rows.every(row => row.counts['observations'] === '0'), 'observations advanced without a publication');
  const v4 = rows.reduce((sum, row) => sum + BigInt(row.counts['v4Observations']!), 0n);
  const rays = rows.reduce((sum, row) => sum + BigInt(row.counts['rayIncompleteObservations']!), 0n);
  const saturated = rows.reduce((sum, row) => sum + BigInt(row.counts['querySaturatedObservations']!), 0n);
  return {ordinaryPublishedSteps: steps.toString(), consumers: rows,
    allConsumerV4Observations: v4.toString(),
    allConsumerRayIncompleteFraction: v4 ? Number(rays) / Number(v4) : null,
    allConsumerQuerySaturatedFraction: v4 ? Number(saturated) / Number(v4) : null,
    meetsOrdinaryRayGate: steps > 0n && v4 > 0n && rays * 100n < v4 && rows.every(row => row.rayBelowOnePercent !== false),
    meetsOrdinaryQueryGate: steps > 0n && v4 > 0n && saturated * 100n < v4 && rows.every(row => row.queryBelowOnePercent !== false)};
}
