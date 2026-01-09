import { describe, it, expect } from 'vitest';
import { createPersistence, initDb, type PopulationSnapshotPayload } from './persistence.ts';
import type { GraphSpec } from '../src/brains/graph/schema.ts';

/** Test suite label for persistence helpers. */
const SUITE = 'persistence';

describe(SUITE, () => {
  it('creates schema and stores snapshots', () => {
    const db = initDb(':memory:');
    const persistence = createPersistence(db);

    const snapshot: PopulationSnapshotPayload = {
      generation: 3,
      archKey: 'test-arch',
      genomes: [{ archKey: 'test-arch', weights: [0.1, 0.2] }],
      cfgHash: 'abc123',
      worldSeed: 42,
      settings: {
        snakeCount: 12,
        simSpeed: 1,
        hiddenLayers: 2,
        neurons1: 16,
        neurons2: 12,
        neurons3: 8,
        neurons4: 6,
        neurons5: 4
      },
      updates: [{ path: 'baselineBots.count', value: 2 }]
    };

    const id = persistence.saveSnapshot(snapshot);
    expect(id).toBeGreaterThan(0);

    const latest = persistence.loadLatestSnapshot();
    expect(latest?.generation).toBe(3);
    expect(latest?.cfgHash).toBe('abc123');
    expect(latest?.settings?.snakeCount).toBe(12);
    expect(latest?.updates?.[0]?.path).toBe('baselineBots.count');

    const tables = db
      .prepare(`SELECT name FROM sqlite_master WHERE type='table' ORDER BY name`)
      .all() as Array<{ name: string }>;
    const names = tables.map(row => row.name);
    expect(names).toContain('hof_entries');
    expect(names).toContain('graph_presets');
    expect(names).toContain('population_snapshots');
    expect(names).toContain('players');

    const columns = db
      .prepare(`PRAGMA table_info(population_snapshots)`)
      .all() as Array<{ name: string }>;
    const columnNames = columns.map(col => col.name);
    expect(columnNames).toContain('settings_json');
    expect(columnNames).toContain('updates_json');
  });

  it('saves and loads graph presets', () => {
    const db = initDb(':memory:');
    const persistence = createPersistence(db);
    const spec: GraphSpec = {
      type: 'graph',
      nodes: [
        { id: 'input', type: 'Input', outputSize: 4 },
        { id: 'head', type: 'Dense', inputSize: 4, outputSize: 2 }
      ],
      edges: [{ from: 'input', to: 'head' }],
      outputs: [{ nodeId: 'head' }],
      outputSize: 2
    };
    const presetId = persistence.saveGraphPreset('Unit test preset', spec);
    const list = persistence.listGraphPresets(10);
    expect(list.length).toBeGreaterThan(0);
    expect(list[0]?.id).toBe(presetId);
    const loaded = persistence.loadGraphPreset(presetId);
    expect(loaded?.name).toBe('Unit test preset');
    expect(loaded?.spec).toEqual(spec);
  });

  it('deduplicates HoF entries based on gen, seed, and fitness', () => {
    const db = initDb(':memory:');
    const persistence = createPersistence(db);
    const entry = {
      gen: 5,
      seed: 123,
      fitness: 100.5,
      points: 10,
      length: 50,
      genome: { archKey: 'test', weights: [1, 2, 3] }
    };

    persistence.saveHofEntry(entry);
    persistence.saveHofEntry(entry); // Duplicate
    persistence.saveHofEntries([entry, entry]); // More duplicates in bulk

    const list = persistence.loadHofEntries(10);
    expect(list.length).toBe(1);
    expect(list[0]?.gen).toBe(5);
    expect(list[0]?.fitness).toBe(100.5);
  });
});
