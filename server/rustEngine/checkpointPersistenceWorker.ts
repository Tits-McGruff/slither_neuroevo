import Database from 'better-sqlite3';
import { createHash } from 'node:crypto';
import { closeSync, fsyncSync, lstatSync, openSync, readdirSync, readSync, realpathSync, renameSync, statSync, unlinkSync, writeSync, type Dirent } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { parentPort, workerData } from 'node:worker_threads';
import type { SavedBrainCatalogResult } from '../protocol.ts';
import { validateCheckpointDatabaseSchema } from './checkpointDatabaseSchema.ts';
import {
  managedCheckpointContentsEqual,
  parseManagedCheckpointDescriptor,
  parseManagedCheckpointDescriptorLimits,
  parseManagedGenerationCommit,
  parseManagedHallOfFameWeightsDescriptor,
  parseManagedImportBranchResult,
  parseManagedImportInventoryDescriptor,
  parseManagedLegacyConversion,
  parseManagedLegacySnapshotSelection,
  parseSavedBrainArchiveEntries,
  type CheckpointOperationId,
  type CheckpointPersistenceWorkerResponse,
  type ManagedCheckpointDescriptor,
  type ManagedCheckpointDescriptorLimits,
  type ManagedCheckpointSelection,
  type ManagedExportInventoryDescriptor,
  type ManagedGenerationCommit,
  type ManagedGenerationSummary,
  type ManagedHallOfFameReference,
  type ManagedHallOfFameSelection,
  type ManagedHallOfFameWeightsDescriptor,
  type ManagedImportBranchResult,
  type ManagedImportInventoryDescriptor,
  type ManagedLegacyConversion,
  type ManagedLegacySnapshotSelection,
  type ManagedStorageDiagnostics,
  type U64Hex
} from './checkpointPersistenceProtocol.ts';
import {
  automaticCapWithPhysicalReserve,
  buildCheckpointRetentionInventory,
  OWNER_CHECKPOINT_RETENTION_DEFAULTS,
  selectManagedCheckpointRetention,
  type CheckpointRetentionCandidate,
  type CheckpointRetentionDecision,
  type CheckpointRetentionInventory
} from './checkpointRetention.ts';
import { encodeExperimentRecord, validateExperimentBytes } from './experimentRecords.ts';
import { boundaryHallOfFameSelection, exportLineageFilter, readImportBranch, readLineageBranch, readRecoveryBranch, recoveryLineage, recoveryLineageFilter } from './lineage_reads.ts';
import { assertDescriptorBounds, f64HexToNumber, recheckManagedFile, resolveManagedRoot, serializeDescriptor, u64HexToBigInt, verifyHallOfFameWeightsFile, verifyManagedDirectFile, verifyManagedFile, type VerifiedManagedFile } from './managed_files.ts';
import { listGraphPresets, loadGraphPreset, readBrowserHallOfFame, readBrowserHistory, saveGraphPreset } from './read_models.ts';
import { parseRecoveryBranchCommit, parseRecoveryBranchResult, parseRecoveryScanCursor, type RecoveryBranchCommit, type RecoveryBranchResult, type RecoveryScanCursor, type RecoveryScanResult } from './recoveryProtocol.ts';
import { SavedBrainCatalog } from './savedBrainCatalog.ts';
import { createSavedBrainNativeCodec } from './savedBrainNativeCodec.ts';
import { parseSavedBrainOperation } from './savedBrainProtocol.ts';
import { initializeGraphPresetSchema, initializeHallOfFameWeightsSchema, initializeImportableHistorySchema, initializeLegacyConversionSchema, initializeRecoverySchema, initializeRetentionSchema, initializeSchema } from './schema.ts';

/** Maximum text length returned to the client for any worker rejection. */
const MAX_REJECTION_REASON_BYTES = 1024;
/** Fixed inventory header bytes: 16-byte magic plus two little-endian u64 counts. */
const EXPORT_INVENTORY_HEADER_BYTES = 32;
/** Fixed bytes for one Hall-of-Fame record, digest, encoding tag, padding, and counts. */
const EXPORT_INVENTORY_HALL_OF_FAME_BYTES = 120;
/** Practical hard ceiling for complete-generation records in one ordinary export. */
const MAX_EXPORT_GENERATIONS = 1_000_000n;
/** Owner-selected number of best unique unpinned Hall-of-Fame genomes. */
const MAX_HALL_OF_FAME_UNIQUE_GENOMES = 50;

/** Worker bootstrap data owned by the client and structured-cloned at spawn time. */
interface CheckpointPersistenceWorkerData {
  /** Disposable or otherwise explicitly selected SQLite metadata database path. */
  databasePath: string;
  /** Require an existing store; managed mode rejects legacy stores before any schema/journal writes. */
  existingOnly: boolean | 'managed';
  /** Existing server-controlled root containing immutable checkpoint-v3 files. */
  managedRootPath: string;
  /** Selected unpinned automatic and physical store cap. */
  automaticByteCapBytes: bigint;
  /** Exact bounded descriptor limits selected before the worker starts. */
  limits: ManagedCheckpointDescriptorLimits;
  /** One-shot test fault around the real FULL-commit transaction. */
  checkpointCommitFailpointForTesting?: 'before-commit' | 'after-commit-before-reply';
  /** One-shot test fault around a managed-import FULL-commit transaction. */
  importCommitFailpointForTesting?: 'before-commit' | 'after-commit-before-reply';
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

/** Parent port required by the worker-thread-only module. */
if (!parentPort) throw new Error('checkpointPersistenceWorker requires parentPort');
/** Non-null parent port after the worker-context assertion. */
const port = parentPort;
/** Immutable bootstrap data supplied by the client. */
const bootstrap = parseWorkerData(workerData);
/** One-shot injected failure, never supplied by normal production startup. */
let checkpointCommitFailpoint = bootstrap.checkpointCommitFailpointForTesting;
/** Test-only one-shot managed-import transaction fault. */
let importCommitFailpoint = bootstrap.importCommitFailpointForTesting;
/** Canonical real managed root used to reject traversal and symlinks. */
const managedRootPath = resolveManagedRoot(bootstrap.managedRootPath);
/** Immutable bounded descriptor limits selected before this worker accepts messages. */
const descriptorLimits = bootstrap.limits;
/** Owner-selected cap shared by retention inventory and physical admission. */
const retentionSettings = { ...OWNER_CHECKPOINT_RETENTION_DEFAULTS,
  automaticByteCap: bootstrap.automaticByteCapBytes };
/** Single synchronous SQLite connection owned exclusively by this worker. */
const db = new Database(bootstrap.databasePath, { fileMustExist: bootstrap.existingOnly !== false });
const existingDatabaseKind = bootstrap.existingOnly ? (() => {
  try {
    const kind = validateCheckpointDatabaseSchema(db);
    if (bootstrap.existingOnly === 'managed' && kind !== 'managed') {
      throw new Error('fresh startup requires a compatible managed checkpoint database; legacy stores require explicit resume/conversion');
    }
    return kind;
  }
  catch (error) { db.close(); throw error; }
})() : 'new';

db.pragma('journal_mode = WAL');
db.pragma('synchronous = FULL');
const journalMode = db.pragma('journal_mode', { simple: true });
const synchronous = db.pragma('synchronous', { simple: true });
if (String(journalMode).toLowerCase() !== 'wal' || Number(synchronous) !== 2) {
  throw new Error('checkpoint persistence worker requires journal_mode=WAL and synchronous=FULL');
}
db.pragma('foreign_keys = ON');
if (!bootstrap.existingOnly || existingDatabaseKind === 'legacy') initializeSchema(db);
initializeGraphPresetSchema(db);
initializeHallOfFameWeightsSchema(db, rebuildHallOfFameRetention);
/** Explicit library shares the metadata connection and immutable HoF weight store. */
const savedBrainCatalog = new SavedBrainCatalog(db, managedRootPath, createSavedBrainNativeCodec());
initializeImportableHistorySchema(db);
initializeRecoverySchema(db);
initializeRetentionSchema(db);
initializeLegacyConversionSchema(db);
/** One exact in-process export reference; worker shutdown cancels it implicitly. */
let activeExportLease: {
  operationId: CheckpointOperationId;
  checkpointId: string;
  inventoryPath: string;
} | undefined;
/** One packed winner protected while Rust verifies and consumes it. */
let activeHallOfFameLease: { operationId: CheckpointOperationId; logicalSha256: string } | undefined;
/** Request currently executing synchronously on this isolated worker. */
let activeRequestOperationId: CheckpointOperationId | undefined;
/** Monotonic bounded work counter for the active request. */
let activeRequestCompletedUnits = 0n;
/** Existing stores scan once at the explicit startup selection barrier, never beside publication. */
let startupOrphansScanned = !bootstrap.existingOnly;

/** Publish progress that the parent watchdog can observe while this thread is busy. */
function reportPersistenceProgress(completedUnits: bigint): void {
  if (!activeRequestOperationId || completedUnits < 0n || completedUnits > 0xffff_ffff_ffff_ffffn) {
    throw new RangeError('invalid persistence progress state');
  }
  activeRequestCompletedUnits = completedUnits;
  post({
    type: 'persistenceProgress',
    operationId: activeRequestOperationId,
    completedUnits: completedUnits.toString(16).padStart(16, '0')
  });
}

/** Advance and publish one bounded unit of long-running file or row work. */
function advancePersistenceProgress(): void {
  reportPersistenceProgress(activeRequestCompletedUnits + 1n);
}

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
  if (keys.length < 5 || keys.length > 7 ||
    keys.some(key => !['databasePath', 'managedRootPath', 'limits', 'existingOnly', 'automaticByteCapBytes',
      'checkpointCommitFailpointForTesting', 'importCommitFailpointForTesting'].includes(key)) ||
    !Object.hasOwn(data, 'databasePath') || !Object.hasOwn(data, 'managedRootPath') ||
    !Object.hasOwn(data, 'limits') || (typeof data['existingOnly'] !== 'boolean' && data['existingOnly'] !== 'managed') ||
    typeof data['automaticByteCapBytes'] !== 'bigint' ||
    data['automaticByteCapBytes'] < 1n || data['automaticByteCapBytes'] > 0xffff_ffff_ffff_ffffn) {
    throw new TypeError('checkpoint persistence worker data has unknown or missing fields');
  }
  if (Object.hasOwn(data, 'checkpointCommitFailpointForTesting') &&
    data['checkpointCommitFailpointForTesting'] !== 'before-commit' &&
    data['checkpointCommitFailpointForTesting'] !== 'after-commit-before-reply') {
    throw new TypeError('invalid checkpoint commit test failpoint');
  }
  if (Object.hasOwn(data, 'importCommitFailpointForTesting') &&
    data['importCommitFailpointForTesting'] !== 'before-commit' &&
    data['importCommitFailpointForTesting'] !== 'after-commit-before-reply') {
    throw new TypeError('invalid import commit test failpoint');
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
    automaticByteCapBytes: data['automaticByteCapBytes'],
    limits: parseManagedCheckpointDescriptorLimits(data['limits']),
    ...(data['checkpointCommitFailpointForTesting']
      ? { checkpointCommitFailpointForTesting: data['checkpointCommitFailpointForTesting'] as
          'before-commit' | 'after-commit-before-reply' }
      : {}),
    ...(data['importCommitFailpointForTesting']
      ? { importCommitFailpointForTesting: data['importCommitFailpointForTesting'] as
          'before-commit' | 'after-commit-before-reply' }
      : {})
  };
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

/** Commit lineage, source-history prefix reference, and active pointer in one FULL transaction. */
function commitRecoveryBranch(value: RecoveryBranchCommit): RecoveryBranchResult {
  const commit = parseRecoveryBranchCommit(value);
  const selected = commit.recoveredDescriptor;
  assertDescriptorBounds(descriptorLimits, selected);
  const file = verifyManagedFile(managedRootPath, selected);
  return db.transaction(() => {
    const replay = readRecoveryBranch(db, commit.branchRunId);
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
    if (commit.explicitResume) {
      const expected = commit.explicitResume;
      const active = db.prepare('SELECT 1 FROM rust_active_run_v1 WHERE singleton = 1 AND run_id = ?').get(expected.activeRunId);
      const pointer = readCurrentPointer(expected.activeRunId);
      if (!active || pointer?.pointer_checkpoint_id !== expected.activeCheckpointId) {
        throw new Error('active pointer changed during explicit checkpoint resume');
      }
      if (!db.prepare(`SELECT 1 FROM rust_checkpoint_retention_v1 WHERE checkpoint_id = ?
          AND retention_kind IN ('automatic', 'pinned')`).get(selected.logicalRootSha256)) {
        throw new Error('explicit checkpoint is no longer retained');
      }
    } else {
      const active = db.prepare('SELECT 1 FROM rust_active_run_v1 WHERE singleton = 1 AND run_id != ?').get(commit.sourceRunId);
      if (active) throw new Error('recovery source is no longer active');
    }
    const lineage = recoveryLineage(db, commit.sourceRunId);
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
    const inherited = readLineageBranch(db, commit.sourceRunId)?.recoveredDescriptor.generation ?? null;
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
function cleanupUnreferencedHallOfFameWeights(): { files: number; storedBytes: bigint } {
  const result = { files: 0, storedBytes: 0n };
  if (activeExportLease || activeHallOfFameLease || savedBrainCatalog.hasLeases) return result;
  const nextPage = db.prepare(`SELECT logical_sha256, relative_filename, encoding,
    stored_byte_count_hex, decoded_byte_count_hex, weight_count_hex
    FROM rust_hall_of_fame_weights_v1 AS weights
    WHERE logical_sha256 > ? AND NOT EXISTS (
      SELECT 1 FROM rust_hall_of_fame_v1 AS hall
      WHERE hall.weights_sha256 = weights.logical_sha256
    ) AND NOT EXISTS (SELECT 1 FROM rust_saved_brains_v1 AS brain
      WHERE brain.weights_sha256 = weights.logical_sha256)
    ORDER BY logical_sha256 LIMIT 64`);
  type UnreferencedWeightsRow = {
    logical_sha256: string;
    relative_filename: string;
    encoding: string;
    stored_byte_count_hex: string;
    decoded_byte_count_hex: string;
    weight_count_hex: string;
  };
  let visited = 0;
  let cursor = '';
  for (;;) {
    const rows = nextPage.all(cursor) as UnreferencedWeightsRow[];
    if (rows.length === 0) break;
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
        const file = verifyManagedDirectFile(managedRootPath, row.relative_filename, storedBytes);
        unlinkSync(file.path);
        result.files++;
        result.storedBytes += storedBytes;
      } catch (error) {
        if (!(error && typeof error === 'object' && (error as NodeJS.ErrnoException).code === 'ENOENT')) {
          continue;
        }
      }
      db.prepare(`DELETE FROM rust_hall_of_fame_weights_v1
        WHERE logical_sha256 = ? AND NOT EXISTS (
          SELECT 1 FROM rust_hall_of_fame_v1 WHERE weights_sha256 = ?
        ) AND NOT EXISTS (SELECT 1 FROM rust_saved_brains_v1 WHERE weights_sha256 = ?)
        `).run(row.logical_sha256, row.logical_sha256, row.logical_sha256);
      visited++;
      if (visited % 64 === 0) advancePersistenceProgress();
    }
    cursor = rows[rows.length - 1]!.logical_sha256;
  }
  return result;
}

/**
 * Verify every retained final object, then remove only exact unreferenced final-file names.
 * @returns Exact file and byte counts reclaimed during this startup pass.
 */
function scavengeUnreferencedManagedFiles(): {
  completed: boolean;
  checkpointFiles: number;
  hallOfFameFiles: number;
  savedBrainFiles: number;
  storedBytes: bigint;
} {
  if (activeExportLease || activeHallOfFameLease || savedBrainCatalog.hasLeases) {
    return { completed: false, checkpointFiles: 0, hallOfFameFiles: 0, savedBrainFiles: 0, storedBytes: 0n };
  }
  const referenced = new Set<string>();
  try {
    const checkpointRows = db.prepare(`SELECT
      CASE WHEN length(CAST(metadata.descriptor_json AS BLOB)) <= 16384
        THEN metadata.descriptor_json END AS descriptor_json,
      retention.retention_kind
      FROM rust_checkpoint_v3_metadata AS metadata
      JOIN rust_checkpoint_retention_v1 AS retention USING(checkpoint_id)
      WHERE retention.retention_kind IN ('automatic', 'pinned', 'pruning')
      ORDER BY metadata.rowid`).all() as Array<{
        descriptor_json: string | null;
        retention_kind: string;
      }>;
    let verifiedCheckpoints = 0;
    for (const row of checkpointRows) {
      if (row.descriptor_json === null ||
          !['automatic', 'pinned', 'pruning'].includes(row.retention_kind)) {
        throw new Error('managed orphan scan found invalid checkpoint metadata');
      }
      const descriptor = parseManagedCheckpointDescriptor(JSON.parse(row.descriptor_json));
      assertDescriptorBounds(descriptorLimits, descriptor);
      referenced.add(descriptor.relativeFilename);
      if (row.retention_kind === 'automatic' || row.retention_kind === 'pinned') {
        verifyManagedFile(managedRootPath, descriptor);
      } else if (row.retention_kind === 'pruning') {
        try { verifyManagedFile(managedRootPath, descriptor); }
        catch (error) {
          if (!(error && typeof error === 'object' &&
              (error as NodeJS.ErrnoException).code === 'ENOENT')) throw error;
        }
      }
      verifiedCheckpoints++;
      if (verifiedCheckpoints % 256 === 0) advancePersistenceProgress();
    }

    const weightRows = db.prepare(`SELECT weights.logical_sha256, weights.relative_filename,
      weights.encoding, weights.stored_byte_count_hex, weights.decoded_byte_count_hex,
      weights.weight_count_hex FROM rust_hall_of_fame_weights_v1 AS weights
      WHERE EXISTS (SELECT 1 FROM rust_hall_of_fame_v1 AS hall
        WHERE hall.weights_sha256 = weights.logical_sha256)
        OR EXISTS (SELECT 1 FROM rust_saved_brains_v1 AS brain
          WHERE brain.weights_sha256 = weights.logical_sha256)
      ORDER BY weights.rowid`).all() as Array<{
        logical_sha256: string;
        relative_filename: string;
        encoding: string;
        stored_byte_count_hex: string;
        decoded_byte_count_hex: string;
        weight_count_hex: string;
      }>;
    for (const original of savedBrainCatalog.referencedOriginals()) {
      referenced.add(original.relativeFilename);
    }
    let verifiedWeights = 0;
    for (const row of weightRows) {
      if (!/^[0-9a-f]{64}$/u.test(row.logical_sha256) ||
          row.relative_filename !== `${row.logical_sha256}.hof-weights-v1` ||
          !['raw-f32le-v1', 'f32le-shuffle4-zstd-v1'].includes(row.encoding) ||
          !/^[0-9a-f]{16}$/u.test(row.stored_byte_count_hex) ||
          !/^[0-9a-f]{16}$/u.test(row.decoded_byte_count_hex) ||
          !/^[0-9a-f]{16}$/u.test(row.weight_count_hex)) {
        throw new Error('managed orphan scan found invalid Hall-of-Fame metadata');
      }
      const storedBytes = u64HexToBigInt(row.stored_byte_count_hex as U64Hex);
      const decodedBytes = u64HexToBigInt(row.decoded_byte_count_hex as U64Hex);
      const weightCount = u64HexToBigInt(row.weight_count_hex as U64Hex);
      if (decodedBytes !== weightCount * 4n ||
          (row.encoding === 'raw-f32le-v1' && storedBytes !== decodedBytes)) {
        throw new Error('managed orphan scan found inconsistent Hall-of-Fame counts');
      }
      verifyManagedDirectFile(managedRootPath, row.relative_filename, storedBytes);
      referenced.add(row.relative_filename);
      verifiedWeights++;
      if (verifiedWeights % 256 === 0) advancePersistenceProgress();
    }
  } catch {
    // Recovery and repair must remain available; malformed retained metadata disables deletion.
    return { completed: false, checkpointFiles: 0, hallOfFameFiles: 0, savedBrainFiles: 0, storedBytes: 0n };
  }

  const removedWeights = cleanupUnreferencedHallOfFameWeights();
  const result = {
    completed: true,
    checkpointFiles: 0,
    hallOfFameFiles: removedWeights.files,
    savedBrainFiles: 0,
    storedBytes: removedWeights.storedBytes
  };
  const finalName = /^[0-9a-f]{64}\.(?<kind>checkpoint-v3|hof-weights-v1|slither-brain)$/u;
  let scannedFiles = 0;
  for (const entry of readdirSync(managedRootPath, { withFileTypes: true })) {
    scannedFiles++;
    if (scannedFiles % 256 === 0) advancePersistenceProgress();
    const match = finalName.exec(entry.name);
    if (!match || !entry.isFile() || entry.isSymbolicLink() || referenced.has(entry.name)) continue;
    const path = resolve(managedRootPath, entry.name);
    const metadata = lstatSync(path, { bigint: true });
    if (!metadata.isFile() || metadata.isSymbolicLink() ||
        dirname(realpathSync(path)) !== managedRootPath) continue;
    unlinkSync(path);
    result.storedBytes += metadata.size;
    if (match.groups?.['kind'] === 'checkpoint-v3') result.checkpointFiles++;
    else if (match.groups?.['kind'] === 'slither-brain') result.savedBrainFiles++;
    else result.hallOfFameFiles++;
  }
  return result;
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

/** Apply one bounded desired Hall-of-Fame selection without rewriting old rows. */
function reconcileHallOfFameSelection(
  runId: string,
  desiredRows: readonly HallOfFameRetentionRow[]
): void {
  const current = db.prepare(`SELECT generation_hex, fitness_value, genome_sha256, pinned, weight_state
    FROM rust_hall_of_fame_v1
    WHERE run_id = ? AND weight_state = 'selected'
    ORDER BY generation_hex`).all(runId) as HallOfFameRetentionRow[];
  const desired = new Map<string, string>();
  for (const row of desiredRows) {
    if (!Number.isFinite(row.fitness_value) || row.genome_sha256 === null ||
        ![0, 1].includes(row.pinned)) {
      throw new Error('Hall-of-Fame retention found invalid selected metadata');
    }
    desired.set(row.generation_hex, row.genome_sha256);
  }
  const selected = new Map<string, string>();
  for (const row of current) {
    if (!Number.isFinite(row.fitness_value) || row.genome_sha256 === null ||
        ![0, 1].includes(row.pinned)) {
      throw new Error('Hall-of-Fame retention found invalid current metadata');
    }
    selected.set(row.generation_hex, row.genome_sha256);
  }
  const deselect = db.prepare(`UPDATE rust_hall_of_fame_v1
    SET weights_sha256 = NULL, weight_state = 'unselected'
    WHERE run_id = ? AND generation_hex = ? AND weight_state = 'selected'`);
  for (const [generation, genome] of selected) {
    if (desired.get(generation) === genome) continue;
    if (deselect.run(runId, generation).changes !== 1) {
      throw new Error('Hall-of-Fame retention removal changed during its transaction');
    }
  }
  const select = db.prepare(`UPDATE rust_hall_of_fame_v1
    SET weights_sha256 = ?, weight_state = 'selected'
    WHERE run_id = ? AND generation_hex = ? AND weight_state = 'unselected'`);
  for (const [generation, genome] of desired) {
    if (selected.get(generation) === genome) continue;
    if (select.run(genome, runId, generation).changes !== 1) {
      throw new Error('Hall-of-Fame retention selection changed during its transaction');
    }
  }
}

/** Keep packed weights for the best 50 unique genomes plus every pinned entry. */
function rebuildHallOfFameRetention(runId: string): void {
  // SQLite ranks one representative per genome before applying the small
  // limit. This keeps normal generation commits independent of lifetime
  // history size; only selected rows and explicitly pinned rows reach JS.
  const automatic = db.prepare(`WITH ranked AS (
      SELECT candidate.generation_hex, candidate.fitness_value, candidate.genome_sha256,
        candidate.pinned,
        row_number() OVER (
          PARTITION BY candidate.genome_sha256
          ORDER BY candidate.fitness_value DESC, candidate.generation_hex ASC
        ) AS genome_rank
      FROM rust_hall_of_fame_v1 AS candidate
      WHERE candidate.run_id = ? AND candidate.weight_state != 'legacy'
        AND candidate.genome_sha256 IS NOT NULL
    )
    SELECT generation_hex, fitness_value, genome_sha256, pinned, 'selected' AS weight_state
    FROM ranked WHERE genome_rank = 1
    ORDER BY fitness_value DESC, generation_hex ASC
    LIMIT ?`).all(runId, MAX_HALL_OF_FAME_UNIQUE_GENOMES) as HallOfFameRetentionRow[];
  const pinned = db.prepare(`SELECT generation_hex, fitness_value, genome_sha256, pinned, weight_state
    FROM rust_hall_of_fame_v1
    WHERE run_id = ? AND pinned = 1 AND weight_state != 'legacy'
    ORDER BY generation_hex`).all(runId) as HallOfFameRetentionRow[];
  reconcileHallOfFameSelection(runId, [...pinned, ...automatic]);
}

/**
 * Admit one newly committed genome against only the currently selected set.
 * Historical rows are consulted through the genome index only when they share
 * the new content identity, so normal work does not grow with generation count.
 */
function applyIncrementalHallOfFameRetention(runId: string, newGenomeSha256: string): void {
  const pinned = db.prepare(`SELECT generation_hex, fitness_value, genome_sha256, pinned, weight_state
    FROM rust_hall_of_fame_v1
    WHERE run_id = ? AND pinned = 1 AND weight_state != 'legacy'
    ORDER BY generation_hex`).all(runId) as HallOfFameRetentionRow[];
  const candidates = db.prepare(`SELECT generation_hex, fitness_value, genome_sha256, pinned, weight_state
    FROM rust_hall_of_fame_v1
    WHERE run_id = ? AND weight_state = 'selected' AND pinned = 0
    ORDER BY generation_hex`).all(runId) as HallOfFameRetentionRow[];
  const bestMatching = db.prepare(`SELECT generation_hex, fitness_value, genome_sha256, pinned, weight_state
    FROM rust_hall_of_fame_v1
    WHERE run_id = ? AND genome_sha256 = ? AND weight_state != 'legacy'
    ORDER BY fitness_value DESC, generation_hex ASC LIMIT 1`)
    .get(runId, newGenomeSha256) as HallOfFameRetentionRow | undefined;
  if (!bestMatching) throw new Error('new Hall-of-Fame genome has no ranked metadata');
  candidates.push(bestMatching, ...pinned);
  const unique = new Map<string, HallOfFameRetentionRow>();
  for (const row of candidates) {
    if (!Number.isFinite(row.fitness_value) || row.genome_sha256 === null ||
        ![0, 1].includes(row.pinned) || !['selected', 'unselected'].includes(row.weight_state)) {
      throw new Error('incremental Hall-of-Fame retention found invalid metadata');
    }
    const previous = unique.get(row.genome_sha256);
    if (!previous || row.fitness_value > previous.fitness_value ||
        (row.fitness_value === previous.fitness_value && row.generation_hex < previous.generation_hex)) {
      unique.set(row.genome_sha256, row);
    }
  }
  const automatic = [...unique.values()]
    .sort((left, right) => right.fitness_value - left.fitness_value ||
      (left.generation_hex < right.generation_hex ? -1 : left.generation_hex > right.generation_hex ? 1 : 0))
    .slice(0, MAX_HALL_OF_FAME_UNIQUE_GENOMES);
  reconcileHallOfFameSelection(runId, [...pinned, ...automatic]);
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
  const branch = current.metadata_run_id !== expectedRunId ? readLineageBranch(db, expectedRunId) : undefined;
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

/** Reclaim crash-orphaned finals once, serialized before startup selection and native publication. */
function scavengeStartupOrphansOnce(): void {
  if (startupOrphansScanned) return;
  const cleanup = scavengeUnreferencedManagedFiles();
  startupOrphansScanned = true;
  if (cleanup.checkpointFiles > 0 || cleanup.hallOfFameFiles > 0 || cleanup.savedBrainFiles > 0) {
    console.warn('[rust.persistence] removed unreferenced managed files', {
      checkpointFiles: cleanup.checkpointFiles,
      hallOfFameFiles: cleanup.hallOfFameFiles,
      savedBrainFiles: cleanup.savedBrainFiles,
      storedBytes: cleanup.storedBytes.toString()
    });
  }
}

/** Read one exact current target without choosing arbitrarily among multiple runs. */
function selectManagedCheckpoint(runId: string | null): ManagedCheckpointSelection {
  scavengeStartupOrphansOnce();
  return db.transaction(() => {
    const selectedRun = resolveSelectedRun(runId);
    if (selectedRun === null) return {
      descriptor: null, runId: null, recovery: null, importBranch: null, legacyConversion: null
    };
    const current = readCurrentPointer(selectedRun);
    if (!current) return {
      descriptor: null, runId: null, recovery: null, importBranch: null, legacyConversion: null
    };
    const descriptor = validateCurrentPointerIdentity(selectedRun, current);
    assertDescriptorBounds(descriptorLimits, descriptor);
    return { descriptor, runId: selectedRun, recovery: readRecoveryBranch(db, selectedRun) ?? null,
      importBranch: readImportBranch(db, selectedRun) ?? null,
      legacyConversion: readLegacyConversion(selectedRun) ?? null };
  }).deferred();
}

/** Build the current decision inside a caller-owned SQLite read or write transaction. */
function currentCheckpointRetentionDecision(automaticByteCap = retentionSettings.automaticByteCap): {
  activeRunId: string;
  candidates: CheckpointRetentionCandidate[];
  decision: CheckpointRetentionDecision;
} {
    const activeRunId = resolveSelectedRun(null);
    if (activeRunId === null) throw new Error('retention inventory requires one active managed run');
    const lineage = recoveryLineage(db, activeRunId);
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
    return { activeRunId, candidates, decision: selectManagedCheckpointRetention(
      candidates, activeRunId, { ...retentionSettings, automaticByteCap }
    ) };
}

/** Inspect the owner-approved retention result without deleting files or metadata. */
function inspectCheckpointRetention(): CheckpointRetentionInventory {
  return db.transaction(() => {
    const { activeRunId, decision } = currentCheckpointRetentionDecision();
    return buildCheckpointRetentionInventory(decision, activeRunId, retentionSettings);
  }).deferred();
}

/** Encode one nonnegative bounded counter for the structured-clone protocol. */
function storageU64(value: bigint, label: string): U64Hex {
  if (value < 0n || value > 0xffff_ffff_ffff_ffffn) {
    throw new RangeError(`${label} does not fit an unsigned 64-bit value`);
  }
  return value.toString(16).padStart(16, '0');
}

/** Read one SQLite integer pragma without permitting lossy Number conversion. */
function sqliteCountPragma(name: 'page_size' | 'page_count' | 'freelist_count'): bigint {
  const value = db.pragma(name, { simple: true });
  if (!Number.isSafeInteger(value) || (value as number) < 0) {
    throw new Error(`SQLite ${name} is not a safe nonnegative integer`);
  }
  return BigInt(value as number);
}

/** Read one SQLite file length, treating an absent optional sidecar as empty. */
function sqliteFileByteCount(path: string, optional: boolean): bigint {
  try {
    return statSync(path, { bigint: true }).size;
  } catch (error) {
    if (optional && (error as NodeJS.ErrnoException).code === 'ENOENT') return 0n;
    throw error;
  }
}

/** Inspect bounded file/page counters on the worker that exclusively owns SQLite. */
function inspectManagedStorage(): ManagedStorageDiagnostics {
  const pageSize = sqliteCountPragma('page_size');
  const pageCount = sqliteCountPragma('page_count');
  const freelistPageCount = sqliteCountPragma('freelist_count');
  if (freelistPageCount > pageCount) throw new Error('SQLite freelist exceeds total page count');
  return {
    schemaVersion: 1,
    databaseByteCount: storageU64(sqliteFileByteCount(bootstrap.databasePath, false), 'database bytes'),
    walByteCount: storageU64(sqliteFileByteCount(`${bootstrap.databasePath}-wal`, true), 'WAL bytes'),
    shmByteCount: storageU64(sqliteFileByteCount(`${bootstrap.databasePath}-shm`, true), 'SHM bytes'),
    pageSizeByteCount: storageU64(pageSize, 'page size'),
    pageCount: storageU64(pageCount, 'page count'),
    freelistPageCount: storageU64(freelistPageCount, 'freelist page count'),
    usedPageByteCount: storageU64((pageCount - freelistPageCount) * pageSize, 'used page bytes')
  };
}

/** Count actual managed files and SQLite sidecars before a bounded publication. */
function physicalCheckpointStoreBytes(): bigint {
  /** Export validation briefly extracts one checkpoint into this private directory. */
  const importValidationDirectory = /^\.[0-9a-f]{32}\.import-validation$/u;
  let total = sqliteFileByteCount(bootstrap.databasePath, false) +
    sqliteFileByteCount(`${bootstrap.databasePath}-wal`, true) +
    sqliteFileByteCount(`${bootstrap.databasePath}-shm`, true);
  for (const entry of readdirSync(managedRootPath, { withFileTypes: true })) {
    const path = resolve(managedRootPath, entry.name);
    let metadata: ReturnType<typeof lstatSync>;
    try { metadata = lstatSync(path, { bigint: true }); }
    catch (error) {
      if ((error as NodeJS.ErrnoException).code === 'ENOENT') continue;
      throw error;
    }
    if (metadata.isSymbolicLink()) {
      throw new Error(`managed checkpoint file changed during budget admission: ${entry.name}`);
    }
    if (metadata.isFile()) { total += metadata.size; continue; }
    if (!metadata.isDirectory() || !importValidationDirectory.test(entry.name)) {
      throw new Error(`managed checkpoint directory contains a non-file during budget admission: ${entry.name}`);
    }
    let children: Dirent[];
    try { children = readdirSync(path, { withFileTypes: true }); }
    catch (error) {
      if ((error as NodeJS.ErrnoException).code === 'ENOENT') continue;
      throw error;
    }
    total += metadata.size;
    for (const child of children) {
      const childPath = resolve(path, child.name);
      let childMetadata: ReturnType<typeof lstatSync>;
      try { childMetadata = lstatSync(childPath, { bigint: true }); }
      catch (error) {
        if ((error as NodeJS.ErrnoException).code === 'ENOENT') continue;
        throw error;
      }
      if (!childMetadata.isFile() || childMetadata.isSymbolicLink()) {
        throw new Error(`managed import validation directory contains a non-file: ${child.name}`);
      }
      total += childMetadata.size;
    }
  }
  return total;
}

/** Leave physical room for codec, archive, winner, and WAL before writing any of them. */
function prepublicationAutomaticCap(reserveBytes: bigint): bigint {
  const { candidates } = currentCheckpointRetentionDecision();
  let automaticBytes = 0n;
  let pinnedBytes = 0n;
  for (const candidate of candidates) {
    if (candidate.pinned) pinnedBytes += candidate.storedBytes;
    else automaticBytes += candidate.storedBytes;
  }
  const physicalBytes = physicalCheckpointStoreBytes();
  return automaticCapWithPhysicalReserve(retentionSettings.automaticByteCap,
    physicalBytes, automaticBytes, pinnedBytes, reserveBytes);
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

/** Read and strictly validate one run's legacy-conversion provenance. */
function readLegacyConversion(runId: string): ManagedLegacyConversion | undefined {
  const row = db.prepare(`SELECT source_snapshot_id AS snapshotId, source_format AS sourceFormat,
    completeness FROM rust_legacy_conversions_v1 WHERE run_id = ?`).get(runId) as
    Record<string, unknown> | undefined;
  return row ? parseManagedLegacyConversion(row) : undefined;
}

/** Select one bounded legacy parent and identify the exact population storage Rust must validate. */
function selectLegacySnapshot(): ManagedLegacySnapshotSelection | null {
  const available = db.prepare(`SELECT count(*) AS count FROM sqlite_schema
    WHERE type = 'table' AND name = 'population_snapshots'`).get() as { count: number };
  if (available.count !== 1) return null;
  const columns = new Set((db.prepare('PRAGMA table_info(population_snapshots)').all() as
    Array<{ name: string }>).map(column => column.name));
  if (!['id', 'payload_json'].every(column => columns.has(column))) return null;
  const conditions = ['length(CAST(payload_json AS BLOB)) BETWEEN 1 AND 536870912'];
  if (columns.has('format_version')) {
    conditions.push('(format_version IS NULL OR format_version IN (0, 2))');
    if (columns.has('population_count')) {
      conditions.push(`(format_version IS NULL OR format_version = 0 OR
        population_count BETWEEN 1 AND 300)`);
    } else {
      conditions.push('(format_version IS NULL OR format_version = 0)');
    }
  }
  if (columns.has('boundary_kind')) {
    conditions.push(`(boundary_kind IS NULL OR boundary_kind IN ('run-start', 'generation'))`);
  }
  const formatExpression = columns.has('format_version') ? 'format_version' : 'NULL';
  const blobLengthExpression = columns.has('genomes_blob')
    ? 'length(CAST(genomes_blob AS BLOB))'
    : 'NULL';
  const row = db.prepare(`SELECT id, ${formatExpression} AS format_version,
    ${blobLengthExpression} AS genomes_blob_length FROM population_snapshots
    WHERE ${conditions.join(' AND ')} ORDER BY id DESC LIMIT 1`).get() as {
      id: number;
      format_version: number | null;
      genomes_blob_length: number | null;
    } | undefined;
  if (!row) return null;
  if (!Number.isSafeInteger(row.id) || row.id <= 0) {
    throw new Error('legacy checkpoint has an invalid SQLite row ID');
  }
  const sourceFormat = row.format_version === 2
    ? 'typescript-v2'
    : row.genomes_blob_length !== null && row.genomes_blob_length > 0
      ? 'legacy-gzip'
      : 'legacy-json';
  return parseManagedLegacySnapshotSelection({ snapshotId: row.id, sourceFormat });
}

/** Select and verify one retained packed winner without decoding its weights in Node. */
function selectHallOfFameEntry(runId: string, entryId: U64Hex,
  operationId: CheckpointOperationId): ManagedHallOfFameSelection {
  if (activeHallOfFameLease) throw new Error('another Hall-of-Fame selection is already active');
  return db.transaction(() => {
    const current = readCurrentPointer(runId);
    if (!current) throw new Error('Hall-of-Fame selection requires a current managed run');
    const checkpoint = validateCurrentPointerIdentity(runId, current);
    const selection = boundaryHallOfFameSelection(db, runId, checkpoint.generation);
    const row = db.prepare(`${selection.sql} SELECT hall.record_version, hall.record_blob,
      weights.logical_sha256, weights.relative_filename, weights.encoding,
      weights.stored_byte_count_hex, weights.decoded_byte_count_hex, weights.weight_count_hex
      FROM boundary_hall AS hall
      JOIN rust_hall_of_fame_weights_v1 AS weights ON weights.logical_sha256 = hall.weights_sha256
      WHERE hall.generation_hex = ?`)
      .get(...selection.parameters, entryId) as {
        record_version: number;
        record_blob: Buffer;
        logical_sha256: string;
        relative_filename: string;
        encoding: string;
        stored_byte_count_hex: string;
        decoded_byte_count_hex: string;
        weight_count_hex: string;
      } | undefined;
    if (!row) throw new Error('Hall-of-Fame entry is not retained in the active lineage');
    if (row.record_version !== 1 || !Buffer.isBuffer(row.record_blob) || row.record_blob.length !== 56 ||
        row.record_blob.readBigUInt64LE(0) !== u64HexToBigInt(entryId)) {
      throw new Error('selected Hall-of-Fame record is malformed');
    }
    const hex64 = (offset: number): U64Hex =>
      row.record_blob.readBigUInt64LE(offset).toString(16).padStart(16, '0') as U64Hex;
    const reference: ManagedHallOfFameReference = {
      completedGeneration: entryId,
      sourcePopulationSlot: row.record_blob.readUInt32LE(8).toString(16).padStart(16, '0') as U64Hex,
      successorPopulationSlot: row.record_blob.readUInt32LE(12).toString(16).padStart(16, '0') as U64Hex,
      sourceSnakeId: hex64(16),
      successorGenomeId: hex64(24),
      fitnessF64Hex: hex64(32),
      pointsF64Hex: hex64(40),
      length: hex64(48)
    };
    if (!Number.isFinite(row.record_blob.readDoubleLE(32)) ||
        !Number.isFinite(row.record_blob.readDoubleLE(40)) ||
        u64HexToBigInt(reference.sourcePopulationSlot) >= u64HexToBigInt(checkpoint.populationCount) ||
        u64HexToBigInt(reference.successorPopulationSlot) >= u64HexToBigInt(checkpoint.populationCount) ||
        u64HexToBigInt(reference.sourceSnakeId) === 0n || u64HexToBigInt(reference.successorGenomeId) === 0n) {
      throw new Error('selected Hall-of-Fame reference is invalid');
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
    verifyHallOfFameWeightsFile(managedRootPath, weights);
    activeHallOfFameLease = { operationId, logicalSha256: weights.logicalSha256 };
    return { operationId, runId, entryId, checkpoint, reference, weights };
  }).deferred();
}

/** Release one exact packed winner after Rust consumes or rejects it. */
function releaseHallOfFameEntry(operationId: CheckpointOperationId): void {
  if (!activeHallOfFameLease || activeHallOfFameLease.operationId !== operationId) {
    throw new Error('Hall-of-Fame selection lease is not active');
  }
  activeHallOfFameLease = undefined;
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
  const historyFilter = exportLineageFilter(db, runId, checkpoint.generation, 'history');
  const hallSelection = boundaryHallOfFameSelection(db, runId, checkpoint.generation);
  const expectedCount = historyFilter.completedGenerationCount;
  if (expectedCount < 0n || expectedCount > MAX_EXPORT_GENERATIONS) {
    throw new RangeError('export generation count exceeds the bounded inventory limit');
  }
  const historyCount = BigInt((db.prepare(`SELECT count(*) AS count
    FROM rust_generation_history_v1 AS history WHERE ${historyFilter.sql}`)
    .get(...historyFilter.parameters) as { count: number }).count);
  const hallOfFameCount = BigInt((db.prepare(`${hallSelection.sql} SELECT count(*) AS count
    FROM boundary_hall`)
    .get(...hallSelection.parameters) as { count: number }).count);
  const linkedHallOfFameCount = BigInt((db.prepare(`${hallSelection.sql} SELECT count(*) AS count
    FROM boundary_hall AS hall
    JOIN rust_hall_of_fame_weights_v1 AS weights ON weights.logical_sha256 = hall.weights_sha256`)
    .get(...hallSelection.parameters) as { count: number }).count);
  if (historyCount !== expectedCount || linkedHallOfFameCount !== hallOfFameCount) {
    throw new Error('export requires complete compact history and every selected Hall-of-Fame object');
  }

  const completionCount = (db.prepare('SELECT count(*) AS count FROM rust_generation_history_v1 AS history JOIN rust_generation_completion_v1 AS completion USING(run_id, generation_hex) WHERE ' + historyFilter.sql)
    .get(...historyFilter.parameters) as { count: number }).count;
  const experimentCount = (db.prepare('SELECT count(*) AS count FROM rust_generation_history_v1 AS history JOIN rust_generation_experiment_v1 AS experiment USING(run_id, generation_hex) WHERE ' + historyFilter.sql)
    .get(...historyFilter.parameters) as { count: number }).count;
  const inventoryVersion = experimentCount > 0 ? 3 : completionCount > 0 ? 2 : 1;
  const relativeFilename = '.' + operationId + '.export-inventory-v' + inventoryVersion;
  const finalPath = resolve(managedRootPath, relativeFilename);
  const partialPath = `${finalPath}.partial`;
  let file: number | undefined;
  let ownsPartial = false;
  let ownsFinal = false;
  const hasher = createHash('sha256');
  const write = (bytes: Buffer): void => {
    hasher.update(bytes);
    writeInventoryBytes(file!, bytes);
  };
  try {
    file = openSync(partialPath, 'wx');
    ownsPartial = true;
    const header = Buffer.alloc(EXPORT_INVENTORY_HEADER_BYTES);
    header.write('SLITHER-EXPV' + inventoryVersion, 0, 'ascii');
    header.writeBigUInt64LE(historyCount, 16);
    header.writeBigUInt64LE(hallOfFameCount, 24);
    write(header);

    let expectedGeneration = 1n;
    let historyProgress = 0;
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
      historyProgress++;
      if (historyProgress % 4096 === 0) advancePersistenceProgress();
    }
    if (expectedGeneration !== historyCount + 1n) {
      throw new Error('export compact history count changed during inventory publication');
    }

    let previousHallOfFameGeneration = 0n;
    const hallRows = db.prepare(`${hallSelection.sql} SELECT hall.generation_hex, hall.record_blob,
      weights.logical_sha256, weights.relative_filename, weights.encoding,
      weights.stored_byte_count_hex, weights.decoded_byte_count_hex, weights.weight_count_hex
      FROM boundary_hall AS hall
      JOIN rust_hall_of_fame_weights_v1 AS weights ON weights.logical_sha256 = hall.weights_sha256
      ORDER BY hall.generation_hex`).iterate(...hallSelection.parameters) as Iterable<{
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
      verifyHallOfFameWeightsFile(managedRootPath, weights);
      const record = Buffer.alloc(EXPORT_INVENTORY_HALL_OF_FAME_BYTES);
      row.record_blob.copy(record, 0);
      Buffer.from(weights.logicalSha256, 'hex').copy(record, 56);
      record[88] = weights.encoding === 'raw-f32le-v1' ? 0 : 1;
      record.writeBigUInt64LE(u64HexToBigInt(weights.storedByteCount), 96);
      record.writeBigUInt64LE(u64HexToBigInt(weights.decodedByteCount), 104);
      record.writeBigUInt64LE(u64HexToBigInt(weights.weightCount), 112);
      write(record);
      previousHallOfFameGeneration = generation;
      advancePersistenceProgress();
    }
    if (inventoryVersion >= 2) {
      const rows = db.prepare('SELECT history.generation_hex, completion.record_blob FROM rust_generation_history_v1 AS history LEFT JOIN rust_generation_completion_v1 AS completion USING(run_id, generation_hex) WHERE ' + historyFilter.sql + ' ORDER BY history.generation_hex')
        .iterate(...historyFilter.parameters) as Iterable<{ generation_hex: string; record_blob: Buffer | null }>;
      for (const row of rows) {
        const record = Buffer.alloc(40);
        record.writeBigUInt64LE(u64HexToBigInt(row.generation_hex), 0);
        if (row.record_blob !== null) {
          if (!Buffer.isBuffer(row.record_blob) || row.record_blob.length !== 32) throw new Error('invalid completion sidecar');
          row.record_blob.copy(record, 8);
        }
        write(record);
        advancePersistenceProgress();
      }
    }
    if (inventoryVersion === 3) {
      const rows = db.prepare('SELECT history.generation_hex, experiment.record_blob FROM rust_generation_history_v1 AS history LEFT JOIN rust_generation_experiment_v1 AS experiment USING(run_id, generation_hex) WHERE ' + historyFilter.sql + ' ORDER BY history.generation_hex')
        .iterate(...historyFilter.parameters) as Iterable<{ generation_hex: string; record_blob: Buffer | null }>;
      for (const row of rows) {
        const record = Buffer.alloc(776);
        record.writeBigUInt64LE(u64HexToBigInt(row.generation_hex), 0);
        if (row.record_blob !== null) {
          validateExperimentBytes(row.record_blob);
          row.record_blob.copy(record, 8);
        }
        write(record);
        advancePersistenceProgress();
      }
    }
    fsyncSync(file!);
    closeSync(file!);
    file = undefined;
    // Reserve publication exclusively; rename may replace only our own reservation.
    file = openSync(finalPath, 'wx');
    ownsFinal = true;
    closeSync(file);
    file = undefined;
    renameSync(partialPath, finalPath);
    ownsPartial = false;
    const storedByteCount = BigInt(EXPORT_INVENTORY_HEADER_BYTES) + historyCount * 56n +
      hallOfFameCount * BigInt(EXPORT_INVENTORY_HALL_OF_FAME_BYTES) + (inventoryVersion >= 2 ? historyCount * 40n : 0n) + (inventoryVersion === 3 ? historyCount * 776n : 0n);
    verifyManagedDirectFile(managedRootPath, relativeFilename, storedByteCount);
    return {
      descriptor: {
        version: inventoryVersion,
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
    for (const path of [...(ownsPartial ? [partialPath] : []), ...(ownsFinal ? [finalPath] : [])]) {
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
    verifyManagedFile(managedRootPath, descriptor);
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
}

/** Apply one automatic retention decision while preserving all compact metadata. */
function applyCheckpointRetention(physicalReserveBytes: bigint | null = null): {
  deletedCheckpointCount: number;
  deletedStoredByteCount: U64Hex;
  inventory: CheckpointRetentionInventory;
} {
  // The caller holds the generation before Rust can publish another winner.
  // Never collect at commit entry or lease release: either can race a reused
  // content hash that Rust has published but SQLite has not referenced yet.
  cleanupUnreferencedHallOfFameWeights();
  const automaticByteCap = physicalReserveBytes === null
    ? retentionSettings.automaticByteCap
    : prepublicationAutomaticCap(physicalReserveBytes);
  const descriptors = db.transaction(() => {
    db.prepare(`UPDATE rust_checkpoint_retention_v1 SET retention_kind = 'automatic', classified_at_ms = ?
      WHERE retention_kind = 'pruning' AND checkpoint_id IN (
        SELECT checkpoint_id FROM rust_hall_of_fame_v1 WHERE weight_state = 'legacy'
      )`).run(Date.now());
    const { activeRunId, decision } = currentCheckpointRetentionDecision(automaticByteCap);
    const hasLegacyWinner = db.prepare(`SELECT 1 FROM rust_hall_of_fame_v1
      WHERE checkpoint_id = ? AND weight_state = 'legacy' LIMIT 1`);
    const planned = decision.pruned.filter(candidate =>
      candidate.checkpointId !== activeExportLease?.checkpointId &&
      !hasLegacyWinner.get(candidate.checkpointId)
    );
    // Old runs keep current pointers for convenient resume until their anchor
    // falls outside retention. Detach only those obsolete pointers in the same
    // transaction that records prune intent; never detach the active run.
    for (const candidate of planned) {
      const pointers = db.prepare(`SELECT run_id FROM rust_checkpoint_v3_current
        WHERE checkpoint_id = ?`).all(candidate.checkpointId) as Array<{ run_id: string }>;
      for (const { run_id: runId } of pointers) {
        if (runId === activeRunId) throw new Error('retention attempted to prune the active run pointer');
        const current = readCurrentPointer(runId);
        if (!current || validateCurrentPointerIdentity(runId, current).logicalRootSha256 !== candidate.checkpointId) {
          throw new Error('retention found a changed prior-run pointer');
        }
        const removed = db.prepare(`DELETE FROM rust_checkpoint_v3_current
          WHERE run_id = ? AND checkpoint_id = ?`).run(runId, candidate.checkpointId);
        if (removed.changes !== 1) throw new Error('retention could not detach a prior-run pointer');
      }
    }
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
      const file = verifyManagedFile(managedRootPath, descriptor);
      unlinkSync(file.path);
    } catch (error) {
      if (!(error && typeof error === 'object' && (error as NodeJS.ErrnoException).code === 'ENOENT')) throw error;
    }
    deletedStoredBytes += u64HexToBigInt(descriptor.storedByteCount);
    advancePersistenceProgress();
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
  const inventory = inspectCheckpointRetention();
  if (physicalReserveBytes !== null) {
    const physicalBytes = physicalCheckpointStoreBytes();
    const pinnedBytes = u64HexToBigInt(inventory.pinnedStoredByteCount);
    if (physicalBytes < pinnedBytes ||
        physicalBytes - pinnedBytes + physicalReserveBytes > retentionSettings.automaticByteCap) {
      throw new Error('physical checkpoint budget remains above the publication limit after retention');
    }
  }
  return {
    deletedCheckpointCount: descriptors.length,
    deletedStoredByteCount: deletedStoredBytes.toString(16).padStart(16, '0'),
    inventory
  };
}

/** Select one exact retained root across runs and bind its later commit to both observed pointers. */
function selectRetainedCheckpoint(checkpointId: string): RecoveryScanResult {
  scavengeStartupOrphansOnce();
  return db.transaction(() => {
    const activeRunId = resolveSelectedRun(null);
    const active = activeRunId ? readCurrentPointer(activeRunId) : undefined;
    if (!activeRunId || !active) throw new Error('no active pointer available for explicit checkpoint resume');
    const row = db.prepare(`SELECT metadata.checkpoint_id,
      CASE WHEN length(CAST(metadata.run_id AS BLOB)) <= 256 THEN metadata.run_id END AS run_id,
      CASE WHEN length(CAST(metadata.generation_hex AS BLOB)) <= 16 THEN metadata.generation_hex END AS generation_hex,
      CASE WHEN length(CAST(metadata.operation_id AS BLOB)) <= 32 THEN metadata.operation_id END AS operation_id,
      CASE WHEN length(CAST(metadata.transition_epoch AS BLOB)) <= 16 THEN metadata.transition_epoch END AS transition_epoch,
      CASE WHEN length(CAST(metadata.completed_step_hex AS BLOB)) <= 16 THEN metadata.completed_step_hex END AS completed_step_hex,
      CASE WHEN length(CAST(descriptor_json AS BLOB)) <= 16384 THEN descriptor_json END AS descriptor_json
      FROM rust_checkpoint_v3_metadata AS metadata JOIN rust_checkpoint_retention_v1 AS retention USING(checkpoint_id)
      WHERE metadata.checkpoint_id = ? AND retention.retention_kind IN ('automatic', 'pinned')`)
      .get(checkpointId) as { run_id: string; generation_hex: string; checkpoint_id: string;
        operation_id: string; transition_epoch: string; completed_step_hex: string; descriptor_json: string | null } | undefined;
    if (!row) throw new Error('requested exact checkpoint is not retained');
    const source = readCurrentPointer(row.run_id);
    if (!source) throw new Error('requested exact checkpoint has no source current pointer');
    const cursor = parseRecoveryScanCursor({ sourceRunId: row.run_id,
      failedCheckpointId: source.pointer_checkpoint_id, generation: row.generation_hex, checkpointId,
      explicitResume: { activeRunId, activeCheckpointId: active.pointer_checkpoint_id } });
    if (!row.descriptor_json) throw new Error('requested exact checkpoint has invalid bounded metadata');
    const descriptor = parseManagedCheckpointDescriptor(JSON.parse(row.descriptor_json));
    if (descriptor.runId !== row.run_id || descriptor.generation !== row.generation_hex ||
        descriptor.logicalRootSha256 !== checkpointId || descriptor.operationId !== row.operation_id ||
        descriptor.transitionEpoch !== row.transition_epoch || descriptor.completedStep !== row.completed_step_hex) {
      throw new Error('requested exact checkpoint metadata identity mismatch');
    }
    assertDescriptorBounds(descriptorLimits, descriptor);
    return { cursor, descriptor, issue: null, exhausted: false };
  }).deferred();
}

/** Visit one retained metadata record without materializing the retained population set. */
function scanRecoveryCandidate(value: RecoveryScanCursor | null): RecoveryScanResult {
  if (value?.explicitResume) throw new Error('explicit checkpoint selection cannot continue as an automatic recovery scan');
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
    const filter = recoveryLineageFilter(recoveryLineage(db, activeRun));
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
      assertDescriptorBounds(descriptorLimits, descriptor);
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

/** Insert one new immutable checkpoint row and its automatic retention class. */
function insertCheckpointMetadata(candidate: ManagedCheckpointDescriptor): void {
  const descriptorJson = serializeDescriptor(candidate);
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
}

/** Read exactly one trusted fixed record at an explicit file position. */
function readExactAt(file: number, target: Buffer, position: number): void {
  let offset = 0;
  while (offset < target.length) {
    const count = readSync(file, target, offset, target.length - offset, position + offset);
    if (count === 0) throw new Error('trusted import inventory ended early');
    offset += count;
  }
}

/** Verify the Rust-written import inventory before opening the SQLite transaction. */
function verifyImportInventory(inventory: ManagedImportInventoryDescriptor): VerifiedManagedFile {
  const expectedBytes = u64HexToBigInt(inventory.storedByteCount);
  const verified = verifyManagedDirectFile(managedRootPath, inventory.relativeFilename, expectedBytes);
  const file = openSync(verified.path, 'r');
  try {
    const hasher = createHash('sha256');
    const chunk = Buffer.allocUnsafe(1024 * 1024);
    let position = 0;
    const total = Number(expectedBytes);
    while (position < total) {
      const count = readSync(file, chunk, 0, Math.min(chunk.length, total - position), position);
      if (count === 0) throw new Error('trusted import inventory ended early');
      hasher.update(chunk.subarray(0, count));
      position += count;
      advancePersistenceProgress();
    }
    if (hasher.digest('hex') !== inventory.sha256) {
      throw new Error('trusted import inventory failed its SHA-256 check');
    }
    const header = Buffer.alloc(32);
    readExactAt(file, header, 0);
    if (header.subarray(0, 13).toString('ascii') !== 'SLITHER-EXPV' + inventory.version ||
        header.subarray(13, 16).some(byte => byte !== 0) ||
        header.readBigUInt64LE(16) !== u64HexToBigInt(inventory.historyCount) ||
        header.readBigUInt64LE(24) !== u64HexToBigInt(inventory.hallOfFameCount)) {
      throw new Error('trusted import inventory header is invalid');
    }
    return verified;
  } finally {
    closeSync(file);
  }
}

/** Commit a complete exact archive identity without exposing its records to Node. */
function commitManagedImport(
  descriptor: ManagedCheckpointDescriptor,
  inventory: ManagedImportInventoryDescriptor,
  branchRunId: string | null
): {
  operationId: CheckpointOperationId;
  transitionEpoch: U64Hex;
  runId: string;
  checkpointId: string;
  descriptor: ManagedCheckpointDescriptor;
  importBranch: ManagedImportBranchResult | null;
} {
  assertDescriptorBounds(descriptorLimits, descriptor);
  const managedFile = verifyManagedFile(managedRootPath, descriptor);
  const inventoryFile = verifyImportInventory(inventory);
  const historyCount = u64HexToBigInt(inventory.historyCount);
  const hallOfFameCount = u64HexToBigInt(inventory.hallOfFameCount);
  if (historyCount !== u64HexToBigInt(descriptor.generation) - 1n ||
      historyCount > MAX_EXPORT_GENERATIONS || hallOfFameCount > historyCount) {
    throw new Error('import inventory coverage differs from its checkpoint boundary');
  }
  const file = openSync(inventoryFile.path, 'r');
  try {
    const committed = db.transaction(() => {
      recheckManagedFile(managedFile);
      recheckManagedFile(inventoryFile);
      const conflictingGeneration = db.prepare(`SELECT 1 FROM rust_checkpoint_v3_metadata
        WHERE run_id = ? AND generation_hex = ? AND checkpoint_id != ? LIMIT 1`)
        .get(descriptor.runId, descriptor.generation, descriptor.logicalRootSha256);
      if (conflictingGeneration) {
        throw new Error('import checkpoint generation identity conflicts with different immutable content');
      }
      const futureCheckpoint = db.prepare(`SELECT 1 FROM rust_checkpoint_v3_metadata
        WHERE run_id = ? AND generation_hex > ? LIMIT 1`).get(descriptor.runId, descriptor.generation);
      const targetHistoryGeneration = historyCount.toString(16).padStart(16, '0');
      const futureHistory = db.prepare(`SELECT 1 FROM rust_generation_history_v1
        WHERE run_id = ? AND generation_hex > ? LIMIT 1`).get(descriptor.runId, targetHistoryGeneration);
      const hasRetainedFuture = Boolean(futureCheckpoint || futureHistory);
      if (hasRetainedFuture && branchRunId === null) {
        throw new Error('exact import cannot move a run behind its retained future history; resume it as a branch');
      }
      if (!hasRetainedFuture && branchRunId !== null) {
        throw new Error('import branch mode requires retained future history for the source run');
      }
      if (branchRunId !== null) {
        if (!branchRunId || branchRunId === descriptor.runId || branchRunId.includes('\0') ||
            Buffer.byteLength(branchRunId) > 256 || Buffer.from(branchRunId, 'utf8').toString('utf8') !== branchRunId) {
          throw new Error('import branch run identity is invalid');
        }
        const occupied = db.prepare(`SELECT 1 FROM (
          SELECT run_id FROM rust_checkpoint_v3_metadata WHERE run_id = ?
          UNION ALL SELECT run_id FROM rust_checkpoint_v3_current WHERE run_id = ?
          UNION ALL SELECT run_id FROM rust_generation_history_v1 WHERE run_id = ?
          UNION ALL SELECT run_id FROM rust_hall_of_fame_v1 WHERE run_id = ?
          UNION ALL SELECT branch_run_id AS run_id FROM rust_recovery_branches_v1 WHERE branch_run_id = ?
          UNION ALL SELECT branch_run_id AS run_id FROM rust_import_branches_v1 WHERE branch_run_id = ?
        ) LIMIT 1`).get(branchRunId, branchRunId, branchRunId, branchRunId, branchRunId, branchRunId);
        if (occupied) throw new Error('import branch run identity already exists');
      }

      const existingOperation = db.prepare(`SELECT descriptor_json FROM rust_checkpoint_v3_metadata
        WHERE operation_id = ?`).get(descriptor.operationId) as ExistingDescriptorRow | undefined;
      if (existingOperation && existingOperation.descriptor_json !== serializeDescriptor(descriptor)) {
        throw new Error('import operation conflicts with a different checkpoint');
      }
      const existingCheckpoint = db.prepare(`SELECT descriptor_json FROM rust_checkpoint_v3_metadata
        WHERE checkpoint_id = ?`).get(descriptor.logicalRootSha256) as ExistingDescriptorRow | undefined;
      let committedDescriptor = descriptor;
      if (existingCheckpoint) {
        committedDescriptor = parseManagedCheckpointDescriptor(JSON.parse(existingCheckpoint.descriptor_json));
        if (!managedCheckpointContentsEqual(committedDescriptor, descriptor)) {
          throw new Error('import checkpoint identity conflicts with different immutable content');
        }
        // Import has republished and verified this immutable file. Revive its
        // previous prune classification atomically with the new current pointer,
        // while preserving an owner's pin and rolling back on any later failure.
        db.prepare(`UPDATE rust_checkpoint_retention_v1 SET
          retention_kind = 'automatic', classified_at_ms = ?
          WHERE checkpoint_id = ? AND retention_kind IN ('pruning', 'pruned')`)
          .run(Date.now(), committedDescriptor.logicalRootSha256);
      } else {
        if (existingOperation) throw new Error('import operation already belongs to another checkpoint');
        insertCheckpointMetadata(descriptor);
      }

      const historyRecord = Buffer.alloc(56);
      for (let generation = 1n; generation <= historyCount; generation++) {
        const position = 32 + Number((generation - 1n) * 56n);
        readExactAt(file, historyRecord, position);
        const generationHex = generation.toString(16).padStart(16, '0');
        if (historyRecord.readBigUInt64LE(0) !== generation ||
            [8, 16, 24, 40, 48].some(offset => !Number.isFinite(historyRecord.readDoubleLE(offset)))) {
          throw new Error('import history record is malformed');
        }
        const existing = db.prepare(`SELECT record_version, record_blob FROM rust_generation_history_v1
          WHERE run_id = ? AND generation_hex = ?`).get(descriptor.runId, generationHex) as {
            record_version: number; record_blob: Buffer;
          } | undefined;
        if (existing) {
          if (existing.record_version !== 1 || !Buffer.isBuffer(existing.record_blob) ||
              !existing.record_blob.equals(historyRecord)) {
            throw new Error(`import history identity conflicts at generation ${generation}`);
          }
        } else {
          db.prepare(`INSERT INTO rust_generation_history_v1
            (run_id, generation_hex, checkpoint_id, record_version, record_blob, created_at_ms)
            VALUES (?, ?, NULL, 1, ?, ?)`).run(
            descriptor.runId, generationHex, historyRecord, Date.now()
          );
        }
        if (generation % 4096n === 0n) advancePersistenceProgress();
      }

      if (inventory.version >= 2) {
        const record = Buffer.alloc(40);
        const offset = 32 + Number(historyCount * 56n + hallOfFameCount * 120n);
        for (let generation = 1n; generation <= historyCount; generation++) {
          readExactAt(file, record, offset + Number((generation - 1n) * 40n));
          if (record.readBigUInt64LE(0) !== generation) throw new Error('invalid completion generation');
          const payload = record.subarray(8);
          if (payload.every(byte => byte === 0)) continue;
          if (payload.readUInt32LE(0) !== 1 || payload.readUInt32LE(4) > 2 ||
              !Number.isFinite(payload.readDoubleLE(8)) || payload.readDoubleLE(8) <= 0 ||
              !Number.isFinite(payload.readDoubleLE(16)) || payload.readDoubleLE(16) < 8 || payload.readDoubleLE(16) > 604800 ||
              payload.readBigUInt64LE(24) > u64HexToBigInt(descriptor.completedStep)) throw new Error('invalid completion record');
          const generationHex = generation.toString(16).padStart(16, '0');
          const existing = db.prepare('SELECT record_blob FROM rust_generation_completion_v1 WHERE run_id = ? AND generation_hex = ?')
            .get(descriptor.runId, generationHex) as { record_blob: Buffer } | undefined;
          if (existing && !existing.record_blob.equals(payload)) throw new Error('import completion identity conflicts');
          if (!existing) db.prepare('INSERT INTO rust_generation_completion_v1 (run_id, generation_hex, checkpoint_id, record_blob) VALUES (?, ?, NULL, ?)')
            .run(descriptor.runId, generationHex, payload);
          advancePersistenceProgress();
        }
      }
      const hallRecord = Buffer.alloc(120);
      let previousHallGeneration = 0n;
      for (let ordinal = 0n; ordinal < hallOfFameCount; ordinal++) {
        const position = 32 + Number(historyCount * 56n + ordinal * 120n);
        readExactAt(file, hallRecord, position);
        const generation = hallRecord.readBigUInt64LE(0);
        const generationHex = generation.toString(16).padStart(16, '0');
        const logicalSha256 = hallRecord.subarray(56, 88).toString('hex');
        const encoding = hallRecord[88] === 0 ? 'raw-f32le-v1' :
          hallRecord[88] === 1 ? 'f32le-shuffle4-zstd-v1' : null;
        const weights = parseManagedHallOfFameWeightsDescriptor({
          version: 1,
          logicalSha256,
          relativeFilename: `${logicalSha256}.hof-weights-v1`,
          encoding,
          storedByteCount: hallRecord.readBigUInt64LE(96).toString(16).padStart(16, '0'),
          decodedByteCount: hallRecord.readBigUInt64LE(104).toString(16).padStart(16, '0'),
          weightCount: hallRecord.readBigUInt64LE(112).toString(16).padStart(16, '0')
        }, descriptor);
        if (generation <= previousHallGeneration || generation > historyCount ||
            !Number.isFinite(hallRecord.readDoubleLE(32)) ||
            !Number.isFinite(hallRecord.readDoubleLE(40))) {
          throw new Error('import Hall-of-Fame record is malformed');
        }
        const weightsFile = verifyHallOfFameWeightsFile(managedRootPath, weights);
        recheckManagedFile(weightsFile);
        db.prepare(`INSERT OR IGNORE INTO rust_hall_of_fame_weights_v1 (
          logical_sha256, relative_filename, encoding, stored_byte_count_hex,
          decoded_byte_count_hex, weight_count_hex, created_at_ms
        ) VALUES (?, ?, ?, ?, ?, ?, ?)`).run(
          weights.logicalSha256, weights.relativeFilename, weights.encoding,
          weights.storedByteCount, weights.decodedByteCount, weights.weightCount, Date.now()
        );
        assertStoredHallOfFameWeights(weights);
        const compactRecord = Buffer.from(hallRecord.subarray(0, 56));
        const existing = db.prepare(`SELECT record_version, record_blob, genome_sha256
          FROM rust_hall_of_fame_v1 WHERE run_id = ? AND generation_hex = ?`)
          .get(descriptor.runId, generationHex) as {
            record_version: number; record_blob: Buffer; genome_sha256: string | null;
          } | undefined;
        if (existing) {
          if (existing.record_version !== 1 || !Buffer.isBuffer(existing.record_blob) ||
              !existing.record_blob.equals(compactRecord) || existing.genome_sha256 !== logicalSha256) {
            throw new Error(`import Hall-of-Fame identity conflicts at generation ${generation}`);
          }
        } else {
          db.prepare(`INSERT INTO rust_hall_of_fame_v1 (
            run_id, generation_hex, checkpoint_id, record_version, record_blob,
            weights_sha256, genome_sha256, fitness_value, pinned, weight_state, created_at_ms
          ) VALUES (?, ?, NULL, 1, ?, ?, ?, ?, 0, 'selected', ?)`).run(
            descriptor.runId, generationHex, compactRecord, logicalSha256, logicalSha256,
            hallRecord.readDoubleLE(32), Date.now()
          );
        }
        previousHallGeneration = generation;
        advancePersistenceProgress();
      }
      if (inventory.version === 3) {
        const record = Buffer.alloc(776);
        const offset = 32 + Number(historyCount * (56n + 40n) + hallOfFameCount * 120n);
        let previousStep = 0n;
        for (let generation = 1n; generation <= historyCount; generation++) {
          readExactAt(file, record, offset + Number((generation - 1n) * 776n));
          if (record.readBigUInt64LE(0) !== generation) throw new Error('invalid experiment generation');
          const payload = record.subarray(8);
          const generationHex = generation.toString(16).padStart(16, '0');
          const existing = db.prepare('SELECT record_blob FROM rust_generation_experiment_v1 WHERE run_id = ? AND generation_hex = ?')
            .get(descriptor.runId, generationHex) as { record_blob: Buffer } | undefined;
          if (payload.every(byte => byte === 0)) {
            if (existing) throw new Error('import omitted existing experiment record');
            continue;
          }
          validateExperimentBytes(payload);
          const step = payload.readBigUInt64LE(24);
          if (payload.readBigUInt64LE(16) !== generation || step <= previousStep || step > u64HexToBigInt(descriptor.completedStep)) throw new Error('invalid experiment chronology');
          previousStep = step;
          const completion = db.prepare('SELECT record_blob FROM rust_generation_completion_v1 WHERE run_id = ? AND generation_hex = ?')
            .get(descriptor.runId, generationHex) as { record_blob: Buffer } | undefined;
          if (!completion || BigInt(completion.record_blob.readUInt32LE(4)) !== payload.readBigUInt64LE(32) ||
              completion.record_blob.readBigUInt64LE(8) !== payload.readBigUInt64LE(40) ||
              completion.record_blob.readBigUInt64LE(16) !== payload.readBigUInt64LE(48) ||
              completion.record_blob.readBigUInt64LE(24) !== step) throw new Error('import experiment completion mismatch');
          const hall = db.prepare('SELECT record_blob FROM rust_hall_of_fame_v1 WHERE run_id = ? AND generation_hex = ?')
            .get(descriptor.runId, generationHex) as { record_blob: Buffer } | undefined;
          if (hall && (BigInt(hall.record_blob.readUInt32LE(8)) !== payload.readBigUInt64LE(528) ||
              hall.record_blob.readBigUInt64LE(16) !== payload.readBigUInt64LE(536) ||
              hall.record_blob.readBigUInt64LE(40) !== payload.readBigUInt64LE(672) ||
              hall.record_blob.readBigUInt64LE(48) !== payload.readBigUInt64LE(704))) throw new Error('import experiment physical HoF mismatch');
          if (existing && !existing.record_blob.equals(payload)) throw new Error('import experiment identity conflicts');
          if (!existing) db.prepare('INSERT INTO rust_generation_experiment_v1 (run_id, generation_hex, checkpoint_id, record_blob) VALUES (?, ?, NULL, ?)')
            .run(descriptor.runId, generationHex, payload);
          advancePersistenceProgress();
        }
      }
      if (inventory.savedBrainsJson !== undefined) {
        for (const entry of parseSavedBrainArchiveEntries(JSON.parse(inventory.savedBrainsJson))) {
          const record = savedBrainCatalog.save(resolve(managedRootPath, `${entry.brainId}.slither-brain`),
            entry.name, descriptor.operationId, entry.createdAtMs);
          if (record.brainId !== entry.brainId || record.artifactByteCount !== entry.artifactByteCount) {
            throw new Error('saved brain import original differs from native archive membership');
          }
          advancePersistenceProgress();
        }
      }
      rebuildHallOfFameRetention(descriptor.runId);
      let importBranch: ManagedImportBranchResult | null = null;
      const effectiveRunId = branchRunId ?? descriptor.runId;
      if (branchRunId !== null) {
        importBranch = parseManagedImportBranchResult({
          operationId: descriptor.operationId,
          branchRunId,
          sourceRunId: committedDescriptor.runId,
          sourceGeneration: committedDescriptor.generation,
          sourceCheckpointId: committedDescriptor.logicalRootSha256,
          recoveredDescriptor: committedDescriptor
        });
        const historyThrough = historyCount.toString(16).padStart(16, '0');
        db.prepare(`INSERT INTO rust_import_branches_v1 (branch_run_id, operation_id, source_run_id,
          recovered_checkpoint_id, history_through_generation_hex, provenance_json) VALUES (?, ?, ?, ?, ?, ?)`)
          .run(branchRunId, descriptor.operationId, committedDescriptor.runId,
            committedDescriptor.logicalRootSha256, historyThrough, JSON.stringify(importBranch));
        db.prepare(`INSERT INTO rust_checkpoint_v3_current
          (run_id, checkpoint_id, transition_epoch, operation_id) VALUES (?, ?, ?, ?)`)
          .run(branchRunId, committedDescriptor.logicalRootSha256,
            committedDescriptor.transitionEpoch, descriptor.operationId);
      } else {
        db.prepare(`INSERT INTO rust_checkpoint_v3_current
          (run_id, checkpoint_id, transition_epoch, operation_id) VALUES (?, ?, ?, ?)
          ON CONFLICT(run_id) DO UPDATE SET checkpoint_id = excluded.checkpoint_id,
            transition_epoch = excluded.transition_epoch, operation_id = excluded.operation_id`)
          .run(descriptor.runId, committedDescriptor.logicalRootSha256,
            committedDescriptor.transitionEpoch, committedDescriptor.operationId);
      }
      db.prepare(`INSERT INTO rust_active_run_v1 (singleton, run_id) VALUES (1, ?)
        ON CONFLICT(singleton) DO UPDATE SET run_id = excluded.run_id`).run(effectiveRunId);
      if (importCommitFailpoint === 'before-commit') {
        importCommitFailpoint = undefined;
        throw new Error('injected managed import failure before SQLite commit');
      }
      return {
        operationId: descriptor.operationId,
        transitionEpoch: committedDescriptor.transitionEpoch,
        runId: effectiveRunId,
        checkpointId: committedDescriptor.logicalRootSha256,
        descriptor: committedDescriptor,
        importBranch
      };
    }).immediate();
    if (importCommitFailpoint === 'after-commit-before-reply') {
      importCommitFailpoint = undefined;
      process.exit(87);
    }
    return committed;
  } finally {
    closeSync(file);
    try { unlinkSync(inventoryFile.path); } catch { /* Rust inventory is operation-local. */ }
  }
}

/**
 * Commit a verified descriptor and monotonic per-run current pointer atomically.
 * @param descriptor - Strict descriptor whose final managed file already exists.
 * @param generationCommit - Exact compact result and Hall-of-Fame reference for a generation.
 * @param activateRun - Whether this transaction selects a replacement active lineage.
 * @param legacyConversion - Optional old SQLite population source for a run-start checkpoint.
 * @returns Matching commit acknowledgement fields.
 */
function commitManagedCheckpoint(
  descriptor: ManagedCheckpointDescriptor,
  generationCommit: ManagedGenerationCommit | null,
  activateRun: boolean,
  legacyConversion: ManagedLegacyConversion | null
): {
  operationId: CheckpointOperationId;
  transitionEpoch: U64Hex;
  runId: string;
  checkpointId: string;
  descriptor: ManagedCheckpointDescriptor;
} {
  assertDescriptorBounds(descriptorLimits, descriptor);
  const descriptorJson = serializeDescriptor(descriptor);
  const summaryRecord = generationCommit === null
    ? null
    : encodeGenerationSummary(generationCommit.summary);
  const hallOfFameRecord = generationCommit === null
    ? null
    : encodeHallOfFameReference(generationCommit.hallOfFame);
  const workloadJson = generationCommit?.workloadJson ?? null;
  const experimentRecord = generationCommit?.experiment ? encodeExperimentRecord(generationCommit.experiment) : null;
  const completion = generationCommit?.completion;
  const completionRecord = completion ? Buffer.alloc(32) : null;
  if (completionRecord && completion) {
    completionRecord.writeUInt32LE(1, 0);
    completionRecord.writeUInt32LE(completion.reason === 'manual' ? 0 : completion.reason === 'duration' ? 1 : 2, 4);
    completionRecord.writeBigUInt64LE(BigInt('0x' + completion.actualElapsedF64Hex), 8);
    completionRecord.writeBigUInt64LE(BigInt('0x' + completion.configuredDurationF64Hex), 16);
    completionRecord.writeBigUInt64LE(BigInt('0x' + completion.completedStep), 24);
  }
  const managedFile = verifyManagedFile(managedRootPath, descriptor);
  const hallOfFameWeightsFile = generationCommit === null
    ? null
    : verifyHallOfFameWeightsFile(managedRootPath, generationCommit.hallOfFameWeights);
  if (legacyConversion !== null &&
      (descriptor.boundaryKind !== 'run-start' || generationCommit !== null)) {
    throw new Error('legacy conversion provenance is valid only for a run-start checkpoint');
  }
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
      const existingCompletion = db.prepare('SELECT record_blob FROM rust_generation_completion_v1 WHERE run_id = ? AND generation_hex = ?')
        .get(candidate.runId, generationCommit?.summary.completedGeneration ?? '') as { record_blob: Buffer } | undefined;
      if ((completionRecord === null && existingCompletion) || (completionRecord !== null &&
          (!existingCompletion || !existingCompletion.record_blob.equals(completionRecord)))) {
        throw new Error('operationId conflicts with generation completion chronology');
      }
      const existingExperiment = db.prepare('SELECT record_blob FROM rust_generation_experiment_v1 WHERE run_id = ? AND generation_hex = ?')
        .get(candidate.runId, generationCommit?.summary.completedGeneration ?? '') as { record_blob: Buffer } | undefined;
      if ((experimentRecord === null && existingExperiment) || (experimentRecord !== null &&
          (!existingExperiment || !Buffer.isBuffer(existingExperiment.record_blob) || !existingExperiment.record_blob.equals(experimentRecord)))) {
        throw new Error('operationId conflicts with generation experiment summary');
      }
      const existingWork = db.prepare('SELECT record_json FROM rust_generation_work_v1 WHERE run_id = ? AND generation_hex = ?')
        .get(candidate.runId, generationCommit?.summary.completedGeneration ?? '') as { record_json: string } | undefined;
      if ((existingWork?.record_json ?? null) !== workloadJson) {
        throw new Error('operationId conflicts with generation workload summary');
      }
      if (generationCommit !== null) assertStoredHallOfFameWeights(generationCommit.hallOfFameWeights);
      if (candidate.boundaryKind === 'run-start') {
        const existingLegacyConversion = readLegacyConversion(candidate.runId) ?? null;
        if (existingLegacyConversion?.snapshotId !== legacyConversion?.snapshotId ||
            existingLegacyConversion?.sourceFormat !== legacyConversion?.sourceFormat ||
            existingLegacyConversion?.completeness !== legacyConversion?.completeness) {
          throw new Error('operationId conflicts with different legacy conversion provenance');
        }
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
      if (activateRun) {
        db.prepare(`INSERT INTO rust_active_run_v1 (singleton, run_id) VALUES (1, ?)
          ON CONFLICT(singleton) DO UPDATE SET run_id = excluded.run_id`).run(candidate.runId);
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
    insertCheckpointMetadata(candidate);
    if (generationCommit !== null && summaryRecord !== null && hallOfFameRecord !== null) {
      if (workloadJson !== null) {
        db.prepare('INSERT INTO rust_generation_work_v1 (run_id, generation_hex, checkpoint_id, record_json) VALUES (?, ?, ?, ?)')
          .run(candidate.runId, generationCommit.summary.completedGeneration, candidate.logicalRootSha256, workloadJson);
      }
      if (completionRecord) {
        db.prepare('INSERT INTO rust_generation_completion_v1 (run_id, generation_hex, checkpoint_id, record_blob) VALUES (?, ?, ?, ?)')
          .run(candidate.runId, generationCommit.summary.completedGeneration, candidate.logicalRootSha256, completionRecord);
      }
      if (experimentRecord) {
        db.prepare('INSERT INTO rust_generation_experiment_v1 (run_id, generation_hex, checkpoint_id, record_blob) VALUES (?, ?, ?, ?)')
          .run(candidate.runId, generationCommit.summary.completedGeneration, candidate.logicalRootSha256, experimentRecord);
      }
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
      applyIncrementalHallOfFameRetention(candidate.runId, weights.logicalSha256);
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
    if (legacyConversion !== null) {
      db.prepare(`INSERT INTO rust_legacy_conversions_v1 (
        run_id, source_snapshot_id, source_format, completeness, created_at_ms
      ) VALUES (?, ?, ?, ?, ?)`).run(candidate.runId, legacyConversion.snapshotId,
        legacyConversion.sourceFormat, legacyConversion.completeness, Date.now());
    }
    if (activateRun) {
      db.prepare(`INSERT INTO rust_active_run_v1 (singleton, run_id) VALUES (1, ?)
        ON CONFLICT(singleton) DO UPDATE SET run_id = excluded.run_id`).run(candidate.runId);
    }
    if (checkpointCommitFailpoint === 'before-commit') {
      checkpointCommitFailpoint = undefined;
      throw new Error('injected checkpoint failure before SQLite commit');
    }
  });
  commit(descriptor);
  if (checkpointCommitFailpoint === 'after-commit-before-reply') {
    checkpointCommitFailpoint = undefined;
    process.exit(86);
  }
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
  if (request['type'] === 'selectManagedCheckpoint' || request['type'] === 'selectLegacySnapshot' || request['type'] === 'scanRecoveryCandidate' || request['type'] === 'selectRetainedCheckpoint' ||
      request['type'] === 'inspectCheckpointRetention' || request['type'] === 'inspectManagedStorage' ||
      request['type'] === 'pinCurrentCheckpoint' ||
      request['type'] === 'applyCheckpointRetention' || request['type'] === 'reclaimManagedOrphans' || request['type'] === 'acquireCurrentExportLease' ||
      request['type'] === 'releaseExportLease' || request['type'] === 'readBrowserHistory' ||
      request['type'] === 'readBrowserHallOfFame' || request['type'] === 'selectHallOfFameEntry' ||
      request['type'] === 'savedBrainCatalog' || request['type'] === 'releaseHallOfFameEntry' || request['type'] === 'saveGraphPreset' ||
      request['type'] === 'listGraphPresets' || request['type'] === 'loadGraphPreset') {
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
    activeHallOfFameLease = undefined;
    db.close();
    port.removeAllListeners('message');
    port.close();
    return;
  }
  const operationId = extractOperationId(message);
  if (operationId) {
    activeRequestOperationId = operationId;
    reportPersistenceProgress(0n);
  }
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
    if (request['type'] === 'reclaimManagedOrphans') {
      if (!operationId || Object.keys(request).length !== 2) throw new TypeError('invalid managed orphan cleanup request');
      const result = scavengeUnreferencedManagedFiles();
      post({ type: 'managedOrphansReclaimed', operationId, result: {
        completed: result.completed,
        deletedCheckpointCount: storageU64(BigInt(result.checkpointFiles), 'deleted checkpoint count'),
        deletedHallOfFameCount: storageU64(BigInt(result.hallOfFameFiles), 'deleted elite count'),
        ...(result.savedBrainFiles > 0 ? { deletedSavedBrainCount: storageU64(BigInt(result.savedBrainFiles), 'deleted saved brain count') } : {}),
        deletedStoredByteCount: storageU64(result.storedBytes, 'deleted bytes')
      } });
      return;
    }
    if (request['type'] === 'applyCheckpointRetention') {
      const reserve = request['physicalReserveBytes'];
      if (!operationId || Object.keys(request).length !== 3 ||
          (reserve !== null && (typeof reserve !== 'string' || !/^[0-9a-f]{16}$/u.test(reserve)))) {
        throw new TypeError('invalid retention apply request');
      }
      post({ type: 'checkpointRetentionApplied', operationId,
        result: applyCheckpointRetention(reserve === null ? null : u64HexToBigInt(reserve as U64Hex)) });
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
    if (request['type'] === 'inspectManagedStorage') {
      if (!operationId || Object.keys(request).length !== 2) throw new TypeError('invalid managed storage request');
      post({ type: 'managedStorageInspected', operationId, diagnostics: inspectManagedStorage() });
      return;
    }
    if (request['type'] === 'saveGraphPreset') {
      const name = request['name'];
      const specJson = request['specJson'];
      if (!operationId || Object.keys(request).length !== 4 || typeof name !== 'string' ||
          typeof specJson !== 'string') {
        throw new TypeError('invalid graph preset save request');
      }
      post({ type: 'graphPresetSaved', operationId, presetId: saveGraphPreset(db, name, specJson) });
      return;
    }
    if (request['type'] === 'listGraphPresets') {
      const limit = request['limit'];
      if (!operationId || Object.keys(request).length !== 3 || typeof limit !== 'number') {
        throw new TypeError('invalid graph preset list request');
      }
      post({ type: 'graphPresetsListed', operationId, presets: listGraphPresets(db, limit) });
      return;
    }
    if (request['type'] === 'loadGraphPreset') {
      const presetId = request['presetId'];
      if (!operationId || Object.keys(request).length !== 3 || typeof presetId !== 'number') {
        throw new TypeError('invalid graph preset load request');
      }
      post({ type: 'graphPresetLoaded', operationId, preset: loadGraphPreset(db, presetId) });
      return;
    }
    if (request['type'] === 'readBrowserHistory') {
      const runId = request['runId'];
      const limit = request['limit'];
      if (!operationId || Object.keys(request).length !== 4 || typeof runId !== 'string' ||
          !runId || runId.includes('\0') || Buffer.byteLength(runId) > 256 ||
          typeof limit !== 'number') {
        throw new TypeError('invalid browser history request');
      }
      post({
        type: 'browserHistoryRead', operationId, runId,
        history: readBrowserHistory(db, readCurrentBoundary, runId, limit)
      });
      return;
    }
    if (request['type'] === 'readBrowserHallOfFame') {
      const runId = request['runId'];
      const limit = request['limit'];
      if (!operationId || Object.keys(request).length !== 4 || typeof runId !== 'string' ||
          !runId || runId.includes('\0') || Buffer.byteLength(runId) > 256 ||
          typeof limit !== 'number') {
        throw new TypeError('invalid browser Hall-of-Fame request');
      }
      post({
        type: 'browserHallOfFameRead', operationId, runId,
        entries: readBrowserHallOfFame(db, readCurrentBoundary, runId, limit)
      });
      return;
    }
    if (request['type'] === 'savedBrainCatalog') {
      if (!operationId || Object.keys(request).length !== 3) throw new TypeError('invalid saved brain catalog envelope');
      const operation = parseSavedBrainOperation(request['request']);
      let result: SavedBrainCatalogResult;
      switch (operation.action) {
        case 'save': result = {action: 'save', record: savedBrainCatalog.save(operation.path, operation.name, operationId)}; break;
        case 'read': result = {action: 'read', record: savedBrainCatalog.get(operation.brainId)}; break;
        case 'list': result = {action: 'list', page: savedBrainCatalog.list(operation.after, operation.limit)}; break;
        case 'delete': result = {action: 'delete', brainId: operation.brainId, deleted: savedBrainCatalog.delete(operation.brainId)}; cleanupUnreferencedHallOfFameWeights(); break;
        case 'acquire': result = {action: 'acquire', record: savedBrainCatalog.acquire(operation.brainId, operation.leaseId)}; break;
        case 'acquireArchive': result = {action: 'acquireArchive', entries: savedBrainCatalog.acquireArchive(operation.brainIds, operation.leaseId)}; break;
        case 'release': savedBrainCatalog.release(operation.leaseId); result = {action: 'release', leaseId: operation.leaseId}; cleanupUnreferencedHallOfFameWeights(); break;
      }
      post({type: 'savedBrainCatalogResult', operationId, result});
      return;
    }
    if (request['type'] === 'releaseHallOfFameEntry') {
      if (!operationId || Object.keys(request).length !== 2) {
        throw new TypeError('invalid Hall-of-Fame release request');
      }
      releaseHallOfFameEntry(operationId);
      post({ type: 'hallOfFameEntryReleased', operationId });
      return;
    }
    if (request['type'] === 'selectHallOfFameEntry') {
      const runId = request['runId'];
      const entryId = request['entryId'];
      if (!operationId || Object.keys(request).length !== 4 || typeof runId !== 'string' ||
          !runId || runId.includes('\0') || Buffer.byteLength(runId) > 256 ||
          typeof entryId !== 'string' || !/^[0-9a-f]{16}$/u.test(entryId)) {
        throw new TypeError('invalid Hall-of-Fame selection request');
      }
      post({ type: 'hallOfFameEntrySelected',
        selection: selectHallOfFameEntry(runId, entryId, operationId) });
      return;
    }
    if (request['type'] === 'selectRetainedCheckpoint') {
      const checkpointId = request['checkpointId'];
      if (!operationId || Object.keys(request).length !== 3 || typeof checkpointId !== 'string' ||
          !/^[0-9a-f]{64}$/u.test(checkpointId)) throw new TypeError('invalid exact retained checkpoint request');
      post({ type: 'recoveryCandidate', operationId, result: selectRetainedCheckpoint(checkpointId) });
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
    if (request['type'] === 'selectLegacySnapshot') {
      if (!operationId || Object.keys(request).length !== 2) {
        throw new TypeError('invalid legacy checkpoint selection request');
      }
      post({ type: 'legacySnapshotSelected', operationId, selection: selectLegacySnapshot() });
      return;
    }
    if (request['type'] === 'commitManagedImport') {
      if (Object.keys(request).length !== 4 || !Object.hasOwn(request, 'descriptor') ||
          !Object.hasOwn(request, 'inventory') || !Object.hasOwn(request, 'branchRunId') ||
          (request['branchRunId'] !== null && typeof request['branchRunId'] !== 'string')) {
        throw new TypeError('invalid managed import request');
      }
      const descriptor = parseManagedCheckpointDescriptor(request['descriptor']);
      const inventory = parseManagedImportInventoryDescriptor(
        request['inventory'], descriptor.operationId
      );
      const committed = commitManagedImport(descriptor, inventory, request['branchRunId'] as string | null);
      post({ type: 'managedImportCommitted', ...committed });
      return;
    }
    if (request['type'] !== 'commitManagedCheckpoint' || Object.keys(request).length !== 5 ||
      !Object.hasOwn(request, 'descriptor') || !Object.hasOwn(request, 'generationCommit') ||
      !Object.hasOwn(request, 'legacyConversion') ||
      typeof request['activateRun'] !== 'boolean') {
      throw new TypeError('worker request has an unsupported type or unknown fields');
    }
    const descriptor = parseManagedCheckpointDescriptor(request['descriptor']);
    const generationCommit = parseManagedGenerationCommit(
      request['generationCommit'],
      descriptor
    );
    const legacyConversion = request['legacyConversion'] === null
      ? null : parseManagedLegacyConversion(request['legacyConversion']);
    const committed = commitManagedCheckpoint(
      descriptor, generationCommit, request['activateRun'], legacyConversion
    );
    post({ type: 'managedCheckpointCommitted', ...committed });
  } catch (error) {
    post({ type: 'managedCheckpointRejected', operationId, reason: rejectionReason(error) });
  } finally {
    activeRequestOperationId = undefined;
    activeRequestCompletedUnits = 0n;
  }
});

/** Read and validate the current boundary inside the caller's existing read transaction. */
function readCurrentBoundary(runId: string): ManagedCheckpointDescriptor | undefined {
  const current = readCurrentPointer(runId);
  return current ? validateCurrentPointerIdentity(runId, current) : undefined;
}
