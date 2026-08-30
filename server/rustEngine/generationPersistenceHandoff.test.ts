import { describe, expect, it } from 'vitest';
import type { ManagedCheckpointCommitResult } from './checkpointPersistenceClient.ts';
import {
  GenerationPersistenceHandoff,
  type GenerationCheckpointCommitter,
  type RustGenerationCheckpointPublishOptions,
  type RustGenerationPersistencePort
} from './generationPersistenceHandoff.ts';
import type {
  ManagedCheckpointDescriptor,
  ManagedGenerationCommit
} from './checkpointPersistenceProtocol.ts';

/** Promise controls used to hold one fake native publication in flight. */
interface Deferred<T> {
  /** Promise returned by the fake boundary. */
  promise: Promise<T>;
  /** Complete the pending operation successfully. */
  resolve(value: T): void;
  /** Complete the pending operation with a failure. */
  reject(reason: unknown): void;
}

/**
 * Create one externally controlled promise for concurrency assertions.
 * @returns Promise plus exact resolve/reject controls.
 */
function createDeferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

/**
 * Construct one strict generation descriptor without any payload bytes.
 * @param operationId - Exact correlation token for this fixture.
 * @returns Valid generation checkpoint descriptor.
 */
function createDescriptor(operationId = '40404040404040404040404040404040'):
ManagedCheckpointDescriptor {
  const logicalRootSha256 = 'a'.repeat(64);
  return {
    protocolVersion: 1,
    operationId,
    transitionEpoch: '0000000000000001',
    runId: '55555555-6666-4777-8888-999999999999',
    generation: '0000000000000002',
    completedStep: '0000000000000001',
    boundaryKind: 'generation',
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
    graphLayoutSha256: 'b'.repeat(64),
    writeValidationPolicy: 'write-hash-count-fsync-rename-v1'
  };
}

/**
 * Construct the exact compact record that Rust would pair with generation two.
 * @returns Valid bounded generation commit metadata.
 */
function createGenerationCommit(): ManagedGenerationCommit {
  return {
    summary: {
      completedGeneration: '0000000000000001',
      bestF64Hex: '3ff0000000000000',
      averageF64Hex: '3fe0000000000000',
      minimumF64Hex: '0000000000000000',
      speciesCount: '0000000000000001',
      topSpeciesSize: '0000000000000004',
      averageWeightF64Hex: '3fc0000000000000',
      weightVarianceF64Hex: '3f90000000000000'
    },
    hallOfFame: {
      completedGeneration: '0000000000000001',
      sourcePopulationSlot: '0000000000000000',
      sourceSnakeId: '0000000000000001',
      fitnessF64Hex: '3ff0000000000000',
      pointsF64Hex: '4029000000000000',
      length: '0000000000000005',
      successorPopulationSlot: '0000000000000000',
      successorGenomeId: '0000000000000005'
    }
  };
}

/**
 * Construct one complete scalar-only native publication.
 * @param descriptor - Optional exact descriptor override.
 * @returns Publication containing the descriptor and Rust-shaped metadata.
 */
function createPublication(descriptor = createDescriptor()): {
  descriptor: ManagedCheckpointDescriptor;
  generationCommit: ManagedGenerationCommit;
} {
  return { descriptor, generationCommit: createGenerationCommit() };
}

/**
 * Construct a successful persistence result for one exact descriptor.
 * @param descriptor - Descriptor returned by the worker client.
 * @returns Redundant acknowledgement fields plus the complete descriptor.
 */
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

describe('generation persistence handoff', () => {
  it('accepts only an operation token and forwards the direct Rust publication unchanged', async () => {
    const publication = createPublication();
    const publishOptions: RustGenerationCheckpointPublishOptions[] = [];
    const committedValues: unknown[][] = [];
    const acknowledged: ManagedCheckpointDescriptor[] = [];
    const rust: RustGenerationPersistencePort = {
      async publishGenerationCheckpoint(options) {
        publishOptions.push(options);
        return publication;
      },
      acknowledgeGenerationPersistence(descriptor) {
        acknowledged.push(descriptor);
      }
    };
    const persistence: GenerationCheckpointCommitter = {
      async commit(descriptor, generationCommit) {
        committedValues.push([descriptor, generationCommit]);
        return createCommitResult(descriptor as ManagedCheckpointDescriptor);
      }
    };
    const handoff = new GenerationPersistenceHandoff({
      rust,
      persistence,
      managedDirectory: 'C:\\controlled\\checkpoint-v3'
    });

    await expect(handoff.commitPendingGeneration({
      operationId: publication.descriptor.operationId,
      generationCommit: publication.generationCommit
    })).rejects.toThrow(/32-character lowercase hexadecimal token/u);
    expect(publishOptions).toHaveLength(0);

    const result = await handoff.commitPendingGeneration(publication.descriptor.operationId);
    expect(publishOptions).toEqual([{
      managedDirectory: 'C:\\controlled\\checkpoint-v3',
      operationId: publication.descriptor.operationId
    }]);
    expect(committedValues).toEqual([[publication.descriptor, publication.generationCommit]]);
    expect(result).toEqual(createCommitResult(publication.descriptor));
    expect(acknowledged).toEqual([publication.descriptor]);
  });

  it('coalesces the same operation and rejects a different overlapping operation', async () => {
    const publication = createPublication();
    const deferred = createDeferred<unknown>();
    let publications = 0;
    let commits = 0;
    let acknowledgements = 0;
    const handoff = new GenerationPersistenceHandoff({
      rust: {
        publishGenerationCheckpoint() {
          publications += 1;
          return deferred.promise;
        },
        acknowledgeGenerationPersistence() {
          acknowledgements += 1;
        }
      },
      persistence: {
        async commit(descriptor) {
          commits += 1;
          return createCommitResult(descriptor as ManagedCheckpointDescriptor);
        }
      },
      managedDirectory: 'managed'
    });

    const first = handoff.commitPendingGeneration(publication.descriptor.operationId);
    const duplicate = handoff.commitPendingGeneration(publication.descriptor.operationId);
    expect(duplicate).toBe(first);
    await expect(handoff.commitPendingGeneration(
      '50505050505050505050505050505050'
    )).rejects.toThrow(/already in flight/u);
    deferred.resolve(publication);
    await expect(Promise.all([first, duplicate])).resolves.toHaveLength(2);
    expect({ publications, commits, acknowledgements }).toEqual({
      publications: 1,
      commits: 1,
      acknowledgements: 1
    });
  });

  it('does not acknowledge persistence failure and permits an explicit retry', async () => {
    const publication = createPublication();
    let publications = 0;
    let commits = 0;
    let acknowledgements = 0;
    const handoff = new GenerationPersistenceHandoff({
      rust: {
        async publishGenerationCheckpoint() {
          publications += 1;
          return publication;
        },
        acknowledgeGenerationPersistence() {
          acknowledgements += 1;
        }
      },
      persistence: {
        async commit(descriptor) {
          commits += 1;
          if (commits === 1) throw new Error('SQLite FULL transaction failed');
          return createCommitResult(descriptor as ManagedCheckpointDescriptor);
        }
      },
      managedDirectory: 'managed'
    });

    await expect(handoff.commitPendingGeneration(
      publication.descriptor.operationId
    )).rejects.toThrow(/SQLite FULL transaction failed/u);
    expect(acknowledgements).toBe(0);
    await expect(handoff.commitPendingGeneration(
      publication.descriptor.operationId
    )).resolves.toEqual(createCommitResult(publication.descriptor));
    expect({ publications, commits, acknowledgements }).toEqual({
      publications: 2,
      commits: 2,
      acknowledgements: 1
    });
  });

  it('withholds acknowledgement for a mismatched client result', async () => {
    const publication = createPublication();
    let acknowledgements = 0;
    const handoff = new GenerationPersistenceHandoff({
      rust: {
        async publishGenerationCheckpoint() {
          return publication;
        },
        acknowledgeGenerationPersistence() {
          acknowledgements += 1;
        }
      },
      persistence: {
        async commit() {
          const mismatch = {
            ...publication.descriptor,
            completedStep: '0000000000000002'
          };
          return createCommitResult(mismatch);
        }
      },
      managedDirectory: 'managed'
    });

    await expect(handoff.commitPendingGeneration(
      publication.descriptor.operationId
    )).rejects.toThrow(/descriptor different from Rust publication/u);
    expect(acknowledgements).toBe(0);
  });

  it('replays the durable operation when the first Rust acknowledgement throws', async () => {
    const publication = createPublication();
    let commits = 0;
    let acknowledgementAttempts = 0;
    const handoff = new GenerationPersistenceHandoff({
      rust: {
        async publishGenerationCheckpoint() {
          return publication;
        },
        acknowledgeGenerationPersistence() {
          acknowledgementAttempts += 1;
          if (acknowledgementAttempts === 1) throw new Error('Rust acknowledgement interrupted');
        }
      },
      persistence: {
        async commit(descriptor) {
          commits += 1;
          return createCommitResult(descriptor as ManagedCheckpointDescriptor);
        }
      },
      managedDirectory: 'managed'
    });

    await expect(handoff.commitPendingGeneration(
      publication.descriptor.operationId
    )).rejects.toThrow(/Rust acknowledgement interrupted/u);
    await expect(handoff.commitPendingGeneration(
      publication.descriptor.operationId
    )).resolves.toEqual(createCommitResult(publication.descriptor));
    expect({ commits, acknowledgementAttempts }).toEqual({
      commits: 2,
      acknowledgementAttempts: 2
    });
  });
});
