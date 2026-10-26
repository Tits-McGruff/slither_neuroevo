/** Bound loaded-spool admission without allocating multi-GiB payloads in unit tests. */
import { expect, it } from 'vitest';
import { planLiveSpoolPrefill, readLiveSpoolPrefillState, verifyLiveSpoolPrefillRecovery, type LiveSpoolPrefillReceipt } from './liveSpoolPrefill.ts';
import { distillationSpoolLimitsForGiB } from '../../server/rustEngine/distillationProtocol.ts';
import { checkpointDiskAdmissionRequest, OPERATING_DISK_RESERVE_BYTES, SQLITE_WAL_ALLOWANCE_BYTES } from '../../server/rustEngine/diskAdmission.ts';
import type { DistillationDataset } from '../../src/protocol/distillation.ts';

/** Preserve the selected candidate publication allowance independently of spool occupancy. */
const checkpoint = checkpointDiskAdmissionRequest(768n * 1024n ** 2n);

it('admits exactly 64 GiB/65536 objects only when spool and future checkpoint reserves fit', () => {
  const limits = distillationSpoolLimitsForGiB(64);
  const required = 64n * 1024n ** 3n + OPERATING_DISK_RESERVE_BYTES + SQLITE_WAL_ALLOWANCE_BYTES
    + checkpoint.candidateSpoolBytes + checkpoint.finalManagedBytes;
  const plan = planLiveSpoolPrefill(64, limits, required, checkpoint);
  expect(plan).toEqual({objectBytes: 1024 ** 2, objects: 65536, targetBytes: 64 * 1024 ** 3, requiredFreeBytes: required, deadlineMs: 1800_000});
  expect(() => planLiveSpoolPrefill(64, limits, required - 1n, checkpoint)).toThrow(/reserves/u);
});

it('rejects fractional or over-quota/object fills and invalid disk arithmetic before allocation', () => {
  const free = 128n * 1024n ** 3n;
  for (const gib of [0, -1, 1.5, Number.NaN, 65]) {
    expect(() => planLiveSpoolPrefill(gib, distillationSpoolLimitsForGiB(64), free, checkpoint)).toThrow();
  }
  expect(() => planLiveSpoolPrefill(2, distillationSpoolLimitsForGiB(1), free, checkpoint)).toThrow(/quota/u);
  expect(() => planLiveSpoolPrefill(64, {...distillationSpoolLimitsForGiB(64), maxChunks: 65535}, free, checkpoint)).toThrow(/quota/u);
  expect(() => planLiveSpoolPrefill(1, distillationSpoolLimitsForGiB(64), free, {...checkpoint, sourceSpoolBytes: -1n})).toThrow(/nonnegative/u);
});

it('accepts ordinary old-fixture eviction but rejects a changed watermark, open epoch or byte count', async () => {
  const receipt = {captureEpoch: 'ab'.repeat(16), objects: 1024, objectBytes: 1024 ** 2};
  /** Metadata only; actual cold checksum verification is performed by the production worker. */
  const state = {state: 'stopped', productionWatermark: '0000000000000400', resolvedThrough: '0000000000000400',
    retainedChunks: 1000, retainedBytes: (1000 * 1024 ** 2).toString(16).padStart(16, '0')};
  const store = {dataset: async () => state as unknown as DistillationDataset};
  expect(await readLiveSpoolPrefillState(store, receipt)).toMatchObject({retainedChunks: 1000});
  for (const fault of [{state: 'active'}, {resolvedThrough: '00000000000003ff'},
    {retainedChunks: 1025}, {retainedBytes: '0000000000000000'}]) {
    await expect(readLiveSpoolPrefillState({dataset: async () => ({...state, ...fault}) as unknown as DistillationDataset}, receipt)).rejects.toThrow();
  }
});

it.each([1, 64])('cold-verifies %i GiB using the retained inventory and refuses a mismatched readiness allowance', async gib => {
  const objects = gib * 1024;
  const beforeRecovery = {retainedChunks: objects, retainedBytes: (gib * 1024 ** 3).toString(16).padStart(16, '0'),
    resolvedThrough: objects.toString(16).padStart(16, '0')};
  const receipt = {captureEpoch: 'ab'.repeat(16), objects, objectBytes: 1024 ** 2,
    targetGiB: gib, beforeRecovery} as LiveSpoolPrefillReceipt;
  let ready = false;
  const recoveryBudgetMs = gib === 64 ? 660_000 : 60_000;
  const store = {ready: Promise.resolve().then(() => {ready = true;}), recoveryBudgetMs,
    dataset: async () => {
      expect(ready).toBe(true);
      return {state: 'stopped', productionWatermark: beforeRecovery.resolvedThrough, ...beforeRecovery} as unknown as DistillationDataset;
    }};
  await verifyLiveSpoolPrefillRecovery(store, receipt, performance.now());
  expect(receipt.afterRecovery).toEqual(beforeRecovery);
  expect(receipt.recoveryBudgetMs).toBe(recoveryBudgetMs);
  await expect(verifyLiveSpoolPrefillRecovery({...store, recoveryBudgetMs: recoveryBudgetMs + 1},
    {...receipt}, performance.now())).rejects.toThrow();
});

it('bounds explicit physical-fill setup separately from production request and recovery deadlines', () => {
  const limits = distillationSpoolLimitsForGiB(64); const free = 128n * 1024n ** 3n;
  expect(planLiveSpoolPrefill(64, limits, free, checkpoint, 10_800).deadlineMs).toBe(10_800_000);
  for (const seconds of [0, -1, 1.5, Number.NaN, 10_801]) {
    expect(() => planLiveSpoolPrefill(64, limits, free, checkpoint, seconds)).toThrow(/setup deadline/u);
  }
});
