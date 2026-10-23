import { describe, expect, it } from 'vitest';
import { OBSERVATION_CONSUMERS, OBSERVATION_COUNT_KEYS, type ObservationQualitySnapshot } from '../../src/protocol/observationQuality.ts';
import { parseObservationQuality, summarizeObservationQuality, type ObservationQualityRead } from './observationQuality.ts';

/** Exact integer fixture; no conversion through lossy floating point. */
const hex = (n: bigint | number): string => BigInt(n).toString(16).padStart(16, '0');

/** Build a coherent ordinary prefix with independent neural/baseline/external counts. */
function snapshot(steps: number, counts: readonly [number, number, number]): ObservationQualitySnapshot {
  return {schemaVersion: 1, runId: steps ? 'run' : null, ordinaryPublishedSteps: hex(steps),
    worldEpoch: hex(steps ? 1 : 0), generation: hex(steps ? 1 : 0), completedStep: hex(steps),
    consumers: OBSERVATION_CONSUMERS.map((consumerClass, index) => ({
      consumerClass, observations: hex(counts[index]!), v4Observations: hex(counts[index]!),
      rayIncompleteObservations: hex(0), bodyCappedObservations: hex(0), pelletCappedObservations: hex(0),
      querySaturatedObservations: hex(0), rayChargedUnits: hex(counts[index]! * 8),
    }))};
}

/** Bracket one exact supplied prefix without reading a native owner. */
function read(at: number, value: ObservationQualitySnapshot): ObservationQualityRead {
  return {beforeMs: at, afterMs: at + 1, snapshot: value};
}

describe('ordinary published observation quality', () => {
  it('never treats zero or legacy-only observations as a quality pass', () => {
    const empty = snapshot(0, [0, 0, 0]);
    expect(summarizeObservationQuality(read(0, empty), read(2, empty)).meetsOrdinaryRayGate).toBe(false);
    const legacy = snapshot(1, [10, 0, 0]);
    legacy.consumers[0]!.v4Observations = hex(0); legacy.consumers[0]!.rayChargedUnits = hex(0);
    expect(summarizeObservationQuality(read(0, empty), read(2, legacy)).allConsumerRayIncompleteFraction).toBeNull();
  });

  it('does not let a large neural denominator hide a failing teacher subgroup or an exact one-percent rate', () => {
    const before = snapshot(0, [0, 0, 0]);
    const after = snapshot(100, [10_000, 100, 0]);
    after.consumers[1]!.rayIncompleteObservations = hex(1);
    after.consumers[1]!.querySaturatedObservations = hex(1);
    const result = summarizeObservationQuality(read(0, before), read(2, after));
    expect(result.allConsumerRayIncompleteFraction).toBeLessThan(0.01);
    expect(result.consumers[1]!.rayIncompleteFraction).toBe(0.01);
    expect(result.meetsOrdinaryRayGate).toBe(false);
    after.consumers[1]!.v4Observations = hex(101); after.consumers[1]!.observations = hex(101);
    expect(summarizeObservationQuality(read(0, before), read(2, after)).meetsOrdinaryRayGate).toBe(true);
  });

  it('counts overlapping cap reasons once and preserves exact integers beyond Number precision', () => {
    const before = snapshot(1, [0, 0, 0]);
    const after = snapshot(2, [0, 0, 0]);
    const count = 2n ** 53n + 1n;
    after.consumers[2]!.observations = hex(count); after.consumers[2]!.v4Observations = hex(count);
    after.consumers[2]!.rayIncompleteObservations = hex(1);
    after.consumers[2]!.bodyCappedObservations = hex(1);
    after.consumers[2]!.querySaturatedObservations = hex(1);
    const result = summarizeObservationQuality(read(0, before), read(2, after));
    expect(result.allConsumerV4Observations).toBe(count.toString());
    expect(result.consumers[2]!.counts['querySaturatedObservations']).toBe('1');
    expect(result.meetsOrdinaryQueryGate).toBe(true);
  });

  it('rejects malformed, saturated, inconsistent and reordered source counters', () => {
    for (const mutate of [
      (v: ObservationQualitySnapshot) => {v.consumers.reverse();},
      (v: ObservationQualitySnapshot) => {v.consumers[0]!.rayIncompleteObservations = hex(2);},
      (v: ObservationQualitySnapshot) => {v.consumers[0]!.v4Observations = hex(2);},
      (v: ObservationQualitySnapshot) => {v.consumers[0]!.rayChargedUnits = 'f'.repeat(16);},
      (v: ObservationQualitySnapshot) => {v.consumers[0]!.querySaturatedObservations = hex(1);},
    ]) {
      const value = snapshot(1, [1, 0, 0]); mutate(value);
      expect(() => parseObservationQuality(JSON.stringify(value))).toThrow();
    }
    const extra = {...snapshot(0, [0, 0, 0]), observations: []};
    expect(() => parseObservationQuality(JSON.stringify(extra))).toThrow();
    expect(() => parseObservationQuality(' '.repeat(4097))).toThrow();
  });

  it('rejects mixed runs, regressed prefixes and observations without an ordinary publication', () => {
    const initial = snapshot(2, [10, 5, 1]);
    const final = snapshot(3, [11, 6, 2]);
    final.runId = 'another';
    expect(() => summarizeObservationQuality(read(0, initial), read(2, final))).toThrow(/run changed/);
    final.runId = initial.runId;
    final.consumers[0]!.observations = hex(9); final.consumers[0]!.v4Observations = hex(9);
    expect(() => summarizeObservationQuality(read(0, initial), read(2, final))).toThrow(/regressed/);
    const unchangedStep = snapshot(2, [11, 6, 2]);
    expect(() => summarizeObservationQuality(read(0, initial), read(2, unchangedStep))).toThrow(/without a publication/);
    expect(OBSERVATION_COUNT_KEYS).toHaveLength(7);
  });
});
