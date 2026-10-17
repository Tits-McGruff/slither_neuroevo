import { DEFAULT_DISTILLATION_SPOOL_GIB, GIB_BYTES, MAX_DISTILLATION_SPOOL_GIB, validateDistillationSpoolGiB } from './distillationSpoolLimits.ts';
import { distillationHex, distillationInteger } from '../../src/protocol/distillation.ts';
/** Shared diagnostic wire contracts remain browser-safe and contain no sample codec. */
export { distillationHex, distillationInteger } from '../../src/protocol/distillation.ts';
export type { DistillationDataset, DistillationChunkPage, DistillationLease, DistillationDatasetSummary, DistillationDatasetPage } from '../../src/protocol/distillation.ts';
import { createHash } from 'node:crypto';
import { sensorLayoutFromContract } from '../../src/protocol/sensors.ts';
import type { TeacherCaptureContract, TeacherCaptureDrain, TeacherChunkDescriptor, TeacherChunkTransfer, DistillationStoragePolicy } from '../../src/protocol/distillation.ts';

/** Largest native immutable teacher object and bounded worker ingestion batch. */
export const TEACHER_OBJECT_BYTES = 1024 * 1024;
/** A worker receives at most one native handoff's opaque ownership. */
export const TEACHER_BATCH_BYTES = 1024 * 1024;
/** Fixed-width zero cursor, preserving exact unsigned ordering in SQLite. */
export const DISTILLATION_ZERO = '0000000000000000';

/** Durable storage/lease admission, separate from native world/controller settings. */
export interface DistillationSpoolLimits {
  /** Maximum retained complete encoded bytes. */
  maxBytes: number;
  /** Global retained complete object ceiling. */
  maxChunks: number;
  /** Maximum immutable dataset manifests. */
  maxDatasets: number;
  /** Global active lease ceiling. */
  maxLeases: number;
  /** Global pinned complete-object count ceiling. */
  maxPinnedChunks: number;
  /** Global pinned encoded-byte ceiling. */
  maxPinnedBytes: number;
  /** Maximum lease duration or renewal interval. */
  maxLeaseMs: number;
  /** Filesystem reserve preserved before any new object publication. */
  diskReserveBytes: number;
}

/** Initial spool defaults requiring workload qualification, as specified by the plan. */
export const DISTILLATION_SPOOL_DEFAULTS: Readonly<DistillationSpoolLimits> = {
  maxBytes: DEFAULT_DISTILLATION_SPOOL_GIB * GIB_BYTES, maxChunks: 65_536, maxDatasets: 256,
  maxLeases: 64, maxPinnedChunks: 1024, maxPinnedBytes: 512 * 1024 ** 2,
  maxLeaseMs: 300_000, diskReserveBytes: 1024 ** 3
};

/** Convert the operator's disk quota while retaining all object, lease and reserve ceilings. */
export function distillationSpoolLimitsForGiB(value: unknown): DistillationSpoolLimits {
  return parseSpoolLimits({ ...DISTILLATION_SPOOL_DEFAULTS,
    maxBytes: validateDistillationSpoolGiB(value) * GIB_BYTES });
}

/** Reject arrays, inherited fields, unknown fields and incomplete metadata records. */
function record(value: unknown, keys: readonly string[], field: string): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value) ||
      Object.keys(value).length !== keys.length || keys.some(key => !Object.hasOwn(value, key))) {
    throw new TypeError(`invalid ${field} fields`);
  }
  const raw = value as Record<string, unknown>;
  return Object.fromEntries(keys.map(key => [key, raw[key]]));
}

/** Validate all configurable storage ceilings before a worker/database is created. */
export function parseSpoolLimits(value: DistillationSpoolLimits): DistillationSpoolLimits {
  const limits = { ...value };
  distillationInteger(limits.maxBytes, 1, MAX_DISTILLATION_SPOOL_GIB * GIB_BYTES, 'maxBytes');
  distillationInteger(limits.maxChunks, 1, 65_536, 'maxChunks');
  distillationInteger(limits.maxDatasets, 1, 256, 'maxDatasets');
  distillationInteger(limits.maxLeases, 1, 64, 'maxLeases');
  distillationInteger(limits.maxPinnedChunks, 1, 1024, 'maxPinnedChunks');
  distillationInteger(limits.maxPinnedBytes, 1, 512 * 1024 ** 2, 'maxPinnedBytes');
  distillationInteger(limits.maxLeaseMs, 1, 300_000, 'maxLeaseMs');
  distillationInteger(limits.diskReserveBytes, 1024 ** 3, Number.MAX_SAFE_INTEGER, 'diskReserveBytes');
  return limits;
}

/** Snapshot the worker's admitted policy without exposing paths, bytes or game ownership. */
export function storagePolicyFromLimits(limits: DistillationSpoolLimits): DistillationStoragePolicy {
  const value = parseSpoolLimits(limits);
  return { schemaVersion: 1, scope: 'producer-global',
    maxBytes: BigInt(value.maxBytes).toString(16).padStart(16, '0'),
    maxChunks: value.maxChunks, maxDatasets: value.maxDatasets, maxLeases: value.maxLeases,
    maxPinnedChunks: value.maxPinnedChunks,
    maxPinnedBytes: BigInt(value.maxPinnedBytes).toString(16).padStart(16, '0'),
    maxLeaseMs: value.maxLeaseMs,
    diskReserveBytes: BigInt(value.diskReserveBytes).toString(16).padStart(16, '0') };
}

/** Independently validate bounded metadata returned by the isolated storage worker. */
export function parseStoragePolicy(value: unknown): DistillationStoragePolicy {
  const raw = record(value, ['schemaVersion', 'scope', 'maxBytes', 'maxChunks', 'maxDatasets',
    'maxLeases', 'maxPinnedChunks', 'maxPinnedBytes', 'maxLeaseMs', 'diskReserveBytes'], 'storage policy');
  if (raw['schemaVersion'] !== 1 || raw['scope'] !== 'producer-global') throw new TypeError('unsupported storage policy');
  for (const field of ['maxBytes', 'maxPinnedBytes', 'diskReserveBytes']) distillationHex(raw[field], 16, field, true);
  const parsed = storagePolicyFromLimits({
    maxBytes: Number(BigInt('0x' + String(raw['maxBytes']))),
    maxChunks: distillationInteger(raw['maxChunks'], 1, 65_536, 'maxChunks'),
    maxDatasets: distillationInteger(raw['maxDatasets'], 1, 256, 'maxDatasets'),
    maxLeases: distillationInteger(raw['maxLeases'], 1, 64, 'maxLeases'),
    maxPinnedChunks: distillationInteger(raw['maxPinnedChunks'], 1, 1024, 'maxPinnedChunks'),
    maxPinnedBytes: Number(BigInt('0x' + String(raw['maxPinnedBytes']))),
    maxLeaseMs: distillationInteger(raw['maxLeaseMs'], 1, 300_000, 'maxLeaseMs'),
    diskReserveBytes: Number(BigInt('0x' + String(raw['diskReserveBytes'])))
  });
  return parsed;
}

/** Validate native identity and the exact sensor metadata bytes, without sampling anything. */
export function parseTeacherContract(value: unknown, sensorJson: unknown): TeacherCaptureContract {
  const raw = record(value, ['captureEpoch', 'runId', 'configSha256', 'sensorSha256', 'teacherSha256',
    'teacherRevision', 'teacherMode', 'controlIntervalSeconds', 'inputSize'], 'teacher contract');
  distillationHex(raw['captureEpoch'], 32, 'capture epoch', true);
  for (const key of ['configSha256', 'sensorSha256', 'teacherSha256']) distillationHex(raw[key], 64, key);
  if (typeof raw['runId'] !== 'string' || Buffer.byteLength(raw['runId']) < 1 || Buffer.byteLength(raw['runId']) > 128 ||
      raw['runId'].includes('\0')) throw new TypeError('invalid teacher run identity');
  distillationInteger(raw['teacherRevision'], 2, 2, 'teacher revision');
  distillationInteger(raw['teacherMode'], 0, 1, 'teacher mode');
  distillationInteger(raw['inputSize'], 1, 504, 'teacher width');
  const interval = raw['controlIntervalSeconds'];
  if (typeof interval !== 'number' || !Number.isFinite(interval) || interval < 0.008 || interval > 0.06) {
    throw new TypeError('invalid teacher cadence');
  }
  if (typeof sensorJson !== 'string' || Buffer.byteLength(sensorJson) > 65_536) throw new TypeError('invalid sensor metadata');
  const digest = createHash('sha256').update('slither.sensor-contract.v1\0').update(sensorJson).digest('hex');
  if (digest !== raw['sensorSha256']) throw new TypeError('sensor metadata digest mismatch');
  const layout = sensorLayoutFromContract({ canonicalJson: sensorJson, sha256: digest });
  if (layout.layoutVersion !== 'v4' || layout.inputSize !== raw['inputSize']) throw new TypeError('teacher sensor contract mismatch');
  return { ...raw } as unknown as TeacherCaptureContract;
}

/** Validate only descriptors and byte ownership; Rust remains the sample codec owner. */
export function parseTeacherTransfer(value: unknown, contract: TeacherCaptureContract): TeacherChunkTransfer {
  const raw = record(value, ['descriptor', 'bytes'], 'teacher transfer');
  const descriptor = record(raw['descriptor'], ['captureEpoch', 'sequence', 'generation', 'baselineSlot',
    'physicalSnakeId', 'firstSample', 'lastSample', 'firstStep', 'lastStep', 'count', 'inputSize',
    'configSha256', 'sensorSha256', 'teacherSha256', 'startReason', 'endReason', 'byteCount', 'sha256'], 'teacher descriptor');
  for (const key of ['sequence', 'generation', 'physicalSnakeId', 'firstSample', 'lastSample', 'firstStep', 'lastStep']) {
    distillationHex(descriptor[key], 16, key, true);
  }
  const count = distillationInteger(descriptor['count'], 1, 256, 'teacher count');
  const word = (key: string): bigint => BigInt(`0x${descriptor[key] as string}`);
  if (word('lastSample') - word('firstSample') !== BigInt(count - 1) ||
      word('lastStep') - word('firstStep') < BigInt(count - 1) || word('lastStep') - word('firstStep') > 60n) {
    throw new TypeError('teacher descriptor chronology mismatch');
  }
  distillationInteger(descriptor['baselineSlot'], 0, 119, 'baseline slot');
  for (const key of ['captureEpoch', 'configSha256', 'sensorSha256', 'teacherSha256', 'inputSize'] as const) {
    if (descriptor[key] !== contract[key]) throw new TypeError(`teacher descriptor ${key} mismatch`);
  }
  if (!['captureStart', 'physicalLifeStart', 'continuation', 'gap', 'generationStart'].includes(descriptor['startReason'] as string) ||
      !['maximumSamples', 'simulatedSecond', 'lifeEnded', 'generationEnded', 'contractChanged', 'captureStopped', 'interrupted']
        .includes(descriptor['endReason'] as string)) throw new TypeError('invalid teacher boundary reasons');
  const length = distillationInteger(descriptor['byteCount'], 1, TEACHER_OBJECT_BYTES, 'teacher byte count');
  distillationHex(descriptor['sha256'], 64, 'teacher byte digest');
  if (!(raw['bytes'] instanceof Uint8Array) || raw['bytes'].byteLength !== length ||
      raw['bytes'].buffer instanceof SharedArrayBuffer) throw new TypeError('teacher byte ownership mismatch');
  const digest = createHash('sha256').update(raw['bytes']).digest('hex');
  if (digest !== descriptor['sha256']) throw new TypeError('teacher object digest mismatch');
  return { descriptor: { ...descriptor } as unknown as TeacherChunkDescriptor, bytes: raw['bytes'] };
}

/** Validate a coherent bounded native drain before any files or metadata change. */
export function parseTeacherDrain(value: TeacherCaptureDrain): TeacherCaptureDrain & {
  contract: TeacherCaptureContract; sensorContractJson: string;
} {
  const contract = parseTeacherContract(value.contract, value.sensorContractJson);
  for (const key of ['productionWatermark', 'releasedThrough', 'queuedBytes', 'droppedChunks', 'droppedSamples', 'droppedBytes'] as const) {
    distillationHex(value[key], 16, key);
  }
  distillationInteger(value.queuedChunks, 0, 65_792, 'queued chunks');
  if (value.releasedThrough > value.productionWatermark || typeof value.stopped !== 'boolean' ||
      typeof value.moreWork !== 'boolean' || (value.error != null && (typeof value.error !== 'string' || value.error.length > 1024))) {
    throw new TypeError('incoherent teacher drain');
  }
  if (!Array.isArray(value.chunks) || value.chunks.length > 256) throw new TypeError('teacher drain count limit');
  let bytes = 0;
  let previous = DISTILLATION_ZERO;
  const chunks = value.chunks.map(chunk => {
    const parsed = parseTeacherTransfer(chunk, contract);
    bytes += parsed.bytes.byteLength;
    if (bytes > TEACHER_BATCH_BYTES || parsed.descriptor.sequence <= previous ||
        parsed.descriptor.sequence > value.releasedThrough) throw new TypeError('teacher drain ordering/byte limit');
    previous = parsed.descriptor.sequence;
    return parsed;
  });
  return { ...value, contract, sensorContractJson: value.sensorContractJson!, chunks };
}
