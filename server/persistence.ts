import fs from 'node:fs';
import path from 'node:path';
import Database from 'better-sqlite3';
import type { HallOfFameEntry, PopulationExport } from '../src/protocol/messages.ts';
import type { GraphSpec } from '../src/brains/graph/schema.ts';
import { validateGraph } from '../src/brains/graph/validate.ts';

const MAX_SNAPSHOT_BYTES = 50 * 1024 * 1024;
const MAX_GENOME_WEIGHTS = 2_000_000;
const MAX_PRESET_BYTES = 256 * 1024;

export interface PopulationSnapshotPayload extends PopulationExport {
  cfgHash: string;
  worldSeed: number;
}

export interface SnapshotMeta {
  id: number;
  createdAt: number;
  gen: number;
}

export interface GraphPresetMeta {
  id: number;
  name: string;
  createdAt: number;
}

export interface GraphPresetPayload extends GraphPresetMeta {
  spec: GraphSpec;
}

export interface Persistence {
  saveHofEntry: (entry: HallOfFameEntry) => void;
  saveSnapshot: (payload: PopulationSnapshotPayload) => number;
  loadLatestSnapshot: () => PopulationSnapshotPayload | null;
  listSnapshots: (limit: number) => SnapshotMeta[];
  exportSnapshot: (id: number) => PopulationSnapshotPayload;
  saveGraphPreset: (name: string, spec: GraphSpec) => number;
  listGraphPresets: (limit: number) => GraphPresetMeta[];
  loadGraphPreset: (id: number) => GraphPresetPayload | null;
}

type DbType = ReturnType<typeof Database>;

const SCHEMA_SQL = `
CREATE TABLE IF NOT EXISTS hof_entries (
  id INTEGER PRIMARY KEY,
  created_at INTEGER,
  gen INTEGER,
  seed INTEGER,
  fitness REAL,
  points REAL,
  length REAL,
  genome_json TEXT
);

CREATE TABLE IF NOT EXISTS population_snapshots (
  id INTEGER PRIMARY KEY,
  created_at INTEGER,
  gen INTEGER,
  payload_json TEXT
);

CREATE TABLE IF NOT EXISTS players (
  id TEXT PRIMARY KEY,
  name TEXT,
  created_at INTEGER
);

CREATE TABLE IF NOT EXISTS graph_presets (
  id INTEGER PRIMARY KEY,
  created_at INTEGER,
  name TEXT,
  spec_json TEXT
);

CREATE INDEX IF NOT EXISTS idx_hof_gen ON hof_entries(gen);
CREATE INDEX IF NOT EXISTS idx_snap_gen ON population_snapshots(gen);
CREATE INDEX IF NOT EXISTS idx_graph_presets_name ON graph_presets(name);
`;

export function initDb(dbPath: string): DbType {
  if (dbPath !== ':memory:') {
    const dir = path.dirname(dbPath);
    fs.mkdirSync(dir, { recursive: true });
  }
  const db = new Database(dbPath);
  db.pragma('journal_mode = WAL');
  db.pragma('synchronous = NORMAL');
  db.exec(SCHEMA_SQL);
  return db;
}

export function createPersistence(db: DbType): Persistence {
  const insertHof = db.prepare(
    `INSERT INTO hof_entries (created_at, gen, seed, fitness, points, length, genome_json)
     VALUES (@created_at, @gen, @seed, @fitness, @points, @length, @genome_json)`
  );
  const insertSnapshot = db.prepare(
    `INSERT INTO population_snapshots (created_at, gen, payload_json)
     VALUES (@created_at, @gen, @payload_json)`
  );
  const latestSnapshot = db.prepare(
    `SELECT payload_json FROM population_snapshots ORDER BY id DESC LIMIT 1`
  );
  const listSnapshotStmt = db.prepare(
    `SELECT id, created_at, gen FROM population_snapshots ORDER BY id DESC LIMIT ?`
  );
  const exportSnapshotStmt = db.prepare(
    `SELECT payload_json FROM population_snapshots WHERE id = ?`
  );
  const insertGraphPreset = db.prepare(
    `INSERT INTO graph_presets (created_at, name, spec_json)
     VALUES (@created_at, @name, @spec_json)`
  );
  const listGraphPresetsStmt = db.prepare(
    `SELECT id, created_at, name FROM graph_presets ORDER BY created_at DESC LIMIT ?`
  );
  const loadGraphPresetStmt = db.prepare(
    `SELECT id, created_at, name, spec_json FROM graph_presets WHERE id = ?`
  );

  const saveHofEntry = (entry: HallOfFameEntry): void => {
    if (!entry || !Number.isFinite(entry.gen)) return;
    if (!Number.isFinite(entry.fitness)) return;
    insertHof.run({
      created_at: Date.now(),
      gen: entry.gen,
      seed: entry.seed,
      fitness: entry.fitness,
      points: entry.points,
      length: entry.length,
      genome_json: JSON.stringify(entry.genome)
    });
  };

  const saveSnapshot = (payload: PopulationSnapshotPayload): number => {
    validateSnapshotPayload(payload);
    const json = JSON.stringify(payload);
    const bytes = Buffer.byteLength(json, 'utf8');
    if (bytes > MAX_SNAPSHOT_BYTES) {
      throw new Error(`snapshot too large (${bytes} bytes)`);
    }
    const info = insertSnapshot.run({
      created_at: Date.now(),
      gen: payload.generation,
      payload_json: json
    });
    return Number(info.lastInsertRowid);
  };

  const loadLatestSnapshot = (): PopulationSnapshotPayload | null => {
    const row = latestSnapshot.get() as { payload_json?: string } | undefined;
    if (!row?.payload_json) return null;
    return JSON.parse(row.payload_json) as PopulationSnapshotPayload;
  };

  const listSnapshots = (limit: number): SnapshotMeta[] => {
    const rows = listSnapshotStmt.all(limit) as Array<{
      id: number;
      created_at: number;
      gen: number;
    }>;
    return rows.map(row => ({
      id: row.id,
      createdAt: row.created_at,
      gen: row.gen
    }));
  };

  const exportSnapshot = (id: number): PopulationSnapshotPayload => {
    const row = exportSnapshotStmt.get(id) as { payload_json?: string } | undefined;
    if (!row?.payload_json) {
      throw new Error('snapshot not found');
    }
    return JSON.parse(row.payload_json) as PopulationSnapshotPayload;
  };

  const saveGraphPreset = (name: string, spec: GraphSpec): number => {
    const trimmed = name.trim();
    if (!trimmed) {
      throw new Error('preset name is required');
    }
    const result = validateGraph(spec);
    if (!result.ok) {
      throw new Error(`invalid graph spec: ${result.reason}`);
    }
    const json = JSON.stringify(spec);
    const bytes = Buffer.byteLength(json, 'utf8');
    if (bytes > MAX_PRESET_BYTES) {
      throw new Error(`preset too large (${bytes} bytes)`);
    }
    const info = insertGraphPreset.run({
      created_at: Date.now(),
      name: trimmed,
      spec_json: json
    });
    return Number(info.lastInsertRowid);
  };

  const listGraphPresets = (limit: number): GraphPresetMeta[] => {
    const rows = listGraphPresetsStmt.all(limit) as Array<{
      id: number;
      created_at: number;
      name: string;
    }>;
    return rows.map(row => ({
      id: row.id,
      name: row.name,
      createdAt: row.created_at
    }));
  };

  const loadGraphPreset = (id: number): GraphPresetPayload | null => {
    const row = loadGraphPresetStmt.get(id) as
      | { id?: number; created_at?: number; name?: string; spec_json?: string }
      | undefined;
    if (!row?.spec_json || !row.id || !row.created_at || !row.name) return null;
    const spec = JSON.parse(row.spec_json) as GraphSpec;
    const result = validateGraph(spec);
    if (!result.ok) {
      throw new Error(`invalid graph preset: ${result.reason}`);
    }
    return {
      id: row.id,
      name: row.name,
      createdAt: row.created_at,
      spec
    };
  };

  return {
    saveHofEntry,
    saveSnapshot,
    loadLatestSnapshot,
    listSnapshots,
    exportSnapshot,
    saveGraphPreset,
    listGraphPresets,
    loadGraphPreset
  };
}

export function validateSnapshotPayload(payload: unknown): asserts payload is PopulationSnapshotPayload {
  if (!payload || typeof payload !== 'object') {
    throw new Error('snapshot payload must be an object');
  }
  const data = payload as PopulationSnapshotPayload;
  if (!Number.isFinite(data.generation)) {
    throw new Error('snapshot generation is invalid');
  }
  if (typeof data.archKey !== 'string' || !data.archKey.trim()) {
    throw new Error('snapshot archKey is invalid');
  }
  if (!Array.isArray(data.genomes) || data.genomes.length === 0) {
    throw new Error('snapshot genomes missing');
  }
  if (typeof data.cfgHash !== 'string' || !data.cfgHash.trim()) {
    throw new Error('snapshot cfgHash missing');
  }
  if (!Number.isFinite(data.worldSeed)) {
    throw new Error('snapshot worldSeed missing');
  }
  for (const genome of data.genomes) {
    if (!genome || typeof genome.archKey !== 'string') {
      throw new Error('genome archKey missing');
    }
    if (!Array.isArray(genome.weights)) {
      throw new Error('genome weights missing');
    }
    if (genome.weights.length > MAX_GENOME_WEIGHTS) {
      throw new Error('genome weights too large');
    }
    for (const w of genome.weights) {
      if (typeof w !== 'number' || !Number.isFinite(w)) {
        throw new Error('genome weights contain invalid numbers');
      }
    }
  }
}
