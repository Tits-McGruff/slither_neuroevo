import { describe, expect, it } from 'vitest';
import { observedTurn, summarizeTurnResponseBounds, TurnResponseMarker, type HeadingObservation } from './turn-response-marker.ts';

/** Realistic same-snake observation with adjustable boundary, heading and receipt time. */
function sample(tick: number, direction: number, receivedAtMs: number): HeadingObservation {
  return { snakeId: 7, tick, direction, receivedAtMs };
}

describe('LAN steering response correlation', () => {
  it('ranks interrupted attempts beyond finite bounds instead of removing them from p95', () => {
    const fast = Array.from({ length: 18 }, () => ({ latencyUpperBoundMs: 30 }));
    expect(summarizeTurnResponseBounds([...fast, {}, {}], 20)).toMatchObject({
      completedTrials: 18, unknownTrials: 2, p95Ms: undefined, meetsP95Gate: false });
    expect(summarizeTurnResponseBounds([...fast, { latencyUpperBoundMs: 30 }, {}], 20)).toMatchObject({
      completedTrials: 19, unknownTrials: 1, p95Ms: 30, meetsP95Gate: true });
    expect(summarizeTurnResponseBounds(fast, 20).meetsP95Gate).toBe(false);
    expect(() => summarizeTurnResponseBounds([{ latencyUpperBoundMs: NaN }], 1)).toThrow(/invalid/u);
  });
  it('waits through queued old-direction motion and includes the return trip', () => {
    const marker = new TurnResponseMarker(sample(10, 0.3, 100), 103, 1, -1);
    expect(marker.observe(sample(11, 0.35, 110))).toBeUndefined();
    expect(marker.observe(sample(12, 0.29, 132))).toEqual({ latencyUpperBoundMs: 29, observedTick: 12 });
    expect(marker.observe(sample(13, 0.2, 150))).toBeUndefined();
  });
  it('cannot count stale ticks, a pre-send receipt or stationary motion', () => {
    const marker = new TurnResponseMarker(sample(10, 0.3, 100), 103, 1, -1);
    expect(marker.observe(sample(9, 0.1, 110))).toBeUndefined();
    expect(marker.observe(sample(11, 0.1, 102))).toBeUndefined();
    expect(marker.observe(sample(11, 0.3, 112))).toBeUndefined();
    expect(marker.observe(sample(11, 0.1, 114))).toBeUndefined();
    expect(marker.observe(sample(12, 0.2, 120))).toEqual({ latencyUpperBoundMs: 17, observedTick: 12 });
  });
  it('handles wrapped headings while rejecting ambiguous large gaps', () => {
    expect(observedTurn(3.13, -3.12)).toBe(1);
    expect(observedTurn(-3.12, 3.13)).toBe(-1);
    expect(observedTurn(0, Math.PI)).toBe(0);
    expect(observedTurn(0, 0.0001)).toBe(0);
  });
  it('rejects replacement identities and invalid correlation boundaries', () => {
    const marker = new TurnResponseMarker(sample(10, 0.3, 100), 103, 1, -1);
    expect(() => marker.observe({ ...sample(11, 0.2, 120), snakeId: 8 })).toThrow(/assignment changed/u);
    expect(() => marker.observe(sample(11, NaN, 120))).toThrow(/invalid/u);
    expect(() => new TurnResponseMarker(sample(10, 0.3, 100), 99, 1, -1)).toThrow(/pre-send/u);
    expect(() => new TurnResponseMarker(sample(10, 0.3, 100), 103, 1, 1)).toThrow(/reversal/u);
  });
});
