import { describe, expect, it } from 'vitest';
import { generationInterval, generationPublication } from './generation-window-summary.ts';

describe('generation publication clock bounds', () => {
  it('uses the last old read and first new read, including both request uncertainties', () => {
    expect(generationPublication(
      { generation: '0000000000000002', beforeMs: 100, afterMs: 110 },
      { generation: '0000000000000003', beforeMs: 600, afterMs: 620 }
    )).toEqual({ generation: '0000000000000003', beforeMs: 100, afterMs: 620 });
    expect(generationPublication(
      { generation: '0000000000000002', beforeMs: 100, afterMs: 110 },
      { generation: '0000000000000002', beforeMs: 600, afterMs: 620 }
    )).toBeUndefined();
  });
  it('contains every possible duration and prevents a polling sighting from falsely passing 62 seconds', () => {
    const previous = { generation: '0000000000000003', beforeMs: 0, afterMs: 500 };
    const current = { generation: '0000000000000004', beforeMs: 62000, afterMs: 62500 };
    const interval = generationInterval(previous, current);
    expect(interval).toEqual({ minimumSeconds: 61.5, maximumSeconds: 62.5 });
    for (let start = 0; start <= 500; start += 100) {
      for (let finish = 62000; finish <= 62500; finish += 100) {
        expect((finish - start) / 1000).toBeGreaterThanOrEqual(interval.minimumSeconds);
        expect((finish - start) / 1000).toBeLessThanOrEqual(interval.maximumSeconds);
      }
    }
    expect((current.afterMs - previous.afterMs) / 1000).toBe(62);
    expect(interval.maximumSeconds).toBeGreaterThan(62);
  });
  it('retains overlapping publication brackets without inventing a positive lower duration', () => {
    expect(generationInterval(
      { generation: '0000000000000003', beforeMs: 0, afterMs: 500 },
      { generation: '0000000000000004', beforeMs: 400, afterMs: 600 }
    )).toEqual({ minimumSeconds: 0, maximumSeconds: 0.6 });
  });
  it('requires every intervening generation and exact identities even above Number precision', () => {
    const previous = { generation: 'fffffffffffffffe', beforeMs: 10, afterMs: 11 };
    expect(generationPublication(previous,
      { generation: 'ffffffffffffffff', beforeMs: 20, afterMs: 21 })?.generation).toBe('ffffffffffffffff');
    for (const generation of ['0000000000000001', '0000000000000004']) {
      expect(() => generationPublication({ ...previous, generation: '0000000000000002' },
        { generation, beforeMs: 20, afterMs: 21 })).toThrow(/unobserved|regressed/);
    }
    expect(() => generationInterval({ ...previous, generation: '0000000000000002' },
      { generation: '0000000000000004', beforeMs: 20, afterMs: 21 })).toThrow(/unobserved/);
  });
  it('rejects malformed clocks, noncanonical counters and reversed observation ordering', () => {
    const previous = { generation: '0000000000000002', beforeMs: 10, afterMs: 11 };
    const current = { generation: '0000000000000003', beforeMs: 20, afterMs: 21 };
    for (const generation of ['2', 'FFFFFFFFFFFFFFFF', 'garbage']) {
      expect(() => generationPublication({ ...previous, generation }, current)).toThrow(/invalid/);
    }
    for (const beforeMs of [NaN, Infinity, -1, 12]) {
      expect(() => generationPublication({ ...previous, beforeMs }, current)).toThrow(/invalid/);
    }
    expect(() => generationPublication(previous, { ...current, beforeMs: 11 })).toThrow(/overlap/);
    expect(() => generationInterval(previous, { ...current, afterMs: 10, beforeMs: 9 })).toThrow(/unordered/);
  });
});
