import { createHash } from 'node:crypto';
import { mkdirSync, mkdtempSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import Database from 'better-sqlite3';
import { afterEach, describe, expect, it, type TestContext } from 'vitest';
import { CheckpointPersistenceClient } from './checkpointPersistenceClient.ts';
import type { ManagedCheckpointDescriptor } from './checkpointPersistenceProtocol.ts';

/**
 * Test-suite label used in runner output.
 *
 * These fixtures cover descriptor validation and metadata transaction mechanics only. Real
 * checkpoint-file compatibility and corruption are owned by the integrated Rust-to-Node path,
 * where Rust validates the logical root during restore/startup.
 */
const SUITE = 'Stage 3 descriptor-only checkpoint persistence worker';
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
  it('commits descriptor-only metadata and advances one per-run current pointer', async () => {
    const fixture = createFixture();
    const first = createDescriptor(fixture.managedRoot);
    const committed = await fixture.client.commit(first);
    expect(committed).toEqual({
      operationId: first.operationId,
      transitionEpoch: first.transitionEpoch,
      runId: first.runId,
      checkpointId: first.logicalRootSha256
    });

    const second = createDescriptor(fixture.managedRoot, {
      operationId: 'fedcba9876543210fedcba9876543210',
      transitionEpoch: u64(2n),
      generation: u64(2n),
      completedStep: u64(120n)
    });
    await fixture.client.commit(second);
    await fixture.client.close();

    expect(readCurrentPointer(fixture.databasePath, first.runId)).toEqual({
      checkpoint_id: second.logicalRootSha256,
      transition_epoch: second.transitionEpoch,
      operation_id: second.operationId
    });
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
      transitionEpoch: u64(2n)
    });
    await fixture.client.commit(first);
    await fixture.client.commit(second);
    await expect(fixture.client.commit(first)).rejects.toThrow(/superseded/);
  });

  it('rejects immutable descriptor conflicts and stale or gapped epochs without changing current', async () => {
    const fixture = createFixture();
    const first = createDescriptor(fixture.managedRoot);
    await fixture.client.commit(first);
    const sameOperationDifferentDescriptor = createDescriptor(fixture.managedRoot, {
      operationId: first.operationId,
      generation: u64(2n)
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
      transitionEpoch: u64(2n)
    });
    await fixture.client.commit(second);
    const stale = createDescriptor(fixture.managedRoot, {
      operationId: '66666666666666666666666666666666',
      transitionEpoch: u64(1n)
    });
    await expect(fixture.client.commit(stale)).rejects.toThrow(/stale/);
    const gapped = createDescriptor(fixture.managedRoot, {
      operationId: '77777777777777777777777777777777',
      transitionEpoch: u64(4n)
    });
    await expect(fixture.client.commit(gapped)).rejects.toThrow(/exactly one/);
    await fixture.client.close();
    expect(readCurrentPointer(fixture.databasePath, first.runId)).toEqual({
      checkpoint_id: second.logicalRootSha256,
      transition_epoch: second.transitionEpoch,
      operation_id: second.operationId
    });
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

  it('requires the first per-run transition epoch to be exactly one', async () => {
    const fixture = createFixture();
    const descriptor = createDescriptor(fixture.managedRoot, { transitionEpoch: u64(2n) });
    await expect(fixture.client.commit(descriptor)).rejects.toThrow(/first per-run transitionEpoch/);
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
