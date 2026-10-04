import type { RustBackgroundHealth } from '../../src/protocol/rustBackground.ts';

/** Exact bounded histogram published by the native authority for one process lifetime. */
export type StepHistogram = Pick<RustBackgroundHealth, 'stepTimingSamples' |
  'stepTimingBucketUpperMicros' | 'stepTimingBucketCounts' | 'stepTimingHistogramConsistent'>;

/** Open-ended bucket sentinel, which cannot prove any finite latency budget. */
const U64_MAX = 0xffff_ffff_ffff_ffffn;

/** Decode an unsaturated exact counter without silently rounding its rank or delta. */
function counter(value: string): bigint {
  if (typeof value !== 'string' || !/^[0-9a-f]{16}$/u.test(value)) throw new Error('invalid step histogram counter');
  const exact = BigInt(`0x${value}`);
  if (exact === U64_MAX) throw new Error('saturated step histogram counter');
  return exact;
}

/** Validate one complete published prefix independently of its native consistency flag. */
function histogram(value: StepHistogram): { samples: bigint; counts: bigint[]; ceilings: bigint[] } {
  if (value.stepTimingHistogramConsistent !== true) throw new Error('inconsistent step histogram observation');
  const layout = value.stepTimingBucketUpperMicros;
  const counts = value.stepTimingBucketCounts;
  if (!Array.isArray(layout) || !Array.isArray(counts) || layout.length < 2 || layout.length > 64 ||
      layout.length !== counts.length || layout.at(-1) !== 'ffffffffffffffff') {
    throw new Error('invalid step histogram layout');
  }
  const ceilings = layout.map((upper, index) => index === layout.length - 1 ? U64_MAX : counter(upper));
  if (ceilings[0] === 0n || ceilings.some((upper, index) => index > 0 && upper <= ceilings[index - 1]!)) {
    throw new Error('unordered step histogram layout');
  }
  const exactCounts = counts.map(counter);
  const samples = counter(value.stepTimingSamples);
  if (exactCounts.reduce((sum, count) => sum + count, 0n) !== samples) {
    throw new Error('step histogram counts differ from its published sample prefix');
  }
  return { samples, counts: exactCounts, ceilings };
}

/** Subtract two observations from the same authority; callers must check process/run identity and clock brackets. */
export function summarizeStepWindow(initial: StepHistogram, final: StepHistogram): {
  samples: string; bucketUpperMicros: string[]; bucketCounts: string[]; overflowSamples: string;
  p50UpperMs: number | null; p95UpperMs: number | null; p99UpperMs: number | null;
  meetsP0P1StepGate: boolean;
} {
  const before = histogram(initial);
  const after = histogram(final);
  if (JSON.stringify(initial.stepTimingBucketUpperMicros) !== JSON.stringify(final.stepTimingBucketUpperMicros)) {
    throw new Error('step histogram layout changed during the window');
  }
  const samples = after.samples - before.samples;
  if (samples <= 0n) throw new Error('step histogram window has no positive sample delta');
  const counts = after.counts.map((count, index) => {
    const delta = count - before.counts[index]!;
    if (delta < 0n) throw new Error('step histogram bucket regressed during the window');
    return delta;
  });
  /** Keep ranks exact; null marks a percentile in the unbounded final bucket. */
  const percentile = (percent: bigint): number | null => {
    const rank = (samples * percent + 99n) / 100n;
    let cumulative = 0n;
    for (const [index, count] of counts.entries()) {
      cumulative += count;
      if (cumulative >= rank) {
        const upper = after.ceilings[index]!;
        if (upper === U64_MAX) return null;
        if (upper > BigInt(Number.MAX_SAFE_INTEGER)) throw new Error('step histogram ceiling exceeds exact numeric range');
        return Number(upper) / 1000;
      }
    }
    throw new Error('step histogram percentile has no bucket');
  };
  const p99UpperMs = percentile(99n);
  return { samples: samples.toString(), bucketUpperMicros: after.ceilings.map(upper => upper.toString(16).padStart(16, '0')),
    bucketCounts: counts.map(count => count.toString()), overflowSamples: counts.at(-1)!.toString(),
    p50UpperMs: percentile(50n), p95UpperMs: percentile(95n), p99UpperMs,
    meetsP0P1StepGate: p99UpperMs !== null && p99UpperMs <= 16.667 };
}
