import { mkdirSync, mkdtempSync, readdirSync, rmSync } from 'node:fs';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import Database from 'better-sqlite3';
import { afterEach, describe, expect, it } from 'vitest';
import { FRAME_HEADER_FLOATS, readFrameHeader } from '../../src/protocol/frame.ts';
import {
  CheckpointPersistenceClient,
  type ManagedCheckpointCommitResult
} from './checkpointPersistenceClient.ts';
import {
  loadExperimentalFreshRunSession,
  type ExperimentalFreshRunFrameV1,
  type ExperimentalFreshRunNativeBinding,
  type ExperimentalFreshRunNativeHandle
} from './experimentalFreshRunSession.ts';
import {
  parseManagedCheckpointDescriptor
} from './checkpointPersistenceProtocol.ts';

/** Native crate directory used for the independent source identity calculation. */
const NATIVE_DIRECTORY = resolve(import.meta.dirname, '../../native');
/** Generated normal production-addon loader. */
const NATIVE_LOADER = resolve(NATIVE_DIRECTORY, 'index.js');
/** Four-GiB hard ceiling admitted by the provisional fixed P0 profile. */
const P0_MEMORY_CEILING = 4n * 1024n * 1024n * 1024n;
/** CommonJS loader scoped to this ESM integration test. */
const require = createRequire(import.meta.url);
/** Disposable fixture roots removed after their workers stop. */
const fixtureRoots: string[] = [];
/** Persistence workers closed before their databases are removed. */
const clients: CheckpointPersistenceClient[] = [];

/** Paths owned by one disposable production-addon fixture. */
interface FixturePaths {
  /** Root recursively removed after the test. */
  root: string;
  /** Controlled immutable managed-file directory. */
  managedRoot: string;
  /** Disposable SQLite metadata database. */
  databasePath: string;
}

/** Current-pointer identity read after the worker closes. */
interface CurrentPointerRow {
  /** Content-addressed checkpoint identity. */
  checkpoint_id: string;
  /** Exact operation token committed by the worker. */
  operation_id: string;
  /** Exact process-local correlation value. */
  transition_epoch: string;
}

/** Create one empty disposable root and managed directory. */
function createFixturePaths(label: string): FixturePaths {
  const root = mkdtempSync(join(tmpdir(), `slither-fresh-session-${label}-`));
  fixtureRoots.push(root);
  const managedRoot = join(root, 'checkpoint-v3');
  mkdirSync(managedRoot);
  return { root, managedRoot, databasePath: join(root, 'metadata.sqlite') };
}

/** Load the freshly built normal addon. */
function loadBinding(): ExperimentalFreshRunNativeBinding {
  return require(NATIVE_LOADER) as ExperimentalFreshRunNativeBinding;
}

/** Convert sync throws and async rejections into one assertion-friendly promise. */
function invokeAsync(operation: () => unknown): Promise<unknown> {
  return Promise.resolve().then(operation);
}

/** Count only final immutable managed files, excluding any unrelated entries. */
function countManagedFiles(directory: string): number {
  return readdirSync(directory).filter(name => name.endsWith('.checkpoint-v3')).length;
}

/** Read one current pointer after the exclusive worker connection closes. */
function readCurrentPointer(databasePath: string, runId: string): CurrentPointerRow | undefined {
  const database = new Database(databasePath, { readonly: true });
  try {
    return database.prepare(
      'SELECT checkpoint_id, operation_id, transition_epoch ' +
      'FROM rust_checkpoint_v3_current WHERE run_id = ?'
    ).get(runId) as CurrentPointerRow | undefined;
  } finally {
    database.close();
  }
}

/** Close one tracked persistence worker exactly once. */
async function closeClient(client: CheckpointPersistenceClient): Promise<void> {
  await client.close();
  const index = clients.indexOf(client);
  if (index >= 0) clients.splice(index, 1);
}

/** Exact scalar expectations for one real Rust frame walk. */
interface FrameExpectation {
  /** Completed authoritative step represented by the frame. */
  completedStep: string;
  /** Exact pellet records expected after the represented authority boundary. */
  pellets: number;
  /** Exact Float32 entry count. */
  floatLength: string;
  /** Exact byte count. */
  byteLength: string;
}

/** Validate one Rust payload by walking the current browser frame-v1 layout. */
function expectCompleteFrameV1(
  frame: ExperimentalFreshRunFrameV1,
  expected: FrameExpectation
): void {
  const bytes = Uint8Array.from(frame.bytes);
  expect(bytes.byteLength % Float32Array.BYTES_PER_ELEMENT).toBe(0);
  const floats = new Float32Array(
    bytes.buffer,
    bytes.byteOffset,
    bytes.byteLength / Float32Array.BYTES_PER_ELEMENT
  );
  const header = readFrameHeader(floats);
  expect(header).toEqual({
    generation: 1,
    totalSnakes: 65,
    aliveCount: 65,
    worldRadius: 3_500,
    cameraX: 0,
    cameraY: 0,
    zoom: 1
  });
  let cursor = FRAME_HEADER_FLOATS;
  for (let snake = 0; snake < header.aliveCount; snake += 1) {
    expect(cursor + 8).toBeLessThanOrEqual(floats.length);
    const pointCount = floats[cursor + 7] ?? Number.NaN;
    expect(Number.isInteger(pointCount)).toBe(true);
    expect(pointCount).toBeGreaterThanOrEqual(0);
    cursor += 8 + pointCount * 2;
    expect(cursor).toBeLessThanOrEqual(floats.length);
  }
  expect(cursor).toBeLessThan(floats.length);
  const pelletCount = floats[cursor] ?? Number.NaN;
  expect(pelletCount).toBe(expected.pellets);
  cursor += 1 + pelletCount * 5;
  expect(cursor).toBe(floats.length);
  expect(frame).toMatchObject({
    generation: '0000000000000001',
    completedStep: expected.completedStep,
    totalSnakes: '0000000000000041',
    aliveSnakes: '0000000000000041',
    pellets: expected.pellets.toString(16).padStart(16, '0'),
    floatLength: expected.floatLength,
    byteLength: expected.byteLength
  });
  expect(BigInt(`0x${frame.floatLength}`)).toBe(BigInt(floats.length));
  expect(BigInt(`0x${frame.byteLength}`)).toBe(BigInt(bytes.byteLength));
}

afterEach(async () => {
  for (const client of clients.splice(0)) await client.close().catch(() => {});
  for (const root of fixtureRoots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe('experimental fixed-P0 production-addon fresh-run session', () => {
  it('keeps one real Rust boundary through file publication, SQLite retry, exact ack, and activation', async () => {
    const binding = loadBinding();
    expect(binding.nativeAddonBuildClass()).toBe('production');
    expect(binding.ExperimentalStage6aFreshRunSession).toBeTypeOf('function');
    expect(Object.getOwnPropertyNames(binding.ExperimentalStage6aFreshRunSession.prototype).sort())
      .toEqual([
        'acknowledgeRunStartPersistence',
        'activateRunningAuthority',
        'constructor',
        'initialize',
        'publishFirstScheduledFrameV1',
        'publishInitialFrameV1',
        'publishRunStartCheckpoint',
        'snapshot'
      ]);
    expect((binding as unknown as Record<string, unknown>)['Stage6RunStartHandoffFixtureSession'])
      .toBeUndefined();

    const donorPaths = createFixturePaths('donor');
    const donor: ExperimentalFreshRunNativeHandle =
      new binding.ExperimentalStage6aFreshRunSession(
        'donor-lineage',
        '89abcdef',
        '0000000100000000'
      );
    expect(donor.snapshot()).toEqual({ phase: 'created' });
    const donorInitialization = donor.initialize();
    expect(() => donor.initialize()).toThrow(/already in flight/i);
    await expect(donorInitialization).resolves.toMatchObject({
      phase: 'pendingDurability',
      generation: '0000000000000001',
      completedStep: '0000000000000000',
      snakeCount: '0000000000000000',
      pelletCount: '0000000000000000'
    });
    await expect(invokeAsync(() => donor.publishInitialFrameV1())).rejects.toThrow(
      /frame-v1.*requires published running authority/i
    );
    expect(donor.snapshot()).toMatchObject({ initialFramePublished: false });
    await expect(invokeAsync(() => donor.publishFirstScheduledFrameV1())).rejects.toThrow(
      /requires the initial frame/i
    );
    await expect(invokeAsync(() => donor.activateRunningAuthority())).rejects.toThrow(
      /persistence.*acknowledgement/i
    );
    await expect(invokeAsync(() => donor.publishRunStartCheckpoint({
      managedDirectory: '\ud800',
      operationId: '10101010101010101010101010101010'
    }))).rejects.toThrow(/managedDirectory.*well-formed|invalid utf-16/i);
    await expect(invokeAsync(() => donor.publishRunStartCheckpoint({
      managedDirectory: donorPaths.managedRoot,
      operationId: '1010101010101010101010101010101A'
    }))).rejects.toThrow(/operation ID.*lowercase hexadecimal/i);
    const donorDescriptor = parseManagedCheckpointDescriptor(
      await donor.publishRunStartCheckpoint({
        managedDirectory: donorPaths.managedRoot,
        operationId: '10101010101010101010101010101010'
      })
    );
    let reentrantSnapshot: unknown;
    let reentrantActivation: Promise<unknown> | undefined;
    let reentrantActivationThrow: unknown;
    const reentrantDescriptor = {
      ...donorDescriptor,
      operationId: '12121212121212121212121212121212'
    };
    Object.defineProperty(reentrantDescriptor, 'protocolVersion', {
      configurable: true,
      enumerable: true,
      get() {
        reentrantSnapshot = donor.snapshot();
        try {
          reentrantActivation = donor.activateRunningAuthority();
        } catch (error) {
          reentrantActivationThrow = error;
        }
        return donorDescriptor.protocolVersion;
      }
    });
    expect(() => donor.acknowledgeRunStartPersistence(reentrantDescriptor)).toThrow(
      /acknowledgement.*operation/i
    );
    expect(reentrantSnapshot).toMatchObject({ phase: 'acknowledgingPersistence' });
    if (reentrantActivation === undefined) {
      expect(reentrantActivationThrow).toBeInstanceOf(Error);
      expect((reentrantActivationThrow as Error).message).toMatch(
        /acknowledgingPersistence.*in flight/i
      );
    } else {
      await expect(reentrantActivation).rejects.toThrow(/acknowledgingPersistence.*in flight/i);
    }
    expect(() => donor.acknowledgeRunStartPersistence({
      ...donorDescriptor,
      protocolVersion: 1.5
    } as unknown as typeof donorDescriptor)).toThrow(/protocolVersion.*exact supported integer/i);
    expect(() => donor.acknowledgeRunStartPersistence({
      ...donorDescriptor,
      unexpectedAuthority: 'forbidden'
    } as unknown as typeof donorDescriptor)).toThrow(/unknown or missing|unknown field/i);
    expect(() => donor.acknowledgeRunStartPersistence({
      ...donorDescriptor,
      runId: 'x'.repeat(257)
    })).toThrow(/runId.*256-byte limit/i);
    expect(() => donor.acknowledgeRunStartPersistence({
      ...donorDescriptor,
      operationId: '11111111111111111111111111111111'
    })).toThrow(/acknowledgement.*operation/i);
    expect(donor.snapshot()).toMatchObject({
      phase: 'awaitingPersistence',
      checkpointPublished: true,
      persistenceAcknowledged: false,
      authorityPublished: false
    });

    const premature: ExperimentalFreshRunNativeHandle =
      new binding.ExperimentalStage6aFreshRunSession(
        'premature-lineage',
        '00000001',
        '0000000100000000'
      );
    await premature.initialize();
    expect(() => premature.acknowledgeRunStartPersistence(donorDescriptor)).toThrow(
      /before checkpoint publication/i
    );
    expect(premature.snapshot()).toMatchObject({
      phase: 'pendingDurability',
      checkpointPublished: false,
      persistenceAcknowledged: false,
      authorityPublished: false
    });

    const paths = createFixturePaths('handoff');
    const client = new CheckpointPersistenceClient({
      databasePath: paths.databasePath,
      managedRootPath: paths.managedRoot
    });
    clients.push(client);
    let commitAttempts = 0;
    const session = await loadExperimentalFreshRunSession({
      nativeManifestDirectory: NATIVE_DIRECTORY,
      loadBinding,
      runId: 'real-fixed-p0-lineage',
      seed: 0x89ab_cdef,
      memoryCeilingBytes: P0_MEMORY_CEILING,
      managedDirectory: paths.managedRoot,
      persistence: {
        async commit(value): Promise<ManagedCheckpointCommitResult> {
          commitAttempts += 1;
          if (commitAttempts === 1) throw new Error('injected SQLite persistence failure');
          return client.commit(value);
        }
      }
    });
    await expect(session.initialize()).resolves.toMatchObject({
      phase: 'pendingDurability',
      checkpointPublished: false,
      persistenceAcknowledged: false,
      authorityPublished: false,
      snakeCount: '0000000000000000',
      pelletCount: '0000000000000000'
    });
    await expect(session.activateRunningAuthority()).rejects.toThrow(
      /persistence.*acknowledgement/i
    );
    const operationId = '20202020202020202020202020202020';
    await expect(session.commitPendingRunStart(operationId)).rejects.toThrow(
      /injected SQLite persistence failure/i
    );
    expect(session.snapshot()).toMatchObject({
      phase: 'awaitingPersistence',
      checkpointPublished: true,
      persistenceAcknowledged: false,
      authorityPublished: false,
      snakeCount: '0000000000000000',
      pelletCount: '0000000000000000'
    });
    expect(countManagedFiles(paths.managedRoot)).toBe(1);
    await expect(session.activateRunningAuthority()).rejects.toThrow(
      /persistence.*acknowledgement/i
    );

    const committed = await session.commitPendingRunStart(operationId);
    expect(commitAttempts).toBe(2);
    expect(countManagedFiles(paths.managedRoot)).toBe(1);
    expect(session.snapshot()).toMatchObject({
      phase: 'durableBoundary',
      transitionEpoch: committed.transitionEpoch,
      checkpointPublished: true,
      persistenceAcknowledged: true,
      authorityPublished: false
    });
    await expect(session.activateRunningAuthority()).resolves.toEqual({
      worldEpoch: committed.transitionEpoch,
      generation: '0000000000000001',
      completedStep: '0000000000000000',
      populationEpoch: '0000000000000001'
    });
    expect(session.snapshot()).toEqual({
      phase: 'running',
      transitionEpoch: committed.transitionEpoch,
      generation: '0000000000000001',
      completedStep: '0000000000000000',
      checkpointPublished: true,
      persistenceAcknowledged: true,
      authorityPublished: true,
      initialFramePublished: false,
      firstScheduledFramePublished: false,
      snakeCount: '0000000000000041',
      pelletCount: '0000000000000dac',
      faultDetail: undefined
    });
    await expect(session.publishFirstScheduledFrameV1()).rejects.toThrow(/requires the initial/i);
    const initialFrame = await session.publishInitialFrameV1();
    expectCompleteFrameV1(initialFrame, {
      completedStep: '0000000000000000',
      pellets: 3_500,
      floatLength: '00000000000048f6',
      byteLength: '00000000000123d8'
    });
    const initialBytes = Uint8Array.from(initialFrame.bytes);
    expect(session.snapshot()).toEqual({
      phase: 'running',
      transitionEpoch: committed.transitionEpoch,
      generation: '0000000000000001',
      completedStep: '0000000000000000',
      checkpointPublished: true,
      persistenceAcknowledged: true,
      authorityPublished: true,
      initialFramePublished: true,
      firstScheduledFramePublished: false,
      snakeCount: '0000000000000041',
      pelletCount: '0000000000000dac',
      faultDetail: undefined
    });
    await expect(session.publishInitialFrameV1()).rejects.toThrow(/already.*published/i);
    const scheduledFrame = await session.publishFirstScheduledFrameV1();
    expectCompleteFrameV1(scheduledFrame, {
      completedStep: '0000000000000001',
      pellets: 3_495,
      floatLength: '00000000000048e7',
      byteLength: '000000000001239c'
    });
    expect(Uint8Array.from(scheduledFrame.bytes)).not.toEqual(initialBytes);
    expect(session.snapshot()).toMatchObject({
      phase: 'running',
      authorityPublished: true,
      initialFramePublished: true,
      firstScheduledFramePublished: true,
      completedStep: '0000000000000001',
      pelletCount: '0000000000000da7'
    });
    await expect(session.publishFirstScheduledFrameV1()).rejects.toThrow(/already.*published/i);
    expect(session.snapshot()).toMatchObject({
      completedStep: '0000000000000001',
      firstScheduledFramePublished: true
    });
    await expect(session.activateRunningAuthority()).rejects.toThrow(/already.*published/i);
    await expect(session.commitPendingRunStart(operationId)).rejects.toThrow(/already.*published/i);
    expect(commitAttempts).toBe(2);
    expect(countManagedFiles(paths.managedRoot)).toBe(1);

    await closeClient(client);
    expect(readCurrentPointer(paths.databasePath, 'real-fixed-p0-lineage')).toEqual({
      checkpoint_id: committed.checkpointId,
      operation_id: operationId,
      transition_epoch: committed.transitionEpoch
    });
  }, 120_000);

  it('keeps a failed low-memory construction uninitialized and explicitly retryable', async () => {
    const binding = loadBinding();
    const session = new binding.ExperimentalStage6aFreshRunSession(
      'low-memory-lineage',
      '00000001',
      '0000000000000001'
    );
    await expect(session.initialize()).rejects.toThrow(/memory|ceiling|admission/i);
    expect(session.snapshot()).toEqual({ phase: 'created' });
    await expect(session.initialize()).rejects.toThrow(/memory|ceiling|admission/i);
    expect(session.snapshot()).toEqual({ phase: 'created' });
  }, 30_000);
});
