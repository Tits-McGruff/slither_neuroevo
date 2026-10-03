import { describe, expect, it } from 'vitest';
import { archiveOverlap, counterWindow } from './archive-overlap-summary.ts';

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

describe('completed-step measurement clock bounds', () => {
  it('bounds every allowed counter-read instant rather than choosing a convenient endpoint', () => {
    const window = counterWindow(
      { beforeMs: 1000, afterMs: 1010, completedStep: hex(60) },
      { beforeMs: 601010, afterMs: 601020, completedStep: hex(36060) }
    );
    expect(window.deltaSteps).toBe('36000');
    expect(window.minimumWallSeconds).toBe(600);
    expect(window.maximumWallSeconds).toBe(600.02);
    for (let initial = 1000; initial <= 1010; initial++) {
      for (let final = 601010; final <= 601020; final++) {
        const actualWall = (final - initial) / 1000;
        const actualRatio = 600 / actualWall;
        expect(actualWall).toBeGreaterThanOrEqual(window.minimumWallSeconds);
        expect(actualWall).toBeLessThanOrEqual(window.maximumWallSeconds);
        expect(actualRatio).toBeGreaterThanOrEqual(window.minimumSimulatedWallRatio);
        expect(actualRatio).toBeLessThanOrEqual(window.maximumSimulatedWallRatio);
      }
    }
  });
  it('keeps a threshold-straddling interval below the acceptance cutoff', () => {
    const window = counterWindow(
      { beforeMs: 0, afterMs: 10, completedStep: hex(0) },
      { beforeMs: 612240, afterMs: 612250, completedStep: hex(36000) }
    );
    expect(window.minimumSimulatedWallRatio).toBeLessThan(0.98);
    expect(window.maximumSimulatedWallRatio).toBeGreaterThan(0.98);
  });
  it('preserves a small exact delta when absolute counters exceed Number precision', () => {
    const window = counterWindow(
      { beforeMs: 0, afterMs: 1, completedStep: 'ffffffffffffffc0' },
      { beforeMs: 1001, afterMs: 1002, completedStep: 'ffffffffffffffff' }
    );
    expect(window.deltaSteps).toBe('63');
    expect(window.minimumSimulatedWallRatio).toBe(1.05 / 1.002);
    expect(window.maximumSimulatedWallRatio).toBe(1.05);
  });
  it('retains zero progress as a measured result', () => {
    expect(counterWindow(
      { beforeMs: 0, afterMs: 1, completedStep: hex(17) },
      { beforeMs: 1001, afterMs: 1002, completedStep: hex(17) }
    )).toMatchObject({ deltaSteps: '0', minimumSimulatedWallRatio: 0, maximumSimulatedWallRatio: 0 });
  });
  it('rejects malformed, reversed, overlapping or inexact readings', () => {
    const initial = { beforeMs: 0, afterMs: 10, completedStep: hex(60) };
    const final = { beforeMs: 1010, afterMs: 1020, completedStep: hex(120) };
    for (const completedStep of ['0', 'FFFFFFFFFFFFFFFF', 'garbage', hex(59), 'ffffffffffffffff']) {
      expect(() => counterWindow(initial, { ...final, completedStep })).toThrow();
    }
    for (const value of [NaN, Infinity, -1]) {
      expect(() => counterWindow({ ...initial, beforeMs: value }, final)).toThrow();
      expect(() => counterWindow(initial, { ...final, afterMs: value })).toThrow();
    }
    expect(() => counterWindow({ ...initial, afterMs: -1 }, final)).toThrow();
    expect(() => counterWindow(initial, { ...final, afterMs: 1000 })).toThrow(/reversed/);
    for (const beforeMs of [9, 10]) expect(() => counterWindow(initial, { ...final, beforeMs })).toThrow(/overlap/);
    expect(() => counterWindow(
      { beforeMs: 0, afterMs: 0, completedStep: hex(0) },
      { beforeMs: Number.MIN_VALUE, afterMs: Number.MIN_VALUE, completedStep: hex(1) }
    )).toThrow(/duration/);
    expect(() => counterWindow(
      { beforeMs: 0, afterMs: 0, completedStep: hex(0) },
      { beforeMs: 1e-300, afterMs: 1e-300, completedStep: hex(Number.MAX_SAFE_INTEGER) }
    )).toThrow(/progress/);
  });
});
