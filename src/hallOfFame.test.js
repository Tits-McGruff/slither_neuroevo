import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { HallOfFame } from './hallOfFame.js';

describe('hallOfFame.js', () => {
  let originalStorage;
  let backing;

  beforeEach(() => {
    originalStorage = globalThis.localStorage;
    backing = new Map();
    globalThis.localStorage = {
      getItem: (key) => (backing.has(key) ? backing.get(key) : null),
      setItem: (key, value) => backing.set(key, value),
      removeItem: (key) => backing.delete(key)
    };
  });

  afterEach(() => {
    globalThis.localStorage = originalStorage;
  });

  it('adds entries sorted by fitness and trims to max', () => {
    const hof = new HallOfFame();
    hof.reset();

    hof.add({ gen: 1, fitness: 10 });
    hof.add({ gen: 2, fitness: 30 });
    hof.add({ gen: 3, fitness: 20 });

    const list = hof.getAll();
    expect(list[0].fitness).toBe(30);
    expect(list[1].fitness).toBe(20);
    expect(list[2].fitness).toBe(10);
  });

  it('loads from localStorage when available', () => {
    const seed = [{ gen: 1, fitness: 99 }];
    globalThis.localStorage.setItem('slither_neuroevo_hof', JSON.stringify(seed));

    const hof = new HallOfFame();
    const list = hof.getAll();
    expect(list.length).toBe(1);
    expect(list[0].fitness).toBe(99);
  });
});
