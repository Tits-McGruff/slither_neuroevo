import { parseRecoveryScanCursor, type RecoveryScanCursor, type RecoveryScanResult, parseRecoveryBranchCommit, parseRecoveryBranchResult, type RecoveryBranchCommit, type RecoveryBranchResult } from './recoveryProtocol.ts';
import { closeSync, fsyncSync, lstatSync, openSync, realpathSync, renameSync, statSync, unlinkSync, writeSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { dirname, resolve, sep } from 'node:path';
import { parentPort, workerData } from 'node:worker_threads';
import Database from 'better-sqlite3';
import {
  buildCheckpointRetentionInventory,
  OWNER_CHECKPOINT_RETENTION_DEFAULTS,
  selectManagedCheckpointRetention,
  type CheckpointRetentionCandidate,
  type CheckpointRetentionDecision,
  type CheckpointRetentionInventory
} from './checkpointRetention.ts';
import {
  parseManagedCheckpointDescriptor,
  parseManagedCheckpointDescriptorLimits,
  parseManagedHallOfFameWeightsDescriptor,
  parseManagedGenerationCommit,
  type CheckpointOperationId,
  type CheckpointPersistenceWorkerResponse,
  type ManagedCheckpointDescriptor,
  type ManagedCheckpointSelection,
  type ManagedCheckpointDescriptorLimits,
  type ManagedGenerationCommit,
  type ManagedGenerationSummary,
  type ManagedExportInventoryDescriptor,
  type ManagedHallOfFameWeightsDescriptor,
  type ManagedHallOfFameReference,
  type U64Hex
} from './checkpointPersistenceProtocol.ts';

/** Maximum text length returned to the client for any worker rejection. */
const MAX_REJECTION_REASON_BYTES = 1024;
/** Fixed inventory header bytes: 16-byte magic plus two little-endian u64 counts. */
const EXPORT_INVENTORY_HEADER_BYTES = 32;
/** Fixed bytes for one Hall-of-Fame record, digest, encoding tag, padding, and counts. */
const EXPORT_INVENTORY_HALL_OF_FAME_BYTES = 120;
/** Practical hard ceiling for complete-generation records in one ordinary export. */
const MAX_EXPORT_GENERATIONS = 1_000_000n;
/** Owner-selected number of best unique unpinned Hall-of-Fame genomes. */
const MAX_UNPINNED_HALL_OF_FAME_GENOMES = 50;

/** Worker bootstrap data owned by the client and structured-cloned at spawn time. */
interface CheckpointPersistenceWorkerData {
  /** Disposable or otherwise explicitly selected SQLite metadata database path. */
  databasePath: string;
  /** Prevent resume from creating or extending an unrelated database. */
  existingOnly: boolean;
  /** Existing server-controlled root containing immutable checkpoint-v3 files. */
  managedRootPath: string;
  /** Exact bounded descriptor limits selected before the worker starts. */
  limits: ManagedCheckpointDescriptorLimits;
}

/** Current pointer row needed for exact monotonic transition checks. */
interface CurrentPointerRow {
  /** Run identity stored in the current pointer. */
  pointer_run_id: string;
  /** Checkpoint identity stored in the current pointer. */
  pointer_checkpoint_id: string;
  /** Transition epoch stored in the current pointer. */
  pointer_transition_epoch: string;
  /** Operation identity stored in the current pointer. */
  pointer_operation_id: string;
  /** Run identity stored in the referenced immutable metadata. */
  metadata_run_id: string | null;
  /** Transition epoch stored in the referenced immutable metadata. */
  metadata_transition_epoch: string | null;
  /** Operation identity stored in the referenced immutable metadata. */
  metadata_operation_id: string | null;
  /** Generation of the checkpoint currently selected for the run. */
  generation_hex: string | null;
  /** Completed-step count of the checkpoint currently selected for the run. */
  completed_step_hex: string | null;
  /** Original strict descriptor stored with the referenced immutable metadata. */
  descriptor_json: string | null;
}

/** Existing immutable metadata row used for idempotent replay checks. */
interface ExistingDescriptorRow {
  /** Exact descriptor JSON stored on the original commit. */
  descriptor_json: string;
}

/** Existing compact history row used for exact replay checks. */
interface ExistingGenerationSummaryRow {
  /** Run identity stored beside the fixed record. */
  run_id: string;
  /** Completed generation identity stored beside the fixed record. */
  generation_hex: string;
  /** Compact record schema version. */
  record_version: number;
  /** Exact fixed-width summary bytes stored by the original transaction. */
  record_blob: Buffer;
}

/** Existing Hall-of-Fame reference used for exact replay checks. */
interface ExistingHallOfFameRow {
  /** Run identity stored beside the fixed record. */
  run_id: string;
  /** Completed generation identity stored beside the fixed record. */
  generation_hex: string;
  /** Compact record schema version. */
  record_version: number;
  /** Exact fixed-width reference bytes stored by the original transaction. */
  record_blob: Buffer;
  /** Content-addressed winner-weight object linked by the original transaction. */
  weights_sha256: string | null;
  /** Durable genome identity retained even when its packed weights are not selected. */
  genome_sha256: string | null;
  /** Whether packed weights are selected, intentionally omitted, or await legacy migration. */
  weight_state: string;
}

/** Existing immutable Hall-of-Fame weight object used for deduplicated replay checks. */
interface ExistingHallOfFameWeightsRow {
  /** Controlled direct-child filename. */
  relative_filename: string;
  /** Exact packed numeric encoding. */
  encoding: string;
  /** Exact stored file length. */
  stored_byte_count_hex: string;
  /** Exact decoded packed-f32 length. */
  decoded_byte_count_hex: string;
  /** Exact number of packed Float32 values. */
  weight_count_hex: string;
}

/** Minimal final-file facts rechecked after SQLite has begun the short transaction. */
interface VerifiedManagedFile {
  /** Fully resolved direct child of the controlled managed root. */
  path: string;
  /** Expected final file size from the strict descriptor. */
  expectedBytes: bigint;
}

/** Parent port required by the worker-thread-only module. */
if (!parentPort) throw new Error('checkpointPersistenceWorker requires parentPort');
/** Non-null parent port after the worker-context assertion. */
const port = parentPort;
/** Immutable bootstrap data supplied by the client. */
const bootstrap = parseWorkerData(workerData);
/** Canonical real managed root used to reject traversal and symlinks. */
const managedRootPath = resolveManagedRoot(bootstrap.managedRootPath);
/** Immutable bounded descriptor limits selected before this worker accepts messages. */
const descriptorLimits = bootstrap.limits;
/** Single synchronous SQLite connection owned exclusively by this worker. */
const db = new Database(bootstrap.databasePath, { fileMustExist: bootstrap.existingOnly });
if (bootstrap.existingOnly) {
  try { validateExistingSchema(db); }
  catch (error) { db.close(); throw error; }
}

db.pragma('journal_mode = WAL');
db.pragma('synchronous = FULL');
const journalMode = db.pragma('journal_mode', { simple: true });
const synchronous = db.pragma('synchronous', { simple: true });
if (String(journalMode).toLowerCase() !== 'wal' || Number(synchronous) !== 2) {
  throw new Error('checkpoint persistence worker requires journal_mode=WAL and synchronous=FULL');
}
db.pragma('foreign_keys = ON');
if (!bootstrap.existingOnly) initializeSchema(db);
initializeHallOfFameWeightsSchema(db);
initializeRecoverySchema(db);
initializeRetentionSchema(db);
/** One exact in-process export reference; worker shutdown cancels it implicitly. */
let activeExportLease: {
  operationId: CheckpointOperationId;
  checkpointId: string;
  inventoryPath: string;
} | undefined;
cleanupUnreferencedHallOfFameWeights();

/**
 * Parse worker bootstrap data without accepting arbitrary nested values.
 * @param value - Structured-cloned worker data.
 * @returns Validated worker data.
 */
function parseWorkerData(value: unknown): CheckpointPersistenceWorkerData {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    throw new TypeError('checkpoint persistence worker data must be an object');
  }
  const data = value as Record<string, unknown>;
  const keys = Object.keys(data);
  if (keys.length !== 4 || !Object.hasOwn(data, 'databasePath') || !Object.hasOwn(data, 'managedRootPath') ||
    !Object.hasOwn(data, 'limits') || typeof data['existingOnly'] !== 'boolean') {
    throw new TypeError('checkpoint persistence worker data has unknown or missing fields');
  }
  if (typeof data['databasePath'] !== 'string' || data['databasePath'].length === 0) {
    throw new TypeError('checkpoint persistence databasePath must be a nonempty string');
  }
  if (typeof data['managedRootPath'] !== 'string' || data['managedRootPath'].length === 0) {
    throw new TypeError('checkpoint persistence managedRootPath must be a nonempty string');
  }
  return {
    databasePath: data['databasePath'],
    existingOnly: data['existingOnly'],
    managedRootPath: data['managedRootPath'],
    limits: parseManagedCheckpointDescriptorLimits(data['limits'])
  };
}

/**
 * Resolve one existing non-symlink managed root.
 * @param candidate - Caller-supplied root path.
 * @returns Canonical managed root path.
 */
function resolveManagedRoot(candidate: string): string {
  const absolute = resolve(candidate);
  const stats = lstatSync(absolute);
  if (stats.isSymbolicLink() || !stats.isDirectory()) {
    throw new TypeError('checkpoint persistence managed root must be one real directory');
  }
  return realpathSync(absolute);
}

/** Reject unrelated or incomplete databases before changing journal settings or schema. */
function validateExistingSchema(database: ReturnType<typeof Database>): void {
  const schema = database.prepare(`SELECT count(*) AS total,
    sum(name IN ('rust_checkpoint_v3_metadata', 'rust_checkpoint_v3_current',
      'rust_generation_history_v1', 'rust_hall_of_fame_v1', 'rust_hall_of_fame_weights_v1',
      'rust_recovery_branches_v1', 'rust_active_run_v1',
      'rust_checkpoint_retention_v1')) AS recognized
    FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%'`).get() as {
      total: number; recognized: number | null;
    };
  if (![4, 5, 6, 7, 8].includes(schema.total) || schema.recognized !== schema.total) {
    throw new Error('resume requires an existing managed checkpoint metadata database');
  }
  // Preparing these fixed reads also rejects incompatible columns without DDL.
  database.prepare('SELECT checkpoint_id, operation_id, run_id, transition_epoch, generation_hex, completed_step_hex, descriptor_json FROM rust_checkpoint_v3_metadata LIMIT 0').all();
  database.prepare('SELECT run_id, checkpoint_id, transition_epoch, operation_id FROM rust_checkpoint_v3_current LIMIT 0').all();
  for (const table of ['rust_generation_history_v1', 'rust_hall_of_fame_v1']) {
    database.prepare(`SELECT run_id, generation_hex, checkpoint_id, record_version, record_blob, created_at_ms FROM ${table} LIMIT 0`).all();
  }
}

/** One bounded metadata row used only for scalar retention planning. */
interface RetentionMetadataRow {
  /** Physical immutable checkpoint identity. */
  checkpoint_id: string;
  /** Bounded original strict descriptor JSON. */
  descriptor_json: string | null;
  /** Automatic or owner-pinned classification. */
  retention_kind: string;
  /** Stable SQLite insertion order. */
  created_ordinal: number;
}

/** One current pointer used to select prior-run anchors. */
interface RetentionPointerRow {
  /** Effective run identity. */
  run_id: string;
  /** Physical immutable checkpoint identity. */
  checkpoint_id: string;
}

/** Add owner pin classification and backfill every Stage 3/6A file as automatic. */
function initializeRetentionSchema(database: ReturnType<typeof Database>): void {
  database.transaction(() => {
    const existing = database.prepare(`SELECT sql FROM sqlite_schema
      WHERE type = 'table' AND name = 'rust_checkpoint_retention_v1'`).get() as { sql: string | null } | undefined;
    if (existing && (!existing.sql?.includes("'pruning'") || !existing.sql.includes("'pruned'"))) {
      database.exec(`
        ALTER TABLE rust_checkpoint_retention_v1 RENAME TO rust_checkpoint_retention_v1_old;
        CREATE TABLE rust_checkpoint_retention_v1 (
          checkpoint_id TEXT PRIMARY KEY NOT NULL REFERENCES rust_checkpoint_v3_metadata(checkpoint_id),
          retention_kind TEXT NOT NULL CHECK(retention_kind IN ('automatic', 'pinned', 'pruning', 'pruned')),
          classified_at_ms INTEGER NOT NULL
        );
        INSERT INTO rust_checkpoint_retention_v1 SELECT * FROM rust_checkpoint_retention_v1_old;
        DROP TABLE rust_checkpoint_retention_v1_old;
      `);
    }
    database.exec(`
      CREATE TABLE IF NOT EXISTS rust_checkpoint_retention_v1 (
        checkpoint_id TEXT PRIMARY KEY NOT NULL REFERENCES rust_checkpoint_v3_metadata(checkpoint_id),
        retention_kind TEXT NOT NULL CHECK(retention_kind IN ('automatic', 'pinned', 'pruning', 'pruned')),
        classified_at_ms INTEGER NOT NULL
      );
    `);
    database.prepare(`INSERT OR IGNORE INTO rust_checkpoint_retention_v1 (
      checkpoint_id, retention_kind, classified_at_ms
    ) SELECT checkpoint_id, 'automatic', created_at_ms FROM rust_checkpoint_v3_metadata
    `).run();
  }).immediate();
}

/** Add bounded recovery provenance without rewriting any existing checkpoint/history rows. */
function initializeRecoverySchema(database: ReturnType<typeof Database>): void {
  database.transaction(() => database.exec(`
    CREATE TABLE IF NOT EXISTS rust_recovery_branches_v1 (
      branch_run_id TEXT PRIMARY KEY NOT NULL,
      operation_id TEXT UNIQUE NOT NULL,
      source_run_id TEXT NOT NULL,
      recovered_checkpoint_id TEXT NOT NULL REFERENCES rust_checkpoint_v3_metadata(checkpoint_id),
      history_through_generation_hex TEXT NOT NULL,
      provenance_json TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS rust_active_run_v1 (
      singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
      run_id TEXT NOT NULL REFERENCES rust_checkpoint_v3_current(run_id)
    );
  `)).immediate();
}

/** Read one strictly bounded branch record used to authorize an aliased source checkpoint. */
function readRecoveryBranch(runId: string): RecoveryBranchResult | undefined {
  const row = db.prepare(`SELECT CASE WHEN length(CAST(provenance_json AS BLOB)) <= 32768
    THEN provenance_json END AS provenance_json FROM rust_recovery_branches_v1 WHERE branch_run_id = ?`)
    .get(runId) as { provenance_json: string | null } | undefined;
  if (!row) return undefined;
  if (row.provenance_json === null) throw new Error('recovery provenance exceeds bounded metadata limits');
  const result = parseRecoveryBranchResult(JSON.parse(row.provenance_json));
  const historyThrough = (BigInt(`0x${result.recoveredDescriptor.generation}`) - 1n).toString(16).padStart(16, '0');
  const matching = db.prepare(`SELECT 1 FROM rust_recovery_branches_v1 WHERE branch_run_id = ?
    AND operation_id = ? AND source_run_id = ? AND recovered_checkpoint_id = ? AND history_through_generation_hex = ?`)
    .get(runId, result.operationId, result.sourceRunId, result.recoveredDescriptor.logicalRootSha256, historyThrough);
  if (result.branchRunId !== runId || !matching) throw new Error('recovery branch identity mismatch');
  return result;
}

/** One bounded ancestor and the inherited checkpoint prefix still eligible for recovery. */
interface RecoveryLineage {
  /** Run that physically owns immutable descriptor rows. */
  runId: string;
  /** Inclusive inherited boundary; null only for the failed active lineage. */
  maximumGeneration: U64Hex | null;
}

/** Resolve inherited history with decreasing cutoffs and reject corrupt cycles. */
function recoveryLineage(sourceRunId: string): RecoveryLineage[] {
  const lineage: RecoveryLineage[] = [];
  const seen = new Set<string>();
  let runId = sourceRunId;
  let maximumGeneration: U64Hex | null = null;
  for (;;) {
    if (seen.has(runId) || lineage.length === 64) throw new Error('recovery ancestry is cyclic or exceeds 64 retained branches');
    seen.add(runId);
    lineage.push({ runId, maximumGeneration });
    const branch = readRecoveryBranch(runId);
    if (!branch) return lineage;
    const boundary = branch.recoveredDescriptor.generation;
    const previous = lineage[lineage.length - 1]!.maximumGeneration;
    maximumGeneration = previous === null || boundary < previous ? boundary : previous;
    runId = branch.sourceRunId;
  }
}

/** Build a parameterized bounded prefix filter; run names never become SQL text. */
function recoveryLineageFilter(lineage: RecoveryLineage[]): { sql: string; parameters: Array<string | null> } {
  return { sql: lineage.map(() => '(run_id = ? AND (? IS NULL OR generation_hex <= ?))').join(' OR '),
    parameters: lineage.flatMap(item => [item.runId, item.maximumGeneration, item.maximumGeneration]) };
}

/** Commit lineage, source-history prefix reference, and active pointer in one FULL transaction. */
function commitRecoveryBranch(value: RecoveryBranchCommit): RecoveryBranchResult {
  const commit = parseRecoveryBranchCommit(value);
  const selected = commit.recoveredDescriptor;
  assertDescriptorBounds(selected);
  const file = verifyManagedFile(selected);
  return db.transaction(() => {
    const replay = readRecoveryBranch(commit.branchRunId);
    if (replay) {
      const { abandonedThroughGeneration: _suffix, ...original } = replay;
      const current = readCurrentPointer(commit.branchRunId);
      const active = db.prepare('SELECT run_id FROM rust_active_run_v1 WHERE singleton = 1 AND run_id = ?').get(commit.branchRunId);
      if (JSON.stringify(original) !== JSON.stringify(commit) || !current || !active ||
          current.pointer_checkpoint_id !== selected.logicalRootSha256 || current.pointer_operation_id !== commit.operationId) {
        throw new Error('recovery replay conflicts or is superseded');
      }
      validateCurrentPointerIdentity(commit.branchRunId, current);
      return replay;
    }
    if (readCurrentPointer(commit.branchRunId) || db.prepare('SELECT 1 FROM rust_checkpoint_v3_metadata WHERE run_id = ? LIMIT 1').get(commit.branchRunId)) {
      throw new Error('recovery branch run already exists');
    }
    if (db.prepare('SELECT 1 FROM rust_checkpoint_v3_metadata WHERE operation_id = ? LIMIT 1').get(commit.operationId)) {
      throw new Error('recovery operation conflicts with a checkpoint publication');
    }
    const source = readCurrentPointer(commit.sourceRunId);
    if (!source || source.pointer_checkpoint_id !== commit.failedCheckpointId) {
      throw new Error('failed source pointer changed during recovery');
    }
    const active = db.prepare('SELECT 1 FROM rust_active_run_v1 WHERE singleton = 1 AND run_id != ?').get(commit.sourceRunId);
    if (active) throw new Error('recovery source is no longer active');
    const lineage = recoveryLineage(commit.sourceRunId);
    if (!lineage.some(item => item.runId === selected.runId &&
        (item.maximumGeneration === null || selected.generation <= item.maximumGeneration))) {
      throw new Error('recovered checkpoint is outside the inherited lineage prefix');
    }
    const retained = db.prepare(`SELECT CASE WHEN length(CAST(descriptor_json AS BLOB)) <= 16384
      THEN descriptor_json END AS descriptor_json FROM rust_checkpoint_v3_metadata
      WHERE checkpoint_id = ? AND run_id = ? AND generation_hex = ? AND completed_step_hex = ?`)
      .get(selected.logicalRootSha256, selected.runId, selected.generation, selected.completedStep) as { descriptor_json: string | null } | undefined;
    if (!retained?.descriptor_json || JSON.stringify(parseManagedCheckpointDescriptor(JSON.parse(retained.descriptor_json))) !== JSON.stringify(selected)) {
      throw new Error('recovered descriptor differs from retained source metadata');
    }
    const invalidChronology = db.prepare(`SELECT 1 FROM rust_checkpoint_v3_metadata WHERE run_id = ? AND
      (length(generation_hex) != 16 OR generation_hex GLOB '*[^0-9a-f]*') LIMIT 1`).get(commit.sourceRunId);
    if (invalidChronology) throw new Error('failed lineage has invalid retained chronology');
    const newest = db.prepare('SELECT max(generation_hex) AS generation FROM rust_checkpoint_v3_metadata WHERE run_id = ?')
      .get(commit.sourceRunId) as { generation: string | null };
    const inherited = readRecoveryBranch(commit.sourceRunId)?.recoveredDescriptor.generation ?? null;
    const abandonedThroughGeneration = newest.generation === null ? inherited :
      inherited !== null && inherited > newest.generation ? inherited : newest.generation;
    const result = parseRecoveryBranchResult({ ...commit, abandonedThroughGeneration });
    const historyThrough = (BigInt(`0x${selected.generation}`) - 1n).toString(16).padStart(16, '0');
    recheckManagedFile(file);
    db.prepare(`INSERT INTO rust_recovery_branches_v1 (branch_run_id, operation_id, source_run_id,
      recovered_checkpoint_id, history_through_generation_hex, provenance_json) VALUES (?, ?, ?, ?, ?, ?)`)
      .run(commit.branchRunId, commit.operationId, commit.sourceRunId, selected.logicalRootSha256, historyThrough, JSON.stringify(result));
    db.prepare('INSERT INTO rust_checkpoint_v3_current (run_id, checkpoint_id, transition_epoch, operation_id) VALUES (?, ?, ?, ?)')
      .run(commit.branchRunId, selected.logicalRootSha256, selected.transitionEpoch, commit.operationId);
    db.prepare(`INSERT INTO rust_active_run_v1 (singleton, run_id) VALUES (1, ?)
      ON CONFLICT(singleton) DO UPDATE SET run_id = excluded.run_id`).run(commit.branchRunId);
    return result;
  }).immediate();
}

/**
 * Create the minimal checkpoint metadata, compact history, and per-run pointer tables.
 * @param database - Worker-owned synchronous SQLite connection.
 */
function initializeSchema(database: ReturnType<typeof Database>): void {
  database.exec(`
    CREATE TABLE IF NOT EXISTS rust_checkpoint_v3_metadata (
      checkpoint_id TEXT PRIMARY KEY NOT NULL,
      operation_id TEXT NOT NULL UNIQUE,
      run_id TEXT NOT NULL,
      transition_epoch TEXT NOT NULL,
      generation_hex TEXT NOT NULL,
      completed_step_hex TEXT NOT NULL,
      boundary_kind TEXT NOT NULL,
      checkpoint_format_version_hex TEXT NOT NULL,
      state_version_hex TEXT NOT NULL,
      graph_layout_version_hex TEXT NOT NULL,
      managed_root TEXT NOT NULL,
      relative_filename TEXT NOT NULL UNIQUE,
      logical_root_sha256 TEXT NOT NULL UNIQUE,
      stored_byte_count_hex TEXT NOT NULL,
      decoded_byte_count_hex TEXT NOT NULL,
      role_count_hex TEXT NOT NULL,
      population_count_hex TEXT NOT NULL,
      weight_count_hex TEXT NOT NULL,
      recurrent_state_count_hex TEXT NOT NULL,
      weights_encoding TEXT NOT NULL,
      recurrent_state_encoding TEXT NOT NULL,
      graph_layout_sha256 TEXT NOT NULL,
      write_validation_policy TEXT NOT NULL,
      descriptor_json TEXT NOT NULL,
      created_at_ms INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS rust_checkpoint_v3_current (
      run_id TEXT PRIMARY KEY NOT NULL,
      checkpoint_id TEXT NOT NULL REFERENCES rust_checkpoint_v3_metadata(checkpoint_id),
      transition_epoch TEXT NOT NULL,
      operation_id TEXT NOT NULL UNIQUE
    );
    CREATE TABLE IF NOT EXISTS rust_generation_history_v1 (
      run_id TEXT NOT NULL,
      generation_hex TEXT NOT NULL,
      checkpoint_id TEXT NOT NULL UNIQUE REFERENCES rust_checkpoint_v3_metadata(checkpoint_id),
      record_version INTEGER NOT NULL,
      record_blob BLOB NOT NULL CHECK(length(record_blob) = 56),
      created_at_ms INTEGER NOT NULL,
      PRIMARY KEY (run_id, generation_hex)
    );
    CREATE TABLE IF NOT EXISTS rust_hall_of_fame_weights_v1 (
      logical_sha256 TEXT PRIMARY KEY NOT NULL,
      relative_filename TEXT NOT NULL UNIQUE,
      encoding TEXT NOT NULL,
      stored_byte_count_hex TEXT NOT NULL,
      decoded_byte_count_hex TEXT NOT NULL,
      weight_count_hex TEXT NOT NULL,
      created_at_ms INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS rust_hall_of_fame_v1 (
      run_id TEXT NOT NULL,
      generation_hex TEXT NOT NULL,
      checkpoint_id TEXT NOT NULL UNIQUE REFERENCES rust_checkpoint_v3_metadata(checkpoint_id),
      record_version INTEGER NOT NULL,
      record_blob BLOB NOT NULL CHECK(length(record_blob) = 56),
      weights_sha256 TEXT REFERENCES rust_hall_of_fame_weights_v1(logical_sha256),
      genome_sha256 TEXT,
      fitness_value REAL NOT NULL,
      pinned INTEGER NOT NULL DEFAULT 0 CHECK(pinned IN (0, 1)),
      weight_state TEXT NOT NULL CHECK(weight_state IN ('selected', 'unselected', 'legacy')),
      created_at_ms INTEGER NOT NULL,
      PRIMARY KEY (run_id, generation_hex)
    );
  `);
}

/** Add the content-addressed winner-weight store without rewriting older Hall-of-Fame rows. */
function initializeHallOfFameWeightsSchema(database: ReturnType<typeof Database>): void {
  database.transaction(() => {
    database.exec(`
      CREATE TABLE IF NOT EXISTS rust_hall_of_fame_weights_v1 (
        logical_sha256 TEXT PRIMARY KEY NOT NULL,
        relative_filename TEXT NOT NULL UNIQUE,
        encoding TEXT NOT NULL,
        stored_byte_count_hex TEXT NOT NULL,
        decoded_byte_count_hex TEXT NOT NULL,
        weight_count_hex TEXT NOT NULL,
        created_at_ms INTEGER NOT NULL
      )
    `);
    let columns = database.prepare('PRAGMA table_info(rust_hall_of_fame_v1)').all() as Array<{ name: string }>;
    if (!columns.some(column => column.name === 'weights_sha256')) {
      database.exec(`ALTER TABLE rust_hall_of_fame_v1 ADD COLUMN weights_sha256 TEXT
        REFERENCES rust_hall_of_fame_weights_v1(logical_sha256)`);
    }
    if (!columns.some(column => column.name === 'fitness_value')) {
      database.exec('ALTER TABLE rust_hall_of_fame_v1 ADD COLUMN fitness_value REAL');
    }
    if (!columns.some(column => column.name === 'genome_sha256')) {
      database.exec('ALTER TABLE rust_hall_of_fame_v1 ADD COLUMN genome_sha256 TEXT');
    }
    if (!columns.some(column => column.name === 'pinned')) {
      database.exec(`ALTER TABLE rust_hall_of_fame_v1 ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0
        CHECK(pinned IN (0, 1))`);
    }
    if (!columns.some(column => column.name === 'weight_state')) {
      database.exec(`ALTER TABLE rust_hall_of_fame_v1 ADD COLUMN weight_state TEXT
        CHECK(weight_state IN ('selected', 'unselected', 'legacy'))`);
    }
    columns = database.prepare('PRAGMA table_info(rust_hall_of_fame_v1)').all() as Array<{ name: string }>;
    if (!['weights_sha256', 'genome_sha256', 'fitness_value', 'pinned', 'weight_state']
      .every(name => columns.some(column => column.name === name))) {
      throw new Error('Hall-of-Fame schema migration did not create every retention field');
    }
    const rows = database.prepare(`SELECT run_id, generation_hex, record_blob, weights_sha256,
      genome_sha256, fitness_value, weight_state FROM rust_hall_of_fame_v1`).all() as Array<{
        run_id: string; generation_hex: string; record_blob: Buffer; weights_sha256: string | null;
        genome_sha256: string | null; fitness_value: number | null; weight_state: string | null;
      }>;
    const update = database.prepare(`UPDATE rust_hall_of_fame_v1
      SET genome_sha256 = ?, fitness_value = ?, weight_state = ?
      WHERE run_id = ? AND generation_hex = ?`);
    for (const row of rows) {
      if (!Buffer.isBuffer(row.record_blob) || row.record_blob.length !== 56) {
        throw new Error('Hall-of-Fame migration found a malformed compact record');
      }
      const fitness = row.record_blob.readDoubleLE(32);
      if (!Number.isFinite(fitness)) throw new Error('Hall-of-Fame migration found non-finite fitness');
      const weightState = row.weight_state ?? (row.weights_sha256 === null ? 'legacy' : 'selected');
      if (!['selected', 'unselected', 'legacy'].includes(weightState)) {
        throw new Error('Hall-of-Fame migration found an invalid weight state');
      }
      const genomeSha256 = row.genome_sha256 ?? row.weights_sha256;
      if (weightState !== 'legacy' && genomeSha256 === null) {
        throw new Error('Hall-of-Fame migration found a retained row without genome identity');
      }
      if (row.genome_sha256 !== genomeSha256 || row.fitness_value !== fitness ||
          row.weight_state !== weightState) {
        update.run(genomeSha256, fitness, weightState, row.run_id, row.generation_hex);
      }
    }
    const runs = database.prepare('SELECT DISTINCT run_id FROM rust_hall_of_fame_v1')
      .all() as Array<{ run_id: string }>;
    for (const run of runs) applyHallOfFameRetention(run.run_id);
  }).immediate();
}

/**
 * Convert one exact hexadecimal descriptor value to bigint for non-lexicographic comparisons.
 * @param value - Canonical fixed-width unsigned-64-bit hexadecimal value.
 * @returns Exact bigint value.
 */
function u64HexToBigInt(value: U64Hex): bigint {
  return BigInt(`0x${value}`);
}

/** Decode one validated network-order Float64 bit string for SQLite ranking. */
function f64HexToNumber(value: string): number {
  const bytes = Buffer.from(value, 'hex');
  if (bytes.length !== 8) throw new TypeError('invalid Float64 bit string');
  const decoded = bytes.readDoubleBE(0);
  if (!Number.isFinite(decoded)) throw new RangeError('Hall-of-Fame fitness must be finite');
  return decoded;
}

/**
 * Enforce bounded descriptor facts before the short SQLite transaction starts.
 * @param descriptor - Strict descriptor supplied by the Rust/Node bridge.
 */
function assertDescriptorBounds(descriptor: ManagedCheckpointDescriptor): void {
  const storedBytes = u64HexToBigInt(descriptor.storedByteCount);
  const decodedBytes = u64HexToBigInt(descriptor.decodedByteCount);
  const maxStoredBytes = u64HexToBigInt(descriptorLimits.maxStoredByteCount);
  const maxDecodedBytes = u64HexToBigInt(descriptorLimits.maxDecodedByteCount);
  if (storedBytes <= 0n || storedBytes > maxStoredBytes) {
    throw new RangeError('storedByteCount is outside the managed checkpoint limit');
  }
  if (decodedBytes <= 0n || decodedBytes > maxDecodedBytes) {
    throw new RangeError('decodedByteCount is outside the managed checkpoint limit');
  }
  const populationCount = u64HexToBigInt(descriptor.populationCount);
  const roleCount = u64HexToBigInt(descriptor.roleCount);
  const weightCount = u64HexToBigInt(descriptor.weightCount);
  const recurrentStateCount = u64HexToBigInt(descriptor.recurrentStateCount);
  const maxPopulationCount = u64HexToBigInt(descriptorLimits.maxPopulationCount);
  const maxWeightsPerGenome = u64HexToBigInt(descriptorLimits.maxWeightsPerGenome);
  const maxRecurrentStateCount = u64HexToBigInt(descriptorLimits.maxRecurrentStateCount);
  const maxRoleCount = u64HexToBigInt(descriptorLimits.maxRoleCount);
  if (roleCount === 0n || roleCount > maxRoleCount) {
    throw new RangeError('roleCount is outside the managed checkpoint limit');
  }
  if (populationCount <= 0n || populationCount > maxPopulationCount) {
    throw new RangeError('populationCount is outside the managed checkpoint limit');
  }
  if (weightCount > populationCount * maxWeightsPerGenome) {
    throw new RangeError('weightCount is outside the managed checkpoint limit');
  }
  if (recurrentStateCount > maxRecurrentStateCount) {
    throw new RangeError('recurrentStateCount is outside the managed checkpoint limit');
  }
  for (const [label, value] of [
    ['transitionEpoch', descriptor.transitionEpoch],
    ['checkpointFormatVersion', descriptor.checkpointFormatVersion],
    ['stateVersion', descriptor.stateVersion],
    ['graphLayoutVersion', descriptor.graphLayoutVersion]
  ] as const) {
    if (u64HexToBigInt(value) === 0n) throw new RangeError(`${label} must be nonzero`);
  }
}

/**
 * Validate the exact final immutable file without reading or cloning its population bytes.
 *
 * Rust's single-pass publisher owns byte-to-logical-root validation, and Rust validates the
 * logical root again on restore/startup. This metadata worker intentionally checks only the
 * controlled path, final regular-file type, and exact stored length.
 * @param relativeFilename - Strict direct-child basename selected by Rust.
 * @param expectedBytes - Exact final stored file length.
 */
function verifyManagedDirectFile(relativeFilename: string, expectedBytes: bigint): VerifiedManagedFile {
  const candidate = resolve(managedRootPath, relativeFilename);
  const rootPrefix = managedRootPath.endsWith(sep) ? managedRootPath : `${managedRootPath}${sep}`;
  if (!candidate.startsWith(rootPrefix) || dirname(candidate) !== managedRootPath) {
    throw new TypeError('managed checkpoint filename escapes the controlled root');
  }
  const rootStats = lstatSync(managedRootPath);
  if (rootStats.isSymbolicLink() || !rootStats.isDirectory()) {
    throw new TypeError('managed checkpoint root changed or became a symlink');
  }
  const fileStats = lstatSync(candidate);
  if (fileStats.isSymbolicLink() || !fileStats.isFile()) {
    throw new TypeError('managed checkpoint must be one final regular file, never a symlink');
  }
  const realCandidate = realpathSync(candidate);
  if (dirname(realCandidate) !== managedRootPath) {
    throw new TypeError('managed checkpoint resolves outside the controlled root');
  }
  if (statSync(candidate, { bigint: true }).size !== expectedBytes) {
    throw new RangeError('managed file size does not match storedByteCount');
  }
  return { path: candidate, expectedBytes };
}

/** Validate one immutable checkpoint file against its strict descriptor. */
function verifyManagedFile(descriptor: ManagedCheckpointDescriptor): VerifiedManagedFile {
  return verifyManagedDirectFile(
    descriptor.relativeFilename,
    u64HexToBigInt(descriptor.storedByteCount)
  );
}

/** Validate one immutable Hall-of-Fame weight file against its strict descriptor. */
function verifyHallOfFameWeightsFile(
  descriptor: ManagedHallOfFameWeightsDescriptor
): VerifiedManagedFile {
  return verifyManagedDirectFile(
    descriptor.relativeFilename,
    u64HexToBigInt(descriptor.storedByteCount)
  );
}

/**
 * Recheck only final-file type and size after the SQLite transaction begins.
 * @param file - Previously resolved controlled managed file.
 */
function recheckManagedFile(file: VerifiedManagedFile): void {
  const stats = lstatSync(file.path, { bigint: true });
  if (stats.isSymbolicLink() || !stats.isFile() ||
    statSync(file.path, { bigint: true }).size !== file.expectedBytes) {
    throw new TypeError('managed checkpoint final file changed before metadata commit');
  }
}

/**
 * Stable descriptor serialization for exact duplicate detection in SQLite.
 * @param descriptor - Strict descriptor containing only scalar data.
 * @returns Deterministic JSON record.
 */
function serializeDescriptor(descriptor: ManagedCheckpointDescriptor): string {
  return JSON.stringify(descriptor);
}

/** Require SQLite's immutable content-object metadata to match one strict Rust descriptor. */
function assertStoredHallOfFameWeights(descriptor: ManagedHallOfFameWeightsDescriptor): void {
  const existing = db.prepare(`SELECT relative_filename, encoding, stored_byte_count_hex,
    decoded_byte_count_hex, weight_count_hex FROM rust_hall_of_fame_weights_v1
    WHERE logical_sha256 = ?`).get(descriptor.logicalSha256) as ExistingHallOfFameWeightsRow | undefined;
  if (!existing || existing.relative_filename !== descriptor.relativeFilename ||
      existing.encoding !== descriptor.encoding ||
      existing.stored_byte_count_hex !== descriptor.storedByteCount ||
      existing.decoded_byte_count_hex !== descriptor.decodedByteCount ||
      existing.weight_count_hex !== descriptor.weightCount) {
    throw new Error('Hall-of-Fame weight identity conflicts with different immutable content');
  }
}

/** Remove immutable winner objects after no compact Hall-of-Fame row retains them. */
function cleanupUnreferencedHallOfFameWeights(): void {
  if (activeExportLease) return;
  const rows = db.prepare(`SELECT logical_sha256, relative_filename, encoding,
    stored_byte_count_hex, decoded_byte_count_hex, weight_count_hex
    FROM rust_hall_of_fame_weights_v1 AS weights
    WHERE NOT EXISTS (
      SELECT 1 FROM rust_hall_of_fame_v1 AS hall
      WHERE hall.weights_sha256 = weights.logical_sha256
    ) ORDER BY weights.rowid`).all() as Array<{
      logical_sha256: string;
      relative_filename: string;
      encoding: string;
      stored_byte_count_hex: string;
      decoded_byte_count_hex: string;
      weight_count_hex: string;
    }>;
  for (const row of rows) {
    if (!/^[0-9a-f]{64}$/u.test(row.logical_sha256) ||
        row.relative_filename !== `${row.logical_sha256}.hof-weights-v1` ||
        !['raw-f32le-v1', 'f32le-shuffle4-zstd-v1'].includes(row.encoding) ||
        !/^[0-9a-f]{16}$/u.test(row.stored_byte_count_hex) ||
        !/^[0-9a-f]{16}$/u.test(row.decoded_byte_count_hex) ||
        !/^[0-9a-f]{16}$/u.test(row.weight_count_hex)) {
      throw new Error('unreferenced Hall-of-Fame object has malformed metadata');
    }
    const decodedBytes = u64HexToBigInt(row.decoded_byte_count_hex as U64Hex);
    const weightCount = u64HexToBigInt(row.weight_count_hex as U64Hex);
    const storedBytes = u64HexToBigInt(row.stored_byte_count_hex as U64Hex);
    if (decodedBytes !== weightCount * 4n ||
        (row.encoding === 'raw-f32le-v1' && storedBytes !== decodedBytes)) {
      throw new Error('unreferenced Hall-of-Fame object has inconsistent counts');
    }
    try {
      const file = verifyManagedDirectFile(row.relative_filename, storedBytes);
      unlinkSync(file.path);
    } catch (error) {
      if (!(error && typeof error === 'object' && (error as NodeJS.ErrnoException).code === 'ENOENT')) {
        continue;
      }
    }
    db.prepare(`DELETE FROM rust_hall_of_fame_weights_v1
      WHERE logical_sha256 = ? AND NOT EXISTS (
        SELECT 1 FROM rust_hall_of_fame_v1 WHERE weights_sha256 = ?
      )`).run(row.logical_sha256, row.logical_sha256);
  }
}

/**
 * Encode the owner-selected eight fields as the measured 56-byte little-endian record.
 * @param summary - Strict exact wire values from the Rust boundary.
 * @returns Fixed-width bytes suitable for exact replay comparison and later chart decoding.
 */
function encodeGenerationSummary(summary: ManagedGenerationSummary): Buffer {
  const record = Buffer.alloc(56);
  record.writeBigUInt64LE(u64HexToBigInt(summary.completedGeneration), 0);
  record.writeBigUInt64LE(BigInt(`0x${summary.bestF64Hex}`), 8);
  record.writeBigUInt64LE(BigInt(`0x${summary.averageF64Hex}`), 16);
  record.writeBigUInt64LE(BigInt(`0x${summary.minimumF64Hex}`), 24);
  record.writeUInt32LE(Number(u64HexToBigInt(summary.speciesCount)), 32);
  record.writeUInt32LE(Number(u64HexToBigInt(summary.topSpeciesSize)), 36);
  record.writeBigUInt64LE(BigInt(`0x${summary.averageWeightF64Hex}`), 40);
  record.writeBigUInt64LE(BigInt(`0x${summary.weightVarianceF64Hex}`), 48);
  return record;
}

/**
 * Encode one run-scoped Hall-of-Fame reference as a fixed 56-byte record.
 * @param reference - Strict scalar reference to the elite stored in the checkpoint.
 * @returns Fixed-width little-endian bytes with no duplicated genome weights.
 */
function encodeHallOfFameReference(reference: ManagedHallOfFameReference): Buffer {
  const record = Buffer.alloc(56);
  record.writeBigUInt64LE(u64HexToBigInt(reference.completedGeneration), 0);
  record.writeUInt32LE(Number(u64HexToBigInt(reference.sourcePopulationSlot)), 8);
  record.writeUInt32LE(Number(u64HexToBigInt(reference.successorPopulationSlot)), 12);
  record.writeBigUInt64LE(u64HexToBigInt(reference.sourceSnakeId), 16);
  record.writeBigUInt64LE(u64HexToBigInt(reference.successorGenomeId), 24);
  record.writeBigUInt64LE(BigInt(`0x${reference.fitnessF64Hex}`), 32);
  record.writeBigUInt64LE(BigInt(`0x${reference.pointsF64Hex}`), 40);
  record.writeBigUInt64LE(u64HexToBigInt(reference.length), 48);
  return record;
}

/** One packed-genome candidate considered by the bounded Hall-of-Fame policy. */
interface HallOfFameRetentionRow {
  /** Run-scoped completed generation. */
  generation_hex: string;
  /** Finite sortable fitness copied from the compact record. */
  fitness_value: number;
  /** Content identity used to collapse duplicate genomes. */
  genome_sha256: string | null;
  /** Owner pin survives the automatic best-50 limit. */
  pinned: number;
  /** Legacy rows without independent weights remain untouched. */
  weight_state: string;
}

/** Return true when the left record wins a deterministic equal-genome comparison. */
function betterHallOfFameRow(left: HallOfFameRetentionRow, right: HallOfFameRetentionRow): boolean {
  if (left.fitness_value !== right.fitness_value) return left.fitness_value > right.fitness_value;
  return left.generation_hex < right.generation_hex;
}

/** Keep packed weights only for the best 50 unique genomes plus pinned unique entries. */
function applyHallOfFameRetention(runId: string): void {
  const rows = db.prepare(`SELECT generation_hex, fitness_value, genome_sha256, pinned, weight_state
    FROM rust_hall_of_fame_v1 WHERE run_id = ? ORDER BY generation_hex`).all(runId) as HallOfFameRetentionRow[];
  const unique = new Map<string, HallOfFameRetentionRow>();
  for (const row of rows) {
    if (!Number.isFinite(row.fitness_value) || !['selected', 'unselected', 'legacy'].includes(row.weight_state) ||
        ![0, 1].includes(row.pinned)) {
      throw new Error('Hall-of-Fame retention found invalid bounded metadata');
    }
    if (row.weight_state === 'legacy') continue;
    if (row.genome_sha256 === null) throw new Error('Hall-of-Fame row is missing genome identity');
    const previous = unique.get(row.genome_sha256);
    if (!previous || (row.pinned > previous.pinned) ||
        (row.pinned === previous.pinned && betterHallOfFameRow(row, previous))) {
      unique.set(row.genome_sha256, row);
    }
  }
  const ranked = [...unique.values()].sort((left, right) => {
    if (left.fitness_value !== right.fitness_value) return right.fitness_value - left.fitness_value;
    return left.generation_hex.localeCompare(right.generation_hex);
  });
  const selected = new Set(ranked.filter(row => row.pinned === 1).map(row => row.generation_hex));
  let unpinnedSelected = 0;
  for (const row of ranked) {
    if (row.pinned === 1 || unpinnedSelected >= MAX_UNPINNED_HALL_OF_FAME_GENOMES) continue;
    selected.add(row.generation_hex);
    unpinnedSelected++;
  }
  db.prepare(`UPDATE rust_hall_of_fame_v1 SET weights_sha256 = NULL, weight_state = 'unselected'
    WHERE run_id = ? AND weight_state != 'legacy'`).run(runId);
  const select = db.prepare(`UPDATE rust_hall_of_fame_v1 SET weights_sha256 = ?, weight_state = 'selected'
    WHERE run_id = ? AND generation_hex = ? AND weight_state = 'unselected'`);
  for (const row of ranked) {
    if (!selected.has(row.generation_hex) || row.genome_sha256 === null) continue;
    if (select.run(row.genome_sha256, runId, row.generation_hex).changes !== 1) {
      throw new Error('Hall-of-Fame retention selection changed during its transaction');
    }
  }
}

/**
 * Read the complete current boundary identity for chronological publication checks.
 * @param runId - Opaque run whose pointer is being advanced.
 * @returns Current pointer plus its immutable boundary identity, when present.
 */
function readCurrentPointer(runId: string): CurrentPointerRow | undefined {
  return db.prepare(`
    SELECT
      CASE WHEN length(CAST(current.run_id AS BLOB)) <= 256 THEN current.run_id END AS pointer_run_id,
      CASE WHEN length(CAST(current.checkpoint_id AS BLOB)) <= 64 THEN current.checkpoint_id END AS pointer_checkpoint_id,
      CASE WHEN length(CAST(current.transition_epoch AS BLOB)) <= 16 THEN current.transition_epoch END AS pointer_transition_epoch,
      CASE WHEN length(CAST(current.operation_id AS BLOB)) <= 32 THEN current.operation_id END AS pointer_operation_id,
      CASE WHEN length(CAST(metadata.run_id AS BLOB)) <= 256 THEN metadata.run_id END AS metadata_run_id,
      CASE WHEN length(CAST(metadata.transition_epoch AS BLOB)) <= 16 THEN metadata.transition_epoch END AS metadata_transition_epoch,
      CASE WHEN length(CAST(metadata.operation_id AS BLOB)) <= 32 THEN metadata.operation_id END AS metadata_operation_id,
      CASE WHEN length(CAST(metadata.generation_hex AS BLOB)) <= 16 THEN metadata.generation_hex END AS generation_hex,
      CASE WHEN length(CAST(metadata.completed_step_hex AS BLOB)) <= 16 THEN metadata.completed_step_hex END AS completed_step_hex,
      CASE WHEN length(CAST(metadata.descriptor_json AS BLOB)) <= 16384
        THEN metadata.descriptor_json ELSE NULL END AS descriptor_json
    FROM rust_checkpoint_v3_current AS current
    LEFT JOIN rust_checkpoint_v3_metadata AS metadata
      ON metadata.checkpoint_id = current.checkpoint_id
    WHERE current.run_id = ?
  `).get(runId) as CurrentPointerRow | undefined;
}

/**
 * Prove one current row and its referenced immutable descriptor describe the same operation.
 * @param expectedRunId - Run identity used to look up the pointer.
 * @param current - Joined current-pointer and immutable-metadata row.
 * @returns Strict stored descriptor whose boundary identity is safe to compare.
 */
function validateCurrentPointerIdentity(
  expectedRunId: string,
  current: CurrentPointerRow
): ManagedCheckpointDescriptor {
  if (current.metadata_run_id === null || current.metadata_transition_epoch === null ||
    current.metadata_operation_id === null || current.generation_hex === null ||
    current.completed_step_hex === null || current.descriptor_json === null) {
    throw new Error('current checkpoint pointer references missing immutable metadata');
  }
  let storedValue: unknown;
  try {
    storedValue = JSON.parse(current.descriptor_json);
  } catch {
    throw new Error('current checkpoint pointer references invalid immutable descriptor JSON');
  }
  let stored: ManagedCheckpointDescriptor;
  try {
    stored = parseManagedCheckpointDescriptor(storedValue);
  } catch {
    throw new Error('current checkpoint pointer references invalid immutable descriptor metadata');
  }
  const branch = current.metadata_run_id !== expectedRunId ? readRecoveryBranch(expectedRunId) : undefined;
  const aliased = branch !== undefined && branch.recoveredDescriptor.runId === current.metadata_run_id &&
    branch.recoveredDescriptor.logicalRootSha256 === current.pointer_checkpoint_id &&
    branch.operationId === current.pointer_operation_id &&
    JSON.stringify(branch.recoveredDescriptor) === JSON.stringify(stored);
  if (current.pointer_run_id !== expectedRunId ||
    (current.metadata_run_id !== current.pointer_run_id && !aliased) ||
    current.metadata_transition_epoch !== current.pointer_transition_epoch ||
    (current.metadata_operation_id !== current.pointer_operation_id && !aliased) ||
    stored.runId !== current.metadata_run_id ||
    stored.transitionEpoch !== current.metadata_transition_epoch ||
    stored.operationId !== current.metadata_operation_id ||
    stored.logicalRootSha256 !== current.pointer_checkpoint_id ||
    stored.generation !== current.generation_hex ||
    stored.completedStep !== current.completed_step_hex) {
    throw new Error('current checkpoint pointer identity does not match immutable metadata');
  }
  return stored;
}

/** Resolve the active lineage without arbitrarily selecting among unrelated runs. */
function resolveSelectedRun(runId: string | null): string | null {
  let selectedRun = runId;
  if (selectedRun === null) {
    const active = db.prepare(`SELECT CASE WHEN length(CAST(run_id AS BLOB)) <= 256 THEN run_id END AS run_id
    FROM rust_active_run_v1 WHERE singleton = 1`).get() as { run_id: string | null } | undefined;
    if (active) {
      if (!active.run_id) throw new Error('invalid active recovery run');
      selectedRun = active.run_id;
    }
  }
  if (selectedRun === null) {
    const rows = db.prepare(`SELECT CASE WHEN length(CAST(run_id AS BLOB)) <= 256
    THEN run_id ELSE NULL END AS run_id FROM rust_checkpoint_v3_current LIMIT 2`).all() as Array<{ run_id: string | null }>;
    if (rows.length === 0) return null;
    if (rows.length !== 1) throw new Error('multiple current runs require an explicit run selection');
    selectedRun = rows[0]!.run_id;
    if (!selectedRun) throw new Error('current checkpoint has an invalid run identity');
  }
  return selectedRun;
}

/** Read one exact current target without choosing arbitrarily among multiple runs. */
function selectManagedCheckpoint(runId: string | null): ManagedCheckpointSelection {
  return db.transaction(() => {
    const selectedRun = resolveSelectedRun(runId);
    if (selectedRun === null) return { descriptor: null, runId: null, recovery: null };
    const current = readCurrentPointer(selectedRun);
    if (!current) return { descriptor: null, runId: null, recovery: null };
    const descriptor = validateCurrentPointerIdentity(selectedRun, current);
    assertDescriptorBounds(descriptor);
    return { descriptor, runId: selectedRun, recovery: readRecoveryBranch(selectedRun) ?? null };
  }).deferred();
}

/** Build the current decision inside a caller-owned SQLite read or write transaction. */
function currentCheckpointRetentionDecision(): {
  activeRunId: string;
  decision: CheckpointRetentionDecision;
} {
    const activeRunId = resolveSelectedRun(null);
    if (activeRunId === null) throw new Error('retention inventory requires one active managed run');
    const lineage = recoveryLineage(activeRunId);
    const pointers = db.prepare(`SELECT
      CASE WHEN length(CAST(run_id AS BLOB)) <= 256 THEN run_id END AS run_id,
      CASE WHEN length(checkpoint_id) = 64 THEN checkpoint_id END AS checkpoint_id
      FROM rust_checkpoint_v3_current`).all() as RetentionPointerRow[];
    if (pointers.some(pointer => !pointer.run_id || !/^[0-9a-f]{64}$/u.test(pointer.checkpoint_id))) {
      throw new Error('retention inventory found an invalid current pointer');
    }
    const priorRunByCheckpoint = new Map<string, string>();
    for (const pointer of pointers) {
      if (pointer.run_id === activeRunId) continue;
      const previous = priorRunByCheckpoint.get(pointer.checkpoint_id);
      if (!previous || pointer.run_id.localeCompare(previous) < 0) {
        priorRunByCheckpoint.set(pointer.checkpoint_id, pointer.run_id);
      }
    }
    const rows = db.prepare(`SELECT metadata.checkpoint_id,
      CASE WHEN length(CAST(metadata.descriptor_json AS BLOB)) <= 16384
        THEN metadata.descriptor_json END AS descriptor_json,
      retention.retention_kind,
      metadata.rowid AS created_ordinal
      FROM rust_checkpoint_v3_metadata AS metadata
      JOIN rust_checkpoint_retention_v1 AS retention USING(checkpoint_id)
      WHERE retention.retention_kind IN ('automatic', 'pinned')
      ORDER BY metadata.rowid`).all() as RetentionMetadataRow[];
    const candidates: CheckpointRetentionCandidate[] = rows.map(row => {
      if (!Number.isSafeInteger(row.created_ordinal) || row.created_ordinal < 1 || row.descriptor_json === null ||
          (row.retention_kind !== 'automatic' && row.retention_kind !== 'pinned')) {
        throw new Error('retention inventory found invalid bounded metadata');
      }
      let descriptor: ManagedCheckpointDescriptor;
      try { descriptor = parseManagedCheckpointDescriptor(JSON.parse(row.descriptor_json)); }
      catch { throw new Error('retention inventory found invalid checkpoint descriptor metadata'); }
      if (descriptor.logicalRootSha256 !== row.checkpoint_id) throw new Error('retention metadata identity mismatch');
      const inherited = lineage.some(item => item.runId === descriptor.runId &&
        (item.maximumGeneration === null || descriptor.generation <= item.maximumGeneration));
      const priorRunId = inherited ? undefined : priorRunByCheckpoint.get(descriptor.logicalRootSha256);
      return {
        checkpointId: descriptor.logicalRootSha256,
        runId: inherited ? activeRunId : (priorRunId ?? descriptor.runId),
        generation: u64HexToBigInt(descriptor.generation),
        storedBytes: u64HexToBigInt(descriptor.storedByteCount),
        decodedBytes: u64HexToBigInt(descriptor.decodedByteCount),
        createdOrdinal: BigInt(row.created_ordinal),
        pinned: row.retention_kind === 'pinned',
        priorRunAnchor: priorRunId !== undefined,
        weightsEncoding: descriptor.weightsEncoding,
        recurrentStateEncoding: descriptor.recurrentStateEncoding
      };
    });
    return { activeRunId, decision: selectManagedCheckpointRetention(
      candidates, activeRunId, OWNER_CHECKPOINT_RETENTION_DEFAULTS
    ) };
}

/** Inspect the owner-approved retention result without deleting files or metadata. */
function inspectCheckpointRetention(): CheckpointRetentionInventory {
  return db.transaction(() => {
    const { activeRunId, decision } = currentCheckpointRetentionDecision();
    return buildCheckpointRetentionInventory(decision, activeRunId, OWNER_CHECKPOINT_RETENTION_DEFAULTS);
  }).deferred();
}

/** Pin the exact current immutable file in one worker-owned transaction. */
function pinCurrentCheckpoint(): { checkpointId: string; generation: U64Hex } {
  return db.transaction(() => {
    const activeRunId = resolveSelectedRun(null);
    if (activeRunId === null) throw new Error('pin requires one active managed run');
    const current = readCurrentPointer(activeRunId);
    if (!current) throw new Error('pin requires a current managed checkpoint');
    const descriptor = validateCurrentPointerIdentity(activeRunId, current);
    const changed = db.prepare(`UPDATE rust_checkpoint_retention_v1 SET
      retention_kind = 'pinned', classified_at_ms = ? WHERE checkpoint_id = ?`)
      .run(Date.now(), descriptor.logicalRootSha256);
    if (changed.changes !== 1) throw new Error('current checkpoint lacks retention metadata');
    return { checkpointId: descriptor.logicalRootSha256, generation: descriptor.generation };
  }).immediate();
}

/** Build the inherited completed-generation filter for one exact checkpoint boundary. */
function exportLineageFilter(
  runId: string,
  checkpointGeneration: U64Hex,
  tableAlias: string
): { sql: string; parameters: string[]; completedGenerationCount: bigint } {
  const completedGenerationCount = u64HexToBigInt(checkpointGeneration) - 1n;
  const lineage = recoveryLineage(runId).map(item => {
    const boundary = item.maximumGeneration === null
      ? completedGenerationCount
      : u64HexToBigInt(item.maximumGeneration) - 1n;
    return { runId: item.runId, maximumCompletedGeneration: boundary };
  });
  return {
    sql: lineage.map(() => `(${tableAlias}.run_id = ? AND ${tableAlias}.generation_hex <= ?)`)
      .join(' OR '),
    parameters: lineage.flatMap(item => [
      item.runId,
      item.maximumCompletedGeneration.toString(16).padStart(16, '0')
    ]),
    completedGenerationCount
  };
}

/** Write every byte of one small fixed record to an already-open inventory file. */
function writeInventoryBytes(file: number, bytes: Buffer): void {
  let offset = 0;
  while (offset < bytes.length) offset += writeSync(file, bytes, offset, bytes.length - offset);
}

/** Publish one bounded fixed-width history and Hall-of-Fame inventory for Rust. */
function publishExportInventory(
  operationId: CheckpointOperationId,
  runId: string,
  checkpoint: ManagedCheckpointDescriptor
): { descriptor: ManagedExportInventoryDescriptor; path: string } {
  const historyFilter = exportLineageFilter(runId, checkpoint.generation, 'history');
  const hallFilter = exportLineageFilter(runId, checkpoint.generation, 'hall');
  const expectedCount = historyFilter.completedGenerationCount;
  if (expectedCount < 0n || expectedCount > MAX_EXPORT_GENERATIONS) {
    throw new RangeError('export generation count exceeds the bounded inventory limit');
  }
  const historyCount = BigInt((db.prepare(`SELECT count(*) AS count
    FROM rust_generation_history_v1 AS history WHERE ${historyFilter.sql}`)
    .get(...historyFilter.parameters) as { count: number }).count);
  const hallOfFameCount = BigInt((db.prepare(`SELECT count(*) AS count
    FROM rust_hall_of_fame_v1 AS hall
    WHERE hall.weight_state = 'selected' AND (${hallFilter.sql})`)
    .get(...hallFilter.parameters) as { count: number }).count);
  const linkedHallOfFameCount = BigInt((db.prepare(`SELECT count(*) AS count
    FROM rust_hall_of_fame_v1 AS hall
    JOIN rust_hall_of_fame_weights_v1 AS weights ON weights.logical_sha256 = hall.weights_sha256
    WHERE hall.weight_state = 'selected' AND (${hallFilter.sql})`)
    .get(...hallFilter.parameters) as { count: number }).count);
  if (historyCount !== expectedCount || linkedHallOfFameCount !== hallOfFameCount) {
    throw new Error('export requires complete compact history and every selected Hall-of-Fame object');
  }

  const relativeFilename = `.${operationId}.export-inventory-v1`;
  const finalPath = resolve(managedRootPath, relativeFilename);
  const partialPath = `${finalPath}.partial`;
  let file: number | undefined;
  let published = false;
  const hasher = createHash('sha256');
  const write = (bytes: Buffer): void => {
    hasher.update(bytes);
    writeInventoryBytes(file!, bytes);
  };
  try {
    file = openSync(partialPath, 'wx');
    const header = Buffer.alloc(EXPORT_INVENTORY_HEADER_BYTES);
    header.write('SLITHER-EXPV1', 0, 'ascii');
    header.writeBigUInt64LE(historyCount, 16);
    header.writeBigUInt64LE(hallOfFameCount, 24);
    write(header);

    let expectedGeneration = 1n;
    const historyRows = db.prepare(`SELECT generation_hex, record_blob
      FROM rust_generation_history_v1 AS history WHERE ${historyFilter.sql}
      ORDER BY generation_hex`).iterate(...historyFilter.parameters) as Iterable<{
        generation_hex: string; record_blob: Buffer;
      }>;
    for (const row of historyRows) {
      if (row.generation_hex !== expectedGeneration.toString(16).padStart(16, '0') ||
          !Buffer.isBuffer(row.record_blob) || row.record_blob.length !== 56) {
        throw new Error('export compact history is missing, duplicated, or malformed');
      }
      write(row.record_blob);
      expectedGeneration++;
    }
    if (expectedGeneration !== historyCount + 1n) {
      throw new Error('export compact history count changed during inventory publication');
    }

    let previousHallOfFameGeneration = 0n;
    const hallRows = db.prepare(`SELECT hall.generation_hex, hall.record_blob,
      weights.logical_sha256, weights.relative_filename, weights.encoding,
      weights.stored_byte_count_hex, weights.decoded_byte_count_hex, weights.weight_count_hex
      FROM rust_hall_of_fame_v1 AS hall
      JOIN rust_hall_of_fame_weights_v1 AS weights ON weights.logical_sha256 = hall.weights_sha256
      WHERE hall.weight_state = 'selected' AND (${hallFilter.sql})
      ORDER BY hall.generation_hex`).iterate(...hallFilter.parameters) as Iterable<{
        generation_hex: string; record_blob: Buffer; logical_sha256: string; relative_filename: string;
        encoding: string; stored_byte_count_hex: string; decoded_byte_count_hex: string;
        weight_count_hex: string;
    }>;
    for (const row of hallRows) {
      const generation = u64HexToBigInt(row.generation_hex);
      if (generation <= previousHallOfFameGeneration || generation > historyFilter.completedGenerationCount ||
          !Buffer.isBuffer(row.record_blob) || row.record_blob.length !== 56) {
        throw new Error('export Hall-of-Fame selection is unordered, duplicated, or malformed');
      }
      const weights = parseManagedHallOfFameWeightsDescriptor({
        version: 1,
        logicalSha256: row.logical_sha256,
        relativeFilename: row.relative_filename,
        encoding: row.encoding,
        storedByteCount: row.stored_byte_count_hex,
        decodedByteCount: row.decoded_byte_count_hex,
        weightCount: row.weight_count_hex
      }, checkpoint);
      verifyHallOfFameWeightsFile(weights);
      const record = Buffer.alloc(EXPORT_INVENTORY_HALL_OF_FAME_BYTES);
      row.record_blob.copy(record, 0);
      Buffer.from(weights.logicalSha256, 'hex').copy(record, 56);
      record[88] = weights.encoding === 'raw-f32le-v1' ? 0 : 1;
      record.writeBigUInt64LE(u64HexToBigInt(weights.storedByteCount), 96);
      record.writeBigUInt64LE(u64HexToBigInt(weights.decodedByteCount), 104);
      record.writeBigUInt64LE(u64HexToBigInt(weights.weightCount), 112);
      write(record);
      previousHallOfFameGeneration = generation;
    }
    fsyncSync(file!);
    closeSync(file!);
    file = undefined;
    renameSync(partialPath, finalPath);
    published = true;
    const storedByteCount = BigInt(EXPORT_INVENTORY_HEADER_BYTES) + historyCount * 56n +
      hallOfFameCount * BigInt(EXPORT_INVENTORY_HALL_OF_FAME_BYTES);
    verifyManagedDirectFile(relativeFilename, storedByteCount);
    return {
      descriptor: {
        version: 1,
        relativeFilename,
        sha256: hasher.digest('hex'),
        storedByteCount: storedByteCount.toString(16).padStart(16, '0'),
        historyCount: historyCount.toString(16).padStart(16, '0'),
        hallOfFameCount: hallOfFameCount.toString(16).padStart(16, '0')
      },
      path: finalPath
    };
  } catch (error) {
    if (file !== undefined) closeSync(file);
    for (const path of [partialPath, ...(published ? [finalPath] : [])]) {
      try { unlinkSync(path); } catch (cleanupError) {
        if (!(cleanupError && typeof cleanupError === 'object' &&
          (cleanupError as NodeJS.ErrnoException).code === 'ENOENT')) throw cleanupError;
      }
    }
    throw error;
  }
}

/** Protect the exact active current file until one direct export releases it. */
function acquireCurrentExportLease(operationId: CheckpointOperationId): {
  operationId: CheckpointOperationId;
  runId: string;
  descriptor: ManagedCheckpointDescriptor;
  inventory: ManagedExportInventoryDescriptor;
} {
  if (activeExportLease) throw new Error('another checkpoint export is already active');
  const selected = db.transaction(() => {
    const runId = resolveSelectedRun(null);
    if (runId === null) throw new Error('export requires one active managed run');
    const current = readCurrentPointer(runId);
    if (!current) throw new Error('export requires a current managed checkpoint');
    const descriptor = validateCurrentPointerIdentity(runId, current);
    const retained = db.prepare(`SELECT retention_kind FROM rust_checkpoint_retention_v1
      WHERE checkpoint_id = ?`).get(descriptor.logicalRootSha256) as { retention_kind: string } | undefined;
    if (!retained || (retained.retention_kind !== 'automatic' && retained.retention_kind !== 'pinned')) {
      throw new Error('current checkpoint is not available for export');
    }
    verifyManagedFile(descriptor);
    return { runId, descriptor };
  }).deferred();
  const inventory = publishExportInventory(operationId, selected.runId, selected.descriptor);
  activeExportLease = { operationId, checkpointId: selected.descriptor.logicalRootSha256,
    inventoryPath: inventory.path };
  return { operationId, ...selected, inventory: inventory.descriptor };
}

/** Release only the exact active export reference. */
function releaseExportLease(operationId: CheckpointOperationId): void {
  if (!activeExportLease || activeExportLease.operationId !== operationId) {
    throw new Error('export lease is not active');
  }
  try { unlinkSync(activeExportLease.inventoryPath); }
  catch (error) {
    if (!(error && typeof error === 'object' && (error as NodeJS.ErrnoException).code === 'ENOENT')) throw error;
  }
  activeExportLease = undefined;
  cleanupUnreferencedHallOfFameWeights();
}

/** Apply one automatic retention decision while preserving all compact metadata. */
function applyCheckpointRetention(): {
  deletedCheckpointCount: number;
  deletedStoredByteCount: U64Hex;
  inventory: CheckpointRetentionInventory;
} {
  const descriptors = db.transaction(() => {
    db.prepare(`UPDATE rust_checkpoint_retention_v1 SET retention_kind = 'automatic', classified_at_ms = ?
      WHERE retention_kind = 'pruning' AND checkpoint_id IN (
        SELECT checkpoint_id FROM rust_hall_of_fame_v1 WHERE weight_state = 'legacy'
      )`).run(Date.now());
    const { decision } = currentCheckpointRetentionDecision();
    const unbackedHallOfFame = new Set((db.prepare(
      "SELECT checkpoint_id FROM rust_hall_of_fame_v1 WHERE weight_state = 'legacy'"
    ).all() as Array<{ checkpoint_id: string }>).map(row => row.checkpoint_id));
    const planned = decision.pruned.filter(candidate =>
      candidate.checkpointId !== activeExportLease?.checkpointId &&
      !unbackedHallOfFame.has(candidate.checkpointId)
    );
    const targetBytes = new Map(planned.map(candidate => [candidate.checkpointId, candidate.storedBytes]));
    const pending = db.prepare(`SELECT metadata.checkpoint_id
      FROM rust_checkpoint_v3_metadata AS metadata
      JOIN rust_checkpoint_retention_v1 AS retention USING(checkpoint_id)
      WHERE retention.retention_kind = 'pruning'
        AND NOT EXISTS (SELECT 1 FROM rust_checkpoint_v3_current WHERE checkpoint_id = metadata.checkpoint_id)
        AND NOT EXISTS (SELECT 1 FROM rust_hall_of_fame_v1
          WHERE checkpoint_id = metadata.checkpoint_id AND weight_state = 'legacy')
      ORDER BY metadata.rowid`).all() as Array<{ checkpoint_id: string }>;
    const checkpointIds = [...pending.map(row => row.checkpoint_id), ...planned.map(candidate => candidate.checkpointId)];
    const uniqueCheckpointIds = [...new Set(checkpointIds)];
    const selected = uniqueCheckpointIds.map(checkpointId => {
      const row = db.prepare(`SELECT
        CASE WHEN length(CAST(metadata.descriptor_json AS BLOB)) <= 16384
          THEN metadata.descriptor_json END AS descriptor_json,
        retention.retention_kind
        FROM rust_checkpoint_v3_metadata AS metadata
        JOIN rust_checkpoint_retention_v1 AS retention USING(checkpoint_id)
        WHERE metadata.checkpoint_id = ?
          AND NOT EXISTS (SELECT 1 FROM rust_checkpoint_v3_current WHERE checkpoint_id = metadata.checkpoint_id)`)
        .get(checkpointId) as { descriptor_json: string | null; retention_kind: string } | undefined;
      if (!row || (row.retention_kind !== 'automatic' && row.retention_kind !== 'pruning') || row.descriptor_json === null) {
        throw new Error('retention decision selected a protected or invalid checkpoint');
      }
      const descriptor = parseManagedCheckpointDescriptor(JSON.parse(row.descriptor_json));
      const expectedBytes = targetBytes.get(checkpointId);
      if (descriptor.logicalRootSha256 !== checkpointId ||
          (expectedBytes !== undefined && u64HexToBigInt(descriptor.storedByteCount) !== expectedBytes)) {
        throw new Error('retention prune descriptor changed after selection');
      }
      return descriptor;
    });
    for (const candidate of planned) {
      const changed = db.prepare(`UPDATE rust_checkpoint_retention_v1 SET
        retention_kind = 'pruning', classified_at_ms = ?
        WHERE checkpoint_id = ? AND retention_kind = 'automatic'
          AND NOT EXISTS (SELECT 1 FROM rust_checkpoint_v3_current WHERE checkpoint_id = ?)`)
        .run(Date.now(), candidate.checkpointId, candidate.checkpointId);
      if (changed.changes !== 1) throw new Error('retention prune target became protected before intent commit');
    }
    return selected;
  }).immediate();

  let deletedStoredBytes = 0n;
  for (const descriptor of descriptors) {
    try {
      const file = verifyManagedFile(descriptor);
      unlinkSync(file.path);
    } catch (error) {
      if (!(error && typeof error === 'object' && (error as NodeJS.ErrnoException).code === 'ENOENT')) throw error;
    }
    deletedStoredBytes += u64HexToBigInt(descriptor.storedByteCount);
    if (deletedStoredBytes > 0xffff_ffff_ffff_ffffn) throw new RangeError('deleted checkpoint bytes exceed u64');
  }

  db.transaction(() => {
    for (const descriptor of descriptors) {
      const changed = db.prepare(`UPDATE rust_checkpoint_retention_v1 SET
        retention_kind = 'pruned', classified_at_ms = ?
        WHERE checkpoint_id = ? AND retention_kind = 'pruning'
          AND NOT EXISTS (SELECT 1 FROM rust_checkpoint_v3_current WHERE checkpoint_id = ?)`)
        .run(Date.now(), descriptor.logicalRootSha256, descriptor.logicalRootSha256);
      if (changed.changes !== 1) throw new Error('retention prune target became protected before classification');
    }
  }).immediate();
  return {
    deletedCheckpointCount: descriptors.length,
    deletedStoredByteCount: deletedStoredBytes.toString(16).padStart(16, '0'),
    inventory: inspectCheckpointRetention()
  };
}

/** Visit one retained metadata record without materializing the retained population set. */
function scanRecoveryCandidate(value: RecoveryScanCursor | null): RecoveryScanResult {
  return db.transaction(() => {
    const activeRun = resolveSelectedRun(null);
    if (!activeRun) throw new Error('no active lineage available for recovery');
    const source = readCurrentPointer(activeRun);
    if (!source) throw new Error('active recovery source pointer is missing');
    const cursor = value ?? parseRecoveryScanCursor({ sourceRunId: activeRun,
      failedCheckpointId: source.pointer_checkpoint_id, generation: null, checkpointId: null });
    if (cursor.sourceRunId !== activeRun || cursor.failedCheckpointId !== source.pointer_checkpoint_id) {
      throw new Error('failed source pointer changed during recovery scan');
    }
    const filter = recoveryLineageFilter(recoveryLineage(activeRun));
    const row = db.prepare(`SELECT run_id, generation_hex, checkpoint_id,
      CASE WHEN length(CAST(descriptor_json AS BLOB)) <= 16384 THEN descriptor_json END AS descriptor_json,
      CASE WHEN length(CAST(operation_id AS BLOB)) <= 32 THEN operation_id END AS operation_id,
      CASE WHEN length(CAST(transition_epoch AS BLOB)) <= 16 THEN transition_epoch END AS transition_epoch,
      CASE WHEN length(CAST(completed_step_hex AS BLOB)) <= 16 THEN completed_step_hex END AS completed_step_hex
      FROM rust_checkpoint_v3_metadata AS metadata
      JOIN rust_checkpoint_retention_v1 AS retention USING(checkpoint_id)
      WHERE retention.retention_kind IN ('automatic', 'pinned') AND (${filter.sql})
        AND length(generation_hex) = 16 AND generation_hex NOT GLOB '*[^0-9a-f]*' AND generation_hex != '0000000000000000'
        AND length(checkpoint_id) = 64 AND checkpoint_id NOT GLOB '*[^0-9a-f]*'
        AND (? IS NULL OR generation_hex < ? OR (generation_hex = ? AND checkpoint_id < ?))
      ORDER BY generation_hex DESC, checkpoint_id DESC LIMIT 1`)
      .get(...filter.parameters, cursor.generation, cursor.generation, cursor.generation, cursor.checkpointId) as {
        run_id: string; generation_hex: string; checkpoint_id: string; descriptor_json: string | null;
        operation_id: string | null; transition_epoch: string | null; completed_step_hex: string | null;
      } | undefined;
    if (!row) return { cursor, descriptor: null, issue: null, exhausted: true };
    const next = { ...cursor, generation: row.generation_hex, checkpointId: row.checkpoint_id };
    try {
      if (row.descriptor_json === null) throw new Error('oversized retained descriptor');
      const descriptor = parseManagedCheckpointDescriptor(JSON.parse(row.descriptor_json));
      if (descriptor.runId !== row.run_id || descriptor.generation !== row.generation_hex ||
          descriptor.logicalRootSha256 !== row.checkpoint_id || descriptor.operationId !== row.operation_id ||
          descriptor.transitionEpoch !== row.transition_epoch || descriptor.completedStep !== row.completed_step_hex) {
        throw new Error('retained metadata identity mismatch');
      }
      assertDescriptorBounds(descriptor);
      return { cursor: next, descriptor, issue: null, exhausted: false };
    } catch {
      return { cursor: next, descriptor: null, issue: 'retained checkpoint metadata is invalid', exhausted: false };
    }
  }).deferred();
}

/**
 * Reject a checkpoint that would regress or skip the run's generation-boundary history.
 *
 * `transitionEpoch` correlates one Rust authority operation and may restart after a new world
 * incarnation. Persistent ordering therefore comes from generation/completed-step identity,
 * not from assuming checkpoint operations are consecutive fixed-step attempts.
 */
function assertChronologicalSuccessor(
  candidate: ManagedCheckpointDescriptor,
  current: CurrentPointerRow | undefined
): void {
  const generation = u64HexToBigInt(candidate.generation);
  const completedStep = u64HexToBigInt(candidate.completedStep);
  if (!current) {
    if (candidate.boundaryKind !== 'run-start') {
      throw new Error(
        'generation checkpoint requires an existing current pointer or explicit branch provenance'
      );
    }
    return;
  }
  const currentDescriptor = validateCurrentPointerIdentity(candidate.runId, current);
  const currentGeneration = u64HexToBigInt(currentDescriptor.generation);
  const currentCompletedStep = u64HexToBigInt(currentDescriptor.completedStep);
  if (candidate.boundaryKind !== 'generation') {
    throw new Error('an existing run can advance only to a generation checkpoint');
  }
  if (generation <= currentGeneration || completedStep <= currentCompletedStep) {
    throw new Error('checkpoint boundary is stale and must not regress the current pointer');
  }
  if (generation !== currentGeneration + 1n) {
    throw new Error('checkpoint generation must advance the per-run current pointer by exactly one');
  }
}

/**
 * Commit a verified descriptor and monotonic per-run current pointer atomically.
 * @param descriptor - Strict descriptor whose final managed file already exists.
 * @param generationCommit - Exact compact result and Hall-of-Fame reference for a generation.
 * @returns Matching commit acknowledgement fields.
 */
function commitManagedCheckpoint(
  descriptor: ManagedCheckpointDescriptor,
  generationCommit: ManagedGenerationCommit | null
): {
  operationId: CheckpointOperationId;
  transitionEpoch: U64Hex;
  runId: string;
  checkpointId: string;
  descriptor: ManagedCheckpointDescriptor;
} {
  cleanupUnreferencedHallOfFameWeights();
  assertDescriptorBounds(descriptor);
  const descriptorJson = serializeDescriptor(descriptor);
  const summaryRecord = generationCommit === null
    ? null
    : encodeGenerationSummary(generationCommit.summary);
  const hallOfFameRecord = generationCommit === null
    ? null
    : encodeHallOfFameReference(generationCommit.hallOfFame);
  const managedFile = verifyManagedFile(descriptor);
  const hallOfFameWeightsFile = generationCommit === null
    ? null
    : verifyHallOfFameWeightsFile(generationCommit.hallOfFameWeights);
  const commit = db.transaction((candidate: ManagedCheckpointDescriptor) => {
    const existingOperation = db.prepare(
      'SELECT descriptor_json FROM rust_checkpoint_v3_metadata WHERE operation_id = ?'
    ).get(candidate.operationId) as ExistingDescriptorRow | undefined;
    if (existingOperation) {
      if (existingOperation.descriptor_json !== descriptorJson) {
        throw new Error('operationId conflicts with a different immutable checkpoint descriptor');
      }
      const existingSummary = db.prepare(
        `SELECT run_id, generation_hex, record_version, record_blob
         FROM rust_generation_history_v1 WHERE checkpoint_id = ?`
      ).get(candidate.logicalRootSha256) as ExistingGenerationSummaryRow | undefined;
      if ((summaryRecord === null && existingSummary) ||
        (summaryRecord !== null && (!existingSummary ||
          existingSummary.run_id !== candidate.runId ||
          existingSummary.generation_hex !== generationCommit?.summary.completedGeneration ||
          existingSummary.record_version !== 1 ||
          !Buffer.isBuffer(existingSummary.record_blob) ||
          !existingSummary.record_blob.equals(summaryRecord)))) {
        throw new Error('operationId conflicts with different compact generation history');
      }
      const existingHallOfFame = db.prepare(
        `SELECT run_id, generation_hex, record_version, record_blob, weights_sha256,
          genome_sha256, weight_state
         FROM rust_hall_of_fame_v1 WHERE checkpoint_id = ?`
      ).get(candidate.logicalRootSha256) as ExistingHallOfFameRow | undefined;
      if ((hallOfFameRecord === null && existingHallOfFame) ||
        (hallOfFameRecord !== null && (!existingHallOfFame ||
          existingHallOfFame.run_id !== candidate.runId ||
          existingHallOfFame.generation_hex !== generationCommit?.hallOfFame.completedGeneration ||
          existingHallOfFame.record_version !== 1 ||
          existingHallOfFame.genome_sha256 !== generationCommit?.hallOfFameWeights.logicalSha256 ||
          (existingHallOfFame.weight_state === 'selected'
            ? existingHallOfFame.weights_sha256 !== generationCommit?.hallOfFameWeights.logicalSha256
            : existingHallOfFame.weight_state !== 'unselected' || existingHallOfFame.weights_sha256 !== null) ||
          !Buffer.isBuffer(existingHallOfFame.record_blob) ||
          !existingHallOfFame.record_blob.equals(hallOfFameRecord)))) {
        throw new Error('operationId conflicts with a different Hall-of-Fame reference');
      }
      if (generationCommit !== null) assertStoredHallOfFameWeights(generationCommit.hallOfFameWeights);
      const current = readCurrentPointer(candidate.runId);
      if (!current) {
        throw new Error('operationId replay is superseded and must not regress the current pointer');
      }
      validateCurrentPointerIdentity(candidate.runId, current);
      if (current.pointer_checkpoint_id !== candidate.logicalRootSha256 ||
        current.pointer_operation_id !== candidate.operationId ||
        current.pointer_transition_epoch !== candidate.transitionEpoch) {
        throw new Error('operationId replay is superseded and must not regress the current pointer');
      }
      return;
    }
    const existingCheckpoint = db.prepare(
      'SELECT descriptor_json FROM rust_checkpoint_v3_metadata WHERE checkpoint_id = ?'
    ).get(candidate.logicalRootSha256) as ExistingDescriptorRow | undefined;
    if (existingCheckpoint) {
      throw new Error('logical checkpoint root is already committed under a different operation');
    }
    assertChronologicalSuccessor(candidate, readCurrentPointer(candidate.runId));
    recheckManagedFile(managedFile);
    if (hallOfFameWeightsFile) recheckManagedFile(hallOfFameWeightsFile);
    db.prepare(`
      INSERT INTO rust_checkpoint_v3_metadata (
        checkpoint_id, operation_id, run_id, transition_epoch, generation_hex, completed_step_hex,
        boundary_kind, checkpoint_format_version_hex, state_version_hex, graph_layout_version_hex,
        managed_root, relative_filename, logical_root_sha256, stored_byte_count_hex,
        decoded_byte_count_hex, role_count_hex, population_count_hex, weight_count_hex, recurrent_state_count_hex,
        weights_encoding, recurrent_state_encoding, graph_layout_sha256,
        write_validation_policy, descriptor_json, created_at_ms
      ) VALUES (
        @checkpointId, @operationId, @runId, @transitionEpoch, @generation, @completedStep,
        @boundaryKind, @checkpointFormatVersion, @stateVersion, @graphLayoutVersion,
        @managedRoot, @relativeFilename, @logicalRootSha256, @storedByteCount,
        @decodedByteCount, @roleCount, @populationCount, @weightCount, @recurrentStateCount,
        @weightsEncoding, @recurrentStateEncoding, @graphLayoutSha256,
        @writeValidationPolicy, @descriptorJson, @createdAtMs
      )
    `).run({ ...candidate, checkpointId: candidate.logicalRootSha256, descriptorJson, createdAtMs: Date.now() });
    db.prepare(`INSERT INTO rust_checkpoint_retention_v1 (
      checkpoint_id, retention_kind, classified_at_ms
    ) VALUES (?, 'automatic', ?)`).run(candidate.logicalRootSha256, Date.now());
    if (generationCommit !== null && summaryRecord !== null && hallOfFameRecord !== null) {
      const weights = generationCommit.hallOfFameWeights;
      db.prepare(`INSERT OR IGNORE INTO rust_hall_of_fame_weights_v1 (
        logical_sha256, relative_filename, encoding, stored_byte_count_hex,
        decoded_byte_count_hex, weight_count_hex, created_at_ms
      ) VALUES (?, ?, ?, ?, ?, ?, ?)`)
        .run(weights.logicalSha256, weights.relativeFilename, weights.encoding,
          weights.storedByteCount, weights.decodedByteCount, weights.weightCount, Date.now());
      assertStoredHallOfFameWeights(weights);
      db.prepare(`
        INSERT INTO rust_generation_history_v1 (
          run_id, generation_hex, checkpoint_id, record_version, record_blob, created_at_ms
        ) VALUES (
          @runId, @completedGeneration, @checkpointId, 1, @summaryRecord, @createdAtMs
        )
      `).run({
        runId: candidate.runId,
        completedGeneration: generationCommit.summary.completedGeneration,
        checkpointId: candidate.logicalRootSha256,
        summaryRecord,
        createdAtMs: Date.now()
      });
      db.prepare(`
        INSERT INTO rust_hall_of_fame_v1 (
          run_id, generation_hex, checkpoint_id, record_version, record_blob, weights_sha256, genome_sha256,
          fitness_value, pinned, weight_state, created_at_ms
        ) VALUES (
          @runId, @completedGeneration, @checkpointId, 1, @hallOfFameRecord, @weightsSha256, @weightsSha256,
          @fitnessValue, 0, 'selected', @createdAtMs
        )
      `).run({
        runId: candidate.runId,
        completedGeneration: generationCommit.hallOfFame.completedGeneration,
        checkpointId: candidate.logicalRootSha256,
        hallOfFameRecord,
        weightsSha256: weights.logicalSha256,
        fitnessValue: f64HexToNumber(generationCommit.hallOfFame.fitnessF64Hex),
        createdAtMs: Date.now()
      });
      applyHallOfFameRetention(candidate.runId);
    }
    db.prepare(`
      INSERT INTO rust_checkpoint_v3_current (run_id, checkpoint_id, transition_epoch, operation_id)
      VALUES (@runId, @checkpointId, @transitionEpoch, @operationId)
      ON CONFLICT(run_id) DO UPDATE SET
        checkpoint_id = excluded.checkpoint_id,
        transition_epoch = excluded.transition_epoch,
        operation_id = excluded.operation_id
    `).run({
      runId: candidate.runId,
      checkpointId: candidate.logicalRootSha256,
      transitionEpoch: candidate.transitionEpoch,
      operationId: candidate.operationId
    });
  });
  commit(descriptor);
  return {
    operationId: descriptor.operationId,
    transitionEpoch: descriptor.transitionEpoch,
    runId: descriptor.runId,
    checkpointId: descriptor.logicalRootSha256,
    descriptor
  };
}

/**
 * Turn an unknown error into one bounded worker-safe rejection detail.
 * @param error - Unknown caught error.
 * @returns Bounded safe reason text.
 */
function rejectionReason(error: unknown): string {
  const text = error instanceof Error ? error.message : 'unknown persistence worker failure';
  return Buffer.from(text, 'utf8').subarray(0, MAX_REJECTION_REASON_BYTES).toString('utf8');
}

/**
 * Extract a valid operation token from malformed input without interpreting descriptor bytes.
 * @param value - Unknown request candidate.
 * @returns Operation token when safely present.
 */
function extractOperationId(value: unknown): CheckpointOperationId | null {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) return null;
  const request = value as Record<string, unknown>;
  if (request['type'] === 'commitRecoveryBranch') {
    const commit = request['commit'];
    if (!commit || typeof commit !== 'object') return null;
    const id = (commit as Record<string, unknown>)['operationId'];
    return typeof id === 'string' && /^[0-9a-f]{32}$/u.test(id) ? id : null;
  }
  if (request['type'] === 'selectManagedCheckpoint' || request['type'] === 'scanRecoveryCandidate' ||
      request['type'] === 'inspectCheckpointRetention' || request['type'] === 'pinCurrentCheckpoint' ||
      request['type'] === 'applyCheckpointRetention' || request['type'] === 'acquireCurrentExportLease' ||
      request['type'] === 'releaseExportLease') {
    const operationId = request['operationId'];
    return typeof operationId === 'string' && /^[0-9a-f]{32}$/u.test(operationId) ? operationId : null;
  }
  const descriptor = (value as Record<string, unknown>)['descriptor'];
  if (descriptor === null || typeof descriptor !== 'object' || Array.isArray(descriptor)) return null;
  const operationId = (descriptor as Record<string, unknown>)['operationId'];
  return typeof operationId === 'string' && /^[0-9a-f]{32}$/u.test(operationId)
    ? operationId
    : null;
}

/**
 * Post one typed worker response.
 * @param response - Structured-clone-safe response for the client.
 */
function post(response: CheckpointPersistenceWorkerResponse): void {
  port.postMessage(response);
}

port.on('message', (message: unknown) => {
  if (message !== null && typeof message === 'object' && !Array.isArray(message) &&
    (message as Record<string, unknown>)['type'] === 'shutdown' && Object.keys(message).length === 1) {
    if (activeExportLease) {
      try { unlinkSync(activeExportLease.inventoryPath); }
      catch { /* Best-effort temporary inventory cleanup during worker shutdown. */ }
      activeExportLease = undefined;
    }
    db.close();
    port.removeAllListeners('message');
    port.close();
    return;
  }
  const operationId = extractOperationId(message);
  try {
    if (message === null || typeof message !== 'object' || Array.isArray(message)) {
      throw new TypeError('worker request must be an object');
    }
    const request = message as Record<string, unknown>;
    if (request['type'] === 'releaseExportLease') {
      if (!operationId || Object.keys(request).length !== 2) throw new TypeError('invalid export lease release request');
      releaseExportLease(operationId);
      post({ type: 'exportLeaseReleased', operationId });
      return;
    }
    if (request['type'] === 'acquireCurrentExportLease') {
      if (!operationId || Object.keys(request).length !== 2) throw new TypeError('invalid export lease request');
      post({ type: 'currentExportLeaseAcquired', lease: acquireCurrentExportLease(operationId) });
      return;
    }
    if (request['type'] === 'applyCheckpointRetention') {
      if (!operationId || Object.keys(request).length !== 2) throw new TypeError('invalid retention apply request');
      post({ type: 'checkpointRetentionApplied', operationId, result: applyCheckpointRetention() });
      return;
    }
    if (request['type'] === 'pinCurrentCheckpoint') {
      if (!operationId || Object.keys(request).length !== 2) throw new TypeError('invalid pin-current request');
      post({ type: 'currentCheckpointPinned', operationId, ...pinCurrentCheckpoint() });
      return;
    }
    if (request['type'] === 'inspectCheckpointRetention') {
      if (!operationId || Object.keys(request).length !== 2) throw new TypeError('invalid retention inventory request');
      post({ type: 'checkpointRetentionInspected', operationId, inventory: inspectCheckpointRetention() });
      return;
    }
    if (request['type'] === 'scanRecoveryCandidate') {
      if (!operationId || Object.keys(request).length !== 3 || !Object.hasOwn(request, 'cursor')) throw new TypeError('invalid recovery scan request');
      const cursor = request['cursor'] === null ? null : parseRecoveryScanCursor(request['cursor']);
      post({ type: 'recoveryCandidate', operationId, result: scanRecoveryCandidate(cursor) });
      return;
    }
    if (request['type'] === 'commitRecoveryBranch') {
      if (Object.keys(request).length !== 2 || !Object.hasOwn(request, 'commit')) throw new TypeError('invalid recovery request');
      post({ type: 'recoveryBranchCommitted', result: commitRecoveryBranch(parseRecoveryBranchCommit(request['commit'])) });
      return;
    }
    if (request['type'] === 'selectManagedCheckpoint') {
      const runId = request['runId'];
      if (!operationId || Object.keys(request).length !== 3 || !Object.hasOwn(request, 'runId') ||
          (runId !== null && (typeof runId !== 'string' || !runId || Buffer.byteLength(runId) > 256 || runId.includes('\0')))) {
        throw new TypeError('invalid checkpoint selection request');
      }
      post({ type: 'managedCheckpointSelected', operationId, ...selectManagedCheckpoint(runId as string | null) });
      return;
    }
    if (request['type'] !== 'commitManagedCheckpoint' || Object.keys(request).length !== 3 ||
      !Object.hasOwn(request, 'descriptor') || !Object.hasOwn(request, 'generationCommit')) {
      throw new TypeError('worker request has an unsupported type or unknown fields');
    }
    const descriptor = parseManagedCheckpointDescriptor(request['descriptor']);
    const generationCommit = parseManagedGenerationCommit(
      request['generationCommit'],
      descriptor
    );
    const committed = commitManagedCheckpoint(descriptor, generationCommit);
    post({ type: 'managedCheckpointCommitted', ...committed });
  } catch (error) {
    post({ type: 'managedCheckpointRejected', operationId, reason: rejectionReason(error) });
  }
});
