import fs from 'node:fs';
import path from 'node:path';
import Database from 'better-sqlite3';
import type { HallOfFameEntry, PopulationExport } from '../src/protocol/messages.ts';
import type { CoreSettings, SettingsUpdate } from '../src/protocol/settings.ts';
import type { GraphSpec } from '../src/brains/graph/schema.ts';
import { validateGraph } from '../src/brains/graph/validate.ts';

/** Maximum serialized snapshot size in bytes. */
const MAX_SNAPSHOT_BYTES = 512 * 1024 * 1024;
/** Upper bound on genome weight array length. */
const MAX_GENOME_WEIGHTS = 2_000_000;
/** Maximum serialized preset size in bytes. */
const MAX_PRESET_BYTES = 256 * 1024;

/** Population snapshot payload stored in SQLite. */
export interface PopulationSnapshotPayload extends PopulationExport {
  cfgHash: string;
  worldSeed: number;
  settings?: CoreSettings;
  updates?: SettingsUpdate[];
}

/** Snapshot metadata returned by list endpoints. */
export interface SnapshotMeta {
  id: number;
  createdAt: number;
  gen: number;
}

/** Graph preset metadata returned by list endpoints. */
export interface GraphPresetMeta {
  id: number;
  name: string;
  createdAt: number;
}

/** Graph preset payload returned by load endpoints. */
export interface GraphPresetPayload extends GraphPresetMeta {
  spec: GraphSpec;
}

/** Persistence interface for snapshots, HoF entries, and graph presets. */
export interface Persistence {
  saveHofEntry: (entry: HallOfFameEntry) => void;
  loadHofEntries: (limit: number) => HallOfFameEntry[];
  saveSnapshot: (payload: PopulationSnapshotPayload) => number;
  loadLatestSnapshot: () => PopulationSnapshotPayload | null;
  listSnapshots: (limit: number) => SnapshotMeta[];
  exportSnapshot: (id: number) => PopulationSnapshotPayload;
  saveGraphPreset: (name: string, spec: GraphSpec) => number;
  listGraphPresets: (limit: number) => GraphPresetMeta[];
  loadGraphPreset: (id: number) => GraphPresetPayload | null;
}

/** Database handle type for better-sqlite3. */
type DbType = ReturnType<typeof Database>;

/** SQLite schema used by the server for persistence. */
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
  payload_json TEXT,
  settings_json TEXT,
  updates_json TEXT
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

/**
 * Ensure optional snapshot columns exist for settings persistence.
 * @param db - Database handle to update.
 */
function ensureSnapshotColumns(db: DbType): void {
  const rows = db.prepare(`PRAGMA table_info(population_snapshots)`).all() as Array<{ name: string }>;
  const columns = new Set(rows.map(row => row.name));
  if (!columns.has('settings_json')) {
    db.exec(`ALTER TABLE population_snapshots ADD COLUMN settings_json TEXT`);
  }
  if (!columns.has('updates_json')) {
    db.exec(`ALTER TABLE population_snapshots ADD COLUMN updates_json TEXT`);
  }
}

/**
 * Parse optional JSON from a nullable column string.
 * @param raw - Raw JSON string or null.
 * @returns Parsed value or null when missing or invalid.
 */
function parseOptionalJson<T>(raw: string | null | undefined): T | null {
  if (!raw || typeof raw !== 'string') return null;
  try {
    return JSON.parse(raw) as T;
  } catch {
    return null;
  }
}

/**
 * Merge optional settings columns into a snapshot payload.
 * @param row - Row containing payload JSON and optional columns.
 * @returns Parsed snapshot payload or null when missing.
 */
function parseSnapshotRow(row: {
  payload_json?: string;
  settings_json?: string | null;
  updates_json?: string | null;
} | undefined): PopulationSnapshotPayload | null {
  if (!row?.payload_json) return null;
  const payload = JSON.parse(row.payload_json) as PopulationSnapshotPayload;
  const settings = parseOptionalJson<CoreSettings>(row.settings_json);
  const updates = parseOptionalJson<SettingsUpdate[]>(row.updates_json);
  if (settings) payload.settings = settings;
  if (updates) payload.updates = updates;
  return payload;
}

/**
 * Initialize the SQLite database and schema.
 * @param dbPath - Path to the sqlite database file.
 * @returns Database handle.
 */
export function initDb(dbPath: string): DbType {
  if (dbPath !== ':memory:') {
    const dir = path.dirname(dbPath);
    fs.mkdirSync(dir, { recursive: true });
  }
  const db = new Database(dbPath);
  db.pragma('journal_mode = WAL');
  db.pragma('synchronous = NORMAL');
  db.exec(SCHEMA_SQL);
  ensureSnapshotColumns(db);
  return db;
}

/**
 * Create persistence helpers backed by a SQLite database.
 * @param db - Database handle.
 * @returns Persistence API surface.
 */
export function createPersistence(db: DbType): Persistence {
  const insertHof = db.prepare(
    `INSERT INTO hof_entries (created_at, gen, seed, fitness, points, length, genome_json)
     VALUES (@created_at, @gen, @seed, @fitness, @points, @length, @genome_json)`
  );
  const insertSnapshot = db.prepare(
    `INSERT INTO population_snapshots (created_at, gen, payload_json, settings_json, updates_json)
     VALUES (@created_at, @gen, @payload_json, @settings_json, @updates_json)`
  );
  const latestSnapshot = db.prepare(
    `SELECT payload_json, settings_json, updates_json FROM population_snapshots ORDER BY id DESC LIMIT 1`
  );
  const listSnapshotStmt = db.prepare(
    `SELECT id, created_at, gen FROM population_snapshots ORDER BY id DESC LIMIT ?`
  );
  const exportSnapshotStmt = db.prepare(
    `SELECT payload_json, settings_json, updates_json FROM population_snapshots WHERE id = ?`
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
  const listHofStmt = db.prepare(
    `SELECT gen, seed, fitness, points, length, genome_json FROM hof_entries ORDER BY fitness DESC LIMIT ?`
  );

  /** Persist a Hall of Fame entry. */
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

  /** Load top Hall of Fame entries. */
  const loadHofEntries = (limit: number): HallOfFameEntry[] => {
    const rows = listHofStmt.all(limit) as Array<{
      gen: number;
      seed: number;
      fitness: number;
      points: number;
      length: number;
      genome_json: string;
    }>;
    return rows.map(row => ({
      gen: row.gen,
      seed: row.seed,
      fitness: row.fitness,
      points: row.points,
      length: row.length,
      genome: JSON.parse(row.genome_json)
    }));
  };

  /** Persist a population snapshot and return its id. */
  const saveSnapshot = (payload: PopulationSnapshotPayload): number => {
    validateSnapshotPayload(payload);
    const json = JSON.stringify(payload);
    const bytes = Buffer.byteLength(json, 'utf8');
    if (bytes > MAX_SNAPSHOT_BYTES) {
      throw new Error(`snapshot too large (${bytes} bytes)`);
    }
    const settingsJson = payload.settings ? JSON.stringify(payload.settings) : null;
    const updatesJson = payload.updates ? JSON.stringify(payload.updates) : null;
    const info = insertSnapshot.run({
      created_at: Date.now(),
      gen: payload.generation,
      payload_json: json,
      settings_json: settingsJson,
      updates_json: updatesJson
    });
    return Number(info.lastInsertRowid);
  };

  /** Load the latest population snapshot. */
  const loadLatestSnapshot = (): PopulationSnapshotPayload | null => {
    const row = latestSnapshot.get() as {
      payload_json?: string;
      settings_json?: string | null;
      updates_json?: string | null;
    } | undefined;
    return parseSnapshotRow(row);
  };

  /** List snapshot metadata in descending order. */
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

  /** Load a specific snapshot payload by id. */
  const exportSnapshot = (id: number): PopulationSnapshotPayload => {
    const row = exportSnapshotStmt.get(id) as {
      payload_json?: string;
      settings_json?: string | null;
      updates_json?: string | null;
    } | undefined;
    const payload = parseSnapshotRow(row);
    if (!payload) {
      throw new Error('snapshot not found');
    }
    return payload;
  };

  /** Persist a graph preset and return its id. */
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

  /** List graph presets in descending order. */
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

  /** Load a graph preset payload by id. */
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
    loadHofEntries,
    saveSnapshot,
    loadLatestSnapshot,
    listSnapshots,
    exportSnapshot,
    saveGraphPreset,
    listGraphPresets,
    loadGraphPreset
  };
}

/**
 * Validate a snapshot payload and assert it matches required fields.
 * @param payload - Raw payload to validate.
 * @throws Error when payload is invalid.
 */
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
