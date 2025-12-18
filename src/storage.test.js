import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { Storage, savePopulation, loadPopulation } from './storage.js';
import { Genome, buildArch } from './mlp.js';

describe('storage.js', () => {
  let originalStorage;
  let backing;

  beforeEach(() => {
    originalStorage = globalThis.localStorage;
    backing = new Map();
    globalThis.localStorage = {
      getItem: (key) => (backing.has(key) ? backing.get(key) : null),
      setItem: (key, value) => backing.set(key, value),
      removeItem: (key) => backing.delete(key),
      clear: () => backing.clear()
    };
  });

  afterEach(() => {
    globalThis.localStorage = originalStorage;
  });

  it('Storage save/load/remove round-trips JSON', () => {
    const payload = { a: 1, b: { c: 2 } };
    expect(Storage.save('key', payload)).toBe(true);
    expect(Storage.load('key')).toEqual(payload);
    Storage.remove('key');
    expect(Storage.load('key')).toBeNull();
  });

  it('savePopulation and loadPopulation round-trip genomes', () => {
    const settings = { hiddenLayers: 1, neurons1: 4 };
    const arch = buildArch(settings);
    const genome = Genome.random(arch);
    genome.fitness = 42;

    savePopulation(7, [genome]);
    const loaded = loadPopulation(arch);

    expect(loaded).not.toBeNull();
    expect(loaded.generation).toBe(7);
    expect(loaded.genomes.length).toBe(1);
    expect(loaded.genomes[0].weights.length).toBe(genome.weights.length);
  });
});
