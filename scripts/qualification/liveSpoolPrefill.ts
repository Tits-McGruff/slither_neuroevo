/** Explicit inactive opaque-byte pressure; it never fabricates active native teacher samples. */
import assert from 'node:assert/strict';
import { randomBytes } from 'node:crypto';
import type { SensorContractEnvelope } from '../../src/protocol/sensors.ts';
import type { TeacherCaptureContract } from '../../src/protocol/distillation.ts';
import type { DistillationSpoolClient } from '../../server/rustEngine/distillationSpoolClient.ts';
import { TEACHER_BATCH_BYTES, distillationRecoveryTimeoutMs, parseSpoolLimits, type DistillationSpoolLimits } from '../../server/rustEngine/distillationProtocol.ts';
import { OPERATING_DISK_RESERVE_BYTES, SQLITE_WAL_ALLOWANCE_BYTES, type DiskAdmissionRequest } from '../../server/rustEngine/diskAdmission.ts';
import { costs, drain, freeBytes, sample, settled, word, type Costs } from './spoolFixture.ts';

/** A fixed byte-pressure plan distinct from the live capture's real sample ownership. */
export interface LiveSpoolPrefillPlan {
  /** Each closed-fixture object occupies exactly one physical MiB. */
  objectBytes: number;
  /** Complete bounded original-object count. */
  objects: number;
  /** Encoded byte occupancy requested before live capture. */
  targetBytes: number;
  /** Minimum free space including future checkpoint publication and normal reserves. */
  requiredFreeBytes: bigint;
  /** Absolute setup allowance; independent of ordinary worker request deadlines. */
  deadlineMs: number;
}
/** Compact original-fixture state; no per-object inventory is retained. */
export interface LiveSpoolPrefillState {
  /** Existing unpinned fixture objects remaining after actual production retention. */
  retainedChunks: number;
  /** Actual retained encoded bytes as the production u64 hexadecimal value. */
  retainedBytes: string;
  /** Closed fixture's immutable resolved watermark. */
  resolvedThrough: string;
}
/** Retained setup and recovery evidence, explicitly labelled as storage-only synthetic pressure. */
export interface LiveSpoolPrefillReceipt {
  /** This is not a native TeacherSampleV1 dataset and is never sent to the trainer. */
  kind: 'inactive-synthetic-opaque-spool-v1';
  /** Separate stopped capture identity, unrelated to the live trainer intake. */
  captureEpoch: string;
  /** Admitted encoded target in whole GiB. */
  targetGiB: number;
  /** Physical original-object size. */
  objectBytes: number;
  /** Admitted original-object count. */
  objects: number;
  /** Selected production encoded-byte ceiling, independently verified again over HTTP. */
  selectedMaxBytes: number;
  /** Selected production object ceiling. */
  selectedMaxChunks: number;
  /** Available bytes read from the actual target filesystem before filling. */
  initialFreeBytes: string;
  /** Required free bytes including future checkpoint and normal disk reserves. */
  requiredFreeBytes: string;
  /** Actual state before closing the first worker. */
  beforeRecovery: LiveSpoolPrefillState;
  /** Complete object ingestion costs, including real writes and metadata publication. */
  ingestion: Costs;
  /** Elapsed setup time, excluded from the later live measurement window. */
  elapsedSeconds: number;
  /** Explicit bounded setup allowance used for this physical fill. */
  deadlineMs: number;
  /** Actual readiness budget selected from retained bytes by the reopened production worker. */
  recoveryBudgetMs?: number;
  /** Measured cold-worker readiness duration. */
  recoveryElapsedMs?: number;
  /** Same stopped fixture independently re-admitted after cold recovery. */
  afterRecovery?: LiveSpoolPrefillState;
  /** Actual retained-byte readiness allowance selected by final cold verification. */
  finalRecoveryBudgetMs?: number;
  /** Final cold-worker readiness duration. */
  finalRecoveryElapsedMs?: number;
  /** Stopped fixture independently verified after the live window and final cold startup. */
  afterFinalRecovery?: LiveSpoolPrefillState;
  /** Old fixture occupancy after real live capture and its ordinary eviction. */
  afterCapture?: LiveSpoolPrefillState;
}

/** Reject over-quota or under-reserved fills before allocating payloads or writing files. */
export function planLiveSpoolPrefill(gib: number, limits: DistillationSpoolLimits,
  availableBytes: bigint, checkpoint: Readonly<DiskAdmissionRequest>, deadlineSeconds = 1800): LiveSpoolPrefillPlan {
  limits = parseSpoolLimits(limits);
  assert(Number.isSafeInteger(deadlineSeconds) && deadlineSeconds >= 1 && deadlineSeconds <= 10_800,
    'prefill setup deadline must be 1..10800 whole seconds');
  assert([availableBytes, checkpoint.sourceSpoolBytes, checkpoint.candidateSpoolBytes, checkpoint.finalManagedBytes]
    .every(value => typeof value === 'bigint' && value >= 0n), 'prefill disk terms must be nonnegative bytes');
  assert(Number.isSafeInteger(gib) && gib >= 1 && gib <= 64, 'prefill must be 1..64 whole GiB');
  const targetBytes = gib * 1024 ** 3; const objectBytes = 1024 ** 2; const objects = targetBytes / objectBytes;
  assert(targetBytes <= limits.maxBytes && objects <= limits.maxChunks, 'prefill exceeds selected byte/object quota');
  const reserve = BigInt(limits.diskReserveBytes) > OPERATING_DISK_RESERVE_BYTES
    ? BigInt(limits.diskReserveBytes) : OPERATING_DISK_RESERVE_BYTES;
  const requiredFreeBytes = BigInt(targetBytes) + reserve + SQLITE_WAL_ALLOWANCE_BYTES
    + checkpoint.sourceSpoolBytes + checkpoint.candidateSpoolBytes + checkpoint.finalManagedBytes;
  assert(availableBytes >= requiredFreeBytes, 'prefill must preserve spool/checkpoint/system disk reserves');
  return {objectBytes, objects, targetBytes, requiredFreeBytes, deadlineMs: deadlineSeconds * 1000};
}

/** Read a closed synthetic fixture without interpreting its opaque payload as observations. */
export async function readLiveSpoolPrefillState(store: Pick<DistillationSpoolClient, 'dataset'>,
  receipt: Pick<LiveSpoolPrefillReceipt, 'captureEpoch' | 'objects' | 'objectBytes'>): Promise<LiveSpoolPrefillState> {
  const dataset = await store.dataset(receipt.captureEpoch);
  assert.equal(dataset.state, 'stopped'); assert.equal(dataset.resolvedThrough, word(receipt.objects));
  assert.equal(dataset.productionWatermark, word(receipt.objects));
  assert(dataset.retainedChunks >= 0 && dataset.retainedChunks <= receipt.objects);
  assert.equal(BigInt('0x' + dataset.retainedBytes), BigInt(dataset.retainedChunks * receipt.objectBytes));
  return {retainedChunks: dataset.retainedChunks, retainedBytes: dataset.retainedBytes, resolvedThrough: dataset.resolvedThrough};
}

/** Fill one separate inactive epoch before a coordinator starts; all normal worker bounds remain active. */
export async function prefillLiveSpool(store: DistillationSpoolClient, directory: string,
  sensors: SensorContractEnvelope, gib: number, limits: DistillationSpoolLimits,
  checkpoint: Readonly<DiskAdmissionRequest>, deadlineSeconds = 1800): Promise<LiveSpoolPrefillReceipt> {
  const initialFreeBytes = await freeBytes(directory);
  const plan = planLiveSpoolPrefill(gib, limits, initialFreeBytes, checkpoint, deadlineSeconds);
  const layout = JSON.parse(sensors.canonicalJson) as {inputSize: number};
  assert.equal(layout.inputSize, 504, 'full-spool live fixture requires admitted Motion/B32 metadata');
  const contract: TeacherCaptureContract = {captureEpoch: randomBytes(16).toString('hex'),
    runId: 'qualification-inactive-opaque-spool', configSha256: '23'.repeat(32),
    sensorSha256: sensors.sha256, teacherSha256: '37'.repeat(32), teacherRevision: 2, teacherMode: 1,
    controlIntervalSeconds: 1 / 60, inputSize: layout.inputSize};
  const ingestion = costs(); const started = performance.now();
  const batch = Math.floor(TEACHER_BATCH_BYTES / plan.objectBytes);
  assert(batch >= 1);
  for (let first = 1; first <= plan.objects;) {
    assert(performance.now() - started < plan.deadlineMs, 'explicit spool prefill total deadline');
    const count = Math.min(batch, plan.objects - first + 1); const before = performance.now();
    const dataset = await store.ingest(drain(contract, sensors.canonicalJson, first, count, plan.objectBytes), 42);
    settled(dataset, first + count - 1, first + count - 1, plan.objectBytes);
    sample(ingestion, performance.now() - before); first += count;
    if ((first - 1) % 1024 === 0 || first > plan.objects) console.info('[live-spool-prefill]', first - 1, '/', plan.objects);
  }
  await store.ingest(drain(contract, sensors.canonicalJson, plan.objects + 1, 0, plan.objectBytes, true), 42);
  assert(performance.now() - started < plan.deadlineMs, 'explicit spool prefill total deadline');
  const receipt: LiveSpoolPrefillReceipt = {kind: 'inactive-synthetic-opaque-spool-v1',
    captureEpoch: contract.captureEpoch, targetGiB: gib, objectBytes: plan.objectBytes, objects: plan.objects,
    beforeRecovery: {retainedChunks: plan.objects, retainedBytes: word(plan.targetBytes), resolvedThrough: word(plan.objects)},
    selectedMaxBytes: limits.maxBytes, selectedMaxChunks: limits.maxChunks,
    initialFreeBytes: initialFreeBytes.toString(), requiredFreeBytes: plan.requiredFreeBytes.toString(),
    ingestion, deadlineMs: plan.deadlineMs, elapsedSeconds: (performance.now() - started) / 1000};
  assert.deepEqual(await readLiveSpoolPrefillState(store, receipt), receipt.beforeRecovery);
  return receipt;
}

/** Reopened readiness must verify the original full fixture within its actual retained-byte allowance. */
export async function verifyLiveSpoolPrefillRecovery(store: Pick<DistillationSpoolClient, 'ready' | 'recoveryBudgetMs' | 'dataset'>,
  receipt: LiveSpoolPrefillReceipt, startedAt: number): Promise<void> {
  await store.ready;
  receipt.recoveryElapsedMs = performance.now() - startedAt;
  receipt.recoveryBudgetMs = store.recoveryBudgetMs;
  assert.equal(receipt.recoveryBudgetMs, distillationRecoveryTimeoutMs({chunks: receipt.beforeRecovery.retainedChunks,
    bytes: Number(BigInt('0x' + receipt.beforeRecovery.retainedBytes))}));
  receipt.afterRecovery = await readLiveSpoolPrefillState(store, receipt);
  assert.deepEqual(receipt.afterRecovery, receipt.beforeRecovery);
}
