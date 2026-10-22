/** Frozen real-clock/job receipts are checked without executing archived code or world state. */
import { createHash } from 'node:crypto';
import { lstatSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { gunzipSync } from 'node:zlib';
import { expect, it } from 'vitest';
import type { DistillationDataset } from '../../src/protocol/distillation.ts';
import { summarizeSensorWall, type SensorWallRead, type SensorGenerationBarrier } from './sensorWallSummary.ts';

/** Exact checksums for one losslessly retained, bounded file. */
interface FrozenFileReceipt {bytes: number; sha256: string; gzipSha256: string}

/** Hash opaque bytes without interpreting populations or routing neural execution. */
function sha256(bytes: Uint8Array): string {
  return createHash('sha256').update(bytes).digest('hex');
}

/** Admit confined ordinary archive files before bounded decompression and retention. */
function readFrozenFiles(root: string, receipts: Record<string, FrozenFileReceipt>): Map<string, Buffer> {
  expect(Object.keys(receipts).length).toBeLessThanOrEqual(64);
  const files = new Map<string, Buffer>(); let total = 0;
  for (const [name, receipt] of Object.entries(receipts)) {
    expect(name).toMatch(/^(?:[a-zA-Z0-9_-]+\/)*[a-zA-Z0-9_-]+\.(?:json|py|ts|slither-brain)$/u);
    const path = resolve(root, name + '.gz'); const info = lstatSync(path);
    expect(info.isFile()).toBe(true); expect(info.isSymbolicLink()).toBe(false);
    expect(info.size).toBeLessThanOrEqual(4 * 1024 ** 2);
    const encoded = readFileSync(path); expect(sha256(encoded)).toBe(receipt.gzipSha256);
    const bytes = gunzipSync(encoded, {maxOutputLength: 4 * 1024 ** 2}); total += bytes.byteLength;
    expect(total).toBeLessThan(8 * 1024 ** 2);
    expect(bytes.byteLength).toBe(receipt.bytes); expect(sha256(bytes)).toBe(receipt.sha256);
    files.set(name, bytes);
  }
  return files;
}

/** Recompute the source identity using the actual archived owner's literal inventory. */
function frozenTrainerSource(files: Map<string, Buffer>, prefix: string): {names: string[]; sha256: string} {
  const declaration = files.get(prefix + 'distillation_job.py')!.toString('utf8')
    .match(/SOURCE_FILES = \(([\s\S]*?)\n\)/u);
  expect(declaration).not.toBeNull();
  const names = [...declaration![1]!.matchAll(/"([a-z_]+\.py)"/gu)].map(match => match[1]!);
  const hash = createHash('sha256').update('slither.distillation-trainer-source.v1\0');
  for (const name of names) hash.update(name + '\0').update(files.get(prefix + name)!);
  return {names, sha256: hash.digest('hex')};
}

/** Compact metadata emitted by the shared real native/trainer host. */
interface TrainerContentionEvidence {
  /** Explicit candidate resource gate; timing failures remain visible. */
  gate: string;
  device: string;
  workers: number;
  seconds: number;
  source: {hostSha256: string; trainerGateSha256: string; nativeSourceSha256: string;
    studentArtifactSha256: string; target: string};
  workload: {scenario: string; request: {profile: string; bins: number; rayCap: number; rayBatch: number};
    metadata: {parameterCount: number; sensorVersion: number}};
  train: {updates: number};
  resume: {updates: number; restartExact: boolean};
  sustain: {initialUpdates: number; updates: number; optimizerUpdates: number; requestedSeconds: number;
    activeWallSeconds: number; initialCursor: number; finalCursor: number; fetches: number;
    peakReplayBytes: number; peakRssBytes: number; peakDeviceBytes: number;
    sequence: {batch_size: number; burn_in_steps: number; bptt_steps: number}};
  summary: ReturnType<typeof summarizeSensorWall>;
  reads: SensorWallRead[];
  shadows: {before: Array<{artifactSha256: string; activationSequence: string; totals: {samples: string}}>;
    after: Array<{artifactSha256: string; activationSequence: string; totals: {samples: string}}>;
    sampleProgress: string[]};
  capture: {noDrops: boolean; after: DistillationDataset};
  frames: {copies: number; bytes: number; reusableBufferBytes: number};
  http: {forbiddenRequests: number};
}

/** Six-hour records add bounded progress, generation joins and cold periodic-job admission. */
type LiveSixHourEvidenceReport = TrainerContentionEvidence & {
  durability: {barriers: SensorGenerationBarrier[]; persistenceRestartExact: boolean;
    requestedBoundaries: number; restoredGeneration: string; restoredCompletedStep: string;
    restoredSourceSha256: string; invalidRestoreAdmissionsRejected: number};
  sustain: TrainerContentionEvidence['sustain'] & {durableSnapshots: number;
    trainerSourceSha256: string; progressWindows: Array<{activeWallSeconds: number; updates: number;
      cursor: number; fetches: number; snapshots: number; peakReplayBytes: number}>};
};

it('preserves six actual live hours, bounded durable progress and failed performance gates', () => {
  const root = resolve(import.meta.dirname, '../../docs/performance/sensor-v4/live-six-hour-debian-v1');
  const manifest = JSON.parse(readFileSync(resolve(root, 'evidence.json'), 'utf8')) as {
    kind: string; performanceQualified: boolean; fullCapacitySoakQualified: boolean;
    nativeSourceSha256: string; trainerSourceSha256: string; trainerSourceFiles: string[];
    files: Record<string, {bytes: number; sha256: string; gzipSha256: string}>};
  expect(manifest.kind).toBe('live-six-hour-window-evidence-v1');
  expect(manifest.performanceQualified).toBe(false); expect(manifest.fullCapacitySoakQualified).toBe(false);
  const files = readFrozenFiles(root, manifest.files);
  const report = JSON.parse(files.get('report.json')!.toString('utf8')) as LiveSixHourEvidenceReport;
  expect(report.gate).toBe('live-trainer-six-hour-window-v1');
  expect(report.device).toBe('cpu'); expect(report.workers).toBe(3); expect(report.seconds).toBe(21600);
  expect(report.source.hostSha256).toBe(sha256(files.get('slither/live-distillation.ts')!));
  expect(report.source.trainerGateSha256).toBe(sha256(files.get('pyrl/live_distillation_gate.py')!));
  expect(report.source.nativeSourceSha256).toBe(manifest.nativeSourceSha256);
  const trainer = frozenTrainerSource(files, 'pyrl/');
  expect(trainer.names).toEqual(manifest.trainerSourceFiles);
  expect(trainer.sha256).toBe(manifest.trainerSourceSha256);
  expect(report.sustain.trainerSourceSha256).toBe(manifest.trainerSourceSha256);
  expect(report.resume).toMatchObject({restartExact: true, updates: 250});
  expect(report.sustain.activeWallSeconds).toBeGreaterThanOrEqual(21600);
  expect(report.sustain.updates - report.sustain.initialUpdates).toBe(report.sustain.optimizerUpdates);
  expect(report.sustain.optimizerUpdates).toBe(27688); expect(report.sustain.durableSnapshots).toBe(35);
  expect(report.sustain.peakReplayBytes).toBeLessThanOrEqual(256 * 1024 ** 2);
  let previous = {activeWallSeconds: 0, updates: report.sustain.initialUpdates,
    cursor: report.sustain.initialCursor, snapshots: 0};
  expect(report.sustain.progressWindows.length).toBeGreaterThanOrEqual(180);
  expect(report.sustain.progressWindows.length).toBeLessThanOrEqual(362);
  for (const progress of report.sustain.progressWindows) {
    expect(progress.activeWallSeconds - previous.activeWallSeconds).toBeGreaterThan(0);
    expect(progress.activeWallSeconds - previous.activeWallSeconds).toBeLessThanOrEqual(120);
    expect(progress.updates).toBeGreaterThan(previous.updates); expect(progress.cursor).toBeGreaterThan(previous.cursor);
    expect(progress.snapshots).toBeGreaterThanOrEqual(previous.snapshots);
    expect(progress.peakReplayBytes).toBeLessThanOrEqual(256 * 1024 ** 2); previous = progress;
  }
  expect(report.sustain.activeWallSeconds - previous.activeWallSeconds).toBeLessThanOrEqual(120);
  expect(report.durability.barriers).toHaveLength(72);
  expect(report.durability).toMatchObject({requestedBoundaries: 72, persistenceRestartExact: true,
    restoredSourceSha256: manifest.nativeSourceSha256, invalidRestoreAdmissionsRejected: 7});
  const last = report.durability.barriers.at(-1)!;
  expect(report.durability.restoredGeneration).toBe(last.checkpoint.generation);
  expect(report.durability.restoredCompletedStep).toBe(last.checkpoint.completedStep);
  expect(report.summary).toEqual(summarizeSensorWall(report.reads, report.seconds,
    {runId: last.checkpoint.runId, barriers: report.durability.barriers}, true));
  expect(report.summary.progress.deltaSteps).toBe('1294095');
  expect(report.summary.sixHourWall?.workComplete).toBe(false);
  expect(report.summary.stepTiming.p99UpperMs).toBe(48); expect(report.summary.meetsMeasuredGates).toBe(false);
  expect(report.capture.noDrops).toBe(true); expect(report.capture.after.state).toBe('stopped');
  expect(report.capture.after.resolvedThrough).toBe(report.capture.after.productionWatermark);
  expect(report.shadows.after).toHaveLength(4);
  for (const [slot, after] of report.shadows.after.entries()) {
    const before = report.shadows.before[slot]!;
    expect(after.artifactSha256).toBe(before.artifactSha256);
    expect(after.activationSequence).toBe(before.activationSequence);
    const delta = BigInt('0x' + after.totals.samples) - BigInt('0x' + before.totals.samples);
    expect(delta).toBeGreaterThan(0n); expect(delta.toString()).toBe(report.shadows.sampleProgress[slot]);
  }
  const restored = JSON.parse(files.get('checkpoint-restore.json')!.toString('utf8')) as {
    trainerSourceSha256: string; retainedCheckpointFiles: number; restoredUpdates: number;
    replayOwnedBytes: number; artifactSha256: string; weightsSha256: string};
  expect(restored.trainerSourceSha256).toBe(manifest.trainerSourceSha256);
  expect(restored.retainedCheckpointFiles).toBe(3); expect(restored.restoredUpdates).toBe(27532);
  expect(restored.replayOwnedBytes).toBeLessThanOrEqual(256 * 1024 ** 2);
  expect(sha256(files.get('periodic-student.slither-brain')!)).toBe(restored.artifactSha256);
  const artifact = JSON.parse(files.get('periodic-student-native.json')!.toString('utf8')) as {
    artifactSha256: string; weightsSha256: string; parameterCount: number};
  expect(artifact).toMatchObject({artifactSha256: restored.artifactSha256,
    weightsSha256: restored.weightsSha256, parameterCount: 25042});
});


/** Current-process dispatch counters never stand in for lifetime optimizer work. */
interface WindowDispatch {
  requested: string; compiledSignatures: number; compiledWindowCalls: number; eagerWindowCalls: number;
}
/** Additional diagnostics from the automatic durable CUDA job owner. */
type CudaJobEvidenceReport = TrainerContentionEvidence & {
  train: {environment: {sequenceExecution: string}; windowExecution: WindowDispatch;
    processPriority?: {platform: string; priorityClass: number}};
  resume: {windowExecution: WindowDispatch; processPriority?: {platform: string; priorityClass: number}};
  sustain: {windowExecution: WindowDispatch; trainerSourceSha256: string; peakReservedDeviceBytes: number;
    processPriority?: {platform: string; priorityClass: number}};
};

it('preserves compiled CUDA job restart, observed scheduling policy and both passing and failing windows', () => {
  const root = resolve(import.meta.dirname, '../../docs/performance/sensor-v4/cuda-job-windows-v1');
  const manifest = JSON.parse(readFileSync(resolve(root, 'evidence.json'), 'utf8')) as {
    releaseQualified: boolean; nativeSourceSha256: string; files: Record<string, FrozenFileReceipt>;
    sources: Record<string, {trainerSourceSha256: string; trainerSourceFiles: string[]}>;
    reports: Record<string, {sourceVariant: string; scenario: string; meetsMeasuredGates: boolean}>};
  expect(manifest.releaseQualified).toBe(false);
  const files = readFrozenFiles(root, manifest.files);
  for (const [variant, source] of Object.entries(manifest.sources)) {
    const actual = frozenTrainerSource(files, variant + '/pyrl/');
    expect(actual.names).toEqual(source.trainerSourceFiles);
    expect(actual.sha256).toBe(source.trainerSourceSha256);
  }
  expect(Object.keys(manifest.reports).sort()).toEqual([
    'best-effort-P0.json', 'best-effort-P1.json', 'compiled-P0.json']);
  for (const [name, receipt] of Object.entries(manifest.reports)) {
    const report = JSON.parse(files.get(name)!.toString('utf8')) as CudaJobEvidenceReport;
    expect(report.gate).toBe('live-trainer-four-shadow-contention-v1');
    expect(report.device).toBe('cuda:0'); expect(report.workers).toBe(3); expect(report.seconds).toBe(60);
    expect(report.workload.scenario).toBe(receipt.scenario);
    expect(report.workload.metadata.parameterCount).toBe(25042);
    expect(report.source.hostSha256).toBe(sha256(files.get('host.ts')!));
    expect(report.source.trainerGateSha256)
      .toBe(sha256(files.get(receipt.sourceVariant + '/pyrl/live_distillation_gate.py')!));
    expect(report.source.nativeSourceSha256).toBe(manifest.nativeSourceSha256);
    expect(report.source.studentArtifactSha256).toBe(sha256(files.get('student.slither-brain')!));
    expect(report.sustain.trainerSourceSha256)
      .toBe(manifest.sources[receipt.sourceVariant]!.trainerSourceSha256);
    expect(report.train.environment.sequenceExecution).toBe('bounded-cuda-window-v1');
    expect(report.train.windowExecution.compiledWindowCalls).toBe(126);
    expect(report.resume).toMatchObject({restartExact: true, updates: 250});
    expect(report.resume.windowExecution.compiledWindowCalls).toBe(125);
    expect(report.sustain.windowExecution.compiledWindowCalls).toBe(report.sustain.optimizerUpdates);
    for (const phase of [report.train, report.resume, report.sustain]) {
      expect(phase.windowExecution).toMatchObject({
        requested: 'cudagraphs', compiledSignatures: 1, eagerWindowCalls: 0});
      if (receipt.sourceVariant === 'best-effort')
        expect(phase.processPriority).toEqual({platform: 'windows', priorityClass: 16384});
    }
    expect(report.sustain.activeWallSeconds).toBeGreaterThanOrEqual(60);
    expect(report.sustain.optimizerUpdates).toBeGreaterThan(0);
    expect(report.sustain.finalCursor).toBeGreaterThan(report.sustain.initialCursor);
    expect(report.sustain.peakReplayBytes).toBeLessThanOrEqual(256 * 1024 ** 2);
    expect(report.sustain.peakDeviceBytes).toBeLessThanOrEqual(8 * 1024 ** 3);
    expect(report.sustain.peakReservedDeviceBytes).toBeLessThanOrEqual(8 * 1024 ** 3);
    expect(report.summary).toEqual(summarizeSensorWall(report.reads, report.seconds));
    expect(report.summary.meetsMeasuredGates).toBe(receipt.meetsMeasuredGates);
    expect(report.capture.noDrops).toBe(true); expect(report.capture.after.state).toBe('stopped');
    expect(report.capture.after.resolvedThrough).toBe(report.capture.after.productionWatermark);
    expect(report.http.forbiddenRequests).toBe(0); expect(report.frames.copies).toBeGreaterThan(0);
    for (const [slot, after] of report.shadows.after.entries()) {
      const before = report.shadows.before[slot]!;
      expect(after.artifactSha256).toBe(before.artifactSha256);
      expect(after.activationSequence).toBe(before.activationSequence);
      const delta = BigInt('0x' + after.totals.samples) - BigInt('0x' + before.totals.samples);
      expect(delta).toBeGreaterThan(0n); expect(delta.toString()).toBe(report.shadows.sampleProgress[slot]);
    }
  }
});
