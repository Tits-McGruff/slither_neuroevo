/**
 * Thin Node orchestration for one Rust-owned pending fresh run start.
 *
 * Callers supply only the operation token. Rust supplies the immutable
 * generation-one/step-zero descriptor, including its transition correlation
 * value. Node commits that scalar descriptor through the dedicated worker and
 * returns only the worker's exact complete echo to Rust.
 */

import {
  managedCheckpointCommitResultMatchesDescriptor,
  type ManagedCheckpointCommitResult
} from './checkpointPersistenceClient.ts';
import {
  parseCheckpointOperationId,
  parseManagedCheckpointDescriptor,
  type CheckpointOperationId,
  type ManagedCheckpointDescriptor
} from './checkpointPersistenceProtocol.ts';

/** Exact options accepted by Rust's managed run-start publisher. */
export interface RustRunStartCheckpointPublishOptions {
  /** Server-controlled managed checkpoint directory. */
  managedDirectory: string;
  /** Correlation token retained by the pending Rust transition. */
  operationId: CheckpointOperationId;
}

/** Narrow production port implemented by the Rust-owned fresh-run session. */
export interface RustRunStartPersistencePort {
  /** Publish or exactly retry the already-admitted run-start checkpoint. */
  publishRunStartCheckpoint(options: RustRunStartCheckpointPublishOptions): Promise<unknown>;
  /** Retain one successful worker commit only when its complete descriptor matches. */
  acknowledgeRunStartPersistence(descriptor: ManagedCheckpointDescriptor): void;
}

/** Narrow persistence dependency implemented by `CheckpointPersistenceClient`. */
export interface RunStartCheckpointCommitter {
  /** Commit the exact Rust descriptor with no completed-generation metadata. */
  commit(descriptor: unknown): Promise<ManagedCheckpointCommitResult>;
}

/** Construction dependencies for one fresh run-start handoff owner. */
export interface RunStartPersistenceHandoffOptions {
  /** Rust session retaining the actual pending run start. */
  rust: RustRunStartPersistencePort;
  /** Dedicated worker client owning the SQLite transaction. */
  persistence: RunStartCheckpointCommitter;
  /** Controlled managed root passed to Rust's file publisher. */
  managedDirectory: string;
}

/** One coalesced in-flight operation; a different operation cannot overlap it. */
interface ActiveRunStartPersistence {
  /** Exact operation being published and committed. */
  operationId: CheckpointOperationId;
  /** Identity used to clear only this particular completion. */
  token: object;
  /** Shared result returned to duplicate callers for the same operation. */
  promise: Promise<ManagedCheckpointCommitResult>;
}

/**
 * Single-owner run-start persistence barrier between Rust and the SQLite worker.
 *
 * Same-operation concurrent calls share one publication/transaction attempt.
 * Failure clears only that attempt so an explicit retry reuses Rust's unchanged
 * staged boundary and the worker's exact replay behavior.
 */
export class RunStartPersistenceHandoff {
  /** Rust authority port that supplies the complete run-start descriptor. */
  private readonly rust: RustRunStartPersistencePort;
  /** Dedicated SQLite worker client. */
  private readonly persistence: RunStartCheckpointCommitter;
  /** Controlled directory receiving Rust's immutable file. */
  private readonly managedDirectory: string;
  /** At most one currently executing publication/commit/acknowledgement. */
  private active: ActiveRunStartPersistence | null = null;

  /**
   * Bind one Rust fresh-run port to one persistence worker client.
   * @param options - Exact ports and controlled managed directory.
   */
  constructor(options: RunStartPersistenceHandoffOptions) {
    if (typeof options.rust?.publishRunStartCheckpoint !== 'function' ||
      typeof options.rust?.acknowledgeRunStartPersistence !== 'function') {
      throw new TypeError('run-start persistence handoff requires a complete Rust port');
    }
    if (typeof options.persistence?.commit !== 'function') {
      throw new TypeError('run-start persistence handoff requires a persistence client');
    }
    if (typeof options.managedDirectory !== 'string' || options.managedDirectory.length === 0 ||
      options.managedDirectory.includes('\0')) {
      throw new TypeError('run-start persistence managedDirectory must be a nonempty path');
    }
    this.rust = options.rust;
    this.persistence = options.persistence;
    this.managedDirectory = options.managedDirectory;
  }

  /**
   * Publish, commit and acknowledge the exact pending Rust run start.
   * @param operationIdValue - Correlation token only; no run data is accepted.
   * @returns The persistence worker's exact committed descriptor acknowledgement.
   */
  commitPendingRunStart(operationIdValue: unknown): Promise<ManagedCheckpointCommitResult> {
    let operationId: CheckpointOperationId;
    try {
      operationId = parseCheckpointOperationId(operationIdValue);
    } catch (error) {
      return Promise.reject(error);
    }
    if (this.active) {
      if (this.active.operationId === operationId) return this.active.promise;
      return Promise.reject(new Error(
        `run-start persistence operation ${this.active.operationId} is already in flight`
      ));
    }

    const token = {};
    const promise = this.commitOne(operationId).finally(() => {
      if (this.active?.token === token) this.active = null;
    });
    this.active = { operationId, token, promise };
    return promise;
  }

  /** Execute one complete direct Rust-to-worker-to-Rust attempt. */
  private async commitOne(
    operationId: CheckpointOperationId
  ): Promise<ManagedCheckpointCommitResult> {
    const descriptor = parseManagedCheckpointDescriptor(
      await this.rust.publishRunStartCheckpoint({
        managedDirectory: this.managedDirectory,
        operationId
      })
    );
    if (descriptor.operationId !== operationId) {
      throw new Error('Rust run-start checkpoint used a different operation ID');
    }
    if (descriptor.boundaryKind !== 'run-start') {
      throw new Error('Rust run-start checkpoint returned a non-run-start boundary');
    }
    const committed = await this.persistence.commit(descriptor);
    if (!managedCheckpointCommitResultMatchesDescriptor(committed, descriptor)) {
      throw new Error('persistence client returned a descriptor different from Rust publication');
    }
    this.rust.acknowledgeRunStartPersistence(committed.descriptor);
    return committed;
  }
}
