import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { HallOfFame } from './hallOfFame.ts';
import type { HallOfFameEntry } from './protocol/messages.ts';

/** Compact authoritative entry without browser-side neural weights. */
function managedEntry(entryId: string, gen: number): HallOfFameEntry {
  return { entryId, gen, fitness: gen * 10, seed: 42, points: 3, length: 4 };
}

describe('hallOfFame.ts', () => {
  let originalStorage: Storage | undefined;
  let backing: Map<string, string>;
  /** Global shim for localStorage swapping in tests. */
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
    vi.unstubAllGlobals();
    if (originalStorage === undefined) {
      delete globalAny.localStorage;
    } else {
      globalAny.localStorage = originalStorage;
    }
  });

  it('clears former-run entries and ignores an older server response after replacement', async () => {
    const registry = new HallOfFame();
    const oldEntry = managedEntry('0000000000000001', 1);
    const newEntry = managedEntry('0000000000000002', 7);
    await registry.replace([oldEntry]);
    const first = Promise.withResolvers<Response>();
    const second = Promise.withResolvers<Response>();
    const fetchMock = vi.fn().mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    vi.stubGlobal('fetch', fetchMock);
    const oldLoad = registry.loadFromServer('http://server', true);
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));
    const newLoad = registry.loadFromServer('http://server', true);
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2));
    expect(await registry.getAll()).toEqual([]);
    second.resolve(new Response(JSON.stringify({ hof: [newEntry] })));
    expect(await newLoad).toBe(true);
    first.resolve(new Response(JSON.stringify({ hof: [oldEntry] })));
    expect(await oldLoad).toBe(false);
    expect(await registry.getAll()).toEqual([newEntry]);
  });

  it('keeps former-run entries out of the view when the replacement refresh fails', async () => {
    const registry = new HallOfFame();
    await registry.replace([managedEntry('0000000000000001', 1)]);
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response('', { status: 503 })));
    expect(await registry.loadFromServer('http://server', true)).toBe(false);
    expect(await registry.getAll()).toEqual([]);
  });

  it('preserves a run-change clear when a generation refresh overtakes initial storage loading', async () => {
    const oldEntry = managedEntry('0000000000000001', 1);
    localStorage.setItem('slither_neuroevo_hof', JSON.stringify([oldEntry]));
    const registry = new HallOfFame();
    const response = Promise.withResolvers<Response>();
    const fetchMock = vi.fn().mockReturnValue(response.promise);
    vi.stubGlobal('fetch', fetchMock);
    const replacement = registry.loadFromServer('http://server', true);
    const generationRefresh = registry.loadFromServer('http://server', false);
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));
    expect(await registry.getAll()).toEqual([]);
    response.resolve(new Response(JSON.stringify({ hof: [] })));
    expect(await replacement).toBe(false);
    expect(await generationRefresh).toBe(true);
  });

  it('adds entries sorted by fitness and trims to max', async () => {
    const hof = new HallOfFame();
    hof.reset();

    /**
     * Builds a Hall of Fame entry with a specific fitness.
     * @param gen - Generation number for the entry.
     * @param fitness - Fitness score to assign.
     * @returns Hall of Fame entry object.
     */
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

    const list = await hof.getAll();
    expect(list.length).toBe(3);
    expect(list[0]).toBeDefined();
    expect(list[0]?.fitness).toBe(30);
    expect(list[1]?.fitness).toBe(20);
    expect(list[2]?.fitness).toBe(10);
  });

  it('loads from localStorage when available', async () => {
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
    await hof.load();
    const list = await hof.getAll();
    expect(list[0]?.fitness).toBe(99);
  });

  it('preserves entries added during async load initialization', async () => {
    // Seed storage
    const seed = [{
      gen: 1,
      fitness: 10,
      seed: 1,
      points: 0,
      length: 0,
      genome: { archKey: 'test', weights: [] }
    }];
    globalThis.localStorage.setItem('slither_neuroevo_hof', JSON.stringify(seed));

    const hof = new HallOfFame();
    const newEntry = {
      gen: 2,
      fitness: 20,
      seed: 2,
      points: 5,
      length: 10,
      genome: { archKey: 'test', weights: [] }
    };

    // Call add() immediately without awaiting constructor side-effect (though it doesn't return anything)
    // In our new implementation, add() awaits initPromise internally.
    await hof.add(newEntry);

    const list = await hof.getAll();
    expect(list.length).toBe(2);
    expect(list[0]?.fitness).toBe(20); // Sorted descending
    expect(list[1]?.fitness).toBe(10);
  });
});
