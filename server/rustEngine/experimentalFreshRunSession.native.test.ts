import { mkdirSync, mkdtempSync, readdirSync, rmSync } from 'node:fs';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import Database from 'better-sqlite3';
import { afterEach, describe, expect, it } from 'vitest';
import { createExperimentalServerRuntime } from './experimentalStartup.ts';
import { BackgroundOutputPump } from './backgroundOutput.ts';
import { ExternalControllerRouting } from './externalRouting.ts';
import { createRustWelcome } from './browserMetadata.ts';
import type { RustBackgroundEvent, RustBackgroundFrameCopy } from '../../src/protocol/rustBackground.ts';
import type { ExperimentalEngineInit } from './experimentalNativeBridge.ts';
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
/** Bounded queues for the real production-addon background owner. */
const BACKGROUND_INIT: ExperimentalEngineInit = {
  contractVersion: 1,
  maxInboundBatches: 16,
  maxInboundCommands: 16,
  maxInboundOwnedBytes: 2 * 1024 * 1024,
  maxBatchCommands: 1,
  maxBatchOwnedBytes: 1024 * 1024,
  maxOutputReliable: 32,
  maxOutputReliableOwnedBytes: 16 * 1024 * 1024,
  maxOutputDiscrete: 4,
  maxOutputDiscreteOwnedBytes: 1024 * 1024,
  maxOutputTotalOwnedBytes: 32 * 1024 * 1024,
  maxOutputEventOwnedBytes: 1024 * 1024,
  maxOutputFrameConnections: 4
};
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

describe('experimental server startup composition', () => {
  it('routes a real fresh controller through assignment, observation, and shared action admission', async () => {
    const paths = createFixturePaths('server-output');
    const owner = await createExperimentalServerRuntime({ databasePath: paths.databasePath,
      managedDirectory: paths.managedRoot, seed: 42, onWake() {} });
    const events: RustBackgroundEvent[] = [];
    const packets: Array<{ type: string }> = [];
    let frameCount = 0;
    let routing!: ExternalControllerRouting;
    const pump = new BackgroundOutputPump({
      owner, maxControllers: 4, hasFrameRecipients: () => true,
      send(_connection, message) { packets.push(message); return true; },
      event(event) { events.push(event); routing.event(event); },
      frame(lease) { frameCount++; lease.release(); }
    });
    routing = new ExternalControllerRouting({ native: owner.runtime, admission: pump.admission,
      maxControllers: 4, maxActionsPerSecond: 120, maxActionsPerTick: 1,
      send(_connection, message) { packets.push(message); return true; } });
    try {
      owner.runtime.start();
      routing.join(1, { type: 'join', mode: 'player', name: 'output-bot' }, 'bot');
      const deadline = performance.now() + 10_000;
      while (!packets.some(packet => packet.type === 'sensors') && performance.now() < deadline) {
        const draining = pump.drain();
        expect(pump.drain()).toBe(draining);
        await draining;
        await new Promise<void>(done => setImmediate(done));
      }
      expect(packets.filter(packet => packet.type === 'assign'), JSON.stringify(events.filter(event => event.kind === 'commandRejected'))).toHaveLength(1);
      expect(packets.some(packet => packet.type === 'sensors')).toBe(true);
      const assignment = events.find(event => event.controllerJoinAssignment)?.controllerJoinAssignment;
      if (!assignment) throw new Error('missing fresh assignment');
      routing.action(1, { type: 'action', snakeId: assignment.snakeId, turn: 0.5, boost: 0, tick: 0 });
      while (!events.some(event => event.kind === 'controllerActionApplied') && performance.now() < deadline) {
        await pump.drain();
        await new Promise<void>(done => setImmediate(done));
      }
      expect(events.some(event => event.kind === 'controllerActionApplied')).toBe(true);
      routing.disconnect(1);
      routing.join(2, { type: 'join', mode: 'player', name: 'output-bot', resumeToken: assignment.resumeToken }, 'bot');
      while (!events.some(event => event.controllerReclaimResolution?.accepted) && performance.now() < deadline) {
        await pump.drain();
        await new Promise<void>(done => setImmediate(done));
      }
      expect(events.some(event => event.controllerReclaimResolution?.accepted)).toBe(true);
      const reclaimed = events.find(event => event.controllerReclaimAssignment)?.controllerReclaimAssignment;
      expect(reclaimed?.snakeId).toBe(assignment.snakeId);
      expect(reclaimed?.resumeToken).not.toBe(assignment.resumeToken);
      expect(frameCount).toBeGreaterThan(0);
      expect(owner.runtime.health().faultCode).toBeUndefined();
    } finally { await owner.close(); }
  }, 30_000);
  it('durably creates one unstarted owner and refuses to replace its database', async () => {
    const paths = createFixturePaths('server-startup');
    const options = { databasePath: paths.databasePath, managedDirectory: paths.managedRoot, seed: 42, onWake: () => {} };
    const owner = await createExperimentalServerRuntime(options);
    try {
      expect(owner.metadata.seed).toBe(42);
      expect(createRustWelcome(owner.metadata)).toMatchObject({ worldSeed: 42, sensorSpec: { sensorCount: 83 },
        settings: { core: { snakeCount: 55, simSpeed: 1 } }, inferenceMode: { activeBackend: 'native' } });
      expect(owner.runtime.health()).toMatchObject({ lifecycle: 'created', completedStep: '0000000000000000' });
      expect(readCurrentPointer(paths.databasePath, owner.metadata.runId)?.checkpoint_id).toBe(owner.runStart.checkpointId);
      await expect(createExperimentalServerRuntime(options)).rejects.toMatchObject({ code: 'EEXIST' });
      expect(owner.runtime.health().lifecycle).toBe('created');
      expect(countManagedFiles(paths.managedRoot)).toBe(1);
      await owner.admitCheckpoint();
    } finally {
      const closing = owner.close();
      expect(owner.close()).toBe(closing);
      await closing;
    }
    expect(owner.runtime.health().lifecycle).toBe('stopped');
    expect(readCurrentPointer(paths.databasePath, owner.metadata.runId)?.checkpoint_id).toBe(owner.runStart.checkpointId);
  }, 30_000);
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
        'createBackgroundRuntime',
        'initialize',
        'publishFirstScheduledFrameV1',
        'publishInitialFrameV1',
        'publishRunStartCheckpoint',
        'snapshot',
        'startupMetadata'
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
    await expect(session.createBackgroundRuntime(BACKGROUND_INIT, () => {})).rejects.toThrow(
      /experimental one-shot|scheduler|already.*authority/i
    );
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

  it('transfers the durable production authority once and services background commands without Node stepping', async () => {
    const paths = createFixturePaths('background');
    const client = new CheckpointPersistenceClient({
      databasePath: paths.databasePath,
      managedRootPath: paths.managedRoot
    });
    clients.push(client);
    const session = await loadExperimentalFreshRunSession({
      nativeManifestDirectory: NATIVE_DIRECTORY,
      loadBinding,
      runId: 'background-production-lineage',
      seed: 42,
      memoryCeilingBytes: P0_MEMORY_CEILING,
      managedDirectory: paths.managedRoot,
      persistence: client
    });
    let wakes = 0;
    /** Retained callback keeps native notifications observable for the test. */
    const wake = (): void => { wakes += 1; };
    await session.initialize();
    const startup = session.startupMetadata();
    expect(startup).toMatchObject({ seed: 42, serializerVersion: 1, sensorVersion: 3 });
    expect(startup.settings.find(setting => setting.path === 'snakeCount')?.value).toBe(55);
    expect(session.startupMetadata()).toEqual(startup);
    await expect(session.createBackgroundRuntime(BACKGROUND_INIT, wake)).rejects.toThrow(
      /running authority|publication/i
    );
    expect(session.snapshot().phase).toBe('pendingDurability');
    const committed = await session.commitPendingRunStart('34343434343434343434343434343434');
    await session.activateRunningAuthority();
    await expect(session.createBackgroundRuntime({
      ...BACKGROUND_INIT, maxOutputEventOwnedBytes: 1
    }, wake)).rejects.toThrow(/fit one authority event/i);
    expect(session.snapshot()).toMatchObject({ phase: 'running', completedStep: '0000000000000000' });
    const creating = session.createBackgroundRuntime(BACKGROUND_INIT, wake);
    await expect(session.createBackgroundRuntime(BACKGROUND_INIT, wake)).rejects.toThrow(/already in flight/i);
    const runtime = await creating;
    try {
      expect(session.snapshot().phase).toBe('background');
      expect(runtime.health()).toMatchObject({
        lifecycle: 'created', worldEpoch: committed.transitionEpoch,
        generation: '0000000000000001', completedStep: '0000000000000000'
      });
      expect(runtime.latestDisplay()).toBeNull();
      await expect(session.createBackgroundRuntime(BACKGROUND_INIT, wake)).rejects.toThrow(/already transferred/i);
      await expect(session.initialize()).rejects.toThrow(/already initialized/i);
      await expect(session.publishFirstScheduledFrameV1()).rejects.toThrow(/not been initialized/i);
      expect(() => runtime.drainOutputs(-1, BACKGROUND_INIT.maxOutputEventOwnedBytes)).toThrow(/positive/i);
      runtime.start();
      expect(() => runtime.start()).toThrow(/cannot start/i);
      const deadline = performance.now() + 10_000;
      const events: RustBackgroundEvent[] = [];
      while (BigInt(`0x${runtime.health().completedStep}`) < 2n && performance.now() < deadline) {
        events.push(...runtime.drainOutputs(16, BACKGROUND_INIT.maxOutputEventOwnedBytes).events);
        await new Promise<void>(resolveImmediate => setImmediate(resolveImmediate));
      }
      expect(BigInt(`0x${runtime.health().completedStep}`)).toBeGreaterThanOrEqual(2n);
      /** Drain priority output before trying the non-blocking cached frame copy. */
      const copyFrame = async (destination: Uint8Array, afterSequence: string): Promise<RustBackgroundFrameCopy> => {
        while (performance.now() < deadline) {
          events.push(...runtime.drainOutputs(16, BACKGROUND_INIT.maxOutputEventOwnedBytes).events);
          const result = runtime.copyLatestFrame(destination, afterSequence);
          if (result.status === 'copied' || result.status === 'tooSmall') return result;
          await new Promise<void>(resolveImmediate => setImmediate(resolveImmediate));
        }
        throw new Error('background display did not become available');
      };
      const short = new Uint8Array(7).fill(0xa5);
      const rejectedCopy = await copyFrame(short, '0000000000000000');
      expect(rejectedCopy.status).toBe('tooSmall');
      expect([...short]).toEqual(Array(7).fill(0xa5));
      expect(() => runtime.copyLatestFrame(short, 'x'.repeat(1_000))).toThrow(/afterSequence/i);
      expect(() => runtime.copyLatestFrame(new Uint8Array(new SharedArrayBuffer(32)), '0000000000000000')).toThrow(/non-shared/i);
      expect(() => runtime.copyLatestFrame(new Float32Array(8) as unknown as Uint8Array, '0000000000000000')).toThrow(/Uint8Array/i);
      const backing = new Uint8Array(1024 * 1024 + 16).fill(0xa5);
      const destination = backing.subarray(8, -8);
      Object.defineProperty(destination, 'buffer', { get: () => { throw new Error('must use intrinsic storage'); } });
      const first = await copyFrame(destination, '0000000000000000');
      if (first.status !== 'copied') throw new Error('admitted Node buffer must fit the P0 frame');
      const retained = backing.slice(8, 8 + first.display.frameByteLength);
      expect(readFrameHeader(new Float32Array(retained.buffer))).toMatchObject({
        generation: Number(BigInt(`0x${first.display.generation}`)),
        totalSnakes: first.display.totalSnakes, aliveCount: first.display.aliveSnakes
      });
      expect([...backing.subarray(0, 8)]).toEqual(Array(8).fill(0xa5));
      expect([...backing.subarray(-8)]).toEqual(Array(8).fill(0xa5));
      expect(backing.subarray(8 + first.display.frameByteLength).every(byte => byte === 0xa5)).toBe(true);
      const nextDestination = new Uint8Array(1024 * 1024);
      const next = await copyFrame(nextDestination, first.display.sequence);
      if (next.status !== 'copied') throw new Error('next complete frame must become available');
      expect(BigInt(`0x${next.display.completedStep}`)).toBeGreaterThan(BigInt(`0x${first.display.completedStep}`));
      expect(next.display.generationTime).toBeGreaterThan(first.display.generationTime);
      expect(backing.subarray(8, 8 + first.display.frameByteLength)).toEqual(retained);
      expect(events.some(event => event.kind === 'display' && event.display?.frameByteLength)).toBe(true);
      // A wrong-phase control must return through the production queue without faulting the game.
      runtime.submitPrepareGenerationReassignments('0000000000000001');
      while (!events.some(event => event.commandSequence === '0000000000000001') && performance.now() < deadline) {
        events.push(...runtime.drainOutputs(16, BACKGROUND_INIT.maxOutputEventOwnedBytes).events);
        await new Promise<void>(resolveImmediate => setImmediate(resolveImmediate));
      }
      expect(events).toContainEqual(expect.objectContaining({
        kind: 'commandRejected', commandSequence: '0000000000000001', rejectionCode: 'InvalidCommand'
      }));
      expect(events.filter(event => event.kind === 'started')).toHaveLength(1);
      expect(runtime.health().faultCode).toBeUndefined();
      expect(wakes).toBeGreaterThan(0);
    } finally {
      runtime.requestStop();
      await runtime.join();
    }
    expect(runtime.health().lifecycle).toBe('stopped');
    expect(runtime.health().faultCode).toBeUndefined();
    await closeClient(client);
    expect(countManagedFiles(paths.managedRoot)).toBe(1);
    expect(readCurrentPointer(paths.databasePath, 'background-production-lineage')?.checkpoint_id)
      .toBe(committed.checkpointId);
  }, 30_000);
});
