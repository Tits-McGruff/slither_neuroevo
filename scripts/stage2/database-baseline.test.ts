/** Regression contracts for the read-only Stage 2 database inventory. */

import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import Database from 'better-sqlite3';
import { afterEach, describe, expect, it } from 'vitest';

/** Temporary roots owned solely by this test suite. */
const temporaryRoots: string[] = [];

afterEach(() => {
  for (const root of temporaryRoots.splice(0)) {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

/**
 * Create a safely named test-owned root.
 * @returns New empty temporary directory.
 */
function createTemporaryRoot(): string {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'slither-stage2-database-'));
  temporaryRoots.push(root);
  return root;
}

/**
 * Execute the public inventory CLI against one fixture.
 * @param databasePath - Fixture database.
 * @param outputPath - JSON result destination.
 * @returns Completed child-process result.
 */
function runInventory(databasePath: string, outputPath: string): ReturnType<typeof spawnSync> {
  const scriptPath = fileURLToPath(new URL('./database-baseline.ts', import.meta.url));
  const tsxCli = path.resolve('node_modules', 'tsx', 'dist', 'cli.mjs');
  return spawnSync(process.execPath, [
    tsxCli,
    scriptPath,
    '--db', databasePath,
    '--output', outputPath
  ], {
    encoding: 'utf8',
    timeout: 30_000,
    maxBuffer: 4 * 1024 * 1024
  });
}

describe('Stage 2 database inventory', () => {
  it('inspects the pre-format-column legacy schema without migrating it', () => {
    const root = createTemporaryRoot();
    const databasePath = path.join(root, 'legacy.db');
    const outputPath = path.join(root, 'inventory.json');
    const database = new Database(databasePath);
    database.exec(`
      CREATE TABLE population_snapshots (
        id INTEGER PRIMARY KEY,
        created_at INTEGER,
        gen INTEGER,
        payload_json TEXT,
        settings_json TEXT,
        updates_json TEXT,
        genomes_blob BLOB
      );
      CREATE TABLE hof_entries (
        id INTEGER PRIMARY KEY,
        created_at INTEGER,
        gen INTEGER,
        seed INTEGER,
        fitness REAL,
        points REAL,
        length REAL,
        genome_json TEXT
      );
    `);
    database.prepare(
      `INSERT INTO population_snapshots
         (id, created_at, gen, payload_json, settings_json, updates_json, genomes_blob)
       VALUES (?, ?, ?, ?, ?, ?, ?)`
    ).run(7, 1000, 3, '{"generation":3}', '{}', '[]', Buffer.from([0x1f, 0x8b, 1, 2, 3]));
    database.prepare(
      `INSERT INTO hof_entries
         (id, created_at, gen, seed, fitness, points, length, genome_json)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?)`
    ).run(2, 1001, 3, 99, 12.5, 8, 10, JSON.stringify({
      archKey: 'legacy-graph',
      brainType: 'graph',
      weights: [1, 2, 3, 4]
    }));
    database.close();

    const before = fs.readFileSync(databasePath);
    const result = runInventory(databasePath, outputPath);
    expect(result.status).toBe(0);
    expect(result.stderr).toBe('');
    expect(fs.readFileSync(databasePath)).toEqual(before);

    const artifact = JSON.parse(fs.readFileSync(outputPath, 'utf8')) as {
      accessMode: string;
      sourcePreservationRequirement: string;
      snapshotStorageShape: {
        parentColumns: string[];
        hasFormatVersionColumn: boolean;
        hasLegacyCombinedBlobColumn: boolean;
        hasPerGenomeChildTable: boolean;
      };
      currentSnapshots: Array<Record<string, unknown>>;
      hallOfFameDetails: {
        totalRows: number;
        omittedRows: number;
        rows: Array<Record<string, unknown>>;
      };
    };
    expect(artifact.accessMode).toBe('read-only');
    expect(artifact.sourcePreservationRequirement).toContain('copied database');
    expect(artifact.snapshotStorageShape).toMatchObject({
      hasFormatVersionColumn: false,
      hasLegacyCombinedBlobColumn: true,
      hasPerGenomeChildTable: false
    });
    expect(artifact.snapshotStorageShape.parentColumns).not.toContain('format_version');
    expect(artifact.currentSnapshots).toEqual([
      expect.objectContaining({
        id: 7,
        gen: 3,
        format_version: null,
        boundary_kind: null,
        population_count: null,
        legacy_blob_bytes: 5
      })
    ]);
    expect(artifact.hallOfFameDetails.totalRows).toBe(1);
    expect(artifact.hallOfFameDetails.omittedRows).toBe(0);
    expect(artifact.hallOfFameDetails.rows).toEqual([
      expect.objectContaining({
        id: 2,
        arch_key: 'legacy-graph',
        brain_type: 'graph',
        weight_count: 4,
        json_valid: 1
      })
    ]);
  });

  it('reports malformed, null, and non-array Hall-of-Fame JSON without throwing', () => {
    const root = createTemporaryRoot();
    const databasePath = path.join(root, 'malformed-hof.db');
    const outputPath = path.join(root, 'inventory.json');
    const database = new Database(databasePath);
    database.exec(`
      CREATE TABLE hof_entries (
        id INTEGER PRIMARY KEY,
        created_at INTEGER,
        gen INTEGER,
        seed INTEGER,
        fitness REAL,
        points REAL,
        length REAL,
        genome_json TEXT
      );
    `);
    const insert = database.prepare(
      `INSERT INTO hof_entries (id, genome_json) VALUES (?, ?)`
    );
    insert.run(1, '{not-json');
    insert.run(2, null);
    insert.run(3, JSON.stringify({ archKey: 'valid-metadata', weights: 7 }));
    database.close();

    const result = runInventory(databasePath, outputPath);
    expect(result.status).toBe(0);
    const artifact = JSON.parse(fs.readFileSync(outputPath, 'utf8')) as {
      hallOfFameDetails: { rows: Array<Record<string, unknown>> };
    };
    expect(artifact.hallOfFameDetails.rows).toEqual([
      expect.objectContaining({ id: 1, json_valid: 0, weight_count: null }),
      expect.objectContaining({ id: 2, json_valid: 0, weight_count: null }),
      expect.objectContaining({ id: 3, json_valid: 1, weight_count: 0 })
    ]);
  });

  it('retains current parent/child snapshot measurements', () => {
    const root = createTemporaryRoot();
    const databasePath = path.join(root, 'current.db');
    const outputPath = path.join(root, 'inventory.json');
    const database = new Database(databasePath);
    database.exec(`
      CREATE TABLE population_snapshots (
        id INTEGER PRIMARY KEY,
        created_at INTEGER,
        gen INTEGER,
        payload_json TEXT,
        settings_json TEXT,
        updates_json TEXT,
        genomes_blob BLOB,
        format_version INTEGER,
        boundary_kind TEXT,
        population_count INTEGER
      );
      CREATE TABLE snapshot_genomes (
        snapshot_id INTEGER,
        population_slot INTEGER,
        weight_count INTEGER,
        weights_blob BLOB
      );
    `);
    database.prepare(
      `INSERT INTO population_snapshots
         (id, created_at, gen, payload_json, format_version, boundary_kind, population_count)
       VALUES (?, ?, ?, ?, ?, ?, ?)`
    ).run(1, 1000, 1, '{}', 2, 'generation', 1);
    database.prepare(
      `INSERT INTO snapshot_genomes
         (snapshot_id, population_slot, weight_count, weights_blob)
       VALUES (?, ?, ?, ?)`
    ).run(1, 0, 2, Buffer.alloc(8));
    database.close();

    const result = runInventory(databasePath, outputPath);
    expect(result.status).toBe(0);
    const artifact = JSON.parse(fs.readFileSync(outputPath, 'utf8')) as {
      snapshotStorageShape: {
        hasFormatVersionColumn: boolean;
        hasPerGenomeChildTable: boolean;
      };
      currentSnapshots: Array<Record<string, unknown>>;
      snapshotGenomes: Array<Record<string, unknown>>;
    };
    expect(artifact.snapshotStorageShape).toMatchObject({
      hasFormatVersionColumn: true,
      hasPerGenomeChildTable: true
    });
    expect(artifact.currentSnapshots).toEqual([
      expect.objectContaining({
        id: 1,
        format_version: 2,
        boundary_kind: 'generation',
        population_count: 1
      })
    ]);
    expect(artifact.snapshotGenomes).toEqual([
      expect.objectContaining({
        snapshot_id: 1,
        genome_count: 1,
        weight_count: 2,
        weight_bytes: 8
      })
    ]);
  });
});
