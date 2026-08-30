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
const db = new Database(bootstrap.databasePath);

db.pragma('journal_mode = WAL');
db.pragma('synchronous = FULL');
const journalMode = db.pragma('journal_mode', { simple: true });
const synchronous = db.pragma('synchronous', { simple: true });
if (String(journalMode).toLowerCase() !== 'wal' || Number(synchronous) !== 2) {
  throw new Error('checkpoint persistence worker requires journal_mode=WAL and synchronous=FULL');
}
db.pragma('foreign_keys = ON');
initializeSchema(db);

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
  if (keys.length !== 3 || !Object.hasOwn(data, 'databasePath') || !Object.hasOwn(data, 'managedRootPath') ||
    !Object.hasOwn(data, 'limits')) {
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
      current.run_id AS pointer_run_id,
      current.checkpoint_id AS pointer_checkpoint_id,
      current.transition_epoch AS pointer_transition_epoch,
      current.operation_id AS pointer_operation_id,
      metadata.run_id AS metadata_run_id,
      metadata.transition_epoch AS metadata_transition_epoch,
      metadata.operation_id AS metadata_operation_id,
      metadata.generation_hex,
      metadata.completed_step_hex,
      metadata.descriptor_json
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
  if (current.pointer_run_id !== expectedRunId ||
    current.metadata_run_id !== current.pointer_run_id ||
    current.metadata_transition_epoch !== current.pointer_transition_epoch ||
    current.metadata_operation_id !== current.pointer_operation_id ||
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
