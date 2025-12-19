import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { HallOfFame } from './hallOfFame.ts';

describe('hallOfFame.ts', () => {
  let originalStorage: Storage | undefined;
  let backing: Map<string, string>;
  const globalAny = globalThis as unknown as { localStorage?: Storage };

  beforeEach(() => {
    originalStorage = globalAny.localStorage;
    backing = new Map();
    globalAny.localStorage = {
      getItem: (key: string) => (backing.has(key) ? backing.get(key)! : null),
      setItem: (key: string, value: string) => backing.set(key, value),
      removeItem: (key: string) => backing.delete(key),
      clear: () => backing.clear(),
      key: (index: number) => Array.from(backing.keys())[index] ?? null,
      length: 0
    } as Storage;
  });

  afterEach(() => {
    globalAny.localStorage = originalStorage;
  });

  it('adds entries sorted by fitness and trims to max', () => {
    const hof = new HallOfFame();
    hof.reset();

    const makeEntry = (gen: number, fitness: number) => ({
      gen,
      fitness,
      seed: gen,
      points: 0,
      length: 0,
      genome: { archKey: 'test', weights: [] }
    });

    hof.add(makeEntry(1, 10));
    hof.add(makeEntry(2, 30));
    hof.add(makeEntry(3, 20));

    const list = hof.getAll();
    expect(list[0].fitness).toBe(30);
    expect(list[1].fitness).toBe(20);
    expect(list[2].fitness).toBe(10);
  });

  it('loads from localStorage when available', () => {
    const seed = [{
      gen: 1,
      fitness: 99,
      seed: 1,
      points: 0,
      length: 0,
      genome: { archKey: 'test', weights: [] }
    }];
    globalThis.localStorage.setItem('slither_neuroevo_hof', JSON.stringify(seed));

    const hof = new HallOfFame();
    const list = hof.getAll();
    expect(list.length).toBe(1);
    expect(list[0].fitness).toBe(99);
  });
});
