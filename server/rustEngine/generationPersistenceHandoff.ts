/**
 * Thin Node orchestration for one Rust-owned pending generation checkpoint.
 *
 * Callers supply only the correlation operation token. The descriptor,
 * generation summary, Hall-of-Fame identity and successor identity must all
 * come from the retained Rust transition. Node validates and forwards those
 * bounded scalars, commits them through the dedicated persistence worker, and
 * returns only the worker's complete descriptor to Rust.
 */

import {
  managedCheckpointCommitResultMatchesDescriptor,
  type ManagedCheckpointCommitResult
} from './checkpointPersistenceClient.ts';
import {
  parseCheckpointOperationId,
  parseManagedCheckpointDescriptor,
  parseManagedGenerationCommit,
  type CheckpointOperationId,
  type ManagedCheckpointDescriptor,
  type ManagedGenerationCommit
} from './checkpointPersistenceProtocol.ts';

/** Exact options accepted by Rust's managed generation publisher. */
export interface RustGenerationCheckpointPublishOptions {
  /** Server-controlled managed checkpoint directory. */
  managedDirectory: string;
  /** Correlation token retained by the pending Rust transition. */
  operationId: CheckpointOperationId;
}

/** Narrow production port implemented by the Rust-owned engine session. */
export interface RustGenerationPersistencePort {
  /** Publish or exactly retry the already-admitted generation checkpoint. */
  publishGenerationCheckpoint(options: RustGenerationCheckpointPublishOptions): Promise<unknown>;
  /** Retain one successful worker commit only when its complete descriptor matches. */
  acknowledgeGenerationPersistence(descriptor: ManagedCheckpointDescriptor): void;
}

/** Narrow persistence dependency implemented by `CheckpointPersistenceClient`. */
export interface GenerationCheckpointCommitter {
  /** Commit the exact Rust descriptor and compact generation record. */
  commit(
    descriptor: unknown,
    generationCommit: unknown
  ): Promise<ManagedCheckpointCommitResult>;
}

/** Construction dependencies for one generation persistence handoff owner. */
export interface GenerationPersistenceHandoffOptions {
  /** Rust session retaining the actual pending generation transition. */
  rust: RustGenerationPersistencePort;
  /** Dedicated worker client owning the SQLite transaction. */
  persistence: GenerationCheckpointCommitter;
  /** Controlled managed root passed to Rust's file publisher. */
  managedDirectory: string;
}

/** Strict bounded publication obtained directly from the Rust port. */
interface RustGenerationCheckpointPublication {
  /** Immutable managed-file descriptor created by Rust. */
  descriptor: ManagedCheckpointDescriptor;
  /** Exact compact metadata constructed during Rust successor admission. */
  generationCommit: ManagedGenerationCommit;
}

/** One coalesced in-flight operation; a different operation cannot overlap it. */
interface ActiveGenerationPersistence {
  /** Exact operation being published and committed. */
  operationId: CheckpointOperationId;
  /** Identity used to clear only this particular completion. */
  token: object;
  /** Shared result returned to duplicate callers for the same operation. */
  promise: Promise<ManagedCheckpointCommitResult>;
}

/**
 * Validate the scalar-only publication returned by Rust without accepting a
 * caller-supplied generation record.
 * @param value - Direct result of `publishGenerationCheckpoint`.
 * @returns Strict descriptor and exact compact generation record.
 */
function parseRustGenerationCheckpointPublication(
  value: unknown
): RustGenerationCheckpointPublication {
  if (value === null || typeof value !== 'object' || Array.isArray(value) ||
    value instanceof ArrayBuffer || ArrayBuffer.isView(value)) {
    throw new TypeError('Rust generation checkpoint publication must be a plain object');
  }
  const prototype = Object.getPrototypeOf(value);
  if (prototype !== Object.prototype && prototype !== null) {
    throw new TypeError('Rust generation checkpoint publication must be a plain object');
  }
  const publication = value as Record<string, unknown>;
  const keys = Object.keys(publication);
  if (keys.length !== 2 || !Object.hasOwn(publication, 'descriptor') ||
    !Object.hasOwn(publication, 'generationCommit')) {
    throw new TypeError('Rust generation checkpoint publication has unknown or missing fields');
  }
  const descriptor = parseManagedCheckpointDescriptor(publication['descriptor']);
  const generationCommit = parseManagedGenerationCommit(
    publication['generationCommit'],
    descriptor
  );
  if (generationCommit === null) {
    throw new TypeError('Rust generation checkpoint publication omitted generation metadata');
  }
  return { descriptor, generationCommit };
}

/**
 * Single-owner generation persistence barrier between Rust and the SQLite worker.
 *
 * Same-operation concurrent calls share one publication/transaction attempt.
 * A failure clears only that attempt so an explicit retry can reuse Rust's
 * unchanged pending transition and the worker's exact replay behavior.
 */
export class GenerationPersistenceHandoff {
  /** Rust authority port that supplies all persisted generation values. */
  private readonly rust: RustGenerationPersistencePort;
  /** Dedicated SQLite worker client. */
  private readonly persistence: GenerationCheckpointCommitter;
  /** Controlled directory receiving Rust's immutable file. */
  private readonly managedDirectory: string;
  /** At most one currently executing publication/commit/acknowledgement. */
  private active: ActiveGenerationPersistence | null = null;

  /**
   * Bind one Rust authority port to one persistence worker client.
   * @param options - Exact ports and controlled managed directory.
   */
  constructor(options: GenerationPersistenceHandoffOptions) {
    if (typeof options.rust?.publishGenerationCheckpoint !== 'function' ||
      typeof options.rust?.acknowledgeGenerationPersistence !== 'function') {
      throw new TypeError('generation persistence handoff requires a complete Rust port');
    }
    if (typeof options.persistence?.commit !== 'function') {
      throw new TypeError('generation persistence handoff requires a persistence client');
    }
    if (typeof options.managedDirectory !== 'string' || options.managedDirectory.length === 0 ||
      options.managedDirectory.includes('\0')) {
      throw new TypeError('generation persistence managedDirectory must be a nonempty path');
    }
    this.rust = options.rust;
    this.persistence = options.persistence;
    this.managedDirectory = options.managedDirectory;
  }

  /**
   * Publish, commit and acknowledge the exact pending Rust generation.
   * @param operationIdValue - Correlation token only; no generation data is accepted.
   * @returns The persistence worker's exact committed descriptor acknowledgement.
   */
  commitPendingGeneration(operationIdValue: unknown): Promise<ManagedCheckpointCommitResult> {
    let operationId: CheckpointOperationId;
    try {
      operationId = parseCheckpointOperationId(operationIdValue);
    } catch (error) {
      return Promise.reject(error);
    }
    if (this.active) {
      if (this.active.operationId === operationId) return this.active.promise;
      return Promise.reject(new Error(
        `generation persistence operation ${this.active.operationId} is already in flight`
      ));
    }

    const token = {};
    const promise = this.commitOne(operationId).finally(() => {
      if (this.active?.token === token) this.active = null;
    });
    this.active = { operationId, token, promise };
    return promise;
  }

  /**
   * Execute one complete direct Rust-to-worker-to-Rust attempt.
   * @param operationId - Already validated exact operation token.
   * @returns Exact successful worker acknowledgement after Rust retains it.
   */
  private async commitOne(
    operationId: CheckpointOperationId
  ): Promise<ManagedCheckpointCommitResult> {
    const publication = parseRustGenerationCheckpointPublication(
      await this.rust.publishGenerationCheckpoint({
        managedDirectory: this.managedDirectory,
        operationId
      })
    );
    if (publication.descriptor.operationId !== operationId) {
      throw new Error('Rust generation checkpoint used a different operation ID');
    }
    const committed = await this.persistence.commit(
      publication.descriptor,
      publication.generationCommit
    );
    if (!managedCheckpointCommitResultMatchesDescriptor(committed, publication.descriptor)) {
      throw new Error('persistence client returned a descriptor different from Rust publication');
    }
    this.rust.acknowledgeGenerationPersistence(committed.descriptor);
    return committed;
  }
}
