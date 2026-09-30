import { describe, expect, it } from 'vitest';
import { summarizeRssSoak } from './rss-soak-summary.ts';

/** Thirty-minute observations with an independently known linear trend. */
function trend(slopeMiBPerMinute: number) {
  return Array.from({ length: 61 }, (_, index) => ({ wallSeconds: index * 30,
    rssBytes: Math.round((200 + slopeMiBPerMinute * index / 2) * 1024 ** 2) }));
}

describe('approved RSS soak gate', () => {
  it('accepts a stable plateau and the exact allowed growth rate', () => {
    expect(summarizeRssSoak(trend(0))).toMatchObject({ meetsMemoryGate: true,
      slopeMiBPerMinute: 0, finalAboveWarmMedianBytes: 0, measuredSeconds: 1200 });
    expect(summarizeRssSoak(trend(1))).toMatchObject({ meetsMemoryGate: true, slopeMiBPerMinute: 1 });
  });
  it('rejects excessive growth even when the final plateau allowance would pass', () => {
    const result = summarizeRssSoak(trend(1.2));
    expect(result.finalAboveWarmMedianBytes).toBeLessThan(64 * 1024 ** 2);
    expect(result.slopeMiBPerMinute).toBeCloseTo(1.2);
    expect(result.meetsMemoryGate).toBe(false);
  });
  it('rejects a large post-warm allocation even when later RSS is perfectly flat', () => {
    const samples = trend(0).map(sample => ({ ...sample,
      rssBytes: sample.rssBytes + (sample.wallSeconds >= 600 ? 65 * 1024 ** 2 : 0) }));
    expect(summarizeRssSoak(samples)).toMatchObject({ slopeMiBPerMinute: 0, meetsMemoryGate: false });
  });
  it('requires thirty real minutes and rejects invalid or reordered observations', () => {
    expect(summarizeRssSoak(trend(0).slice(0, -1)).meetsMemoryGate).toBe(false);
    expect(summarizeRssSoak(trend(0).map(sample => ({ ...sample,
      wallSeconds: sample.wallSeconds + 1 }))).meetsMemoryGate).toBe(false);
    expect(() => summarizeRssSoak(trend(0).reverse())).toThrow(/increasing/);
    expect(() => summarizeRssSoak([{ wallSeconds: 0, rssBytes: NaN }])).toThrow();
  });
});
