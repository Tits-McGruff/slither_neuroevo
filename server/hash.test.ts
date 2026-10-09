import { describe, expect, it } from 'vitest';
import { canonicalizeConfig, hashConfig } from './hash.ts';

describe('canonical configuration hash', () => {
  it('is invariant to recursive object insertion order', () => {
    const first = {
      z: 1,
      nested: { beta: [3, { y: 2, x: 1 }], alpha: true }
    };
    const second = {
      nested: { alpha: true, beta: [3, { x: 1, y: 2 }] },
      z: 1
    };

    expect(canonicalizeConfig(first)).toBe(canonicalizeConfig(second));
    expect(hashConfig(first)).toBe(hashConfig(second));
    expect(hashConfig(first)).toMatch(/^v1-[0-9a-f]{8}$/);
  });

  it('keeps array order content-significant', () => {
    expect(hashConfig({ values: [1, 2] })).not.toBe(hashConfig({ values: [2, 1] }));
  });
});
