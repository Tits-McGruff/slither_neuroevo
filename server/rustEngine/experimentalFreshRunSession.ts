/**
 * Explicit Stage 6A prerequisite for one real fixed-P0 Rust fresh run.
 *
 * This module is deliberately not imported by normal server startup. Rust
 * constructs and retains all authoritative state; Node supplies only fresh-run
 * identity inputs, a managed directory, and a persistence operation token.
 */

import {
  validateExperimentalEngineBinding,
  type ExperimentalEngineNativeBinding
} from './experimentalNativeBridge.ts';
import { computeNativeSourceIdentity, type NativeSourceIdentity } from './nativeSourceIdentity.ts';
import type { ManagedCheckpointCommitResult } from './checkpointPersistenceClient.ts';
import type {
  CheckpointOperationId,
  ManagedCheckpointDescriptor,
  U64Hex
} from './checkpointPersistenceProtocol.ts';
import {
  RunStartPersistenceHandoff,
  type RunStartCheckpointCommitter,
  type RustRunStartCheckpointPublishOptions,
  type RustRunStartPersistencePort
} from './runStartPersistenceHandoff.ts';

/** Maximum exact unsigned 64-bit input used by the native memory ceiling. */
const U64_MAX = (1n << 64n) - 1n;
/** Fixed lowercase unsigned-64-bit wire representation. */
const U64_HEX = /^[0-9a-f]{16}$/u;
/** Maximum UTF-8 bytes accepted by the Rust fresh-run identity contract. */
const MAX_RUN_ID_UTF8_BYTES = 256;
/** Maximum retained diagnostic bytes exposed by a permanently faulted session. */
const MAX_FAULT_DETAIL_UTF8_BYTES = 512;
/** Exact native class prototype for this isolated experimental prerequisite. */
const REQUIRED_FRESH_RUN_METHODS = [
  'acknowledgeRunStartPersistence',
  'activateRunningAuthority',
  'constructor',
  'initialize',
  'publishRunStartCheckpoint',
  'snapshot'
] as const;
/** Instance methods required after native construction. */
const REQUIRED_FRESH_RUN_HANDLE_METHODS = [
  'acknowledgeRunStartPersistence',
  'activateRunningAuthority',
  'initialize',
  'publishRunStartCheckpoint',
  'snapshot'
] as const;

/** Stable native lifecycle phases exposed without copying authority. */
export type ExperimentalFreshRunPhase =
  | 'created'
  | 'initializing'
  | 'pendingDurability'
  | 'publishingCheckpoint'
  | 'acknowledgingPersistence'
  | 'awaitingPersistence'
  | 'durableBoundary'
  | 'activating'
  | 'running'
  | 'faulted';

/** Bounded scalar view of the retained Rust transition. */
export interface ExperimentalFreshRunSnapshot {
  /** Stable current session phase. */
  phase: ExperimentalFreshRunPhase;
  /** Exact process-local transition token after initialization. */
  transitionEpoch: U64Hex | undefined;
  /** Exact generation after initialization. */
  generation: U64Hex | undefined;
  /** Exact completed-step count after initialization. */
  completedStep: U64Hex | undefined;
  /** Whether the immutable file has published. */
  checkpointPublished: boolean | undefined;
  /** Whether Rust retained the exact SQLite acknowledgement. */
  persistenceAcknowledged: boolean | undefined;
  /** Whether running authority has published. */
  authorityPublished: boolean | undefined;
  /** Exact authoritative snake count after initialization. */
  snakeCount: U64Hex | undefined;
  /** Exact authoritative pellet count after initialization. */
  pelletCount: U64Hex | undefined;
  /** First bounded panic diagnostic when Rust permanently faults the session. */
  faultDetail: string | undefined;
}

/** Scalar-only result of the one durable-boundary-to-running activation. */
export interface ExperimentalFreshRunPublication {
  /** Exact Rust world incarnation. */
  worldEpoch: U64Hex;
  /** Exact first running generation. */
  generation: U64Hex;
  /** Exact run-start completed-step count. */
  completedStep: U64Hex;
  /** Exact first population epoch. */
  populationEpoch: U64Hex;
}

/** Native session handle admitted only after the production-addon handshake. */
export interface ExperimentalFreshRunNativeHandle extends RustRunStartPersistencePort {
  /** Publish the Rust-owned immutable checkpoint descriptor. */
  publishRunStartCheckpoint(options: RustRunStartCheckpointPublishOptions): Promise<unknown>;
  /** Apply only the worker's exact committed descriptor. */
  acknowledgeRunStartPersistence(descriptor: ManagedCheckpointDescriptor): void;
  /** Construct and admit the complete fixed P0 boundary off the Node loop. */
  initialize(): Promise<unknown>;
  /** Construct and publish the running world off the Node loop. */
  activateRunningAuthority(): Promise<unknown>;
  /** Read bounded scalar state without copying a world or population. */
  snapshot(): unknown;
}

/** Production addon exports plus the explicit experimental fresh-run class. */
export interface ExperimentalFreshRunNativeBinding extends ExperimentalEngineNativeBinding {
  /** Constructor accepting exact string encodings for all numeric inputs. */
  ExperimentalStage6aFreshRunSession: new (
    runId: string,
    seedHex: string,
    memoryCeilingBytesHex: string
  ) => ExperimentalFreshRunNativeHandle;
}

/** Dependencies for constructing one isolated experimental fresh-run owner. */
export interface CreateExperimentalFreshRunSessionOptions {
  /** Loaded addon exports. */
  binding: unknown;
  /** Independently computed identity of the current native source tree. */
  sourceIdentity: NativeSourceIdentity;
  /** Bounded opaque run/lineage identity. */
  runId: string;
  /** Exact normalized Uint32 simulation seed. */
  seed: number;
  /** Positive hard state-memory ceiling. */
  memoryCeilingBytes: bigint;
  /** Dedicated worker-backed SQLite committer. */
  persistence: RunStartCheckpointCommitter;
  /** Server-controlled directory receiving immutable files. */
  managedDirectory: string;
}

/** Real-addon loader dependencies for an explicitly requested session. */
export interface LoadExperimentalFreshRunSessionOptions
  extends Omit<CreateExperimentalFreshRunSessionOptions, 'binding' | 'sourceIdentity'> {
  /** Directory containing the native manifest and source tree. */
  nativeManifestDirectory: string;
  /** Explicit addon loader; normal startup never calls it in this slice. */
  loadBinding(): unknown | Promise<unknown>;
}

/**
 * Validate source/build identity and the exact coarse fresh-run class surface.
 * @param candidate - Unknown loaded addon exports.
 * @param sourceIdentity - Current source identity computed outside the addon.
 * @returns Strict production-addon binding.
 */
export function validateExperimentalFreshRunBinding(
  candidate: unknown,
  sourceIdentity: NativeSourceIdentity
): ExperimentalFreshRunNativeBinding {
  validateExperimentalEngineBinding(candidate, sourceIdentity);
  const binding = candidate as Partial<ExperimentalFreshRunNativeBinding>;
  if (typeof binding.ExperimentalStage6aFreshRunSession !== 'function') {
    throw new TypeError(
      'Experimental native addon is missing ExperimentalStage6aFreshRunSession. ' +
      'Run `npm --prefix native run build` from the repository root.'
    );
  }
  const prototype = binding.ExperimentalStage6aFreshRunSession.prototype as unknown;
  if (prototype === null || typeof prototype !== 'object') {
    throw new TypeError('ExperimentalStage6aFreshRunSession has no class prototype');
  }
  const methods = collectPrototypeMethods(prototype);
  if (methods.length !== REQUIRED_FRESH_RUN_METHODS.length ||
    REQUIRED_FRESH_RUN_METHODS.some((method, index) => methods[index] !== method)) {
    throw new TypeError(
      `ExperimentalStage6aFreshRunSession has an unsupported surface: ${methods.join(', ')}`
    );
  }
  if (methods.some(method => /(?:snake|layer|fixed.?step|world.?step|neural.?step)/iu.test(method))) {
    throw new TypeError('ExperimentalStage6aFreshRunSession exposes prohibited fine-grained controls');
  }
  return binding as ExperimentalFreshRunNativeBinding;
}

/** Collect the complete callable class surface through any test adapter prototype. */
function collectPrototypeMethods(prototype: object): string[] {
  const methods = new Set<string>();
  let current: object | null = prototype;
  while (current !== null && current !== Object.prototype) {
    for (const name of Object.getOwnPropertyNames(current)) {
      const descriptor = Object.getOwnPropertyDescriptor(current, name);
      if (name === 'constructor' || typeof descriptor?.value === 'function') {
        methods.add(name);
      }
    }
    current = Object.getPrototypeOf(current) as object | null;
  }
  return [...methods].sort();
}

/**
 * Create one identity-checked session without changing normal server startup.
 * @param options - Exact addon, fresh-run inputs, persistence port, and managed root.
 * @returns Rust-owned experimental session.
 */
export function createExperimentalFreshRunSession(
  options: CreateExperimentalFreshRunSessionOptions
): ExperimentalFreshRunSession {
  const binding = validateExperimentalFreshRunBinding(options.binding, options.sourceIdentity);
  return new ExperimentalFreshRunSession(binding, options);
}

/**
 * Compute current-tree identity, explicitly load the addon, and create a session.
 * @param options - Loader and fresh-run dependencies.
 * @returns Rust-owned experimental session.
 */
export async function loadExperimentalFreshRunSession(
  options: LoadExperimentalFreshRunSessionOptions
): Promise<ExperimentalFreshRunSession> {
  const sourceIdentity = computeNativeSourceIdentity(options.nativeManifestDirectory);
  const binding = await options.loadBinding();
  return createExperimentalFreshRunSession({ ...options, binding, sourceIdentity });
}

/**
 * Thin Node composition of the native owner and existing persistence handoff.
 *
 * No population, world, controller, generation statistic, slot, or genome ID
 * can be supplied through this class.
 */
export class ExperimentalFreshRunSession {
  /** Native owner retaining the only fresh-run transition and authority. */
  private readonly native: ExperimentalFreshRunNativeHandle;
  /** Exact Rust-to-worker-to-Rust durability handoff. */
  private readonly persistenceHandoff: RunStartPersistenceHandoff;

  /**
   * Construct only from a fully identity-validated production addon.
   * @param binding - Validated native binding.
   * @param options - Bounded identity and persistence dependencies.
   */
  public constructor(
    binding: ExperimentalFreshRunNativeBinding,
    options: Omit<CreateExperimentalFreshRunSessionOptions, 'binding' | 'sourceIdentity'>
  ) {
    const runId = validateRunId(options.runId);
    const seedHex = encodeSeed(options.seed);
    const memoryCeilingBytesHex = encodePositiveU64(
      options.memoryCeilingBytes,
      'memoryCeilingBytes'
    );
    const managedDirectory = validateManagedDirectory(options.managedDirectory);
    this.native = validateFreshRunHandle(new binding.ExperimentalStage6aFreshRunSession(
      runId,
      seedHex,
      memoryCeilingBytesHex
    ));
    this.persistenceHandoff = new RunStartPersistenceHandoff({
      rust: this.native,
      persistence: options.persistence,
      managedDirectory
    });
  }

  /** Construct and admit the real fixed-P0 Rust boundary off-loop. */
  public async initialize(): Promise<ExperimentalFreshRunSnapshot> {
    return parseFreshRunSnapshot(await this.native.initialize());
  }

  /** Commit and acknowledge only Rust's exact pending run-start descriptor. */
  public commitPendingRunStart(
    operationId: CheckpointOperationId
  ): Promise<ManagedCheckpointCommitResult> {
    return this.persistenceHandoff.commitPendingRunStart(operationId);
  }

  /** Activate the retained Rust authority only after exact durability. */
  public async activateRunningAuthority(): Promise<ExperimentalFreshRunPublication> {
    return parseFreshRunPublication(await this.native.activateRunningAuthority());
  }

  /** Read only the native session's bounded scalar proof. */
  public snapshot(): ExperimentalFreshRunSnapshot {
    return parseFreshRunSnapshot(this.native.snapshot());
  }
}

/** Validate the native instance exposes exactly the required coarse operations. */
function validateFreshRunHandle(value: unknown): ExperimentalFreshRunNativeHandle {
  if (value === null || typeof value !== 'object') {
    throw new TypeError('ExperimentalStage6aFreshRunSession constructor returned no handle');
  }
  const handle = value as Partial<ExperimentalFreshRunNativeHandle>;
  for (const method of REQUIRED_FRESH_RUN_HANDLE_METHODS) {
    if (typeof handle[method] !== 'function') {
      throw new TypeError(`experimental fresh-run handle is missing ${method}`);
    }
  }
  return handle as ExperimentalFreshRunNativeHandle;
}

/** Validate and preserve one opaque well-formed UTF-16/UTF-8 run identity. */
function validateRunId(value: unknown): string {
  if (typeof value !== 'string' || value.length === 0 || value.includes('\0')) {
    throw new TypeError('runId must be a nonempty NUL-free string');
  }
  if (!isWellFormedUtf16(value)) {
    throw new TypeError('runId must be well-formed UTF-16');
  }
  if (Buffer.byteLength(value, 'utf8') > MAX_RUN_ID_UTF8_BYTES) {
    throw new RangeError(`runId exceeds ${MAX_RUN_ID_UTF8_BYTES} UTF-8 bytes`);
  }
  return value;
}

/** Validate the controlled path before N-API can reinterpret malformed UTF-16. */
function validateManagedDirectory(value: unknown): string {
  if (typeof value !== 'string' || value.length === 0 || value.includes('\0')) {
    throw new TypeError('managedDirectory must be a nonempty NUL-free string');
  }
  if (!isWellFormedUtf16(value)) {
    throw new TypeError('managedDirectory must be well-formed UTF-16');
  }
  if (Buffer.byteLength(value, 'utf8') > 32_768) {
    throw new RangeError('managedDirectory exceeds 32768 UTF-8 bytes');
  }
  return value;
}

/** Check every UTF-16 surrogate belongs to one valid pair. */
function isWellFormedUtf16(value: string): boolean {
  for (let index = 0; index < value.length; index++) {
    const unit = value.charCodeAt(index);
    if (unit >= 0xd800 && unit <= 0xdbff) {
      if (index + 1 >= value.length) return false;
      const next = value.charCodeAt(index + 1);
      if (next < 0xdc00 || next > 0xdfff) return false;
      index += 1;
    } else if (unit >= 0xdc00 && unit <= 0xdfff) {
      return false;
    }
  }
  return true;
}

/** Encode one normalized Uint32 seed as exact canonical lowercase hex. */
function encodeSeed(value: unknown): string {
  if (typeof value !== 'number' || !Number.isInteger(value) || value < 0 || value > 0xffff_ffff) {
    throw new RangeError('seed must be an unsigned 32-bit integer');
  }
  return value.toString(16).padStart(8, '0');
}

/** Encode one positive unsigned-64-bit bigint without Number narrowing. */
function encodePositiveU64(value: unknown, field: string): U64Hex {
  if (typeof value !== 'bigint' || value <= 0n || value > U64_MAX) {
    throw new RangeError(`${field} must be a positive unsigned 64-bit bigint`);
  }
  return value.toString(16).padStart(16, '0');
}

/** Require an ordinary object at a scalar native-output boundary. */
function asRecord(value: unknown, field: string): Record<string, unknown> {
  if (value === null || typeof value !== 'object' || Array.isArray(value) ||
    value instanceof ArrayBuffer || ArrayBuffer.isView(value)) {
    throw new TypeError(`${field} must be a scalar object`);
  }
  return value as Record<string, unknown>;
}

/** Reject native scalar output that grows beyond its named bounded fields. */
function requireOnlyKeys(value: Record<string, unknown>, allowed: readonly string[]): void {
  const allowedKeys = new Set(allowed);
  for (const key of Object.keys(value)) {
    if (!allowedKeys.has(key)) {
      throw new TypeError(`experimental fresh-run scalar output contains unknown field ${key}`);
    }
  }
}

/** Parse one exact unsigned-64-bit native scalar without converting to Number. */
function parseU64Hex(value: unknown, field: string): U64Hex {
  if (typeof value !== 'string' || !U64_HEX.test(value)) {
    throw new TypeError(`${field} must be 16 lowercase hexadecimal digits`);
  }
  return value;
}

/** Parse one optional exact native scalar. */
function parseOptionalU64Hex(value: unknown, field: string): U64Hex | undefined {
  return value === undefined ? undefined : parseU64Hex(value, field);
}

/** Parse one optional native boolean. */
function parseOptionalBoolean(value: unknown, field: string): boolean | undefined {
  if (value === undefined) return undefined;
  if (typeof value !== 'boolean') throw new TypeError(`${field} must be boolean or undefined`);
  return value;
}

/** Parse the one bounded terminal panic diagnostic without accepting malformed text. */
function parseOptionalFaultDetail(value: unknown): string | undefined {
  if (value === undefined) return undefined;
  if (typeof value !== 'string' || value.length === 0) {
    throw new TypeError('faultDetail must be a nonempty string or undefined');
  }
  if (!isWellFormedUtf16(value)) {
    throw new TypeError('faultDetail must be well-formed UTF-16');
  }
  if (Buffer.byteLength(value, 'utf8') > MAX_FAULT_DETAIL_UTF8_BYTES) {
    throw new RangeError(`faultDetail exceeds ${MAX_FAULT_DETAIL_UTF8_BYTES} UTF-8 bytes`);
  }
  return value;
}

/** Validate the scalar-only native session snapshot and its absence invariants. */
function parseFreshRunSnapshot(value: unknown): ExperimentalFreshRunSnapshot {
  const raw = asRecord(value, 'experimental fresh-run snapshot');
  requireOnlyKeys(raw, [
    'phase',
    'transitionEpoch',
    'generation',
    'completedStep',
    'checkpointPublished',
    'persistenceAcknowledged',
    'authorityPublished',
    'snakeCount',
    'pelletCount',
    'faultDetail'
  ]);
  const phases: readonly ExperimentalFreshRunPhase[] = [
    'created', 'initializing', 'pendingDurability', 'publishingCheckpoint',
    'acknowledgingPersistence', 'awaitingPersistence', 'durableBoundary',
    'activating', 'running', 'faulted'
  ];
  if (typeof raw['phase'] !== 'string' ||
    !phases.includes(raw['phase'] as ExperimentalFreshRunPhase)) {
    throw new TypeError('experimental fresh-run snapshot has an unknown phase');
  }
  const snapshot: ExperimentalFreshRunSnapshot = {
    phase: raw['phase'] as ExperimentalFreshRunPhase,
    transitionEpoch: parseOptionalU64Hex(raw['transitionEpoch'], 'transitionEpoch'),
    generation: parseOptionalU64Hex(raw['generation'], 'generation'),
    completedStep: parseOptionalU64Hex(raw['completedStep'], 'completedStep'),
    checkpointPublished: parseOptionalBoolean(raw['checkpointPublished'], 'checkpointPublished'),
    persistenceAcknowledged: parseOptionalBoolean(
      raw['persistenceAcknowledged'],
      'persistenceAcknowledged'
    ),
    authorityPublished: parseOptionalBoolean(raw['authorityPublished'], 'authorityPublished'),
    snakeCount: parseOptionalU64Hex(raw['snakeCount'], 'snakeCount'),
    pelletCount: parseOptionalU64Hex(raw['pelletCount'], 'pelletCount'),
    faultDetail: parseOptionalFaultDetail(raw['faultDetail'])
  };
  const metadata = [
    snapshot.generation,
    snapshot.completedStep,
    snapshot.checkpointPublished,
    snapshot.persistenceAcknowledged,
    snapshot.authorityPublished,
    snapshot.snakeCount,
    snapshot.pelletCount
  ];
  if (snapshot.transitionEpoch === undefined && metadata.some(item => item !== undefined)) {
    throw new TypeError('experimental fresh-run snapshot has metadata without a transition');
  }
  if (snapshot.transitionEpoch !== undefined && metadata.some(item => item === undefined)) {
    throw new TypeError('experimental fresh-run snapshot omits retained transition metadata');
  }
  if (snapshot.transitionEpoch === '0000000000000000') {
    throw new TypeError('experimental fresh-run snapshot has a zero transition epoch');
  }
  if (snapshot.transitionEpoch !== undefined &&
    (snapshot.generation !== '0000000000000001' ||
      snapshot.completedStep !== '0000000000000000')) {
    throw new TypeError('experimental fresh-run snapshot is not generation one at step zero');
  }
  if (snapshot.phase === 'faulted') {
    if (snapshot.faultDetail === undefined || snapshot.transitionEpoch !== undefined) {
      throw new TypeError('faulted experimental fresh-run snapshot must contain only fault detail');
    }
  } else if (snapshot.faultDetail !== undefined) {
    throw new TypeError('nonfaulted experimental fresh-run snapshot contains fault detail');
  }
  if (snapshot.transitionEpoch === undefined &&
    ![
      'created', 'initializing', 'publishingCheckpoint',
      'acknowledgingPersistence', 'activating', 'faulted'
    ]
      .includes(snapshot.phase)) {
    throw new TypeError('experimental fresh-run stable phase omits retained transition metadata');
  }
  if (snapshot.transitionEpoch !== undefined &&
    (snapshot.phase === 'created' || snapshot.phase === 'faulted')) {
    throw new TypeError('experimental fresh-run phase unexpectedly retains transition metadata');
  }
  return snapshot;
}

/** Validate one scalar-only native activation result. */
function parseFreshRunPublication(value: unknown): ExperimentalFreshRunPublication {
  const raw = asRecord(value, 'experimental fresh-run publication');
  requireOnlyKeys(raw, ['worldEpoch', 'generation', 'completedStep', 'populationEpoch']);
  const publication: ExperimentalFreshRunPublication = {
    worldEpoch: parseU64Hex(raw['worldEpoch'], 'worldEpoch'),
    generation: parseU64Hex(raw['generation'], 'generation'),
    completedStep: parseU64Hex(raw['completedStep'], 'completedStep'),
    populationEpoch: parseU64Hex(raw['populationEpoch'], 'populationEpoch')
  };
  if (publication.generation !== '0000000000000001' ||
    publication.completedStep !== '0000000000000000') {
    throw new TypeError('experimental fresh-run publication is not generation one at step zero');
  }
  if (publication.worldEpoch === '0000000000000000' ||
    publication.populationEpoch !== '0000000000000001') {
    throw new TypeError('experimental fresh-run publication has invalid authority epochs');
  }
  return publication;
}
