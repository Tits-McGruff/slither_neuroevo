import type { RecoveryBranchCommit, RecoveryBranchResult, RecoveryScanCursor, RecoveryScanResult } from './recoveryProtocol.ts';
import type { CheckpointPruneResult, CheckpointRetentionInventory } from './checkpointRetention.ts';
import type { GraphSpec } from '../../src/brains/graph/schema.ts';
/** Descriptor protocol for the isolated Stage 3 checkpoint metadata worker. */

/** Protocol version understood by the checkpoint persistence worker. */
export const MANAGED_CHECKPOINT_DESCRIPTOR_PROTOCOL_VERSION = 1;

/** Fixed lowercase hexadecimal representation of an unsigned 64-bit value. */
export type U64Hex = string;

/** Exact lowercase hexadecimal representation of one IEEE-754 Float64 bit pattern. */
export type F64Hex = string;

/** Fixed lowercase hexadecimal operation token, independent of the run identity. */
export type CheckpointOperationId = string;

/** Small graph-preset list row stored beside Rust checkpoint metadata. */
export interface ManagedGraphPresetMeta {
  /** SQLite row identity. */
  id: number;
  /** User-visible bounded name. */
  name: string;
  /** Creation time in milliseconds since the Unix epoch. */
  createdAt: number;
}

/** Complete bounded graph preset returned only on an explicit load. */
export interface ManagedGraphPreset extends ManagedGraphPresetMeta {
  /** Independently validated current graph definition. */
  spec: GraphSpec;
}

/** Immutable checkpoint boundary kinds supported by the Stage 3 bridge. */
export type ManagedCheckpointBoundaryKind = 'run-start' | 'generation';

/** Packed numeric encodings selected by the Rust checkpoint writer. */
export type ManagedCheckpointNumericEncoding =
  | 'raw-f32le-v1'
  | 'f32le-shuffle4-zstd-v1';

/** Single-pass publication policy selected for automatic Stage 3 checkpoints. */
export type ManagedCheckpointWriteValidationPolicy =
  | 'write-hash-count-fsync-rename-v1';

/**
 * Bounded descriptor facts admitted by one isolated persistence worker.
 *
 * These are caller-supplied bootstrap options so later measured runtime limits can change
 * without changing the descriptor wire contract. Every integer remains exact on the wire.
 */
export interface ManagedCheckpointDescriptorLimits {
  /** Largest final managed file length. */
  maxStoredByteCount: U64Hex;
  /** Largest decoded logical checkpoint length. */
  maxDecodedByteCount: U64Hex;
  /** Largest population slot count. */
  maxPopulationCount: U64Hex;
  /** Largest packed weight scalar count per population slot. */
  maxWeightsPerGenome: U64Hex;
  /** Largest aggregate recurrent-state scalar count. */
  maxRecurrentStateCount: U64Hex;
  /** Largest number of logical checkpoint roles. */
  maxRoleCount: U64Hex;
}

/**
 * Provisional Stage 3 envelope pending the measured runtime-limit configuration surface.
 *
 * The 32-role ceiling is only a temporary structural envelope for the minimal descriptor;
 * it is not an owner-selected retention or checkpoint-content policy.
 */
export const DEFAULT_MANAGED_CHECKPOINT_DESCRIPTOR_LIMITS: ManagedCheckpointDescriptorLimits = {
  maxStoredByteCount: '0000000100000000',
  maxDecodedByteCount: '0000000200000000',
  maxPopulationCount: '0000000000002710',
  maxWeightsPerGenome: '00000000001e8480',
  maxRecurrentStateCount: '00000004a817c800',
  maxRoleCount: '0000000000000020'
};

/**
 * Small, structured-clone-safe facts about an already-published immutable checkpoint file.
 *
 * No checkpoint bytes, archive contents, world state, or population data may be added here.
 * Every field representing a potentially large integer is a canonical unsigned-64-bit hex
 * string, preserving Rust values without a JavaScript-number conversion.
 */
export interface ManagedCheckpointDescriptor {
  /** Descriptor protocol revision. */
  protocolVersion: 1;
  /** Exact correlated operation token. */
  operationId: CheckpointOperationId;
  /** Exact engine transition epoch that must match the commit acknowledgement. */
  transitionEpoch: U64Hex;
  /** Opaque bounded run identity retained as text without numeric coercion. */
  runId: string;
  /** Exact generation represented by the checkpoint. */
  generation: U64Hex;
  /** Exact completed fixed-step count represented by the checkpoint. */
  completedStep: U64Hex;
  /** Generation boundary represented by the checkpoint. */
  boundaryKind: ManagedCheckpointBoundaryKind;
  /** Checkpoint container format revision. */
  checkpointFormatVersion: U64Hex;
  /** Restorable engine-state schema revision. */
  stateVersion: U64Hex;
  /** Graph layout schema revision. */
  graphLayoutVersion: U64Hex;
  /** Controlled logical-root label for the managed checkpoint directory. */
  managedRoot: 'checkpoint-v3';
  /** Digest-derived basename below the controlled managed root. */
  relativeFilename: string;
  /** One encoding-independent SHA-256 logical checkpoint root. */
  logicalRootSha256: string;
  /** Final stored file length in bytes. */
  storedByteCount: U64Hex;
  /** Decoded logical checkpoint length in bytes. */
  decodedByteCount: U64Hex;
  /** Number of logical roles included in the checkpoint root. */
  roleCount: U64Hex;
  /** Population slot count. */
  populationCount: U64Hex;
  /** Total packed weight scalar count. */
  weightCount: U64Hex;
  /** Required recurrent-state scalar count. */
  recurrentStateCount: U64Hex;
  /** Encoding selected for packed weights. */
  weightsEncoding: ManagedCheckpointNumericEncoding;
  /** Encoding selected for recurrent state. */
  recurrentStateEncoding: ManagedCheckpointNumericEncoding;
  /** SHA-256 of the compiled graph layout. */
  graphLayoutSha256: string;
  /** Write validation performed before the file was atomically published. */
  writeValidationPolicy: ManagedCheckpointWriteValidationPolicy;
}

/**
 * Complete compact result retained for every finished generation.
 *
 * Float64 values use their exact IEEE-754 bits rather than JavaScript numbers so the
 * persistence boundary cannot round, stringify, or otherwise reinterpret Rust results.
 */
export interface ManagedGenerationSummary {
  /** Generation whose round just completed. */
  completedGeneration: U64Hex;
  /** Maximum fitness as exact finite Float64 bits. */
  bestF64Hex: F64Hex;
  /** Arithmetic mean fitness as exact finite Float64 bits. */
  averageF64Hex: F64Hex;
  /** Minimum fitness as exact finite Float64 bits. */
  minimumF64Hex: F64Hex;
  /** Greedy RMS-threshold species count. */
  speciesCount: U64Hex;
  /** Largest greedy species bucket. */
  topSpeciesSize: U64Hex;
  /** Mean absolute parameter value as exact finite Float64 bits. */
  averageWeightF64Hex: F64Hex;
  /** Variance of absolute parameter values as exact finite Float64 bits. */
  weightVarianceF64Hex: F64Hex;
}

/**
 * Run-scoped Hall-of-Fame metadata identifying the elite selected for one immutable checkpoint.
 *
 * No genome weights are embedded here. `successorPopulationSlot` and `successorGenomeId`
 * identify the bit-exact elite in the checkpoint, while the same transaction links its
 * independently retained content-addressed weight object.
 */
export interface ManagedHallOfFameReference {
  /** Completed generation that produced the selected genome. */
  completedGeneration: U64Hex;
  /** Stable source population slot before evolution sorting. */
  sourcePopulationSlot: U64Hex;
  /** Stable source snake identity used by current Hall-of-Fame metadata. */
  sourceSnakeId: U64Hex;
  /** Selected fitness as exact finite Float64 bits. */
  fitnessF64Hex: F64Hex;
  /** Selected points score as exact finite Float64 bits. */
  pointsF64Hex: F64Hex;
  /** Selected body-point count. */
  length: U64Hex;
  /** New-population slot containing the exact elite copy. */
  successorPopulationSlot: U64Hex;
  /** Durable lineage identity of that successor elite. */
  successorGenomeId: U64Hex;
}

/** Immutable adaptive binary object containing one Hall-of-Fame genome's weights. */
export interface ManagedHallOfFameWeightsDescriptor {
  /** Descriptor and file contract version. */
  version: 1;
  /** SHA-256 of decoded packed little-endian Float32 bits. */
  logicalSha256: string;
  /** Digest-derived direct child of the controlled checkpoint directory. */
  relativeFilename: string;
  /** Adaptive raw or shuffled-Zstandard encoding. */
  encoding: ManagedCheckpointNumericEncoding;
  /** Exact stored file bytes. */
  storedByteCount: U64Hex;
  /** Exact decoded packed bytes. */
  decodedByteCount: U64Hex;
  /** Exact Float32 parameter count. */
  weightCount: U64Hex;
}

/** Complete small metadata that must commit with one generation checkpoint pointer. */
export interface ManagedGenerationCommit {
  /** Compact eight-field chart/history record. */
  summary: ManagedGenerationSummary;
  /** Run-scoped identity of the selected elite inside the same checkpoint. */
  hallOfFame: ManagedHallOfFameReference;
  /** Independently retained winner weights produced by the same Rust transition. */
  hallOfFameWeights: ManagedHallOfFameWeightsDescriptor;
}

/** Commit request sent from the Node client to its one persistence worker. */
export interface CommitManagedCheckpointRequest {
  /** Message discriminator. */
  type: 'commitManagedCheckpoint';
  /** Descriptor-only checkpoint publication request. */
  descriptor: ManagedCheckpointDescriptor;
  /** Exact small generation metadata, otherwise null for run start. */
  generationCommit: ManagedGenerationCommit | null;
  /** Whether this commit also selects a newly replacing live run. */
  activateRun: boolean;
}

/** Bounded browser-chart projection decoded by the SQLite worker from compact history. */
export interface ManagedBrowserHistoryEntry {
  /** Completed generation, narrowed only within the browser-safe range. */
  gen: number;
  /** Best fitness for the generation. */
  best: number;
  /** Mean fitness for the generation. */
  avg: number;
  /** Minimum fitness for the generation. */
  min: number;
  /** Greedy species count. */
  speciesCount: number;
  /** Largest species bucket. */
  topSpeciesSize: number;
  /** Mean absolute parameter value. */
  avgWeight: number;
  /** Variance of absolute parameter values. */
  weightVariance: number;
}

/** Bounded Hall-of-Fame row shown by the browser without transferring genome weights. */
export interface ManagedBrowserHallOfFameEntry {
  /** Exact run-scoped generation key used by later resurrection requests. */
  entryId: U64Hex;
  /** Completed generation narrowed to the browser-safe range. */
  gen: number;
  /** Retained winner fitness. */
  fitness: number;
  /** Retained winner score. */
  points: number;
  /** Retained winner body length. */
  length: number;
  /** Whether the owner explicitly pinned this historical entry. */
  pinned: boolean;
}

/** Exact selected retained genome descriptor passed to Rust for one resurrection. */
export interface ManagedHallOfFameSelection {
  /** Correlation token for the isolated worker read. */
  operationId: CheckpointOperationId;
  /** Effective run containing the selected inherited or local entry. */
  runId: string;
  /** Exact run-scoped completed-generation key. */
  entryId: U64Hex;
  /** Current active boundary that fixes the compatible graph and weight count. */
  checkpoint: ManagedCheckpointDescriptor;
  /** Compact winner identity and display metadata. */
  reference: ManagedHallOfFameReference;
  /** Verified immutable packed weights consumed directly by Rust. */
  weights: ManagedHallOfFameWeightsDescriptor;
}

/** Atomic import request referencing only Rust-published managed files. */
export interface CommitManagedImportRequest {
  /** Message discriminator. */
  type: 'commitManagedImport';
  /** Exact imported checkpoint descriptor retained by Rust. */
  descriptor: ManagedCheckpointDescriptor;
  /** Trusted fixed-width history and Hall-of-Fame inventory. */
  inventory: ManagedImportInventoryDescriptor;
  /** Fresh owner-selected lineage, or null for an exact-identity import. */
  branchRunId: string | null;
}

/** Orderly client-owned worker shutdown request. */
export interface CheckpointPersistenceShutdownRequest {
  /** Message discriminator. */
  type: 'shutdown';
}

/** Requests accepted by the isolated persistence worker. */
export type CheckpointPersistenceWorkerRequest =
  | { type: 'scanRecoveryCandidate'; operationId: string; cursor: RecoveryScanCursor | null }
  | { type: 'commitRecoveryBranch'; commit: RecoveryBranchCommit }
  | { type: 'inspectCheckpointRetention'; operationId: CheckpointOperationId }
  | { type: 'pinCurrentCheckpoint'; operationId: CheckpointOperationId }
  | { type: 'applyCheckpointRetention'; operationId: CheckpointOperationId }
  | { type: 'acquireCurrentExportLease'; operationId: CheckpointOperationId }
  | { type: 'releaseExportLease'; operationId: CheckpointOperationId }
  | { type: 'readBrowserHistory'; operationId: CheckpointOperationId; runId: string; limit: number }
  | { type: 'readBrowserHallOfFame'; operationId: CheckpointOperationId; runId: string; limit: number }
  | { type: 'saveGraphPreset'; operationId: CheckpointOperationId; name: string; specJson: string }
  | { type: 'listGraphPresets'; operationId: CheckpointOperationId; limit: number }
  | { type: 'loadGraphPreset'; operationId: CheckpointOperationId; presetId: number }
  | { type: 'selectHallOfFameEntry'; operationId: CheckpointOperationId; runId: string; entryId: U64Hex }
  | { type: 'releaseHallOfFameEntry'; operationId: CheckpointOperationId }
  | CommitManagedCheckpointRequest
  | CommitManagedImportRequest
  | SelectManagedCheckpointRequest
  | CheckpointPersistenceShutdownRequest;

/** Bounded current-pointer selection; population bytes stay in managed files. */
export interface SelectManagedCheckpointRequest {
  /** Request discriminator. */
  type: 'selectManagedCheckpoint';
  /** Read correlation, distinct from the checkpoint's original publication token. */
  operationId: CheckpointOperationId;
  /** Exact run, or null when the dedicated database must contain only one current run. */
  runId: string | null;
}

/** Selected source content and its durable effective lineage. */
export interface ManagedCheckpointSelection {
  /** Original immutable checkpoint descriptor, or null for an empty store. */
  descriptor: ManagedCheckpointDescriptor | null;
  /** Effective active run, which differs from source content only at a branch base. */
  runId: string | null;
  /** Durable provenance remains visible after the branch advances. */
  recovery: RecoveryBranchResult | null;
  /** Durable provenance for an owner-selected older-checkpoint import branch. */
  importBranch: ManagedImportBranchResult | null;
}

/** Durable provenance for an older same-run archive resumed under a fresh lineage. */
export interface ManagedImportBranchResult {
  /** Import operation that created the branch. */
  operationId: CheckpointOperationId;
  /** Fresh effective lineage used by the running authority. */
  branchRunId: string;
  /** Original archive lineage whose retained future remains untouched. */
  sourceRunId: string;
  /** Exact source generation selected by the archive. */
  sourceGeneration: U64Hex;
  /** Exact immutable source checkpoint root. */
  sourceCheckpointId: string;
  /** Original immutable descriptor aliased by the branch pointer. */
  recoveredDescriptor: ManagedCheckpointDescriptor;
}

/** Exact immutable checkpoint protected for one direct archive download. */
export interface ManagedCheckpointExportLease {
  /** Worker-owned lease token used for exact release. */
  operationId: CheckpointOperationId;
  /** Effective run identity at acquisition time. */
  runId: string;
  /** Original immutable checkpoint descriptor selected by the current pointer. */
  descriptor: ManagedCheckpointDescriptor;
  /** Bounded worker-written inventory consumed directly by Rust archive composition. */
  inventory: ManagedExportInventoryDescriptor;
}

/** Fixed-width history/Hall-of-Fame inventory spooled for one exact export lease. */
export interface ManagedExportInventoryDescriptor {
  /** Binary inventory contract version. */
  version: 1;
  /** Operation-derived direct child of the controlled managed directory. */
  relativeFilename: string;
  /** SHA-256 of the complete inventory bytes. */
  sha256: string;
  /** Exact final inventory length. */
  storedByteCount: U64Hex;
  /** Number of 56-byte compact history records. */
  historyCount: U64Hex;
  /** Number of Hall-of-Fame records and linked winner objects. */
  hallOfFameCount: U64Hex;
}

/** Rust-validated fixed-width metadata used by one exact import transaction. */
export interface ManagedImportInventoryDescriptor {
  /** Inventory contract version. */
  version: 1;
  /** Operation-derived direct child of the controlled managed directory. */
  relativeFilename: string;
  /** SHA-256 of the complete trusted inventory bytes. */
  sha256: string;
  /** Exact complete inventory length. */
  storedByteCount: U64Hex;
  /** Number of contiguous compact history records. */
  historyCount: U64Hex;
  /** Number of selected unique Hall-of-Fame records. */
  hallOfFameCount: U64Hex;
}

/** One validated metadata selection, without opening or decoding population payloads. */
export interface ManagedCheckpointSelectedResponse extends ManagedCheckpointSelection {
  /** Response discriminator. */
  type: 'managedCheckpointSelected';
  /** Exact read correlation. */
  operationId: CheckpointOperationId;
  /** Exact stored publication descriptor, or null when no current checkpoint exists. */
  descriptor: ManagedCheckpointDescriptor | null;
}

/** Successful matching acknowledgement from the persistence worker. */
export interface ManagedCheckpointCommittedResponse {
  /** Message discriminator. */
  type: 'managedCheckpointCommitted';
  /** Exact operation identifier from the request. */
  operationId: CheckpointOperationId;
  /** Exact transition epoch from the request. */
  transitionEpoch: U64Hex;
  /** Exact run identity whose current pointer changed. */
  runId: string;
  /** Exact content-addressed checkpoint identity selected as current. */
  checkpointId: string;
  /** Complete strictly validated descriptor committed by the worker. */
  descriptor: ManagedCheckpointDescriptor;
}

/** Imported metadata/current-pointer commit echoed to the retained Rust candidate. */
export interface ManagedImportCommittedResponse
  extends Omit<ManagedCheckpointCommittedResponse, 'type'> {
  /** Message discriminator. */
  type: 'managedImportCommitted';
  /** Durable branch provenance, or null for an exact-identity import. */
  importBranch: ManagedImportBranchResult | null;
}

/** Correlated rejection returned without changing an existing current pointer. */
export interface ManagedCheckpointRejectedResponse {
  /** Message discriminator. */
  type: 'managedCheckpointRejected';
  /** Operation identifier when it was safely extractable, otherwise null. */
  operationId: CheckpointOperationId | null;
  /** Plain-language bounded rejection reason. */
  reason: string;
}

/** Worker responses understood by the client. */
export type CheckpointPersistenceWorkerResponse =
  | { type: 'recoveryCandidate'; operationId: string; result: RecoveryScanResult }
  | { type: 'recoveryBranchCommitted'; result: RecoveryBranchResult }
  | { type: 'checkpointRetentionInspected'; operationId: CheckpointOperationId; inventory: CheckpointRetentionInventory }
  | { type: 'currentCheckpointPinned'; operationId: CheckpointOperationId; checkpointId: string; generation: U64Hex }
  | { type: 'checkpointRetentionApplied'; operationId: CheckpointOperationId; result: CheckpointPruneResult }
  | { type: 'currentExportLeaseAcquired'; lease: ManagedCheckpointExportLease }
  | { type: 'exportLeaseReleased'; operationId: CheckpointOperationId }
  | { type: 'browserHistoryRead'; operationId: CheckpointOperationId; runId: string; history: ManagedBrowserHistoryEntry[] }
  | { type: 'browserHallOfFameRead'; operationId: CheckpointOperationId; runId: string; entries: ManagedBrowserHallOfFameEntry[] }
  | { type: 'graphPresetSaved'; operationId: CheckpointOperationId; presetId: number }
  | { type: 'graphPresetsListed'; operationId: CheckpointOperationId; presets: ManagedGraphPresetMeta[] }
  | { type: 'graphPresetLoaded'; operationId: CheckpointOperationId; preset: ManagedGraphPreset | null }
  | { type: 'hallOfFameEntrySelected'; selection: ManagedHallOfFameSelection }
  | { type: 'hallOfFameEntryReleased'; operationId: CheckpointOperationId }
  | ManagedCheckpointCommittedResponse
  | ManagedImportCommittedResponse
  | ManagedCheckpointSelectedResponse
  | ManagedCheckpointRejectedResponse;

/** Strict lowercase SHA-256 digest pattern. */
const SHA256_HEX = /^[0-9a-f]{64}$/u;
/** Strict fixed-width unsigned-64-bit hexadecimal pattern. */
const U64_HEX = /^[0-9a-f]{16}$/u;
/** Strict exact-width IEEE-754 Float64 hexadecimal pattern. */
const F64_HEX = /^[0-9a-f]{16}$/u;
/** Strict fixed-width operation token pattern. */
const OPERATION_ID_HEX = /^[0-9a-f]{32}$/u;
/** Fixed digest-derived filename suffix. */
const CHECKPOINT_FILENAME_SUFFIX = '.checkpoint-v3';
/** Complete ordered descriptor field set used by strict parsing and equality. */
const MANAGED_CHECKPOINT_DESCRIPTOR_KEYS = [
  'protocolVersion', 'operationId', 'transitionEpoch', 'runId', 'generation',
  'completedStep', 'boundaryKind', 'checkpointFormatVersion', 'stateVersion',
  'graphLayoutVersion', 'managedRoot', 'relativeFilename', 'logicalRootSha256',
  'storedByteCount', 'decodedByteCount', 'roleCount', 'populationCount',
  'weightCount', 'recurrentStateCount', 'weightsEncoding', 'recurrentStateEncoding',
  'graphLayoutSha256', 'writeValidationPolicy'
] as const satisfies readonly (keyof ManagedCheckpointDescriptor)[];

/**
 * Raise one descriptor validation failure.
 * @param reason - Stable human-readable rejection detail.
 */
function reject(reason: string): never {
  throw new TypeError(`invalid managed checkpoint descriptor: ${reason}`);
}

/**
 * Check that a candidate is a plain structured-clone object.
 * @param value - Candidate value.
 * @param label - Field label included in rejections.
 * @returns Plain record with unknown-valued fields.
 */
function asRecord(value: unknown, label: string): Record<string, unknown> {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    reject(`${label} must be a plain object`);
  }
  if (value instanceof ArrayBuffer || ArrayBuffer.isView(value)) {
    reject(`${label} must not contain binary payload data`);
  }
  const prototype = Object.getPrototypeOf(value);
  if (prototype !== Object.prototype && prototype !== null) {
    reject(`${label} must be a plain object`);
  }
  return value as Record<string, unknown>;
}

/**
 * Reject unknown descriptor properties, including names that suggest prohibited payloads.
 * @param value - Descriptor record to inspect.
 * @param allowed - Exact allowed property names.
 */
function requireOnlyKeys(value: Record<string, unknown>, allowed: readonly string[]): void {
  const allowedKeys = new Set(allowed);
  for (const key of Object.keys(value)) {
    if (!allowedKeys.has(key)) {
      if (/(archive|population|world|buffer|bytes|payload)/iu.test(key)) {
        reject(`prohibited payload field ${JSON.stringify(key)}`);
      }
      reject(`unknown field ${JSON.stringify(key)}`);
    }
  }
  for (const key of allowed) {
    if (!Object.hasOwn(value, key)) reject(`missing field ${JSON.stringify(key)}`);
  }
}

/**
 * Read one canonical unsigned-64-bit fixed-width hexadecimal string.
 * @param value - Candidate wire value.
 * @param label - Field label included in rejections.
 * @returns Validated wire value.
 */
function asU64Hex(value: unknown, label: string): U64Hex {
  if (typeof value !== 'string' || !U64_HEX.test(value)) {
    reject(`${label} must be a 16-character lowercase unsigned-64-bit hex string`);
  }
  return value;
}

/**
 * Read one exact finite IEEE-754 Float64 bit pattern without converting it to Number.
 * @param value - Candidate wire value.
 * @param label - Field label included in rejections.
 * @returns Validated exact Float64 bits.
 */
function asFiniteF64Hex(value: unknown, label: string): F64Hex {
  if (typeof value !== 'string' || !F64_HEX.test(value)) {
    reject(`${label} must be a 16-character lowercase IEEE-754 Float64 hex string`);
  }
  const bits = BigInt(`0x${value}`);
  if ((bits & 0x7ff0000000000000n) === 0x7ff0000000000000n) {
    reject(`${label} must encode a finite Float64 value`);
  }
  return value;
}

/**
 * Read one canonical operation token without treating it as a run ID or number.
 * @param value - Candidate wire value.
 * @returns Validated operation token.
 */
export function parseCheckpointOperationId(value: unknown): CheckpointOperationId {
  if (typeof value !== 'string' || !OPERATION_ID_HEX.test(value)) {
    reject('operationId must be a 32-character lowercase hexadecimal token');
  }
  return value;
}

/**
 * Read one bounded opaque run identity without coercing it to a number.
 * @param value - Candidate wire value.
 * @returns Validated run identity.
 */
function asRunId(value: unknown): string {
  if (typeof value !== 'string' || value.length === 0 || value.includes('\0')) {
    reject('runId must be a nonempty UTF-8 string within its byte limit');
  }
  if (!isWellFormedUtf16(value)) {
    reject('runId must be well-formed UTF-16 without lone surrogate code units');
  }
  if (Buffer.byteLength(value, 'utf8') > 256) {
    reject('runId must be a nonempty UTF-8 string within its byte limit');
  }
  return value;
}

/**
 * Check that a JavaScript string has no unpaired UTF-16 surrogate code units.
 * @param value - Candidate opaque Unicode text.
 * @returns True when every surrogate belongs to one valid pair.
 */
function isWellFormedUtf16(value: string): boolean {
  for (let index = 0; index < value.length; index++) {
    const codeUnit = value.charCodeAt(index);
    if (codeUnit >= 0xd800 && codeUnit <= 0xdbff) {
      if (index + 1 >= value.length) return false;
      const next = value.charCodeAt(index + 1);
      if (next < 0xdc00 || next > 0xdfff) return false;
      index += 1;
    } else if (codeUnit >= 0xdc00 && codeUnit <= 0xdfff) {
      return false;
    }
  }
  return true;
}

/**
 * Read one lowercase SHA-256 digest.
 * @param value - Candidate wire value.
 * @param label - Field label included in rejections.
 * @returns Validated digest.
 */
function asSha256(value: unknown, label: string): string {
  if (typeof value !== 'string' || !SHA256_HEX.test(value)) {
    reject(`${label} must be a lowercase SHA-256 hex string`);
  }
  return value;
}

/**
 * Validate and normalize a descriptor before it crosses a worker boundary.
 * @param value - Untrusted descriptor candidate.
 * @returns Strict descriptor with no transferable or population-sized fields.
 */
export function parseManagedCheckpointDescriptor(value: unknown): ManagedCheckpointDescriptor {
  const descriptor = asRecord(value, 'descriptor');
  requireOnlyKeys(descriptor, MANAGED_CHECKPOINT_DESCRIPTOR_KEYS);
  const raw = descriptor as unknown as ManagedCheckpointDescriptor;
  if (raw.protocolVersion !== MANAGED_CHECKPOINT_DESCRIPTOR_PROTOCOL_VERSION) {
    reject(`unsupported protocol version ${String(raw.protocolVersion)}`);
  }
  if (raw.boundaryKind !== 'run-start' && raw.boundaryKind !== 'generation') {
    reject('boundaryKind is unsupported');
  }
  if (raw.managedRoot !== 'checkpoint-v3') reject('managedRoot is unsupported');
  if (
    raw.weightsEncoding !== 'raw-f32le-v1' &&
    raw.weightsEncoding !== 'f32le-shuffle4-zstd-v1'
  ) reject('weightsEncoding is unsupported');
  if (
    raw.recurrentStateEncoding !== 'raw-f32le-v1' &&
    raw.recurrentStateEncoding !== 'f32le-shuffle4-zstd-v1'
  ) reject('recurrentStateEncoding is unsupported');
  if (raw.writeValidationPolicy !== 'write-hash-count-fsync-rename-v1') {
    reject('writeValidationPolicy is unsupported');
  }
  const logicalRootSha256 = asSha256(raw.logicalRootSha256, 'logicalRootSha256');
  if (
    typeof raw.relativeFilename !== 'string' ||
    raw.relativeFilename !== `${logicalRootSha256}${CHECKPOINT_FILENAME_SUFFIX}`
  ) {
    reject('relativeFilename must be the digest-derived checkpoint-v3 basename');
  }
  const parsed: ManagedCheckpointDescriptor = {
    protocolVersion: MANAGED_CHECKPOINT_DESCRIPTOR_PROTOCOL_VERSION,
    operationId: parseCheckpointOperationId(raw.operationId),
    transitionEpoch: asU64Hex(raw.transitionEpoch, 'transitionEpoch'),
    runId: asRunId(raw.runId),
    generation: asU64Hex(raw.generation, 'generation'),
    completedStep: asU64Hex(raw.completedStep, 'completedStep'),
    boundaryKind: raw.boundaryKind,
    checkpointFormatVersion: asU64Hex(raw.checkpointFormatVersion, 'checkpointFormatVersion'),
    stateVersion: asU64Hex(raw.stateVersion, 'stateVersion'),
    graphLayoutVersion: asU64Hex(raw.graphLayoutVersion, 'graphLayoutVersion'),
    managedRoot: 'checkpoint-v3',
    relativeFilename: raw.relativeFilename,
    logicalRootSha256,
    storedByteCount: asU64Hex(raw.storedByteCount, 'storedByteCount'),
    decodedByteCount: asU64Hex(raw.decodedByteCount, 'decodedByteCount'),
    roleCount: asU64Hex(raw.roleCount, 'roleCount'),
    populationCount: asU64Hex(raw.populationCount, 'populationCount'),
    weightCount: asU64Hex(raw.weightCount, 'weightCount'),
    recurrentStateCount: asU64Hex(raw.recurrentStateCount, 'recurrentStateCount'),
    weightsEncoding: raw.weightsEncoding,
    recurrentStateEncoding: raw.recurrentStateEncoding,
    graphLayoutSha256: asSha256(raw.graphLayoutSha256, 'graphLayoutSha256'),
    writeValidationPolicy: 'write-hash-count-fsync-rename-v1'
  };
  const generation = BigInt(`0x${parsed.generation}`);
  const completedStep = BigInt(`0x${parsed.completedStep}`);
  if (parsed.boundaryKind === 'run-start') {
    if (generation !== 1n || completedStep !== 0n) {
      reject('run-start checkpoint must represent generation one at completed step zero');
    }
  } else if (completedStep === 0n) {
    reject('generation checkpoint completedStep must be nonzero');
  }
  return parsed;
}

/**
 * Compare every bounded descriptor field without numeric or JSON coercion.
 * @param left - First strictly parsed descriptor.
 * @param right - Second strictly parsed descriptor.
 * @returns True only when the complete descriptors are identical.
 */
export function managedCheckpointDescriptorsEqual(
  left: ManagedCheckpointDescriptor,
  right: ManagedCheckpointDescriptor
): boolean {
  return MANAGED_CHECKPOINT_DESCRIPTOR_KEYS.every(key => left[key] === right[key]);
}

/**
 * Validate the optional compact history record against its checkpoint boundary.
 * @param value - Candidate summary, or null for a run-start checkpoint.
 * @param descriptor - Already validated immutable checkpoint descriptor.
 * @returns Strict generation summary or null for run start.
 */
export function parseManagedGenerationSummary(
  value: unknown,
  descriptor: ManagedCheckpointDescriptor
): ManagedGenerationSummary | null {
  if (descriptor.boundaryKind === 'run-start') {
    if (value !== null) reject('run-start checkpoints must not include a generation summary');
    return null;
  }
  const summary = asRecord(value, 'generationSummary');
  const keys = [
    'completedGeneration', 'bestF64Hex', 'averageF64Hex', 'minimumF64Hex',
    'speciesCount', 'topSpeciesSize', 'averageWeightF64Hex', 'weightVarianceF64Hex'
  ] as const;
  requireOnlyKeys(summary, keys);
  const parsed: ManagedGenerationSummary = {
    completedGeneration: asU64Hex(summary['completedGeneration'], 'completedGeneration'),
    bestF64Hex: asFiniteF64Hex(summary['bestF64Hex'], 'bestF64Hex'),
    averageF64Hex: asFiniteF64Hex(summary['averageF64Hex'], 'averageF64Hex'),
    minimumF64Hex: asFiniteF64Hex(summary['minimumF64Hex'], 'minimumF64Hex'),
    speciesCount: asU64Hex(summary['speciesCount'], 'speciesCount'),
    topSpeciesSize: asU64Hex(summary['topSpeciesSize'], 'topSpeciesSize'),
    averageWeightF64Hex: asFiniteF64Hex(
      summary['averageWeightF64Hex'],
      'averageWeightF64Hex'
    ),
    weightVarianceF64Hex: asFiniteF64Hex(
      summary['weightVarianceF64Hex'],
      'weightVarianceF64Hex'
    )
  };
  const completed = BigInt(`0x${parsed.completedGeneration}`);
  const successor = BigInt(`0x${descriptor.generation}`);
  if (completed === 0n || completed === 0xffffffffffffffffn || completed + 1n !== successor) {
    reject('generation summary must describe exactly the generation preceding its checkpoint');
  }
  const populationCount = BigInt(`0x${descriptor.populationCount}`);
  if (BigInt(`0x${parsed.speciesCount}`) > populationCount) {
    reject('speciesCount exceeds the checkpoint populationCount');
  }
  if (BigInt(`0x${parsed.topSpeciesSize}`) > populationCount) {
    reject('topSpeciesSize exceeds the checkpoint populationCount');
  }
  if (BigInt(`0x${parsed.speciesCount}`) > 0xffff_ffffn ||
    BigInt(`0x${parsed.topSpeciesSize}`) > 0xffff_ffffn) {
    reject('species counts exceed the compact history-v1 unsigned-32-bit fields');
  }
  return parsed;
}

/**
 * Validate the run-scoped Hall-of-Fame reference paired with a compact summary.
 * @param value - Candidate reference containing no genome payload bytes.
 * @param descriptor - Same immutable generation checkpoint descriptor.
 * @param summary - Exact validated summary for the completed generation.
 * @returns Strict scalar Hall-of-Fame reference.
 */
function parseManagedHallOfFameReference(
  value: unknown,
  descriptor: ManagedCheckpointDescriptor,
  summary: ManagedGenerationSummary
): ManagedHallOfFameReference {
  const reference = asRecord(value, 'hallOfFame');
  const keys = [
    'completedGeneration', 'sourcePopulationSlot', 'sourceSnakeId', 'fitnessF64Hex',
    'pointsF64Hex', 'length', 'successorPopulationSlot', 'successorGenomeId'
  ] as const;
  requireOnlyKeys(reference, keys);
  const parsed: ManagedHallOfFameReference = {
    completedGeneration: asU64Hex(reference['completedGeneration'], 'hallOfFame.completedGeneration'),
    sourcePopulationSlot: asU64Hex(reference['sourcePopulationSlot'], 'sourcePopulationSlot'),
    sourceSnakeId: asU64Hex(reference['sourceSnakeId'], 'sourceSnakeId'),
    fitnessF64Hex: asFiniteF64Hex(reference['fitnessF64Hex'], 'fitnessF64Hex'),
    pointsF64Hex: asFiniteF64Hex(reference['pointsF64Hex'], 'pointsF64Hex'),
    length: asU64Hex(reference['length'], 'hallOfFame.length'),
    successorPopulationSlot: asU64Hex(
      reference['successorPopulationSlot'],
      'successorPopulationSlot'
    ),
    successorGenomeId: asU64Hex(reference['successorGenomeId'], 'successorGenomeId')
  };
  if (parsed.completedGeneration !== summary.completedGeneration) {
    reject('Hall-of-Fame generation does not match compact history');
  }
  if (parsed.fitnessF64Hex !== summary.bestF64Hex) {
    reject('Hall-of-Fame fitness does not match compact-history best fitness');
  }
  const populationCount = BigInt(`0x${descriptor.populationCount}`);
  const sourceSlot = BigInt(`0x${parsed.sourcePopulationSlot}`);
  const successorSlot = BigInt(`0x${parsed.successorPopulationSlot}`);
  if (sourceSlot >= populationCount || successorSlot >= populationCount) {
    reject('Hall-of-Fame population slot is outside the checkpoint population');
  }
  if (sourceSlot > 0xffff_ffffn || successorSlot > 0xffff_ffffn) {
    reject('Hall-of-Fame population slot exceeds its unsigned-32-bit record field');
  }
  if (BigInt(`0x${parsed.sourceSnakeId}`) === 0n ||
    BigInt(`0x${parsed.successorGenomeId}`) === 0n) {
    reject('Hall-of-Fame snake and successor genome identities must be nonzero');
  }
  return parsed;
}

/** Validate one Rust-published deduplicated Hall-of-Fame weight object. */
export function parseManagedHallOfFameWeightsDescriptor(
  value: unknown,
  checkpoint: ManagedCheckpointDescriptor
): ManagedHallOfFameWeightsDescriptor {
  const descriptor = asRecord(value, 'hallOfFameWeights');
  requireOnlyKeys(descriptor, [
    'version', 'logicalSha256', 'relativeFilename', 'encoding',
    'storedByteCount', 'decodedByteCount', 'weightCount'
  ]);
  if (descriptor['version'] !== 1 || typeof descriptor['logicalSha256'] !== 'string' ||
      !SHA256_HEX.test(descriptor['logicalSha256'])) {
    reject('Hall-of-Fame weight descriptor has invalid version or logical SHA-256');
  }
  const logicalSha256 = descriptor['logicalSha256'];
  if (descriptor['relativeFilename'] !== `${logicalSha256}.hof-weights-v1`) {
    reject('Hall-of-Fame weight filename is not digest-derived');
  }
  if (descriptor['encoding'] !== 'raw-f32le-v1' &&
      descriptor['encoding'] !== 'f32le-shuffle4-zstd-v1') {
    reject('Hall-of-Fame weight descriptor has an unsupported encoding');
  }
  const storedByteCount = asU64Hex(descriptor['storedByteCount'], 'hallOfFameWeights.storedByteCount');
  const decodedByteCount = asU64Hex(descriptor['decodedByteCount'], 'hallOfFameWeights.decodedByteCount');
  const weightCount = asU64Hex(descriptor['weightCount'], 'hallOfFameWeights.weightCount');
  const count = BigInt(`0x${weightCount}`);
  const populationCount = BigInt(`0x${checkpoint.populationCount}`);
  const aggregateWeightCount = BigInt(`0x${checkpoint.weightCount}`);
  const storedBytes = BigInt(`0x${storedByteCount}`);
  const decodedBytes = BigInt(`0x${decodedByteCount}`);
  if (populationCount === 0n || aggregateWeightCount % populationCount !== 0n ||
      count !== aggregateWeightCount / populationCount || count > 0x3fff_ffff_ffff_ffffn ||
      decodedBytes !== count * 4n || (count > 0n && storedBytes === 0n) ||
      (descriptor['encoding'] === 'raw-f32le-v1' && storedBytes !== decodedBytes)) {
    reject('Hall-of-Fame weight descriptor has inconsistent counts');
  }
  return {
    version: 1,
    logicalSha256,
    relativeFilename: descriptor['relativeFilename'] as string,
    encoding: descriptor['encoding'],
    storedByteCount,
    decodedByteCount,
    weightCount
  };
}

/** Validate the worker-written fixed-width inventory for one exact export operation. */
export function parseManagedExportInventoryDescriptor(
  value: unknown,
  operationId: CheckpointOperationId
): ManagedExportInventoryDescriptor {
  const descriptor = asRecord(value, 'exportInventory');
  requireOnlyKeys(descriptor, [
    'version', 'relativeFilename', 'sha256', 'storedByteCount',
    'historyCount', 'hallOfFameCount'
  ]);
  if (descriptor['version'] !== 1 ||
      descriptor['relativeFilename'] !== `.${operationId}.export-inventory-v1` ||
      typeof descriptor['sha256'] !== 'string' || !SHA256_HEX.test(descriptor['sha256'])) {
    reject('export inventory has invalid identity');
  }
  const storedByteCount = asU64Hex(descriptor['storedByteCount'], 'exportInventory.storedByteCount');
  const historyCount = asU64Hex(descriptor['historyCount'], 'exportInventory.historyCount');
  const hallOfFameCount = asU64Hex(descriptor['hallOfFameCount'], 'exportInventory.hallOfFameCount');
  const history = BigInt(`0x${historyCount}`);
  const hallOfFame = BigInt(`0x${hallOfFameCount}`);
  const expectedBytes = 32n + history * 56n + hallOfFame * 120n;
  if (hallOfFame > history || expectedBytes > 0xffff_ffff_ffff_ffffn ||
      BigInt(`0x${storedByteCount}`) !== expectedBytes) {
    reject('export inventory has inconsistent counts');
  }
  return {
    version: 1,
    relativeFilename: descriptor['relativeFilename'] as string,
    sha256: descriptor['sha256'],
    storedByteCount,
    historyCount,
    hallOfFameCount
  };
}

/** Compare immutable checkpoint content while ignoring local publication correlation. */
export function managedCheckpointContentsEqual(
  left: ManagedCheckpointDescriptor,
  right: ManagedCheckpointDescriptor
): boolean {
  return MANAGED_CHECKPOINT_DESCRIPTOR_KEYS.every(key =>
    key === 'operationId' || key === 'transitionEpoch' || left[key] === right[key]
  );
}

/** Validate the Rust-written fixed-width inventory for one import operation. */
export function parseManagedImportInventoryDescriptor(
  value: unknown,
  operationId: CheckpointOperationId
): ManagedImportInventoryDescriptor {
  const descriptor = asRecord(value, 'importInventory');
  requireOnlyKeys(descriptor, [
    'version', 'relativeFilename', 'sha256', 'storedByteCount',
    'historyCount', 'hallOfFameCount'
  ]);
  if (descriptor['version'] !== 1 ||
      descriptor['relativeFilename'] !== `.${operationId}.import-inventory-v1` ||
      typeof descriptor['sha256'] !== 'string' || !SHA256_HEX.test(descriptor['sha256'])) {
    reject('import inventory has invalid identity');
  }
  const storedByteCount = asU64Hex(descriptor['storedByteCount'], 'importInventory.storedByteCount');
  const historyCount = asU64Hex(descriptor['historyCount'], 'importInventory.historyCount');
  const hallOfFameCount = asU64Hex(descriptor['hallOfFameCount'], 'importInventory.hallOfFameCount');
  const history = BigInt(`0x${historyCount}`);
  const hallOfFame = BigInt(`0x${hallOfFameCount}`);
  const expectedBytes = 32n + history * 56n + hallOfFame * 120n;
  if (hallOfFame > history || expectedBytes > 0xffff_ffff_ffff_ffffn ||
      BigInt(`0x${storedByteCount}`) !== expectedBytes) {
    reject('import inventory has inconsistent counts');
  }
  return {
    version: 1,
    relativeFilename: descriptor['relativeFilename'] as string,
    sha256: descriptor['sha256'],
    storedByteCount,
    historyCount,
    hallOfFameCount
  };
}

/** Validate durable provenance for one owner-selected import branch. */
export function parseManagedImportBranchResult(value: unknown): ManagedImportBranchResult {
  const raw = asRecord(value, 'importBranch');
  requireOnlyKeys(raw, [
    'operationId', 'branchRunId', 'sourceRunId', 'sourceGeneration',
    'sourceCheckpointId', 'recoveredDescriptor'
  ]);
  const recoveredDescriptor = parseManagedCheckpointDescriptor(raw['recoveredDescriptor']);
  const result: ManagedImportBranchResult = {
    operationId: parseCheckpointOperationId(raw['operationId']),
    branchRunId: asRunId(raw['branchRunId']),
    sourceRunId: asRunId(raw['sourceRunId']),
    sourceGeneration: asU64Hex(raw['sourceGeneration'], 'importBranch.sourceGeneration'),
    sourceCheckpointId: asSha256(raw['sourceCheckpointId'], 'importBranch.sourceCheckpointId'),
    recoveredDescriptor
  };
  if (result.branchRunId === result.sourceRunId ||
      result.sourceRunId !== recoveredDescriptor.runId ||
      result.sourceGeneration !== recoveredDescriptor.generation ||
      result.sourceCheckpointId !== recoveredDescriptor.logicalRootSha256) {
    reject('import branch provenance differs from its immutable source');
  }
  return result;
}

/**
 * Validate all small metadata that must commit atomically with one checkpoint pointer.
 * @param value - Candidate generation commit, or null for run start.
 * @param descriptor - Already validated immutable checkpoint descriptor.
 * @returns Complete strict generation commit or null.
 */
export function parseManagedGenerationCommit(
  value: unknown,
  descriptor: ManagedCheckpointDescriptor
): ManagedGenerationCommit | null {
  if (descriptor.boundaryKind === 'run-start') {
    if (value !== null) reject('run-start checkpoints must not include generation metadata');
    return null;
  }
  const commit = asRecord(value, 'generationCommit');
  requireOnlyKeys(commit, ['summary', 'hallOfFame', 'hallOfFameWeights']);
  const summary = parseManagedGenerationSummary(commit['summary'], descriptor);
  if (summary === null) reject('generation checkpoint is missing compact history');
  return {
    summary,
    hallOfFame: parseManagedHallOfFameReference(commit['hallOfFame'], descriptor, summary),
    hallOfFameWeights: parseManagedHallOfFameWeightsDescriptor(commit['hallOfFameWeights'], descriptor)
  };
}

/**
 * Validate bounded worker bootstrap limits without accepting unknown nested data.
 * @param value - Candidate limits object.
 * @returns Strict exact limit values.
 */
export function parseManagedCheckpointDescriptorLimits(
  value: unknown
): ManagedCheckpointDescriptorLimits {
  const limits = asRecord(value, 'limits');
  const keys = [
    'maxStoredByteCount', 'maxDecodedByteCount', 'maxPopulationCount', 'maxWeightsPerGenome',
    'maxRecurrentStateCount', 'maxRoleCount'
  ] as const;
  requireOnlyKeys(limits, keys);
  const parsed = {
    maxStoredByteCount: asU64Hex(limits['maxStoredByteCount'], 'maxStoredByteCount'),
    maxDecodedByteCount: asU64Hex(limits['maxDecodedByteCount'], 'maxDecodedByteCount'),
    maxPopulationCount: asU64Hex(limits['maxPopulationCount'], 'maxPopulationCount'),
    maxWeightsPerGenome: asU64Hex(limits['maxWeightsPerGenome'], 'maxWeightsPerGenome'),
    maxRecurrentStateCount: asU64Hex(limits['maxRecurrentStateCount'], 'maxRecurrentStateCount'),
    maxRoleCount: asU64Hex(limits['maxRoleCount'], 'maxRoleCount')
  };
  for (const [label, item] of Object.entries(parsed)) {
    if (BigInt(`0x${item}`) === 0n) throw new RangeError(`${label} must be nonzero`);
  }
  return parsed;
}
