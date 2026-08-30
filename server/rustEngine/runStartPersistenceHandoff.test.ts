import { describe, expect, it } from 'vitest';
import type { ManagedCheckpointCommitResult } from './checkpointPersistenceClient.ts';
import type { ManagedCheckpointDescriptor } from './checkpointPersistenceProtocol.ts';
import {
  RunStartPersistenceHandoff,
  type RunStartCheckpointCommitter,
  type RustRunStartCheckpointPublishOptions,
  type RustRunStartPersistencePort
} from './runStartPersistenceHandoff.ts';

/** Promise controls used to hold one fake native publication in flight. */
interface Deferred<T> {
  /** Promise returned by the fake boundary. */
  promise: Promise<T>;
  /** Complete the pending operation successfully. */
  resolve(value: T): void;
  /** Complete the pending operation with a failure. */
  reject(reason: unknown): void;
}

/** Create one externally controlled promise for concurrency assertions. */
function createDeferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

/** Construct one strict run-start descriptor without payload bytes. */
function createDescriptor(
  operationId = '30303030303030303030303030303030'
): ManagedCheckpointDescriptor {
  const logicalRootSha256 = 'c'.repeat(64);
  return {
    protocolVersion: 1,
    operationId,
    transitionEpoch: '0000000000000007',
    runId: '11111111-2222-4333-8444-555555555555',
    generation: '0000000000000001',
    completedStep: '0000000000000000',
    boundaryKind: 'run-start',
    checkpointFormatVersion: '0000000000000003',
    stateVersion: '0000000000000001',
    graphLayoutVersion: '0000000000000001',
    managedRoot: 'checkpoint-v3',
    relativeFilename: `${logicalRootSha256}.checkpoint-v3`,
    logicalRootSha256,
    storedByteCount: '0000000000001000',
    decodedByteCount: '0000000000002000',
    roleCount: '0000000000000004',
    populationCount: '0000000000000004',
    weightCount: '0000000000000100',
    recurrentStateCount: '0000000000000000',
    weightsEncoding: 'raw-f32le-v1',
    recurrentStateEncoding: 'raw-f32le-v1',
    graphLayoutSha256: 'd'.repeat(64),
    writeValidationPolicy: 'write-hash-count-fsync-rename-v1'
  };
}

/** Construct a successful persistence result for one exact descriptor. */
function createCommitResult(
  descriptor: ManagedCheckpointDescriptor
): ManagedCheckpointCommitResult {
  return {
    operationId: descriptor.operationId,
    transitionEpoch: descriptor.transitionEpoch,
    runId: descriptor.runId,
    checkpointId: descriptor.logicalRootSha256,
    descriptor
  };
}

describe('run-start persistence handoff', () => {
  it('accepts only an operation token and forwards Rust descriptor authority unchanged', async () => {
    const descriptor = createDescriptor();
    const publishOptions: RustRunStartCheckpointPublishOptions[] = [];
    const committedValues: unknown[] = [];
    const acknowledged: ManagedCheckpointDescriptor[] = [];
    const rust: RustRunStartPersistencePort = {
      async publishRunStartCheckpoint(options) {
        publishOptions.push(options);
        return descriptor;
      },
      acknowledgeRunStartPersistence(value) {
        acknowledged.push(value);
      }
    };
    const persistence: RunStartCheckpointCommitter = {
      async commit(value) {
        committedValues.push(value);
        return createCommitResult(descriptor);
      }
    };
    const handoff = new RunStartPersistenceHandoff({
      rust,
      persistence,
      managedDirectory: 'C:\\controlled\\managed'
    });

    await expect(handoff.commitPendingRunStart({
      operationId: descriptor.operationId,
      transitionEpoch: descriptor.transitionEpoch
    })).rejects.toThrow(/operationId/i);
    const result = await handoff.commitPendingRunStart(descriptor.operationId);

    expect(publishOptions).toEqual([{
      managedDirectory: 'C:\\controlled\\managed',
      operationId: descriptor.operationId
    }]);
    expect(committedValues).toEqual([descriptor]);
    expect(acknowledged).toEqual([descriptor]);
    expect(result).toEqual(createCommitResult(descriptor));
  });

  it('coalesces one operation and rejects a different overlapping transition', async () => {
    const descriptor = createDescriptor();
    const deferred = createDeferred<unknown>();
    let publications = 0;
    const handoff = new RunStartPersistenceHandoff({
      rust: {
        publishRunStartCheckpoint() {
          publications += 1;
          return deferred.promise;
        },
        acknowledgeRunStartPersistence() {}
      },
      persistence: {
        async commit() { return createCommitResult(descriptor); }
      },
      managedDirectory: 'managed'
    });

    const first = handoff.commitPendingRunStart(descriptor.operationId);
    const duplicate = handoff.commitPendingRunStart(descriptor.operationId);
    expect(duplicate).toBe(first);
    await expect(handoff.commitPendingRunStart(
      '31313131313131313131313131313131'
    )).rejects.toThrow(/already in flight/i);
    deferred.resolve(descriptor);
    await expect(first).resolves.toEqual(createCommitResult(descriptor));
    expect(publications).toBe(1);
  });

  it('withholds acknowledgement after persistence failure and permits explicit retry', async () => {
    const descriptor = createDescriptor();
    let commits = 0;
    let acknowledgements = 0;
    const handoff = new RunStartPersistenceHandoff({
      rust: {
        async publishRunStartCheckpoint() { return descriptor; },
        acknowledgeRunStartPersistence() { acknowledgements += 1; }
      },
      persistence: {
        async commit() {
          commits += 1;
          if (commits === 1) throw new Error('sqlite unavailable');
          return createCommitResult(descriptor);
        }
      },
      managedDirectory: 'managed'
    });

    await expect(handoff.commitPendingRunStart(descriptor.operationId)).rejects.toThrow(
      /sqlite unavailable/i
    );
    expect(acknowledgements).toBe(0);
    await expect(handoff.commitPendingRunStart(descriptor.operationId)).resolves.toEqual(
      createCommitResult(descriptor)
    );
    expect(commits).toBe(2);
    expect(acknowledgements).toBe(1);
  });

  it('rejects a mismatched persistence result without acknowledging Rust', async () => {
    const descriptor = createDescriptor();
    let acknowledgements = 0;
    const handoff = new RunStartPersistenceHandoff({
      rust: {
        async publishRunStartCheckpoint() { return descriptor; },
        acknowledgeRunStartPersistence() { acknowledgements += 1; }
      },
      persistence: {
        async commit() {
          return {
            ...createCommitResult(descriptor),
            checkpointId: 'e'.repeat(64)
          };
        }
      },
      managedDirectory: 'managed'
    });

    await expect(handoff.commitPendingRunStart(descriptor.operationId)).rejects.toThrow(
      /different from Rust publication/i
    );
    expect(acknowledgements).toBe(0);
  });

  it('exact-replays a durable commit when the first Rust acknowledgement throws', async () => {
    const descriptor = createDescriptor();
    let publications = 0;
    let commits = 0;
    let acknowledgements = 0;
    const handoff = new RunStartPersistenceHandoff({
      rust: {
        async publishRunStartCheckpoint() {
          publications += 1;
          return descriptor;
        },
        acknowledgeRunStartPersistence() {
          acknowledgements += 1;
          if (acknowledgements === 1) throw new Error('ack transport lost');
        }
      },
      persistence: {
        async commit() {
          commits += 1;
          return createCommitResult(descriptor);
        }
      },
      managedDirectory: 'managed'
    });

    await expect(handoff.commitPendingRunStart(descriptor.operationId)).rejects.toThrow(
      /ack transport lost/i
    );
    await expect(handoff.commitPendingRunStart(descriptor.operationId)).resolves.toEqual(
      createCommitResult(descriptor)
    );
    expect(publications).toBe(2);
    expect(commits).toBe(2);
    expect(acknowledgements).toBe(2);
  });
});
