import { Worker } from 'node:worker_threads';
import {
  DEFAULT_MANAGED_CHECKPOINT_DESCRIPTOR_LIMITS,
  managedCheckpointDescriptorsEqual,
  parseManagedCheckpointDescriptor,
  parseManagedCheckpointDescriptorLimits,
  parseManagedGenerationCommit,
  type CheckpointOperationId,
  type CheckpointPersistenceWorkerResponse,
  type ManagedCheckpointDescriptor,
  type ManagedCheckpointDescriptorLimits,
  type ManagedGenerationCommit,
  type U64Hex
} from './checkpointPersistenceProtocol.ts';

/** Options for the client-owned isolated persistence worker. */
export interface CheckpointPersistenceClientOptions {
  /** Disposable/test SQLite database path supplied to the isolated worker. */
  databasePath: string;
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
  /** Resolve callback for its matching acknowledgement. */
  resolve: (result: ManagedCheckpointCommitResult) => void;
  /** Reject callback for rejection, protocol fault, or worker exit. */
  reject: (error: Error) => void;
}

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
   * @returns Matching durable metadata/current-pointer acknowledgement.
   */
  commit(
    value: unknown,
    generationCommitValue: unknown = null
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
      this.pending.set(descriptor.operationId, { descriptor, resolve, reject });
      try {
        this.worker.postMessage({
          type: 'commitManagedCheckpoint',
          descriptor,
          generationCommit
        });
      } catch (error) {
        this.pending.delete(descriptor.operationId);
        reject(asError(error));
      }
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
      if (response.type === 'managedCheckpointRejected') {
        if (!response.operationId) {
          throw new Error(`persistence worker rejected an uncorrelated request: ${response.reason}`);
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
      if (
        response.transitionEpoch !== pending.descriptor.transitionEpoch ||
        response.runId !== pending.descriptor.runId ||
        response.checkpointId !== pending.descriptor.logicalRootSha256 ||
        !managedCheckpointDescriptorsEqual(response.descriptor, pending.descriptor)
      ) {
        throw new Error(`persistence worker acknowledgement mismatched operation ${response.operationId}`);
      }
      this.pending.delete(response.operationId);
      pending.resolve({
        operationId: response.operationId,
        transitionEpoch: response.transitionEpoch,
        runId: response.runId,
        checkpointId: response.checkpointId,
        descriptor: response.descriptor
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
    if (this.stopping && code === 0 && this.pending.size === 0) {
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
  if (response['type'] === 'managedCheckpointCommitted') {
    requireExactKeys(response, [
      'type',
      'operationId',
      'transitionEpoch',
      'runId',
      'checkpointId',
      'descriptor'
    ]);
    if (!isOperationId(response['operationId']) || !isU64Hex(response['transitionEpoch']) ||
      typeof response['runId'] !== 'string' || typeof response['checkpointId'] !== 'string') {
      throw new TypeError('checkpoint persistence worker sent an invalid commit acknowledgement');
    }
    const descriptor = parseManagedCheckpointDescriptor(response['descriptor']);
    if (response['operationId'] !== descriptor.operationId ||
      response['transitionEpoch'] !== descriptor.transitionEpoch ||
      response['runId'] !== descriptor.runId ||
      response['checkpointId'] !== descriptor.logicalRootSha256) {
      throw new TypeError('checkpoint persistence worker sent internally mismatched commit fields');
    }
    return {
      type: 'managedCheckpointCommitted',
      operationId: response['operationId'],
      transitionEpoch: response['transitionEpoch'],
      runId: response['runId'],
      checkpointId: response['checkpointId'],
      descriptor
    };
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
