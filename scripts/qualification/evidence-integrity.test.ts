import { summarizeSensorWall, type SensorWallRead } from './sensorWallSummary.ts';
import type { DistillationDataset } from '../../src/protocol/distillation.ts';
import { createHash } from 'node:crypto';
import { existsSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { gunzipSync } from 'node:zlib';
import { expect, it } from 'vitest';

/** One losslessly retained native report and its two exact byte receipts. */
interface EvidenceRecord {
  /** Relative compressed filename, confined to the selected evidence directory. */
  report: string;
  /** Hash of the retained gzip bytes. */
  reportSha256: string;
  /** Hash of the original native JSON bytes. */
  originalReportSha256: string;
}
/** Common integrity receipts from the retained comparison formats. */
interface EvidenceManifest {
  /** Raw script hash when the original recorder hashes filesystem bytes. */
  scriptSha256?: string;
  /** Explicit canonical-LF script receipt used by the split-bin experiment. */
  comparisonScriptCanonicalSha256?: string;
  /** Exact patch hash, when it is stored beside the comparison receipts. */
  candidatePatchSha256?: string;
  /** Standard comparison records. */
  records?: EvidenceRecord[];
  /** Split-bin comparison records. */
  rows?: EvidenceRecord[];
}
/** Checksummed comparisons must survive both Windows and Linux Git checkouts. */
const COMPARISONS = [
  ['ray-split-bins16-windows-v1', 'summary.json'],
  ['packet-bounds-windows-v1', 'comparison.json'],
  ['pellet-cell-rank-windows-v1', 'comparison.json'],
  ['pellet-counting-windows-v1', 'P0-w3-comparison.json'],
  ['pellet-counting-windows-v1', 'P1-w5-comparison.json'],
  ['head-pair-bound-windows-v1', 'comparison.json'],
  ['head-pair-bound-debian-v1', 'comparison.json']
] as const;

/** Hash exact retained bytes without interpreting samples, models or simulation state. */
function sha256(bytes: Uint8Array): string {
  return createHash('sha256').update(bytes).digest('hex');
}

it.each(COMPARISONS)('%s/%s preserves reports and their measured source bytes', (directory, filename) => {
  const root = resolve(import.meta.dirname, '../../docs/performance/sensor-v4', directory);
  const manifest = JSON.parse(readFileSync(resolve(root, filename), 'utf8')) as EvidenceManifest;
  const script = readFileSync(resolve(root, 'comparison.py'));
  if (manifest.comparisonScriptCanonicalSha256) {
    expect(sha256(Buffer.from(script.toString('utf8').replace(/\r\n/gu, '\n'))))
      .toBe(manifest.comparisonScriptCanonicalSha256);
  } else expect(sha256(script)).toBe(manifest.scriptSha256);
  const sourcePath = resolve(root, 'source.json');
  const patchSha256 = existsSync(sourcePath)
    ? (JSON.parse(readFileSync(sourcePath, 'utf8')) as { patchSha256: string }).patchSha256
    : manifest.candidatePatchSha256;
  expect(sha256(readFileSync(resolve(root, 'candidate.patch')))).toBe(patchSha256);
  const records = manifest.records ?? manifest.rows ?? [];
  expect(records).toHaveLength(4);
  for (const record of records) {
    expect(record.report).toMatch(/^[a-zA-Z0-9_.-]+\.json\.gz$/u);
    const compressed = readFileSync(resolve(root, record.report));
    expect(sha256(compressed)).toBe(record.reportSha256);
    const original = gunzipSync(compressed, { maxOutputLength: 4 * 1024 ** 2 });
    expect(sha256(original)).toBe(record.originalReportSha256);
  }
});

/** Receipts for independently measured historical opaque-spool hosts and reports. */
interface SpoolEvidence {
  /** Lossless host sources keyed by their safe compressed filename. */
  hosts: Record<string, {gzipSha256: string; canonicalLfSha256: string}>;
  /** Exact retained source patches. */
  candidatePatch: string;
  /** Byte receipt for the retention patch. */
  candidatePatchSha256: string;
  /** Exact publication patch. */
  publicationPatch: string;
  /** Byte receipt for the publication patch. */
  publicationPatchSha256: string;
  /** Read-only investigation receipts, never treated as qualification reports. */
  diagnostics?: Array<{reportPath: string; reportCanonicalLfSha256: string; sourceArchive: string;
    sourceArchiveSha256: string; sourceCanonicalLfSha256: string}>;
  /** Completed reports, including failed capacity probes. */
  reports: Array<{path: string; canonicalLfSha256: string; status: string;
    hostCanonicalLfSha256: string; storeCanonicalLfSha256: string; hostSourcePath?: string;
    auxiliaryCanonicalLfSha256?: Record<string, string>}>;
}

it('preserves opaque-spool report bytes, outcomes and exact historical host sources', () => {
  const root = resolve(import.meta.dirname, '../../docs/performance/distillation');
  const manifest = JSON.parse(readFileSync(resolve(root, 'spool-capacity-evidence.json'), 'utf8')) as SpoolEvidence;
  const hosts = new Set<string>();
  for (const [filename, receipt] of Object.entries(manifest.hosts)) {
    expect(filename).toMatch(/^[a-zA-Z0-9_.-]+\.ts\.gz$/u);
    const bytes = readFileSync(resolve(root, filename)); expect(sha256(bytes)).toBe(receipt.gzipSha256);
    const source = gunzipSync(bytes, {maxOutputLength: 1024 ** 2});
    expect(sha256(Buffer.from(source.toString('utf8').replace(/\r\n/gu, '\n')))).toBe(receipt.canonicalLfSha256);
    hosts.add(receipt.canonicalLfSha256);
  }
  for (const receipt of manifest.diagnostics ?? []) {
    expect(receipt.reportPath).toMatch(/^[a-zA-Z0-9_.-]+\.json$/u);
    expect(receipt.sourceArchive).toMatch(/^[a-zA-Z0-9_.-]+\.mjs\.gz$/u);
    const bytes = readFileSync(resolve(root, receipt.reportPath), 'utf8').replace(/\r\n/gu, '\n');
    expect(sha256(Buffer.from(bytes))).toBe(receipt.reportCanonicalLfSha256);
    const compressed = readFileSync(resolve(root, receipt.sourceArchive));
    expect(sha256(compressed)).toBe(receipt.sourceArchiveSha256);
    expect(sha256(gunzipSync(compressed, {maxOutputLength: 1024 ** 2}))).toBe(receipt.sourceCanonicalLfSha256);
  }
  for (const [filename, receipt] of [[manifest.candidatePatch, manifest.candidatePatchSha256],
    [manifest.publicationPatch, manifest.publicationPatchSha256]]) {
    expect(filename).toMatch(/^[a-zA-Z0-9_.-]+\.patch$/u); expect(sha256(readFileSync(resolve(root, filename!)))).toBe(receipt);
  }
  const paths = new Set<string>(); for (const receipt of manifest.reports) {
    expect(receipt.path).toMatch(/^spool-capacity-(windows|debian)-v1\/[a-zA-Z0-9_.-]+\.json$/u);
    expect(paths.has(receipt.path)).toBe(false); paths.add(receipt.path);
    const bytes = readFileSync(resolve(root, receipt.path), 'utf8');
    expect(sha256(Buffer.from(bytes.replace(/\r\n/gu, '\n')))).toBe(receipt.canonicalLfSha256);
    const report = JSON.parse(bytes) as {status: string; qualificationComplete: boolean; source: {canonicalLfSha256: Record<string, string>}};
    expect(report.status).toBe(receipt.status);
    expect(report.source.canonicalLfSha256[receipt.hostSourcePath ?? 'scripts/qualification/spool-capacity.ts']).toBe(receipt.hostCanonicalLfSha256);
    expect(hosts.has(receipt.hostCanonicalLfSha256)).toBe(true);
    for (const [path, expected] of Object.entries(receipt.auxiliaryCanonicalLfSha256 ?? {})) {
      expect(report.source.canonicalLfSha256[path]).toBe(expected); expect(hosts.has(expected)).toBe(true);
    }
    expect(report.source.canonicalLfSha256['server/rustEngine/distillationSpoolStore.ts']).toBe(receipt.storeCanonicalLfSha256);
    if (receipt.status === 'passed') continue;
    expect(receipt.status).toBe('failed'); expect(report.qualificationComplete).toBe(false);
  }
});

/** Compact source and report receipts for synthetic canonical training capacity. */
interface TrainingCapacityEvidence {
  /** Exact retained host archive. */
  hostGzipSha256: string;
  /** Original Windows host bytes, including its original line endings. */
  hostSha256: string;
  /** Canonical-LF JSON and exact binary input receipts. */
  inputs: Record<string, string>;
  /** Completed admitted/rejected cases. */
  reports: Array<{path: string; canonicalLfSha256: string; status: string}>;
}

it('binds training capacity to Rust inputs and completed work under the admitted device budget', () => {
  const root = resolve(import.meta.dirname, '../../docs/performance/distillation/canonical-capacity-windows-v1');
  const evidence = JSON.parse(readFileSync(resolve(root, 'summary.json'), 'utf8')) as TrainingCapacityEvidence;
  const host = readFileSync(resolve(root, 'host.py.gz')); expect(sha256(host)).toBe(evidence.hostGzipSha256);
  expect(sha256(gunzipSync(host, {maxOutputLength: 1024 ** 2}))).toBe(evidence.hostSha256);
  for (const [filename, expected] of Object.entries(evidence.inputs)) {
    expect(filename).toMatch(/^[a-zA-Z0-9_.-]+$/u);
    const bytes = readFileSync(resolve(root, filename));
    expect(sha256(filename.endsWith('.json') ? Buffer.from(bytes.toString('utf8').replace(/\r\n/gu, '\n')) : bytes)).toBe(expected);
  }
  for (const entry of evidence.reports) {
    expect(entry.path).toMatch(/^[a-zA-Z0-9_.-]+\.json$/u);
    const bytes = readFileSync(resolve(root, entry.path), 'utf8');
    expect(sha256(Buffer.from(bytes.replace(/\r\n/gu, '\n')))).toBe(entry.canonicalLfSha256);
    const report = JSON.parse(bytes) as {status: string; admitted: boolean; qualificationComplete: boolean;
      hostSha256: string; manifestSha256: string; initialWeightsSha256: string; parameters: number; inputSize: number;
      deviceEstimateBytes: number; peakAllocatedDeviceBytes?: number; peakReservedDeviceBytes?: number;
      options: {device: string; deviceMemoryMiB: number; updates: number; batch_size: number; bptt_steps: number; burn_in_steps: number};
      updates: Array<{optimizer_updated: boolean; training_samples: number; burn_in_samples: number}>};
    expect(report.status).toBe(entry.status); expect(report.hostSha256).toBe(evidence.hostSha256);
    expect(report.manifestSha256).toBe(evidence.inputs['manifest.json']);
    const native = JSON.parse(readFileSync(resolve(root, 'receipt.json'), 'utf8')) as {weightsSha256: string};
    expect(report.initialWeightsSha256).toBe(native.weightsSha256);
    expect(report.parameters).toBe(494306); expect(report.inputSize).toBe(504);
    const budget = report.options.deviceMemoryMiB * 1024 ** 2;
    if (report.status === 'rejected') {
      expect(report.admitted).toBe(false); expect(report.qualificationComplete).toBe(false);
      expect(report.deviceEstimateBytes).toBeGreaterThan(budget); expect(report.updates).toHaveLength(0); continue;
    }
    expect(report.status).toBe('passed'); expect(report.admitted).toBe(true); expect(report.qualificationComplete).toBe(true);
    expect(report.deviceEstimateBytes).toBeLessThanOrEqual(budget); expect(report.updates).toHaveLength(report.options.updates);
    for (const update of report.updates) {
      expect(update.optimizer_updated).toBe(true);
      expect(update.training_samples).toBe(report.options.batch_size * report.options.bptt_steps);
      expect(update.burn_in_samples).toBe(report.options.batch_size * report.options.burn_in_steps);
    }
    if (report.options.device === 'cuda:0') {
      expect(report.peakAllocatedDeviceBytes).toBeGreaterThan(0); expect(report.peakReservedDeviceBytes).toBeGreaterThan(0);
      expect(report.peakAllocatedDeviceBytes).toBeLessThanOrEqual(budget); expect(report.peakReservedDeviceBytes).toBeLessThanOrEqual(budget);
    }
  }
});

it('preserves settled interrupted/new capture identities through loaded quota reduction', () => {
  const root = resolve(import.meta.dirname, '../../docs/performance/distillation/spool-capacity-windows-v1');
  const report = JSON.parse(readFileSync(resolve(root, 'max-count-corrected-continuation.json'), 'utf8')) as {
    status: string; qualificationComplete: boolean; acknowledged: number; workspaceRemoved: boolean;
    predecessorCanonicalLfSha256: string; phases: {fullInventory: DistillationDataset[];
      restart64: {milliseconds: number; exactPinnedDownloads: boolean; datasets: DistillationDataset[]};
      restart1: {milliseconds: number; exactPinnedDownloads: boolean; datasets: DistillationDataset[]}}};
  const previous = JSON.parse(readFileSync(resolve(root, 'max-count-candidate.json'), 'utf8')) as {originalCanonicalLfSha256: string};
  expect(report.predecessorCanonicalLfSha256).toBe(previous.originalCanonicalLfSha256);
  expect(report.status).toBe('passed'); expect(report.qualificationComplete).toBe(true); expect(report.workspaceRemoved).toBe(true);
  expect(report.acknowledged).toBe(65536);
  const initial = report.phases.fullInventory; expect(initial).toHaveLength(2);
  expect(initial.map(value => value.state).sort()).toEqual(['interrupted', 'stopped']);
  expect(initial.reduce((sum, value) => sum + value.retainedChunks, 0)).toBe(65536);
  for (const [phase, count] of [[report.phases.restart64, 65536], [report.phases.restart1, 16384]] as const) {
    expect(phase.milliseconds).toBeLessThan(60000); expect(phase.exactPinnedDownloads).toBe(true);
    expect(phase.datasets).toHaveLength(initial.length);
    expect(phase.datasets.reduce((sum, value) => sum + value.retainedChunks, 0)).toBe(count);
    expect(phase.datasets.reduce((sum, value) => sum + BigInt('0x' + value.retainedBytes), 0n)).toBe(BigInt(count) * 65536n);
    for (const restored of phase.datasets) {
      const before = initial.find(value => value.contract.captureEpoch === restored.contract.captureEpoch); expect(before).toBeDefined();
      for (const field of ['contract', 'state', 'productionWatermark', 'storageWatermark', 'resolvedThrough', 'error'] as const) {
        expect(restored[field]).toEqual(before![field]);
      }
      for (const field of ['nativeDroppedChunks', 'nativeDroppedSamples', 'nativeDroppedBytes', 'spoolDroppedChunks'] as const) {
        expect(restored[field]).toBe('0000000000000000');
      }
    }
  }
});

/** Loaded replay reports retain exact fixture-generator and measured Python sources. */
interface ReplayCapacityEvidence {
  /** Original compressed measured host bytes. */
  hostGzipSha256: string;
  /** Exact measured host identity. */
  hostSha256: string;
  /** Generator source and separately locked dependency graph. */
  generatorInputsCanonicalLfSha256: Record<string, string>;
  /** Immutable native fixture receipt. */
  referenceCanonicalLfSha256: string;
  /** Distinct loaded RAM envelopes. */
  reports: Array<{path: string; canonicalLfSha256: string; ramMiB: number}>;
}

it.each(['windows', 'debian'] as const)('binds %s loaded replay RAM to genuine Rust codec bytes and exact independent restore', platform => {
  const root = resolve(import.meta.dirname, `../../docs/performance/distillation/replay-capacity-${platform}-v1`);
  const summary = JSON.parse(readFileSync(resolve(root, 'summary.json'), 'utf8')) as ReplayCapacityEvidence;
  const host = readFileSync(resolve(root, 'host.py.gz')); expect(sha256(host)).toBe(summary.hostGzipSha256);
  expect(sha256(gunzipSync(host, {maxOutputLength: 1024 ** 2}))).toBe(summary.hostSha256);
  for (const [filename, expected] of Object.entries(summary.generatorInputsCanonicalLfSha256)) {
    expect(filename).toMatch(/^(Cargo\.toml|Cargo\.lock|main\.rs)$/u);
    const bytes = readFileSync(resolve(import.meta.dirname, 'teacher-replay-fixture', filename), 'utf8');
    expect(sha256(Buffer.from(bytes.replace(/\r\n/gu, '\n')))).toBe(expected);
  }
  const referenceBytes = readFileSync(resolve(root, 'reference.json'), 'utf8').replace(/\r\n/gu, '\n');
  expect(sha256(Buffer.from(referenceBytes))).toBe(summary.referenceCanonicalLfSha256);
  const reference = JSON.parse(referenceBytes) as {sourceSha256: string; hostCanonicalLfSha256: string;
    bundleSha256: string; chunks: number; samplesPerChunk: number; encodedBytes: number; bundleBytes: number};
  expect(reference.hostCanonicalLfSha256).toBe(summary.generatorInputsCanonicalLfSha256['main.rs']);
  expect(reference.bundleBytes).toBe(reference.encodedBytes + reference.chunks * 4);
  expect(summary.reports.map(value => value.ramMiB)).toEqual([64, 256, 2048]);
  for (const entry of summary.reports) {
    expect(entry.path).toMatch(/^ram-[0-9]+\.json$/u);
    const bytes = readFileSync(resolve(root, entry.path), 'utf8').replace(/\r\n/gu, '\n');
    expect(sha256(Buffer.from(bytes))).toBe(entry.canonicalLfSha256);
    const report = JSON.parse(bytes) as {status: string; qualificationComplete: boolean; hostSha256: string;
      referenceSha256: string; nativeSourceSha256: string; fixtureHostCanonicalLfSha256: string; bundleSha256: string;
      environment: {pythonOptimized: boolean; pointerBytes: number; platform: string};
      options: {ramMiB: number}; decodedChunks: number; retainedChunks: number; ownedReplayBytes: number;
      perChunkChargedBytes: number; evictedChunks: number; evictedSamples: number;
      independentlyDecodedRestoreExact: boolean; filledPeakRssBytes: number; peakRssBytes: number};
    expect(report.status).toBe('passed'); expect(report.qualificationComplete).toBe(true);
    expect(report.hostSha256).toBe(summary.hostSha256); expect(report.referenceSha256).toBe(summary.referenceCanonicalLfSha256);
    expect(report.nativeSourceSha256).toBe(reference.sourceSha256);
    expect(report.fixtureHostCanonicalLfSha256).toBe(reference.hostCanonicalLfSha256);
    expect(report.bundleSha256).toBe(reference.bundleSha256); expect(report.options.ramMiB).toBe(entry.ramMiB);
    expect(report.environment.pythonOptimized).toBe(false); expect(report.environment.pointerBytes).toBe(8);
    expect(report.environment.platform).toMatch(platform === 'windows' ? /^Windows-/u : /^Linux-/u);
    const budget = entry.ramMiB * 1024 ** 2; const fixed = 256 * 1024;
    expect(report.decodedChunks).toBe(reference.chunks); expect(report.evictedChunks).toBeGreaterThan(0);
    expect(report.retainedChunks + report.evictedChunks).toBe(reference.chunks);
    expect(report.evictedSamples).toBe(report.evictedChunks * reference.samplesPerChunk);
    expect(report.retainedChunks).toBe(Math.floor((budget - fixed) / report.perChunkChargedBytes));
    expect(report.ownedReplayBytes).toBe(fixed + report.retainedChunks * report.perChunkChargedBytes);
    expect(report.ownedReplayBytes).toBeLessThanOrEqual(budget);
    expect(budget - report.ownedReplayBytes).toBeLessThan(report.perChunkChargedBytes);
    expect(report.independentlyDecodedRestoreExact).toBe(true);
    expect(report.filledPeakRssBytes).toBeGreaterThan(0); expect(report.peakRssBytes).toBeGreaterThanOrEqual(report.filledPeakRssBytes);
  }
});


it.each(['windows', 'debian'] as const)('binds %s full-byte recovery to its failed predecessor and exact retained identities', platform => {
  const root = resolve(import.meta.dirname, `../../docs/performance/distillation/spool-capacity-${platform}-v1`);
  const previousBytes = readFileSync(resolve(root, 'full64g-candidate.json'), 'utf8').replace(/\r\n/gu, '\n');
  const previous = JSON.parse(previousBytes) as {status: string; selectedByteQuotaLoaded: boolean;
    phases: {rotation: {dataset: DistillationDataset}}};
  const recovered = JSON.parse(readFileSync(resolve(root, 'full64g-recovery.json'), 'utf8')) as {
    status: string; qualificationComplete: boolean; workspaceRemoved: boolean; predecessorCanonicalLfSha256: string;
    phases: {fullRestart: {milliseconds: number; recoveryBudgetMs: number};
      smallerRestart: {milliseconds: number; recoveryBudgetMs: number};
      fullDataset: DistillationDataset; finalDataset: DistillationDataset;
      leaseChecks: {freshRecentPinExact: boolean; checks: Array<{result: string}>}}};
  expect(previous.status).toBe('failed'); expect(previous.selectedByteQuotaLoaded).toBe(true);
  expect(recovered.predecessorCanonicalLfSha256).toBe(sha256(Buffer.from(previousBytes)));
  expect(recovered.status).toBe('passed'); expect(recovered.qualificationComplete).toBe(true);
  expect(recovered.workspaceRemoved).toBe(true);
  for (const phase of [recovered.phases.fullRestart, recovered.phases.smallerRestart]) {
    expect(phase.recoveryBudgetMs).toBe(660000); expect(phase.milliseconds).toBeLessThan(phase.recoveryBudgetMs);
  }
  const before = previous.phases.rotation.dataset;
  for (const [dataset, chunks] of [[recovered.phases.fullDataset, 65536], [recovered.phases.finalDataset, 1024]] as const) {
    expect(dataset.retainedChunks).toBe(chunks);
    expect(BigInt('0x' + dataset.retainedBytes)).toBe(BigInt(chunks) * 1024n ** 2n);
    for (const field of ['contract', 'state', 'productionWatermark', 'storageWatermark', 'resolvedThrough', 'error'] as const) {
      expect(dataset[field]).toEqual(before[field]);
    }
    for (const field of ['nativeDroppedChunks', 'nativeDroppedSamples', 'nativeDroppedBytes', 'spoolDroppedChunks'] as const) {
      expect(dataset[field]).toBe('0000000000000000');
    }
  }
  expect(recovered.phases.leaseChecks.freshRecentPinExact).toBe(true);
  expect(recovered.phases.leaseChecks.checks).toHaveLength(2);
  for (const check of recovered.phases.leaseChecks.checks) expect(['exact', 'expired-as-declared']).toContain(check.result);
});


/** Loaded trainer checkpoint measurements bind the same portable Rust fixture and graph. */
interface CheckpointCapacityEvidence {
  /** Exact archived measured host. */
  hostGzipSha256: string;
  /** Raw measured source, including its original line endings. */
  hostSha256: string;
  /** Canonical source graph receipt. */
  canonicalGraphReceiptCanonicalLfSha256: string;
  /** Canonical graph input. */
  canonicalGraphManifestCanonicalLfSha256: string;
  /** Previously retained native codec fixture provenance. */
  replayReferenceCanonicalLfSha256: string;
  /** Completed load and device cases. */
  reports: Array<{path: string; canonicalLfSha256: string; ramMiB: number; device: string; checkpointSaves?: number}>;
}

it.each([['windows', 'capacity'], ['debian', 'capacity'], ['windows', 'rotation'], ['debian', 'rotation']] as const)(
  'binds %s checkpoint %s to populated Adam and exact restored updates', (platform, kind) => {
  const base = resolve(import.meta.dirname, '../../docs/performance/distillation');
  const root = resolve(base, `checkpoint-${kind}-${platform}-v1`);
  const summary = JSON.parse(readFileSync(resolve(root, 'summary.json'), 'utf8')) as CheckpointCapacityEvidence;
  const host = readFileSync(resolve(root, 'host.py.gz')); expect(sha256(host)).toBe(summary.hostGzipSha256);
  expect(sha256(gunzipSync(host, {maxOutputLength: 1024 ** 2}))).toBe(summary.hostSha256);
  const graphBytes = readFileSync(resolve(base, 'canonical-capacity-windows-v1/manifest.json'), 'utf8').replace(/\r\n/gu, '\n');
  expect(sha256(Buffer.from(graphBytes))).toBe(summary.canonicalGraphManifestCanonicalLfSha256);
  const receiptBytes = readFileSync(resolve(base, 'canonical-capacity-windows-v1/receipt.json'), 'utf8').replace(/\r\n/gu, '\n');
  expect(sha256(Buffer.from(receiptBytes))).toBe(summary.canonicalGraphReceiptCanonicalLfSha256);
  const receipt = JSON.parse(receiptBytes) as {weightsSha256: string};
  const referenceBytes = readFileSync(resolve(base, 'replay-capacity-windows-v1/reference.json'), 'utf8').replace(/\r\n/gu, '\n');
  expect(sha256(Buffer.from(referenceBytes))).toBe(summary.replayReferenceCanonicalLfSha256);
  const reference = JSON.parse(referenceBytes) as {bundleSha256: string; chunks: number; encodedBytes: number};
  expect(summary.reports.length).toBeGreaterThan(0);
  for (const entry of summary.reports) {
    expect(entry.path).toMatch(/^[a-zA-Z0-9_.-]+\.json$/u);
    const bytes = readFileSync(resolve(root, entry.path), 'utf8').replace(/\r\n/gu, '\n');
    expect(sha256(Buffer.from(bytes))).toBe(entry.canonicalLfSha256);
    const report = JSON.parse(bytes) as {status: string; qualificationComplete: boolean; workspaceRemoved: boolean;
      hostSha256: string; referenceSha256: string; manifestSha256: string; initialWeightsSha256: string; bundleSha256: string;
      options: {ramMiB: number; device: string; checkpointSaves?: number}; decodedChunks: number; retainedChunks: number; evictedChunks: number;
      ownedReplayBytes: number; checkpointBytes: number; parameters: number; inputSize: number;
      completedUpdates: number; trainingSamples: number; burnInSamples: number; nextUpdateExact: boolean;
      independentReplayOwners: boolean; peakAllocatedDeviceBytes?: number; peakRssBytes: number;
      rotation?: {saves: Array<{serial: number; bytes: number}>; retained: number; retainedBytes: number; oldestRotated: boolean};
      nextUpdateWitness: {weights: string; optimizer: string; sampler: string; state: string; windows: string}};
    expect(report.status).toBe('passed'); expect(report.qualificationComplete).toBe(true); expect(report.workspaceRemoved).toBe(true);
    expect(report.hostSha256).toBe(summary.hostSha256); expect(report.referenceSha256).toBe(summary.replayReferenceCanonicalLfSha256);
    expect(report.manifestSha256).toBe(summary.canonicalGraphManifestCanonicalLfSha256);
    expect(report.initialWeightsSha256).toBe(receipt.weightsSha256); expect(report.bundleSha256).toBe(reference.bundleSha256);
    expect(report.options).toEqual({ramMiB: entry.ramMiB, device: entry.device,
      ...(entry.checkpointSaves === undefined ? {} : {checkpointSaves: entry.checkpointSaves})});
    if (kind === 'rotation') {
      expect(entry.checkpointSaves).toBe(4); expect(report.rotation).toBeDefined();
      const rotation = report.rotation!;
      expect(rotation.saves.map(value => value.serial)).toEqual([1, 2, 3, 4]);
      expect(rotation.retained).toBe(3); expect(rotation.oldestRotated).toBe(true);
      expect(rotation.retainedBytes).toBe(rotation.saves.slice(1).reduce((sum, value) => sum + value.bytes, 0));
      expect(rotation.saves[3]!.bytes).toBe(report.checkpointBytes);
    }
    expect(report.parameters).toBe(494306); expect(report.inputSize).toBe(504);
    expect(report.decodedChunks).toBe(reference.chunks); expect(report.evictedChunks).toBeGreaterThan(0);
    expect(report.retainedChunks + report.evictedChunks).toBe(reference.chunks);
    expect(report.ownedReplayBytes).toBeLessThanOrEqual(entry.ramMiB * 1024 ** 2);
    expect(report.checkpointBytes).toBeGreaterThan(report.retainedChunks * reference.encodedBytes / reference.chunks);
    expect(report.nextUpdateExact).toBe(true); expect(report.independentReplayOwners).toBe(true);
    expect(report.completedUpdates).toBe(2); expect(report.trainingSamples).toBe(4096); expect(report.burnInSamples).toBe(2048);
    expect(report.peakRssBytes).toBeGreaterThan(report.ownedReplayBytes);
    for (const field of ['weights', 'optimizer', 'sampler', 'state', 'windows'] as const) {
      expect(report.nextUpdateWitness[field]).toMatch(/^[0-9a-f]{64}$/u);
    }
    if (entry.device === 'cuda:0') {
      expect(report.peakAllocatedDeviceBytes).toBeGreaterThan(0);
      expect(report.peakAllocatedDeviceBytes).toBeLessThanOrEqual(8 * 1024 ** 3);
    }
  }
});


/** Large-graph cohort receipts keep rejected admission distinct from completed gameplay. */
interface LargeWorkloadEvidence {
  /** Optional current-source P1 attribution; its performance gates retain measured outcomes. */
  phaseBaseline?: EvidenceRecord;
  /** Existing native qualifier construction ceiling, independent of the checkpoint envelope. */
  qualificationConstructionCeilingBytes: number;
  /** Canonical-LF native summary and its source identities. */
  nativeSummaryCanonicalLfSha256: string;
  /** All measured host/helper bytes are retained losslessly. */
  sources: Record<string, {archive: string; gzipSha256: string; canonicalLfSha256: string}>;
  /** Exact original and compressed bytes for each current-envelope case. */
  records: Array<{name: string; report: string; reportSha256: string; originalReportSha256: string; admitted: boolean}>;
}

it.each(['windows', 'debian'] as const)('preserves %s large-graph admission and worker-continuation evidence', platform => {
  const root = resolve(import.meta.dirname, `../../docs/performance/sensor-v4/large-motion32-${platform}-v1`);
  const manifest = JSON.parse(readFileSync(resolve(root, 'summary.json'), 'utf8')) as LargeWorkloadEvidence;
  const nativeSummary = Buffer.from(readFileSync(resolve(root, 'native-summary.json'), 'utf8').replace(/\r\n/gu, '\n'));
  expect(sha256(nativeSummary)).toBe(manifest.nativeSummaryCanonicalLfSha256);
  const native = JSON.parse(nativeSummary.toString('utf8')) as {checksPassed: boolean; releaseQualified: boolean;
    nativeSourceSha256: string; sourceCanonicalLfSha256: Record<string, string>; checkpointEnvelopeBytes: number;
    workerContinuationExact: boolean; warmupSteps: number; measuredSteps: number};
  expect(native.checksPassed).toBe(true); expect(native.releaseQualified).toBe(false);
  expect(native.workerContinuationExact).toBe(true); expect(native.checkpointEnvelopeBytes).toBe(512 * 1024 ** 2);
  expect(manifest.qualificationConstructionCeilingBytes).toBe(4 * 1024 ** 3);
  expect(native.warmupSteps).toBe(600); expect(native.measuredSteps).toBe(3600);
  for (const [filename, source] of Object.entries(manifest.sources)) {
    expect(filename).toMatch(/^qualify-[a-z-]+\.py$/u); expect(source.archive).toBe(filename + '.gz');
    const compressed = readFileSync(resolve(root, source.archive)); expect(sha256(compressed)).toBe(source.gzipSha256);
    const bytes = gunzipSync(compressed, {maxOutputLength: 1024 ** 2});
    expect(sha256(Buffer.from(bytes.toString('utf8').replace(/\r\n/gu, '\n')))).toBe(source.canonicalLfSha256);
    expect(native.sourceCanonicalLfSha256[filename]).toBe(source.canonicalLfSha256);
  }
  if (manifest.phaseBaseline) {
    const baseline = manifest.phaseBaseline; expect(baseline.report).toBe('current-p1-phase.json.gz');
    const compressed = readFileSync(resolve(root, baseline.report)); expect(sha256(compressed)).toBe(baseline.reportSha256);
    const bytes = gunzipSync(compressed, {maxOutputLength: 4 * 1024 ** 2}); expect(sha256(bytes)).toBe(baseline.originalReportSha256);
    const report = JSON.parse(bytes.toString('utf8')) as {sourceSha256: string; admitted: boolean; population: number;
      graphSize: string; parameterCount: number; completedSteps: number;
      nativePhaseCosts: Record<string, number>; capture: {exactSamplesMatch: boolean; queueAccountingMatches: boolean}};
    expect(report.sourceSha256).toBe(native.nativeSourceSha256); expect(report.admitted).toBe(true);
    expect(report.population).toBe(300); expect(report.graphSize).toBe('modest'); expect(report.parameterCount).toBe(25042);
    expect(report.completedSteps).toBe(4200); expect(report.capture.exactSamplesMatch).toBe(true);
    expect(report.capture.queueAccountingMatches).toBe(true); expect(report.nativePhaseCosts['samples']).toBe(3600);
    const costs = report.nativePhaseCosts;
    expect(['indexMicros', 'inferenceMicros', 'sensingMicros', 'otherControlMicros', 'worldStepMicros', 'otherMicros']
      .reduce((sum, field) => sum + costs[field]!, 0)).toBe(costs['totalMicros']);
  }
  expect(manifest.records.map(value => value.name)).toEqual(['large300-admission', 'large55-w1', 'large55-w3']);
  const admitted: Record<string, unknown>[] = [];
  for (const receipt of manifest.records) {
    expect(receipt.report).toMatch(/^[a-z0-9-]+\.json\.gz$/u);
    const compressed = readFileSync(resolve(root, receipt.report)); expect(sha256(compressed)).toBe(receipt.reportSha256);
    const bytes = gunzipSync(compressed, {maxOutputLength: 4 * 1024 ** 2}); expect(sha256(bytes)).toBe(receipt.originalReportSha256);
    const report = JSON.parse(bytes.toString('utf8')) as Record<string, unknown>;
    expect(report['sourceSha256']).toBe(native.nativeSourceSha256); expect(report['graphSize']).toBe('large');
    expect(report['admitted']).toBe(receipt.admitted);
    if (receipt.name === 'large300-admission') {
      expect(report['admitted']).toBe(false); expect(report['completedSteps']).toBe(0); expect(report['observations']).toBe(0);
      expect(report['capture']).toBeNull(); expect(report['rejection']).toMatch(/494306 parameters.*512\.00 MiB/u); continue;
    }
    expect(report['admitted']).toBe(true); expect(report['population']).toBe(55); expect(report['parameterCount']).toBe(494306);
    expect(report['completedSteps']).toBe(4200); expect(report['measuredSteps']).toBe(3600);
    expect(report['sharedBatchBudgetHeld']).toBe(true); expect(report['peakRssBytes']).toBeGreaterThan(0);
    const capture = report['capture'] as {exactSamplesMatch: boolean; queueAccountingMatches: boolean; droppedChunks: number;
      sourceSampleSha256: string[]; drainedSampleSha256: string[]; samplesObserved: number[]; samplesDrained: number[]};
    expect(capture.exactSamplesMatch).toBe(true); expect(capture.queueAccountingMatches).toBe(true); expect(capture.droppedChunks).toBe(0);
    expect(capture.samplesObserved).toEqual(capture.samplesDrained); expect(capture.sourceSampleSha256).toEqual(capture.drainedSampleSha256);
    admitted.push(report);
  }
  expect(admitted).toHaveLength(2);
  for (const field of ['sourceSha256', 'target', 'rustc', 'mathBackend', 'sensorSha256', 'parameterCount',
    'continuationSha256', 'consumerWork', 'raw', 'normalized']) expect(admitted[0]![field]).toEqual(admitted[1]![field]);
  for (const field of ['sourceSampleSha256', 'drainedSampleSha256', 'samplesObserved', 'dueTeacherHistogram']) {
    expect((admitted[0]!['capture'] as Record<string, unknown>)[field]).toEqual((admitted[1]!['capture'] as Record<string, unknown>)[field]);
  }
});


/** Ray-budget candidates preserve successful service separately from failed tail latency. */
interface RayBudgetEvidence {
  /** Current measured native implementation. */
  nativeSourceSha256: string;
  /** These partial workload probes never select production defaults. */
  releaseQualified: boolean;
  /** Exact measured TypeScript sources. */
  sources: Record<string, {archive: string; gzipSha256: string; rawSha256: string; canonicalLfSha256: string}>;
  /** Controlled-step and real-clock cases for each worker count. */
  native: Array<EvidenceRecord & {workers: number}>;
  /** Separate real background-clock measurements. */
  wall: Array<EvidenceRecord & {workers: number}>;
  /** Actual finite staging rejection, with no executed-world claim. */
  stagingAdmission: {report: string; canonicalLfSha256: string};
}

it('preserves zero-incomplete ray candidates, real-clock timing failures and unchanged staging admission', () => {
  const root = resolve(import.meta.dirname, '../../docs/performance/sensor-v4/ray-budget-windows-v1');
  const manifest = JSON.parse(readFileSync(resolve(root, 'summary.json'), 'utf8')) as RayBudgetEvidence;
  expect(manifest.releaseQualified).toBe(false);
  for (const [filename, receipt] of Object.entries(manifest.sources)) {
    expect(filename).toMatch(/^(sensor-wall|sensorWallSummary)\.ts$/u); expect(receipt.archive).toBe(filename + '.gz');
    const compressed = readFileSync(resolve(root, receipt.archive)); expect(sha256(compressed)).toBe(receipt.gzipSha256);
    const bytes = gunzipSync(compressed, {maxOutputLength: 1024 ** 2}); expect(sha256(bytes)).toBe(receipt.rawSha256);
    expect(sha256(Buffer.from(bytes.toString('utf8').replace(/\r\n/gu, '\n')))).toBe(receipt.canonicalLfSha256);
  }
  /** Decode only retained reports; sample payloads remain opaque to JavaScript. */
  const read = (receipt: EvidenceRecord): Record<string, unknown> => {
    expect(receipt.report).toMatch(/^(native|wall)-w[78]\.json\.gz$/u);
    const compressed = readFileSync(resolve(root, receipt.report)); expect(sha256(compressed)).toBe(receipt.reportSha256);
    const bytes = gunzipSync(compressed, {maxOutputLength: 4 * 1024 ** 2}); expect(sha256(bytes)).toBe(receipt.originalReportSha256);
    return JSON.parse(bytes.toString('utf8')) as Record<string, unknown>;
  };
  expect(manifest.native.map(value => value.workers)).toEqual([7, 8]);
  const controlled = manifest.native.map(receipt => {
    const report = read(receipt); expect(report['sourceSha256']).toBe(manifest.nativeSourceSha256);
    expect(report['workers']).toBe(receipt.workers); expect(report['admitted']).toBe(true);
    expect(report['rayCap']).toBe(512); expect(report['rayBatch']).toBe(131072);
    expect(report['completedSteps']).toBe(4200); expect(report['allConsumerRayIncompleteFraction']).toBe(0);
    expect(report['nativeStepP95Within60hz']).toBe(true); return report;
  });
  for (const field of ['sourceSha256', 'target', 'rustc', 'mathBackend', 'sensorSha256', 'parameterCount',
    'continuationSha256', 'consumerWork', 'raw', 'normalized']) expect(controlled[0]![field]).toEqual(controlled[1]![field]);
  expect(manifest.wall.map(value => value.workers)).toEqual([7, 8]);
  for (const receipt of manifest.wall) {
    const report = read(receipt); expect(report['workers']).toBe(receipt.workers);
    const source = report['source'] as {nativeSourceSha256: string; hostSha256: string; summarySha256: string};
    expect(source.nativeSourceSha256).toBe(manifest.nativeSourceSha256);
    expect(source.hostSha256).toBe(manifest.sources['sensor-wall.ts']!.rawSha256);
    expect(source.summarySha256).toBe(manifest.sources['sensorWallSummary.ts']!.rawSha256);
    const requested = (report['workload'] as {request: {rayCap: number; rayBatch: number}}).request;
    expect(requested.rayCap).toBe(512); expect(requested.rayBatch).toBe(131072);
    const summary = summarizeSensorWall(report['reads'] as SensorWallRead[], report['requestedSeconds'] as number);
    expect(summary).toEqual(report['summary']); expect(summary.meetsRealtimeGate).toBe(true);
    expect(summary.droppedWallMicros).toBe('0'); expect(summary.meetsExistingStepTimingGate).toBe(false);
    expect(summary.meetsMeasuredGates).toBe(false);
    expect((report['diagnosticDelivery'] as {captureHasNoDrops: boolean}).captureHasNoDrops).toBe(true);
    expect((report['frames'] as {copies: number}).copies).toBeGreaterThan(0);
  }
  expect(manifest.stagingAdmission.report).toBe('staging-w10.json');
  const bytes = readFileSync(resolve(root, manifest.stagingAdmission.report), 'utf8').replace(/\r\n/gu, '\n');
  expect(sha256(Buffer.from(bytes))).toBe(manifest.stagingAdmission.canonicalLfSha256);
  const rejection = JSON.parse(bytes) as {status: string; qualificationComplete: boolean; nativeSourceSha256: string;
    stagingBudgetBytes: number; requiredStagingBytes: number; workspaceRemoved: boolean; nativeReportProduced: boolean};
  expect(rejection.status).toBe('rejected'); expect(rejection.qualificationComplete).toBe(false);
  expect(rejection.nativeSourceSha256).toBe(manifest.nativeSourceSha256);
  expect(rejection.stagingBudgetBytes).toBe(512 * 1024 ** 2); expect(rejection.requiredStagingBytes).toBeGreaterThan(rejection.stagingBudgetBytes);
  expect(rejection.workspaceRemoved).toBe(true); expect(rejection.nativeReportProduced).toBe(false);
});


it('recomputes passing seeded Windows P0/P1 head-pair clock gates from retained health reads', () => {
  const root = resolve(import.meta.dirname, '../../docs/performance/sensor-v4/head-pair-bound-windows-v1');
  const manifest = JSON.parse(readFileSync(resolve(root, 'wall.json'), 'utf8')) as
    Pick<RayBudgetEvidence, 'nativeSourceSha256' | 'releaseQualified' | 'sources'> & {
      wall: Array<EvidenceRecord & {workers: number; scenario: 'P0' | 'P1'}>;
    };
  expect(manifest.releaseQualified).toBe(false);
  for (const [filename, receipt] of Object.entries(manifest.sources)) {
    expect(filename).toMatch(/^(sensor-wall|sensorWallSummary)\.ts$/u);
    expect(receipt.archive).toBe(filename + '.gz');
    const compressed = readFileSync(resolve(root, receipt.archive)); expect(sha256(compressed)).toBe(receipt.gzipSha256);
    const bytes = gunzipSync(compressed, {maxOutputLength: 1024 ** 2}); expect(sha256(bytes)).toBe(receipt.rawSha256);
    expect(sha256(Buffer.from(bytes.toString('utf8').replace(/\r\n/gu, '\n')))).toBe(receipt.canonicalLfSha256);
  }
  expect(manifest.wall.map(value => [value.scenario, value.workers])).toEqual([['P0', 8], ['P1', 8]]);
  for (const receipt of manifest.wall) {
    expect(receipt.report).toBe(`wall-${receipt.scenario}-w8.json.gz`);
    const compressed = readFileSync(resolve(root, receipt.report)); expect(sha256(compressed)).toBe(receipt.reportSha256);
    const bytes = gunzipSync(compressed, {maxOutputLength: 4 * 1024 ** 2}); expect(sha256(bytes)).toBe(receipt.originalReportSha256);
    const report = JSON.parse(bytes.toString('utf8')) as Record<string, unknown>;
    const source = report['source'] as {nativeSourceSha256: string; hostSha256: string; summarySha256: string};
    expect(source.nativeSourceSha256).toBe(manifest.nativeSourceSha256);
    expect(source.hostSha256).toBe(manifest.sources['sensor-wall.ts']!.rawSha256);
    expect(source.summarySha256).toBe(manifest.sources['sensorWallSummary.ts']!.rawSha256);
    expect(report['scenario']).toBe(receipt.scenario); expect(report['workers']).toBe(receipt.workers);
    const request = (report['workload'] as {request: {rayCap: number; rayBatch: number}}).request;
    expect(request.rayCap).toBe(512); expect(request.rayBatch).toBe(131072);
    const summary = summarizeSensorWall(report['reads'] as SensorWallRead[], report['requestedSeconds'] as number);
    expect(summary).toEqual(report['summary']); expect(summary.meetsRealtimeGate).toBe(true);
    expect(summary.droppedWallMicros).toBe('0'); expect(summary.meetsExistingStepTimingGate).toBe(true);
    expect(summary.meetsMeasuredGates).toBe(true);
    expect((report['diagnosticDelivery'] as {captureHasNoDrops: boolean}).captureHasNoDrops).toBe(true);
    expect((report['frames'] as {copies: number}).copies).toBeGreaterThan(0);
  }
});


it('preserves the negative Debian three-worker probe and exact continuation against eight workers', () => {
  const root = resolve(import.meta.dirname, '../../docs/performance/sensor-v4/head-pair-bound-debian-v1');
  const manifest = JSON.parse(readFileSync(resolve(root, 'workers.json'), 'utf8')) as {
    nativeSourceSha256: string; releaseQualified: boolean; records: Array<EvidenceRecord & {workers: number}>;
  };
  expect(manifest.releaseQualified).toBe(false);
  expect(manifest.records.map(value => value.workers)).toEqual([3, 8]);
  const reports = manifest.records.map(receipt => {
    expect(receipt.report).toMatch(/^(worker-3|1-candidate)\.json\.gz$/u);
    const compressed = readFileSync(resolve(root, receipt.report)); expect(sha256(compressed)).toBe(receipt.reportSha256);
    const bytes = gunzipSync(compressed, {maxOutputLength: 4 * 1024 ** 2}); expect(sha256(bytes)).toBe(receipt.originalReportSha256);
    const report = JSON.parse(bytes.toString('utf8')) as Record<string, unknown>;
    expect(report['sourceSha256']).toBe(manifest.nativeSourceSha256); expect(report['workers']).toBe(receipt.workers);
    expect(report['completedSteps']).toBe(4200); expect(report['allConsumerRayIncompleteFraction']).toBe(0);
    expect(report['nativeStepP95Within60hz']).toBe(false); return report;
  });
  for (const field of ['sourceSha256', 'target', 'rustc', 'mathBackend', 'sensorSha256', 'parameterCount',
    'continuationSha256', 'consumerWork', 'raw', 'normalized']) expect(reports[0]![field]).toEqual(reports[1]![field]);
  for (const field of ['sourceSampleSha256', 'drainedSampleSha256']) {
    expect((reports[0]!['capture'] as Record<string, unknown>)[field]).toEqual((reports[1]!['capture'] as Record<string, unknown>)[field]);
  }
});
