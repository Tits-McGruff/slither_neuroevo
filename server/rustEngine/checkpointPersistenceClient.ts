import { randomBytes } from 'node:crypto';
import { Worker } from 'node:worker_threads';
import type { GraphSpec } from '../../src/brains/graph/schema.ts';
import type { SavedBrainCatalogOperation, SavedBrainCatalogResult } from '../protocol.ts';
import {
  DEFAULT_MANAGED_CHECKPOINT_DESCRIPTOR_LIMITS,
  managedCheckpointContentsEqual,
  managedCheckpointDescriptorsEqual,
  parseManagedCheckpointDescriptor,
  parseManagedCheckpointDescriptorLimits,
  parseManagedGenerationCommit,
  parseManagedImportInventoryDescriptor,
  parseManagedLegacyConversion,
  type CheckpointOperationId,
  type ManagedBrowserHallOfFameEntry,
  type ManagedBrowserHistoryEntry,
  type ManagedCheckpointDescriptor,
  type ManagedCheckpointDescriptorLimits,
  type ManagedCheckpointExportLease,
  type ManagedCheckpointSelection,
  type ManagedGenerationCommit,
  type ManagedGraphPreset,
  type ManagedGraphPresetMeta,
  type ManagedHallOfFameSelection,
  type ManagedImportBranchResult,
  type ManagedImportInventoryDescriptor,
  type ManagedLegacyConversion,
  type ManagedLegacySnapshotSelection,
  type ManagedOrphanCleanupResult,
  type ManagedStorageDiagnostics,
  type U64Hex
} from './checkpointPersistenceProtocol.ts';
import {
  OWNER_CHECKPOINT_RETENTION_DEFAULTS,
  type CheckpointPruneResult,
  type CheckpointRetentionInventory
} from './checkpointRetention.ts';
import { parseRecoveryBranchCommit, parseRecoveryScanCursor, type RecoveryBranchCommit, type RecoveryBranchResult, type RecoveryScanCursor, type RecoveryScanResult } from './recoveryProtocol.ts';
import { isU64Hex, normalizeGraphPreset, parseWorkerResponse } from './response_decode.ts';
import { parseSavedBrainOperation } from './savedBrainProtocol.ts';

/** Options for the client-owned isolated persistence worker. */
export interface CheckpointPersistenceClientOptions {
  /** Disposable/test SQLite database path supplied to the isolated worker. */
  databasePath: string;
  /** Require an existing store; managed mode accepts only compatible Rust metadata without legacy conversion. */
  existingOnly?: boolean | 'managed';
  /** Existing controlled root containing final immutable checkpoint-v3 files. */
  managedRootPath: string;
  /** Selected cap for unpinned automatic files and the physical store. */
  automaticByteCapBytes?: bigint;
  /** Explicit bounded descriptor limits, defaulting only to the provisional Stage 3 envelope. */
  limits?: ManagedCheckpointDescriptorLimits;
  /** Test-only worker module override for client protocol/lifecycle tests. */
  workerUrlForTesting?: URL;
  /** Test-only response mode consumed exclusively by a supplied test worker module. */
  workerResponseModeForTesting?: 'invalid' | 'mismatched' | 'exit' | 'exit-clean' | 'stall' | 'stall-after-progress' | 'progressing';
  /** Test-only one-shot fault at a real SQLite checkpoint transaction boundary. */
  checkpointCommitFailpointForTesting?: 'before-commit' | 'after-commit-before-reply';
  /** Test-only one-shot fault at a real SQLite managed-import transaction boundary. */
  importCommitFailpointForTesting?: 'before-commit' | 'after-commit-before-reply';
  /** No-progress limit for one worker request; production defaults to 60 seconds. */
  noProgressTimeoutMs?: number;
}

/** Persistence-worker no-progress deadline selected by the approved runtime plan. */
const DEFAULT_PERSISTENCE_NO_PROGRESS_MS = 60_000;

/** Matching acknowledgement returned after metadata/current-pointer commit. */
export interface ManagedCheckpointCommitResult {
  /** Exact correlated operation token. */
  operationId: CheckpointOperationId;
  /** Exact engine transition epoch. */
  transitionEpoch: U64Hex;
  /** Opaque run identity. */
  runId: string;
  /** Content-addressed checkpoint identity. */
  checkpointId: string;
  /** Complete descriptor echoed only after its exact transaction committed. */
  descriptor: ManagedCheckpointDescriptor;
  /** Import-only durable branch provenance. */
  importBranch?: ManagedImportBranchResult | null;
}

/** Exact immutable boundary protected by an owner pin operation. */
export interface PinnedCheckpointResult {
  /** Content-addressed managed checkpoint identity. */
  checkpointId: string;
  /** Exact generation boundary. */
  generation: U64Hex;
}

/**
 * Compare every redundant worker-result identity with one Rust-selected descriptor.
 * @param committed - Complete acknowledgement returned by the persistence client.
 * @param expected - Strict descriptor originally selected by Rust.
 * @returns True only when the complete descriptor and every echoed identity match.
 */
export function managedCheckpointCommitResultMatchesDescriptor(
  committed: ManagedCheckpointCommitResult,
  expected: ManagedCheckpointDescriptor
): boolean {
  return managedCheckpointDescriptorsEqual(committed.descriptor, expected) &&
    committed.operationId === expected.operationId &&
    committed.transitionEpoch === expected.transitionEpoch &&
    committed.runId === expected.runId &&
    committed.checkpointId === expected.logicalRootSha256;
}

/** One pending descriptor-only commit waiting for its exact operation response. */
interface PendingCommit {
  /** Original strictly validated descriptor. */
  descriptor: ManagedCheckpointDescriptor;
  /** Import may idempotently reuse an older local publication token. */
  import: boolean;
  /** Fresh effective run requested for an older-checkpoint import. */
  branchRunId: string | null;
  /** Resolve callback for its matching acknowledgement. */
  resolve: (result: ManagedCheckpointCommitResult) => void;
  /** Reject callback for rejection, protocol fault, or worker exit. */
  reject: (error: Error) => void;
}

/** Client-side lifecycle for the worker's one temporary export reference. */
type ExportLeaseState =
  | { phase: 'acquiring'; operationId: CheckpointOperationId;
      resolve(value: ManagedCheckpointExportLease): void; reject(error: Error): void }
  | { phase: 'active'; operationId: CheckpointOperationId }
  | { phase: 'releasing'; operationId: CheckpointOperationId;
      resolve(): void; reject(error: Error): void };

/** Client lifecycle for one exact packed winner protected across native consumption. */
type HallOfFameLeaseState =
  | { phase: 'acquiring'; operationId: CheckpointOperationId; runId: string; entryId: U64Hex;
      resolve(value: ManagedHallOfFameSelection): void; reject(error: Error): void }
  | { phase: 'active'; operationId: CheckpointOperationId; runId: string; entryId: U64Hex }
  | { phase: 'releasing'; operationId: CheckpointOperationId; runId: string; entryId: U64Hex;
      resolve(): void; reject(error: Error): void };

/** One correlated graph-preset metadata operation. */
type PendingGraphPresetOperation =
  | { kind: 'save'; resolve(value: number): void; reject(error: Error): void }
  | { kind: 'list'; resolve(value: ManagedGraphPresetMeta[]): void; reject(error: Error): void }
  | { kind: 'load'; resolve(value: ManagedGraphPreset | null): void; reject(error: Error): void };

/**
 * Client lifecycle wrapper around exactly one dedicated SQLite persistence worker.
 *
 * This class sends only validated scalar descriptors and two fixed-size generation records.
 * It deliberately exposes no API that accepts a population buffer, archive bytes, World
 * object, or typed array.
 */
export class CheckpointPersistenceClient {
  /** Isolated worker exclusively owning the synchronous SQLite connection. */
  private readonly worker: Worker;
  /** Pending commits indexed by exact nonnumeric operation token. */
  private readonly pending = new Map<CheckpointOperationId, PendingCommit>();
  /** At most one bounded startup selection may be in flight. */
  private selection: {
    operationId: CheckpointOperationId;
    runId: string | null;
    resolve(value: ManagedCheckpointSelection): void;
    reject(error: Error): void;
  } | undefined;
  /** One bounded legacy parent-row selection; population columns remain unread. */
  private legacySelection: {
    operationId: CheckpointOperationId;
    resolve(value: ManagedLegacySnapshotSelection | null): void;
    reject(error: Error): void;
  } | undefined;
  /** One candidate read; a corrupt row advances only its stable scalar cursor. */
  private scan: { operationId: string; cursor: RecoveryScanCursor | null; checkpointId?: string;
    resolve(value: RecoveryScanResult): void; reject(error: Error): void } | undefined;
  /** One startup recovery transaction; retries use the same caller-owned operation token. */
  private recovery: { commit: RecoveryBranchCommit; resolve(value: RecoveryBranchResult): void; reject(error: Error): void } | undefined;
  /** At most one bounded retention inventory read may be in flight. */
  private retention: { operationId: CheckpointOperationId; resolve(value: CheckpointRetentionInventory): void; reject(error: Error): void } | undefined;
  /** At most one small SQLite storage inspection may be in flight. */
  private storage: { operationId: CheckpointOperationId; resolve(value: ManagedStorageDiagnostics): void; reject(error: Error): void } | undefined;
  /** At most one owner pin transaction may be in flight. */
  private pin: { operationId: CheckpointOperationId; resolve(value: PinnedCheckpointResult): void; reject(error: Error): void } | undefined;
  /** At most one verified automatic pruning pass may be in flight. */
  private pruning: { operationId: CheckpointOperationId; resolve(value: CheckpointPruneResult): void; reject(error: Error): void } | undefined;
  /** One reference-checked cleanup while the caller holds native publication. */
  private orphanCleanup: { operationId: CheckpointOperationId; resolve(value: ManagedOrphanCleanupResult): void; reject(error: Error): void } | undefined;
  /** At most one compact browser-history read may be in flight. */
  private history: { operationId: CheckpointOperationId; runId: string; resolve(value: ManagedBrowserHistoryEntry[]): void; reject(error: Error): void } | undefined;
  /** At most one compact browser Hall-of-Fame read may be in flight. */
  private hallOfFame: { operationId: CheckpointOperationId; runId: string; resolve(value: ManagedBrowserHallOfFameEntry[]): void; reject(error: Error): void } | undefined;
  /** At most one exact retained winner selection may be in flight. */
  private hallOfFameSelection: HallOfFameLeaseState | undefined;
  /** Bounded explicit library requests; no model buffers enter this queue. */
  private readonly savedBrains = new Map<string, {request: SavedBrainCatalogOperation;
    resolve(result: SavedBrainCatalogResult): void; reject(error: Error): void}>();
  /** Independent bounded graph-preset requests indexed by their correlation token. */
  private readonly graphPresets = new Map<CheckpointOperationId, PendingGraphPresetOperation>();
  /** One temporary exact-checkpoint reference across preparation and download. */
  private exportLease: ExportLeaseState | undefined;
  /** Terminal lifecycle failure, if the worker violates protocol or exits unexpectedly. */
  private failure: Error | null = null;
  /** Whether orderly shutdown has been requested. */
  private stopping = false;
  /** Shared orderly shutdown promise. */
  private stopPromise: Promise<void> | null = null;
  /** Promise resolved after the worker has actually emitted its exit event. */
  private readonly exitPromise: Promise<void>;
  /** Resolver for the worker-exit promise. */
  private resolveExited!: () => void;
  /** Whether the worker has emitted its exit event. */
  private workerExited = false;
  /** Whether Node has finished starting the worker thread. */
  private workerOnline = false;
  /** One best-effort termination request started only for a terminal client failure. */
  private terminationPromise: Promise<void> | null = null;
  /** Correlated operations currently awaiting a response from the worker. */
  private readonly watchedOperations = new Map<CheckpointOperationId, bigint | null>();
  /** Shared deadline refreshed by any validated worker progress. */
  private watchdog: NodeJS.Timeout | undefined;
  /** Configured no-progress duration. */
  private readonly noProgressTimeoutMs: number;
  /** Resolver waiting for the worker's exit after shutdown. */
  private resolveStopped: (() => void) | null = null;
  /** Rejecter waiting for an unsuccessful worker exit after shutdown. */
  private rejectStopped: ((error: Error) => void) | null = null;

  /**
   * Spawn the isolated worker with only database/root path bootstrap data.
   * @param options - Worker path options and controlled storage locations.
   */
  constructor(options: CheckpointPersistenceClientOptions) {
    if (typeof options.databasePath !== 'string' || options.databasePath.length === 0) {
      throw new TypeError('checkpoint persistence databasePath must be a nonempty string');
    }
    if (typeof options.managedRootPath !== 'string' || options.managedRootPath.length === 0) {
      throw new TypeError('checkpoint persistence managedRootPath must be a nonempty string');
    }
    this.noProgressTimeoutMs = options.noProgressTimeoutMs ?? DEFAULT_PERSISTENCE_NO_PROGRESS_MS;
    if (!Number.isSafeInteger(this.noProgressTimeoutMs) || this.noProgressTimeoutMs < 1) {
      throw new RangeError('checkpoint persistence no-progress timeout must be a positive safe integer');
    }
    const limits = parseManagedCheckpointDescriptorLimits(
      options.limits ?? DEFAULT_MANAGED_CHECKPOINT_DESCRIPTOR_LIMITS
    );
    const automaticByteCapBytes = options.automaticByteCapBytes ??
      OWNER_CHECKPOINT_RETENTION_DEFAULTS.automaticByteCap;
    if (automaticByteCapBytes < 1n || automaticByteCapBytes > 0xffff_ffff_ffff_ffffn) {
      throw new RangeError('checkpoint automatic byte cap must fit positive u64');
    }
    const workerUrl = options.workerUrlForTesting ??
      new URL('./checkpointPersistenceWorker.ts', import.meta.url);
    this.worker = new Worker(workerUrl, {
      workerData: {
        databasePath: options.databasePath,
        managedRootPath: options.managedRootPath,
        limits,
        automaticByteCapBytes,
        existingOnly: options.existingOnly ?? false,
        ...(options.checkpointCommitFailpointForTesting
          ? { checkpointCommitFailpointForTesting: options.checkpointCommitFailpointForTesting }
          : {}),
        ...(options.importCommitFailpointForTesting
          ? { importCommitFailpointForTesting: options.importCommitFailpointForTesting }
          : {}),
        ...(options.workerUrlForTesting && options.workerResponseModeForTesting
          ? { checkpointPersistenceTestMode: options.workerResponseModeForTesting }
          : {})
      }
    });
    this.exitPromise = new Promise<void>(resolve => { this.resolveExited = resolve; });
    this.worker.on('online', () => {
      this.workerOnline = true;
      this.armWatchdog();
    });
    this.worker.on('message', message => this.onMessage(message));
    this.worker.on('messageerror', error => this.fail(asError(error)));
    this.worker.on('error', error => this.fail(error));
    this.worker.on('exit', code => this.onExit(code));
  }

  /**
   * Commit a descriptor after its file is already final under the controlled root.
   * @param value - Strict descriptor candidate containing no checkpoint payload bytes.
   * @param generationCommitValue - Exact compact history and Hall-of-Fame reference.
   * @param activateRun - Select this new run as the process-restart lineage in the same transaction.
   * @param legacyConversionValue - Optional population-only source for a converted run start.
   * @returns Matching durable metadata/current-pointer acknowledgement.
   */
  commit(
    value: unknown,
    generationCommitValue: unknown = null,
    activateRun = false,
    legacyConversionValue: unknown = null
  ): Promise<ManagedCheckpointCommitResult> {
    if (this.failure) return Promise.reject(this.failure);
    if (this.stopping) return Promise.reject(new Error('checkpoint persistence client is stopping'));
    let descriptor: ManagedCheckpointDescriptor;
    let generationCommit: ManagedGenerationCommit | null;
    let legacyConversion: ManagedLegacyConversion | null;
    try {
      descriptor = parseManagedCheckpointDescriptor(value);
      generationCommit = parseManagedGenerationCommit(generationCommitValue, descriptor);
      legacyConversion = legacyConversionValue === null
        ? null : parseManagedLegacyConversion(legacyConversionValue);
      if (legacyConversion !== null && descriptor.boundaryKind !== 'run-start') {
        throw new TypeError('legacy conversion provenance requires a run-start checkpoint');
      }
    } catch (error) {
      return Promise.reject(asError(error));
    }
    if (this.pending.has(descriptor.operationId)) {
      return Promise.reject(new Error(`checkpoint operation ${descriptor.operationId} is already pending`));
    }
    return new Promise<ManagedCheckpointCommitResult>((resolve, reject) => {
      this.pending.set(descriptor.operationId, { descriptor, import: false, branchRunId: null, resolve, reject });
      try {
        this.postOperation({
          type: 'commitManagedCheckpoint', descriptor, generationCommit, activateRun, legacyConversion
        }, descriptor.operationId);
      } catch (error) {
        this.pending.delete(descriptor.operationId);
        reject(asError(error));
      }
    });
  }

  /** Read one current descriptor on the worker, preserving every source row and file. */
  async selectCurrent(runId: string | null = null): Promise<ManagedCheckpointDescriptor | null> {
    const selected = await this.selectStartup(runId);
    if (selected.descriptor && selected.descriptor.runId !== selected.runId) {
      throw new Error('recovery branch requires provenance-aware startup selection');
    }
    return selected.descriptor;
  }

  /** Read active lineage and immutable source together, including durable recovery provenance. */
  selectStartup(runId: string | null = null): Promise<ManagedCheckpointSelection> {
    if (this.failure) return Promise.reject(this.failure);
    if (this.stopping || this.selection) return Promise.reject(new Error('checkpoint selection is busy or stopping'));
    if (runId !== null && (typeof runId !== 'string' || !runId || Buffer.byteLength(runId) > 256 || runId.includes('\0'))) {
      return Promise.reject(new TypeError('invalid checkpoint selection run ID'));
    }
    const operationId = randomBytes(16).toString('hex');
    return new Promise((resolve, reject) => {
      this.selection = { operationId, runId, resolve, reject };
      try { this.postOperation({ type: 'selectManagedCheckpoint', operationId, runId }, operationId); }
      catch (error) { this.selection = undefined; reject(asError(error)); }
    });
  }

  /** Read one descending candidate while pinning the failed source pointer. */
  scanRecoveryCandidate(value: RecoveryScanCursor | null = null): Promise<RecoveryScanResult> {
    if (this.failure) return Promise.reject(this.failure);
    if (this.stopping || this.scan) return Promise.reject(new Error('recovery scan is busy or stopping'));
    const cursor = value === null ? null : parseRecoveryScanCursor(value);
    const operationId = randomBytes(16).toString('hex');
    return new Promise((resolve, reject) => {
      this.scan = { operationId, cursor, resolve, reject };
      try { this.postOperation({ type: 'scanRecoveryCandidate', operationId, cursor }, operationId); }
      catch (error) { this.scan = undefined; reject(asError(error)); }
    });
  }

  /** Commit a validated recovery branch before native activation, without copying population bytes. */
  commitRecoveryBranch(value: RecoveryBranchCommit): Promise<RecoveryBranchResult> {
    if (this.failure) return Promise.reject(this.failure);
    if (this.stopping || this.recovery) return Promise.reject(new Error('recovery commit is busy or stopping'));
    const commit = parseRecoveryBranchCommit(value);
    return new Promise((resolve, reject) => {
      this.recovery = { commit, resolve, reject };
      try { this.postOperation({ type: 'commitRecoveryBranch', commit }, commit.operationId); }
      catch (error) { this.recovery = undefined; reject(asError(error)); }
    });
  }

  /** Atomically import trusted compact metadata and make its exact boundary current. */
  commitImport(
    descriptorValue: unknown,
    inventoryValue: unknown,
    branchRunId: string | null = null
  ): Promise<ManagedCheckpointCommitResult> {
    if (this.failure) return Promise.reject(this.failure);
    if (this.stopping) return Promise.reject(new Error('checkpoint persistence client is stopping'));
    let descriptor: ManagedCheckpointDescriptor;
    let inventory: ManagedImportInventoryDescriptor;
    try {
      descriptor = parseManagedCheckpointDescriptor(descriptorValue);
      inventory = parseManagedImportInventoryDescriptor(inventoryValue, descriptor.operationId);
      if (branchRunId !== null && (!branchRunId || branchRunId === descriptor.runId ||
          branchRunId.includes('\0') || Buffer.byteLength(branchRunId) > 256 ||
          Buffer.from(branchRunId, 'utf8').toString('utf8') !== branchRunId)) {
        throw new TypeError('invalid import branch run identity');
      }
    } catch (error) {
      return Promise.reject(asError(error));
    }
    if (this.pending.has(descriptor.operationId)) {
      return Promise.reject(new Error(`checkpoint operation ${descriptor.operationId} is already pending`));
    }
    return new Promise<ManagedCheckpointCommitResult>((resolve, reject) => {
      this.pending.set(descriptor.operationId, { descriptor, import: true, branchRunId, resolve, reject });
      try {
        this.postOperation({ type: 'commitManagedImport', descriptor, inventory, branchRunId }, descriptor.operationId);
      } catch (error) {
        this.pending.delete(descriptor.operationId);
        reject(asError(error));
      }
    });
  }

  /** Read the current keep/prune accounting without deleting any managed file. */
  inspectRetention(): Promise<CheckpointRetentionInventory> {
    if (this.failure) return Promise.reject(this.failure);
    if (this.stopping || this.retention) return Promise.reject(new Error('checkpoint retention inspection is busy or stopping'));
    const operationId = randomBytes(16).toString('hex');
    return new Promise((resolve, reject) => {
      this.retention = { operationId, resolve, reject };
      try { this.postOperation({ type: 'inspectCheckpointRetention', operationId }, operationId); }
      catch (error) { this.retention = undefined; reject(asError(error)); }
    });
  }

  /** Select one retained ID across all runs without scanning or changing the active pointer. */
  selectRetainedCheckpoint(checkpointId: string): Promise<RecoveryScanResult> {
    if (this.failure) return Promise.reject(this.failure);
    if (this.stopping || this.scan) return Promise.reject(new Error('retained checkpoint selection is busy or stopping'));
    if (!/^[0-9a-f]{64}$/u.test(checkpointId)) return Promise.reject(new TypeError('invalid retained checkpoint ID'));
    const operationId = randomBytes(16).toString('hex');
    return new Promise((resolve, reject) => {
      this.scan = { operationId, cursor: null, checkpointId, resolve, reject };
      try { this.postOperation({ type: 'selectRetainedCheckpoint', operationId, checkpointId }, operationId); }
      catch (error) { this.scan = undefined; reject(asError(error)); }
    });
  }

  /** Read only SQLite file and page counters from the worker that owns the connection. */
  inspectStorage(): Promise<ManagedStorageDiagnostics> {
    if (this.failure) return Promise.reject(this.failure);
    if (this.stopping || this.storage) return Promise.reject(new Error('managed storage inspection is busy or stopping'));
    const operationId = randomBytes(16).toString('hex');
    return new Promise((resolve, reject) => {
      this.storage = { operationId, resolve, reject };
      try { this.postOperation({ type: 'inspectManagedStorage', operationId }, operationId); }
      catch (error) { this.storage = undefined; reject(asError(error)); }
    });
  }

  /** Atomically pin the effective active run's exact current managed file. */
  pinCurrentCheckpoint(): Promise<PinnedCheckpointResult> {
    if (this.failure) return Promise.reject(this.failure);
    if (this.stopping || this.pin) return Promise.reject(new Error('checkpoint pin is busy or stopping'));
    const operationId = randomBytes(16).toString('hex');
    return new Promise((resolve, reject) => {
      this.pin = { operationId, resolve, reject };
      try { this.postOperation({ type: 'pinCurrentCheckpoint', operationId }, operationId); }
      catch (error) { this.pin = undefined; reject(asError(error)); }
    });
  }

  /** Acquire the active run's exact current checkpoint for one direct export. */
  acquireCurrentExportLease(): Promise<ManagedCheckpointExportLease> {
    if (this.failure) return Promise.reject(this.failure);
    if (this.stopping || this.exportLease) return Promise.reject(new Error('checkpoint export is busy or stopping'));
    const operationId = randomBytes(16).toString('hex');
    return new Promise((resolve, reject) => {
      this.exportLease = { phase: 'acquiring', operationId, resolve, reject };
      try { this.postOperation({ type: 'acquireCurrentExportLease', operationId }, operationId); }
      catch (error) { this.exportLease = undefined; reject(asError(error)); }
    });
  }

  /** Release the exact export reference after preparation, transfer, failure, or cancellation. */
  releaseExportLease(operationId: CheckpointOperationId): Promise<void> {
    if (this.failure) return Promise.reject(this.failure);
    const lease = this.exportLease;
    if (this.stopping || !lease || lease.phase !== 'active' || lease.operationId !== operationId) {
      return Promise.reject(new Error('checkpoint export lease is not active'));
    }
    return new Promise((resolve, reject) => {
      this.exportLease = { phase: 'releasing', operationId, resolve, reject };
      try { this.postOperation({ type: 'releaseExportLease', operationId }, operationId); }
      catch (error) { this.exportLease = { phase: 'active', operationId }; reject(asError(error)); }
    });
  }

  /** Apply retention, optionally reserving physical space before publication. */
  applyRetention(physicalReserveBytes: bigint | null = null): Promise<CheckpointPruneResult> {
    if (this.failure) return Promise.reject(this.failure);
    if (this.stopping || this.pruning) return Promise.reject(new Error('checkpoint retention pruning is busy or stopping'));
    if (physicalReserveBytes !== null &&
        (physicalReserveBytes < 0n || physicalReserveBytes > 0xffff_ffff_ffff_ffffn)) {
      return Promise.reject(new RangeError('physical retention reserve must fit u64'));
    }
    const operationId = randomBytes(16).toString('hex');
    return new Promise((resolve, reject) => {
      this.pruning = { operationId, resolve, reject };
      try { this.postOperation({ type: 'applyCheckpointRetention', operationId,
        physicalReserveBytes: physicalReserveBytes?.toString(16).padStart(16, '0') ?? null }, operationId); }
      catch (error) { this.pruning = undefined; reject(asError(error)); }
    });
  }

  /** Reclaim immutable orphans only while the caller holds a native durability boundary. */
  reclaimManagedOrphans(): Promise<ManagedOrphanCleanupResult> {
    if (this.failure) return Promise.reject(this.failure);
    if (this.stopping || this.orphanCleanup) return Promise.reject(new Error('managed orphan cleanup is busy or stopping'));
    const operationId = randomBytes(16).toString('hex');
    return new Promise((resolve, reject) => {
      this.orphanCleanup = { operationId, resolve, reject };
      try { this.postOperation({ type: 'reclaimManagedOrphans', operationId }, operationId); }
      catch (error) { this.orphanCleanup = undefined; reject(asError(error)); }
    });
  }

  /** Read the newest compact generation summaries for one active lineage. */
  readBrowserHistory(runId: string, limit = 120): Promise<ManagedBrowserHistoryEntry[]> {
    if (this.failure) return Promise.reject(this.failure);
    if (this.stopping || this.history) return Promise.reject(new Error('browser history read is busy or stopping'));
    if (!runId || runId.includes('\0') || Buffer.byteLength(runId) > 256 ||
        !Number.isSafeInteger(limit) || limit < 1 || limit > 120) {
      return Promise.reject(new TypeError('invalid browser history request'));
    }
    const operationId = randomBytes(16).toString('hex');
    return new Promise((resolve, reject) => {
      this.history = { operationId, runId, resolve, reject };
      try { this.postOperation({ type: 'readBrowserHistory', operationId, runId, limit }, operationId); }
      catch (error) { this.history = undefined; reject(asError(error)); }
    });
  }

  /** Read a best-first compact Hall-of-Fame window without loading packed weights. */
  readBrowserHallOfFame(runId: string, limit = 100): Promise<ManagedBrowserHallOfFameEntry[]> {
    if (this.failure) return Promise.reject(this.failure);
    if (this.stopping || this.hallOfFame) return Promise.reject(new Error('browser Hall-of-Fame read is busy or stopping'));
    if (!runId || runId.includes('\0') || Buffer.byteLength(runId) > 256 ||
        !Number.isSafeInteger(limit) || limit < 1 || limit > 100) {
      return Promise.reject(new TypeError('invalid browser Hall-of-Fame request'));
    }
    const operationId = randomBytes(16).toString('hex');
    return new Promise((resolve, reject) => {
      this.hallOfFame = { operationId, runId, resolve, reject };
      try { this.postOperation({ type: 'readBrowserHallOfFame', operationId, runId, limit }, operationId); }
      catch (error) { this.hallOfFame = undefined; reject(asError(error)); }
    });
  }

  /** Select only the newest compatible old parent identity. */
  selectLegacySnapshot(): Promise<ManagedLegacySnapshotSelection | null> {
    if (this.failure) return Promise.reject(this.failure);
    if (this.stopping || this.legacySelection) {
      return Promise.reject(new Error('legacy checkpoint selection is busy or stopping'));
    }
    const operationId = randomBytes(16).toString('hex');
    return new Promise((resolve, reject) => {
      this.legacySelection = { operationId, resolve, reject };
      try { this.postOperation({ type: 'selectLegacySnapshot', operationId }, operationId); }
      catch (error) { this.legacySelection = undefined; reject(asError(error)); }
    });
  }

  /** Select and verify one retained winner descriptor for direct Rust consumption. */
  selectHallOfFameEntry(runId: string, entryId: U64Hex): Promise<ManagedHallOfFameSelection> {
    if (this.failure) return Promise.reject(this.failure);
    if (this.stopping || this.hallOfFameSelection) {
      return Promise.reject(new Error('Hall-of-Fame selection is busy or stopping'));
    }
    if (!runId || runId.includes('\0') || Buffer.byteLength(runId) > 256 || !isU64Hex(entryId)) {
      return Promise.reject(new TypeError('invalid Hall-of-Fame selection request'));
    }
    const operationId = randomBytes(16).toString('hex');
    return new Promise((resolve, reject) => {
      this.hallOfFameSelection = { phase: 'acquiring', operationId, runId, entryId, resolve, reject };
      try { this.postOperation({ type: 'selectHallOfFameEntry', operationId, runId, entryId }, operationId); }
      catch (error) { this.hallOfFameSelection = undefined; reject(asError(error)); }
    });
  }

  /** Submit an explicit bounded library action to the same serialized metadata owner. */
  savedBrainCatalog(value: SavedBrainCatalogOperation): Promise<SavedBrainCatalogResult> {
    if (this.failure) return Promise.reject(this.failure);
    if (this.stopping || this.savedBrains.size >= 8) return Promise.reject(new Error('saved brain metadata is busy or stopping'));
    let request: SavedBrainCatalogOperation;
    try { request = parseSavedBrainOperation(value); } catch (error) { return Promise.reject(asError(error)); }
    const operationId = randomBytes(16).toString('hex');
    return new Promise((resolve, reject) => {
      this.savedBrains.set(operationId, {request, resolve, reject});
      try { this.postOperation({type: 'savedBrainCatalog', operationId, request}, operationId); }
      catch (error) { this.savedBrains.delete(operationId); reject(asError(error)); }
    });
  }

  /** Persist one bounded graph preset without involving the authoritative engine. */
  saveGraphPreset(name: string, spec: GraphSpec): Promise<number> {
    if (this.failure) return Promise.reject(this.failure);
    if (this.stopping || this.graphPresets.size >= 8) {
      return Promise.reject(new Error('graph preset persistence is busy or stopping'));
    }
    let normalized: { name: string; specJson: string };
    try { normalized = normalizeGraphPreset(name, spec); }
    catch (error) { return Promise.reject(asError(error)); }
    const operationId = randomBytes(16).toString('hex');
    return new Promise((resolve, reject) => {
      this.graphPresets.set(operationId, { kind: 'save', resolve, reject });
      try { this.postOperation({ type: 'saveGraphPreset', operationId, ...normalized }, operationId); }
      catch (error) { this.graphPresets.delete(operationId); reject(asError(error)); }
    });
  }

  /** List newest graph-preset metadata without returning graph documents. */
  listGraphPresets(limit = 50): Promise<ManagedGraphPresetMeta[]> {
    if (this.failure) return Promise.reject(this.failure);
    if (this.stopping || this.graphPresets.size >= 8) {
      return Promise.reject(new Error('graph preset persistence is busy or stopping'));
    }
    if (!Number.isSafeInteger(limit) || limit < 1 || limit > 200) {
      return Promise.reject(new TypeError('graph preset limit must be an integer from 1 to 200'));
    }
    const operationId = randomBytes(16).toString('hex');
    return new Promise((resolve, reject) => {
      this.graphPresets.set(operationId, { kind: 'list', resolve, reject });
      try { this.postOperation({ type: 'listGraphPresets', operationId, limit }, operationId); }
      catch (error) { this.graphPresets.delete(operationId); reject(asError(error)); }
    });
  }

  /** Load and independently validate one complete graph preset. */
  loadGraphPreset(presetId: number): Promise<ManagedGraphPreset | null> {
    if (this.failure) return Promise.reject(this.failure);
    if (this.stopping || this.graphPresets.size >= 8) {
      return Promise.reject(new Error('graph preset persistence is busy or stopping'));
    }
    if (!Number.isSafeInteger(presetId) || presetId < 1) {
      return Promise.reject(new TypeError('graph preset id must be a positive safe integer'));
    }
    const operationId = randomBytes(16).toString('hex');
    return new Promise((resolve, reject) => {
      this.graphPresets.set(operationId, { kind: 'load', resolve, reject });
      try { this.postOperation({ type: 'loadGraphPreset', operationId, presetId }, operationId); }
      catch (error) { this.graphPresets.delete(operationId); reject(asError(error)); }
    });
  }

  /** Release the retained winner after native success or rejection. */
  releaseHallOfFameEntry(operationId: CheckpointOperationId): Promise<void> {
    if (this.failure) return Promise.reject(this.failure);
    const lease = this.hallOfFameSelection;
    if (this.stopping || !lease || lease.phase !== 'active' || lease.operationId !== operationId) {
      return Promise.reject(new Error('Hall-of-Fame selection lease is not active'));
    }
    return new Promise((resolve, reject) => {
      this.hallOfFameSelection = { ...lease, phase: 'releasing', resolve, reject };
      try { this.postOperation({ type: 'releaseHallOfFameEntry', operationId }, operationId); }
      catch (error) { this.hallOfFameSelection = lease; reject(asError(error)); }
    });
  }

  /** Send one correlated request and start its shared no-progress deadline. */
  private postOperation(message: unknown, operationId: CheckpointOperationId): void {
    const wasIdle = this.watchedOperations.size === 0;
    this.watchedOperations.set(operationId, null);
    if (wasIdle) this.armWatchdog();
    try {
      this.worker.postMessage(message);
    } catch (error) {
      this.finishWatchedOperation(operationId);
      throw error;
    }
  }

  /** Restart the no-progress deadline while at least one request is pending. */
  private armWatchdog(): void {
    if (this.watchdog) clearTimeout(this.watchdog);
    if (this.watchedOperations.size === 0 || !this.workerOnline) {
      this.watchdog = undefined;
      return;
    }
    this.watchdog = setTimeout(() => {
      this.watchdog = undefined;
      const operationId = this.watchedOperations.keys().next().value as CheckpointOperationId;
      this.fail(new Error(
        `checkpoint persistence worker made no progress for ${this.noProgressTimeoutMs} ms ` +
        `(oldest pending operation ${operationId})`
      ));
    }, this.noProgressTimeoutMs);
    this.watchdog.unref();
  }

  /** Stop watching one operation after its terminal response. */
  private finishWatchedOperation(operationId: CheckpointOperationId): void {
    this.watchedOperations.delete(operationId);
    this.armWatchdog();
  }

  /**
   * Stop the client-owned worker after it has completed all preceding synchronous messages.
   * @returns Promise resolved after the worker exits cleanly.
   */
  close(): Promise<void> {
    if (this.stopPromise) return this.stopPromise;
    if (this.failure) {
      this.stopPromise = this.terminateForFailure().then(() => { throw this.failure!; });
      return this.stopPromise;
    }
    this.stopping = true;
    this.stopPromise = new Promise<void>((resolve, reject) => {
      this.resolveStopped = resolve;
      this.rejectStopped = reject;
      if (this.failure) {
        reject(this.failure);
        return;
      }
      try {
        this.worker.postMessage({ type: 'shutdown' });
      } catch (error) {
        const failure = asError(error);
        this.fail(failure);
        reject(failure);
      }
    });
    return this.stopPromise;
  }

  /**
   * Report whether the worker has exited; primarily useful for bounded lifecycle diagnostics.
   */
  get terminated(): boolean {
    return this.workerExited;
  }

  /**
   * Route and validate one worker response before resolving any caller promise.
   * @param value - Unknown structured-cloned worker response.
   */
  private onMessage(value: unknown): void {
    try {
      const response = parseWorkerResponse(value);
      if (response.type === 'persistenceProgress') {
        const previous = this.watchedOperations.get(response.operationId);
        const completed = BigInt(`0x${response.completedUnits}`);
        if (previous === undefined) throw new Error('persistence worker reported progress for an unknown operation');
        if (previous !== null && completed <= previous) {
          throw new Error('persistence worker reported non-monotonic operation progress');
        }
        this.watchedOperations.set(response.operationId, completed);
        this.armWatchdog();
        return;
      }
      const responseOperationId = response.type === 'recoveryBranchCommitted'
        ? response.result.operationId
        : response.type === 'currentExportLeaseAcquired'
          ? response.lease.operationId
          : response.type === 'hallOfFameEntrySelected'
            ? response.selection.operationId
            : response.operationId;
      if (responseOperationId !== null) this.finishWatchedOperation(responseOperationId);
      if (response.type === 'savedBrainCatalogResult') {
        const pending = this.savedBrains.get(response.operationId);
        if (!pending || pending.request.action !== response.result.action) throw new Error('mismatched saved brain metadata response');
        const request = pending.request;
        const result = response.result;
        if ('brainId' in request && ((result.action === 'delete' && result.brainId !== request.brainId) ||
            ('record' in result && result.record && result.record.brainId !== request.brainId))) throw new Error('mismatched saved brain identity');
        if (request.action === 'list' && result.action === 'list' && (result.page.entries.length > request.limit ||
            result.page.entries.some(entry => request.after !== null && entry.brainId <= request.after))) throw new Error('mismatched saved brain page');
        if (request.action === 'acquireArchive' && result.action === 'acquireArchive' &&
            (result.entries.length !== request.brainIds.length || result.entries.some((entry, index) => entry.brainId !== request.brainIds[index]))) throw new Error('mismatched saved brain archive selection');
        if (request.action === 'release' && result.action === 'release' && request.leaseId !== result.leaseId) throw new Error('mismatched saved brain lease');
        this.savedBrains.delete(response.operationId);
        pending.resolve(result);
        return;
      }
      if (response.type === 'managedOrphansReclaimed') {
        const pending = this.orphanCleanup;
        if (!pending || pending.operationId !== response.operationId) {
          throw new Error('persistence worker returned a mismatched orphan cleanup');
        }
        this.orphanCleanup = undefined;
        pending.resolve(response.result);
        return;
      }
      if (response.type === 'hallOfFameEntryReleased') {
        const lease = this.hallOfFameSelection;
        if (!lease || lease.phase !== 'releasing' || lease.operationId !== response.operationId) {
          throw new Error('persistence worker returned mismatched Hall-of-Fame release');
        }
        this.hallOfFameSelection = undefined;
        lease.resolve();
        return;
      }
      if (response.type === 'exportLeaseReleased') {
        const lease = this.exportLease;
        if (!lease || lease.phase !== 'releasing' || lease.operationId !== response.operationId) {
          throw new Error('persistence worker returned a mismatched export lease release');
        }
        this.exportLease = undefined;
        lease.resolve();
        return;
      }
      if (response.type === 'currentExportLeaseAcquired') {
        const lease = this.exportLease;
        if (!lease || lease.phase !== 'acquiring' || lease.operationId !== response.lease.operationId) {
          throw new Error('persistence worker returned a mismatched export lease');
        }
        this.exportLease = { phase: 'active', operationId: lease.operationId };
        lease.resolve(response.lease);
        return;
      }
      if (response.type === 'checkpointRetentionApplied') {
        const pending = this.pruning;
        if (!pending || pending.operationId !== response.operationId) {
          throw new Error('persistence worker returned a mismatched retention result');
        }
        this.pruning = undefined;
        pending.resolve(response.result);
        return;
      }
      if (response.type === 'browserHistoryRead') {
        const pending = this.history;
        if (!pending || pending.operationId !== response.operationId || pending.runId !== response.runId) {
          throw new Error('persistence worker returned mismatched browser history');
        }
        this.history = undefined;
        pending.resolve(response.history);
        return;
      }
      if (response.type === 'browserHallOfFameRead') {
        const pending = this.hallOfFame;
        if (!pending || pending.operationId !== response.operationId || pending.runId !== response.runId) {
          throw new Error('persistence worker returned mismatched browser Hall of Fame');
        }
        this.hallOfFame = undefined;
        pending.resolve(response.entries);
        return;
      }
      if (response.type === 'graphPresetSaved') {
        const pending = this.graphPresets.get(response.operationId);
        if (!pending || pending.kind !== 'save') {
          throw new Error('persistence worker returned a mismatched graph preset save');
        }
        this.graphPresets.delete(response.operationId);
        pending.resolve(response.presetId);
        return;
      }
      if (response.type === 'graphPresetsListed') {
        const pending = this.graphPresets.get(response.operationId);
        if (!pending || pending.kind !== 'list') {
          throw new Error('persistence worker returned a mismatched graph preset list');
        }
        this.graphPresets.delete(response.operationId);
        pending.resolve(response.presets);
        return;
      }
      if (response.type === 'graphPresetLoaded') {
        const pending = this.graphPresets.get(response.operationId);
        if (!pending || pending.kind !== 'load') {
          throw new Error('persistence worker returned a mismatched graph preset load');
        }
        this.graphPresets.delete(response.operationId);
        pending.resolve(response.preset);
        return;
      }
      if (response.type === 'hallOfFameEntrySelected') {
        const pending = this.hallOfFameSelection;
        const selected = response.selection;
        if (!pending || pending.phase !== 'acquiring' || pending.operationId !== selected.operationId || pending.runId !== selected.runId ||
            pending.entryId !== selected.entryId) {
          throw new Error('persistence worker returned mismatched Hall-of-Fame selection');
        }
        this.hallOfFameSelection = { phase: 'active', operationId: pending.operationId,
          runId: pending.runId, entryId: pending.entryId };
        pending.resolve(selected);
        return;
      }
      if (response.type === 'currentCheckpointPinned') {
        const pending = this.pin;
        if (!pending || pending.operationId !== response.operationId) {
          throw new Error('persistence worker returned a mismatched checkpoint pin');
        }
        this.pin = undefined;
        pending.resolve({ checkpointId: response.checkpointId, generation: response.generation });
        return;
      }
      if (response.type === 'checkpointRetentionInspected') {
        const pending = this.retention;
        if (!pending || pending.operationId !== response.operationId) {
          throw new Error('persistence worker returned a mismatched retention inventory');
        }
        this.retention = undefined;
        pending.resolve(response.inventory);
        return;
      }
      if (response.type === 'managedStorageInspected') {
        const pending = this.storage;
        if (!pending || pending.operationId !== response.operationId) {
          throw new Error('persistence worker returned mismatched storage diagnostics');
        }
        this.storage = undefined;
        pending.resolve(response.diagnostics);
        return;
      }
      if (response.type === 'recoveryCandidate') {
        const pending = this.scan;
        const cursor = response.result.cursor;
        const previous = pending?.cursor;
        if (!pending || pending.operationId !== response.operationId ||
            (pending.checkpointId !== undefined ? cursor.checkpointId !== pending.checkpointId ||
              !cursor.explicitResume || response.result.exhausted : cursor.explicitResume !== undefined) || (previous &&
            (previous.sourceRunId !== cursor.sourceRunId || previous.failedCheckpointId !== cursor.failedCheckpointId ||
              (!response.result.exhausted && previous.generation !== null &&
                (cursor.generation! > previous.generation || (cursor.generation === previous.generation && cursor.checkpointId! >= previous.checkpointId!)))))) {
          throw new Error('recovery scan response does not advance the pinned source');
        }
        this.scan = undefined;
        pending.resolve(response.result);
        return;
      }
      if (response.type === 'recoveryBranchCommitted') {
        const pending = this.recovery;
        const { abandonedThroughGeneration: _suffix, ...commit } = response.result;
        if (!pending || JSON.stringify(commit) !== JSON.stringify(pending.commit)) {
          throw new Error('persistence worker returned a mismatched recovery acknowledgement');
        }
        this.recovery = undefined;
        pending.resolve(response.result);
        return;
      }
      if (response.type === 'managedCheckpointSelected') {
        const selection = this.selection;
        if (!selection || response.operationId !== selection.operationId ||
            (selection.runId !== null && response.descriptor !== null && response.runId !== selection.runId)) {
          throw new Error('persistence worker returned a mismatched checkpoint selection');
        }
        this.selection = undefined;
        selection.resolve({ descriptor: response.descriptor, runId: response.runId,
          recovery: response.recovery, importBranch: response.importBranch,
          legacyConversion: response.legacyConversion });
        return;
      }
      if (response.type === 'legacySnapshotSelected') {
        const selection = this.legacySelection;
        if (!selection || response.operationId !== selection.operationId) {
          throw new Error('persistence worker returned a mismatched legacy selection');
        }
        this.legacySelection = undefined;
        selection.resolve(response.selection);
        return;
      }
      if (response.type === 'managedCheckpointRejected') {
        if (!response.operationId) {
          throw new Error(`persistence worker rejected an uncorrelated request: ${response.reason}`);
        }
        if (response.operationId === this.scan?.operationId) {
          const pending = this.scan;
          this.scan = undefined;
          pending.reject(new Error(response.reason));
          return;
        }
        if (response.operationId === this.recovery?.commit.operationId) {
          const pending = this.recovery;
          this.recovery = undefined;
          pending.reject(new Error(response.reason));
          return;
        }
        if (response.operationId === this.selection?.operationId) {
          const selection = this.selection;
          this.selection = undefined;
          selection.reject(new Error(response.reason));
          return;
        }
        if (response.operationId === this.retention?.operationId) {
          const pending = this.retention;
          this.retention = undefined;
          pending.reject(new Error(response.reason));
          return;
        }
        if (response.operationId === this.storage?.operationId) {
          const pending = this.storage;
          this.storage = undefined;
          pending.reject(new Error(response.reason));
          return;
        }
        if (response.operationId === this.pin?.operationId) {
          const pending = this.pin;
          this.pin = undefined;
          pending.reject(new Error(response.reason));
          return;
        }
        if (response.operationId === this.pruning?.operationId) {
          const pending = this.pruning;
          this.pruning = undefined;
          pending.reject(new Error(response.reason));
          return;
        }
        if (response.operationId === this.orphanCleanup?.operationId) {
          const pending = this.orphanCleanup;
          this.orphanCleanup = undefined;
          pending.reject(new Error(response.reason));
          return;
        }
        if (response.operationId === this.history?.operationId) {
          const pending = this.history;
          this.history = undefined;
          pending.reject(new Error(response.reason));
          return;
        }
        if (response.operationId === this.hallOfFame?.operationId) {
          const pending = this.hallOfFame;
          this.hallOfFame = undefined;
          pending.reject(new Error(response.reason));
          return;
        }
        if (response.operationId === this.legacySelection?.operationId) {
          const selection = this.legacySelection;
          this.legacySelection = undefined;
          selection.reject(new Error(response.reason));
          return;
        }
        const savedBrain = this.savedBrains.get(response.operationId);
        if (savedBrain) { this.savedBrains.delete(response.operationId); savedBrain.reject(new Error(response.reason)); return; }
        const graphPreset = this.graphPresets.get(response.operationId);
        if (graphPreset) {
          this.graphPresets.delete(response.operationId);
          graphPreset.reject(new Error(response.reason));
          return;
        }
        if (response.operationId === this.hallOfFameSelection?.operationId &&
            this.hallOfFameSelection.phase !== 'active') {
          const pending = this.hallOfFameSelection;
          if (pending.phase === 'releasing') {
            this.hallOfFameSelection = { phase: 'active', operationId: pending.operationId,
              runId: pending.runId, entryId: pending.entryId };
          } else {
            this.hallOfFameSelection = undefined;
          }
          pending.reject(new Error(response.reason));
          return;
        }
        if (response.operationId === this.exportLease?.operationId && this.exportLease.phase !== 'active') {
          const lease = this.exportLease;
          if (lease.phase === 'releasing') this.exportLease = { phase: 'active', operationId: lease.operationId };
          else this.exportLease = undefined;
          lease.reject(new Error(response.reason));
          return;
        }
        const pending = this.pending.get(response.operationId);
        if (!pending) {
          throw new Error(`persistence worker rejected unknown operation ${response.operationId}`);
        }
        this.pending.delete(response.operationId);
        pending.reject(new Error(response.reason));
        return;
      }
      const pending = this.pending.get(response.operationId);
      if (!pending) {
        throw new Error(`persistence worker acknowledged unknown operation ${response.operationId}`);
      }
      const descriptorMatches = pending.import
        ? managedCheckpointContentsEqual(response.descriptor, pending.descriptor)
        : managedCheckpointDescriptorsEqual(response.descriptor, pending.descriptor);
      const expectedRunId = pending.branchRunId ?? pending.descriptor.runId;
      if (response.runId !== expectedRunId ||
        response.checkpointId !== pending.descriptor.logicalRootSha256 || !descriptorMatches ||
        (!pending.import && response.transitionEpoch !== pending.descriptor.transitionEpoch) ||
        (pending.import && response.type !== 'managedImportCommitted') ||
        (pending.import && response.type === 'managedImportCommitted' &&
          (response.importBranch?.branchRunId ?? null) !== pending.branchRunId) ||
        (!pending.import && response.type !== 'managedCheckpointCommitted')) {
        throw new Error(`persistence worker acknowledgement mismatched operation ${response.operationId}`);
      }
      this.pending.delete(response.operationId);
      pending.resolve({
        operationId: response.operationId,
        transitionEpoch: response.transitionEpoch,
        runId: response.runId,
        checkpointId: response.checkpointId,
        descriptor: response.descriptor,
        ...(pending.import ? { importBranch: response.type === 'managedImportCommitted'
          ? response.importBranch : null } : {})
      });
    } catch (error) {
      this.fail(asError(error));
    }
  }

  /**
   * Retain a terminal worker failure and reject all unresolved commits exactly once.
   * @param error - Terminal lifecycle or protocol error.
   */
  private fail(error: Error): void {
    if (this.failure) return;
    this.failure = error;
    this.stopping = true;
    if (this.watchdog) clearTimeout(this.watchdog);
    this.watchdog = undefined;
    this.watchedOperations.clear();
    for (const pending of this.pending.values()) pending.reject(error);
    this.pending.clear();
    this.scan?.reject(error);
    this.scan = undefined;
    this.recovery?.reject(error);
    this.recovery = undefined;
    this.selection?.reject(error);
    this.selection = undefined;
    this.legacySelection?.reject(error);
    this.legacySelection = undefined;
    this.retention?.reject(error);
    this.retention = undefined;
    this.storage?.reject(error);
    this.storage = undefined;
    this.pin?.reject(error);
    this.pin = undefined;
    this.pruning?.reject(error);
    this.pruning = undefined;
    this.history?.reject(error);
    this.history = undefined;
    this.hallOfFame?.reject(error);
    this.hallOfFame = undefined;
    this.orphanCleanup?.reject(error);
    this.orphanCleanup = undefined;
    for (const pending of this.savedBrains.values()) pending.reject(error);
    this.savedBrains.clear();
    for (const pending of this.graphPresets.values()) pending.reject(error);
    this.graphPresets.clear();
    if (this.hallOfFameSelection?.phase !== 'active') this.hallOfFameSelection?.reject(error);
    this.hallOfFameSelection = undefined;
    if (this.exportLease?.phase !== 'active') this.exportLease?.reject(error);
    this.exportLease = undefined;
    void this.terminateForFailure();
  }

  /**
   * Terminate a protocol-faulted worker and wait for its exit without adding a watchdog timeout.
   * @returns Promise resolved only after the terminated worker has exited.
   */
  private terminateForFailure(): Promise<void> {
    if (this.workerExited) return Promise.resolve();
    if (!this.terminationPromise) {
      this.terminationPromise = this.worker.terminate().then(
        () => this.exitPromise,
        () => this.exitPromise
      );
    }
    return this.terminationPromise;
  }

  /**
   * Reject pending work on unexpected exit, or complete an orderly close on clean exit.
   * @param code - Worker process exit code.
   */
  private onExit(code: number): void {
    this.workerExited = true;
    this.resolveExited();
    if (this.failure) {
      this.rejectStopped?.(this.failure);
      this.resolveStopped = null;
      this.rejectStopped = null;
      return;
    }
    if (this.stopping && code === 0 && this.pending.size === 0 && this.savedBrains.size === 0 && this.graphPresets.size === 0 && !this.selection && !this.legacySelection && !this.recovery && !this.scan && !this.retention && !this.storage && !this.pin && !this.pruning && !this.history && !this.hallOfFame &&
        !this.orphanCleanup && (!this.hallOfFameSelection || this.hallOfFameSelection.phase === 'active') &&
        (!this.exportLease || this.exportLease.phase === 'active')) {
      this.resolveStopped?.();
      this.resolveStopped = null;
      this.rejectStopped = null;
      return;
    }
    const failure = this.stopping && code === 0
      ? new Error(
          `checkpoint persistence worker exited cleanly with ${this.pending.size} pending operation(s)`
        )
      : new Error(`checkpoint persistence worker exited with code ${code}`);
    this.fail(failure);
    this.rejectStopped?.(failure);
    this.resolveStopped = null;
    this.rejectStopped = null;
  }
}

/**
 * Convert an unknown thrown value to an Error instance.
 * @param error - Unknown caught value.
 * @returns Error preserving available message text.
 */
function asError(error: unknown): Error {
  return error instanceof Error ? error : new Error(String(error));
}
