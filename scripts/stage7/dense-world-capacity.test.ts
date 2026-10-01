import { describe, expect, it } from 'vitest';
import { frameCounts } from './dense-world-capacity.ts';

/** Encode one complete snake with no pellets using the real frame-v1 layout. */
function frame(points: number, xAt: (point: number) => number): Buffer {
  const values = new Float32Array(7 + 8 + points * 2 + 1);
  values.set([1, 1, 1, 800, 0, 0, 1]);
  values.set([1, 3, 0, xAt(0), 0, 0, 0, points], 7);
  for (let point = 0; point < points; point++) values[15 + point * 2] = xAt(point);
  return Buffer.from(values.buffer);
}

describe('dense-world capacity geometry evidence', () => {
  it('keeps a >200k off-arena tail from qualifying crowded collision capacity', () => {
    const bytes = frame(200_031, point => 9000 + point * 3);
    const counts = frameCounts(bytes, true);
    expect(counts.bodySegments).toBeGreaterThan(200_000);
    expect(counts.inArenaCollisionSegments).toBe(0);
    expect(counts.outsideArenaPoints).toBe(200_031);
    expect(frameCounts(bytes).inArenaCollisionSegments).toBeUndefined();
  });

  it('excludes head skips and requires both endpoints to be inside the circular arena', () => {
    const counts = frameCounts(frame(34, point => point === 29 ? 800 : point === 31 ? 801 : 795), true);
    expect(counts.bodySegments).toBe(33);
    expect(counts.inArenaCollisionSegments).toBe(2);
    expect(counts.outsideArenaPoints).toBe(1);
  });

  it('rejects nonfinite coordinates and truncated payloads instead of counting them', () => {
    const bytes = frame(34, point => point === 31 ? Number.NaN : 0);
    expect(() => frameCounts(bytes, true)).toThrow('nonfinite capacity body');
    expect(() => frameCounts(frame(34, () => 0).subarray(0, 100), true)).toThrow('truncated audited capacity body');
  });
});
