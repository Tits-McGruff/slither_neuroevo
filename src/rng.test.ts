import { describe, expect, it } from 'vitest';
import { StatefulRng, deriveSeed } from './rng.ts';

/** Phase 2 versioned random-stream contract suite. */
const SUITE = 'versioned authoritative RNG';

describe(SUITE, () => {
  it('matches the xorshift32 v1 known sequence', () => {
    const rng = new StatefulRng(1);
    const states = Array.from({ length: 6 }, () => {
      rng.next();
      return rng.exportState().stateHex;
    });

    expect(states).toEqual([
      '0x00042021',
      '0x04080601',
      '0x9dcca8c5',
      '0x1255994f',
      '0x8ef917d1',
      '0x2c6f5bd0'
    ]);
  });

  it('restores uniform and bounded-operation continuation exactly', () => {
    const original = new StatefulRng(0x12345678);
    for (let draw = 0; draw < 17; draw++) original.next();
    const serialized = original.exportState();
    const restored = StatefulRng.fromState(JSON.parse(JSON.stringify(serialized)));

    const expected = Array.from({ length: 32 }, () => ({
      uniform: original.next(),
      float: original.float(-3.5, 8.25),
      integer: original.int(97)
    }));
    const actual = Array.from({ length: 32 }, () => ({
      uniform: restored.next(),
      float: restored.float(-3.5, 8.25),
      integer: restored.int(97)
    }));

    expect(actual).toEqual(expected);
    expect(restored.exportState()).toEqual(original.exportState());
  });

  it('round-trips a cached Gaussian sample losslessly', () => {
    const original = new StatefulRng(0xdecafbad);
    original.gaussian();
    const serialized = original.exportState();
    expect(serialized.gaussianSpareValid).toBe(true);
    expect(serialized.gaussianSpareHex).toMatch(/^0x[0-9a-f]{16}$/u);

    const restored = StatefulRng.fromState(JSON.parse(JSON.stringify(serialized)));
    expect(restored.gaussian()).toBe(original.gaussian());
    expect(restored.next()).toBe(original.next());
    expect(restored.exportState()).toEqual(original.exportState());
  });

  it('derives labeled streams directly from the run seed', () => {
    const seed = 0xabcdef01;
    const worldSeed = deriveSeed(seed, 'world');
    const evolutionSeed = deriveSeed(seed, 'evolution');
    const busyWorld = new StatefulRng(worldSeed);
    for (let draw = 0; draw < 100; draw++) busyWorld.next();

    expect(deriveSeed(seed, 'evolution')).toBe(evolutionSeed);
    expect(evolutionSeed).not.toBe(worldSeed);
    expect(deriveSeed(seed, 'baseline:0')).not.toBe(deriveSeed(seed, 'baseline:1'));
  });
});
