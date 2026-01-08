import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import {
  Storage,
  savePopulation,
  loadPopulation,
  loadBaselineBotSettings,
  saveBaselineBotSettings
} from './storage.ts';
import { Genome, buildArch } from './mlp.ts';

/** Test suite label for storage helpers. */
const SUITE = 'storage.ts';

describe(SUITE, () => {
  /** Stored localStorage reference for cleanup. */
  let originalStorage: Storage | undefined;
  /** Backing map for the localStorage stub. */
  let backing: Map<string, string>;
  /** Global shim for swapping localStorage in tests. */
  const globalAny = globalThis as typeof globalThis & { localStorage?: Storage };

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

  it('saveBaselineBotSettings and loadBaselineBotSettings round-trip values', () => {
    const settings = { count: 3, seed: 42, randomizeSeedPerGen: true };
    expect(saveBaselineBotSettings(settings)).toBe(true);
    expect(loadBaselineBotSettings()).toEqual(settings);
  });

  it('loadBaselineBotSettings clamps negative values and clears invalid payloads', () => {
    const key = 'slither_neuroevo_baseline_bot_settings';
    backing.set(key, JSON.stringify({
      version: 1,
      count: -4,
      seed: -9,
      randomizeSeedPerGen: false
    }));
    expect(loadBaselineBotSettings()).toEqual({ count: 0, seed: 0, randomizeSeedPerGen: false });
    backing.set(key, JSON.stringify({ version: 0 }));
    expect(loadBaselineBotSettings()).toBeNull();
    expect(backing.has(key)).toBe(false);
  });
});
