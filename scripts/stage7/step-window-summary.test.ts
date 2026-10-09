import { describe, expect, it } from 'vitest';
import { summarizeStepWindow, type StepHistogram } from './step-window-summary.ts';

/** Build an exact synthetic timing prefix around the production P0/P1 threshold. */
function snapshot(fast: bigint, slow: bigint = 0n, overflow: bigint = 0n): Required<StepHistogram> {
  const hex = (value: bigint): string => value.toString(16).padStart(16, '0');
  return { stepTimingSamples: hex(fast + slow + overflow), stepTimingHistogramConsistent: true,
    stepTimingBucketUpperMicros: [hex(16_667n), hex(24_000n), 'ffffffffffffffff'],
    stepTimingBucketCounts: [hex(fast), hex(slow), hex(overflow)] };
}

describe('measured step-computation histogram window', () => {
  it('excludes old slow warm-up samples and rejects a slow window hidden by old fast samples', () => {
    expect(summarizeStepWindow(snapshot(0n, 1000n), snapshot(100n, 1000n))).toMatchObject({
      samples: '100', p99UpperMs: 16.667, meetsP0P1StepGate: true
    });
    expect(summarizeStepWindow(snapshot(10_000n), snapshot(10_000n, 100n))).toMatchObject({
      samples: '100', p99UpperMs: 24, meetsP0P1StepGate: false
    });
  });
  it('uses nearest-rank p99 at the exact one-percent threshold and preserves huge prefix deltas', () => {
    const huge = 9007199254740993n;
    expect(summarizeStepWindow(snapshot(huge), snapshot(huge + 99n, 1n))).toMatchObject({
      samples: '100', bucketCounts: ['99', '1', '0'], p99UpperMs: 16.667, meetsP0P1StepGate: true
    });
    expect(summarizeStepWindow(snapshot(huge), snapshot(huge + 98n, 2n))).toMatchObject({
      p99UpperMs: 24, meetsP0P1StepGate: false
    });
  });
  it('keeps an open-ended percentile unknown and cannot certify its finite budget', () => {
    expect(summarizeStepWindow(snapshot(0n), snapshot(0n, 0n, 100n))).toMatchObject({
      overflowSamples: '100', p50UpperMs: null, p95UpperMs: null, p99UpperMs: null, meetsP0P1StepGate: false
    });
  });
  it('rejects partial, missing, saturated or arithmetically inconsistent observations', () => {
    const cases = [
      { ...snapshot(100n), stepTimingHistogramConsistent: false },
      { ...snapshot(100n), stepTimingSamples: '0000000000000063' },
      { ...snapshot(100n), stepTimingBucketCounts: ['ffffffffffffffff', '0000000000000000', '0000000000000000'] },
      { ...snapshot(100n), stepTimingBucketCounts: ['64', '0000000000000000', '0000000000000000'] },
      { ...snapshot(100n), stepTimingBucketUpperMicros: [] },
      { ...snapshot(100n), stepTimingBucketCounts: undefined },
      { ...snapshot(100n), stepTimingBucketUpperMicros: undefined }
    ];
    for (const value of cases) expect(() => summarizeStepWindow(snapshot(0n), value)).toThrow(/histogram/);
    expect(() => summarizeStepWindow(snapshot(0n), undefined as unknown as StepHistogram)).toThrow();
  });
  it('rejects layout changes, malformed ceilings, empty deltas and individual bucket regressions', () => {
    const changed = snapshot(100n); changed.stepTimingBucketUpperMicros[0] = '0000000000003e80';
    expect(() => summarizeStepWindow(snapshot(0n), changed)).toThrow(/layout changed/);
    for (const ceiling of ['0000000000000000', '0000000000005dc0', '0x416b', 'fffffffffffffffe']) {
      const malformed = snapshot(100n); malformed.stepTimingBucketUpperMicros[0] = ceiling;
      expect(() => summarizeStepWindow(snapshot(0n), malformed)).toThrow(/layout|counter/);
    }
    expect(() => summarizeStepWindow(snapshot(100n), snapshot(100n))).toThrow(/positive sample delta/);
    expect(() => summarizeStepWindow(snapshot(100n, 10n), snapshot(99n, 100n))).toThrow(/regressed/);
  });
});
