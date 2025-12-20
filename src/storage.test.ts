import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { Storage, savePopulation, loadPopulation } from './storage.ts';
import { Genome, buildArch } from './mlp.ts';

describe('storage.ts', () => {
  let originalStorage: Storage | undefined;
  let backing: Map<string, string>;
  const globalAny = globalThis as any;

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
    if (originalStorage === undefined) {
      delete globalAny.localStorage;
    } else {
      globalAny.localStorage = originalStorage;
    }
  });

  it('Storage save/load/remove round-trips JSON', () => {
    const payload = { a: 1, b: { c: 2 } };
    expect(Storage.save('key', payload)).toBe(true);
    expect(Storage.load('key')).toEqual(payload);
    Storage.remove('key');
    expect(Storage.load('key')).toBeNull();
  });

  it('savePopulation and loadPopulation round-trip genomes', () => {
    const settings = {
      hiddenLayers: 1,
      neurons1: 4,
      neurons2: 4,
      neurons3: 4,
      neurons4: 4,
      neurons5: 4
    };
    const arch = buildArch(settings);
    const genome = Genome.random(arch);
    genome.fitness = 42;

    savePopulation(7, [genome]);
    const loaded = loadPopulation(arch);

    expect(loaded).not.toBeNull();
    if (!loaded) return;
    expect(loaded.generation).toBe(7);
    expect(loaded.genomes.length).toBe(1);
    const first = loaded.genomes[0];
    expect(first).toBeDefined();
    if (!first) return;
    expect(first.weights.length).toBe(genome.weights.length);
  });
});
