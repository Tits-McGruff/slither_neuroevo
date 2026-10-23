/** Frozen real-clock/job receipts are checked without executing archived code or world state. */
import { createHash } from 'node:crypto';
import { lstatSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { gunzipSync } from 'node:zlib';
import { expect, it } from 'vitest';
import type { DistillationDataset } from '../../src/protocol/distillation.ts';
import { distillationRecoveryTimeoutMs } from '../../server/rustEngine/distillationProtocol.ts';
import type { NativeStepCostRead } from './nativeStepCosts.ts';
import type { LiveSpoolPrefillReceipt } from './liveSpoolPrefill.ts';
import type { LiveCatalogReceipt } from './liveCapacityLibrary.ts';
import type { BrainArtifactDescriptor } from '../../src/protocol/brainArtifacts.ts';
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
    expect(name).toMatch(/^(?:[a-zA-Z0-9_-]+\/)*[a-zA-Z0-9_-]+\.(?:json|py|ts|rs|slither-brain)$/u);
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


/** Exact initial families accompany a short live window; they do not imply policy maturity. */
type LearnedCohortEvidence = TrainerContentionEvidence & {
  durability: LiveSixHourEvidenceReport['durability'];
  workload: TrainerContentionEvidence['workload'] & {trainedCohort: {
    indexSha256: string; sourceReservationBytes: number;
    families: {artifact: BrainArtifactDescriptor; populationSlots: number}[];
  }};
  source: TrainerContentionEvidence['source'] & {slitherRevision: string; pyrlRevision: string};
  sustain: TrainerContentionEvidence['sustain'] & {trainerSourceSha256: string;
    environment: {device: string; threads: number; sequenceExecution: string};
    processPriority: {platform: string; nice: number}; windowExecution: WindowDispatch};
};

it('preserves exact learned initial families and cold generation durability without hiding failed throughput', () => {
  const root = resolve(import.meta.dirname, '../../docs/performance/sensor-v4/live-cohort-debian-v1');
  const manifest = JSON.parse(readFileSync(resolve(root, 'evidence.json'), 'utf8')) as {
    kind: string; performanceQualified: boolean; maturityQualified: boolean; slitherCommit: string; pyrlCommit: string;
    nativeSourceSha256: string; trainerSourceSha256: string; trainerSourceFiles: string[];
    files: Record<string, FrozenFileReceipt>};
  expect(manifest.kind).toBe('live-learned-cohort-window-evidence-v1');
  expect(manifest.performanceQualified).toBe(false); expect(manifest.maturityQualified).toBe(false);
  const files = readFrozenFiles(root, manifest.files);
  const report = JSON.parse(files.get('report.json')!.toString('utf8')) as LearnedCohortEvidence;
  expect(report.source.slitherRevision).toBe(manifest.slitherCommit);
  expect(report.source.pyrlRevision).toBe(manifest.pyrlCommit);
  expect(report.source.nativeSourceSha256).toBe(manifest.nativeSourceSha256);
  expect(report.source.hostSha256).toBe(sha256(files.get('host/live-distillation.ts')!));
  expect(report.source.trainerGateSha256).toBe(sha256(files.get('pyrl/live_distillation_gate.py')!));
  expect(frozenTrainerSource(files, 'pyrl/')).toEqual({
    names: manifest.trainerSourceFiles, sha256: manifest.trainerSourceSha256,
  });
  expect(report.sustain.trainerSourceSha256).toBe(manifest.trainerSourceSha256);
  expect(report).toMatchObject({device: 'cpu', workers: 8, seconds: 60,
    workload: {scenario: 'P1', metadata: {parameterCount: 25042, sensorVersion: 4}}});
  const cohort = report.workload.trainedCohort;
  const indexBytes = files.get('cohort.json')!;
  const index = JSON.parse(indexBytes.toString('utf8')) as {schemaVersion: number; artifacts: {file: string; sha256: string}[]};
  expect(index.schemaVersion).toBe(1); expect(cohort.indexSha256).toBe(sha256(indexBytes));
  expect(cohort.sourceReservationBytes).toBeGreaterThan(0);
  expect(cohort.sourceReservationBytes).toBeLessThanOrEqual(512 * 1024 ** 2);
  expect(index.artifacts).toHaveLength(4); expect(cohort.families).toHaveLength(4);
  expect(new Set(cohort.families.map(family => family.artifact.weightsSha256)).size).toBe(4);
  for (const [ordinal, family] of cohort.families.entries()) {
    const member = index.artifacts[ordinal]!;
    expect(member.file).toMatch(/^[a-zA-Z0-9-]+\.slither-brain$/u);
    expect(sha256(files.get(member.file)!)).toBe(member.sha256);
    expect(family.artifact.artifactSha256).toBe(member.sha256);
    expect(family.populationSlots).toBe(75);
    expect(family.artifact).toMatchObject({parameterCount: 25042, inputSize: 264, stateSize: 16,
      controlIntervalSeconds: 1 / 60, lifeStateRule: 'reset', sourceKind: 'distilled'});
  }
  expect(report.resume).toMatchObject({restartExact: true, updates: 250});
  expect(report.sustain.initialUpdates).toBe(250);
  expect(report.sustain.updates - report.sustain.initialUpdates).toBe(report.sustain.optimizerUpdates);
  expect(report.sustain.optimizerUpdates).toBe(250);
  expect(report.sustain.peakReplayBytes).toBeLessThanOrEqual(256 * 1024 ** 2);
  expect(report.sustain.environment).toMatchObject({device: 'cpu', threads: 2, sequenceExecution: 'eager-v1'});
  expect(report.sustain.processPriority).toEqual({platform: 'posix', nice: 5});
  expect(report.sustain.windowExecution).toEqual({
    compiledSignatures: 0, compiledWindowCalls: 0, eagerWindowCalls: 250, requested: 'eager',
  });
  expect(report.durability).toMatchObject({requestedBoundaries: 2, persistenceRestartExact: true,
    restoredSourceSha256: manifest.nativeSourceSha256, invalidRestoreAdmissionsRejected: 7});
  expect(report.durability.barriers).toHaveLength(2);
  const last = report.durability.barriers.at(-1)!;
  expect(report.durability.restoredGeneration).toBe(last.checkpoint.generation);
  expect(report.durability.restoredCompletedStep).toBe(last.checkpoint.completedStep);
  expect(report.summary).toEqual(summarizeSensorWall(report.reads, report.seconds,
    {runId: last.checkpoint.runId, barriers: report.durability.barriers}));
  expect(report.summary.progress.deltaSteps).toBe('1692');
  expect(report.summary.stepTiming.p99UpperMs).toBe(96); expect(report.summary.meetsMeasuredGates).toBe(false);
  expect(report.capture.noDrops).toBe(true); expect(report.capture.after.state).toBe('stopped');
  expect(report.capture.after.resolvedThrough).toBe(report.capture.after.productionWatermark);
  expect(report.shadows.after).toHaveLength(4);
  for (const [slot, after] of report.shadows.after.entries()) {
    const before = report.shadows.before[slot]!;
    const delta = BigInt('0x' + after.totals.samples) - BigInt('0x' + before.totals.samples);
    expect(delta).toBeGreaterThan(0n); expect(delta.toString()).toBe(report.shadows.sampleProgress[slot]);
  }
});

/** A persisted replay ceiling is distinct from real occupied replay in each trainer process. */
interface ReplayCapacityReceipt {
  replayCapacityBytes: number; replayBytes: number; trainerSourceSha256: string;
  processPriority: {platform: string; nice: number};
}
/** Short live capacity receipts retain ordinary generation chronology and explicit occupied bytes. */
type LiveReplayCeilingEvidence = TrainerContentionEvidence & {
  replayMiB: number;
  source: TrainerContentionEvidence['source'] & {slitherRevision: string; pyrlRevision: string};
  train: TrainerContentionEvidence['train'] & ReplayCapacityReceipt;
  resume: TrainerContentionEvidence['resume'] & ReplayCapacityReceipt;
  sustain: TrainerContentionEvidence['sustain'] & ReplayCapacityReceipt;
  durability: LiveSixHourEvidenceReport['durability'];
};

it('preserves the max live replay ceiling across cold restart without claiming filled capacity or passing timing', () => {
  const root = resolve(import.meta.dirname, '../../docs/performance/sensor-v4/live-replay-ceiling-debian-v1');
  const manifest = JSON.parse(readFileSync(resolve(root, 'evidence.json'), 'utf8')) as {
    kind: string; replayCeilingRestartQualified: boolean; loadedReplayQualified: boolean;
    fullCapacitySoakQualified: boolean; performanceQualified: boolean; maturityQualified: boolean;
    slitherCommit: string; pyrlCommit: string; nativeSourceSha256: string; trainerSourceSha256: string;
    trainerSourceFiles: string[]; trainerSourcesFrom: string;
    studentInput: {evidence: string; file: string; sha256: string}; files: Record<string, FrozenFileReceipt>;
  };
  expect(manifest.kind).toBe('live-replay-ceiling-window-evidence-v1');
  expect(manifest.replayCeilingRestartQualified).toBe(true);
  for (const qualified of [manifest.loadedReplayQualified, manifest.fullCapacitySoakQualified,
    manifest.performanceQualified, manifest.maturityQualified]) expect(qualified).toBe(false);
  const files = readFrozenFiles(root, manifest.files);
  const report = JSON.parse(files.get('report.json')!.toString('utf8')) as LiveReplayCeilingEvidence;
  expect(report.source).toMatchObject({slitherRevision: manifest.slitherCommit, pyrlRevision: manifest.pyrlCommit,
    nativeSourceSha256: manifest.nativeSourceSha256});
  expect(report.source.hostSha256).toBe(sha256(files.get('host/live-distillation.ts')!));
  expect(report.source.trainerGateSha256).toBe(sha256(files.get('pyrl/live_distillation_gate.py')!));
  expect(manifest.trainerSourcesFrom).toBe('../live-cohort-debian-v1');
  expect(manifest.studentInput.evidence).toBe(manifest.trainerSourcesFrom);
  const priorRoot = resolve(root, manifest.trainerSourcesFrom);
  const prior = JSON.parse(readFileSync(resolve(priorRoot, 'evidence.json'), 'utf8')) as {
    files: Record<string, FrozenFileReceipt>};
  const priorFiles = readFrozenFiles(priorRoot, prior.files);
  expect(frozenTrainerSource(priorFiles, 'pyrl/')).toEqual({
    names: manifest.trainerSourceFiles, sha256: manifest.trainerSourceSha256});
  expect(sha256(priorFiles.get(manifest.studentInput.file)!)).toBe(manifest.studentInput.sha256);
  expect(report.source.studentArtifactSha256).toBe(manifest.studentInput.sha256);
  expect(report).toMatchObject({device: 'cpu', workers: 4, seconds: 60, replayMiB: 2048,
    workload: {scenario: 'P0', metadata: {parameterCount: 25042, sensorVersion: 4}}});
  for (const phase of [report.train, report.resume, report.sustain]) {
    expect(phase.replayCapacityBytes).toBe(2048 * 1024 ** 2);
    expect(phase.replayBytes).toBeGreaterThan(0);
    expect(phase.replayBytes).toBeLessThan(phase.replayCapacityBytes / 10);
    expect(phase.trainerSourceSha256).toBe(manifest.trainerSourceSha256);
    expect(phase.processPriority).toEqual({platform: 'posix', nice: 5});
  }
  expect(report.train.updates).toBe(126);
  expect(report.resume).toMatchObject({restartExact: true, updates: 250});
  expect(report.sustain.initialUpdates).toBe(250);
  expect(report.sustain.updates - report.sustain.initialUpdates).toBe(report.sustain.optimizerUpdates);
  expect(report.sustain.optimizerUpdates).toBe(277);
  expect(report.sustain.peakReplayBytes).toBe(109272776);
  expect(report.sustain.activeWallSeconds).toBeGreaterThanOrEqual(60);
  expect(report.sustain.finalCursor).toBeGreaterThan(report.sustain.initialCursor);
  expect(report.sustain.fetches).toBeGreaterThan(0);
  expect(report.sustain.sequence).toMatchObject({batch_size: 32, burn_in_steps: 32, bptt_steps: 64});
  expect(report.durability).toMatchObject({requestedBoundaries: 2, persistenceRestartExact: true,
    restoredSourceSha256: manifest.nativeSourceSha256, invalidRestoreAdmissionsRejected: 7});
  expect(report.durability.barriers).toHaveLength(2);
  const durability = {runId: report.durability.barriers[0]!.checkpoint.runId, barriers: report.durability.barriers};
  expect(summarizeSensorWall(report.reads, 60, durability)).toEqual(report.summary);
  expect(report.summary.stepTiming.p99UpperMs).toBe(48);
  expect(report.summary.droppedWallMicros).toBe('127667');
  expect(report.summary.meetsMeasuredGates).toBe(false);
  expect(report.capture.noDrops).toBe(true);
  expect(report.http.forbiddenRequests).toBe(0);
  expect(report.shadows.sampleProgress).toHaveLength(4);
  for (const value of report.shadows.sampleProgress) expect(BigInt(value)).toBeGreaterThan(0n);
  expect(report.frames.copies).toBeGreaterThan(0);
  const observerSha = sha256(files.get('host/system-resource-observer.py')!);
  const resources = JSON.parse(files.get('aggregate-resources.json')!.toString('utf8')) as Array<{
    platform: string; processDetailsIncluded: boolean; observerSourceSha256: string;
    host: Record<string, unknown>; [key: string]: unknown}>;
  expect(resources.length).toBeGreaterThanOrEqual(3);
  for (const observation of resources) {
    expect(observation.platform).toBe('Linux');
    expect(observation.observerSourceSha256).toBe(observerSha);
    expect(observation.processDetailsIncluded).toBe(false);
    expect(observation).not.toHaveProperty('gpuProcesses');
    expect(observation).not.toHaveProperty('wsl');
    expect(Object.keys(observation.host).every(key =>
      ['available', 'cpuTicks', 'loadAverage', 'memoryKiB'].includes(key))).toBe(true);
  }
});

/** A measured large-population/catalog window preserves explicit unfilled storage and failed timing. */
type LiveLargeCatalogEvidence = LiveReplayCeilingEvidence & {
  source: {slitherRevision: string; pyrlRevision: string; libraryHelperSha256: string};
  workload: {capacityGraph: string; checkpointEnvelopeMiB: number; capacityAudit: {
    population: number; parametersPerGenome: number; decodedRoleBytes: number[]; decodedBytes: number;
    conservativeTwoStateBytes: number; processMemoryCeilingBytes: number;
    decodedEnvelopeFits: boolean; twoStateBudgetFits: boolean; workerScratchBytes: number; checkpointScratchBytes: number;
    limits: {maxTotalDecodedBytes: number}}};
  catalogOverlap: {fixtureIndexSha256: string; before: LiveCatalogReceipt; afterColdRestart: LiveCatalogReceipt;
    overflowRejected: boolean; overflowPublishedFiles: boolean};
  inspectorOverlap: {before: {subscriptions: number; deliveredSamples: number};
    after: {subscriptions: number; deliveredSamples: number; maximumWireBytes: number}; pureProbe: boolean; inputSize: number};
  spoolGiB: number; producerStoragePolicy: {maxBytes: string}; hostPeakRssBytes: number;
};

it('preserves real large-population/full-catalog/inspector overlap and the failed performance result', () => {
  const root = resolve(import.meta.dirname, '../../docs/performance/sensor-v4/live-large-catalog-debian-v1');
  const manifest = JSON.parse(readFileSync(resolve(root, 'evidence.json'), 'utf8')) as {
    kind: string; largeCatalogOverlapQualified: boolean; performanceQualified: boolean;
    loadedReplayQualified: boolean; loadedSpoolQualified: boolean; fullCapacitySoakQualified: boolean;
    slitherCommit: string; pyrlCommit: string; nativeSourceSha256: string; trainerSourceSha256: string;
    trainerSourceFiles: string[]; files: Record<string, FrozenFileReceipt>};
  expect(manifest).toMatchObject({kind: 'live-large-catalog-window-evidence-v1', largeCatalogOverlapQualified: true,
    performanceQualified: false, loadedReplayQualified: false, loadedSpoolQualified: false, fullCapacitySoakQualified: false});
  const files = readFrozenFiles(root, manifest.files);
  const report = JSON.parse(files.get('report.json')!.toString('utf8')) as LiveLargeCatalogEvidence;
  expect(report.source).toMatchObject({slitherRevision: manifest.slitherCommit, pyrlRevision: manifest.pyrlCommit,
    nativeSourceSha256: manifest.nativeSourceSha256, target: 'x86_64-unknown-linux-gnu',
    hostSha256: sha256(files.get('host/live-distillation.ts')!),
    libraryHelperSha256: sha256(files.get('host/liveCapacityLibrary.ts')!),
    trainerGateSha256: sha256(files.get('pyrl/live_distillation_gate.py')!),
    studentArtifactSha256: sha256(files.get('large-motion32-student.slither-brain')!)});
  const trainer = frozenTrainerSource(files, 'pyrl/');
  expect(trainer.names).toEqual(manifest.trainerSourceFiles); expect(trainer.sha256).toBe(manifest.trainerSourceSha256);
  const fixture = JSON.parse(files.get('fixture-index.json')!.toString('utf8')) as {
    schemaVersion: number; records: {brainId: string; weightsSha256: string; artifactByteCount: string; parameterCount: number}[]};
  expect(fixture.schemaVersion).toBe(1); expect(fixture.records).toHaveLength(101);
  expect(new Set(fixture.records.map(record => record.brainId)).size).toBe(101);
  expect(new Set(fixture.records.map(record => record.weightsSha256)).size).toBe(101);
  expect(fixture.records.every(record => record.parameterCount === 494306)).toBe(true);
  expect(sha256(files.get('native-manual-source.slither-brain')!)).toBe(fixture.records[0]!.brainId);
  expect(report.catalogOverlap.fixtureIndexSha256).toBe(sha256(files.get('fixture-index.json')!));
  expect(JSON.parse(files.get('fixture-generation.json')!.toString('utf8'))).toMatchObject({
    sourceSha256: manifest.nativeSourceSha256, libraryFixture: {recordCount: 100, overflowProbes: 1}});
  expect(report).toMatchObject({device: 'cpu', workers: 4, seconds: 60, spoolGiB: 64, replayMiB: 2048,
    workload: {scenario: 'P1', capacityGraph: 'large', checkpointEnvelopeMiB: 768,
      request: {profile: 'motion', bins: 32}, metadata: {parameterCount: 494306, sensorVersion: 4}}});
  const audit = report.workload.capacityAudit;
  expect(audit).toMatchObject({population: 300, parametersPerGenome: 494306, decodedEnvelopeFits: true,
    twoStateBudgetFits: true, workerScratchBytes: 512 * 1024 ** 2, checkpointScratchBytes: 512 * 1024 ** 2});
  expect(audit.decodedRoleBytes[3]).toBe(300 * 494306 * 4);
  expect(audit.decodedBytes).toBeGreaterThan(512 * 1024 ** 2);
  expect(audit.decodedBytes).toBeLessThanOrEqual(768 * 1024 ** 2);
  expect(audit.limits.maxTotalDecodedBytes).toBe(768 * 1024 ** 2);
  expect(audit.conservativeTwoStateBytes).toBeLessThanOrEqual(audit.processMemoryCeilingBytes);
  const catalog = report.catalogOverlap;
  expect(catalog.before).toMatchObject({pages: 4, recordCount: 100});
  expect(catalog.before.brainIds).toEqual(fixture.records.slice(0, 100).map(record => record.brainId).sort());
  expect(catalog.before.originalBytes).toBe(fixture.records.slice(0, 100)
    .reduce((sum, record) => sum + BigInt('0x' + record.artifactByteCount), 0n).toString());
  expect(catalog.afterColdRestart).toEqual(catalog.before);
  expect(catalog.overflowRejected).toBe(true); expect(catalog.overflowPublishedFiles).toBe(false);
  const inspector = report.inspectorOverlap;
  expect(inspector.pureProbe).toBe(false); expect(inspector.inputSize).toBe(504);
  expect(inspector.after.deliveredSamples).toBeGreaterThan(inspector.before.deliveredSamples);
  expect(inspector.after.subscriptions).toBeGreaterThan(inspector.before.subscriptions);
  expect(inspector.after.maximumWireBytes).toBeLessThan(1024 * 1024);
  expect(report.resume).toMatchObject({restartExact: true, updates: 250});
  expect(report.sustain.initialUpdates).toBe(250);
  expect(report.sustain.updates - report.sustain.initialUpdates).toBe(report.sustain.optimizerUpdates);
  expect(report.sustain.optimizerUpdates).toBe(79);
  expect(report.sustain.sequence).toMatchObject({batch_size: 32, burn_in_steps: 32, bptt_steps: 64});
  for (const phase of [report.train, report.resume, report.sustain]) {
    expect(phase.trainerSourceSha256).toBe(manifest.trainerSourceSha256);
    expect(phase.replayCapacityBytes).toBe(2048 * 1024 ** 2);
    expect(phase.processPriority).toEqual({platform: 'posix', nice: 5});
  }
  expect(report.sustain.activeWallSeconds).toBeGreaterThanOrEqual(60);
  expect(report.sustain.finalCursor).toBeGreaterThan(report.sustain.initialCursor);
  expect(report.sustain.peakReplayBytes).toBe(88070918);
  expect(report.sustain.peakReplayBytes).toBeLessThan(report.sustain.replayCapacityBytes / 10);
  expect(BigInt('0x' + report.producerStoragePolicy.maxBytes)).toBe(64n * 1024n ** 3n);
  expect(BigInt('0x' + report.capture.after.retainedBytes)).toBeLessThan(64n * 1024n ** 3n / 10n);
  expect(report.durability).toMatchObject({requestedBoundaries: 3, persistenceRestartExact: true,
    restoredSourceSha256: manifest.nativeSourceSha256, invalidRestoreAdmissionsRejected: 7});
  expect(report.durability.barriers).toHaveLength(3);
  expect(report.durability.barriers.every(barrier => barrier.terminal.population === 300)).toBe(true);
  const last = report.durability.barriers.at(-1)!;
  expect(report.durability.restoredGeneration).toBe(last.checkpoint.generation);
  expect(report.durability.restoredCompletedStep).toBe(last.checkpoint.completedStep);
  expect(summarizeSensorWall(report.reads, 60, {runId: last.checkpoint.runId, barriers: report.durability.barriers})).toEqual(report.summary);
  expect(report.summary.progress.deltaSteps).toBe('242');
  expect(report.summary.stepTiming.p95UpperMs).toBe(128);
  expect(report.summary.stepTiming.p99UpperMs).toBeNull(); expect(report.summary.stepTiming.overflowSamples).toBe('3');
  expect(BigInt('0x' + report.summary.stepTiming.bucketUpperMicros.at(-2)!)).toBe(256000n);
  expect(report.summary.droppedWallMicros).toBe('15222667'); expect(report.summary.meetsMeasuredGates).toBe(false);
  expect(report.capture.noDrops).toBe(true); expect(report.http.forbiddenRequests).toBe(0);
  expect(report.shadows.sampleProgress).toHaveLength(4);
  for (const value of report.shadows.sampleProgress) expect(BigInt(value)).toBeGreaterThan(0n);
  expect(report.hostPeakRssBytes).toBeGreaterThan(0);
  const observerSha = sha256(files.get('host/system-resource-observer.py')!);
  const resources = JSON.parse(files.get('aggregate-resources.json')!.toString('utf8')) as Array<{
    platform: string; processDetailsIncluded: boolean; observerSourceSha256: string; host: Record<string, unknown>}>;
  expect(resources.length).toBeGreaterThanOrEqual(3);
  for (const observation of resources) {
    expect(observation).toMatchObject({platform: 'Linux', processDetailsIncluded: false, observerSourceSha256: observerSha});
    expect(Object.keys(observation).every(key => ['schemaVersion', 'utc', 'platform', 'gpu', 'processDetailsIncluded',
      'attribution', 'host', 'collectionSeconds', 'sample', 'observerSourceSha256'].includes(key))).toBe(true);
    expect(Object.keys(observation.host).every(key => ['available', 'cpuTicks', 'loadAverage', 'memoryKiB'].includes(key))).toBe(true);
  }
});


/** Loaded cases preserve their distinct population, byte occupancy and cold recovery evidence. */
type LiveLoadedEvidence = LiveLargeCatalogEvidence & {
  source: {spoolHelperSha256: string; spoolFixtureSha256: string};
  replayMinMiB?: number; replayFillSeconds?: number; spoolPrefill?: LiveSpoolPrefillReceipt;
  capture: {before: DistillationDataset};
  train: {replayFill?: {minimumOwnedBytes: number; ownedBytes: number; dataBytes: number;
    encodedBytes: number; retainedChunks: number; cursor: number; elapsedSeconds: number;
    setupDeadlineSeconds: number; captureEpoch: string}};
  resume: {candidateSha256: string; candidateWeightsSha256: string};
  sustain: {minimumReplayBytes?: number; sequence: {learning_rate: number; gradient_clip: number}};
};

it.each(['spool', 'replay'] as const)('preserves the actual loaded %s gate and its failed shared-host performance', mode => {
  const root = resolve(import.meta.dirname, '../../docs/performance/sensor-v4/live-loaded-' + mode + '-debian-v1');
  const manifest = JSON.parse(readFileSync(resolve(root, 'evidence.json'), 'utf8')) as {
    kind: string; largeCatalogOverlapQualified: boolean; loadedSpoolQualified: boolean;
    loadedReplayQualified: boolean; performanceQualified: boolean; fullCapacitySoakQualified: boolean;
    maturityQualified: boolean; slitherCommit: string; pyrlCommit: string; nativeSourceSha256: string;
    trainerSourceSha256: string; trainerSourceFiles: string[]; resourceSamples: number;
    captureLifetimeDropFree: boolean; captureWindowDropFree: boolean;
    files: Record<string, FrozenFileReceipt>};
  expect(manifest).toMatchObject({kind: 'live-loaded-' + mode + '-evidence-v1',
    largeCatalogOverlapQualified: true, loadedSpoolQualified: mode === 'spool', loadedReplayQualified: mode === 'replay',
    performanceQualified: false, fullCapacitySoakQualified: false, maturityQualified: false,
    captureLifetimeDropFree: mode === 'spool', captureWindowDropFree: true});
  const files = readFrozenFiles(root, manifest.files);
  const report = JSON.parse(files.get('report.json')!.toString('utf8')) as LiveLoadedEvidence;
  expect(report.source).toMatchObject({slitherRevision: manifest.slitherCommit, pyrlRevision: manifest.pyrlCommit,
    nativeSourceSha256: manifest.nativeSourceSha256, target: 'x86_64-unknown-linux-gnu',
    hostSha256: sha256(files.get('host/live-distillation.ts')!),
    libraryHelperSha256: sha256(files.get('host/liveCapacityLibrary.ts')!),
    spoolHelperSha256: sha256(files.get('host/liveSpoolPrefill.ts')!),
    spoolFixtureSha256: sha256(files.get('host/spoolFixture.ts')!),
    trainerGateSha256: sha256(files.get('pyrl/live_distillation_gate.py')!),
    studentArtifactSha256: sha256(files.get('large-motion32-student.slither-brain')!)});
  const trainer = frozenTrainerSource(files, 'pyrl/');
  expect(trainer.names).toEqual(manifest.trainerSourceFiles);
  expect(trainer.sha256).toBe(manifest.trainerSourceSha256);
  expect(report).toMatchObject({device: 'cpu', workers: 4, seconds: 60,
    workload: {scenario: mode === 'spool' ? 'P1' : 'P0', capacityGraph: 'large',
      checkpointEnvelopeMiB: mode === 'spool' ? 768 : 512,
      request: {profile: 'motion', bins: 32}, metadata: {parameterCount: 494306, sensorVersion: 4}}});
  expect(report.workload.capacityAudit).toMatchObject({population: mode === 'spool' ? 300 : 55,
    parametersPerGenome: 494306, decodedEnvelopeFits: true, twoStateBudgetFits: true,
    workerScratchBytes: 512 * 1024 ** 2, checkpointScratchBytes: 512 * 1024 ** 2});
  const fixture = JSON.parse(files.get('fixture-index.json')!.toString('utf8')) as {
    records: {brainId: string; artifactByteCount: string}[]};
  expect(fixture.records).toHaveLength(101);
  expect(report.catalogOverlap.fixtureIndexSha256).toBe(sha256(files.get('fixture-index.json')!));
  expect(sha256(files.get('native-manual-source.slither-brain')!)).toBe(fixture.records[0]!.brainId);
  expect(report.catalogOverlap.before).toMatchObject({pages: 4, recordCount: 100});
  expect(report.catalogOverlap.before.brainIds).toEqual(fixture.records.slice(0, 100).map(row => row.brainId).sort());
  expect(report.catalogOverlap.before.originalBytes).toBe(fixture.records.slice(0, 100)
    .reduce((sum, row) => sum + BigInt('0x' + row.artifactByteCount), 0n).toString());
  expect(report.catalogOverlap.afterColdRestart).toEqual(report.catalogOverlap.before);
  expect(report.catalogOverlap.overflowRejected).toBe(true);
  expect(report.catalogOverlap.overflowPublishedFiles).toBe(false);
  expect(report.inspectorOverlap.pureProbe).toBe(false); expect(report.inspectorOverlap.inputSize).toBe(504);
  expect(report.inspectorOverlap.after.deliveredSamples).toBeGreaterThan(report.inspectorOverlap.before.deliveredSamples);
  expect(report.inspectorOverlap.after.maximumWireBytes).toBeLessThan(1024 ** 2);
  const candidate = files.get('trained-candidate.slither-brain')!;
  expect(candidate.subarray(0, 8).toString('ascii')).toBe('SLTHBRN1');
  expect(sha256(candidate)).toBe(report.resume.candidateSha256);
  expect(candidate.subarray(24, 56).toString('hex')).toBe(report.resume.candidateWeightsSha256);
  expect(report.train.updates).toBe(126);
  expect(report.resume).toMatchObject({restartExact: true, updates: 250});
  expect(report.sustain.initialUpdates).toBe(250);
  expect(report.sustain.updates - 250).toBe(report.sustain.optimizerUpdates);
  expect(report.sustain.optimizerUpdates).toBeGreaterThan(0);
  expect(report.sustain.finalCursor).toBeGreaterThan(report.sustain.initialCursor);
  expect(report.sustain.sequence).toEqual({batch_size: 32, burn_in_steps: 32, bptt_steps: 64,
    learning_rate: 0.0003, gradient_clip: 1});
  for (const phase of [report.train, report.resume, report.sustain]) {
    expect(phase.replayCapacityBytes).toBe(2048 * 1024 ** 2);
    expect(phase.replayBytes).toBeLessThanOrEqual(phase.replayCapacityBytes);
    expect(phase.trainerSourceSha256).toBe(manifest.trainerSourceSha256);
    expect(phase.processPriority).toEqual({platform: 'posix', nice: 5});
  }
  expect(report.durability).toMatchObject({requestedBoundaries: 3, persistenceRestartExact: true,
    restoredSourceSha256: manifest.nativeSourceSha256, invalidRestoreAdmissionsRejected: 7});
  expect(report.durability.barriers).toHaveLength(3);
  const last = report.durability.barriers.at(-1)!;
  expect(report.durability.restoredGeneration).toBe(last.checkpoint.generation);
  expect(report.durability.restoredCompletedStep).toBe(last.checkpoint.completedStep);
  expect(summarizeSensorWall(report.reads, 60, {runId: last.checkpoint.runId,
    barriers: report.durability.barriers})).toEqual(report.summary);
  expect(report.summary.meetsMeasuredGates).toBe(false);
  expect(report.capture.noDrops).toBe(mode === 'spool'); expect(report.http.forbiddenRequests).toBe(0);
  for (const counter of ['nativeDroppedChunks', 'nativeDroppedSamples', 'nativeDroppedBytes', 'spoolDroppedChunks'] as const) {
    expect(report.capture.after[counter]).toBe(report.capture.before[counter]);
  }
  expect(report.capture.after.spoolDroppedChunks).toBe('0000000000000000');
  expect(report.shadows.sampleProgress).toHaveLength(4);
  for (const [index, value] of report.shadows.sampleProgress.entries()) {
    expect(BigInt(value)).toBeGreaterThan(0n);
    expect(report.shadows.after[index]!.artifactSha256).toBe(report.resume.candidateSha256);
  }
  const resources = JSON.parse(files.get('aggregate-resources.json')!.toString('utf8')) as Array<{
    platform: string; processDetailsIncluded: boolean; observerSourceSha256: string;
    host: Record<string, unknown>; gpu: {rows?: Array<Record<string, unknown>>}}>;
  expect(resources).toHaveLength(manifest.resourceSamples);
  expect(resources.length).toBeGreaterThan(50);
  const observer = sha256(files.get('host/system-resource-observer.py')!);
  for (const row of resources) {
    expect(row).toMatchObject({platform: 'Linux', processDetailsIncluded: false, observerSourceSha256: observer});
    expect(Object.keys(row).every(key => ['schemaVersion', 'utc', 'platform', 'gpu', 'host',
      'processDetailsIncluded', 'attribution', 'collectionSeconds', 'sample', 'observerSourceSha256'].includes(key))).toBe(true);
    expect(Object.keys(row.host).every(key => ['available', 'cpuTicks', 'loadAverage', 'memoryKiB',
      'error', 'exitCode', 'oversized'].includes(key))).toBe(true);
    expect(Object.keys(row.gpu).every(key => ['available', 'rows', 'error', 'exitCode', 'oversized'].includes(key))).toBe(true);
    for (const gpu of row.gpu.rows ?? []) expect(Object.keys(gpu).every(key => ['name',
      'utilization.gpu [%]', 'utilization.memory [%]', 'memory.used [MiB]', 'memory.total [MiB]',
      'power.draw [W]', 'temperature.gpu'].includes(key))).toBe(true);
  }
  if (mode === 'spool') {
    expect(report.spoolGiB).toBe(64);
    const prefill = report.spoolPrefill!;
    expect(prefill).toMatchObject({kind: 'inactive-synthetic-opaque-spool-v1', targetGiB: 64,
      objectBytes: 1024 ** 2, objects: 65536, selectedMaxBytes: 64 * 1024 ** 3, selectedMaxChunks: 65536,
      deadlineMs: 10_800_000, recoveryBudgetMs: 660_000, finalRecoveryBudgetMs: 660_000});
    expect(BigInt(prefill.initialFreeBytes)).toBeGreaterThanOrEqual(BigInt(prefill.requiredFreeBytes));
    expect(prefill.elapsedSeconds * 1000).toBeLessThan(prefill.deadlineMs);
    expect(prefill.ingestion.samples).toBe(65536);
    expect(prefill.ingestion.counts.reduce((sum, count) => sum + count, 0)).toBe(65536);
    expect(prefill.beforeRecovery).toEqual({retainedChunks: 65536,
      retainedBytes: '0000001000000000', resolvedThrough: '0000000000010000'});
    expect(prefill.afterRecovery).toEqual(prefill.beforeRecovery);
    expect(prefill.recoveryElapsedMs).toBeLessThan(prefill.recoveryBudgetMs!);
    expect(prefill.finalRecoveryElapsedMs).toBeLessThan(prefill.finalRecoveryBudgetMs!);
    expect(prefill.afterFinalRecovery).toEqual(prefill.afterCapture);
    expect(prefill.afterCapture!.retainedChunks).toBeLessThan(65536);
    expect(BigInt('0x' + prefill.afterCapture!.retainedBytes))
      .toBe(BigInt(prefill.afterCapture!.retainedChunks) * 1024n ** 2n);
    const chunks = prefill.afterCapture!.retainedChunks + report.capture.after.retainedChunks;
    const bytes = Number(BigInt('0x' + prefill.afterCapture!.retainedBytes) + BigInt('0x' + report.capture.after.retainedBytes));
    expect(chunks).toBeLessThanOrEqual(65536); expect(bytes).toBeLessThanOrEqual(64 * 1024 ** 3);
    expect(distillationRecoveryTimeoutMs({chunks, bytes})).toBe(prefill.finalRecoveryBudgetMs);
    expect(prefill.captureEpoch).not.toBe(report.capture.after.contract.captureEpoch);
    expect(report.sustain.peakReplayBytes).toBeLessThan(128 * 1024 ** 2);
    expect(report.summary.stepTiming.p99UpperMs).toBeNull();
    expect(report.summary.stepTiming.overflowSamples).toBe('3');
  } else {
    expect(report.spoolGiB).toBe(4); expect(report).not.toHaveProperty('spoolPrefill');
    expect(report.replayMinMiB).toBe(2040); expect(report.replayFillSeconds).toBe(7200);
    expect(BigInt('0x' + report.capture.after.nativeDroppedChunks)).toBe(216n);
    expect(BigInt('0x' + report.capture.after.nativeDroppedSamples)).toBe(13115n);
    const fill = report.train.replayFill!;
    expect(fill).toMatchObject({minimumOwnedBytes: 2040 * 1024 ** 2, setupDeadlineSeconds: 7200,
      captureEpoch: report.capture.after.contract.captureEpoch});
    expect(fill.ownedBytes).toBeGreaterThanOrEqual(fill.minimumOwnedBytes);
    expect(fill.elapsedSeconds).toBeLessThan(fill.setupDeadlineSeconds);
    expect(fill.ownedBytes).toBe(fill.dataBytes + 256 * 1024 + fill.retainedChunks * 4096);
    expect(fill.dataBytes).toBeGreaterThan(fill.encodedBytes);
    expect(fill.ownedBytes).toBe(report.train.replayBytes);
    expect(report.resume.replayBytes).toBe(fill.ownedBytes);
    expect(report.sustain.minimumReplayBytes).toBeGreaterThanOrEqual(fill.minimumOwnedBytes);
    expect(report.sustain.peakReplayBytes).toBeGreaterThanOrEqual(2047 * 1024 ** 2);
    expect(report.summary.stepTiming.p99UpperMs).toBe(48);
  }
});


/** Existing native costs are independent diagnostic snapshots beside the ordinary live histogram. */
type NativeCostInvestigation = LiveLargeCatalogEvidence & {
  source: {costHelperSha256: string; spoolHelperSha256: string; spoolFixtureSha256: string};
  phaseCosts: {scope: string; before: NativeStepCostRead; after: NativeStepCostRead};
};

it('binds native large-graph cost attribution and preserves a rejected cold-streaming affine candidate', () => {
  const root = resolve(import.meta.dirname, '../../docs/performance/sensor-v4/native-large-costs-debian-v1');
  const manifest = JSON.parse(readFileSync(resolve(root, 'evidence.json'), 'utf8')) as {
    kind: string; performanceQualified: boolean; fullCapacitySoakQualified: boolean;
    runtimeOptimized: boolean; denseRowBatchingSelected: boolean; slitherCommit: string; pyrlCommit: string;
    nativeSourceSha256: string; trainerSourceSha256: string; trainerSourceFiles: string[];
    files: Record<string, FrozenFileReceipt>};
  expect(manifest).toMatchObject({kind: 'native-large-cost-investigation-v1', performanceQualified: false,
    fullCapacitySoakQualified: false, runtimeOptimized: false, denseRowBatchingSelected: false});
  const files = readFrozenFiles(root, manifest.files);
  const report = JSON.parse(files.get('report.json')!.toString('utf8')) as NativeCostInvestigation;
  expect(report.source).toMatchObject({slitherRevision: manifest.slitherCommit, pyrlRevision: manifest.pyrlCommit,
    nativeSourceSha256: manifest.nativeSourceSha256, target: 'x86_64-unknown-linux-gnu',
    hostSha256: sha256(files.get('host/live-distillation.ts')!),
    costHelperSha256: sha256(files.get('host/nativeStepCosts.ts')!),
    libraryHelperSha256: sha256(files.get('host/liveCapacityLibrary.ts')!),
    spoolHelperSha256: sha256(files.get('host/liveSpoolPrefill.ts')!),
    spoolFixtureSha256: sha256(files.get('host/spoolFixture.ts')!),
    trainerGateSha256: sha256(files.get('pyrl/live_distillation_gate.py')!),
    studentArtifactSha256: sha256(files.get('large-motion32-student.slither-brain')!)});
  const trainer = frozenTrainerSource(files, 'pyrl/');
  expect(trainer.names).toEqual(manifest.trainerSourceFiles);
  expect(trainer.sha256).toBe(manifest.trainerSourceSha256);
  expect(report).toMatchObject({device: 'cpu', workers: 4, seconds: 60,
    workload: {scenario: 'P1', capacityGraph: 'large', checkpointEnvelopeMiB: 768,
      metadata: {parameterCount: 494306}}, train: {updates: 126}, resume: {updates: 250, restartExact: true}});
  expect(report.catalogOverlap.before).toMatchObject({recordCount: 100, pages: 4});
  expect(report.catalogOverlap.afterColdRestart).toEqual(report.catalogOverlap.before);
  expect(report.inspectorOverlap.after.deliveredSamples).toBeGreaterThan(report.inspectorOverlap.before.deliveredSamples);
  expect(report.durability.barriers).toHaveLength(3);
  const last = report.durability.barriers.at(-1)!;
  expect(summarizeSensorWall(report.reads, 60, {runId: last.checkpoint.runId,
    barriers: report.durability.barriers})).toEqual(report.summary);
  expect(report.summary.meetsMeasuredGates).toBe(false);
  const phases = report.phaseCosts;
  expect(phases.scope).toContain('not simultaneous step snapshots');
  expect(phases.scope).toContain('Lifetime maximum is not a window maximum');
  expect(phases.before.beforeMs).toBeGreaterThanOrEqual(report.reads[0]!.beforeMs);
  expect(phases.after.afterMs).toBeGreaterThanOrEqual(report.reads.at(-1)!.beforeMs);
  const declaration = files.get('host/nativeStepCosts.ts')!.toString('utf8')
    .match(/NATIVE_STEP_COST_KEYS = \[([\s\S]*?)\] as const/u);
  expect(declaration).not.toBeNull();
  const keys = [...declaration![1]!.matchAll(/'([a-zA-Z]+)'/gu)].map(match => match[1]!);
  expect(keys).toHaveLength(21);
  /** Use a string-keyed view only after matching the frozen literal inventory. */
  const before = phases.before.counters as Record<string, string>;
  const after = phases.after.counters as Record<string, string>;
  expect(Object.keys(before)).toEqual(keys); expect(Object.keys(after)).toEqual(keys);
  const deltas: Record<string, bigint> = {};
  for (const key of keys) {
    expect(before[key]).toMatch(/^[0-9a-f]{16}$/u); expect(after[key]).toMatch(/^[0-9a-f]{16}$/u);
    expect(before[key]).not.toBe('f'.repeat(16)); expect(after[key]).not.toBe('f'.repeat(16));
    expect(BigInt('0x' + after[key]!)).toBeGreaterThanOrEqual(BigInt('0x' + before[key]!));
    if (key !== 'terminalStepMaxMicros') deltas[key] = BigInt('0x' + after[key]!) - BigInt('0x' + before[key]!);
  }
  expect(deltas['terminalStepSamples']).toBe(3n);
  expect(deltas['slowStepSamples']).toBe(173n);
  expect(deltas['terminalStepPreparationMicros']).toBe(34701064n);
  expect(deltas['terminalStepEvolutionSummaryMicros']).toBe(28201694n);
  expect(deltas['terminalStepEvolutionReproductionMicros']).toBe(5963211n);
  expect(deltas['slowStepControlNeuralMicros']).toBe(15589631n);
  expect(deltas['slowStepWorldMicros']).toBe(2261696n);
  const micro = JSON.parse(files.get('micro/result.json')!.toString('utf8')) as {
    exactOutputComparisons: number; exactDotComparisons: number;
    records: {width: number; rows: number; iterations: number; pass: number; streaming: boolean;
      oldFirst: boolean; originalSeconds: number; candidateSeconds: number}[]};
  const metadata = JSON.parse(files.get('micro/metadata.json')!.toString('utf8')) as {
    sourceSha256: string; referenceSha256: string; scope: string;
    summary: {width: number; rows: number; streaming: boolean; medianCandidateRatio: number}[]};
  expect(metadata.sourceSha256).toBe(sha256(files.get('micro/benchmark.rs')!));
  expect(metadata.referenceSha256).toBe(sha256(files.get('micro/simd_kernels.rs')!));
  expect(metadata.scope).toContain('Endpoint aggregate resource context only');
  expect(micro.exactOutputComparisons).toBe(7707); expect(micro.exactDotComparisons).toBe(7476);
  expect(micro.records).toHaveLength(56); expect(metadata.summary).toHaveLength(7);
  for (const record of micro.records) {
    expect(record.originalSeconds).toBeGreaterThan(0); expect(record.candidateSeconds).toBeGreaterThan(0);
    expect(record.oldFirst).toBe([0, 3].includes(record.pass % 4));
  }
  for (const summary of metadata.summary) {
    const cells = micro.records.filter(row => row.width === summary.width &&
      row.rows === summary.rows && row.streaming === summary.streaming);
    expect(cells.map(row => row.pass)).toEqual([0, 1, 2, 3, 4, 5, 6, 7]);
    const ratios = cells.map(row => row.candidateSeconds / row.originalSeconds).sort((a, b) => a - b);
    expect(summary.medianCandidateRatio).toBeCloseTo((ratios[3]! + ratios[4]!) / 2, 14);
  }
  expect(metadata.summary.find(row => row.width === 256 && row.streaming)!.medianCandidateRatio).toBeGreaterThan(1.1);
  expect(metadata.summary.find(row => row.width === 256 && !row.streaming)!.medianCandidateRatio).toBeLessThan(0.5);
  const observer = sha256(files.get('host/system-resource-observer.py')!);
  for (const name of ['aggregate-resources.json', 'micro/before-resources.json', 'micro/after-resources.json']) {
    const rows = JSON.parse(files.get(name)!.toString('utf8')) as Array<{
      processDetailsIncluded: boolean; observerSourceSha256: string; host: Record<string, unknown>;
      gpu: {rows?: Array<Record<string, unknown>>}}>;
    expect(rows.length).toBeGreaterThan(0);
    for (const row of rows) {
      expect(row.processDetailsIncluded).toBe(false); expect(row.observerSourceSha256).toBe(observer);
      expect(Object.keys(row).every(key => ['schemaVersion', 'utc', 'platform', 'gpu', 'host',
        'processDetailsIncluded', 'attribution', 'collectionSeconds', 'sample', 'observerSourceSha256'].includes(key))).toBe(true);
      expect(Object.keys(row.host).every(key => ['available', 'cpuTicks', 'loadAverage', 'memoryKiB',
        'error', 'exitCode', 'oversized'].includes(key))).toBe(true);
      expect(Object.keys(row.gpu).every(key => ['available', 'rows', 'error', 'exitCode', 'oversized'].includes(key))).toBe(true);
      for (const gpu of row.gpu.rows ?? []) expect(Object.keys(gpu).every(key => ['name',
        'utilization.gpu [%]', 'utilization.memory [%]', 'memory.used [MiB]', 'memory.total [MiB]',
        'power.draw [W]', 'temperature.gpu'].includes(key))).toBe(true);
    }
  }
});

/** Paired scalar/SIMD timings share exact inputs and order-balanced passes. */
interface SpeciesMicroRecord {
  case?: string; population?: number; clustered?: boolean; pass: number; oldFirst: boolean;
  originalSeconds: number; candidateSeconds: number;
  speciesCount?: number; topSpeciesSize?: number; averageWeightBits?: string; weightVarianceBits?: string;
}

/** Recompute retained paired medians without executing archived benchmark code. */
function speciesMedian(records: SpeciesMicroRecord[]): number {
  const ratios = records.map(row => row.candidateSeconds / row.originalSeconds).sort((a, b) => a - b);
  expect(ratios.length % 2).toBe(0);
  for (const row of records) {
    expect(row.originalSeconds).toBeGreaterThan(0); expect(row.candidateSeconds).toBeGreaterThan(0);
    expect(row.oldFirst).toBe([0, 3].includes(row.pass % 4));
  }
  return (ratios[ratios.length / 2 - 1]! + ratios[ratios.length / 2]!) / 2;
}

it('binds guarded species rejection to paired summaries, native generations and aggregate host context', () => {
  const root = resolve(import.meta.dirname, '../../docs/performance/sensor-v4/species-distance-native-v1');
  const manifest = JSON.parse(readFileSync(resolve(root, 'evidence.json'), 'utf8')) as {
    kind: string; slitherCommit: string; pyrlCommit: string; nativeSourceSha256: string;
    trainerSourceSha256: string; sourcesFrom: string; performanceQualified: boolean;
    fullCapacitySoakQualified: boolean; captureDropFree: boolean; resourceSamples: number;
    phaseDeltas: Record<string, string>; files: Record<string, FrozenFileReceipt>;
  };
  expect(manifest).toMatchObject({kind: 'guarded-species-native-window-v1', performanceQualified: false,
    fullCapacitySoakQualified: false, sourcesFrom: '../native-large-costs-debian-v1'});
  const files = readFrozenFiles(root, manifest.files);
  const priorRoot = resolve(root, manifest.sourcesFrom);
  const priorManifest = JSON.parse(readFileSync(resolve(priorRoot, 'evidence.json'), 'utf8')) as {
    files: Record<string, FrozenFileReceipt>};
  const prior = readFrozenFiles(priorRoot, priorManifest.files);
  const report = JSON.parse(files.get('report.json')!.toString('utf8')) as NativeCostInvestigation;
  expect(report.source).toMatchObject({slitherRevision: manifest.slitherCommit, pyrlRevision: manifest.pyrlCommit,
    nativeSourceSha256: manifest.nativeSourceSha256, target: 'x86_64-unknown-linux-gnu'});
  for (const [key, name] of Object.entries({hostSha256: 'host/live-distillation.ts',
    costHelperSha256: 'host/nativeStepCosts.ts', libraryHelperSha256: 'host/liveCapacityLibrary.ts',
    spoolHelperSha256: 'host/liveSpoolPrefill.ts', spoolFixtureSha256: 'host/spoolFixture.ts',
    trainerGateSha256: 'pyrl/live_distillation_gate.py', studentArtifactSha256: 'large-motion32-student.slither-brain'})) {
    expect((report.source as Record<string, unknown>)[key]).toBe(sha256(prior.get(name)!));
  }
  expect(frozenTrainerSource(prior, 'pyrl/').sha256).toBe(manifest.trainerSourceSha256);
  expect(report).toMatchObject({device: 'cpu', workers: 4, seconds: 60,
    workload: {scenario: 'P1', capacityGraph: 'large', checkpointEnvelopeMiB: 768,
      metadata: {parameterCount: 494306}}, train: {updates: 126}, resume: {updates: 250, restartExact: true}});
  expect(report.sustain.optimizerUpdates).toBeGreaterThan(0);
  expect(report.capture.noDrops).toBe(manifest.captureDropFree);
  expect(report.catalogOverlap.before).toMatchObject({recordCount: 100, pages: 4});
  expect(report.catalogOverlap.afterColdRestart).toEqual(report.catalogOverlap.before);
  expect(report.catalogOverlap.overflowRejected).toBe(true); expect(report.catalogOverlap.overflowPublishedFiles).toBe(false);
  expect(report.inspectorOverlap.after.deliveredSamples).toBeGreaterThan(report.inspectorOverlap.before.deliveredSamples);
  expect(report.shadows.sampleProgress).toHaveLength(4);
  for (const progress of report.shadows.sampleProgress) expect(BigInt(progress)).toBeGreaterThan(0n);
  expect(report.durability.barriers).toHaveLength(3);
  expect(report.durability).toMatchObject({persistenceRestartExact: true, restoredSourceSha256: manifest.nativeSourceSha256});
  const last = report.durability.barriers.at(-1)!;
  expect(summarizeSensorWall(report.reads, 60, {runId: last.checkpoint.runId,
    barriers: report.durability.barriers})).toEqual(report.summary);
  expect(report.summary.meetsMeasuredGates).toBe(false);
  const before = report.phaseCosts.before.counters as Record<string, string>;
  const after = report.phaseCosts.after.counters as Record<string, string>;
  expect(Object.keys(before)).toHaveLength(21); expect(Object.keys(after)).toEqual(Object.keys(before));
  for (const key of Object.keys(before).filter(key => key !== 'terminalStepMaxMicros')) {
    expect((BigInt('0x' + after[key]!) - BigInt('0x' + before[key]!)).toString()).toBe(manifest.phaseDeltas[key]);
  }
  expect(manifest.phaseDeltas['terminalStepSamples']).toBe('3');

  const evolution = files.get('native/evolution.rs')!.toString('utf8').replaceAll('\r\n', '\n');
  const nativeSpecies = evolution.slice(evolution.indexOf('fn species_probe_proves_far('), evolution.indexOf('fn tournament_pick('));
  for (const platform of ['debian', 'windows']) {
    const micro = JSON.parse(files.get(platform + '/result.json')!.toString('utf8')) as {
      exactDecisions: number; records: SpeciesMicroRecord[]};
    const metadata = JSON.parse(files.get(platform + '/metadata.json')!.toString('utf8')) as {
      sourceSha256: string; kernelSha256: string; medianCandidateRatios: Record<string, number>};
    expect(metadata.sourceSha256).toBe(sha256(files.get('micro/benchmark-production.rs')!));
    expect(metadata.kernelSha256).toBe(sha256(files.get('micro/simd_kernels.rs')!));
    expect(micro.exactDecisions).toBe(500); expect(micro.records).toHaveLength(40);
    for (const [name, median] of Object.entries(metadata.medianCandidateRatios)) {
      const records = micro.records.filter(row => row.case === name);
      expect(records.map(row => row.pass)).toEqual([0, 1, 2, 3, 4, 5, 6, 7]);
      expect(speciesMedian(records)).toBeCloseTo(median, 14);
    }
    expect(metadata.medianCandidateRatios['distant-stream']).toBeLessThan(1);
  }
  const benchmark = files.get('micro/benchmark-production.rs')!.toString('utf8').replaceAll('\r\n', '\n');
  expect(benchmark.slice(benchmark.indexOf('fn species_probe_proves_far('), benchmark.indexOf('fn candidate(')).trim())
    .toBe(nativeSpecies.trim());
  const whole = JSON.parse(files.get('debian/summary-result.json')!.toString('utf8')) as {
    parametersPerGenome: number; records: SpeciesMicroRecord[]};
  const wholeMetadata = JSON.parse(files.get('debian/summary-metadata.json')!.toString('utf8')) as {
    sourceSha256: string; summary: Array<{population: number; clustered: boolean; medianCandidateRatio: number}>};
  expect(whole.parametersPerGenome).toBe(494306); expect(whole.records).toHaveLength(16);
  expect(wholeMetadata.sourceSha256).toBe(sha256(files.get('micro/benchmark-summary.rs')!));
  const summarySource = files.get('micro/benchmark-summary.rs')!.toString('utf8').replaceAll('\r\n', '\n');
  const originalEvolution = files.get('native/evolution-reference.rs')!.toString('utf8').replaceAll('\r\n', '\n');
  const originalDistance = originalEvolution.slice(originalEvolution.indexOf('fn genomes_share_species('),
    originalEvolution.indexOf('fn tournament_pick(')).replace('fn genomes_share_species(', 'fn genomes_share_species_reference(');
  expect(summarySource).toContain(originalDistance);
  const originalSummary = originalEvolution.slice(originalEvolution.indexOf('fn calculate_summary('),
    originalEvolution.indexOf('/// Prove that the original scalar RMS comparison'))
    .replace('fn calculate_summary(', 'fn calculate_summary_reference(')
    .replaceAll('genomes_share_species(', 'genomes_share_species_reference(');
  expect(summarySource).toContain(originalSummary);
  const currentSummary = evolution.slice(evolution.indexOf('struct SpeciesRepresentative {'),
    evolution.indexOf('/// Prove that the original scalar RMS comparison'));
  expect(summarySource).toContain(currentSummary);
  expect(summarySource).toContain(nativeSpecies);
  for (const cell of wholeMetadata.summary) {
    const records = whole.records.filter(row => row.population === cell.population && row.clustered === cell.clustered);
    expect(records.map(row => row.pass)).toEqual([0, 1, 2, 3]);
    expect(speciesMedian(records)).toBeCloseTo(cell.medianCandidateRatio, 14);
    expect(new Set(records.map(row => JSON.stringify([row.speciesCount, row.topSpeciesSize,
      row.averageWeightBits, row.weightVarianceBits]))).size).toBe(1);
  }

  const observerNames = {Linux: prior.get('host/system-resource-observer.py')!, Windows: files.get('windows/system-resource-observer.py')!};
  for (const name of ['aggregate-resources.json', 'debian/summary-before-resources.json',
    'debian/summary-after-resources.json', 'windows/before-resources.json', 'windows/after-resources.json']) {
    const rows = JSON.parse(files.get(name)!.toString('utf8')) as Array<{
      platform: 'Linux' | 'Windows'; processDetailsIncluded: boolean; observerSourceSha256: string;
      host: Record<string, unknown>; gpu: {rows?: Array<Record<string, unknown>>};
      wsl?: {available: boolean; distributions?: Array<Record<string, unknown>>}}>;
    expect(rows.length).toBeGreaterThan(0);
    if (name === 'aggregate-resources.json') expect(rows).toHaveLength(manifest.resourceSamples);
    for (const row of rows) {
      expect(row.processDetailsIncluded).toBe(false);
      expect(row.observerSourceSha256).toBe(sha256(observerNames[row.platform]));
      expect(Object.keys(row).every(key => ['schemaVersion', 'utc', 'platform', 'gpu', 'host', 'wsl',
        'processDetailsIncluded', 'attribution', 'collectionSeconds', 'sample', 'observerSourceSha256'].includes(key))).toBe(true);
      expect(Object.keys(row.host).every(key => ['available', 'cpuTicks', 'loadAverage', 'memoryKiB',
        'cpuPercent', 'totalMemoryKiB', 'freeMemoryKiB', 'error', 'exitCode', 'oversized'].includes(key))).toBe(true);
      if (row.platform === 'Linux') expect(row.wsl).toBeUndefined();
      if (row.wsl) {
        expect(Object.keys(row.wsl).every(key => ['available', 'distributions', 'error', 'exitCode', 'oversized'].includes(key))).toBe(true);
        for (const distribution of row.wsl.distributions ?? []) {
          expect(Object.keys(distribution).every(key => ['available', 'cpuTicks', 'loadAverage', 'memoryKiB',
            'error', 'exitCode', 'oversized'].includes(key))).toBe(true);
        }
      }
      for (const host of [row.host, ...row.wsl?.distributions ?? []]) {
        const memory = host['memoryKiB'] as Record<string, unknown> | undefined;
        if (memory) {
          expect(Object.keys(memory).every(key => ['MemTotal', 'MemFree', 'MemAvailable', 'Buffers', 'Cached'].includes(key))).toBe(true);
          for (const value of Object.values(memory)) {
            expect(typeof value).toBe('number'); expect(Number.isFinite(value)).toBe(true);
          }
        }
      }
      expect(Object.keys(row.gpu).every(key => ['available', 'rows', 'error', 'exitCode', 'oversized'].includes(key))).toBe(true);
      for (const gpu of row.gpu.rows ?? []) expect(Object.keys(gpu).every(key => ['name',
        'utilization.gpu [%]', 'utilization.memory [%]', 'memory.used [MiB]', 'memory.total [MiB]',
        'power.draw [W]', 'temperature.gpu'].includes(key))).toBe(true);
    }
  }
});


/** Current Windows CPU receipts keep source/durability and learned initial families together. */
type CurrentCpuCohortEvidence = TrainerContentionEvidence & {
  source: TrainerContentionEvidence['source'] & {slitherRevision: string; pyrlRevision: string};
  workload: TrainerContentionEvidence['workload'] & {trainedCohort: {
    indexSha256: string; families: {artifact: BrainArtifactDescriptor; populationSlots: number}[]}};
  train: TrainerContentionEvidence['train'] & {trainerSourceSha256: string};
  resume: TrainerContentionEvidence['resume'] & {trainerSourceSha256: string};
  sustain: TrainerContentionEvidence['sustain'] & {trainerSourceSha256: string};
  durability: LiveSixHourEvidenceReport['durability'];
};

it('retains current CPU P1 learned-cohort work without hiding the failed timing gate', () => {
  const root = resolve(import.meta.dirname, '../../docs/performance/sensor-v4/live-precise-cpu-windows-v1');
  const manifest = JSON.parse(readFileSync(resolve(root, 'evidence.json'), 'utf8')) as {
    files: Record<string, FrozenFileReceipt>; slitherBaseCommit: string; pyrlCommit: string;
    nativeSourceSha256: string; trainerSourceSha256: string; trainerSourceFiles: string[]; studentSha256: string;
  };
  const files = readFrozenFiles(root, manifest.files);
  const report = JSON.parse(files.get('report.json')!.toString('utf8')) as CurrentCpuCohortEvidence;
  expect(frozenTrainerSource(files, 'pyrl/')).toEqual({names: manifest.trainerSourceFiles, sha256: manifest.trainerSourceSha256});
  expect(report.source).toMatchObject({slitherRevision: manifest.slitherBaseCommit, pyrlRevision: manifest.pyrlCommit,
    nativeSourceSha256: manifest.nativeSourceSha256, target: 'x86_64-pc-windows-msvc'});
  expect(report.workload).toMatchObject({scenario: 'P1', request: {profile: 'motion', bins: 16, rayCap: 512, rayBatch: 131072},
    metadata: {parameterCount: 25042, sensorVersion: 4}});
  const cohortRoot = resolve(import.meta.dirname, '../../docs/performance/sensor-v4/live-cohort-debian-v1');
  const cohortManifest = JSON.parse(readFileSync(resolve(cohortRoot, 'evidence.json'), 'utf8')) as {files: Record<string, FrozenFileReceipt>};
  const original = readFrozenFiles(cohortRoot, cohortManifest.files);
  const index = JSON.parse(original.get('cohort.json')!.toString('utf8')) as {artifacts: {file: string; sha256: string}[]};
  expect(report.workload.trainedCohort.indexSha256).toBe(sha256(original.get('cohort.json')!));
  expect(report.workload.trainedCohort.families.map(family => family.artifact.artifactSha256)).toEqual(index.artifacts.map(entry => entry.sha256));
  expect(report.workload.trainedCohort.families.map(family => family.populationSlots)).toEqual([75, 75, 75, 75]);
  expect(report.resume).toMatchObject({updates: 250, restartExact: true, trainerSourceSha256: manifest.trainerSourceSha256});
  expect(report.sustain).toMatchObject({initialUpdates: 250, updates: 550, optimizerUpdates: 300,
    trainerSourceSha256: manifest.trainerSourceSha256});
  expect(report.sustain.finalCursor).toBeGreaterThan(report.sustain.initialCursor);
  expect(report.durability).toMatchObject({persistenceRestartExact: true, requestedBoundaries: 2,
    restoredSourceSha256: manifest.nativeSourceSha256});
  expect(report.durability.barriers).toHaveLength(2);
  const last = report.durability.barriers.at(-1)!;
  expect(report.summary).toEqual(summarizeSensorWall(report.reads, report.seconds,
    {runId: last.checkpoint.runId, barriers: report.durability.barriers}));
  expect(report.summary.progress.deltaSteps).toBe('3668');
  expect(report.summary.stepTiming.p99UpperMs).toBe(24);
  expect(report.summary.droppedWallMicros).toBe('2925000');
  expect(report.summary.meetsMeasuredGates).toBe(false);
  expect(report.capture.noDrops).toBe(true);
  expect(report.capture.after.resolvedThrough).toBe(report.capture.after.productionWatermark);
  expect(report.http.forbiddenRequests).toBe(0);
  expect(report.frames.copies).toBeGreaterThan(0);
  expect(sha256(files.get('student.slither-brain')!)).toBe(manifest.studentSha256);
  for (const [slot, after] of report.shadows.after.entries()) {
    const before = report.shadows.before[slot]!;
    expect(after.artifactSha256).toBe(manifest.studentSha256);
    expect(before.artifactSha256).toBe(manifest.studentSha256);
    expect(BigInt('0x' + before.totals.samples)).toBeGreaterThan(0n);
    const delta = BigInt('0x' + after.totals.samples) - BigInt('0x' + before.totals.samples);
    expect(delta).toBeGreaterThan(0n);
    expect(delta.toString()).toBe(report.shadows.sampleProgress[slot]);
  }
});
