/** Descriptor protocol for the isolated Stage 3 checkpoint metadata worker. */

/** Protocol version understood by the checkpoint persistence worker. */
export const MANAGED_CHECKPOINT_DESCRIPTOR_PROTOCOL_VERSION = 1;

/** Fixed lowercase hexadecimal representation of an unsigned 64-bit value. */
export type U64Hex = string;

/** Exact lowercase hexadecimal representation of one IEEE-754 Float64 bit pattern. */
export type F64Hex = string;

/** Fixed lowercase hexadecimal operation token, independent of the run identity. */
export type CheckpointOperationId = string;

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
 * Run-scoped Hall-of-Fame metadata referencing the elite copy inside the immutable checkpoint.
 *
 * No genome weights are duplicated here. `successorPopulationSlot` and `successorGenomeId`
 * identify the bit-exact elite already stored in the checkpoint selected by the same commit.
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

/** Complete small metadata that must commit with one generation checkpoint pointer. */
export interface ManagedGenerationCommit {
  /** Compact eight-field chart/history record. */
  summary: ManagedGenerationSummary;
  /** Run-scoped reference to the selected elite inside the same checkpoint. */
  hallOfFame: ManagedHallOfFameReference;
}

/** Commit request sent from the Node client to its one persistence worker. */
export interface CommitManagedCheckpointRequest {
  /** Message discriminator. */
  type: 'commitManagedCheckpoint';
  /** Descriptor-only checkpoint publication request. */
  descriptor: ManagedCheckpointDescriptor;
  /** Exact small generation metadata, otherwise null for run start. */
  generationCommit: ManagedGenerationCommit | null;
}

/** Orderly client-owned worker shutdown request. */
export interface CheckpointPersistenceShutdownRequest {
  /** Message discriminator. */
  type: 'shutdown';
}

/** Requests accepted by the isolated persistence worker. */
export type CheckpointPersistenceWorkerRequest =
  | CommitManagedCheckpointRequest
  | CheckpointPersistenceShutdownRequest;

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
  | ManagedCheckpointCommittedResponse
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
  requireOnlyKeys(commit, ['summary', 'hallOfFame']);
  const summary = parseManagedGenerationSummary(commit['summary'], descriptor);
  if (summary === null) reject('generation checkpoint is missing compact history');
  return {
    summary,
    hallOfFame: parseManagedHallOfFameReference(commit['hallOfFame'], descriptor, summary)
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
