import { existsSync, mkdirSync, mkdtempSync, readdirSync, rmSync, statSync, truncateSync, unlinkSync } from 'node:fs';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { isAbsolute, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import Database from 'better-sqlite3';
import { afterEach, describe, expect, it } from 'vitest';
import type { RustBackgroundControllerAction, RustBackgroundControllerDisconnect, RustBackgroundControllerMessage, RustControllerReceiptResolution, RustGenerationAssignmentReceipt, RustBackgroundReclaimAssignment, RustBackgroundReclaimRequest, RustBackgroundReclaimReceipt } from '../../src/protocol/rustBackground.ts';
import { CheckpointPersistenceClient } from './checkpointPersistenceClient.ts';
import { ControllerDeliveryRouter, ReclaimDeliveryRouter } from './controllerDelivery.ts';
import { BackgroundCommandAdmission } from './commandAdmission.ts';
import { GenerationPersistenceHandoff } from './generationPersistenceHandoff.ts';
import { RunStartPersistenceHandoff } from './runStartPersistenceHandoff.ts';
import {
  parseManagedCheckpointDescriptor,
  parseManagedGenerationCommit,
  type ManagedCheckpointDescriptor,
  type ManagedGenerationCommit
} from './checkpointPersistenceProtocol.ts';
import { computeNativeSourceIdentity } from './nativeSourceIdentity.ts';

/** Native source root whose exact build identity the test-hooks addon must contain. */
const NATIVE_DIRECTORY = fileURLToPath(new URL('../../native', import.meta.url));

/** Generated production addon loader, resolved independently from the test working directory. */
const PRODUCTION_NATIVE_LOADER = resolve(import.meta.dirname, '../../native/index.js');

/** Optional isolated test-hooks addon used only by the explicit Stage 3 evidence command. */
const TEST_HOOK_ADDON = process.env['SLITHER_STAGE3_CHECKPOINT_TEST_ADDON'];

/** Optional isolated production addon supplied by the self-contained evidence command. */
const TEST_PRODUCTION_ADDON = process.env['SLITHER_STAGE3_CHECKPOINT_PRODUCTION_ADDON'];

/** CommonJS loader scoped to this ESM integration-test module. */
const require = createRequire(import.meta.url);

/** Temporary fixture roots removed only after their persistence workers have stopped. */
const fixtureRoots: string[] = [];

/** Persistence clients that must stop before their SQLite files and roots are removed. */
const clients: CheckpointPersistenceClient[] = [];

/** Minimal production identity surface used by the accidental-export assertion. */
interface ProductionNativeBinding {
  /** Report whether this is a production or explicitly test-hooks build. */
  nativeAddonBuildClass(): string;
  /** Report the exact native source-tree digest embedded by build.rs. */
  nativeAddonSourceSha256(): string;
  /** Allow inspection of any accidental test-only export without declaring it present. */
  [name: string]: unknown;
}

/** Resolve an optional addon path independently from the caller's working directory. */
function resolveAddonPath(addonPath: string): string {
  return isAbsolute(addonPath) ? addonPath : resolve(addonPath);
}

/** Reject any native evidence addon that was built from a different source tree. */
function assertCurrentNativeSource(
  binding: { nativeAddonSourceSha256(): string },
  label: string
): void {
  const expectedSourceSha256 = computeNativeSourceIdentity(NATIVE_DIRECTORY).sha256;
  const embeddedSourceSha256 = binding.nativeAddonSourceSha256();
  if (embeddedSourceSha256 !== expectedSourceSha256) {
    throw new Error(
      `${label} source SHA is stale: addon=${embeddedSourceSha256}, tree=${expectedSourceSha256}`
    );
  }
}

/** Load the normal generated addon or the harness's isolated production build. */
function loadProductionBinding(): ProductionNativeBinding {
  const addonPath = TEST_PRODUCTION_ADDON
    ? resolveAddonPath(TEST_PRODUCTION_ADDON)
    : PRODUCTION_NATIVE_LOADER;
  const loaded = require(addonPath) as unknown;
  if (typeof loaded !== 'object' || loaded === null) {
    throw new TypeError('Stage 3 production addon did not export an object');
  }
  const exports = loaded as Record<string, unknown>;
  if (typeof exports['nativeAddonBuildClass'] !== 'function') {
    throw new TypeError('Stage 3 production addon is missing nativeAddonBuildClass()');
  }
  if (typeof exports['nativeAddonSourceSha256'] !== 'function') {
    throw new TypeError('Stage 3 production addon is missing nativeAddonSourceSha256()');
  }
  const binding = exports as unknown as ProductionNativeBinding;
  assertCurrentNativeSource(binding, 'Stage 3 production addon');
  return binding;
}

/** Test-only native surface emitted only by the isolated engine-test-hooks build. */
interface Stage3CheckpointHookBinding {
  /** Report the isolated addon's build class. */
  nativeAddonBuildClass(): string;
  /** Report the exact native source-tree digest embedded by build.rs. */
  nativeAddonSourceSha256(): string;
  /** Publish a real managed checkpoint and return only its scalar descriptor. */
  publishStage3CheckpointFixture(options: {
    managedDirectory: string;
    operationId: string;
    transitionEpoch: string;
  }): Promise<unknown>;
  /** Construct one retained real Rust generation persistence/authority session. */
  Stage6GenerationHandoffFixtureSession: new () => Stage6GenerationHandoffSession;
  /** Construct one retained real Rust run-start persistence/activation session. */
  Stage6RunStartHandoffFixtureSession: new () => Stage6RunStartHandoffSession;
  /** Construct one real background-thread generation handoff fixture. */
  Stage6BackgroundGenerationHandoffFixtureSession: new (
    wakeCallback: () => void
  ) => Stage6BackgroundGenerationHandoffSession;
}

/** Scalar-only surface of the retained feature-gated fresh run-start session. */
interface Stage6RunStartHandoffSession {
  /** Publish or exactly retry the admitted generation-one boundary. */
  publishRunStartCheckpoint(options: {
    managedDirectory: string;
    operationId: string;
  }): Promise<unknown>;
  /** Apply only the complete descriptor echoed after the worker transaction. */
  acknowledgeRunStartPersistence(descriptor: ManagedCheckpointDescriptor): void;
  /** Construct and publish the running authority off the event loop. */
  publishRunningAuthority(): Promise<Stage6RunStartPublication>;
  /** Read bounded current-authority and barrier proof. */
  snapshot(): Stage6RunStartHandoffSnapshot;
}

/** Bounded proof of the fresh run-start durability and activation barriers. */
interface Stage6RunStartHandoffSnapshot {
  /** Rust-owned nonzero handoff correlation token. */
  transitionEpoch: string;
  /** Current generation. */
  generation: string;
  /** Current completed fixed-step count. */
  completedStep: string;
  /** Whether the immutable descriptor has published. */
  checkpointPublished: boolean;
  /** Whether the exact worker acknowledgement is retained. */
  persistenceAcknowledged: boolean;
  /** Whether the collision-safe running authority has published. */
  authorityPublished: boolean;
  /** Exact current authoritative snake count. */
  snakeCount: string;
  /** Exact current authoritative pellet count. */
  pelletCount: string;
  /** Physical immutable publications, excluding exact retries. */
  checkpointPublications: number;
  /** Successful boundary-to-running publications. */
  authorityPublications: number;
}

/** Scalar result of one durable run-start activation. */
interface Stage6RunStartPublication {
  /** Activated process-local world incarnation. */
  worldEpoch: string;
  /** Activated generation. */
  generation: string;
  /** Activated completed-step count. */
  completedStep: string;
  /** Activated population/brain epoch. */
  populationEpoch: string;
}

/** Scalar-only surface of the retained feature-gated Rust coordinator session. */
interface Stage6GenerationHandoffSession {
  /** Publish the same-run generation-one step-zero boundary. */
  publishRunStartCheckpoint(options: {
    managedDirectory: string;
    operationId: string;
    transitionEpoch: string;
  }): Promise<unknown>;
  /** Publish or exactly retry the already-admitted generation boundary. */
  publishGenerationCheckpoint(options: {
    managedDirectory: string;
    operationId: string;
  }): Promise<unknown>;
  /** Apply only the complete descriptor echoed after the worker transaction. */
  acknowledgeGenerationPersistence(descriptor: ManagedCheckpointDescriptor): void;
  /** Stage or reborrow the required fresh-snake assignment. */
  prepareGenerationAssignment(): Stage6GenerationAssignment;
  /** Apply one exact local socket-send result. */
  submitGenerationAssignment(result: Omit<Stage6GenerationAssignment, 'snakeId' | 'resumeToken'> & {
    accepted: boolean;
  }): void;
  /** Perform the one final authority swap. */
  publishGenerationStart(): Stage6GenerationStartPublication;
  /** Read bounded current-authority and barrier proof. */
  snapshot(): Stage6GenerationHandoffSnapshot;
}

/** Exact scalar assignment emitted by Rust for the connected fixture controller. */
interface Stage6GenerationAssignment {
  operationEpoch: string;
  eventSequence: string;
  connectionId: string;
  leaseId: string;
  snakeId: string;
  resumeToken: string;
}

/** Bounded current-authority proof returned by the retained Rust session. */
interface Stage6GenerationHandoffSnapshot {
  worldEpoch: string;
  generation: string;
  completedStep: string;
  transitionPending: boolean;
  checkpointPublished: boolean;
  persistenceAcknowledged: boolean;
  generationCheckpointPublications: number;
  authorityPublications: number;
}

/** Scalar result of the one final Rust authority swap. */
interface Stage6GenerationStartPublication {
  worldEpoch: string;
  generation: string;
  completedStep: string;
  populationEpoch: string;
  externalAssignments: number;
}

/** Exact Rust-owned assignment from the background authority output queue. */
interface Stage6BackgroundGenerationAssignment extends Stage6GenerationAssignment {
  /** Browser player or Protocol 2 controller kind. */
  controllerKind: 'player' | 'reinforcementLearning';
  /** Exact frame-v1 identity without Number narrowing. */
  frameV1Id: string;
}

/** One typed output from the background generation fixture. */
interface Stage6BackgroundGenerationEvent {
  /** Same-snake reconnect staged by the actual background runtime. */
  controllerReclaimAssignment?: RustBackgroundReclaimAssignment;
  /** Stable output discriminator. */
  kind: string;
  /** Exact originating command sequence when this is a command response. */
  commandSequence?: string;
  /** Ordinary reliable messages preserved across command-result waits. */
  controllerMessages?: RustBackgroundControllerMessage[];
  /** Exact ordinary-step receipt completion. */
  controllerReceiptResolution?: RustControllerReceiptResolution;
  /** Boundary at which a deferred action entered the current lease. */
  controllerActionCompletedStep?: string;
  /** Rust-owned pending-transition scalars. */
  transition?: {
    ticketSequence: string;
    successorGeneration: string;
    successorCompletedStep: string;
  };
  /** Immutable descriptor and exact compact Rust generation record. */
  checkpoint?: unknown;
  /** Exact operation accepted by Rust's descriptor barrier. */
  acknowledgedOperationId?: string;
  /** Deterministic successor assignments retained by Rust. */
  reassignments?: {
    ready: boolean;
    assignments: Stage6BackgroundGenerationAssignment[];
  };
  /** Exact receipt accounting and retained readiness state. */
  receiptResolution?: {
    matchedAcceptances: string;
    matchedFailures: string;
    ignoredReceipts: string;
    state: 'pending' | 'ready';
    remaining?: string;
    successorGeneration?: string;
    successorCompletedStep?: string;
  };
  /** Final Rust authority publication. */
  generationStart?: {
    ticketSequence: string;
    dueSteps: string;
    publication: Stage6GenerationStartPublication;
    unavailableControllers: unknown[];
  };
  /** Stable recoverable rejection category. */
  rejectionCode?: string;
  /** Bounded recoverable rejection detail. */
  rejectionDetail?: string;
  /** Stable terminal fault category. */
  faultCode?: string;
  /** Bounded terminal fault detail. */
  faultDetail?: string;
}

/** Scalar-only background authority health. */
interface Stage6BackgroundGenerationHealth {
  lifecycle: 'created' | 'running' | 'stopRequested' | 'faulted' | 'stopped';
  loopState: 'ready' | 'externalDeliveryPending' | 'generationTransitionPending' | 'faulted';
  worldEpoch: string;
  generation: string;
  completedStep: string;
  generationCheckpointPublished: boolean;
  generationPersistenceAcknowledged: boolean;
  pendingExternalDeliveries: string;
  schedulerCompletedSteps: string;
  processedCommands: string;
  faultCode?: string;
  faultDetail?: string;
}

/** Feature-gated real background runtime surface used only by isolated evidence. */
interface Stage6BackgroundGenerationHandoffSession {
  /** Publish the independent same-run generation-one step-zero file. */
  publishRunStartCheckpoint(options: {
    managedDirectory: string;
    operationId: string;
    transitionEpoch: string;
  }): Promise<unknown>;
  /** Start the Rust authority thread exactly once. */
  start(): void;
  /** Queue immutable generation file publication. */
  submitGenerationCheckpoint(sequenceHex: string, options: {
    managedDirectory: string;
    operationId: string;
  }): void;
  /** Queue the worker's complete descriptor echo. */
  submitGenerationPersistenceAcknowledgement(
    sequenceHex: string,
    descriptor: ManagedCheckpointDescriptor
  ): void;
  /** Queue deterministic connected-controller reassignment staging. */
  submitPrepareGenerationReassignments(sequenceHex: string): void;
  /** Queue one exact local socket-send receipt. */
  submitGenerationAssignmentReceipt(
    sequenceHex: string,
    result: Omit<Stage6GenerationAssignment, 'snakeId' | 'resumeToken'> & { accepted: boolean }
  ): void;
  /** Queue the separately gated final authority swap. */
  submitPublishGenerationStart(sequenceHex: string): void;
  /** Resolve only an ordinary-step transport barrier. */
  submitControllerDeliveryReceipt(sequenceHex: string, receipt: RustGenerationAssignmentReceipt): void;
  /** Queue one latest action behind the current retained ordinary step. */
  submitControllerAction(sequenceHex: string, action: RustBackgroundControllerAction): void;
  /** Close the exact retained assignment at the next eligible boundary. */
  submitControllerDisconnect(sequenceHex: string, close: RustBackgroundControllerDisconnect): void;
  /** Reclaim the same Rust-owned snake using its previous token. */
  submitControllerReclaim(sequenceHex: string, request: RustBackgroundReclaimRequest): void;
  /** Submit an exact local-send result for the retained reclaim assignment. */
  submitControllerReclaimReceipt(sequenceHex: string, receipt: RustBackgroundReclaimReceipt): void;
  /** Drain typed bounded output. */
  drainOutputs(maxEvents: number, maxOwnedBytes: number): {
    events: Stage6BackgroundGenerationEvent[];
    moreWork: boolean;
    generation: string;
  };
  /** Read bounded scalar state. */
  health(): Stage6BackgroundGenerationHealth;
  /** Request orderly stop. */
  requestStop(): void;
  /** Join on libuv's worker pool. */
  join(): Promise<void>;
}

/** Paths belonging to one disposable cross-language handoff fixture. */
interface FixturePaths {
  /** Root removed after the worker exits. */
  root: string;
  /** Controlled directory where Rust publishes immutable checkpoint files. */
  managedRoot: string;
  /** SQLite metadata database that must not exist before Node commits the descriptor. */
  databasePath: string;
}

/** Exact current-pointer row stored after a successful descriptor commit. */
interface CurrentPointerRow {
  /** Content-addressed checkpoint identity. */
  checkpoint_id: string;
  /** Exact fixed-width transition epoch. */
  transition_epoch: string;
  /** Exact correlation token for this publication. */
  operation_id: string;
}

/** Create one empty disposable managed-checkpoint root without starting SQLite. */
function createFixturePaths(): FixturePaths {
  const root = mkdtempSync(join(tmpdir(), 'slither-stage3-checkpoint-handoff-'));
  fixtureRoots.push(root);
  const managedRoot = join(root, 'managed-checkpoints');
  mkdirSync(managedRoot);
  return {
    root,
    managedRoot,
    databasePath: join(root, 'metadata.sqlite')
  };
}

/** Load and validate the explicitly supplied isolated test-hooks addon. */
function loadTestHookBinding(): Stage3CheckpointHookBinding {
  if (!TEST_HOOK_ADDON) {
    throw new Error('SLITHER_STAGE3_CHECKPOINT_TEST_ADDON is required for this evidence test');
  }
  const addonPath = resolveAddonPath(TEST_HOOK_ADDON);
  const loaded = require(addonPath) as unknown;
  if (typeof loaded !== 'object' || loaded === null) {
    throw new TypeError('Stage 3 test-hooks addon did not export an object');
  }
  const exports = loaded as Record<string, unknown>;
  if (typeof exports['nativeAddonBuildClass'] !== 'function') {
    throw new TypeError('Stage 3 test-hooks addon is missing nativeAddonBuildClass()');
  }
  if (typeof exports['nativeAddonSourceSha256'] !== 'function') {
    throw new TypeError('Stage 3 test-hooks addon is missing nativeAddonSourceSha256()');
  }
  if (typeof exports['publishStage3CheckpointFixture'] !== 'function') {
    throw new TypeError('Stage 3 test-hooks addon is missing publishStage3CheckpointFixture()');
  }
  if (typeof exports['Stage6GenerationHandoffFixtureSession'] !== 'function') {
    throw new TypeError(
      'Stage 6 test-hooks addon is missing Stage6GenerationHandoffFixtureSession'
    );
  }
  if (typeof exports['Stage6RunStartHandoffFixtureSession'] !== 'function') {
    throw new TypeError(
      'Stage 6 test-hooks addon is missing Stage6RunStartHandoffFixtureSession'
    );
  }
  if (typeof exports['Stage6BackgroundGenerationHandoffFixtureSession'] !== 'function') {
    throw new TypeError(
      'Stage 6 test-hooks addon is missing Stage6BackgroundGenerationHandoffFixtureSession'
    );
  }
  const binding = exports as unknown as Stage3CheckpointHookBinding;
  assertCurrentNativeSource(binding, 'Stage 3 test-hooks addon');
  return binding;
}

/** Start the isolated descriptor-only persistence worker for one fixture. */
function createClient(paths: FixturePaths): CheckpointPersistenceClient {
  const client = new CheckpointPersistenceClient({
    databasePath: paths.databasePath,
    managedRootPath: paths.managedRoot
  });
  clients.push(client);
  return client;
}

/** Publish and strictly parse one deterministic real Rust checkpoint fixture. */
async function publishFixture(
  binding: Stage3CheckpointHookBinding,
  paths: FixturePaths,
  operationId: string
): Promise<ManagedCheckpointDescriptor> {
  return parseManagedCheckpointDescriptor(await binding.publishStage3CheckpointFixture({
    managedDirectory: paths.managedRoot,
    operationId,
    transitionEpoch: '0000000000000001'
  }));
}

/** Read one current pointer after closing the worker that exclusively writes SQLite. */
function readCurrentPointer(databasePath: string, runId: string): CurrentPointerRow | undefined {
  const db = new Database(databasePath, { readonly: true });
  try {
    return db.prepare(
      'SELECT checkpoint_id, transition_epoch, operation_id FROM rust_checkpoint_v3_current WHERE run_id = ?'
    ).get(runId) as CurrentPointerRow | undefined;
  } finally {
    db.close();
  }
}

/** Count committed metadata rows without reading any managed archive bytes. */
function countMetadataRows(databasePath: string): number {
  const db = new Database(databasePath, { readonly: true });
  try {
    const row = db.prepare('SELECT COUNT(*) AS count FROM rust_checkpoint_v3_metadata').get() as {
      count: number;
    };
    return row.count;
  } finally {
    db.close();
  }
}

/** Read the two exact fixed-size generation records selected by one checkpoint. */
function readGenerationRecords(
  databasePath: string,
  checkpointId: string
): { summary: Buffer; hallOfFame: Buffer } {
  const db = new Database(databasePath, { readonly: true });
  try {
    const summary = db.prepare(
      'SELECT record_blob FROM rust_generation_history_v1 WHERE checkpoint_id = ?'
    ).get(checkpointId) as { record_blob: Buffer } | undefined;
    const hallOfFame = db.prepare(
      'SELECT record_blob FROM rust_hall_of_fame_v1 WHERE checkpoint_id = ?'
    ).get(checkpointId) as { record_blob: Buffer } | undefined;
    if (!summary || !hallOfFame || !Buffer.isBuffer(summary.record_blob) ||
      !Buffer.isBuffer(hallOfFame.record_blob)) {
      throw new Error('generation checkpoint omitted one exact metadata record');
    }
    return { summary: summary.record_blob, hallOfFame: hallOfFame.record_blob };
  } finally {
    db.close();
  }
}

/** Reproduce the worker's exact 56-byte generation-history encoding. */
function encodeExpectedSummary(commit: ManagedGenerationCommit): Buffer {
  const summary = commit.summary;
  const record = Buffer.alloc(56);
  record.writeBigUInt64LE(BigInt(`0x${summary.completedGeneration}`), 0);
  record.writeBigUInt64LE(BigInt(`0x${summary.bestF64Hex}`), 8);
  record.writeBigUInt64LE(BigInt(`0x${summary.averageF64Hex}`), 16);
  record.writeBigUInt64LE(BigInt(`0x${summary.minimumF64Hex}`), 24);
  record.writeUInt32LE(Number(BigInt(`0x${summary.speciesCount}`)), 32);
  record.writeUInt32LE(Number(BigInt(`0x${summary.topSpeciesSize}`)), 36);
  record.writeBigUInt64LE(BigInt(`0x${summary.averageWeightF64Hex}`), 40);
  record.writeBigUInt64LE(BigInt(`0x${summary.weightVarianceF64Hex}`), 48);
  return record;
}

/** Reproduce the worker's exact 56-byte Hall-of-Fame reference encoding. */
function encodeExpectedHallOfFame(commit: ManagedGenerationCommit): Buffer {
  const reference = commit.hallOfFame;
  const record = Buffer.alloc(56);
  record.writeBigUInt64LE(BigInt(`0x${reference.completedGeneration}`), 0);
  record.writeUInt32LE(Number(BigInt(`0x${reference.sourcePopulationSlot}`)), 8);
  record.writeUInt32LE(Number(BigInt(`0x${reference.successorPopulationSlot}`)), 12);
  record.writeBigUInt64LE(BigInt(`0x${reference.sourceSnakeId}`), 16);
  record.writeBigUInt64LE(BigInt(`0x${reference.successorGenomeId}`), 24);
  record.writeBigUInt64LE(BigInt(`0x${reference.fitnessF64Hex}`), 32);
  record.writeBigUInt64LE(BigInt(`0x${reference.pointsF64Hex}`), 40);
  record.writeBigUInt64LE(BigInt(`0x${reference.length}`), 48);
  return record;
}

/** Strictly parse the two-field scalar result returned by the native session. */
function parseGenerationPublication(value: unknown): {
  descriptor: ManagedCheckpointDescriptor;
  generationCommit: ManagedGenerationCommit;
} {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    throw new TypeError('native generation publication must be an object');
  }
  const record = value as Record<string, unknown>;
  if (Object.keys(record).length !== 2 || !Object.hasOwn(record, 'descriptor') ||
    !Object.hasOwn(record, 'generationCommit')) {
    throw new TypeError('native generation publication has unknown or missing fields');
  }
  const descriptor = parseManagedCheckpointDescriptor(record['descriptor']);
  const generationCommit = parseManagedGenerationCommit(record['generationCommit'], descriptor);
  if (generationCommit === null) {
    throw new TypeError('native generation publication omitted generation metadata');
  }
  return { descriptor, generationCommit };
}

/** Reliable events retained between predicate-based waits for each fixture session. */
const pendingBackgroundEvents = new WeakMap<Stage6BackgroundGenerationHandoffSession, Stage6BackgroundGenerationEvent[]>();

/** Preserve unrelated reliable events for later waits, including ordinary controller batches. */
async function waitForBackgroundEvent(
  session: Stage6BackgroundGenerationHandoffSession,
  predicate: (event: Stage6BackgroundGenerationEvent) => boolean
): Promise<Stage6BackgroundGenerationEvent> {
  const deadline = Date.now() + 5_000;
  const pending = pendingBackgroundEvents.get(session) ?? [];
  pendingBackgroundEvents.set(session, pending);
  while (Date.now() < deadline) {
    const drained = session.drainOutputs(64, 2 * 1024 * 1024);
    pending.push(...drained.events);
    for (const event of drained.events) {
      if (event.kind === 'fault') {
        throw new Error(`background Rust authority faulted: ${event.faultCode ?? 'unknown'}: ${event.faultDetail ?? ''}`);
      }
    }
    const index = pending.findIndex(predicate);
    if (index >= 0) return pending.splice(index, 1)[0]!;
    await new Promise<void>(resolveImmediate => setImmediate(resolveImmediate));
  }
  throw new Error(`background Rust authority event timed out: ${JSON.stringify(session.health())}`);
}

/** Wait for one bounded background-health condition without reading authority arrays. */
async function waitForBackgroundHealth(
  session: Stage6BackgroundGenerationHandoffSession,
  predicate: (health: Stage6BackgroundGenerationHealth) => boolean
): Promise<Stage6BackgroundGenerationHealth> {
  const deadline = Date.now() + 5_000;
  while (Date.now() < deadline) {
    const health = session.health();
    if (health.faultCode || health.lifecycle === 'faulted') {
      throw new Error(`background Rust authority faulted: ${health.faultCode ?? 'unknown'}: ${health.faultDetail ?? ''}`);
    }
    if (predicate(health)) return health;
    await new Promise<void>(resolveImmediate => setImmediate(resolveImmediate));
  }
  throw new Error(`background Rust authority health timed out: ${JSON.stringify(session.health())}`);
}

/** Stop a client once and remove it from the shared cleanup list. */
async function closeClient(client: CheckpointPersistenceClient): Promise<void> {
  await client.close();
  const index = clients.indexOf(client);
  if (index >= 0) clients.splice(index, 1);
}

afterEach(async () => {
  for (const client of clients.splice(0)) await client.close().catch(() => {});
  for (const root of fixtureRoots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe('Stage 3/6 Rust-to-Node managed checkpoint publication handoff', () => {
  it('keeps the checkpoint fixture export out of the normal production addon', () => {
    const production = loadProductionBinding();
    expect(production.nativeAddonBuildClass()).toBe('production');
    expect(production['publishStage3CheckpointFixture']).toBeUndefined();
    expect(production['Stage6GenerationHandoffFixtureSession']).toBeUndefined();
    expect(production['Stage6RunStartHandoffFixtureSession']).toBeUndefined();
    expect(production['Stage6BackgroundGenerationHandoffFixtureSession']).toBeUndefined();
  });

  const hookIt = TEST_HOOK_ADDON ? it : it.skip;

  hookIt('publishes the immutable Rust file before committing its SQLite pointer', async () => {
    const binding = loadTestHookBinding();
    const paths = createFixturePaths();
    expect(binding.nativeAddonBuildClass()).toBe('test-hooks');
    expect(existsSync(paths.databasePath)).toBe(false);

    const descriptor = await publishFixture(
      binding,
      paths,
      '0123456789abcdef0123456789abcdef'
    );
    const checkpointPath = join(paths.managedRoot, descriptor.relativeFilename);
    expect(existsSync(paths.databasePath)).toBe(false);
    expect(readdirSync(paths.managedRoot)).toEqual([descriptor.relativeFilename]);
    expect(statSync(checkpointPath).isFile()).toBe(true);
    expect(BigInt(statSync(checkpointPath).size)).toBe(BigInt(`0x${descriptor.storedByteCount}`));

    const client = createClient(paths);
    await expect(client.commit(descriptor)).resolves.toEqual({
      operationId: descriptor.operationId,
      transitionEpoch: descriptor.transitionEpoch,
      runId: descriptor.runId,
      checkpointId: descriptor.logicalRootSha256,
      descriptor
    });
    await closeClient(client);

    expect(readCurrentPointer(paths.databasePath, descriptor.runId)).toEqual({
      checkpoint_id: descriptor.logicalRootSha256,
      transition_epoch: descriptor.transitionEpoch,
      operation_id: descriptor.operationId
    });
    expect(countMetadataRows(paths.databasePath)).toBe(1);
  });

  hookIt.each(['truncate', 'delete'] as const)(
    'rejects when the final file is changed by %s without publishing SQLite metadata or a current pointer',
    async mutation => {
      const binding = loadTestHookBinding();
      const paths = createFixturePaths();
      const descriptor = await publishFixture(
        binding,
        paths,
        mutation === 'truncate'
          ? '11111111111111111111111111111111'
          : '22222222222222222222222222222222'
      );
      const checkpointPath = join(paths.managedRoot, descriptor.relativeFilename);
      if (mutation === 'truncate') truncateSync(checkpointPath, 1);
      else unlinkSync(checkpointPath);

      const client = createClient(paths);
      await expect(client.commit(descriptor)).rejects.toThrow(
        mutation === 'truncate' ? /size does not match/i : /ENOENT|no such file/i
      );
      await closeClient(client);

      expect(readCurrentPointer(paths.databasePath, descriptor.runId)).toBeUndefined();
      expect(countMetadataRows(paths.databasePath)).toBe(0);
    }
  );

  hookIt(
    'keeps a real fresh run staged through failed SQLite, exact acknowledgement, and one activation',
    async () => {
      const binding = loadTestHookBinding();
      const paths = createFixturePaths();
      const session = new binding.Stage6RunStartHandoffFixtureSession();
      const initial = session.snapshot();
      expect(initial.transitionEpoch).toMatch(/^[0-9a-f]{16}$/u);
      expect(initial.transitionEpoch).not.toBe('0000000000000000');
      expect(initial).toEqual({
        transitionEpoch: initial.transitionEpoch,
        generation: '0000000000000001',
        completedStep: '0000000000000000',
        checkpointPublished: false,
        persistenceAcknowledged: false,
        authorityPublished: false,
        snakeCount: '0000000000000000',
        pelletCount: '0000000000000000',
        checkpointPublications: 0,
        authorityPublications: 0
      });

      const foreignDescriptor = await publishFixture(
        binding,
        paths,
        '25252525252525252525252525252525'
      );
      expect(() => session.acknowledgeRunStartPersistence(foreignDescriptor)).toThrow(
        /before checkpoint publication/i
      );
      await expect(session.publishRunningAuthority()).rejects.toThrow(
        /requires a successful persistence acknowledgement/i
      );
      expect(session.snapshot()).toEqual(initial);

      const operationId = '26262626262626262626262626262626';
      const descriptor = parseManagedCheckpointDescriptor(
        await session.publishRunStartCheckpoint({
          managedDirectory: paths.managedRoot,
          operationId
        })
      );
      expect(descriptor).toMatchObject({
        operationId,
        transitionEpoch: initial.transitionEpoch,
        generation: '0000000000000001',
        completedStep: '0000000000000000',
        boundaryKind: 'run-start'
      });
      await expect(session.publishRunStartCheckpoint({
        managedDirectory: paths.managedRoot,
        operationId
      })).resolves.toEqual(descriptor);
      await expect(session.publishRunStartCheckpoint({
        managedDirectory: paths.managedRoot,
        operationId: '27272727272727272727272727272727'
      })).rejects.toThrow(/already bound/i);
      expect(session.snapshot()).toMatchObject({
        checkpointPublished: true,
        persistenceAcknowledged: false,
        authorityPublished: false,
        snakeCount: '0000000000000000',
        pelletCount: '0000000000000000',
        checkpointPublications: 1,
        authorityPublications: 0
      });

      const wrongManagedRoot = join(paths.root, 'wrong-managed-root');
      mkdirSync(wrongManagedRoot);
      const failedClient = new CheckpointPersistenceClient({
        databasePath: paths.databasePath,
        managedRootPath: wrongManagedRoot
      });
      clients.push(failedClient);
      const failedHandoff = new RunStartPersistenceHandoff({
        rust: session,
        persistence: failedClient,
        managedDirectory: paths.managedRoot
      });
      await expect(failedHandoff.commitPendingRunStart(operationId)).rejects.toThrow(
        /ENOENT|no such file|missing/i
      );
      await closeClient(failedClient);
      expect(session.snapshot()).toMatchObject({
        persistenceAcknowledged: false,
        authorityPublished: false,
        checkpointPublications: 1,
        authorityPublications: 0
      });
      expect(countMetadataRows(paths.databasePath)).toBe(0);

      const client = createClient(paths);
      const committed = await client.commit(descriptor);
      const mismatchedLogicalRoot = committed.descriptor.logicalRootSha256.endsWith('0')
        ? `${committed.descriptor.logicalRootSha256.slice(0, -1)}1`
        : `${committed.descriptor.logicalRootSha256.slice(0, -1)}0`;
      const mismatchedDescriptor = {
        ...committed.descriptor,
        logicalRootSha256: mismatchedLogicalRoot,
        relativeFilename: `${mismatchedLogicalRoot}.checkpoint-v3`
      };
      expect(() => session.acknowledgeRunStartPersistence(mismatchedDescriptor)).toThrow(
        /acknowledgement.*logical root/i
      );
      expect(session.snapshot()).toMatchObject({
        persistenceAcknowledged: false,
        authorityPublished: false,
        snakeCount: '0000000000000000',
        authorityPublications: 0
      });

      const handoff = new RunStartPersistenceHandoff({
        rust: session,
        persistence: client,
        managedDirectory: paths.managedRoot
      });
      await expect(handoff.commitPendingRunStart(operationId)).resolves.toEqual(committed);
      expect(session.snapshot()).toMatchObject({
        persistenceAcknowledged: true,
        authorityPublished: false,
        snakeCount: '0000000000000000',
        pelletCount: '0000000000000000',
        checkpointPublications: 1,
        authorityPublications: 0
      });
      await handoff.commitPendingRunStart(operationId);
      expect(session.snapshot().persistenceAcknowledged).toBe(true);

      const publication = await session.publishRunningAuthority();
      expect(publication).toEqual({
        worldEpoch: initial.transitionEpoch,
        generation: '0000000000000001',
        completedStep: '0000000000000000',
        populationEpoch: '0000000000000001'
      });
      const activated = session.snapshot();
      expect(activated).toMatchObject({
        transitionEpoch: initial.transitionEpoch,
        persistenceAcknowledged: true,
        authorityPublished: true,
        checkpointPublications: 1,
        authorityPublications: 1
      });
      expect(BigInt(`0x${activated.snakeCount}`)).toBeGreaterThan(0n);
      expect(BigInt(`0x${activated.pelletCount}`)).toBeGreaterThan(0n);
      await expect(session.publishRunningAuthority()).rejects.toThrow(/already been published/i);
      expect(session.snapshot().authorityPublications).toBe(1);
      await closeClient(client);

      expect(readCurrentPointer(paths.databasePath, descriptor.runId)).toEqual({
        checkpoint_id: descriptor.logicalRootSha256,
        transition_epoch: descriptor.transitionEpoch,
        operation_id: descriptor.operationId
      });
      expect(countMetadataRows(paths.databasePath)).toBe(1);
    }
  );

  hookIt(
    'retains one real Rust transition through SQLite commit, exact acknowledgement, assignment, and final swap',
    async () => {
      const binding = loadTestHookBinding();
      const paths = createFixturePaths();
      const session = new binding.Stage6GenerationHandoffFixtureSession();
      const initial = session.snapshot();
      expect(initial.worldEpoch).toMatch(/^[0-9a-f]{16}$/u);
      expect(initial).toEqual({
        worldEpoch: initial.worldEpoch,
        generation: '0000000000000001',
        completedStep: '0000000000000000',
        transitionPending: true,
        checkpointPublished: false,
        persistenceAcknowledged: false,
        generationCheckpointPublications: 0,
        authorityPublications: 0
      });

      const runStart = parseManagedCheckpointDescriptor(
        await session.publishRunStartCheckpoint({
          managedDirectory: paths.managedRoot,
          operationId: '30303030303030303030303030303030',
          transitionEpoch: '0000000000000001'
        })
      );
      expect(() => session.acknowledgeGenerationPersistence(runStart)).toThrow(
        /cannot commit before checkpoint publication/i
      );
      expect(session.snapshot()).toEqual(initial);

      const firstPublication = parseGenerationPublication(
        await session.publishGenerationCheckpoint({
          managedDirectory: paths.managedRoot,
          operationId: '40404040404040404040404040404040'
        })
      );
      expect(firstPublication.descriptor.boundaryKind).toBe('generation');
      expect(firstPublication.descriptor.generation).toBe('0000000000000002');
      expect(firstPublication.descriptor.completedStep).toBe('0000000000000001');
      expect(firstPublication.generationCommit.summary.completedGeneration).toBe(
        '0000000000000001'
      );
      expect(firstPublication.generationCommit.hallOfFame.completedGeneration).toBe(
        '0000000000000001'
      );
      expect(firstPublication.generationCommit.hallOfFame.successorPopulationSlot).toBe(
        '0000000000000000'
      );
      expect(firstPublication.generationCommit.hallOfFame.successorGenomeId).not.toBe(
        '0000000000000000'
      );
      const retryPublication = parseGenerationPublication(
        await session.publishGenerationCheckpoint({
          managedDirectory: paths.managedRoot,
          operationId: firstPublication.descriptor.operationId
        })
      );
      expect(retryPublication).toEqual(firstPublication);
      await expect(session.publishGenerationCheckpoint({
        managedDirectory: paths.managedRoot,
        operationId: '50505050505050505050505050505050'
      })).rejects.toThrow(/already bound/i);
      expect(session.snapshot()).toMatchObject({
        generation: '0000000000000001',
        completedStep: '0000000000000000',
        transitionPending: true,
        checkpointPublished: true,
        persistenceAcknowledged: false,
        generationCheckpointPublications: 1,
        authorityPublications: 0
      });
      expect(() => session.prepareGenerationAssignment()).toThrow(
        /requires a successful persistence acknowledgement/i
      );
      expect(() => session.publishGenerationStart()).toThrow(
        /requires a successful persistence acknowledgement/i
      );

      const client = createClient(paths);
      const handoff = new GenerationPersistenceHandoff({
        rust: session,
        persistence: client,
        managedDirectory: paths.managedRoot
      });
      await expect(handoff.commitPendingGeneration(
        firstPublication.descriptor.operationId
      )).rejects.toThrow(/existing current pointer or explicit branch provenance/i);
      expect(session.snapshot()).toMatchObject({
        generation: '0000000000000001',
        completedStep: '0000000000000000',
        persistenceAcknowledged: false,
        authorityPublications: 0
      });
      expect(countMetadataRows(paths.databasePath)).toBe(0);

      const runStartCommit = await client.commit(runStart);
      expect(runStartCommit.descriptor).toEqual(runStart);
      const generationCommit = await client.commit(
        firstPublication.descriptor,
        firstPublication.generationCommit
      );
      expect(generationCommit.descriptor).toEqual(firstPublication.descriptor);
      expect(() => session.prepareGenerationAssignment()).toThrow(
        /requires a successful persistence acknowledgement/i
      );

      const mismatchedLogicalRoot = generationCommit.descriptor.logicalRootSha256.endsWith('0')
        ? `${generationCommit.descriptor.logicalRootSha256.slice(0, -1)}1`
        : `${generationCommit.descriptor.logicalRootSha256.slice(0, -1)}0`;
      const mismatchedDescriptor = {
        ...generationCommit.descriptor,
        logicalRootSha256: mismatchedLogicalRoot,
        relativeFilename: `${mismatchedLogicalRoot}.checkpoint-v3`
      };
      expect(() => session.acknowledgeGenerationPersistence(mismatchedDescriptor)).toThrow(
        /acknowledgement.*logical root/i
      );
      expect(session.snapshot()).toMatchObject({
        generation: '0000000000000001',
        completedStep: '0000000000000000',
        transitionPending: true,
        persistenceAcknowledged: false,
        authorityPublications: 0
      });

      const acknowledged = await handoff.commitPendingGeneration(
        firstPublication.descriptor.operationId
      );
      expect(acknowledged.descriptor).toEqual(generationCommit.descriptor);
      expect(session.snapshot()).toMatchObject({
        generation: '0000000000000001',
        completedStep: '0000000000000000',
        transitionPending: true,
        persistenceAcknowledged: true,
        generationCheckpointPublications: 1,
        authorityPublications: 0
      });
      await handoff.commitPendingGeneration(firstPublication.descriptor.operationId);
      expect(session.snapshot().persistenceAcknowledged).toBe(true);

      const assignment = session.prepareGenerationAssignment();
      expect(assignment.operationEpoch).toBe(firstPublication.descriptor.transitionEpoch);
      expect(assignment.resumeToken.length).toBeGreaterThan(0);
      expect(() => session.publishGenerationStart()).toThrow(
        /still require matching Node delivery results/i
      );
      expect(() => session.submitGenerationAssignment({
        operationEpoch: assignment.operationEpoch,
        eventSequence: assignment.eventSequence,
        connectionId: assignment.connectionId,
        leaseId: assignment.leaseId === '0000000000000001'
          ? '0000000000000002'
          : '0000000000000001',
        accepted: true
      })).toThrow(/does not match/i);
      expect(session.snapshot().authorityPublications).toBe(0);
      session.submitGenerationAssignment({
        operationEpoch: assignment.operationEpoch,
        eventSequence: assignment.eventSequence,
        connectionId: assignment.connectionId,
        leaseId: assignment.leaseId,
        accepted: true
      });
      const publication = session.publishGenerationStart();
      const expectedWorldEpoch = (BigInt(`0x${initial.worldEpoch}`) + 1n)
        .toString(16)
        .padStart(16, '0');
      expect(publication).toEqual({
        worldEpoch: expectedWorldEpoch,
        generation: '0000000000000002',
        completedStep: '0000000000000001',
        populationEpoch: '0000000000000002',
        externalAssignments: 1
      });
      expect(session.snapshot()).toEqual({
        worldEpoch: expectedWorldEpoch,
        generation: '0000000000000002',
        completedStep: '0000000000000001',
        transitionPending: false,
        checkpointPublished: false,
        persistenceAcknowledged: false,
        generationCheckpointPublications: 1,
        authorityPublications: 1
      });
      expect(() => session.publishGenerationStart()).toThrow(
        /requires generation-transition-pending state/i
      );
      await closeClient(client);

      expect(readCurrentPointer(paths.databasePath, runStart.runId)).toEqual({
        checkpoint_id: firstPublication.descriptor.logicalRootSha256,
        transition_epoch: firstPublication.descriptor.transitionEpoch,
        operation_id: firstPublication.descriptor.operationId
      });
      expect(countMetadataRows(paths.databasePath)).toBe(2);
      expect(readdirSync(paths.managedRoot).sort()).toEqual([
        firstPublication.descriptor.relativeFilename,
        runStart.relativeFilename
      ].sort());
      const records = readGenerationRecords(
        paths.databasePath,
        firstPublication.descriptor.logicalRootSha256
      );
      expect(records.summary).toEqual(encodeExpectedSummary(firstPublication.generationCommit));
      expect(records.hallOfFame).toEqual(
        encodeExpectedHallOfFame(firstPublication.generationCommit)
      );
    }
  );

  hookIt(
    'routes the real background Rust generation barrier through the worker and exact queued acknowledgement',
    async () => {
      const binding = loadTestHookBinding();
      const paths = createFixturePaths();
      let wakeCount = 0;
      const wakeCallback = (): void => {
        wakeCount += 1;
      };
      const session = new binding.Stage6BackgroundGenerationHandoffFixtureSession(wakeCallback);
      let joined = false;
      try {
        const runStart = parseManagedCheckpointDescriptor(
          await session.publishRunStartCheckpoint({
            managedDirectory: paths.managedRoot,
            operationId: '61616161616161616161616161616161',
            transitionEpoch: '0000000000000001'
          })
        );
        const client = createClient(paths);
        await expect(client.commit(runStart)).resolves.toMatchObject({ descriptor: runStart });

        const created = session.health();
        expect(created).toMatchObject({
          lifecycle: 'created',
          loopState: 'ready',
          generation: '0000000000000001',
          completedStep: '0000000000000000',
          generationCheckpointPublished: false,
          generationPersistenceAcknowledged: false,
          processedCommands: '0000000000000000'
        });
        const sourceWorldEpoch = created.worldEpoch;
        session.start();

        const transition = await waitForBackgroundEvent(
          session,
          event => event.kind === 'generationTransitionPending'
        );
        expect(transition.transition).toMatchObject({
          successorGeneration: '0000000000000002',
          successorCompletedStep: '0000000000000001'
        });
        expect(session.health()).toMatchObject({
          loopState: 'generationTransitionPending',
          worldEpoch: sourceWorldEpoch,
          generation: '0000000000000001',
          completedStep: '0000000000000000'
        });

        session.submitGenerationPersistenceAcknowledgement(
          '0000000000000001',
          runStart
        );
        const prematureAcknowledgement = await waitForBackgroundEvent(
          session,
          event => event.kind === 'commandRejected' && event.commandSequence === '0000000000000001'
        );
        expect(prematureAcknowledgement.rejectionDetail).toMatch(/checkpoint publication/i);
        expect(session.health()).toMatchObject({
          worldEpoch: sourceWorldEpoch,
          generationPersistenceAcknowledged: false
        });

        const generationOperationId = '71717171717171717171717171717171';
        session.submitGenerationCheckpoint('0000000000000002', {
          managedDirectory: paths.managedRoot,
          operationId: generationOperationId
        });
        const checkpointEvent = await waitForBackgroundEvent(
          session,
          event => event.kind === 'generationCheckpointPublished' &&
            event.commandSequence === '0000000000000002'
        );
        const firstPublication = parseGenerationPublication(checkpointEvent.checkpoint);
        expect(firstPublication.descriptor).toMatchObject({
          operationId: generationOperationId,
          boundaryKind: 'generation',
          generation: '0000000000000002',
          completedStep: '0000000000000001'
        });
        expect(firstPublication.generationCommit.hallOfFame.successorGenomeId).not.toBe(
          '0000000000000000'
        );

        session.submitGenerationCheckpoint('0000000000000003', {
          managedDirectory: paths.managedRoot,
          operationId: generationOperationId
        });
        const retryEvent = await waitForBackgroundEvent(
          session,
          event => event.kind === 'generationCheckpointPublished' &&
            event.commandSequence === '0000000000000003'
        );
        expect(parseGenerationPublication(retryEvent.checkpoint)).toEqual(firstPublication);
        expect(session.health()).toMatchObject({
          worldEpoch: sourceWorldEpoch,
          generationCheckpointPublished: true,
          generationPersistenceAcknowledged: false
        });

        const mismatchedRoot = firstPublication.descriptor.logicalRootSha256.endsWith('0')
          ? `${firstPublication.descriptor.logicalRootSha256.slice(0, -1)}1`
          : `${firstPublication.descriptor.logicalRootSha256.slice(0, -1)}0`;
        session.submitGenerationPersistenceAcknowledgement('0000000000000004', {
          ...firstPublication.descriptor,
          logicalRootSha256: mismatchedRoot,
          relativeFilename: `${mismatchedRoot}.checkpoint-v3`
        });
        const mismatchedAcknowledgement = await waitForBackgroundEvent(
          session,
          event => event.kind === 'commandRejected' && event.commandSequence === '0000000000000004'
        );
        expect(mismatchedAcknowledgement.rejectionDetail).toMatch(/logical root/i);
        expect(session.health()).toMatchObject({
          worldEpoch: sourceWorldEpoch,
          generationPersistenceAcknowledged: false
        });

        const wrongManagedRoot = join(paths.root, 'background-wrong-managed-root');
        mkdirSync(wrongManagedRoot);
        const failedClient = new CheckpointPersistenceClient({
          databasePath: paths.databasePath,
          managedRootPath: wrongManagedRoot
        });
        clients.push(failedClient);
        await expect(failedClient.commit(
          firstPublication.descriptor,
          firstPublication.generationCommit
        )).rejects.toThrow(/ENOENT|no such file|missing/i);
        await closeClient(failedClient);
        expect(session.health()).toMatchObject({
          worldEpoch: sourceWorldEpoch,
          generation: '0000000000000001',
          completedStep: '0000000000000000',
          generationPersistenceAcknowledged: false
        });
        expect(countMetadataRows(paths.databasePath)).toBe(1);

        const committed = await client.commit(
          firstPublication.descriptor,
          firstPublication.generationCommit
        );
        expect(committed.descriptor).toEqual(firstPublication.descriptor);
        session.submitGenerationPersistenceAcknowledgement(
          '0000000000000005',
          committed.descriptor
        );
        const acknowledged = await waitForBackgroundEvent(
          session,
          event => event.kind === 'generationPersistenceAcknowledged' &&
            event.commandSequence === '0000000000000005'
        );
        expect(acknowledged.acknowledgedOperationId).toBe(generationOperationId);
        expect(session.health()).toMatchObject({
          worldEpoch: sourceWorldEpoch,
          generationPersistenceAcknowledged: true
        });

        session.submitPublishGenerationStart('0000000000000006');
        await expect(waitForBackgroundEvent(
          session,
          event => event.kind === 'commandRejected' && event.commandSequence === '0000000000000006'
        )).resolves.toMatchObject({ kind: 'commandRejected' });
        expect(session.health().worldEpoch).toBe(sourceWorldEpoch);

        session.submitPrepareGenerationReassignments('0000000000000007');
        const assignmentEvent = await waitForBackgroundEvent(
          session,
          event => event.kind === 'generationReassignmentsPrepared' &&
            event.commandSequence === '0000000000000007'
        );
        expect(assignmentEvent.reassignments?.ready).toBe(false);
        expect(assignmentEvent.reassignments?.assignments).toHaveLength(1);
        const assignment = assignmentEvent.reassignments?.assignments[0];
        if (!assignment) throw new Error('background Rust assignment was missing');
        expect(assignment).toMatchObject({
          controllerKind: 'player',
          operationEpoch: firstPublication.descriptor.transitionEpoch
        });
        expect(assignment.resumeToken.length).toBeGreaterThan(0);

        const wrongLeaseId = (BigInt(`0x${assignment.leaseId}`) + 1n)
          .toString(16)
          .padStart(16, '0');
        session.submitGenerationAssignmentReceipt('0000000000000008', {
          operationEpoch: assignment.operationEpoch,
          eventSequence: assignment.eventSequence,
          connectionId: assignment.connectionId,
          leaseId: wrongLeaseId,
          accepted: true
        });
        const ignoredReceipt = await waitForBackgroundEvent(
          session,
          event => event.kind === 'generationAssignmentReceiptsApplied' &&
            event.commandSequence === '0000000000000008'
        );
        expect(ignoredReceipt.receiptResolution).toMatchObject({
          matchedAcceptances: '0000000000000000',
          ignoredReceipts: '0000000000000001',
          state: 'pending',
          remaining: '0000000000000001'
        });
        expect(session.health().worldEpoch).toBe(sourceWorldEpoch);

        session.submitPublishGenerationStart('0000000000000009');
        await expect(waitForBackgroundEvent(
          session,
          event => event.kind === 'commandRejected' && event.commandSequence === '0000000000000009'
        )).resolves.toMatchObject({ kind: 'commandRejected' });
        expect(session.health().worldEpoch).toBe(sourceWorldEpoch);

        session.submitGenerationAssignmentReceipt('000000000000000a', {
          operationEpoch: assignment.operationEpoch,
          eventSequence: assignment.eventSequence,
          connectionId: assignment.connectionId,
          leaseId: assignment.leaseId,
          accepted: true
        });
        const exactReceipt = await waitForBackgroundEvent(
          session,
          event => event.kind === 'generationAssignmentReceiptsApplied' &&
            event.commandSequence === '000000000000000a'
        );
        expect(exactReceipt.receiptResolution).toMatchObject({
          matchedAcceptances: '0000000000000001',
          matchedFailures: '0000000000000000',
          ignoredReceipts: '0000000000000000',
          state: 'ready',
          successorGeneration: '0000000000000002',
          successorCompletedStep: '0000000000000001'
        });
        expect(session.health().worldEpoch).toBe(sourceWorldEpoch);

        session.submitPublishGenerationStart('000000000000000b');
        const finalEvent = await waitForBackgroundEvent(
          session,
          event => event.kind === 'generationStartPublished' &&
            event.commandSequence === '000000000000000b'
        );
        expect(finalEvent.generationStart).toMatchObject({
          publication: {
            generation: '0000000000000002',
            completedStep: '0000000000000001',
            populationEpoch: '0000000000000002',
            externalAssignments: 1
          },
          unavailableControllers: []
        });
        const successorWorldEpoch = finalEvent.generationStart?.publication.worldEpoch;
        expect(successorWorldEpoch).toBeDefined();
        expect(successorWorldEpoch).not.toBe(sourceWorldEpoch);
        await waitForBackgroundHealth(
          session,
          health => health.generation === '0000000000000002' &&
            health.worldEpoch === successorWorldEpoch
        );

        session.submitPublishGenerationStart('000000000000000c');
        await expect(waitForBackgroundEvent(
          session,
          event => event.kind === 'commandRejected' && event.commandSequence === '000000000000000c'
        )).resolves.toMatchObject({ kind: 'commandRejected' });
        // Output can be drained immediately before the coordinator increments
        // its processed counter; wait for that separately published health fact.
        const completedCommands = await waitForBackgroundHealth(
          session,
          health => health.processedCommands === '000000000000000c'
        );
        expect(completedCommands).toMatchObject({
          worldEpoch: successorWorldEpoch,
          generation: '0000000000000002',
          completedStep: '0000000000000001',
          processedCommands: '000000000000000c'
        });

        const ordinary = await waitForBackgroundEvent(session, event => event.kind === 'controllerMessages');
        const messages = ordinary.controllerMessages;
        expect(messages).toHaveLength(1);
        const observation = messages![0]!;
        expect(observation).toMatchObject({ kind: 'observation', snakeId: Number(BigInt(`0x${assignment.frameV1Id}`)) });
        expect(observation.sensors).toHaveLength(83);
        expect(observation.sensors!.every(Number.isFinite)).toBe(true);
        expect(observation.sourceCompletedStep).toBe('0000000000000001');
        expect(session.health().completedStep).toBe('0000000000000001');
        const receipt: RustGenerationAssignmentReceipt = {
          operationEpoch: observation.operationEpoch, eventSequence: observation.eventSequence,
          connectionId: observation.connectionId, leaseId: observation.leaseId, accepted: true
        };
        // Wrong-phase generation receipts must not consume this ordinary observation.
        session.submitGenerationAssignmentReceipt('000000000000000d', receipt);
        await expect(waitForBackgroundEvent(session, event => event.commandSequence === '000000000000000d'))
          .resolves.toMatchObject({ kind: 'commandRejected' });
        session.submitControllerDeliveryReceipt('000000000000000e', { ...receipt, connectionId: 'ffffffffffffffff' });
        await expect(waitForBackgroundEvent(session, event => event.commandSequence === '000000000000000e'))
          .resolves.toMatchObject({ controllerReceiptResolution: {
            matchedAcceptances: '0000000000000000', ignoredReceipts: '0000000000000001', remaining: '0000000000000001'
          } });
        expect(session.health().completedStep).toBe('0000000000000001');
        const sent: unknown[] = [];
        const action: RustBackgroundControllerAction = {
          leaseId: observation.leaseId, connectionId: observation.connectionId,
          turn: 0.75, boost: false, clientTick: '000000000000002a'
        };
        expect(() => session.submitControllerAction('000000000000000f', { ...action, turn: NaN })).toThrow();
        let admitReceipt = false;
        const admission = new BackgroundCommandAdmission({
          submitControllerReclaimReceipt(sequence, completion) {
            if (!admitReceipt) throw new Error('QueueCountLimit: injected reclaim receipt backpressure');
            session.submitControllerReclaimReceipt(sequence, completion);
          },
          submitControllerDeliveryReceipt(sequence, completion) {
            if (!admitReceipt) throw new Error('QueueCountLimit: injected transport receipt backpressure');
            session.submitControllerDeliveryReceipt(sequence, completion);
          }
        }, 15n);
        expect(admission.trySubmitControl(sequence => session.submitControllerAction(sequence, action))).toBe(true);
        const router = new ControllerDeliveryRouter(1, {
          send(connectionId, message) { sent.push({ connectionId, message }); return true; },
          nextSequence() { return admission.nextSequence(); },
          trySubmitReceipt(sequence, completion) {
            return admission.trySubmitReceipt(sequence, completion);
          }
        });
        expect(router.deliver(messages!)).toBe(true);
        expect(router.blocked).toBe(true);
        expect(router.flushReceipts()).toBe(false);
        expect(session.health().completedStep).toBe('0000000000000001');
        admitReceipt = true;
        expect(router.flushReceipts()).toBe(true);
        expect(sent).toEqual([{ connectionId: observation.connectionId, message: {
          type: 'sensors', tick: 1, snakeId: observation.snakeId, sensors: observation.sensors,
          meta: { x: observation.x, y: observation.y, dir: observation.direction }
        } }]);
        await expect(waitForBackgroundEvent(session, event => event.commandSequence === '0000000000000010'))
          .resolves.toMatchObject({ controllerReceiptResolution: {
            matchedAcceptances: '0000000000000001', remaining: '0000000000000000', publishedCompletedStep: '0000000000000002'
          } });
        await expect(waitForBackgroundEvent(session, event => event.commandSequence === '000000000000000f'))
          .resolves.toMatchObject({ kind: 'controllerActionApplied', controllerActionCompletedStep: '0000000000000002' });
        await waitForBackgroundHealth(session, health => health.completedStep === '0000000000000002');

        const nextOrdinary = await waitForBackgroundEvent(session, event => event.kind === 'controllerMessages');
        const nextObservation = nextOrdinary.controllerMessages![0]!;
        expect(nextObservation.sourceCompletedStep).toBe('0000000000000002');
        const close = { leaseId: nextObservation.leaseId, connectionId: nextObservation.connectionId };
        expect(admission.trySubmitControl(sequence => session.submitControllerDisconnect(sequence, close))).toBe(true);
        expect(session.health().completedStep).toBe('0000000000000002');
        expect(admission.trySubmitReceipt(admission.nextSequence(), {
          operationEpoch: nextObservation.operationEpoch, eventSequence: nextObservation.eventSequence,
          ...close, accepted: true
        })).toBe(true);
        await expect(waitForBackgroundEvent(session, event => event.commandSequence === '0000000000000011'))
          .resolves.toMatchObject({ kind: 'controllerDisconnected', controllerDisconnect: {
            leaseId: close.leaseId, completedStep: '0000000000000003', applied: true
          } });
        expect(admission.trySubmitControl(sequence => session.submitControllerDisconnect(sequence, close))).toBe(true);
        await expect(waitForBackgroundEvent(session, event => event.commandSequence === '0000000000000013'))
          .resolves.toMatchObject({ kind: 'controllerDisconnected', controllerDisconnect: { applied: false } });
        await waitForBackgroundHealth(session, health => BigInt(`0x${health.completedStep}`) >= 3n);

        const reclaimSequence = admission.nextSequence();
        expect(admission.trySubmitControl(sequence => session.submitControllerReclaim(sequence, {
          connectionId: '0000000000000099', controllerKind: observation.controllerKind, resumeToken: assignment.resumeToken
        }))).toBe(true);
        const reclaim = (await waitForBackgroundEvent(session, event => event.kind === 'controllerReclaimAssignment')).controllerReclaimAssignment!;
        expect(reclaim).toMatchObject({ requestSequence: reclaimSequence, leaseId: close.leaseId,
          snakeId: observation.snakeId, connectionId: '0000000000000099' });
        expect(reclaim.resumeToken).not.toBe(assignment.resumeToken);
        const staleActionSequence = admission.nextSequence();
        expect(admission.trySubmitControl(sequence => session.submitControllerAction(sequence, action))).toBe(true);
        const freshActionSequence = admission.nextSequence();
        expect(admission.trySubmitControl(sequence => session.submitControllerAction(sequence, { ...action, connectionId: reclaim.connectionId }))).toBe(true);
        const reclaimSent: unknown[] = [];
        const reclaimRouter = new ReclaimDeliveryRouter({
          send(connectionId, message) { reclaimSent.push({ connectionId, message }); return true; },
          nextSequence() { return admission.nextSequence(); },
          trySubmitReceipt(sequence, completion) { return admission.trySubmitReclaimReceipt(sequence, completion); }
        });
        admitReceipt = false;
        expect(reclaimRouter.deliver(reclaim)).toBe(true);
        expect(reclaimRouter.flushReceipts()).toBe(false);
        expect(session.health().loopState).toBe('controllerReclaimPending');
        expect(session.health().completedStep).toBe(reclaim.completedStep);
        admitReceipt = true;
        expect(reclaimRouter.flushReceipts()).toBe(true);
        expect(reclaimSent).toHaveLength(2);
        await expect(waitForBackgroundEvent(session, event => event.kind === 'controllerReclaimResolved'))
          .resolves.toMatchObject({ controllerReclaimResolution: { requestSequence: reclaimSequence, matched: true, accepted: true } });
        await expect(waitForBackgroundEvent(session, event => event.commandSequence === staleActionSequence))
          .resolves.toMatchObject({ kind: 'commandRejected' });
        await expect(waitForBackgroundEvent(session, event => event.commandSequence === freshActionSequence))
          .resolves.toMatchObject({ kind: 'controllerActionApplied', controllerActionCompletedStep: reclaim.completedStep });
        const reclaimedObservation = (await waitForBackgroundEvent(session, event => event.kind === 'controllerMessages')).controllerMessages![0]!;
        expect(reclaimedObservation).toMatchObject({ connectionId: reclaim.connectionId, leaseId: reclaim.leaseId, snakeId: reclaim.snakeId });
        expect(admission.trySubmitReclaimReceipt(admission.nextSequence(), { requestSequence: reclaimSequence,
          connectionId: reclaim.connectionId, leaseId: reclaim.leaseId, accepted: true })).toBe(true);
        await expect(waitForBackgroundEvent(session, event => event.kind === 'controllerReclaimResolved'))
          .resolves.toMatchObject({ controllerReclaimResolution: { matched: false, accepted: false } });
        expect(session.health().completedStep).toBe(reclaimedObservation.sourceCompletedStep);
        expect(admission.trySubmitControl(sequence => session.submitControllerDisconnect(sequence, {
          leaseId: reclaim.leaseId, connectionId: reclaim.connectionId
        }))).toBe(true);
        expect(() => session.submitControllerReclaim(admission.nextSequence(), {
          connectionId: '000000000000009a', controllerKind: observation.controllerKind, identityKey: 'x'.repeat(129)
        })).toThrow();
        const invalidTokenSequence = admission.nextSequence();
        expect(admission.trySubmitControl(sequence => session.submitControllerReclaim(sequence, {
          connectionId: '000000000000009a', controllerKind: observation.controllerKind,
          identityKey: 'player:background-fixture', resumeToken: 'invalid-explicit-token'
        }))).toBe(true);
        const legacySequence = admission.nextSequence();
        expect(admission.trySubmitControl(sequence => session.submitControllerReclaim(sequence, {
          connectionId: '000000000000009a', controllerKind: observation.controllerKind, identityKey: 'player:background-fixture'
        }))).toBe(true);
        expect(router.deliver([reclaimedObservation])).toBe(true);
        await expect(waitForBackgroundEvent(session, event => event.commandSequence === invalidTokenSequence))
          .resolves.toMatchObject({ kind: 'commandRejected' });
        const legacyEvent = await waitForBackgroundEvent(session, event => event.commandSequence === legacySequence);
        expect(legacyEvent.kind).toBe('controllerReclaimAssignment');
        const legacy = legacyEvent.controllerReclaimAssignment!;
        expect(legacy).toMatchObject({ leaseId: reclaim.leaseId, snakeId: reclaim.snakeId, connectionId: '000000000000009a' });
        expect(legacy.resumeToken).not.toBe(reclaim.resumeToken);
        expect(reclaimRouter.deliver(legacy)).toBe(true);
        await expect(waitForBackgroundEvent(session, event => event.kind === 'controllerReclaimResolved'))
          .resolves.toMatchObject({ controllerReclaimResolution: { requestSequence: legacySequence, matched: true, accepted: true } });

        await closeClient(client);
        expect(readCurrentPointer(paths.databasePath, runStart.runId)).toEqual({
          checkpoint_id: firstPublication.descriptor.logicalRootSha256,
          transition_epoch: firstPublication.descriptor.transitionEpoch,
          operation_id: firstPublication.descriptor.operationId
        });
        expect(countMetadataRows(paths.databasePath)).toBe(2);
        expect(readdirSync(paths.managedRoot).sort()).toEqual([
          firstPublication.descriptor.relativeFilename,
          runStart.relativeFilename
        ].sort());
        const records = readGenerationRecords(
          paths.databasePath,
          firstPublication.descriptor.logicalRootSha256
        );
        expect(records.summary).toEqual(encodeExpectedSummary(firstPublication.generationCommit));
        expect(records.hallOfFame).toEqual(
          encodeExpectedHallOfFame(firstPublication.generationCommit)
        );
        expect(wakeCount).toBeGreaterThan(0);

        session.requestStop();
        await session.join();
        joined = true;
        expect(session.health()).toMatchObject({ lifecycle: 'stopped' });
      } finally {
        if (!joined) {
          try {
            session.requestStop();
            await session.join();
          } catch {
            // Preserve the original assertion or native failure.
          }
        }
      }
    }
  );
});
