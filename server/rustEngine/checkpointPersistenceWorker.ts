import { lstatSync, realpathSync, statSync } from 'node:fs';
import { dirname, resolve, sep } from 'node:path';
import { parentPort, workerData } from 'node:worker_threads';
import Database from 'better-sqlite3';
import {
  parseManagedCheckpointDescriptor,
  parseManagedCheckpointDescriptorLimits,
  type CheckpointOperationId,
  type CheckpointPersistenceWorkerResponse,
  type ManagedCheckpointDescriptor,
  type ManagedCheckpointDescriptorLimits,
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
  /** Existing exact transition epoch. */
  transition_epoch: string;
}

/** Existing immutable metadata row used for idempotent replay checks. */
interface ExistingDescriptorRow {
  /** Exact descriptor JSON stored on the original commit. */
  descriptor_json: string;
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
 * Create only the isolated Stage 3 v3 metadata and per-run pointer tables.
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
 * Commit a verified descriptor and monotonic per-run current pointer atomically.
 * @param descriptor - Strict descriptor whose final managed file already exists.
 * @returns Matching commit acknowledgement fields.
 */
function commitManagedCheckpoint(descriptor: ManagedCheckpointDescriptor): {
  operationId: CheckpointOperationId;
  transitionEpoch: U64Hex;
  runId: string;
  checkpointId: string;
} {
  assertDescriptorBounds(descriptor);
  const descriptorJson = serializeDescriptor(descriptor);
  const managedFile = verifyManagedFile(descriptor);
  const commit = db.transaction((candidate: ManagedCheckpointDescriptor) => {
    const existingOperation = db.prepare(
      'SELECT descriptor_json FROM rust_checkpoint_v3_metadata WHERE operation_id = ?'
    ).get(candidate.operationId) as ExistingDescriptorRow | undefined;
    if (existingOperation) {
      if (existingOperation.descriptor_json !== descriptorJson) {
        throw new Error('operationId conflicts with a different immutable checkpoint descriptor');
      }
      const current = db.prepare(
        'SELECT transition_epoch FROM rust_checkpoint_v3_current WHERE run_id = ? AND checkpoint_id = ? AND operation_id = ?'
      ).get(candidate.runId, candidate.logicalRootSha256, candidate.operationId) as CurrentPointerRow | undefined;
      if (!current || current.transition_epoch !== candidate.transitionEpoch) {
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
    const current = db.prepare(
      'SELECT transition_epoch FROM rust_checkpoint_v3_current WHERE run_id = ?'
    ).get(candidate.runId) as CurrentPointerRow | undefined;
    const nextEpoch = u64HexToBigInt(candidate.transitionEpoch);
    if (current) {
      const currentEpoch = u64HexToBigInt(current.transition_epoch);
      if (nextEpoch <= currentEpoch) {
        throw new Error('transitionEpoch is stale and must not regress the current pointer');
      }
      if (nextEpoch !== currentEpoch + 1n) {
        throw new Error('transitionEpoch must advance the per-run current pointer by exactly one');
      }
    } else if (nextEpoch !== 1n) {
      throw new Error('first per-run transitionEpoch must be exactly one');
    }
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
    checkpointId: descriptor.logicalRootSha256
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
    if (request['type'] !== 'commitManagedCheckpoint' || Object.keys(request).length !== 2) {
      throw new TypeError('worker request has an unsupported type or unknown fields');
    }
    const descriptor = parseManagedCheckpointDescriptor(request['descriptor']);
    const committed = commitManagedCheckpoint(descriptor);
    post({ type: 'managedCheckpointCommitted', ...committed });
  } catch (error) {
    post({ type: 'managedCheckpointRejected', operationId, reason: rejectionReason(error) });
  }
});
