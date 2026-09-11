import { parseRecoveryScanCursor, parseRecoveryScanResult, type RecoveryScanCursor, type RecoveryScanResult, parseRecoveryBranchCommit, parseRecoveryBranchResult, type RecoveryBranchCommit, type RecoveryBranchResult } from './recoveryProtocol.ts';
import { Worker } from 'node:worker_threads';
import { randomBytes } from 'node:crypto';
import {
  parseCheckpointPruneResult,
  parseCheckpointRetentionInventory,
  type CheckpointPruneResult,
  type CheckpointRetentionInventory
} from './checkpointRetention.ts';
import {
  DEFAULT_MANAGED_CHECKPOINT_DESCRIPTOR_LIMITS,
  managedCheckpointContentsEqual,
  managedCheckpointDescriptorsEqual,
  parseManagedCheckpointDescriptor,
  parseManagedCheckpointDescriptorLimits,
  parseManagedExportInventoryDescriptor,
  parseManagedImportBranchResult,
  parseManagedImportInventoryDescriptor,
  parseManagedGenerationCommit,
  type CheckpointOperationId,
  type CheckpointPersistenceWorkerResponse,
  type ManagedCheckpointDescriptor,
  type ManagedCheckpointExportLease,
  type ManagedCheckpointSelection,
  type ManagedCheckpointDescriptorLimits,
  type ManagedGenerationCommit,
  type ManagedImportBranchResult,
  type ManagedImportInventoryDescriptor,
  type U64Hex
} from './checkpointPersistenceProtocol.ts';

/** Options for the client-owned isolated persistence worker. */
export interface CheckpointPersistenceClientOptions {
  /** Disposable/test SQLite database path supplied to the isolated worker. */
  databasePath: string;
  /** Resume requires an existing managed-metadata schema and never initializes another database. */
  existingOnly?: boolean;
  /** Existing controlled root containing final immutable checkpoint-v3 files. */
  managedRootPath: string;
  /** Explicit bounded descriptor limits, defaulting only to the provisional Stage 3 envelope. */
  limits?: ManagedCheckpointDescriptorLimits;
  /** Test-only worker module override for client protocol/lifecycle tests. */
  workerUrlForTesting?: URL;
  /** Test-only response mode consumed exclusively by a supplied test worker module. */
  workerResponseModeForTesting?: 'invalid' | 'mismatched' | 'exit' | 'exit-clean';
}

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
  /** One candidate read; a corrupt row advances only its stable scalar cursor. */
  private scan: { operationId: string; cursor: RecoveryScanCursor | null; resolve(value: RecoveryScanResult): void; reject(error: Error): void } | undefined;
  /** One startup recovery transaction; retries use the same caller-owned operation token. */
  private recovery: { commit: RecoveryBranchCommit; resolve(value: RecoveryBranchResult): void; reject(error: Error): void } | undefined;
  /** At most one bounded retention inventory read may be in flight. */
  private retention: { operationId: CheckpointOperationId; resolve(value: CheckpointRetentionInventory): void; reject(error: Error): void } | undefined;
  /** At most one owner pin transaction may be in flight. */
  private pin: { operationId: CheckpointOperationId; resolve(value: PinnedCheckpointResult): void; reject(error: Error): void } | undefined;
  /** At most one verified automatic pruning pass may be in flight. */
  private pruning: { operationId: CheckpointOperationId; resolve(value: CheckpointPruneResult): void; reject(error: Error): void } | undefined;
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
  /** One best-effort termination request started only for a terminal client failure. */
  private terminationPromise: Promise<void> | null = null;
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
    const limits = parseManagedCheckpointDescriptorLimits(
      options.limits ?? DEFAULT_MANAGED_CHECKPOINT_DESCRIPTOR_LIMITS
    );
    const workerUrl = options.workerUrlForTesting ??
      new URL('./checkpointPersistenceWorker.ts', import.meta.url);
    this.worker = new Worker(workerUrl, {
      workerData: {
        databasePath: options.databasePath,
        managedRootPath: options.managedRootPath,
        limits,
        existingOnly: options.existingOnly ?? false,
        ...(options.workerUrlForTesting && options.workerResponseModeForTesting
          ? { checkpointPersistenceTestMode: options.workerResponseModeForTesting }
          : {})
      }
    });
    this.exitPromise = new Promise<void>(resolve => { this.resolveExited = resolve; });
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
   * @returns Matching durable metadata/current-pointer acknowledgement.
   */
  commit(
    value: unknown,
    generationCommitValue: unknown = null,
    activateRun = false
  ): Promise<ManagedCheckpointCommitResult> {
    if (this.failure) return Promise.reject(this.failure);
    if (this.stopping) return Promise.reject(new Error('checkpoint persistence client is stopping'));
    let descriptor: ManagedCheckpointDescriptor;
    let generationCommit: ManagedGenerationCommit | null;
    try {
      descriptor = parseManagedCheckpointDescriptor(value);
      generationCommit = parseManagedGenerationCommit(generationCommitValue, descriptor);
    } catch (error) {
      return Promise.reject(asError(error));
    }
    if (this.pending.has(descriptor.operationId)) {
      return Promise.reject(new Error(`checkpoint operation ${descriptor.operationId} is already pending`));
    }
    return new Promise<ManagedCheckpointCommitResult>((resolve, reject) => {
      this.pending.set(descriptor.operationId, { descriptor, import: false, branchRunId: null, resolve, reject });
      try {
        this.worker.postMessage({ type: 'commitManagedCheckpoint', descriptor, generationCommit, activateRun });
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
      try { this.worker.postMessage({ type: 'selectManagedCheckpoint', operationId, runId }); }
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
      try { this.worker.postMessage({ type: 'scanRecoveryCandidate', operationId, cursor }); }
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
      try { this.worker.postMessage({ type: 'commitRecoveryBranch', commit }); }
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
        this.worker.postMessage({ type: 'commitManagedImport', descriptor, inventory, branchRunId });
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
      try { this.worker.postMessage({ type: 'inspectCheckpointRetention', operationId }); }
      catch (error) { this.retention = undefined; reject(asError(error)); }
    });
  }

  /** Atomically pin the effective active run's exact current managed file. */
  pinCurrentCheckpoint(): Promise<PinnedCheckpointResult> {
    if (this.failure) return Promise.reject(this.failure);
    if (this.stopping || this.pin) return Promise.reject(new Error('checkpoint pin is busy or stopping'));
    const operationId = randomBytes(16).toString('hex');
    return new Promise((resolve, reject) => {
      this.pin = { operationId, resolve, reject };
      try { this.worker.postMessage({ type: 'pinCurrentCheckpoint', operationId }); }
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
      try { this.worker.postMessage({ type: 'acquireCurrentExportLease', operationId }); }
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
      try { this.worker.postMessage({ type: 'releaseExportLease', operationId }); }
      catch (error) { this.exportLease = { phase: 'active', operationId }; reject(asError(error)); }
    });
  }

  /** Apply the owner retention rule to verified unpinned managed files. */
  applyRetention(): Promise<CheckpointPruneResult> {
    if (this.failure) return Promise.reject(this.failure);
    if (this.stopping || this.pruning) return Promise.reject(new Error('checkpoint retention pruning is busy or stopping'));
    const operationId = randomBytes(16).toString('hex');
    return new Promise((resolve, reject) => {
      this.pruning = { operationId, resolve, reject };
      try { this.worker.postMessage({ type: 'applyCheckpointRetention', operationId }); }
      catch (error) { this.pruning = undefined; reject(asError(error)); }
    });
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
      if (response.type === 'recoveryCandidate') {
        const pending = this.scan;
        const cursor = response.result.cursor;
        const previous = pending?.cursor;
        if (!pending || pending.operationId !== response.operationId || (previous &&
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
          recovery: response.recovery, importBranch: response.importBranch });
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
    for (const pending of this.pending.values()) pending.reject(error);
    this.pending.clear();
    this.scan?.reject(error);
    this.scan = undefined;
    this.recovery?.reject(error);
    this.recovery = undefined;
    this.selection?.reject(error);
    this.selection = undefined;
    this.retention?.reject(error);
    this.retention = undefined;
    this.pin?.reject(error);
    this.pin = undefined;
    this.pruning?.reject(error);
    this.pruning = undefined;
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
    if (this.stopping && code === 0 && this.pending.size === 0 && !this.selection && !this.recovery && !this.scan && !this.retention && !this.pin && !this.pruning &&
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

/**
 * Validate a response has only the exact scalar fields defined by the worker protocol.
 * @param value - Unknown structured-cloned response.
 * @returns Strict worker response.
 */
function parseWorkerResponse(value: unknown): CheckpointPersistenceWorkerResponse {
  if (value === null || typeof value !== 'object' || Array.isArray(value) ||
    value instanceof ArrayBuffer || ArrayBuffer.isView(value)) {
    throw new TypeError('checkpoint persistence worker sent a non-object response');
  }
  const response = value as Record<string, unknown>;
  if (response['type'] === 'exportLeaseReleased') {
    requireExactKeys(response, ['type', 'operationId']);
    if (!isOperationId(response['operationId'])) throw new TypeError('invalid export lease release correlation');
    return { type: 'exportLeaseReleased', operationId: response['operationId'] };
  }
  if (response['type'] === 'currentExportLeaseAcquired') {
    requireExactKeys(response, ['type', 'lease']);
    if (!response['lease'] || typeof response['lease'] !== 'object' || Array.isArray(response['lease'])) {
      throw new TypeError('invalid checkpoint export lease');
    }
    const lease = response['lease'] as Record<string, unknown>;
    requireExactKeys(lease, ['operationId', 'runId', 'descriptor', 'inventory']);
    if (!isOperationId(lease['operationId']) || typeof lease['runId'] !== 'string' || !lease['runId'] ||
        lease['runId'].includes('\0') || Buffer.byteLength(lease['runId']) > 256) {
      throw new TypeError('invalid checkpoint export lease identity');
    }
    const descriptor = parseManagedCheckpointDescriptor(lease['descriptor']);
    if (descriptor.runId !== lease['runId']) throw new TypeError('checkpoint export lease run identity mismatch');
    const inventory = parseManagedExportInventoryDescriptor(lease['inventory'], lease['operationId']);
    return { type: 'currentExportLeaseAcquired', lease: {
      operationId: lease['operationId'],
      runId: lease['runId'],
      descriptor,
      inventory
    } };
  }
  if (response['type'] === 'checkpointRetentionApplied') {
    requireExactKeys(response, ['type', 'operationId', 'result']);
    if (!isOperationId(response['operationId'])) throw new TypeError('invalid retention pruning correlation');
    return {
      type: 'checkpointRetentionApplied',
      operationId: response['operationId'],
      result: parseCheckpointPruneResult(response['result'])
    };
  }
  if (response['type'] === 'currentCheckpointPinned') {
    requireExactKeys(response, ['type', 'operationId', 'checkpointId', 'generation']);
    if (!isOperationId(response['operationId']) || typeof response['checkpointId'] !== 'string' ||
        !/^[0-9a-f]{64}$/u.test(response['checkpointId']) || !isU64Hex(response['generation'])) {
      throw new TypeError('invalid checkpoint pin acknowledgement');
    }
    return {
      type: 'currentCheckpointPinned', operationId: response['operationId'],
      checkpointId: response['checkpointId'], generation: response['generation']
    };
  }
  if (response['type'] === 'checkpointRetentionInspected') {
    requireExactKeys(response, ['type', 'operationId', 'inventory']);
    if (!isOperationId(response['operationId'])) throw new TypeError('invalid retention inventory correlation');
    return {
      type: 'checkpointRetentionInspected',
      operationId: response['operationId'],
      inventory: parseCheckpointRetentionInventory(response['inventory'])
    };
  }
  if (response['type'] === 'recoveryCandidate') {
    requireExactKeys(response, ['type', 'operationId', 'result']);
    if (!isOperationId(response['operationId'])) throw new Error('invalid recovery scan correlation');
    return { type: 'recoveryCandidate', operationId: response['operationId'], result: parseRecoveryScanResult(response['result']) };
  }
  if (response['type'] === 'recoveryBranchCommitted') {
    requireExactKeys(response, ['type', 'result']);
    return { type: 'recoveryBranchCommitted', result: parseRecoveryBranchResult(response['result']) };
  }
  if (response['type'] === 'managedCheckpointSelected') {
    requireExactKeys(response, ['type', 'operationId', 'descriptor', 'runId', 'recovery', 'importBranch']);
    if (!isOperationId(response['operationId'])) throw new TypeError('invalid checkpoint selection correlation');
    const descriptor = response['descriptor'] === null ? null : parseManagedCheckpointDescriptor(response['descriptor']);
    const recovery = response['recovery'] === null ? null : parseRecoveryBranchResult(response['recovery']);
    const importBranch = response['importBranch'] === null
      ? null : parseManagedImportBranchResult(response['importBranch']);
    const runId = response['runId'];
    const branch = recovery ?? importBranch;
    if (descriptor === null) {
      if (runId !== null || branch !== null) throw new Error('empty selection contains lineage');
    } else if (typeof runId !== 'string' || !runId || Buffer.byteLength(runId) > 256 ||
        (recovery !== null && importBranch !== null) || (branch && branch.branchRunId !== runId) ||
        (descriptor.runId !== runId && (!branch || !managedCheckpointDescriptorsEqual(branch.recoveredDescriptor, descriptor)))) {
      throw new Error('selected checkpoint lacks matching branch provenance');
    }
    return { type: 'managedCheckpointSelected', operationId: response['operationId'], descriptor,
      runId: runId as string | null, recovery, importBranch };
  }
  if (response['type'] === 'managedCheckpointCommitted' || response['type'] === 'managedImportCommitted') {
    requireExactKeys(response, [
      'type',
      'operationId',
      'transitionEpoch',
      'runId',
      'checkpointId',
      'descriptor',
      ...(response['type'] === 'managedImportCommitted' ? ['importBranch'] : [])
    ]);
    if (!isOperationId(response['operationId']) || !isU64Hex(response['transitionEpoch']) ||
      typeof response['runId'] !== 'string' || typeof response['checkpointId'] !== 'string') {
      throw new TypeError('checkpoint persistence worker sent an invalid commit acknowledgement');
    }
    const descriptor = parseManagedCheckpointDescriptor(response['descriptor']);
    const importBranch = response['type'] === 'managedImportCommitted'
      ? response['importBranch'] === null ? null : parseManagedImportBranchResult(response['importBranch'])
      : null;
    if ((response['type'] === 'managedCheckpointCommitted' && response['operationId'] !== descriptor.operationId) ||
      response['transitionEpoch'] !== descriptor.transitionEpoch ||
      (response['runId'] !== descriptor.runId &&
        (!importBranch || importBranch.branchRunId !== response['runId'] ||
          !managedCheckpointDescriptorsEqual(importBranch.recoveredDescriptor, descriptor))) ||
      response['checkpointId'] !== descriptor.logicalRootSha256) {
      throw new TypeError('checkpoint persistence worker sent internally mismatched commit fields');
    }
    const common = {
      operationId: response['operationId'],
      transitionEpoch: response['transitionEpoch'],
      runId: response['runId'],
      checkpointId: response['checkpointId'],
      descriptor
    };
    return response['type'] === 'managedImportCommitted'
      ? { type: 'managedImportCommitted', ...common, importBranch }
      : { type: 'managedCheckpointCommitted', ...common };
  }
  if (response['type'] === 'managedCheckpointRejected') {
    requireExactKeys(response, ['type', 'operationId', 'reason']);
    if ((response['operationId'] !== null && !isOperationId(response['operationId'])) ||
      typeof response['reason'] !== 'string') {
      throw new TypeError('checkpoint persistence worker sent an invalid rejection');
    }
    return { type: 'managedCheckpointRejected', operationId: response['operationId'], reason: response['reason'] };
  }
  throw new TypeError('checkpoint persistence worker sent an unknown response type');
}

/**
 * Require that an object has exactly the specified own keys.
 * @param value - Response object to inspect.
 * @param keys - Required and exclusive key set.
 */
function requireExactKeys(value: Record<string, unknown>, keys: readonly string[]): void {
  if (Object.keys(value).length !== keys.length || keys.some(key => !Object.hasOwn(value, key))) {
    throw new TypeError('checkpoint persistence worker response has unknown or missing fields');
  }
}

/**
 * Check the exact nonnumeric operation-token wire format.
 * @param value - Candidate operation token.
 * @returns True only for a canonical operation token.
 */
function isOperationId(value: unknown): value is CheckpointOperationId {
  return typeof value === 'string' && /^[0-9a-f]{32}$/u.test(value);
}

/**
 * Check the exact fixed-width unsigned-64-bit hexadecimal wire format.
 * @param value - Candidate wire value.
 * @returns True only for a canonical u64 value.
 */
function isU64Hex(value: unknown): value is U64Hex {
  return typeof value === 'string' && /^[0-9a-f]{16}$/u.test(value);
}
