import { createHash } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import zlib from 'node:zlib';
import Database from 'better-sqlite3';
import type { GenomeJSON, HallOfFameEntry } from '../src/protocol/messages.ts';
import { SETTINGS_PATHS } from '../src/protocol/settings.ts';
import type { GraphSpec } from '../src/brains/graph/schema.ts';
import { validateGraph } from '../src/brains/graph/validate.ts';
import { compileBrainSpec } from '../src/brains/registry.ts';
import { StatefulRng, type SerializedRngState } from '../src/rng.ts';
import {
  SNAPSHOT_BOUNDARY_VERSION,
  SNAPSHOT_FORMAT_VERSION,
  typedGenomeToJson,
  type LoadedLegacyCheckpoint,
  type LoadedPopulationCheckpoint,
  type LoadedResumeSnapshot,
  type PopulationCheckpoint,
  type PopulationCheckpointMetadata,
  type PopulationSnapshotPayload,
  type SnapshotMeta,
  type TypedGenomeSnapshot
} from './snapshotTypes.ts';

export type {
  LoadedLegacyCheckpoint,
  LoadedPopulationCheckpoint,
  LoadedResumeSnapshot,
  PopulationCheckpoint,
  PopulationCheckpointMetadata,
  PopulationSnapshotPayload,
  SnapshotMeta,
  TypedGenomeSnapshot
} from './snapshotTypes.ts';

/** Maximum current-format population size accepted by persistence. */
const MAX_POPULATION_COUNT = 10_000;
/** Upper bound on one genome's Float32 parameter count. */
const MAX_GENOME_WEIGHTS = 2_000_000;
/** Maximum current-format metadata JSON size. */
const MAX_METADATA_BYTES = 16 * 1024 * 1024;
/** Maximum compressed legacy blob size accepted before decompression. */
const MAX_LEGACY_COMPRESSED_BYTES = 512 * 1024 * 1024;
/** Maximum legacy decompressed population size. */
const MAX_LEGACY_UNCOMPRESSED_BYTES = 512 * 1024 * 1024;
/** Maximum JSON bytes accepted for one legacy genome record. */
const MAX_LEGACY_GENOME_JSON_BYTES = 64 * 1024 * 1024;
/** Maximum serialized preset size in bytes. */
const MAX_PRESET_BYTES = 256 * 1024;
/** SHA-256 checksum hex character count. */
const SHA256_HEX_LENGTH = 64;
/** Runtime membership set for persisted authoritative setting paths. */
const SETTINGS_PATH_SET = new Set<string>(SETTINGS_PATHS);

/** Snapshot metadata returned by graph-preset list endpoints. */
export interface GraphPresetMeta {
  /** SQLite row id. */
  id: number;
  /** User-visible preset name. */
  name: string;
  /** Creation time in milliseconds since epoch. */
  createdAt: number;
}

/** Graph preset payload returned by load endpoints. */
export interface GraphPresetPayload extends GraphPresetMeta {
  /** Validated graph definition. */
  spec: GraphSpec;
}

/** Persistence interface for snapshots, HoF entries, and graph presets. */
export interface Persistence {
  /** Persist one Hall-of-Fame entry. */
  saveHofEntry: (entry: HallOfFameEntry) => void;
  /** Persist several Hall-of-Fame entries atomically. */
  saveHofEntries: (entries: HallOfFameEntry[]) => void;
  /** Load top Hall-of-Fame entries. */
  loadHofEntries: (limit: number) => HallOfFameEntry[];
  /** Save one current-format metadata row plus per-genome child rows. */
  saveCheckpoint: (checkpoint: PopulationCheckpoint) => number;
  /** Select the latest or one explicit resumable snapshot. */
  loadResumeSnapshot: (selection: 'latest' | number) => LoadedResumeSnapshot | null;
  /** Return older valid resume alternatives for actionable errors. */
  listValidResumeSnapshots: (limit: number, excludeId?: number) => SnapshotMeta[];
  /** Load the newest snapshot in JSON-compatible transport form. */
  loadLatestSnapshot: () => PopulationSnapshotPayload | null;
  /** List snapshot metadata in newest-first order. */
  listSnapshots: (limit: number) => SnapshotMeta[];
  /** Load one snapshot in JSON-compatible transport form. */
  exportSnapshot: (id: number) => PopulationSnapshotPayload;
  /** Return the latest snapshot row id, or null when empty. */
  getLatestSnapshotId: () => number | null;
  /** Yield a JSON export incrementally, bounded to one genome at a time. */
  exportSnapshotJsonChunks: (id: number) => Iterable<string>;
  /** Save one graph preset. */
  saveGraphPreset: (name: string, spec: GraphSpec) => number;
  /** List graph presets. */
  listGraphPresets: (limit: number) => GraphPresetMeta[];
  /** Load one graph preset. */
  loadGraphPreset: (id: number) => GraphPresetPayload | null;
}

/** Database handle type for better-sqlite3. */
type DbType = ReturnType<typeof Database>;

/** Parent snapshot row selected from SQLite. */
interface SnapshotRow {
  /** SQLite snapshot id. */
  id: number;
  /** Creation time in milliseconds since epoch. */
  created_at: number;
  /** Stored generation number. */
  gen: number;
  /** Current metadata JSON or legacy payload JSON. */
  payload_json: string | null;
  /** Legacy settings column. */
  settings_json: string | null;
  /** Legacy updates column. */
  updates_json: string | null;
  /** Read-only legacy combined population blob. */
  genomes_blob: Buffer | null;
  /** Current format version or null for legacy. */
  format_version: number | null;
  /** Current boundary kind or null for legacy. */
  boundary_kind: string | null;
  /** Current declared population count or null for legacy. */
  population_count: number | null;
}

/** One per-genome child row selected from SQLite. */
interface GenomeRow {
  /** Dense population slot. */
  slot: number;
  /** Architecture key. */
  arch_key: string;
  /** Brain-family metadata. */
  brain_type: string;
  /** Stored fitness. */
  fitness: number;
  /** Declared Float32 element count. */
  weight_count: number;
  /** Little-endian Float32 bytes. */
  weights_blob: Buffer;
  /** SHA-256 checksum of weights_blob. */
  weights_checksum: string;
}

/** Error that identifies the snapshot responsible for a load failure. */
export class SnapshotLoadError extends Error {
  /** SQLite snapshot id that failed validation. */
  readonly snapshotId: number;

  /**
   * Create a snapshot-specific load error.
   * @param snapshotId - SQLite row id.
   * @param reason - Actionable validation failure.
   * @param cause - Optional underlying exception.
   */
  constructor(snapshotId: number, reason: string, cause?: unknown) {
    super(`snapshot ${snapshotId}: ${reason}`, cause === undefined ? undefined : { cause });
    this.name = 'SnapshotLoadError';
    this.snapshotId = snapshotId;
  }
}

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
  genome_json TEXT,
  UNIQUE(gen, seed, fitness)
);

CREATE TABLE IF NOT EXISTS population_snapshots (
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

CREATE TABLE IF NOT EXISTS snapshot_genomes (
  snapshot_id INTEGER NOT NULL,
  slot INTEGER NOT NULL,
  arch_key TEXT NOT NULL,
  brain_type TEXT NOT NULL,
  fitness REAL NOT NULL,
  weight_count INTEGER NOT NULL,
  weights_blob BLOB NOT NULL,
  weights_checksum TEXT NOT NULL,
  PRIMARY KEY (snapshot_id, slot),
  FOREIGN KEY (snapshot_id) REFERENCES population_snapshots(id)
    ON DELETE CASCADE
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

/** Snapshot columns added idempotently to legacy databases. */
const SNAPSHOT_COLUMN_MIGRATIONS = [
  ['settings_json', 'TEXT'],
  ['updates_json', 'TEXT'],
  ['genomes_blob', 'BLOB'],
  ['format_version', 'INTEGER'],
  ['boundary_kind', 'TEXT'],
  ['population_count', 'INTEGER']
] as const;

/**
 * Test whether an unknown value is a non-null record.
 * @param value - Value to inspect.
 * @returns True when value is an object record.
 */
function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

/**
 * Ensure optional and current snapshot columns exist on a legacy parent table.
 * @param db - Database handle to update.
 */
function ensureSnapshotColumns(db: DbType): void {
  const rows = db.prepare(`PRAGMA table_info(population_snapshots)`).all() as Array<{
    name: string;
  }>;
  const columns = new Set(rows.map((row) => row.name));
  for (const [name, sqlType] of SNAPSHOT_COLUMN_MIGRATIONS) {
    if (!columns.has(name)) {
      db.exec(`ALTER TABLE population_snapshots ADD COLUMN ${name} ${sqlType}`);
    }
  }
}

/**
 * Assert SQLite foreign-key enforcement is active on the opened connection.
 * @param db - Database connection to configure and verify.
 */
function enableForeignKeys(db: DbType): void {
  db.pragma('foreign_keys = ON');
  const enabled = db.pragma('foreign_keys', { simple: true }) as number;
  if (enabled !== 1) {
    throw new Error('SQLite foreign-key enforcement could not be enabled');
  }
}

/**
 * Encode one Float32 buffer explicitly as little-endian bytes.
 * @param weights - Finite Float32 parameters.
 * @param target - Optional exactly sized scratch buffer reused by bounded saves.
 * @returns One-genome byte buffer.
 */
export function encodeWeightsLittleEndian(
  weights: Float32Array,
  target?: Buffer
): Buffer {
  const byteLength = weights.length * Float32Array.BYTES_PER_ELEMENT;
  if (target && target.byteLength !== byteLength) {
    throw new Error(`weight scratch buffer has ${target.byteLength} bytes; expected ${byteLength}`);
  }
  const output = target ?? Buffer.allocUnsafe(byteLength);
  for (let index = 0; index < weights.length; index++) {
    output.writeFloatLE(weights[index]!, index * Float32Array.BYTES_PER_ELEMENT);
  }
  return output;
}

/**
 * Decode validated little-endian bytes into one Float32 parameter buffer.
 * @param blob - Raw little-endian bytes.
 * @param weightCount - Declared Float32 element count.
 * @returns Typed parameter buffer.
 */
export function decodeWeightsLittleEndian(blob: Buffer, weightCount: number): Float32Array {
  const expectedBytes = weightCount * Float32Array.BYTES_PER_ELEMENT;
  if (blob.byteLength !== expectedBytes) {
    throw new Error(
      `weight byte length ${blob.byteLength} does not match ${weightCount} Float32 values`
    );
  }
  const weights = new Float32Array(weightCount);
  for (let index = 0; index < weightCount; index++) {
    const value = blob.readFloatLE(index * Float32Array.BYTES_PER_ELEMENT);
    if (!Number.isFinite(value)) {
      throw new Error(`weight ${index} is not finite`);
    }
    weights[index] = value;
  }
  return weights;
}

/**
 * Hash one genome byte buffer with built-in SHA-256.
 * @param bytes - Encoded Float32 bytes.
 * @returns Lowercase hexadecimal checksum.
 */
export function checksumWeights(bytes: Buffer): string {
  return createHash('sha256').update(bytes).digest('hex');
}

/**
 * Parse optional JSON from a nullable legacy column.
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
 * Validate one serialized PRNG stream using the production state decoder.
 * @param value - Untrusted serialized stream.
 * @param label - Diagnostic stream label.
 */
function validateSerializedRng(value: unknown, label: string): void {
  if (!isRecord(value)) throw new Error(`checkpoint ${label} RNG state is invalid`);
  try {
    StatefulRng.fromState(value as unknown as SerializedRngState);
  } catch (error) {
    throw new Error(
      `checkpoint ${label} RNG state is invalid: ${error instanceof Error ? error.message : String(error)}`,
      { cause: error }
    );
  }
}

/**
 * Validate exact World RNG and deterministic allocator continuation metadata.
 * @param metadata - Current checkpoint metadata under validation.
 */
function validateWorldContinuation(metadata: PopulationCheckpointMetadata): void {
  const rng = metadata.rng;
  if (!isRecord(rng) || rng.version !== 1 || rng.seed !== metadata.worldSeed) {
    throw new Error('checkpoint RNG state does not match world seed/version');
  }
  validateSerializedRng(rng.world, 'world');
  validateSerializedRng(rng.evolution, 'evolution');
  validateSerializedRng(rng.observer, 'observer');
  if (!Array.isArray(rng.baselines)) {
    throw new Error('checkpoint baseline RNG states are invalid');
  }
  const baselineCountUpdate = metadata.updates.find(
    (update) => update.path === 'baselineBots.count'
  );
  if (
    !baselineCountUpdate ||
    !Number.isSafeInteger(baselineCountUpdate.value) ||
    baselineCountUpdate.value < 0 ||
    rng.baselines.length !== baselineCountUpdate.value
  ) {
    throw new Error('checkpoint baseline RNG count does not match settings');
  }
  for (let slot = 0; slot < rng.baselines.length; slot++) {
    const baseline = rng.baselines[slot];
    if (
      !isRecord(baseline) ||
      baseline.slot !== slot ||
      !Number.isSafeInteger(baseline.seed) ||
      baseline.seed < 0 ||
      baseline.seed > 0xffffffff
    ) {
      throw new Error(`checkpoint baseline RNG slot ${slot} is invalid`);
    }
    validateSerializedRng(baseline.rng, `baseline ${slot}`);
  }
  const allocators = metadata.allocators;
  if (
    !isRecord(allocators) ||
    allocators.version !== 1 ||
    !Number.isSafeInteger(allocators.nextExternalSnakeId) ||
    allocators.nextExternalSnakeId < 100000 ||
    !Number.isSafeInteger(allocators.nextBaselineBotId) ||
    allocators.nextBaselineBotId < 200000 ||
    !Number.isSafeInteger(allocators.nextResurrectedSnakeId) ||
    allocators.nextResurrectedSnakeId < 1000000000
  ) {
    throw new Error('checkpoint allocator state is invalid');
  }
}

/**
 * Validate current checkpoint metadata and return compiled parameter count.
 * @param value - Untrusted metadata parsed from JSON or supplied by a caller.
 * @returns Parameter count implied by the graph definition.
 */
function validateCheckpointMetadata(value: unknown): number {
  if (!isRecord(value)) throw new Error('checkpoint metadata must be an object');
  const metadata = value as unknown as PopulationCheckpointMetadata;
  if (metadata.formatVersion !== SNAPSHOT_FORMAT_VERSION) {
    throw new Error(`unsupported format version ${String(metadata.formatVersion)}`);
  }
  if (metadata.boundaryVersion !== SNAPSHOT_BOUNDARY_VERSION) {
    throw new Error(`unsupported boundary version ${String(metadata.boundaryVersion)}`);
  }
  if (!['run-start', 'generation', 'population-export'].includes(metadata.boundaryKind)) {
    throw new Error(`unsupported boundary kind ${String(metadata.boundaryKind)}`);
  }
  const expectedResumable = metadata.boundaryKind !== 'population-export';
  if (metadata.resumable !== expectedResumable) {
    throw new Error('boundary resumable flag is inconsistent with its kind');
  }
  if (!Number.isSafeInteger(metadata.generation) || metadata.generation < 1) {
    throw new Error('checkpoint generation is invalid');
  }
  if (!Number.isSafeInteger(metadata.simulationStep) || metadata.simulationStep < 0) {
    throw new Error('checkpoint simulation step is invalid');
  }
  if (typeof metadata.runId !== 'string' || !metadata.runId.trim()) {
    throw new Error('checkpoint runId is invalid');
  }
  if (
    !Number.isSafeInteger(metadata.worldSeed) ||
    metadata.worldSeed < 0 ||
    metadata.worldSeed > 0xffffffff
  ) {
    throw new Error('checkpoint worldSeed is invalid');
  }
  if (typeof metadata.configHash !== 'string' || !metadata.configHash.trim()) {
    throw new Error('checkpoint config hash is invalid');
  }
  if (!Number.isSafeInteger(metadata.configRevision) || metadata.configRevision < 0) {
    throw new Error('checkpoint config revision is invalid');
  }
  if (typeof metadata.archKey !== 'string' || !metadata.archKey.trim()) {
    throw new Error('checkpoint architecture key is invalid');
  }
  if (
    !Number.isSafeInteger(metadata.populationCount) ||
    metadata.populationCount < 1 ||
    metadata.populationCount > MAX_POPULATION_COUNT
  ) {
    throw new Error(`checkpoint population count ${metadata.populationCount} is invalid`);
  }
  if (!isRecord(metadata.settings) || metadata.settings.snakeCount !== metadata.populationCount) {
    throw new Error('checkpoint settings do not match population count');
  }
  for (const valuePart of Object.values(metadata.settings)) {
    if (typeof valuePart !== 'number' || !Number.isFinite(valuePart)) {
      throw new Error('checkpoint core settings contain invalid values');
    }
  }
  if (!Array.isArray(metadata.updates)) {
    throw new Error('checkpoint settings updates are invalid');
  }
  for (const update of metadata.updates) {
    if (
      !isRecord(update) ||
      typeof update.path !== 'string' ||
      !SETTINGS_PATH_SET.has(update.path) ||
      typeof update.value !== 'number' ||
      !Number.isFinite(update.value)
    ) {
      throw new Error('checkpoint settings update is invalid');
    }
  }
  validateWorldContinuation(metadata);
  if (!Number.isFinite(metadata.bestFitnessEver)) {
    throw new Error('checkpoint best fitness is invalid');
  }
  if (!Array.isArray(metadata.fitnessHistory) || metadata.fitnessHistory.length > 1000) {
    throw new Error('checkpoint fitness history is invalid');
  }
  for (const entry of metadata.fitnessHistory) {
    if (!isRecord(entry)) throw new Error('checkpoint fitness history entry is invalid');
    for (const part of Object.values(entry)) {
      if (typeof part !== 'number' || !Number.isFinite(part)) {
        throw new Error('checkpoint fitness history contains invalid values');
      }
    }
  }
  const graphValidation = validateGraph(metadata.graphSpec);
  if (!graphValidation.ok) {
    throw new Error(`checkpoint graph is invalid: ${graphValidation.reason}`);
  }
  const compiled = compileBrainSpec(metadata.graphSpec);
  if (compiled.key !== metadata.archKey) {
    throw new Error(
      `checkpoint graph key ${compiled.key} does not match ${metadata.archKey}`
    );
  }
  if (compiled.totalParams < 1 || compiled.totalParams > MAX_GENOME_WEIGHTS) {
    throw new Error(`checkpoint graph parameter count ${compiled.totalParams} is invalid`);
  }
  return compiled.totalParams;
}

/**
 * Validate and encode one current-format genome.
 * @param genome - Typed genome supplied by the checkpoint source.
 * @param expectedSlot - Dense slot expected at this point in iteration.
 * @param metadata - Validated snapshot metadata.
 * @param expectedWeightCount - Parameter count implied by the graph.
 * @param scratch - Exactly one reusable genome-sized encoding buffer.
 * @returns Encoded bytes and checksum.
 */
function validateAndEncodeGenome(
  genome: TypedGenomeSnapshot,
  expectedSlot: number,
  metadata: PopulationCheckpointMetadata,
  expectedWeightCount: number,
  scratch: Buffer
): { bytes: Buffer; checksum: string } {
  if (genome.slot !== expectedSlot) {
    throw new Error(`genome slot ${genome.slot} is not dense at expected slot ${expectedSlot}`);
  }
  if (genome.archKey !== metadata.archKey) {
    throw new Error(`genome ${genome.slot} architecture key does not match checkpoint`);
  }
  if (genome.brainType !== metadata.graphSpec.type) {
    throw new Error(`genome ${genome.slot} brain type ${genome.brainType} is unsupported`);
  }
  if (!Number.isFinite(genome.fitness)) {
    throw new Error(`genome ${genome.slot} fitness is invalid`);
  }
  if (!(genome.weights instanceof Float32Array)) {
    throw new Error(`genome ${genome.slot} weights are not Float32`);
  }
  if (genome.weights.length !== expectedWeightCount) {
    throw new Error(
      `genome ${genome.slot} weight count ${genome.weights.length} does not match ${expectedWeightCount}`
    );
  }
  for (let index = 0; index < genome.weights.length; index++) {
    if (!Number.isFinite(genome.weights[index])) {
      throw new Error(`genome ${genome.slot} weight ${index} is not finite`);
    }
  }
  const bytes = encodeWeightsLittleEndian(genome.weights, scratch);
  return { bytes, checksum: checksumWeights(bytes) };
}

/**
 * Decode and validate one current-format child row.
 * @param row - SQLite child row.
 * @param snapshotId - Owning snapshot id for diagnostics.
 * @param expectedSlot - Dense expected slot.
 * @param metadata - Validated current metadata.
 * @param expectedWeightCount - Graph parameter count.
 * @returns Typed genome.
 */
function decodeGenomeRow(
  row: GenomeRow,
  snapshotId: number,
  expectedSlot: number,
  metadata: PopulationCheckpointMetadata,
  expectedWeightCount: number
): TypedGenomeSnapshot {
  if (row.slot !== expectedSlot) {
    throw new SnapshotLoadError(
      snapshotId,
      `genome slots are not dense: expected ${expectedSlot}, found ${row.slot}`
    );
  }
  if (row.arch_key !== metadata.archKey) {
    throw new SnapshotLoadError(snapshotId, `genome ${row.slot} architecture key mismatch`);
  }
  if (row.brain_type !== metadata.graphSpec.type) {
    throw new SnapshotLoadError(
      snapshotId,
      `genome ${row.slot} has unsupported brain type ${row.brain_type}`
    );
  }
  if (!Number.isFinite(row.fitness)) {
    throw new SnapshotLoadError(snapshotId, `genome ${row.slot} fitness is invalid`);
  }
  if (row.weight_count !== expectedWeightCount) {
    throw new SnapshotLoadError(
      snapshotId,
      `genome ${row.slot} weight count ${row.weight_count} does not match graph ${expectedWeightCount}`
    );
  }
  if (
    typeof row.weights_checksum !== 'string' ||
    row.weights_checksum.length !== SHA256_HEX_LENGTH ||
    !/^[0-9a-f]+$/u.test(row.weights_checksum)
  ) {
    throw new SnapshotLoadError(snapshotId, `genome ${row.slot} checksum metadata is invalid`);
  }
  const actualChecksum = checksumWeights(row.weights_blob);
  if (actualChecksum !== row.weights_checksum) {
    throw new SnapshotLoadError(snapshotId, `genome ${row.slot} checksum mismatch`);
  }
  try {
    return {
      slot: row.slot,
      archKey: row.arch_key,
      brainType: row.brain_type,
      fitness: row.fitness,
      weights: decodeWeightsLittleEndian(row.weights_blob, row.weight_count)
    };
  } catch (error) {
    throw new SnapshotLoadError(
      snapshotId,
      `genome ${row.slot} is invalid: ${error instanceof Error ? error.message : String(error)}`,
      error
    );
  }
}

/**
 * Decode the read-only legacy combined blob with explicit size and framing limits.
 * @param snapshotId - Owning snapshot id.
 * @param blob - Gzipped length-prefixed genome JSON records.
 * @param expectedPopulation - Expected population count when known.
 * @returns Parsed legacy genome JSON values.
 */
function deserializeLegacyGenomes(
  snapshotId: number,
  blob: Buffer,
  expectedPopulation: number | null
): unknown[] {
  if (blob.byteLength > MAX_LEGACY_COMPRESSED_BYTES) {
    throw new SnapshotLoadError(
      snapshotId,
      `legacy blob compressed size ${blob.byteLength} exceeds ${MAX_LEGACY_COMPRESSED_BYTES}`
    );
  }
  let decompressed: Buffer;
  try {
    decompressed = zlib.gunzipSync(blob, {
      maxOutputLength: MAX_LEGACY_UNCOMPRESSED_BYTES
    });
  } catch (error) {
    throw new SnapshotLoadError(
      snapshotId,
      `legacy gzip failed (compressed=${blob.byteLength}, maxOutput=${MAX_LEGACY_UNCOMPRESSED_BYTES}, expectedPopulation=${expectedPopulation ?? 'unknown'}): ${error instanceof Error ? error.message : String(error)}`,
      error
    );
  }
  const genomes: unknown[] = [];
  let offset = 0;
  while (offset < decompressed.length) {
    if (offset + 4 > decompressed.length) {
      throw new SnapshotLoadError(snapshotId, `legacy genome length prefix is truncated at ${offset}`);
    }
    const length = decompressed.readUInt32LE(offset);
    offset += 4;
    if (length > MAX_LEGACY_GENOME_JSON_BYTES) {
      throw new SnapshotLoadError(
        snapshotId,
        `legacy genome ${genomes.length} JSON size ${length} exceeds ${MAX_LEGACY_GENOME_JSON_BYTES}`
      );
    }
    if (offset + length > decompressed.length) {
      throw new SnapshotLoadError(
        snapshotId,
        `legacy genome ${genomes.length} overruns decompressed size ${decompressed.length}`
      );
    }
    try {
      genomes.push(JSON.parse(decompressed.toString('utf8', offset, offset + length)));
    } catch (error) {
      throw new SnapshotLoadError(
        snapshotId,
        `legacy genome ${genomes.length} JSON is invalid`,
        error
      );
    }
    offset += length;
    if (genomes.length > MAX_POPULATION_COUNT) {
      throw new SnapshotLoadError(
        snapshotId,
        `legacy population exceeds ${MAX_POPULATION_COUNT} genomes`
      );
    }
  }
  if (expectedPopulation !== null && genomes.length !== expectedPopulation) {
    throw new SnapshotLoadError(
      snapshotId,
      `legacy population count ${genomes.length} does not match expected ${expectedPopulation}`
    );
  }
  return genomes;
}

/**
 * Convert a validated JSON genome to a typed compatibility record.
 * @param value - JSON genome.
 * @param slot - Dense slot assigned by array order.
 * @returns Typed genome.
 */
function legacyGenomeToTyped(value: unknown, slot: number): TypedGenomeSnapshot {
  if (!isRecord(value)) throw new Error(`legacy genome ${slot} must be an object`);
  const archKey = value['archKey'];
  const brainType = value['brainType'];
  const fitness = value['fitness'];
  const weights = value['weights'];
  if (typeof archKey !== 'string' || !archKey.trim()) {
    throw new Error(`legacy genome ${slot} architecture key is invalid`);
  }
  if (brainType !== undefined && typeof brainType !== 'string') {
    throw new Error(`legacy genome ${slot} brain type is invalid`);
  }
  if (fitness !== undefined && (typeof fitness !== 'number' || !Number.isFinite(fitness))) {
    throw new Error(`legacy genome ${slot} fitness is invalid`);
  }
  if (!Array.isArray(weights) || weights.length < 1 || weights.length > MAX_GENOME_WEIGHTS) {
    throw new Error(`legacy genome ${slot} weight count is invalid`);
  }
  const typed = new Float32Array(weights.length);
  for (let index = 0; index < weights.length; index++) {
    const weight = weights[index];
    if (typeof weight !== 'number' || !Number.isFinite(weight)) {
      throw new Error(`legacy genome ${slot} weight ${index} is invalid`);
    }
    typed[index] = weight;
  }
  return {
    slot,
    archKey,
    brainType: typeof brainType === 'string' && brainType ? brainType : 'mlp',
    fitness: typeof fitness === 'number' ? fitness : 0,
    weights: typed
  };
}

/**
 * Parse and cross-check one current parent row without loading child BLOBs.
 * @param row - Current parent row.
 * @returns Validated metadata and graph-implied weight count.
 */
function parseCurrentCheckpointMetadata(row: SnapshotRow): {
  metadata: PopulationCheckpointMetadata;
  expectedWeightCount: number;
} {
  if (!row.payload_json) throw new SnapshotLoadError(row.id, 'metadata JSON is missing');
  const metadataBytes = Buffer.byteLength(row.payload_json, 'utf8');
  if (metadataBytes > MAX_METADATA_BYTES) {
    throw new SnapshotLoadError(row.id, `metadata size ${metadataBytes} exceeds ${MAX_METADATA_BYTES}`);
  }
  let metadata: PopulationCheckpointMetadata;
  let expectedWeightCount: number;
  try {
    metadata = JSON.parse(row.payload_json) as PopulationCheckpointMetadata;
    expectedWeightCount = validateCheckpointMetadata(metadata);
  } catch (error) {
    throw new SnapshotLoadError(
      row.id,
      error instanceof Error ? error.message : String(error),
      error
    );
  }
  if (row.format_version !== SNAPSHOT_FORMAT_VERSION) {
    throw new SnapshotLoadError(row.id, `parent format column is ${String(row.format_version)}`);
  }
  if (row.boundary_kind !== metadata.boundaryKind) {
    throw new SnapshotLoadError(row.id, 'parent boundary column does not match metadata');
  }
  if (row.population_count !== metadata.populationCount) {
    throw new SnapshotLoadError(row.id, 'parent population count does not match metadata');
  }
  if (row.genomes_blob !== null) {
    throw new SnapshotLoadError(row.id, 'current snapshot unexpectedly contains a legacy blob');
  }
  return { metadata, expectedWeightCount };
}

/**
 * Parse one current parent row and all strict child rows.
 * @param row - Current parent row.
 * @param selectGenomes - Prepared child-row query.
 * @returns Loaded current checkpoint.
 */
function loadCurrentCheckpoint(
  row: SnapshotRow,
  selectGenomes: ReturnType<DbType['prepare']>
): LoadedPopulationCheckpoint {
  const { metadata, expectedWeightCount } = parseCurrentCheckpointMetadata(row);
  const rows = selectGenomes.all(row.id) as GenomeRow[];
  if (rows.length !== metadata.populationCount) {
    throw new SnapshotLoadError(
      row.id,
      `child genome count ${rows.length} does not match ${metadata.populationCount}`
    );
  }
  const genomes = rows.map((genomeRow, slot) =>
    decodeGenomeRow(genomeRow, row.id, slot, metadata, expectedWeightCount));
  return {
    id: row.id,
    createdAt: row.created_at,
    compatibility: 'current',
    metadata,
    genomes
  };
}

/**
 * Parse one read-only legacy parent row and its optional combined blob.
 * @param row - Legacy parent row.
 * @returns Loaded compatibility checkpoint.
 */
function loadLegacyCheckpoint(row: SnapshotRow): LoadedLegacyCheckpoint {
  if (!row.payload_json) throw new SnapshotLoadError(row.id, 'legacy payload JSON is missing');
  if (Buffer.byteLength(row.payload_json, 'utf8') > MAX_LEGACY_UNCOMPRESSED_BYTES) {
    throw new SnapshotLoadError(row.id, 'legacy payload JSON exceeds the compatibility limit');
  }
  let payload: PopulationSnapshotPayload;
  try {
    payload = JSON.parse(row.payload_json) as PopulationSnapshotPayload;
  } catch (error) {
    throw new SnapshotLoadError(row.id, 'legacy payload JSON is invalid', error);
  }
  const expectedPopulation = Number.isSafeInteger(row.population_count)
    ? row.population_count
    : (
      payload.settings && Number.isSafeInteger(payload.settings.snakeCount)
        ? payload.settings.snakeCount
        : null
    );
  if (row.genomes_blob) {
    payload.genomes = deserializeLegacyGenomes(
      row.id,
      row.genomes_blob,
      expectedPopulation
    ) as GenomeJSON[];
  }
  const settings = parseOptionalJson<PopulationSnapshotPayload['settings']>(row.settings_json);
  const updates = parseOptionalJson<PopulationSnapshotPayload['updates']>(row.updates_json);
  if (settings) payload.settings = settings;
  if (updates) payload.updates = updates;
  validateSnapshotPayload(payload);
  let genomes: TypedGenomeSnapshot[];
  try {
    genomes = payload.genomes.map((genome, slot) => legacyGenomeToTyped(genome, slot));
  } catch (error) {
    throw new SnapshotLoadError(
      row.id,
      error instanceof Error ? error.message : String(error),
      error
    );
  }
  console.warn('[persistence] loading read-only legacy snapshot', {
    snapshotId: row.id,
    compressedBytes: row.genomes_blob?.byteLength ?? 0,
    populationCount: genomes.length,
    limitation: 'legacy load may allocate the combined population; new writes never use this format'
  });
  return {
    id: row.id,
    createdAt: row.created_at,
    compatibility: 'legacy',
    payload,
    genomes
  };
}

/**
 * Convert a loaded current checkpoint to the established JSON transfer shape.
 * @param checkpoint - Strict current checkpoint.
 * @returns JSON-compatible payload.
 */
function currentCheckpointToPayload(
  checkpoint: LoadedPopulationCheckpoint
): PopulationSnapshotPayload {
  const metadata = checkpoint.metadata;
  return {
    generation: metadata.generation,
    archKey: metadata.archKey,
    genomes: checkpoint.genomes.map(typedGenomeToJson),
    cfgHash: metadata.configHash,
    worldSeed: metadata.worldSeed,
    settings: metadata.settings,
    updates: metadata.updates,
    formatVersion: metadata.formatVersion,
    runId: metadata.runId,
    configRevision: metadata.configRevision,
    graphSpec: metadata.graphSpec,
    boundary: {
      version: metadata.boundaryVersion,
      kind: metadata.boundaryKind,
      simulationStep: metadata.simulationStep,
      resumable: metadata.resumable
    }
  };
}

/**
 * Initialize the SQLite database, migrations, and foreign-key enforcement.
 * @param dbPath - Path to the sqlite database file.
 * @returns Database handle.
 */
export function initDb(dbPath: string): DbType {
  if (dbPath !== ':memory:') {
    const dir = path.dirname(dbPath);
    fs.mkdirSync(dir, { recursive: true });
  }
  const db = new Database(dbPath);
  enableForeignKeys(db);
  db.pragma('journal_mode = WAL');
  db.pragma('synchronous = NORMAL');
  db.exec(SCHEMA_SQL);
  ensureSnapshotColumns(db);
  db.exec(`
    CREATE INDEX IF NOT EXISTS idx_snap_resume
      ON population_snapshots(format_version, boundary_kind, id)
  `);
  db.exec(`
    CREATE TABLE IF NOT EXISTS snapshot_genomes (
      snapshot_id INTEGER NOT NULL,
      slot INTEGER NOT NULL,
      arch_key TEXT NOT NULL,
      brain_type TEXT NOT NULL,
      fitness REAL NOT NULL,
      weight_count INTEGER NOT NULL,
      weights_blob BLOB NOT NULL,
      weights_checksum TEXT NOT NULL,
      PRIMARY KEY (snapshot_id, slot),
      FOREIGN KEY (snapshot_id) REFERENCES population_snapshots(id)
        ON DELETE CASCADE
    )
  `);
  enableForeignKeys(db);
  return db;
}

/**
 * Create persistence helpers backed by one SQLite database.
 * @param db - Database handle.
 * @returns Persistence API surface.
 */
export function createPersistence(db: DbType): Persistence {
  enableForeignKeys(db);
  const insertHof = db.prepare(
    `INSERT OR IGNORE INTO hof_entries (created_at, gen, seed, fitness, points, length, genome_json)
     VALUES (@created_at, @gen, @seed, @fitness, @points, @length, @genome_json)`
  );
  const insertSnapshot = db.prepare(
    `INSERT INTO population_snapshots (
       created_at, gen, payload_json, settings_json, updates_json, genomes_blob,
       format_version, boundary_kind, population_count
     ) VALUES (
       @created_at, @gen, @payload_json, NULL, NULL, NULL,
       @format_version, @boundary_kind, @population_count
     )`
  );
  const insertGenome = db.prepare(
    `INSERT INTO snapshot_genomes (
       snapshot_id, slot, arch_key, brain_type, fitness,
       weight_count, weights_blob, weights_checksum
     ) VALUES (
       @snapshot_id, @slot, @arch_key, @brain_type, @fitness,
       @weight_count, @weights_blob, @weights_checksum
     )`
  );
  const selectLatest = db.prepare(
    `SELECT id, created_at, gen, payload_json, settings_json, updates_json,
            genomes_blob, format_version, boundary_kind, population_count
       FROM population_snapshots ORDER BY id DESC LIMIT 1`
  );
  const selectLatestResumeCandidate = db.prepare(
    `SELECT id, created_at, gen, payload_json, settings_json, updates_json,
            genomes_blob, format_version, boundary_kind, population_count
       FROM population_snapshots
      WHERE boundary_kind IS NULL OR boundary_kind <> 'population-export'
      ORDER BY id DESC LIMIT 1`
  );
  const selectSnapshot = db.prepare(
    `SELECT id, created_at, gen, payload_json, settings_json, updates_json,
            genomes_blob, format_version, boundary_kind, population_count
       FROM population_snapshots WHERE id = ?`
  );
  const selectGenomes = db.prepare(
    `SELECT slot, arch_key, brain_type, fitness, weight_count,
            weights_blob, weights_checksum
       FROM snapshot_genomes WHERE snapshot_id = ? ORDER BY slot ASC`
  );
  const listSnapshotStmt = db.prepare(
    `SELECT id, created_at, gen, format_version, boundary_kind
       FROM population_snapshots ORDER BY id DESC LIMIT ?`
  );
  const listResumeCandidateStmt = db.prepare(
    `SELECT id, created_at, gen, payload_json, settings_json, updates_json,
            genomes_blob, format_version, boundary_kind, population_count
       FROM population_snapshots
      WHERE (boundary_kind IS NULL OR boundary_kind <> 'population-export')
        AND (? IS NULL OR id <> ?)
      ORDER BY id DESC LIMIT ?`
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
    `SELECT gen, seed, fitness, points, length, genome_json
       FROM hof_entries ORDER BY fitness DESC LIMIT ?`
  );

  /** Persist a Hall-of-Fame entry. */
  const saveHofEntry = (entry: HallOfFameEntry): void => {
    if (!entry || !Number.isFinite(entry.gen) || !Number.isFinite(entry.fitness)) return;
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

  /** Persist multiple Hall-of-Fame entries in a transaction. */
  const saveHofEntries = db.transaction((entries: HallOfFameEntry[]) => {
    for (const entry of entries) saveHofEntry(entry);
  });

  /** Load top Hall-of-Fame entries. */
  const loadHofEntries = (limit: number): HallOfFameEntry[] => {
    const safeLimit = Math.max(1, Math.min(1000, Math.floor(limit)));
    const rows = listHofStmt.all(safeLimit) as Array<{
      gen: number;
      seed: number;
      fitness: number;
      points: number;
      length: number;
      genome_json: string;
    }>;
    return rows.map((row) => ({
      gen: row.gen,
      seed: row.seed,
      fitness: row.fitness,
      points: row.points,
      length: row.length,
      genome: JSON.parse(row.genome_json)
    }));
  };

  /** Transactional bounded-memory current-format save. */
  const saveCheckpointTransaction = db.transaction((checkpoint: PopulationCheckpoint): number => {
    const expectedWeightCount = validateCheckpointMetadata(checkpoint.metadata);
    const weightScratch = Buffer.allocUnsafe(
      expectedWeightCount * Float32Array.BYTES_PER_ELEMENT
    );
    const metadataJson = JSON.stringify(checkpoint.metadata);
    const metadataBytes = Buffer.byteLength(metadataJson, 'utf8');
    if (metadataBytes > MAX_METADATA_BYTES) {
      throw new Error(`snapshot metadata too large (${metadataBytes} bytes)`);
    }
    const parent = insertSnapshot.run({
      created_at: Date.now(),
      gen: checkpoint.metadata.generation,
      payload_json: metadataJson,
      format_version: checkpoint.metadata.formatVersion,
      boundary_kind: checkpoint.metadata.boundaryKind,
      population_count: checkpoint.metadata.populationCount
    });
    const snapshotId = Number(parent.lastInsertRowid);
    let slot = 0;
    for (const genome of checkpoint.genomes) {
      const encoded = validateAndEncodeGenome(
        genome,
        slot,
        checkpoint.metadata,
        expectedWeightCount,
        weightScratch
      );
      insertGenome.run({
        snapshot_id: snapshotId,
        slot,
        arch_key: genome.archKey,
        brain_type: genome.brainType,
        fitness: genome.fitness,
        weight_count: genome.weights.length,
        weights_blob: encoded.bytes,
        weights_checksum: encoded.checksum
      });
      slot += 1;
    }
    if (slot !== checkpoint.metadata.populationCount) {
      throw new Error(
        `checkpoint yielded ${slot} genomes; expected ${checkpoint.metadata.populationCount}`
      );
    }
    return snapshotId;
  });

  /** Persist a current-format checkpoint. */
  const saveCheckpoint = (checkpoint: PopulationCheckpoint): number =>
    saveCheckpointTransaction(checkpoint);

  /** Parse one selected row according to its format marker. */
  const loadSelectedRow = (row: SnapshotRow): LoadedResumeSnapshot => {
    if (row.format_version === null || row.format_version === 0) {
      return loadLegacyCheckpoint(row);
    }
    if (row.format_version !== SNAPSHOT_FORMAT_VERSION) {
      throw new SnapshotLoadError(row.id, `unsupported format version ${row.format_version}`);
    }
    return loadCurrentCheckpoint(row, selectGenomes);
  };

  /** Select latest or explicit resumable snapshot. */
  const loadResumeSnapshot = (selection: 'latest' | number): LoadedResumeSnapshot | null => {
    const row = selection === 'latest'
      ? selectLatestResumeCandidate.get()
      : selectSnapshot.get(selection);
    if (!row) {
      if (selection === 'latest') return null;
      throw new SnapshotLoadError(selection, 'snapshot not found');
    }
    const loaded = loadSelectedRow(row as SnapshotRow);
    if (loaded.compatibility === 'current' && !loaded.metadata.resumable) {
      throw new SnapshotLoadError(loaded.id, 'snapshot is a population export, not resumable');
    }
    return loaded;
  };

  /** List older valid resume alternatives, excluding corrupt rows. */
  const listValidResumeSnapshots = (limit: number, excludeId?: number): SnapshotMeta[] => {
    const safeLimit = Math.max(1, Math.min(100, Math.floor(limit)));
    const scanLimit = Math.min(500, safeLimit * 5);
    const exclude = Number.isSafeInteger(excludeId) ? excludeId! : null;
    const rows = listResumeCandidateStmt.all(exclude, exclude, scanLimit) as SnapshotRow[];
    const valid: SnapshotMeta[] = [];
    for (const row of rows) {
      try {
        const loaded = loadSelectedRow(row);
        if (loaded.compatibility === 'current' && !loaded.metadata.resumable) continue;
        valid.push({
          id: row.id,
          createdAt: row.created_at,
          gen: row.gen,
          formatVersion: loaded.compatibility === 'current' ? SNAPSHOT_FORMAT_VERSION : 0,
          boundaryKind: loaded.compatibility === 'current'
            ? loaded.metadata.boundaryKind
            : 'legacy',
          resumable: true
        });
        if (valid.length >= safeLimit) break;
      } catch {
        // Invalid alternatives are intentionally omitted from the actionable list.
      }
    }
    return valid;
  };

  /** List snapshot metadata in descending order. */
  const listSnapshots = (limit: number): SnapshotMeta[] => {
    const safeLimit = Math.max(1, Math.min(1000, Math.floor(limit)));
    const rows = listSnapshotStmt.all(safeLimit) as Array<{
      id: number;
      created_at: number;
      gen: number;
      format_version: number | null;
      boundary_kind: string | null;
    }>;
    return rows.map((row) => {
      const current = row.format_version === SNAPSHOT_FORMAT_VERSION;
      const boundaryKind = current && [
        'run-start',
        'generation',
        'population-export'
      ].includes(row.boundary_kind ?? '')
        ? row.boundary_kind as SnapshotMeta['boundaryKind']
        : 'legacy';
      return {
        id: row.id,
        createdAt: row.created_at,
        gen: row.gen,
        formatVersion: current ? SNAPSHOT_FORMAT_VERSION : 0,
        boundaryKind,
        resumable: boundaryKind !== 'population-export'
      };
    });
  };

  /** Load one snapshot in JSON-compatible transport form. */
  const exportSnapshot = (id: number): PopulationSnapshotPayload => {
    const row = selectSnapshot.get(id) as SnapshotRow | undefined;
    if (!row) throw new SnapshotLoadError(id, 'snapshot not found');
    const loaded = loadSelectedRow(row);
    return loaded.compatibility === 'current'
      ? currentCheckpointToPayload(loaded)
      : loaded.payload;
  };

  /** Load latest snapshot in JSON-compatible transport form. */
  const loadLatestSnapshot = (): PopulationSnapshotPayload | null => {
    const row = selectLatest.get() as SnapshotRow | undefined;
    if (!row) return null;
    const loaded = loadSelectedRow(row);
    return loaded.compatibility === 'current'
      ? currentCheckpointToPayload(loaded)
      : loaded.payload;
  };

  /** Return newest snapshot id. */
  const getLatestSnapshotId = (): number | null => {
    const row = selectLatest.get() as SnapshotRow | undefined;
    return row?.id ?? null;
  };

  /** Yield one JSON export without a population-sized string. */
  function* exportSnapshotJsonChunks(id: number): Iterable<string> {
    const row = selectSnapshot.get(id) as SnapshotRow | undefined;
    if (!row) throw new SnapshotLoadError(id, 'snapshot not found');
    if (row.format_version === SNAPSHOT_FORMAT_VERSION) {
      const { metadata, expectedWeightCount } = parseCurrentCheckpointMetadata(row);
      const prefix = JSON.stringify({
        generation: metadata.generation,
        archKey: metadata.archKey
      });
      yield `${prefix.slice(0, -1)},"genomes":[`;
      let slot = 0;
      for (const rawRow of selectGenomes.iterate(row.id)) {
        const genome = decodeGenomeRow(
          rawRow as GenomeRow,
          row.id,
          slot,
          metadata,
          expectedWeightCount
        );
        if (slot > 0) yield ',';
        yield JSON.stringify(typedGenomeToJson(genome));
        slot += 1;
      }
      if (slot !== metadata.populationCount) {
        throw new SnapshotLoadError(
          row.id,
          `child genome count ${slot} does not match ${metadata.populationCount}`
        );
      }
      const suffix = JSON.stringify({
        cfgHash: metadata.configHash,
        worldSeed: metadata.worldSeed,
        settings: metadata.settings,
        updates: metadata.updates,
        formatVersion: metadata.formatVersion,
        runId: metadata.runId,
        configRevision: metadata.configRevision,
        graphSpec: metadata.graphSpec,
        boundary: {
          version: metadata.boundaryVersion,
          kind: metadata.boundaryKind,
          simulationStep: metadata.simulationStep,
          resumable: metadata.resumable
        }
      });
      yield `],${suffix.slice(1)}`;
      return;
    }
    if (row.format_version !== null && row.format_version !== 0) {
      throw new SnapshotLoadError(row.id, `unsupported format version ${row.format_version}`);
    }
    const loaded = loadLegacyCheckpoint(row);
    const prefix = JSON.stringify({
      generation: loaded.payload.generation,
      archKey: loaded.payload.archKey
    });
    yield `${prefix.slice(0, -1)},"genomes":[`;
    for (let slot = 0; slot < loaded.genomes.length; slot++) {
      if (slot > 0) yield ',';
      yield JSON.stringify(typedGenomeToJson(loaded.genomes[slot]!));
    }
    const { generation: _generation, archKey: _archKey, genomes: _genomes, ...rest } =
      loaded.payload;
    const suffix = JSON.stringify(rest);
    yield `],${suffix.slice(1)}`;
  }

  /** Persist a graph preset and return its id. */
  const saveGraphPreset = (name: string, spec: GraphSpec): number => {
    const trimmed = name.trim();
    if (!trimmed) throw new Error('preset name is required');
    const result = validateGraph(spec);
    if (!result.ok) throw new Error(`invalid graph spec: ${result.reason}`);
    const json = JSON.stringify(spec);
    const bytes = Buffer.byteLength(json, 'utf8');
    if (bytes > MAX_PRESET_BYTES) throw new Error(`preset too large (${bytes} bytes)`);
    const info = insertGraphPreset.run({
      created_at: Date.now(),
      name: trimmed,
      spec_json: json
    });
    return Number(info.lastInsertRowid);
  };

  /** List graph presets in descending order. */
  const listGraphPresets = (limit: number): GraphPresetMeta[] => {
    const safeLimit = Math.max(1, Math.min(1000, Math.floor(limit)));
    const rows = listGraphPresetsStmt.all(safeLimit) as Array<{
      id: number;
      created_at: number;
      name: string;
    }>;
    return rows.map((row) => ({
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
    if (!result.ok) throw new Error(`invalid graph preset: ${result.reason}`);
    return { id: row.id, name: row.name, createdAt: row.created_at, spec };
  };

  return {
    saveHofEntry,
    saveHofEntries,
    loadHofEntries,
    saveCheckpoint,
    loadResumeSnapshot,
    listValidResumeSnapshots,
    loadLatestSnapshot,
    listSnapshots,
    exportSnapshot,
    getLatestSnapshotId,
    exportSnapshotJsonChunks,
    saveGraphPreset,
    listGraphPresets,
    loadGraphPreset
  };
}

/**
 * Validate a JSON population-transfer payload accepted by the HTTP import path.
 * @param payload - Raw payload to validate.
 * @throws Error when payload is invalid.
 */
export function validateSnapshotPayload(
  payload: unknown
): asserts payload is PopulationSnapshotPayload {
  if (!isRecord(payload)) throw new Error('snapshot payload must be an object');
  const data = payload as unknown as PopulationSnapshotPayload;
  if (!Number.isFinite(data.generation)) throw new Error('snapshot generation is invalid');
  if (typeof data.archKey !== 'string' || !data.archKey.trim()) {
    throw new Error('snapshot archKey is invalid');
  }
  if (!Array.isArray(data.genomes) || data.genomes.length === 0) {
    throw new Error('snapshot genomes missing');
  }
  if (data.genomes.length > MAX_POPULATION_COUNT) {
    throw new Error(`snapshot population exceeds ${MAX_POPULATION_COUNT}`);
  }
  if (typeof data.cfgHash !== 'string' || !data.cfgHash.trim()) {
    throw new Error('snapshot cfgHash missing');
  }
  if (!Number.isFinite(data.worldSeed)) throw new Error('snapshot worldSeed missing');
  for (let slot = 0; slot < data.genomes.length; slot++) {
    legacyGenomeToTyped(data.genomes[slot], slot);
  }
}
