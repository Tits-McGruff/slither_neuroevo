import { createHash } from 'node:crypto';
import { mkdirSync, mkdtempSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import Database from 'better-sqlite3';
import { afterEach, describe, expect, it, type TestContext } from 'vitest';
import { CheckpointPersistenceClient } from './checkpointPersistenceClient.ts';
import type {
  ManagedCheckpointDescriptor,
  ManagedGenerationCommit,
  ManagedHallOfFameReference,
  ManagedGenerationSummary
} from './checkpointPersistenceProtocol.ts';

/**
 * Test-suite label used in runner output.
 *
 * These fixtures cover descriptor validation and metadata transaction mechanics only. Real
 * checkpoint-file compatibility and corruption are owned by the integrated Rust-to-Node path,
 * where Rust validates the logical root during restore/startup.
 */
const SUITE = 'minimal checkpoint metadata and compact-history persistence worker';
/** Disposable fixture roots removed after their clients stop. */
const fixtureRoots: string[] = [];
/** Active clients closed before their temporary roots are removed. */
const clients: CheckpointPersistenceClient[] = [];

/**
 * Format one small unsigned integer as the canonical descriptor u64 wire form.
 * @param value - Nonnegative value represented exactly in a test fixture.
 * @returns Fixed-width lowercase hexadecimal value.
 */
function u64(value: bigint): string {
  return value.toString(16).padStart(16, '0');
}

/**
 * Encode one JavaScript fixture number as its exact big-endian IEEE-754 Float64 bits.
 * @param value - Finite fixture value.
 * @returns Fixed-width lowercase hexadecimal bits matching Rust `f64::to_bits()`.
 */
function f64(value: number): string {
  const bytes = Buffer.allocUnsafe(8);
  bytes.writeDoubleBE(value);
  return bytes.toString('hex');
}

/**
 * Build one exact compact eight-field generation result.
 * @param completedGeneration - Generation whose round completed.
 * @param overrides - Optional field changes for rejection/idempotency tests.
 * @returns Strict small generation-history record.
 */
function createGenerationSummary(
  completedGeneration: bigint,
  overrides: Partial<ManagedGenerationSummary> = {}
): ManagedGenerationSummary {
  return {
    completedGeneration: u64(completedGeneration),
    bestF64Hex: f64(12.5),
    averageF64Hex: f64(7.25),
    minimumF64Hex: f64(-1.5),
    speciesCount: u64(2n),
    topSpeciesSize: u64(1n),
    averageWeightF64Hex: f64(0.125),
    weightVarianceF64Hex: f64(0.03125),
    ...overrides
  };
}

/**
 * Build one compact reference to the elite already stored in a generation checkpoint.
 * @param completedGeneration - Generation whose elite was selected.
 * @param fitnessF64Hex - Exact best-fitness bits shared with compact history.
 * @param overrides - Optional field changes for validation and replay tests.
 * @returns Strict scalar Hall-of-Fame reference without duplicated genome weights.
 */
function createHallOfFameReference(
  completedGeneration: bigint,
  fitnessF64Hex: string,
  overrides: Partial<ManagedHallOfFameReference> = {}
): ManagedHallOfFameReference {
  return {
    completedGeneration: u64(completedGeneration),
    sourcePopulationSlot: u64(1n),
    sourceSnakeId: u64(17n),
    fitnessF64Hex,
    pointsF64Hex: f64(6.25),
    length: u64(9n),
    successorPopulationSlot: u64(0n),
    successorGenomeId: u64(1_001n),
    ...overrides
  };
}

/**
 * Build the complete small metadata transaction payload for one finished generation.
 * @param completedGeneration - Generation whose round completed.
 * @param summaryOverrides - Optional compact-history changes.
 * @param hallOfFameOverrides - Optional Hall-of-Fame reference changes.
 * @returns Atomic history/reference commit fixture.
 */
function createGenerationCommit(
  completedGeneration: bigint,
  summaryOverrides: Partial<ManagedGenerationSummary> = {},
  hallOfFameOverrides: Partial<ManagedHallOfFameReference> = {}
): ManagedGenerationCommit {
  const summary = createGenerationSummary(completedGeneration, summaryOverrides);
  return {
    summary,
    hallOfFame: createHallOfFameReference(
      completedGeneration,
      summary.bestF64Hex,
      hallOfFameOverrides
    )
  };
}

/**
 * Decode one stored 56-byte compact record back to its exact protocol fields.
 * @param record - SQLite BLOB returned by the disposable test database.
 * @returns Exact generation-summary wire values.
 */
function decodeGenerationSummary(record: Buffer): ManagedGenerationSummary {
  if (record.length !== 56) throw new RangeError('history fixture record must contain 56 bytes');
  const hex = (offset: number): string => record.readBigUInt64LE(offset).toString(16).padStart(16, '0');
  return {
    completedGeneration: hex(0),
    bestF64Hex: hex(8),
    averageF64Hex: hex(16),
    minimumF64Hex: hex(24),
    speciesCount: u64(BigInt(record.readUInt32LE(32))),
    topSpeciesSize: u64(BigInt(record.readUInt32LE(36))),
    averageWeightF64Hex: hex(40),
    weightVarianceF64Hex: hex(48)
  };
}

/**
 * Decode one stored 56-byte Hall-of-Fame reference without numeric coercion.
 * @param record - SQLite BLOB returned by the disposable test database.
 * @returns Exact Hall-of-Fame wire values.
 */
function decodeHallOfFameReference(record: Buffer): ManagedHallOfFameReference {
  if (record.length !== 56) throw new RangeError('Hall-of-Fame fixture record must contain 56 bytes');
  const hex = (offset: number): string => record.readBigUInt64LE(offset).toString(16).padStart(16, '0');
  return {
    completedGeneration: hex(0),
    sourcePopulationSlot: u64(BigInt(record.readUInt32LE(8))),
    successorPopulationSlot: u64(BigInt(record.readUInt32LE(12))),
    sourceSnakeId: hex(16),
    successorGenomeId: hex(24),
    fitnessF64Hex: hex(32),
    pointsF64Hex: hex(40),
    length: hex(48)
  };
}

/**
 * Build one isolated root, managed directory, and client-owned worker.
 * @returns Paths and client for a disposable real SQLite checkpoint publication test.
 */
function createFixture(
  workerUrlForTesting?: URL,
  workerResponseModeForTesting?: 'invalid' | 'mismatched' | 'exit' | 'exit-clean'
): {
  root: string;
  managedRoot: string;
  databasePath: string;
  client: CheckpointPersistenceClient;
} {
  const root = mkdtempSync(join(tmpdir(), 'slither-checkpoint-worker-'));
  fixtureRoots.push(root);
  const managedRoot = join(root, 'managed-checkpoints');
  mkdirSync(managedRoot);
  const databasePath = join(root, 'metadata.sqlite');
  const client = new CheckpointPersistenceClient({
    databasePath,
    managedRootPath: managedRoot,
    ...(workerUrlForTesting ? { workerUrlForTesting } : {}),
    ...(workerResponseModeForTesting ? { workerResponseModeForTesting } : {})
  });
  clients.push(client);
  return { root, managedRoot, databasePath, client };
}

/**
 * Create one final digest-derived file and matching scalar descriptor.
 * @param managedRoot - Controlled directory receiving the final immutable file.
 * @param options - Optional exact descriptor overrides.
 * @returns Strict descriptor ready for client submission.
 */
function createDescriptor(
  managedRoot: string,
  options: Partial<ManagedCheckpointDescriptor> = {}
): ManagedCheckpointDescriptor {
  const bytes = Buffer.from(options.operationId ?? 'checkpoint-v3-fixture', 'utf8');
  const logicalRootSha256 = options.logicalRootSha256 ?? createHash('sha256').update(bytes).digest('hex');
  const relativeFilename = options.relativeFilename ?? `${logicalRootSha256}.checkpoint-v3`;
  if (!options.relativeFilename) writeFileSync(join(managedRoot, relativeFilename), bytes);
  return {
    protocolVersion: 1,
    operationId: '0123456789abcdef0123456789abcdef',
    transitionEpoch: u64(1n),
    runId: '8c50f3b8-b27a-4d88-b0bc-7a8557c0e456',
    generation: u64(1n),
    completedStep: u64(0n),
    boundaryKind: 'run-start',
    checkpointFormatVersion: u64(3n),
    stateVersion: u64(1n),
    graphLayoutVersion: u64(1n),
    managedRoot: 'checkpoint-v3',
    relativeFilename,
    logicalRootSha256,
    storedByteCount: u64(BigInt(bytes.length)),
    decodedByteCount: u64(BigInt(bytes.length)),
    roleCount: u64(3n),
    populationCount: u64(2n),
    weightCount: u64(8n),
    recurrentStateCount: u64(0n),
    weightsEncoding: 'raw-f32le-v1',
    recurrentStateEncoding: 'raw-f32le-v1',
    graphLayoutSha256: createHash('sha256').update('graph-layout').digest('hex'),
    writeValidationPolicy: 'write-hash-count-fsync-rename-v1',
    ...options
  };
}

/**
 * Read the current pointer after closing the worker that exclusively writes it.
 * @param databasePath - Disposable SQLite database path.
 * @param runId - Run whose pointer is inspected.
 * @returns Current checkpoint/root tuple, or undefined before first commit.
 */
function readCurrentPointer(databasePath: string, runId: string): {
  checkpoint_id: string;
  transition_epoch: string;
  operation_id: string;
} | undefined {
  const db = new Database(databasePath, { readonly: true });
  try {
    return db.prepare(
      'SELECT checkpoint_id, transition_epoch, operation_id FROM rust_checkpoint_v3_current WHERE run_id = ?'
    ).get(runId) as { checkpoint_id: string; transition_epoch: string; operation_id: string } | undefined;
  } finally {
    db.close();
  }
}

/**
 * Create a symlink or skip only when the current platform disallows this test fixture.
 * @param context - Current Vitest test context.
 * @param target - Existing target file.
 * @param linkPath - New link path beneath the controlled root.
 * @returns True when the symlink exists and can be tested.
 */
function createSymlinkOrSkip(context: TestContext, target: string, linkPath: string): boolean {
  try {
    symlinkSync(target, linkPath, 'file');
    return true;
  } catch (error) {
    const code = (error as NodeJS.ErrnoException).code;
    if (code === 'EPERM' || code === 'EACCES' || code === 'ENOSYS') {
      context.skip();
    }
    throw error;
  }
}

afterEach(async () => {
  for (const client of clients.splice(0)) await client.close().catch(() => {});
  for (const root of fixtureRoots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe(SUITE, () => {
  it('uses boundary identity rather than resettable Rust operation epochs to advance current', async () => {
    const fixture = createFixture();
    const first = createDescriptor(fixture.managedRoot);
    const committed = await fixture.client.commit(first);
    expect(committed).toEqual({
      operationId: first.operationId,
      transitionEpoch: first.transitionEpoch,
      runId: first.runId,
      checkpointId: first.logicalRootSha256,
      descriptor: first
    });

    const second = createDescriptor(fixture.managedRoot, {
      operationId: 'fedcba9876543210fedcba9876543210',
      transitionEpoch: u64(3_600n),
      generation: u64(2n),
      completedStep: u64(3_600n),
      boundaryKind: 'generation'
    });
    await fixture.client.commit(second, createGenerationCommit(1n));
    const third = createDescriptor(fixture.managedRoot, {
      operationId: '11111111111111111111111111111111',
      transitionEpoch: u64(3_600n),
      generation: u64(3n),
      completedStep: u64(7_200n),
      boundaryKind: 'generation'
    });
    await fixture.client.commit(third, createGenerationCommit(2n));
    await fixture.client.close();

    expect(readCurrentPointer(fixture.databasePath, first.runId)).toEqual({
      checkpoint_id: third.logicalRootSha256,
      transition_epoch: third.transitionEpoch,
      operation_id: third.operationId
    });
    const db = new Database(fixture.databasePath, { readonly: true });
    try {
      const rows = db.prepare(`
        SELECT generation_hex, checkpoint_id, record_version, record_blob
        FROM rust_generation_history_v1 ORDER BY generation_hex
      `).all() as Array<{
        generation_hex: string;
        checkpoint_id: string;
        record_version: number;
        record_blob: Buffer;
      }>;
      expect(rows.map(row => ({
        generation_hex: row.generation_hex,
        checkpoint_id: row.checkpoint_id,
        record_version: row.record_version,
        summary: decodeGenerationSummary(row.record_blob)
      }))).toEqual([
        {
          generation_hex: u64(1n),
          checkpoint_id: second.logicalRootSha256,
          record_version: 1,
          summary: createGenerationSummary(1n)
        },
        {
          generation_hex: u64(2n),
          checkpoint_id: third.logicalRootSha256,
          record_version: 1,
          summary: createGenerationSummary(2n)
        }
      ]);
      const hallOfFameRows = db.prepare(`
        SELECT generation_hex, checkpoint_id, record_version, record_blob
        FROM rust_hall_of_fame_v1 ORDER BY generation_hex
      `).all() as Array<{
        generation_hex: string;
        checkpoint_id: string;
        record_version: number;
        record_blob: Buffer;
      }>;
      expect(hallOfFameRows.map(row => ({
        generation_hex: row.generation_hex,
        checkpoint_id: row.checkpoint_id,
        record_version: row.record_version,
        reference: decodeHallOfFameReference(row.record_blob)
      }))).toEqual([
        {
          generation_hex: u64(1n),
          checkpoint_id: second.logicalRootSha256,
          record_version: 1,
          reference: createGenerationCommit(1n).hallOfFame
        },
        {
          generation_hex: u64(2n),
          checkpoint_id: third.logicalRootSha256,
          record_version: 1,
          reference: createGenerationCommit(2n).hallOfFame
        }
      ]);
    } finally {
      db.close();
    }
  });

  it('commits a zero-weight descriptor for a valid parameterless population graph', async () => {
    const fixture = createFixture();
    const descriptor = createDescriptor(fixture.managedRoot, {
      weightCount: u64(0n)
    });
    const committed = await fixture.client.commit(descriptor);
    expect(committed.checkpointId).toBe(descriptor.logicalRootSha256);
    await fixture.client.close();

    const db = new Database(fixture.databasePath, { readonly: true });
    try {
      expect(db.prepare(
        'SELECT weight_count_hex FROM rust_checkpoint_v3_metadata WHERE checkpoint_id = ?'
      ).get(descriptor.logicalRootSha256)).toEqual({ weight_count_hex: u64(0n) });
    } finally {
      db.close();
    }
    expect(readCurrentPointer(fixture.databasePath, descriptor.runId)?.checkpoint_id).toBe(
      descriptor.logicalRootSha256
    );
  });

  it('is idempotent for an exact replay without duplicating metadata or regressing the pointer', async () => {
    const fixture = createFixture();
    const descriptor = createDescriptor(fixture.managedRoot);
    await fixture.client.commit(descriptor);
    await fixture.client.commit({ ...descriptor });
    await fixture.client.close();

    const db = new Database(fixture.databasePath, { readonly: true });
    try {
      expect(db.prepare('SELECT COUNT(*) AS count FROM rust_checkpoint_v3_metadata').get()).toEqual({ count: 1 });
    } finally {
      db.close();
    }
    expect(readCurrentPointer(fixture.databasePath, descriptor.runId)?.checkpoint_id).toBe(
      descriptor.logicalRootSha256
    );
  });

  it('rejects a replay that was superseded by a later current pointer', async () => {
    const fixture = createFixture();
    const first = createDescriptor(fixture.managedRoot);
    const second = createDescriptor(fixture.managedRoot, {
      operationId: 'fedcba9876543210fedcba9876543210',
      transitionEpoch: u64(3_600n),
      generation: u64(2n),
      completedStep: u64(3_600n),
      boundaryKind: 'generation'
    });
    await fixture.client.commit(first);
    await fixture.client.commit(second, createGenerationCommit(1n));
    await expect(fixture.client.commit(first)).rejects.toThrow(/superseded/);
  });

  it('rejects immutable conflicts and stale or skipped generations without changing current', async () => {
    const fixture = createFixture();
    const first = createDescriptor(fixture.managedRoot);
    await fixture.client.commit(first);
    const sameOperationDifferentDescriptor = createDescriptor(fixture.managedRoot, {
      operationId: first.operationId,
      stateVersion: u64(2n)
    });
    await expect(fixture.client.commit(sameOperationDifferentDescriptor)).rejects.toThrow(/operationId conflicts/);
    const sameRootDifferentOperation = createDescriptor(fixture.managedRoot, {
      operationId: '44444444444444444444444444444444',
      logicalRootSha256: first.logicalRootSha256,
      relativeFilename: first.relativeFilename,
      storedByteCount: first.storedByteCount,
      decodedByteCount: first.decodedByteCount
    });
    await expect(fixture.client.commit(sameRootDifferentOperation)).rejects.toThrow(/logical checkpoint root/);

    const second = createDescriptor(fixture.managedRoot, {
      operationId: '55555555555555555555555555555555',
      transitionEpoch: u64(3_600n),
      generation: u64(2n),
      completedStep: u64(3_600n),
      boundaryKind: 'generation'
    });
    await fixture.client.commit(second, createGenerationCommit(1n));
    const stale = createDescriptor(fixture.managedRoot, {
      operationId: '66666666666666666666666666666666',
      transitionEpoch: u64(1n),
      generation: u64(2n),
      completedStep: u64(3_000n),
      boundaryKind: 'generation'
    });
    await expect(fixture.client.commit(stale, createGenerationCommit(1n))).rejects.toThrow(/stale/);
    const gapped = createDescriptor(fixture.managedRoot, {
      operationId: '77777777777777777777777777777777',
      transitionEpoch: u64(4n),
      generation: u64(4n),
      completedStep: u64(10_800n),
      boundaryKind: 'generation'
    });
    await expect(fixture.client.commit(gapped, createGenerationCommit(3n))).rejects.toThrow(/exactly one/);
    await fixture.client.close();
    expect(readCurrentPointer(fixture.databasePath, first.runId)).toEqual({
      checkpoint_id: second.logicalRootSha256,
      transition_epoch: second.transitionEpoch,
      operation_id: second.operationId
    });
  });

  it('rejects and preserves a current pointer whose operation identity no longer matches metadata', async () => {
    const fixture = createFixture();
    const first = createDescriptor(fixture.managedRoot);
    await fixture.client.commit(first);
    const forgedOperationId = 'ffffffffffffffffffffffffffffffff';
    const corrupt = new Database(fixture.databasePath);
    try {
      corrupt.prepare(`
        UPDATE rust_checkpoint_v3_current
        SET operation_id = ?
        WHERE run_id = ?
      `).run(forgedOperationId, first.runId);
    } finally {
      corrupt.close();
    }
    const second = createDescriptor(fixture.managedRoot, {
      operationId: '12121212121212121212121212121212',
      transitionEpoch: u64(3_600n),
      generation: u64(2n),
      completedStep: u64(3_600n),
      boundaryKind: 'generation'
    });
    await expect(fixture.client.commit(second, createGenerationCommit(1n))).rejects.toThrow(
      /pointer identity does not match immutable metadata/
    );
    await fixture.client.close();
    expect(readCurrentPointer(fixture.databasePath, first.runId)).toEqual({
      checkpoint_id: first.logicalRootSha256,
      transition_epoch: first.transitionEpoch,
      operation_id: forgedOperationId
    });
    const check = new Database(fixture.databasePath, { readonly: true });
    try {
      expect(check.prepare(
        'SELECT COUNT(*) AS count FROM rust_checkpoint_v3_metadata WHERE checkpoint_id = ?'
      ).get(second.logicalRootSha256)).toEqual({ count: 0 });
      expect(check.prepare('SELECT COUNT(*) AS count FROM rust_generation_history_v1').get()).toEqual({ count: 0 });
      expect(check.prepare('SELECT COUNT(*) AS count FROM rust_hall_of_fame_v1').get()).toEqual({ count: 0 });
    } finally {
      check.close();
    }
  });

  it('terminates a worker after an invalid protocol response and makes close wait for that exit', async () => {
    const fixture = createFixture(
      new URL('./checkpointPersistenceInvalidResponseWorker.ts', import.meta.url)
    );
    const descriptor = createDescriptor(fixture.managedRoot);
    await expect(fixture.client.commit(descriptor)).rejects.toThrow(/unknown response type/);
    await expect(fixture.client.close()).rejects.toThrow(/unknown response type/);
    expect(fixture.client.terminated).toBe(true);
  });

  it('terminates a worker after a correlated but mismatched acknowledgement', async () => {
    const fixture = createFixture(
      new URL('./checkpointPersistenceInvalidResponseWorker.ts', import.meta.url),
      'mismatched'
    );
    const descriptor = createDescriptor(fixture.managedRoot);
    await expect(fixture.client.commit(descriptor)).rejects.toThrow(/acknowledgement mismatched/);
    await expect(fixture.client.close()).rejects.toThrow(/acknowledgement mismatched/);
    expect(fixture.client.terminated).toBe(true);
  });

  it('rejects pending work and close after an unexpected worker exit', async () => {
    const fixture = createFixture(
      new URL('./checkpointPersistenceInvalidResponseWorker.ts', import.meta.url),
      'exit'
    );
    const descriptor = createDescriptor(fixture.managedRoot);
    await expect(fixture.client.commit(descriptor)).rejects.toThrow(/exited with code 3/);
    await expect(fixture.client.close()).rejects.toThrow(/exited with code 3/);
    expect(fixture.client.terminated).toBe(true);
  });

  it('treats exit zero during close as failure while a commit still awaits its reply', async () => {
    const fixture = createFixture(
      new URL('./checkpointPersistenceInvalidResponseWorker.ts', import.meta.url),
      'exit-clean'
    );
    const descriptor = createDescriptor(fixture.managedRoot);
    const commit = fixture.client.commit(descriptor);
    const close = fixture.client.close();
    await expect(commit).rejects.toThrow(/exited cleanly with 1 pending operation/);
    await expect(close).rejects.toThrow(/exited cleanly with 1 pending operation/);
    expect(fixture.client.terminated).toBe(true);
  });

  it('accepts any positive first operation epoch because it is an acknowledgement token', async () => {
    const fixture = createFixture();
    const descriptor = createDescriptor(fixture.managedRoot, { transitionEpoch: u64(2_741n) });
    await expect(fixture.client.commit(descriptor)).resolves.toMatchObject({
      transitionEpoch: u64(2_741n)
    });
  });

  it('requires exact finite compact history only for generation checkpoints', async () => {
    const fixture = createFixture();
    const runStart = createDescriptor(fixture.managedRoot);
    await expect(fixture.client.commit(runStart, createGenerationCommit(1n))).rejects.toThrow(
      /run-start checkpoints must not include/
    );
    await fixture.client.commit(runStart);
    const generation = createDescriptor(fixture.managedRoot, {
      operationId: '99999999999999999999999999999999',
      transitionEpoch: u64(3_600n),
      generation: u64(2n),
      completedStep: u64(3_600n),
      boundaryKind: 'generation'
    });
    await expect(fixture.client.commit(generation)).rejects.toThrow(/generationCommit must be a plain object/);
    await expect(fixture.client.commit(generation, createGenerationCommit(2n))).rejects.toThrow(
      /exactly the generation preceding/
    );
    await expect(fixture.client.commit(generation, createGenerationCommit(1n, {
      bestF64Hex: '7ff0000000000000'
    }))).rejects.toThrow(/finite Float64/);
    await expect(fixture.client.commit(generation, createGenerationCommit(1n, {
      speciesCount: u64(3n)
    }))).rejects.toThrow(/speciesCount exceeds/);
    await expect(fixture.client.commit(generation, createGenerationCommit(1n, {}, {
      completedGeneration: u64(2n)
    }))).rejects.toThrow(/Hall-of-Fame generation/);
    await expect(fixture.client.commit(generation, createGenerationCommit(1n, {}, {
      fitnessF64Hex: f64(11.5)
    }))).rejects.toThrow(/fitness does not match/);
    await expect(fixture.client.commit(generation, createGenerationCommit(1n, {}, {
      sourcePopulationSlot: u64(2n)
    }))).rejects.toThrow(/outside the checkpoint population/);
    await expect(fixture.client.commit(generation, createGenerationCommit(1n, {}, {
      successorGenomeId: u64(0n)
    }))).rejects.toThrow(/identities must be nonzero/);
  });

  it('rejects zero-step generation boundaries and first pointers without branch provenance', async () => {
    const fixture = createFixture();
    const runStart = createDescriptor(fixture.managedRoot);
    await fixture.client.commit(runStart);
    const zeroStep = createDescriptor(fixture.managedRoot, {
      operationId: 'abababababababababababababababab',
      boundaryKind: 'generation',
      generation: u64(2n),
      completedStep: u64(0n),
      transitionEpoch: u64(1n)
    });
    await expect(fixture.client.commit(zeroStep, createGenerationCommit(1n))).rejects.toThrow(
      /completedStep must be nonzero/
    );
    const branchWithoutProvenance = createDescriptor(fixture.managedRoot, {
      operationId: 'cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd',
      runId: 'branch-without-provenance',
      boundaryKind: 'generation',
      generation: u64(8n),
      completedStep: u64(25_200n),
      transitionEpoch: u64(1n)
    });
    await expect(fixture.client.commit(
      branchWithoutProvenance,
      createGenerationCommit(7n)
    )).rejects.toThrow(/explicit branch provenance/);
  });

  it('replays all generation metadata exactly and rejects changed records', async () => {
    const fixture = createFixture();
    const runStart = createDescriptor(fixture.managedRoot);
    await fixture.client.commit(runStart);
    const descriptor = createDescriptor(fixture.managedRoot, {
      operationId: 'edededededededededededededededed',
      boundaryKind: 'generation',
      generation: u64(2n),
      completedStep: u64(3_600n),
      transitionEpoch: u64(3_600n)
    });
    const generationCommit = createGenerationCommit(1n);
    await fixture.client.commit(descriptor, generationCommit);
    await fixture.client.commit({ ...descriptor }, {
      summary: { ...generationCommit.summary },
      hallOfFame: { ...generationCommit.hallOfFame }
    });
    await expect(fixture.client.commit(
      descriptor,
      createGenerationCommit(1n, { averageF64Hex: f64(7.5) })
    )).rejects.toThrow(/different compact generation history/);
    await expect(fixture.client.commit(
      descriptor,
      createGenerationCommit(1n, {}, { pointsF64Hex: f64(6.5) })
    )).rejects.toThrow(/different Hall-of-Fame reference/);
    const corrupt = new Database(fixture.databasePath);
    try {
      corrupt.prepare(
        'UPDATE rust_generation_history_v1 SET record_version = 2 WHERE checkpoint_id = ?'
      ).run(descriptor.logicalRootSha256);
    } finally {
      corrupt.close();
    }
    await expect(fixture.client.commit(descriptor, generationCommit)).rejects.toThrow(
      /different compact generation history/
    );
    await fixture.client.close();
    const db = new Database(fixture.databasePath, { readonly: true });
    try {
      expect(db.prepare('SELECT COUNT(*) AS count FROM rust_checkpoint_v3_metadata').get()).toEqual({ count: 2 });
      expect(db.prepare('SELECT COUNT(*) AS count FROM rust_generation_history_v1').get()).toEqual({ count: 1 });
      expect(db.prepare('SELECT COUNT(*) AS count FROM rust_hall_of_fame_v1').get()).toEqual({ count: 1 });
    } finally {
      db.close();
    }
  });

  it('rolls back history, metadata, and pointer when the Hall-of-Fame reference cannot insert', async () => {
    const fixture = createFixture();
    const first = createDescriptor(fixture.managedRoot);
    await fixture.client.commit(first);
    const forged = createGenerationCommit(1n).hallOfFame;
    const db = new Database(fixture.databasePath);
    try {
      db.pragma('foreign_keys = ON');
      db.prepare(`
        INSERT INTO rust_hall_of_fame_v1 (
          run_id, generation_hex, checkpoint_id, record_version, record_blob, created_at_ms
        ) VALUES (?, ?, ?, ?, ?, ?)
      `).run(
        first.runId, forged.completedGeneration, first.logicalRootSha256,
        1, Buffer.alloc(56), Date.now()
      );
    } finally {
      db.close();
    }
    const second = createDescriptor(fixture.managedRoot, {
      operationId: 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
      transitionEpoch: u64(3_600n),
      generation: u64(2n),
      completedStep: u64(3_600n),
      boundaryKind: 'generation'
    });
    await expect(fixture.client.commit(second, createGenerationCommit(1n))).rejects.toThrow(
      /UNIQUE constraint failed/
    );
    await fixture.client.close();
    const check = new Database(fixture.databasePath, { readonly: true });
    try {
      expect(check.prepare(
        'SELECT COUNT(*) AS count FROM rust_checkpoint_v3_metadata WHERE checkpoint_id = ?'
      ).get(second.logicalRootSha256)).toEqual({ count: 0 });
      expect(check.prepare(
        'SELECT COUNT(*) AS count FROM rust_generation_history_v1 WHERE checkpoint_id = ?'
      ).get(second.logicalRootSha256)).toEqual({ count: 0 });
      expect(readCurrentPointer(fixture.databasePath, first.runId)?.checkpoint_id).toBe(
        first.logicalRootSha256
      );
    } finally {
      check.close();
    }
  });

  it('rejects traversal, binary/population fields, invalid digest, and out-of-range count before publication', async () => {
    const fixture = createFixture();
    const descriptor = createDescriptor(fixture.managedRoot);
    await expect(fixture.client.commit({
      ...descriptor,
      relativeFilename: '../escape.checkpoint-v3'
    })).rejects.toThrow(/digest-derived/);
    await expect(fixture.client.commit({
      ...descriptor,
      logicalRootSha256: 'A'.repeat(64)
    })).rejects.toThrow(/SHA-256/);
    const populationPayload = new Uint8Array(2 * 1024 * 1024);
    await expect(fixture.client.commit({ ...descriptor, population: populationPayload })).rejects.toThrow(
      /prohibited payload field/
    );
    await expect(fixture.client.commit({
      ...descriptor,
      populationCount: 'ffffffffffffffff'
    })).rejects.toThrow(/populationCount/);
  });

  it('rejects distinct lone-surrogate run IDs before UTF-8 encoding or SQLite key insertion', async () => {
    const fixture = createFixture();
    const descriptor = createDescriptor(fixture.managedRoot);
    await expect(fixture.client.commit({
      ...descriptor,
      runId: `run-${String.fromCharCode(0xd800)}`
    })).rejects.toThrow(/well-formed UTF-16/);
    await expect(fixture.client.commit({
      ...descriptor,
      operationId: '88888888888888888888888888888888',
      runId: `run-${String.fromCharCode(0xd801)}`
    })).rejects.toThrow(/well-formed UTF-16/);
    await fixture.client.close();
    const db = new Database(fixture.databasePath, { readonly: true });
    try {
      expect(db.prepare('SELECT COUNT(*) AS count FROM rust_checkpoint_v3_metadata').get()).toEqual({ count: 0 });
    } finally {
      db.close();
    }
  });

  it('rejects a symlinked final file without changing the existing pointer', async context => {
    const fixture = createFixture();
    const first = createDescriptor(fixture.managedRoot);
    await fixture.client.commit(first);
    const second = createDescriptor(fixture.managedRoot, {
      operationId: '11111111111111111111111111111111',
      transitionEpoch: u64(2n)
    });
    const actualPath = join(fixture.managedRoot, second.relativeFilename);
    rmSync(actualPath);
    if (!createSymlinkOrSkip(context, join(fixture.managedRoot, first.relativeFilename), actualPath)) return;

    await expect(fixture.client.commit(second)).rejects.toThrow(/never a symlink/);
    await fixture.client.close();
    expect(readCurrentPointer(fixture.databasePath, first.runId)?.checkpoint_id).toBe(first.logicalRootSha256);
  });

  it('preserves the old pointer when a final file is missing or its length disagrees with the descriptor', async () => {
    const fixture = createFixture();
    const first = createDescriptor(fixture.managedRoot);
    await fixture.client.commit(first);
    const missingRoot = createHash('sha256').update('missing').digest('hex');
    const missing = createDescriptor(fixture.managedRoot, {
      operationId: '22222222222222222222222222222222',
      transitionEpoch: u64(2n),
      logicalRootSha256: missingRoot,
      relativeFilename: `${missingRoot}.checkpoint-v3`
    });
    await expect(fixture.client.commit(missing)).rejects.toThrow(/ENOENT|no such file/i);

    const mismatch = createDescriptor(fixture.managedRoot, {
      operationId: '33333333333333333333333333333333',
      transitionEpoch: u64(2n),
      storedByteCount: u64(1n)
    });
    await expect(fixture.client.commit(mismatch)).rejects.toThrow(/size does not match/);
    await fixture.client.close();
    expect(readCurrentPointer(fixture.databasePath, first.runId)).toMatchObject({
      checkpoint_id: first.logicalRootSha256,
      transition_epoch: first.transitionEpoch
    });
  });
});
