import { parseRecoveryBranchCommit, parseRecoveryBranchResult, type RecoveryBranchCommit, type RecoveryBranchResult } from './recoveryProtocol.ts';
import { lstatSync, realpathSync, statSync } from 'node:fs';
import { dirname, resolve, sep } from 'node:path';
import { parentPort, workerData } from 'node:worker_threads';
import Database from 'better-sqlite3';
import {
  parseManagedCheckpointDescriptor,
  parseManagedCheckpointDescriptorLimits,
  parseManagedGenerationCommit,
  type CheckpointOperationId,
  type CheckpointPersistenceWorkerResponse,
  type ManagedCheckpointDescriptor,
  type ManagedCheckpointSelection,
  type ManagedCheckpointDescriptorLimits,
  type ManagedGenerationCommit,
  type ManagedGenerationSummary,
  type ManagedHallOfFameReference,
  type U64Hex
} from './checkpointPersistenceProtocol.ts';

/** Maximum text length returned to the client for any worker rejection. */
const MAX_REJECTION_REASON_BYTES = 1024;

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
initializeRecoverySchema(db);

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
      'rust_generation_history_v1', 'rust_hall_of_fame_v1',
      'rust_recovery_branches_v1', 'rust_active_run_v1')) AS recognized
    FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%'`).get() as {
      total: number; recognized: number | null;
    };
  if (![4, 6].includes(schema.total) || schema.recognized !== schema.total) {
    throw new Error('resume requires an existing managed checkpoint metadata database');
  }
  // Preparing these fixed reads also rejects incompatible columns without DDL.
  database.prepare('SELECT checkpoint_id, operation_id, run_id, transition_epoch, generation_hex, completed_step_hex, descriptor_json FROM rust_checkpoint_v3_metadata LIMIT 0').all();
  database.prepare('SELECT run_id, checkpoint_id, transition_epoch, operation_id FROM rust_checkpoint_v3_current LIMIT 0').all();
  for (const table of ['rust_generation_history_v1', 'rust_hall_of_fame_v1']) {
    database.prepare(`SELECT run_id, generation_hex, checkpoint_id, record_version, record_blob, created_at_ms FROM ${table} LIMIT 0`).all();
  }
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
    const retained = db.prepare(`SELECT CASE WHEN length(CAST(descriptor_json AS BLOB)) <= 16384
      THEN descriptor_json END AS descriptor_json FROM rust_checkpoint_v3_metadata
      WHERE checkpoint_id = ? AND run_id = ? AND generation_hex = ? AND completed_step_hex = ?`)
      .get(selected.logicalRootSha256, commit.sourceRunId, selected.generation, selected.completedStep) as { descriptor_json: string | null } | undefined;
    if (!retained?.descriptor_json || JSON.stringify(parseManagedCheckpointDescriptor(JSON.parse(retained.descriptor_json))) !== JSON.stringify(selected)) {
      throw new Error('recovered descriptor differs from retained source metadata');
    }
    const invalidChronology = db.prepare(`SELECT 1 FROM rust_checkpoint_v3_metadata WHERE run_id = ? AND
      (length(generation_hex) != 16 OR generation_hex GLOB '*[^0-9a-f]*') LIMIT 1`).get(commit.sourceRunId);
    if (invalidChronology) throw new Error('failed lineage has invalid retained chronology');
    const newest = db.prepare('SELECT max(generation_hex) AS generation FROM rust_checkpoint_v3_metadata WHERE run_id = ?')
      .get(commit.sourceRunId) as { generation: string };
    const result = parseRecoveryBranchResult({ ...commit, abandonedThroughGeneration: newest.generation });
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
    CREATE TABLE IF NOT EXISTS rust_hall_of_fame_v1 (
      run_id TEXT NOT NULL,
      generation_hex TEXT NOT NULL,
      checkpoint_id TEXT NOT NULL UNIQUE REFERENCES rust_checkpoint_v3_metadata(checkpoint_id),
      record_version INTEGER NOT NULL,
      record_blob BLOB NOT NULL CHECK(length(record_blob) = 56),
      created_at_ms INTEGER NOT NULL,
      PRIMARY KEY (run_id, generation_hex)
    );
  `);
}

/**
 * Convert one exact hexadecimal descriptor value to bigint for non-lexicographic comparisons.
 * @param value - Canonical fixed-width unsigned-64-bit hexadecimal value.
 * @returns Exact bigint value.
 */
function u64HexToBigInt(value: U64Hex): bigint {
  return BigInt(`0x${value}`);
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
 * @param descriptor - Strict descriptor whose basename and byte length are checked.
 */
function verifyManagedFile(descriptor: ManagedCheckpointDescriptor): VerifiedManagedFile {
  const candidate = resolve(managedRootPath, descriptor.relativeFilename);
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
  const expectedBytes = u64HexToBigInt(descriptor.storedByteCount);
  if (statSync(candidate, { bigint: true }).size !== expectedBytes) {
    throw new RangeError('managed checkpoint file size does not match storedByteCount');
  }
  return { path: candidate, expectedBytes };
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
  const aliased = branch !== undefined && branch.sourceRunId === current.metadata_run_id &&
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

/** Read one exact current target without choosing arbitrarily among multiple runs. */
function selectManagedCheckpoint(runId: string | null): ManagedCheckpointSelection {
  return db.transaction(() => {
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
      if (rows.length === 0) return { descriptor: null, runId: null, recovery: null };
      if (rows.length !== 1) throw new Error('multiple current runs require an explicit run selection');
      selectedRun = rows[0]!.run_id;
      if (!selectedRun) throw new Error('current checkpoint has an invalid run identity');
    }
    const current = readCurrentPointer(selectedRun);
    if (!current) return { descriptor: null, runId: null, recovery: null };
    const descriptor = validateCurrentPointerIdentity(selectedRun, current);
    assertDescriptorBounds(descriptor);
    return { descriptor, runId: selectedRun, recovery: readRecoveryBranch(selectedRun) ?? null };
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
  assertDescriptorBounds(descriptor);
  const descriptorJson = serializeDescriptor(descriptor);
  const summaryRecord = generationCommit === null
    ? null
    : encodeGenerationSummary(generationCommit.summary);
  const hallOfFameRecord = generationCommit === null
    ? null
    : encodeHallOfFameReference(generationCommit.hallOfFame);
  const managedFile = verifyManagedFile(descriptor);
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
        `SELECT run_id, generation_hex, record_version, record_blob
         FROM rust_hall_of_fame_v1 WHERE checkpoint_id = ?`
      ).get(candidate.logicalRootSha256) as ExistingHallOfFameRow | undefined;
      if ((hallOfFameRecord === null && existingHallOfFame) ||
        (hallOfFameRecord !== null && (!existingHallOfFame ||
          existingHallOfFame.run_id !== candidate.runId ||
          existingHallOfFame.generation_hex !== generationCommit?.hallOfFame.completedGeneration ||
          existingHallOfFame.record_version !== 1 ||
          !Buffer.isBuffer(existingHallOfFame.record_blob) ||
          !existingHallOfFame.record_blob.equals(hallOfFameRecord)))) {
        throw new Error('operationId conflicts with a different Hall-of-Fame reference');
      }
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
    if (generationCommit !== null && summaryRecord !== null && hallOfFameRecord !== null) {
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
          run_id, generation_hex, checkpoint_id, record_version, record_blob, created_at_ms
        ) VALUES (
          @runId, @completedGeneration, @checkpointId, 1, @hallOfFameRecord, @createdAtMs
        )
      `).run({
        runId: candidate.runId,
        completedGeneration: generationCommit.hallOfFame.completedGeneration,
        checkpointId: candidate.logicalRootSha256,
        hallOfFameRecord,
        createdAtMs: Date.now()
      });
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
  if (request['type'] === 'selectManagedCheckpoint') {
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
