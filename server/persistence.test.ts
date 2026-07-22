import zlib from 'node:zlib';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { CFG, resetCFGToDefaults } from '../src/config.ts';
import type { GraphSpec } from '../src/brains/graph/schema.ts';
import { World, type GenerationBoundaryState } from '../src/world.ts';
import { buildGenerationCheckpoint } from './checkpoint.ts';
import {
  checksumWeights,
  createPersistence,
  encodeWeightsLittleEndian,
  initDb,
  type Persistence,
  type PopulationCheckpoint
} from './persistence.ts';

/** Test suite label for bounded current-format persistence. */
const SUITE = 'Phase 7 bounded persistence';

/** Database type returned by the production initializer. */
type TestDb = ReturnType<typeof initDb>;

/** Captured exact-boundary fixture used by persistence contract tests. */
interface CheckpointFixture {
  /** Constructed World whose typed population backs the lazy checkpoint. */
  world: World;
  /** Exact run-start boundary captured before construction draws. */
  boundary: GenerationBoundaryState;
  /** Current-format checkpoint ready for synchronous consumption. */
  checkpoint: PopulationCheckpoint;
}

/**
 * Install a compact valid graph while retaining the active sensor contract.
 * @param withHidden - Whether to insert a hidden Dense node.
 * @returns Installed graph definition.
 */
function installTestGraph(withHidden = false): GraphSpec {
  const inputSize = CFG.brain.inSize;
  const nodes: GraphSpec['nodes'] = withHidden
    ? [
        { id: 'input', type: 'Input', outputSize: inputSize },
        { id: 'hidden', type: 'Dense', inputSize, outputSize: 3 },
        { id: 'head', type: 'Dense', inputSize: 3, outputSize: 2 }
      ]
    : [
        { id: 'input', type: 'Input', outputSize: inputSize },
        { id: 'head', type: 'Dense', inputSize, outputSize: 2 }
      ];
  const spec: GraphSpec = {
    type: 'graph',
    nodes,
    edges: withHidden
      ? [{ from: 'input', to: 'hidden' }, { from: 'hidden', to: 'head' }]
      : [{ from: 'input', to: 'head' }],
    outputs: [{ nodeId: 'head' }],
    outputSize: 2
  };
  CFG.brain.graphSpec = spec;
  return spec;
}

/**
 * Build a real World and capture its exact run-start checkpoint.
 * @param populationCount - Dense population size.
 * @param withHidden - Whether to use the alternate graph architecture.
 * @returns World, boundary, and lazy typed checkpoint.
 */
function createCheckpointFixture(
  populationCount = 3,
  withHidden = false
): CheckpointFixture {
  installTestGraph(withHidden);
  let boundary: GenerationBoundaryState | null = null;
  let checkpoint: PopulationCheckpoint | null = null;
  const world = new World(
    {
      snakeCount: populationCount,
      simSpeed: 1,
      hiddenLayers: 1,
      neurons1: withHidden ? 3 : 2,
      neurons2: 2,
      neurons3: 2,
      neurons4: 2,
      neurons5: 2
    },
    {
      seed: 0x12345678,
      runId: withHidden ? 'phase7-hidden' : 'phase7-direct',
      onGenerationBoundary: (captured, candidate) => {
        boundary = captured;
        checkpoint = buildGenerationCheckpoint(candidate, captured, 7);
      }
    }
  );
  if (!boundary || !checkpoint) throw new Error('run-start boundary was not captured');
  return { world, boundary, checkpoint };
}

/**
 * Persist a fresh fixture into an in-memory database.
 * @param populationCount - Dense population size.
 * @returns Open DB, persistence API, fixture, and inserted snapshot id.
 */
function saveFixture(populationCount = 3): {
  db: TestDb;
  persistence: Persistence;
  fixture: CheckpointFixture;
  id: number;
} {
  const db = initDb(':memory:');
  const persistence = createPersistence(db);
  const fixture = createCheckpointFixture(populationCount);
  const id = persistence.saveCheckpoint(fixture.checkpoint);
  return { db, persistence, fixture, id };
}

/**
 * Encode legacy length-prefixed JSON genomes using the historical gzip framing.
 * @param genomes - JSON genome records.
 * @returns Historical combined gzip BLOB.
 */
function encodeLegacyGenomes(genomes: unknown[]): Buffer {
  const records: Buffer[] = [];
  for (const genome of genomes) {
    const json = Buffer.from(JSON.stringify(genome), 'utf8');
    const prefix = Buffer.alloc(4);
    prefix.writeUInt32LE(json.byteLength, 0);
    records.push(prefix, json);
  }
  return zlib.gzipSync(Buffer.concat(records));
}

beforeEach(() => {
  resetCFGToDefaults();
  CFG.baselineBots.count = 0;
  CFG.pelletCountTarget = 0;
  CFG.pelletSpawnPerSecond = 0;
});

afterEach(() => {
  resetCFGToDefaults();
  vi.restoreAllMocks();
});

describe(SUITE, () => {
  it('round-trips typed Float32 genomes and metadata without a legacy population blob', () => {
    const { db, persistence, fixture, id } = saveFixture();
    fixture.world.population[0]!.weights[0] = Math.fround(0.125);
    fixture.world.population[1]!.weights[0] = Math.fround(-0.75);

    const secondId = persistence.saveCheckpoint(fixture.checkpoint);
    const loaded = persistence.loadResumeSnapshot(secondId);
    expect(loaded?.compatibility).toBe('current');
    if (!loaded || loaded.compatibility !== 'current') throw new Error('current snapshot missing');
    expect(loaded.metadata.boundaryKind).toBe('run-start');
    expect(loaded.metadata.runId).toBe('phase7-direct');
    expect(loaded.metadata.simulationStep).toBe(0);
    expect(loaded.genomes).toHaveLength(3);
    expect(Array.from(loaded.genomes[0]!.weights)).toEqual(
      Array.from(fixture.world.population[0]!.weights)
    );
    expect(Array.from(loaded.genomes[1]!.weights)).toEqual(
      Array.from(fixture.world.population[1]!.weights)
    );
    const parent = db.prepare(
      'SELECT format_version, genomes_blob FROM population_snapshots WHERE id = ?'
    ).get(secondId) as { format_version: number; genomes_blob: Buffer | null };
    expect(parent.format_version).toBe(2);
    expect(parent.genomes_blob).toBeNull();
    expect(id).toBeLessThan(secondId);
    db.close();
  });

  it('stores and selects checkpoints with different graph architecture metadata', () => {
    const db = initDb(':memory:');
    const persistence = createPersistence(db);
    const direct = createCheckpointFixture(2, false);
    const directId = persistence.saveCheckpoint(direct.checkpoint);
    const hidden = createCheckpointFixture(2, true);
    const hiddenId = persistence.saveCheckpoint(hidden.checkpoint);

    const loadedDirect = persistence.loadResumeSnapshot(directId);
    const loadedHidden = persistence.loadResumeSnapshot(hiddenId);
    expect(loadedDirect?.compatibility).toBe('current');
    expect(loadedHidden?.compatibility).toBe('current');
    if (
      !loadedDirect || loadedDirect.compatibility !== 'current' ||
      !loadedHidden || loadedHidden.compatibility !== 'current'
    ) {
      throw new Error('architecture snapshots failed to load');
    }
    expect(loadedDirect.metadata.archKey).not.toBe(loadedHidden.metadata.archKey);
    expect(loadedDirect.genomes[0]!.weights.length).not.toBe(
      loadedHidden.genomes[0]!.weights.length
    );
    db.close();
  });

  it('encodes Float32 BLOB bytes explicitly in little-endian order', () => {
    const bytes = encodeWeightsLittleEndian(Float32Array.of(1, -2.5, 0.125));
    expect(bytes.toString('hex')).toBe('0000803f000020c00000003e');
    expect(bytes.readFloatLE(0)).toBe(1);
    expect(bytes.readFloatLE(4)).toBe(-2.5);
  });

  it.each([
    ['checksum', (db: TestDb, id: number) => {
      db.prepare(
        'UPDATE snapshot_genomes SET weights_checksum = ? WHERE snapshot_id = ? AND slot = 0'
      ).run('0'.repeat(64), id);
    }, 'checksum mismatch'],
    ['byte length', (db: TestDb, id: number) => {
      const bytes = Buffer.from([0, 0, 0]);
      db.prepare(
        'UPDATE snapshot_genomes SET weights_blob = ?, weights_checksum = ? WHERE snapshot_id = ? AND slot = 0'
      ).run(bytes, checksumWeights(bytes), id);
    }, 'byte length'],
    ['finite value', (db: TestDb, id: number) => {
      const row = db.prepare(
        'SELECT weights_blob FROM snapshot_genomes WHERE snapshot_id = ? AND slot = 0'
      ).get(id) as { weights_blob: Buffer };
      const bytes = Buffer.from(row.weights_blob);
      bytes.writeFloatLE(Number.NaN, 0);
      db.prepare(
        'UPDATE snapshot_genomes SET weights_blob = ?, weights_checksum = ? WHERE snapshot_id = ? AND slot = 0'
      ).run(bytes, checksumWeights(bytes), id);
    }, 'not finite'],
    ['slot continuity', (db: TestDb, id: number) => {
      db.prepare(
        'UPDATE snapshot_genomes SET slot = 99 WHERE snapshot_id = ? AND slot = 0'
      ).run(id);
    }, 'not dense'],
    ['brain type', (db: TestDb, id: number) => {
      db.prepare(
        'UPDATE snapshot_genomes SET brain_type = ? WHERE snapshot_id = ? AND slot = 0'
      ).run('unsupported', id);
    }, 'unsupported brain type'],
    ['weight count', (db: TestDb, id: number) => {
      db.prepare(
        'UPDATE snapshot_genomes SET weight_count = weight_count - 1 WHERE snapshot_id = ? AND slot = 0'
      ).run(id);
    }, 'does not match graph']
  ])('reports %s corruption with the snapshot and genome context', (_label, mutate, reason) => {
    const { db, persistence, id } = saveFixture();
    mutate(db, id);
    expect(() => persistence.loadResumeSnapshot(id)).toThrow(
      expect.objectContaining({ message: expect.stringContaining(`snapshot ${id}`) })
    );
    expect(() => persistence.loadResumeSnapshot(id)).toThrow(reason);
    db.close();
  });

  it('rejects corrupt parent metadata and declared population mismatches', () => {
    const first = saveFixture();
    first.db.prepare(
      'UPDATE population_snapshots SET payload_json = ? WHERE id = ?'
    ).run('{}', first.id);
    expect(() => first.persistence.loadResumeSnapshot(first.id)).toThrow('format version');
    first.db.close();

    const second = saveFixture();
    second.db.prepare(
      'UPDATE population_snapshots SET population_count = population_count + 1 WHERE id = ?'
    ).run(second.id);
    expect(() => second.persistence.loadResumeSnapshot(second.id)).toThrow(
      'parent population count does not match metadata'
    );
    second.db.close();
  });

  it('rejects corrupt RNG versions/state and allocator continuations before bootstrap', () => {
    const rngFixture = saveFixture();
    const rngRow = rngFixture.db.prepare(
      'SELECT payload_json FROM population_snapshots WHERE id = ?'
    ).get(rngFixture.id) as { payload_json: string };
    const rngMetadata = JSON.parse(rngRow.payload_json) as {
      rng: { world: { version: number } };
    };
    rngMetadata.rng.world.version = 999;
    rngFixture.db.prepare(
      'UPDATE population_snapshots SET payload_json = ? WHERE id = ?'
    ).run(JSON.stringify(rngMetadata), rngFixture.id);
    expect(() => rngFixture.persistence.loadResumeSnapshot(rngFixture.id)).toThrow(
      'world RNG state is invalid'
    );
    rngFixture.db.close();

    const allocatorFixture = saveFixture();
    const allocatorRow = allocatorFixture.db.prepare(
      'SELECT payload_json FROM population_snapshots WHERE id = ?'
    ).get(allocatorFixture.id) as { payload_json: string };
    const allocatorMetadata = JSON.parse(allocatorRow.payload_json) as {
      allocators: { nextExternalSnakeId: number };
    };
    allocatorMetadata.allocators.nextExternalSnakeId = 0;
    allocatorFixture.db.prepare(
      'UPDATE population_snapshots SET payload_json = ? WHERE id = ?'
    ).run(JSON.stringify(allocatorMetadata), allocatorFixture.id);
    expect(() => allocatorFixture.persistence.loadResumeSnapshot(allocatorFixture.id)).toThrow(
      'allocator state is invalid'
    );
    allocatorFixture.db.close();
  });

  it('enforces foreign keys and cascades child genomes on parent deletion', () => {
    const { db, id } = saveFixture();
    expect(db.pragma('foreign_keys', { simple: true })).toBe(1);
    const before = db.prepare(
      'SELECT COUNT(*) AS count FROM snapshot_genomes WHERE snapshot_id = ?'
    ).get(id) as { count: number };
    expect(before.count).toBeGreaterThan(0);
    db.prepare('DELETE FROM population_snapshots WHERE id = ?').run(id);
    const after = db.prepare(
      'SELECT COUNT(*) AS count FROM snapshot_genomes WHERE snapshot_id = ?'
    ).get(id) as { count: number };
    expect(after.count).toBe(0);
    db.close();
  });

  it('saves and loads hundreds of genomes without calling the JSON population DTO', () => {
    const exportSpy = vi.spyOn(World.prototype, 'exportPopulation').mockImplementation(() => {
      throw new Error('population JSON DTO must not be called');
    });
    const { db, persistence, fixture } = saveFixture(400);
    expect(exportSpy).not.toHaveBeenCalled();
    const loaded = persistence.loadResumeSnapshot('latest');
    expect(loaded?.genomes).toHaveLength(400);
    expect(exportSpy).not.toHaveBeenCalled();
    const legacyBlobWrites = db.prepare(
      'SELECT COUNT(*) AS count FROM population_snapshots WHERE genomes_blob IS NOT NULL'
    ).get() as { count: number };
    expect(legacyBlobWrites.count).toBe(0);
    expect(fixture.world.population).toHaveLength(400);
    db.close();
  });

  it('rolls back the parent and prior child rows when a genome iterator fails', () => {
    const db = initDb(':memory:');
    const persistence = createPersistence(db);
    const fixture = createCheckpointFixture(3);
    const firstGenome = fixture.world.population[0]!;
    const broken: PopulationCheckpoint = {
      metadata: fixture.checkpoint.metadata,
      genomes: {
        *[Symbol.iterator]() {
          yield {
            slot: 0,
            archKey: firstGenome.archKey,
            brainType: firstGenome.brainType,
            fitness: firstGenome.fitness,
            weights: firstGenome.weights
          };
          throw new Error('synthetic iterator failure');
        }
      }
    };
    expect(() => persistence.saveCheckpoint(broken)).toThrow('synthetic iterator failure');
    const parents = db.prepare('SELECT COUNT(*) AS count FROM population_snapshots').get() as {
      count: number;
    };
    const children = db.prepare('SELECT COUNT(*) AS count FROM snapshot_genomes').get() as {
      count: number;
    };
    expect(parents.count).toBe(0);
    expect(children.count).toBe(0);
    db.close();
  });

  it('loads the historical combined gzip BLOB through the read-only compatibility path', () => {
    const db = initDb(':memory:');
    const persistence = createPersistence(db);
    const fixture = createCheckpointFixture(2);
    const genomes = fixture.world.population.map((genome) => genome.toJSON());
    const payload = {
      generation: 5,
      archKey: fixture.world.archKey,
      genomes: [],
      cfgHash: 'legacy-config',
      worldSeed: fixture.world.seed
    };
    const insert = db.prepare(
      `INSERT INTO population_snapshots (
         created_at, gen, payload_json, settings_json, updates_json, genomes_blob,
         format_version, boundary_kind, population_count
       ) VALUES (?, ?, ?, ?, ?, ?, NULL, NULL, NULL)`
    ).run(
      Date.now(),
      payload.generation,
      JSON.stringify(payload),
      JSON.stringify(fixture.checkpoint.metadata.settings),
      JSON.stringify(fixture.checkpoint.metadata.updates),
      encodeLegacyGenomes(genomes)
    );
    const id = Number(insert.lastInsertRowid);
    const loaded = persistence.loadResumeSnapshot(id);
    expect(loaded?.compatibility).toBe('legacy');
    expect(loaded?.genomes).toHaveLength(2);
    expect(loaded?.genomes[0]?.archKey).toBe(fixture.world.archKey);
    db.close();
  });

  it('reports bounded legacy gzip failures with snapshot and compressed-size context', () => {
    const db = initDb(':memory:');
    const persistence = createPersistence(db);
    const payload = {
      generation: 1,
      archKey: 'legacy-broken',
      genomes: [],
      cfgHash: 'legacy-config',
      worldSeed: 1
    };
    const result = db.prepare(
      `INSERT INTO population_snapshots (
         created_at, gen, payload_json, genomes_blob, format_version, boundary_kind
       ) VALUES (?, ?, ?, ?, NULL, NULL)`
    ).run(Date.now(), 1, JSON.stringify(payload), Buffer.from('not gzip'));
    const id = Number(result.lastInsertRowid);
    expect(() => persistence.loadResumeSnapshot(id)).toThrow(
      expect.objectContaining({
        message: expect.stringContaining(`snapshot ${id}: legacy gzip failed (compressed=8`)
      })
    );
    db.close();
  });

  it('streams current JSON in the established shape one genome record at a time', () => {
    const { db, persistence, id } = saveFixture(5);
    const chunks = Array.from(persistence.exportSnapshotJsonChunks(id));
    expect(chunks.length).toBeGreaterThan(7);
    expect(chunks.filter((chunk) => chunk.includes('"weights"'))).toHaveLength(5);
    const parsed = JSON.parse(chunks.join('')) as {
      generation: number;
      genomes: unknown[];
      cfgHash: string;
      worldSeed: number;
      boundary: { resumable: boolean };
    };
    expect(parsed.generation).toBe(1);
    expect(parsed.genomes).toHaveLength(5);
    expect(parsed.cfgHash).toMatch(/^v1-/u);
    expect(parsed.worldSeed).toBe(0x12345678);
    expect(parsed.boundary.resumable).toBe(true);
    db.close();
  });

  it('retains graph presets and Hall-of-Fame deduplication beside the new schema', () => {
    const db = initDb(':memory:');
    const persistence = createPersistence(db);
    const spec = installTestGraph();
    const presetId = persistence.saveGraphPreset('Unit test preset', spec);
    expect(persistence.loadGraphPreset(presetId)?.spec).toEqual(spec);
    const entry = {
      gen: 5,
      seed: 123,
      fitness: 100.5,
      points: 10,
      length: 50,
      genome: { archKey: 'test', weights: [1, 2, 3] }
    };
    persistence.saveHofEntry(entry);
    persistence.saveHofEntries([entry, entry]);
    expect(persistence.loadHofEntries(10)).toHaveLength(1);
    db.close();
  });
});
