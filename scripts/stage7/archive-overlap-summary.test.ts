import { describe, expect, it } from 'vitest';
import { archiveOverlap } from './archive-overlap-summary.ts';

/** Encode a bounded native monotonic offset without a rounded numeric identity. */
function hex(micros: number): string { return BigInt(micros).toString(16).padStart(16, '0'); }

describe('archive/checkpoint clock bounds', () => {
  it('does not claim overlap when one permitted native origin misses the barrier', () => {
    expect(archiveOverlap({ beforeMs: 100, afterMs: 130 },
      { startedMicros: hex(10_000), finishedMicros: hex(20_000) },
      { startedMs: 125, finishedMs: 140 })).toEqual({ guaranteedMs: 0, possibleMs: 15, originUncertaintyMs: 30 });
  });
  it('reports the duration present for every possible origin, including a contained phase', () => {
    expect(archiveOverlap({ beforeMs: 100, afterMs: 102 },
      { startedMicros: hex(10_000), finishedMicros: hex(40_000) },
      { startedMs: 115, finishedMs: 135 })).toMatchObject({ guaranteedMs: 20 });
    expect(archiveOverlap({ beforeMs: 100, afterMs: 102 },
      { startedMicros: hex(10_000), finishedMicros: hex(40_000) },
      { startedMs: 90, finishedMs: 180 })).toMatchObject({ guaranteedMs: 28 });
  });
  it('keeps touching or disjoint intervals unproven', () => {
    for (const start of [120, 121, 300]) expect(archiveOverlap({ beforeMs: 100, afterMs: 100 },
      { startedMicros: hex(10_000), finishedMicros: hex(20_000) },
      { startedMs: start, finishedMs: start + 10 }).guaranteedMs).toBe(0);
  });
  it('rejects open, reversed, malformed or inexact clocks rather than guessing a finish', () => {
    expect(() => archiveOverlap({ beforeMs: 10, afterMs: 9 },
      { startedMicros: hex(0), finishedMicros: hex(1) }, { startedMs: 0, finishedMs: 1 })).toThrow(/reversed/);
    for (const finish of [undefined, 'garbage', 'ffffffffffffffff', hex(1)]) {
      expect(() => archiveOverlap({ beforeMs: 10, afterMs: 11 },
        { startedMicros: hex(2), finishedMicros: finish }, { startedMs: 0, finishedMs: 1 })).toThrow();
    }
    expect(() => archiveOverlap({ beforeMs: NaN, afterMs: 11 },
      { startedMicros: hex(0), finishedMicros: hex(1) }, { startedMs: 0, finishedMs: 1 })).toThrow(/invalid/);
  });
});
