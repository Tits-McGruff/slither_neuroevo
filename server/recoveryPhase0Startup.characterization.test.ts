import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { CFG, resetCFGToDefaults } from '../src/config.ts';
import type { GraphSpec } from '../src/brains/graph/schema.ts';
import { World } from '../src/world.ts';
import { buildGenerationCheckpoint } from './checkpoint.ts';
import { DEFAULT_CONFIG } from './config.ts';
import { startServer, type RunningServer } from './index.ts';
import { createPersistence, initDb } from './persistence.ts';

/** Converted startup characterization label for actual Phase 7 resume behavior. */
const SUITE = 'recovery Phase 7 — converted PER-003 startup resume characterization';

/** Seeded database information used by restart assertions. */
interface SeededDatabase {
  /** Latest exact generation checkpoint id. */
  snapshotId: number;
  /** Persisted lineage id. */
  runId: string;
  /** Persisted root seed. */
  worldSeed: number;
  /** Persisted generation. */
  generation: number;
  /** Population weights in durable slot order. */
  weights: number[][];
  /** Persisted configuration hash. */
  configHash: string;
}

/** Temporary directories created by startup tests. */
const temporaryRoots: string[] = [];
/** Live servers closed defensively after each test. */
const runningServers: RunningServer[] = [];

/**
 * Install a compact graph that still matches the active sensor contract.
 * @returns Installed graph spec.
 */
function installStartupGraph(): GraphSpec {
  const inputSize = CFG.brain.inSize;
  const spec: GraphSpec = {
    type: 'graph',
    nodes: [
      { id: 'input', type: 'Input', outputSize: inputSize },
      { id: 'head', type: 'Dense', inputSize, outputSize: 2 }
    ],
    edges: [{ from: 'input', to: 'head' }],
    outputs: [{ nodeId: 'head' }],
    outputSize: 2
  };
  CFG.brain.graphSpec = spec;
  return spec;
}

/**
 * Create a temporary SQLite path for one test.
 * @returns Absolute DB file path.
 */
function createDatabasePath(): string {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'slither-phase7-startup-'));
  temporaryRoots.push(root);
  return path.join(root, 'resume.db');
}

/**
 * Append a run-start and evolved-generation checkpoint to a database.
 * @param dbPath - SQLite path to seed.
 * @param runId - Lineage id stored in both boundaries.
 * @param worldSeed - Root simulation seed.
 * @returns Latest generation checkpoint metadata and weights.
 */
function seedEvolvedCheckpoint(
  dbPath: string,
  runId: string,
  worldSeed: number
): SeededDatabase {
  resetCFGToDefaults();
  CFG.baselineBots.count = 0;
  CFG.pelletCountTarget = 100;
  CFG.pelletSpawnPerSecond = 5;
  installStartupGraph();
  const db = initDb(dbPath);
  const persistence = createPersistence(db);
  const world = new World(
    {
      snakeCount: 4,
      simSpeed: 1,
      hiddenLayers: 1,
      neurons1: 4,
      neurons2: 3,
      neurons3: 2,
      neurons4: 2,
      neurons5: 2
    },
    {
      seed: worldSeed,
      runId,
      onGenerationBoundary: (boundary, candidate) => {
        persistence.saveCheckpoint(buildGenerationCheckpoint(candidate, boundary, 4));
      }
    }
  );
  world._endGeneration(73);
  const loaded = persistence.loadResumeSnapshot('latest');
  if (!loaded || loaded.compatibility !== 'current') {
    throw new Error('failed to seed current checkpoint');
  }
  const result: SeededDatabase = {
    snapshotId: loaded.id,
    runId: loaded.metadata.runId,
    worldSeed: loaded.metadata.worldSeed,
    generation: loaded.metadata.generation,
    weights: loaded.genomes.map((genome) => Array.from(genome.weights)),
    configHash: loaded.metadata.configHash
  };
  db.close();
  return result;
}

/**
 * Fetch and decode JSON from a running test server.
 * @param server - Running server handle.
 * @param pathname - HTTP path.
 * @param init - Optional fetch request options.
 * @returns Parsed JSON object.
 */
async function fetchJson(
  server: RunningServer,
  pathname: string,
  init?: RequestInit
): Promise<Record<string, unknown>> {
  const response = await fetch(`http://127.0.0.1:${server.port}${pathname}`, init);
  const payload = await response.json() as Record<string, unknown>;
  if (!response.ok) throw new Error(`HTTP ${response.status}: ${JSON.stringify(payload)}`);
  return payload;
}

beforeEach(() => {
  resetCFGToDefaults();
});

afterEach(async () => {
  while (runningServers.length > 0) {
    await runningServers.pop()!.close();
  }
  while (temporaryRoots.length > 0) {
    fs.rmSync(temporaryRoots.pop()!, { recursive: true, force: true });
  }
  resetCFGToDefaults();
});

describe(SUITE, () => {
  it('PER-003 resumes the evolved population, generation, seed, run, and committed step', async () => {
    const dbPath = createDatabasePath();
    const seeded = seedEvolvedCheckpoint(dbPath, 'phase7-resume-run', 0x11223344);
    const server = await startServer({
      ...DEFAULT_CONFIG,
      host: '127.0.0.1',
      port: 0,
      dbPath,
      logLevel: 'error',
      inferenceBackend: 'js',
      mtEnabled: false,
      resume: 'latest'
    });
    runningServers.push(server);

    const health = await fetchJson(server, '/health');
    expect(health['run']).toEqual({ seed: seeded.worldSeed, runId: seeded.runId });
    expect(health['configRevision']).toBe(4);
    expect(health['configHash']).toBe(seeded.configHash);
    expect(health['persistence']).toMatchObject({
      lastDurableSnapshotId: seeded.snapshotId,
      lastDurableGeneration: seeded.generation,
      inMemoryGeneration: seeded.generation,
      exactStartupResume: true
    });
    expect(Number(health['tick'])).toBeGreaterThanOrEqual(73);

    await fetchJson(server, '/api/save', { method: 'POST' });
    const exported = await fetchJson(server, '/api/export/latest');
    const exportedGenomes = exported['genomes'] as Array<{ weights: number[] }>;
    expect(exported['generation']).toBe(seeded.generation);
    expect(exported['worldSeed']).toBe(seeded.worldSeed);
    expect(exported['cfgHash']).toBe(seeded.configHash);
    expect(exportedGenomes.map((genome) => genome.weights)).toEqual(seeded.weights);
    expect(exported['boundary']).toMatchObject({
      kind: 'population-export',
      resumable: false
    });
  });

  it('--fresh ignores and preserves older snapshots while committing a new run-start row', async () => {
    const dbPath = createDatabasePath();
    const seeded = seedEvolvedCheckpoint(dbPath, 'phase7-old-run', 111);
    const beforeDb = initDb(dbPath);
    const beforeCount = (beforeDb.prepare(
      'SELECT COUNT(*) AS count FROM population_snapshots'
    ).get() as { count: number }).count;
    beforeDb.close();

    const server = await startServer({
      ...DEFAULT_CONFIG,
      host: '127.0.0.1',
      port: 0,
      dbPath,
      logLevel: 'error',
      inferenceBackend: 'js',
      mtEnabled: false,
      resume: 'fresh',
      seed: 999
    });
    runningServers.push(server);
    const health = await fetchJson(server, '/health');
    expect(health['run']).toMatchObject({ seed: 999 });
    expect((health['run'] as { runId: string }).runId).not.toBe(seeded.runId);
    expect(health['persistence']).toMatchObject({
      lastDurableGeneration: 1,
      inMemoryGeneration: 1,
      exactStartupResume: false
    });
    await server.close();
    runningServers.pop();

    const afterDb = initDb(dbPath);
    const afterCount = (afterDb.prepare(
      'SELECT COUNT(*) AS count FROM population_snapshots'
    ).get() as { count: number }).count;
    const oldRow = afterDb.prepare(
      'SELECT id FROM population_snapshots WHERE id = ?'
    ).get(seeded.snapshotId) as { id: number } | undefined;
    expect(afterCount).toBe(beforeCount + 1);
    expect(oldRow?.id).toBe(seeded.snapshotId);
    afterDb.close();
  });

  it('--resume <id> selects an older valid checkpoint explicitly', async () => {
    const dbPath = createDatabasePath();
    const older = seedEvolvedCheckpoint(dbPath, 'phase7-older-run', 222);
    const newer = seedEvolvedCheckpoint(dbPath, 'phase7-newer-run', 333);
    expect(newer.snapshotId).toBeGreaterThan(older.snapshotId);

    const server = await startServer({
      ...DEFAULT_CONFIG,
      host: '127.0.0.1',
      port: 0,
      dbPath,
      logLevel: 'error',
      inferenceBackend: 'js',
      mtEnabled: false,
      resume: older.snapshotId
    });
    runningServers.push(server);
    const health = await fetchJson(server, '/health');
    expect(health['run']).toEqual({ seed: older.worldSeed, runId: older.runId });
    expect(health['persistence']).toMatchObject({
      lastDurableSnapshotId: older.snapshotId,
      exactStartupResume: true
    });
  });

  it('fails on a corrupt latest checkpoint and lists an older valid alternative', async () => {
    const dbPath = createDatabasePath();
    const older = seedEvolvedCheckpoint(dbPath, 'phase7-valid-alternative', 444);
    const corrupt = seedEvolvedCheckpoint(dbPath, 'phase7-corrupt-latest', 555);
    const db = initDb(dbPath);
    db.prepare(
      'UPDATE snapshot_genomes SET weights_checksum = ? WHERE snapshot_id = ? AND slot = 0'
    ).run('0'.repeat(64), corrupt.snapshotId);
    db.close();

    await expect(startServer({
      ...DEFAULT_CONFIG,
      host: '127.0.0.1',
      port: 0,
      dbPath,
      logLevel: 'error',
      inferenceBackend: 'js',
      mtEnabled: false,
      resume: 'latest'
    })).rejects.toThrow(
      new RegExp(`snapshot ${corrupt.snapshotId}.*checksum mismatch.*${older.snapshotId}`, 'u')
    );
  });

  it('requires --fresh when a seed override conflicts with an available resume', async () => {
    const dbPath = createDatabasePath();
    seedEvolvedCheckpoint(dbPath, 'phase7-seed-conflict', 666);
    await expect(startServer({
      ...DEFAULT_CONFIG,
      host: '127.0.0.1',
      port: 0,
      dbPath,
      logLevel: 'error',
      inferenceBackend: 'js',
      mtEnabled: false,
      resume: 'latest',
      seed: 777
    })).rejects.toThrow('configured seed conflicts with resume; use --fresh');
  });
});
