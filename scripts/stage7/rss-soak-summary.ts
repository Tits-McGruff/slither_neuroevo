/** Exact memory-growth gates for the approved thirty-minute real-time soak. */

/** One process RSS observation on a monotonic wall clock. */
export interface RssSample {
  /** Seconds since the measured workload began. */
  wallSeconds: number;
  /** Actual resident bytes, rather than the process high-water counter. */
  rssBytes: number;
}

/** Summarize the ten-minute warm window and subsequent twenty-minute trend. */
export function summarizeRssSoak(samples: readonly RssSample[]): {
  warmSamples: number; measuredSamples: number; warmMedianBytes: number;
  finalRssBytes: number; finalAboveWarmMedianBytes: number;
  slopeMiBPerMinute: number; warmSeconds: number; measuredSeconds: number;
  meetsMemoryGate: boolean;
} {
  if (samples.length < 3 || samples.some((sample, index) =>
    !Number.isFinite(sample.wallSeconds) || sample.wallSeconds < 0 ||
    !Number.isSafeInteger(sample.rssBytes) || sample.rssBytes <= 0 ||
    (index > 0 && sample.wallSeconds <= samples[index - 1]!.wallSeconds))) {
    throw new Error('RSS soak needs positive byte counts and strictly increasing finite observation times');
  }
  const warm = samples.filter(sample => sample.wallSeconds < 600);
  const measured = samples.filter(sample => sample.wallSeconds >= 600);
  if (warm.length < 2 || measured.length < 2) throw new Error('RSS soak needs both warm and measured windows');
  const sortedWarm = warm.map(sample => sample.rssBytes).sort((left, right) => left - right);
  const middle = Math.floor(sortedWarm.length / 2);
  const warmMedianBytes = sortedWarm.length % 2 === 0
    ? (sortedWarm[middle - 1]! + sortedWarm[middle]!) / 2 : sortedWarm[middle]!;
  const meanMinutes = measured.reduce((sum, sample) => sum + sample.wallSeconds / 60, 0) / measured.length;
  const meanMiB = measured.reduce((sum, sample) => sum + sample.rssBytes / 1024 ** 2, 0) / measured.length;
  let covariance = 0;
  let timeVariance = 0;
  for (const sample of measured) {
    const deltaMinutes = sample.wallSeconds / 60 - meanMinutes;
    covariance += deltaMinutes * (sample.rssBytes / 1024 ** 2 - meanMiB);
    timeVariance += deltaMinutes ** 2;
  }
  const slopeMiBPerMinute = covariance / timeVariance;
  const final = samples.at(-1)!;
  const finalAboveWarmMedianBytes = final.rssBytes - warmMedianBytes;
  const warmSeconds = 600 - samples[0]!.wallSeconds;
  const measuredSeconds = final.wallSeconds - 600;
  return { warmSamples: warm.length, measuredSamples: measured.length, warmMedianBytes,
    finalRssBytes: final.rssBytes, finalAboveWarmMedianBytes, slopeMiBPerMinute,
    warmSeconds, measuredSeconds,
    meetsMemoryGate: warmSeconds >= 600 && measuredSeconds >= 1200 &&
      slopeMiBPerMinute <= 1 && finalAboveWarmMedianBytes <= 64 * 1024 ** 2 };
}
