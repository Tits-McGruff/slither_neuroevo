/** Shared opaque storage fixtures and bounded measurements; no native sample codec. */
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { statfs } from 'node:fs/promises';
import type { DistillationDataset, TeacherCaptureDrain, TeacherCaptureContract } from '../../src/protocol/distillation.ts';
import { DISTILLATION_ZERO } from '../../server/rustEngine/distillationProtocol.ts';

/** Fixed-size latency histogram, independent of the number of stored objects. */
export interface Costs {
  /** Completed operation count. */
  samples: number;
  /** Aggregate elapsed milliseconds. */
  totalMs: number;
  /** Largest observed elapsed milliseconds. */
  maximumMs: number;
  /** Inclusive bucket ceilings in milliseconds. */
  upperMs: number[];
  /** Exactly one bucket per completed operation. */
  counts: number[];
}
/** Preserve exact unsigned metadata ordering without a JavaScript sample decoder. */
export function word(value: number): string { return value.toString(16).padStart(16, '0'); }
/** Hash bytes for opaque-object verification and canonical-LF source provenance. */
export function digest(value: Uint8Array): string { return createHash('sha256').update(value).digest('hex'); }
/** Recreate one bounded synthetic storage object, distinct for every sequence. */
export function object(sequence: number, size: number): Buffer {
  const bytes = Buffer.alloc(size, 0xa5); bytes.writeBigUInt64LE(BigInt(sequence)); return bytes;
}
/** Describe opaque test objects; this intentionally does not encode native TeacherSampleV1. */
export function drain(contract: TeacherCaptureContract, sensors: string, first: number, count: number, size: number, stopped = false): TeacherCaptureDrain {
  const chunks = Array.from({length: count}, (_, offset) => {
    const sequence = first + offset; const bytes = object(sequence, size);
    return {descriptor: {captureEpoch: contract.captureEpoch, sequence: word(sequence), generation: word(1), baselineSlot: 0,
      physicalSnakeId: word(1), firstSample: word(sequence), lastSample: word(sequence), firstStep: word(sequence), lastStep: word(sequence),
      count: 1, inputSize: contract.inputSize, configSha256: contract.configSha256, sensorSha256: contract.sensorSha256,
      teacherSha256: contract.teacherSha256, startReason: 'continuation' as const, endReason: 'simulatedSecond' as const,
      byteCount: bytes.byteLength, sha256: digest(bytes)}, bytes};
  });
  const last = count ? first + count - 1 : first - 1;
  return {contract, sensorContractJson: sensors, chunks, productionWatermark: word(last), releasedThrough: word(last),
    queuedChunks: 0, queuedBytes: DISTILLATION_ZERO, droppedChunks: DISTILLATION_ZERO, droppedSamples: DISTILLATION_ZERO,
    droppedBytes: DISTILLATION_ZERO, stopped, moreWork: false};
}
/** Allocate a fixed histogram whose final bucket exceeds every operation deadline. */
export function costs(): Costs { return {samples: 0, totalMs: 0, maximumMs: 0, upperMs: Array.from({length: 32}, (_, i) => 0.25 * 2 ** i), counts: Array<number>(32).fill(0)}; }
/** Attribute one completed operation without retaining per-object timing arrays. */
export function sample(target: Costs, milliseconds: number): void {
  target.samples++; target.totalMs += milliseconds; target.maximumMs = Math.max(target.maximumMs, milliseconds);
  const bucket = target.upperMs.findIndex(upper => milliseconds <= upper);
  assert(bucket >= 0); target.counts[bucket]!++;
}
/** Read actual available physical space on the explicitly selected filesystem. */
export async function freeBytes(path: string): Promise<bigint> { const fs = await statfs(path, {bigint: true}); return fs.bavail * fs.bsize; }
/** Require exact durable watermarks and absence of fabricated native/storage drops. */
export function settled(dataset: DistillationDataset, last: number, retained: number, bytes: number): void {
  assert.equal(dataset.productionWatermark, word(last)); assert.equal(dataset.storageWatermark, word(last));
  assert.equal(dataset.resolvedThrough, word(last)); assert.equal(dataset.retainedChunks, retained);
  assert.equal(dataset.retainedBytes, word(retained * bytes)); assert.equal(dataset.nativeDroppedChunks, DISTILLATION_ZERO);
  assert.equal(dataset.nativeDroppedSamples, DISTILLATION_ZERO); assert.equal(dataset.spoolDroppedChunks, DISTILLATION_ZERO);
}
