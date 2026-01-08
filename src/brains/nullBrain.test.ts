import { describe, it, expect } from 'vitest';
import { CFG } from '../config.ts';
import { NullBrain } from './nullBrain.ts';

/** Test suite label for NullBrain behavior. */
const SUITE = 'NullBrain';

describe(SUITE, () => {
  it('NullBrain returns stable zero buffer', () => {
    const brain = new NullBrain();
    const input = new Float32Array(CFG.brain.inSize);
    const outA = brain.forward(input);
    const outB = brain.forward(input);

    expect(outA).toBe(outB);
    expect(outA.length).toBe(CFG.brain.outSize);
    expect(Array.from(outA).every(value => value === 0)).toBe(true);
  });
});
