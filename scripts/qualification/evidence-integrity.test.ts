import { summarizeNativeSoak, type NativeSoakReceipt } from './soak-summary.ts';
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


/** Exact immutable files retained from one native/CUDA tactical study. */
interface TacticalStudyFile {
  /** Original study-relative path. */
  path: string;
  /** Safe retained gzip path. */
  archive: string;
  /** Original byte length. */
  bytes: number;
  /** Original byte digest. */
  sha256: string;
  /** Retained gzip digest. */
  gzipSha256: string;
}

it('binds CUDA tactical ablations to identical windows, exact artifacts and native held-out physics', () => {
  const root = resolve(import.meta.dirname, '../../docs/performance/sensor-v4/tactical-ablation-windows-v1');
  const evidence = JSON.parse(readFileSync(resolve(root, 'evidence.json'), 'utf8')) as {
    nativeSourceSha256: string; candidatePatchSha256: string; files: TacticalStudyFile[];
  };
  expect(sha256(readFileSync(resolve(root, 'candidate.patch')))).toBe(evidence.candidatePatchSha256);
  const files = new Map<string, Buffer>();
  for (const entry of evidence.files) {
    expect(entry.path).toMatch(/^[a-zA-Z0-9_.-]+(?:\/[a-zA-Z0-9_.-]+)*$/u);
    expect(entry.path.split('/')).not.toContain('..'); expect(entry.archive).toBe(entry.path + '.gz');
    expect(Number.isSafeInteger(entry.bytes) && entry.bytes > 0 && entry.bytes <= 256 * 1024 ** 2).toBe(true);
    expect(files.has(entry.path)).toBe(false);
    const compressed = readFileSync(resolve(root, entry.archive)); expect(sha256(compressed)).toBe(entry.gzipSha256);
    const bytes = gunzipSync(compressed, {maxOutputLength: entry.bytes + 1});
    expect(bytes.byteLength).toBe(entry.bytes); expect(sha256(bytes)).toBe(entry.sha256);
    files.set(entry.path, bytes);
  }
  /** Parse only metadata; observation, state and portable-weight bytes remain opaque. */
  const json = (path: string): Record<string, unknown> => JSON.parse(files.get(path)!.toString('utf8')) as Record<string, unknown>;
  const summary = json('summary.json');
  expect(summary['status']).toBe('measured'); expect(summary['measurementComplete']).toBe(true);
  expect(summary['releaseQualified']).toBe(false);
  expect((summary['environment'] as {device: string; tf32: boolean}).device).toBe('cuda:0');
  expect((summary['environment'] as {device: string; tf32: boolean}).tf32).toBe(false);
  const corpus = json('corpus/corpus.json'); expect(sha256(files.get('corpus/corpus.json')!)).toBe(summary['corpusManifestSha256']);
  expect(corpus['inputSize']).toBe(264); expect(corpus['seedCount']).toBe(3);
  for (const object of corpus['objects'] as Array<{file: string; bytes: number; sha256: string}>) {
    const bytes = files.get('corpus/' + object.file)!;
    expect(bytes.byteLength).toBe(object.bytes); expect(sha256(bytes)).toBe(object.sha256);
  }
  const variants = summary['variants'] as Array<Record<string, unknown>>;
  expect(variants.map(value => value['variant'])).toEqual([
    'full', 'no-bearing', 'no-heading', 'no-velocity', 'no-urgency', 'no-density', 'core-only'
  ]);
  let referenceWindows: unknown; let referencePhysics: unknown;
  for (const variant of variants) {
    const name = variant['variant'] as string;
    expect(variant['nativeSourceSha256']).toBe(evidence.nativeSourceSha256);
    expect(variant['initialWeightsSha256']).not.toBe(variant['trainedWeightsSha256']);
    const training = variant['training'] as Record<string, unknown>;
    expect(training['updates']).toBe(250); expect(training['trainingRows']).toBeGreaterThan(0);
    const identity = [training['scheduleSha256'], training['trainingRows'], training['burnInRows']];
    referenceWindows ??= identity; expect(identity).toEqual(referenceWindows);
    const parity = variant['parity'] as {steps: number; outputMaxAbs: number; stateMaxAbs: number};
    expect(parity.steps).toBeGreaterThan(0); expect(parity.steps).toBeLessThanOrEqual(256);
    expect(Number.isFinite(parity.outputMaxAbs) && parity.outputMaxAbs >= 0).toBe(true);
    expect(Number.isFinite(parity.stateMaxAbs) && parity.stateMaxAbs >= 0).toBe(true);
    for (const [phase, artifact] of [['nativeBefore', 'initial'], ['nativeAfter', 'trained']] as const) {
      const receipt = variant[phase] as {report: string; sha256: string};
      expect(receipt.report).toBe(phase === 'nativeBefore' ? 'native-before.json' : 'native-after.json');
      const path = name + '/' + receipt.report; expect(sha256(files.get(path)!)).toBe(receipt.sha256);
      const native = json(path);
      expect(native['sourceSha256']).toBe(evidence.nativeSourceSha256);
      expect(native['kind']).toBe('native-student-tactics-v1'); expect(native['firstSeed']).toBe(3);
      expect(native['seedCount']).toBe(1);
      const descriptor = native['candidate'] as {artifactSha256: string; weightsSha256: string; parameterCount: number; inputSize: number};
      expect(descriptor.artifactSha256).toBe(sha256(files.get(name + '/' + artifact + '.slither-brain')!));
      expect(descriptor.weightsSha256).toBe(variant[artifact === 'initial' ? 'initialWeightsSha256' : 'trainedWeightsSha256']);
      expect(descriptor.parameterCount).toBe(variant['parameters']); expect(descriptor.inputSize).toBe(264);
      const trials = native['trials'] as Array<{seed: number; observations: number; sharedBatchBudgetHeld: boolean}>;
      expect(trials).toHaveLength(14);
      expect(trials.every(trial => trial.seed === 3 && trial.observations > 0 && trial.sharedBatchBudgetHeld)).toBe(true);
      referencePhysics ??= native['referenceV1']; expect(native['referenceV1']).toEqual(referencePhysics);
    }
  }
  for (const [name, receipt] of Object.entries(summary['sources'] as Record<string, {sha256: string; gzipSha256: string}>)) {
    expect(name).toMatch(/^[a-zA-Z0-9_-]+\.py$/u);
    expect(sha256(files.get('sources/' + name)!)).toBe(receipt.sha256);
    const entry = evidence.files.find(file => file.path === 'sources/' + name)!;
    expect(entry.gzipSha256).toBe(receipt.gzipSha256);
  }
});


it.each(['windows', 'debian'])('binds %s learned P0/P1 clone workloads to retained CUDA policies and exact worker continuations', platform => {
  const root = resolve(import.meta.dirname, `../../docs/performance/sensor-v4/trained-cohorts-${platform}-v1`);
  const evidence = JSON.parse(readFileSync(resolve(root, 'evidence.json'), 'utf8')) as {
    nativeSourceSha256: string; candidatePatchSha256: string; files: TacticalStudyFile[];
  };
  expect(sha256(readFileSync(resolve(root, 'candidate.patch')))).toBe(evidence.candidatePatchSha256);
  const files = new Map<string, Buffer>();
  for (const entry of evidence.files) {
    expect(entry.path).toMatch(/^[a-zA-Z0-9_.-]+(?:\/[a-zA-Z0-9_.-]+)*$/u);
    expect(entry.path.split('/')).not.toContain('..'); expect(entry.archive).toBe(entry.path + '.gz');
    expect(entry.bytes).toBeGreaterThan(0); expect(entry.bytes).toBeLessThanOrEqual(4 * 1024 ** 2);
    const compressed = readFileSync(resolve(root, entry.archive)); expect(sha256(compressed)).toBe(entry.gzipSha256);
    const bytes = gunzipSync(compressed, {maxOutputLength: entry.bytes + 1});
    expect(bytes.byteLength).toBe(entry.bytes); expect(sha256(bytes)).toBe(entry.sha256);
    expect(files.has(entry.path)).toBe(false); files.set(entry.path, bytes);
  }
  /** Decode metadata while keeping portable graph/weight bytes opaque. */
  const json = (name: string): Record<string, unknown> => JSON.parse(files.get(name)!.toString('utf8')) as Record<string, unknown>;
  const summary = json('summary.json');
  expect(summary['measurementComplete']).toBe(true); expect(summary['releaseQualified']).toBe(false);
  expect(summary['nativeSourceSha256']).toBe(evidence.nativeSourceSha256);
  expect(summary['steps']).toBe(3600); expect(summary['warmup']).toBe(600); expect(summary['seed']).toBe(42);
  expect(summary['scriptSha256']).toBe(sha256(files.get('trained-sensor-cohorts.py')!));
  const study = resolve(root, '../tactical-ablation-windows-v1');
  expect(summary['studyEvidenceSha256']).toBe(sha256(readFileSync(resolve(study, 'evidence.json'))));
  const studyEvidence = JSON.parse(readFileSync(resolve(study, 'evidence.json'), 'utf8')) as {files: TacticalStudyFile[]};
  const records = summary['records'] as Array<{variant: string; scenario: string; workers: number; report: string;
    sha256: string; meanNativeMs: number; p95NativeMs: number; incompleteFraction: number; observations: number}>;
  expect(records).toHaveLength(8);
  if (platform === 'windows') {
    const live = json('live-cuda-w3.json');
    expect(live['gate']).toBe('live-canonical-distillation-v2'); expect(live['device']).toBe('cuda:0');
    expect(live['workers']).toBe(3);
    const source = live['source'] as Record<string, string>;
    expect(source['nativeSourceSha256']).toBe(evidence.nativeSourceSha256);
    expect(source['pyrlRevision']).toBe('1a500ab0ac214f19c02dc1bbd42870eba52fdf98');
    expect(source['hostSha256']).toBe(sha256(files.get('live-distillation.ts')!));
    expect(source['trainerGateSha256']).toBe(sha256(files.get('live_distillation_gate.py')!));
    const resume = live['resume'] as {restartExact: boolean; updates: number};
    expect(resume.restartExact).toBe(true); expect(resume.updates).toBe(250);
    const deployment = live['deployment'] as Record<string, unknown>;
    expect(deployment['restartExact']).toBe(true); expect(deployment['trainedUpdates']).toBe(250);
    for (const key of ['mismatchedGraphRejected', 'exportExact', 'retryReceiptExact', 'catalogDeleted', 'coldFilesReclaimed']) {
      expect(deployment[key]).toBe(true);
    }
    for (const shadow of live['shadows'] as Array<{enabled: boolean; error: unknown; totals: {samples: string}}>) {
      expect(shadow.enabled).toBe(true); expect(shadow.error).toBeNull(); expect(BigInt('0x' + shadow.totals.samples)).toBeGreaterThan(0n);
    }
    const authority = live['authority'] as Record<string, string>;
    expect(BigInt('0x' + authority['completedStepAfter']!)).toBeGreaterThan(BigInt('0x' + authority['completedStepBefore']!));
    expect(authority['schedulerDroppedWallMicros']).toBe('0000000000000000');
  }
  for (const variant of ['full', 'core-only']) {
    const artifact = files.get(variant + '.slither-brain')!;
    const original = studyEvidence.files.find(entry => entry.path === variant + '/trained.slither-brain')!;
    expect(sha256(artifact)).toBe(original.sha256);
    for (const [scenario, population] of [['P0', 55], ['P1', 300]] as const) {
      const pair = records.filter(record => record.variant === variant && record.scenario === scenario);
      expect(pair.map(record => record.workers)).toEqual([1, 8]);
      const reports = pair.map(record => {
        expect(record.report).toBe(`${variant}-${scenario}-w${record.workers}.json`);
        expect(sha256(files.get(record.report)!)).toBe(record.sha256);
        const report = json(record.report);
        expect(report['version']).toBe(6); expect(report['admitted']).toBe(true);
        expect(report['sourceSha256']).toBe(evidence.nativeSourceSha256);
        expect(report['population']).toBe(population); expect(report['workers']).toBe(record.workers);
        expect(report['completedSteps']).toBe(4200); expect(report['measuredSteps']).toBe(3600);
        expect(report['rayCap']).toBe(512); expect(report['rayBatch']).toBe(131072);
        expect(report['graphSize']).toBe('trained-artifact'); expect(report['sharedBatchBudgetHeld']).toBe(true);
        expect((report['trainedArtifact'] as {artifactSha256: string}).artifactSha256).toBe(original.sha256);
        expect(record.meanNativeMs).toBe((report['nativePhaseCosts'] as {totalMicros: number}).totalMicros / 3600 / 1000);
        expect(record.p95NativeMs).toBe((report['stepCost'] as {p95Ms: number}).p95Ms);
        expect(record.incompleteFraction).toBe(report['allConsumerRayIncompleteFraction']);
        expect(record.observations).toBe(report['observations']); expect(record.observations).toBeGreaterThan(0);
        const capture = report['capture'] as Record<string, unknown>;
        expect(capture['exactSamplesMatch']).toBe(true); expect(capture['queueAccountingMatches']).toBe(true);
        expect(capture['droppedSamples']).toBe(0); expect(capture['droppedChunks']).toBe(0);
        for (const hist of [...report['raw'] as Array<{nonfinite: number}>, ...report['normalized'] as Array<{nonfinite: number}>]) {
          expect(hist.nonfinite).toBe(0);
        }
        return report;
      });
      for (const field of ['target', 'rustc', 'mathBackend', 'sensorSha256', 'trainedArtifact', 'parameterCount',
        'continuationSha256', 'consumerWork', 'raw', 'normalized']) expect(reports[0]![field]).toEqual(reports[1]![field]);
      for (const field of ['sourceSampleSha256', 'drainedSampleSha256']) {
        expect((reports[0]!['capture'] as Record<string, unknown>)[field]).toEqual((reports[1]!['capture'] as Record<string, unknown>)[field]);
      }
    }
  }
});


it.each(['windows', 'debian'])('verifies %s head-distance reuse against the exact learned-policy workload', platform => {
  const root = resolve(import.meta.dirname, `../../docs/performance/sensor-v4/head-distance-reuse-${platform}-v1`);
  const evidence = JSON.parse(readFileSync(resolve(root, 'evidence.json'), 'utf8')) as {
    baselineSourceSha256: string; candidateSourceSha256: string; candidatePatchSha256: string; files: TacticalStudyFile[];
  };
  expect(sha256(readFileSync(resolve(root, 'candidate.patch')))).toBe(evidence.candidatePatchSha256);
  expect(evidence.candidateSourceSha256).not.toBe(evidence.baselineSourceSha256);
  const files = new Map<string, Buffer>();
  for (const entry of evidence.files) {
    expect(entry.path).toMatch(/^[a-zA-Z0-9_.-]+(?:\/[a-zA-Z0-9_.-]+)*$/u);
    expect(entry.path.split('/')).not.toContain('..'); expect(entry.archive).toBe(entry.path + '.gz');
    expect(entry.bytes).toBeGreaterThan(0); expect(entry.bytes).toBeLessThanOrEqual(4 * 1024 ** 2);
    const encoded = readFileSync(resolve(root, entry.archive)); expect(sha256(encoded)).toBe(entry.gzipSha256);
    const bytes = gunzipSync(encoded, {maxOutputLength: entry.bytes + 1});
    expect(bytes.byteLength).toBe(entry.bytes); expect(sha256(bytes)).toBe(entry.sha256);
    expect(files.has(entry.path)).toBe(false); files.set(entry.path, bytes);
  }
  /** Decode cold metadata while keeping model weights opaque. */
  const json = (name: string): Record<string, unknown> => JSON.parse(files.get(name)!.toString('utf8')) as Record<string, unknown>;
  const comparison = json('comparison.json');
  expect(comparison['equivalent']).toBe(true); expect(comparison['scriptSha256']).toBe(sha256(files.get('comparison.py')!));
  expect(comparison['trainedArtifactSha256']).toBe(sha256(files.get('full.slither-brain')!));
  const records = comparison['records'] as Array<{role: string; report: string; reportSha256: string;
    nativeMeanMs: number; sensingMeanMs: number; nativeP95Ms: number; processCpuSeconds: number}>;
  expect(records.map(value => value.role)).toEqual(['baseline', 'candidate', 'candidate', 'baseline']);
  const reports = records.map((record, index) => {
    expect(record.report).toBe(`${index}-${record.role}.json`);
    expect(sha256(files.get(record.report)!)).toBe(record.reportSha256);
    const report = json(record.report);
    expect(report['sourceSha256']).toBe(record.role === 'baseline' ? evidence.baselineSourceSha256 : evidence.candidateSourceSha256);
    expect(report['admitted']).toBe(true); expect(report['version']).toBe(6); expect(report['population']).toBe(300);
    expect(report['workers']).toBe(8); expect(report['completedSteps']).toBe(4200); expect(report['measuredSteps']).toBe(3600);
    expect(report['rayCap']).toBe(512); expect(report['rayBatch']).toBe(131072);
    const descriptor = report['trainedArtifact'] as {artifactSha256: string; parameterCount: number};
    expect(descriptor.artifactSha256).toBe(comparison['trainedArtifactSha256']); expect(descriptor.parameterCount).toBe(25042);
    const phases = report['nativePhaseCosts'] as {totalMicros: number; sensingMicros: number};
    expect(record.nativeMeanMs).toBe(phases.totalMicros / 3600 / 1000);
    expect(record.sensingMeanMs).toBe(phases.sensingMicros / 3600 / 1000);
    expect(record.nativeP95Ms).toBe((report['stepCost'] as {p95Ms: number}).p95Ms);
    expect(report['allConsumerRayIncompleteFraction']).toBe(0); expect(report['sharedBatchBudgetHeld']).toBe(true);
    const capture = report['capture'] as Record<string, unknown>;
    expect(capture['exactSamplesMatch']).toBe(true); expect(capture['queueAccountingMatches']).toBe(true);
    expect(capture['droppedSamples']).toBe(0);
    return report;
  });
  for (const report of reports.slice(1)) {
    for (const field of ['target', 'rustc', 'mathBackend', 'sensorSha256', 'trainedArtifact', 'parameterCount',
      'authoritativeMemoryReservationBytes', 'continuationSha256', 'consumerWork', 'raw', 'normalized']) {
      expect(report[field]).toEqual(reports[0]![field]);
    }
    for (const field of ['sourceSampleSha256', 'drainedSampleSha256']) {
      expect((report['capture'] as Record<string, unknown>)[field]).toEqual((reports[0]!['capture'] as Record<string, unknown>)[field]);
    }
  }
  for (const role of ['baseline', 'candidate']) {
    const pair = records.filter(record => record.role === role);
    const medians = comparison['medians'] as Record<string, Record<string, number>>;
    for (const field of ['nativeMeanMs', 'sensingMeanMs', 'nativeP95Ms'] as const) {
      expect(medians[role]![field]).toBe((pair[0]![field] + pair[1]![field]) / 2);
    }
  }
  if (platform === 'debian') {
    const profile = json('profile/profile.json');
    expect(profile['sourceSha256']).toBe(evidence.baselineSourceSha256); expect(profile['samplingHz']).toBe(200);
    const bytes = files.get('profile/cpu.pprof')!; expect(sha256(bytes)).toBe(profile['profileSha256']);
    expect([0, 1, 2, 3, 4].map(index => Number(bytes.readBigUInt64LE(index * 8)))).toEqual([0, 3, 0, 5000, 0]);
    let offset = 40; let samples = 0n; let stacks = 0;
    while (offset + 16 <= bytes.byteLength) {
      const count = bytes.readBigUInt64LE(offset); const depth = Number(bytes.readBigUInt64LE(offset + 8));
      expect(depth).toBeLessThanOrEqual(1024); offset += 16;
      expect(offset + depth * 8).toBeLessThanOrEqual(bytes.byteLength);
      if (count === 0n) { expect(depth).toBe(1); expect(bytes.readBigUInt64LE(offset)).toBe(0n); break; }
      offset += depth * 8; samples += count; stacks++;
    }
    expect(samples).toBe(30680n); expect(stacks).toBe(profile['stackRecords']); expect(Number(samples)).toBe(profile['samples']);
    for (const [name, receipt] of Object.entries(profile['symbolText'] as Record<string, {sha256: string}>)) {
      const text = files.get('profile/' + name)!; expect(sha256(text)).toBe(receipt.sha256);
      expect(text.toString('utf8')).not.toContain('/home/');
    }
    const profiled = json('profile/profile-report.json');
    for (const field of ['sourceSha256', 'target', 'rustc', 'mathBackend', 'trainedArtifact', 'continuationSha256', 'consumerWork', 'raw', 'normalized']) {
      expect(profiled[field]).toEqual(reports[0]![field]);
    }
    for (const field of ['sourceSampleSha256', 'drainedSampleSha256']) {
      expect((profiled['capture'] as Record<string, unknown>)[field]).toEqual((reports[0]!['capture'] as Record<string, unknown>)[field]);
    }
  }
});

/** Scalar-only receipts for the full checkpoint candidate, without decoding populations. */
interface CapacityEvidence {
  /** Native source identity and exact shape/memory/codec audit. */
  native: {
    sourceSha256: string;
    initialRestoreExact: boolean;
    generationRestoreExact: boolean;
    continuationExcludesWallDebt: boolean;
    peakRssBytes: number;
    combinedGenerationStateBytes: number;
    continuationSha256: string;
    generation: {logicalRootSha256: string; storedByteCount: string; weightsEncoding: string};
    runStart: {logicalRootSha256: string};
    audit: {
      population: number; parametersPerGenome: number; recurrentPerBrain: number;
      decodedRoleBytes: number[]; decodedBytes: number; processMemoryCeilingBytes: number;
      workerScratchBytes: number; checkpointScratchBytes: number; checkpointWorkspaceBytes: number;
      decodedEnvelopeFits: boolean; twoStateBudgetFits: boolean;
      limits: {maxArchiveBytes: number; maxTotalDecodedBytes: number; maxNumericCandidateBytes: number};
    };
  };
  /** Real opaque archive/import verification and native resident peak. */
  archive: {sourceSha256: string; importLimitsPreserved: boolean; idempotentImportExact: boolean;
    continuationSha256: string; peakRssBytes: number};
  /** Host-owned durable SQLite receipts and process peak. */
  sqliteCommitted: boolean;
  sqliteImported: boolean;
  peakHostRssBytes: number;
}

it.each(['windows', 'debian'])('checkpoint capacity %s retains full raw/compressed and durable import evidence', target => {
  const root = resolve(import.meta.dirname, '../../docs/performance/sensor-v4', `checkpoint-capacity-${target}-v1`);
  const manifest = JSON.parse(readFileSync(resolve(root, 'evidence.json'), 'utf8')) as {
    sourceSha256: string; candidatePatchSha256: string; reports: Record<string, string>;
  };
  expect(sha256(readFileSync(resolve(root, 'candidate.patch')))).toBe(manifest.candidatePatchSha256);
  const reports: CapacityEvidence[] = [];
  for (const mode of ['compressed', 'raw']) {
    const name = `${mode}.json`; const bytes = readFileSync(resolve(root, name));
    expect(sha256(bytes)).toBe(manifest.reports[name]);
    const receipt = JSON.parse(bytes.toString('utf8')) as CapacityEvidence;
    reports.push(receipt);
    const native = receipt.native; const audit = native.audit;
    expect(native.sourceSha256).toBe(manifest.sourceSha256);
    expect(receipt.archive.sourceSha256).toBe(manifest.sourceSha256);
    expect(audit.population).toBe(300); expect(audit.parametersPerGenome).toBe(494306);
    expect(audit.recurrentPerBrain).toBe(96);
    expect(audit.decodedRoleBytes[3]).toBe(300 * 494306 * 4);
    expect(audit.decodedBytes).toBe(audit.decodedRoleBytes.reduce((total, value) => total + value, 0));
    expect(audit.decodedBytes).toBeGreaterThan(512 * 1024 ** 2);
    expect(audit.limits.maxArchiveBytes).toBe(768 * 1024 ** 2);
    expect(audit.limits.maxTotalDecodedBytes).toBe(768 * 1024 ** 2);
    expect(audit.limits.maxNumericCandidateBytes).toBe((mode === 'raw' ? 1 : 512) * 1024 ** 2);
    expect(audit.workerScratchBytes).toBe(512 * 1024 ** 2);
    expect(audit.checkpointScratchBytes).toBe(512 * 1024 ** 2);
    expect(audit.checkpointWorkspaceBytes).toBeLessThan(audit.checkpointScratchBytes);
    expect(audit.decodedEnvelopeFits).toBe(true); expect(audit.twoStateBudgetFits).toBe(true);
    expect(audit.processMemoryCeilingBytes).toBe(4 * 1024 ** 3);
    expect(native.combinedGenerationStateBytes).toBeLessThanOrEqual(audit.processMemoryCeilingBytes);
    expect(native.peakRssBytes).toBeGreaterThan(0); expect(native.peakRssBytes).toBeLessThan(audit.processMemoryCeilingBytes);
    expect(receipt.archive.peakRssBytes).toBeGreaterThan(0);
    expect(receipt.archive.peakRssBytes).toBeLessThan(audit.processMemoryCeilingBytes);
    expect(native.initialRestoreExact && native.generationRestoreExact).toBe(true);
    expect(native.continuationExcludesWallDebt).toBe(true);
    expect(receipt.sqliteCommitted && receipt.sqliteImported).toBe(true);
    expect(receipt.archive.importLimitsPreserved && receipt.archive.idempotentImportExact).toBe(true);
    expect(receipt.archive.continuationSha256).toBe(native.continuationSha256);
    expect(receipt.peakHostRssBytes).toBeGreaterThan(0);
    expect(native.generation.weightsEncoding).toBe(mode === 'raw' ? 'raw-f32le-v1' : 'f32le-shuffle4-zstd-v1');
    if (mode === 'raw') expect(BigInt(`0x${native.generation.storedByteCount}`)).toBeGreaterThan(512n * 1024n ** 2n);
  }
  expect(reports[0]!.native.runStart.logicalRootSha256).toBe(reports[1]!.native.runStart.logicalRootSha256);
  expect(reports[0]!.native.generation.logicalRootSha256).toBe(reports[1]!.native.generation.logicalRootSha256);
  expect(reports[0]!.native.continuationSha256).toBe(reports[1]!.native.continuationSha256);
});

/** Actual delivered inspector snapshot transported through the native mailbox and wire encoder. */
interface InspectorCapacityReceipt {
  /** Completed simulation step at publication. */
  completedStep: number;
  /** Full Motion/B32 width. */
  inputSize: number;
  /** Exact delivered observation step. */
  sampleStep: number;
  /** Complete encoded inspector message size. */
  wireBytes: number;
  /** Capacity cases use delivered observations. */
  pureProbe: boolean;
  /** Trace storage survives source/import overlap. */
  tracePresent: boolean;
  /** Repeated reads do not generate new publications. */
  duplicatePublicationSuppressed: boolean;
}

/** Check that diagnostics describe the exact delivered step without changing its boundary. */
function verifyCapacityInspector(receipt: InspectorCapacityReceipt, step: number): void {
  expect(receipt.completedStep).toBe(step); expect(receipt.sampleStep).toBe(step);
  expect(receipt.inputSize).toBe(504); expect(receipt.wireBytes).toBeGreaterThan(0);
  expect(receipt.pureProbe).toBe(false); expect(receipt.tracePresent).toBe(true);
  expect(receipt.duplicatePublicationSuppressed).toBe(true);
}

/** Loaded live replacement and three independently restored successor receipts. */
interface HandoffCapacityEvidence extends CapacityEvidence {
  /** Revised host report binds all three durable metadata/current-pointer commits. */
  schemaVersion: number;
  sqlitePostImportGenerations: number;
  finalCheckpointId: string;
  hostRssMeasurement: string;
  /** The original imported boundary retains its separate continuation witness. */
  archive: CapacityEvidence['archive'] & {
    schemaVersion: number;
    retiredSourceStateBytes: number;
    overlapCombinedStateBytes: number;
    stagingMemoryCeilingBytes: number;
    runningMemoryCeilingBytes: number;
    /** New receipts exercise reservation before decoding rather than after candidate construction. */
    nativeAdmissionBeforeAllocation?: boolean;
    postImportGenerations: Array<{
      descriptor: {generation: string; completedStep: string; logicalRootSha256: string; weightsEncoding: string};
      combinedStateBytes: number;
      continuationSha256: string;
      independentRestoreExact: boolean;
    }>;
  };
}

/** Full 100-record original/archive/catalog and selected-inspector coexistence receipts. */
interface LibraryInspectorCapacityEvidence extends HandoffCapacityEvidence {
  /** Explicit offline mode; ordinary production envelopes are unchanged. */
  coexistence: string;
  /** Source/imported membership, quota rejection and final reference-aware reclamation. */
  library: {
    recordCount: number; overflowRejected: boolean; overflowPublishedFiles: boolean;
    importedPages: number; deletedRecords: number; deletedLibraryWeights: number;
    preservedSharedWeights: number; finalCatalogEmpty: boolean; populationCheckpointPreserved: boolean;
    brainIds: string[]; originalBytes: number;
  };
  /** Native selected-original verification and actual mailbox snapshots. */
  archive: Omit<HandoffCapacityEvidence['archive'], 'postImportGenerations'> & {
    coexistence: {savedBrainCount: number; selectedOriginalBytes: number;
      sourceInspector: InspectorCapacityReceipt; replacementInspector: InspectorCapacityReceipt};
    postImportGenerations: Array<HandoffCapacityEvidence['archive']['postImportGenerations'][number] & {inspector: InspectorCapacityReceipt}>;
  };
}

it.each(['windows', 'debian'].flatMap(target =>
  ['checkpoint-handoffs', 'population-admission', 'library-inspector'].map(kind => ({target, kind}))))(
  '$kind $target reclaim only retired budget and retain three exact successors', ({target, kind}) => {
  const root = resolve(import.meta.dirname, '../../docs/performance/sensor-v4', `${kind}-${target}-v1`);
  const manifest = JSON.parse(readFileSync(resolve(root, 'evidence.json'), 'utf8')) as {
    sourceSha256: string; candidatePatchSha256: string; reports: Record<string, string>;
  };
  expect(sha256(readFileSync(resolve(root, 'candidate.patch')))).toBe(manifest.candidatePatchSha256);
  const reports: HandoffCapacityEvidence[] = [];
  for (const mode of ['compressed', 'raw']) {
    const filename = `${mode}.json`; const bytes = readFileSync(resolve(root, filename));
    expect(sha256(bytes)).toBe(manifest.reports[filename]);
    const report = JSON.parse(bytes.toString('utf8')) as HandoffCapacityEvidence;
    reports.push(report);
    const archive = report.archive; const full = report.native.audit.processMemoryCeilingBytes;
    expect(report.schemaVersion).toBe(2); expect(archive.schemaVersion).toBe(2);
    expect(report.native.sourceSha256).toBe(manifest.sourceSha256); expect(archive.sourceSha256).toBe(manifest.sourceSha256);
    expect(report.native.audit.population).toBe(300); expect(report.native.audit.parametersPerGenome).toBe(494306);
    expect(full).toBe(4 * 1024 ** 3); expect(archive.runningMemoryCeilingBytes).toBe(full);
    expect(archive.retiredSourceStateBytes + archive.stagingMemoryCeilingBytes).toBe(full);
    expect(archive.overlapCombinedStateBytes).toBeLessThanOrEqual(full);
    expect(archive.stagingMemoryCeilingBytes).toBeLessThan(full);
    expect(archive.idempotentImportExact && archive.importLimitsPreserved).toBe(true);
    if (kind !== 'checkpoint-handoffs') expect(archive.nativeAdmissionBeforeAllocation).toBe(true);
    expect(archive.continuationSha256).toBe(report.native.continuationSha256);
    expect(archive.postImportGenerations).toHaveLength(3);
    const roots = new Set<string>();
    for (const [ordinal, generation] of archive.postImportGenerations.entries()) {
      expect(BigInt(`0x${generation.descriptor.generation}`)).toBe(BigInt(3 + ordinal));
      expect(BigInt(`0x${generation.descriptor.completedStep}`)).toBe(BigInt(4 + ordinal * 2));
      expect(generation.combinedStateBytes).toBeGreaterThan(archive.stagingMemoryCeilingBytes);
      expect(generation.combinedStateBytes).toBeLessThanOrEqual(full);
      expect(generation.independentRestoreExact).toBe(true);
      expect(generation.descriptor.weightsEncoding).toBe(mode === 'raw' ? 'raw-f32le-v1' : 'f32le-shuffle4-zstd-v1');
      roots.add(generation.descriptor.logicalRootSha256);
    }
    expect(roots.size).toBe(3);
    expect(report.sqliteCommitted && report.sqliteImported).toBe(true); expect(report.sqlitePostImportGenerations).toBe(3);
    expect(report.finalCheckpointId).toBe(archive.postImportGenerations.at(-1)!.descriptor.logicalRootSha256);
    expect(report.hostRssMeasurement).toBe('os-high-water-and-100ms-samples');
    expect(report.peakHostRssBytes).toBeGreaterThan(0); expect(archive.peakRssBytes).toBeGreaterThan(0);
    expect(archive.peakRssBytes).toBeLessThan(full);
    if (kind === 'library-inspector') {
      const loaded = report as LibraryInspectorCapacityEvidence;
      expect(loaded.coexistence).toBe('library-inspector');
      expect(loaded.library).toMatchObject({recordCount: 100, overflowRejected: true,
        overflowPublishedFiles: false, importedPages: 4, deletedRecords: 100,
        finalCatalogEmpty: true, populationCheckpointPreserved: true});
      expect(loaded.library.deletedLibraryWeights + loaded.library.preservedSharedWeights).toBe(100);
      expect(new Set(loaded.library.brainIds).size).toBe(100);
      expect(loaded.library.originalBytes).toBeGreaterThan(100 * 494306 * 4);
      expect(loaded.archive.coexistence.savedBrainCount).toBe(100);
      expect(loaded.archive.coexistence.selectedOriginalBytes).toBe(loaded.library.originalBytes);
      verifyCapacityInspector(loaded.archive.coexistence.sourceInspector, 3);
      verifyCapacityInspector(loaded.archive.coexistence.replacementInspector, 3);
      loaded.archive.postImportGenerations.forEach((generation, ordinal) => verifyCapacityInspector(generation.inspector, 5 + ordinal * 2));
    }
  }
  if (kind === 'library-inspector') expect((reports[0] as LibraryInspectorCapacityEvidence).library.brainIds)
    .toEqual((reports[1] as LibraryInspectorCapacityEvidence).library.brainIds);
  for (let ordinal = 0; ordinal < 3; ordinal++) {
    const compressed = reports[0]!.archive.postImportGenerations[ordinal]!;
    const raw = reports[1]!.archive.postImportGenerations[ordinal]!;
    expect(compressed.descriptor.logicalRootSha256).toBe(raw.descriptor.logicalRootSha256);
    expect(compressed.continuationSha256).toBe(raw.continuationSha256);
  }
});


/** Measured concurrent trainer/authority reports bind their immutable host and trainer sources. */
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

it('retains real trainer/four-shadow progress and honest measured performance failures', () => {
  const root = resolve(import.meta.dirname, '../../docs/performance/sensor-v4/trainer-contention-v1');
  const manifest = JSON.parse(readFileSync(resolve(root, 'evidence.json'), 'utf8')) as {
    nativeSourceSha256: string; studentArtifactSha256: string;
    sources: Record<string, {sha256: string; gzipSha256: string}>;
    reports: Record<string, {sha256: string; gzipSha256: string; device: string; target: string}>;
  };
  for (const [filename, receipt] of Object.entries(manifest.sources)) {
    expect(filename).toMatch(/^(host\.ts|trainer\.py)\.gz$/u);
    const encoded = readFileSync(resolve(root, filename)); expect(sha256(encoded)).toBe(receipt.gzipSha256);
    expect(sha256(gunzipSync(encoded, {maxOutputLength: 128 * 1024}))).toBe(receipt.sha256);
  }
  expect(Object.keys(manifest.reports).sort()).toEqual([
    'debian-cpu-P1.json.gz', 'windows-cpu-P1.json.gz', 'windows-cuda-P1.json.gz']);
  for (const [filename, receipt] of Object.entries(manifest.reports)) {
    const encoded = readFileSync(resolve(root, filename)); expect(sha256(encoded)).toBe(receipt.gzipSha256);
    const decoded = gunzipSync(encoded, {maxOutputLength: 1024 * 1024}); expect(sha256(decoded)).toBe(receipt.sha256);
    const report = JSON.parse(decoded.toString('utf8')) as TrainerContentionEvidence;
    expect(report.gate).toBe('live-trainer-four-shadow-contention-v1');
    expect(report.device).toBe(receipt.device); expect(report.workers).toBe(3); expect(report.seconds).toBe(60);
    expect(report.source.target).toBe(receipt.target);
    expect(report.source.hostSha256).toBe(manifest.sources['host.ts.gz']!.sha256);
    expect(report.source.trainerGateSha256).toBe(manifest.sources['trainer.py.gz']!.sha256);
    expect(report.source.nativeSourceSha256).toBe(manifest.nativeSourceSha256);
    expect(report.source.studentArtifactSha256).toBe(manifest.studentArtifactSha256);
    expect(report.workload.scenario).toBe('P1'); expect(report.workload.metadata.parameterCount).toBe(25042);
    expect(report.workload.request).toMatchObject({profile: 'motion', bins: 16, rayCap: 512, rayBatch: 131072});
    expect(report.train.updates).toBe(126); expect(report.resume).toMatchObject({updates: 250, restartExact: true});
    expect(report.sustain.initialUpdates).toBe(250);
    expect(report.sustain.updates - 250).toBe(report.sustain.optimizerUpdates);
    expect(report.sustain.optimizerUpdates).toBeGreaterThan(0); expect(report.sustain.fetches).toBeGreaterThan(0);
    expect(report.sustain.finalCursor).toBeGreaterThan(report.sustain.initialCursor);
    expect(report.sustain.requestedSeconds).toBe(60); expect(report.sustain.activeWallSeconds).toBeGreaterThanOrEqual(60);
    expect(report.sustain.sequence).toMatchObject({batch_size: 32, burn_in_steps: 32, bptt_steps: 64});
    expect(report.sustain.peakReplayBytes).toBeLessThanOrEqual(256 * 1024 ** 2);
    expect(report.sustain.peakRssBytes).toBeGreaterThan(0);
    expect(report.sustain.peakDeviceBytes).toBeLessThanOrEqual(8 * 1024 ** 3);
    expect(report.summary).toEqual(summarizeSensorWall(report.reads, report.seconds));
    expect(report.shadows.after).toHaveLength(4); expect(report.shadows.before).toHaveLength(4);
    for (const [slot, after] of report.shadows.after.entries()) {
      const before = report.shadows.before[slot]!;
      expect(after.artifactSha256).toBe(before.artifactSha256);
      expect(after.activationSequence).toBe(before.activationSequence);
      const delta = BigInt('0x' + after.totals.samples) - BigInt('0x' + before.totals.samples);
      expect(delta).toBeGreaterThan(0n); expect(delta.toString()).toBe(report.shadows.sampleProgress[slot]);
    }
    expect(report.capture.noDrops).toBe(true); expect(report.capture.after.state).toBe('stopped');
    expect(report.capture.after.resolvedThrough).toBe(report.capture.after.productionWatermark);
    expect(report.capture.after.error).toBe(null); expect(report.http.forbiddenRequests).toBe(0);
    expect(report.frames.copies).toBeGreaterThan(0); expect(report.frames.bytes).toBeGreaterThan(0);
  }
});


it.each(['windows', 'debian'])('retains %s uninterrupted six-hour native pressure and bounded lifecycle storage', platform => {
  const root = resolve(import.meta.dirname, `../../docs/performance/sensor-v4/continuous-soak-${platform}-v1`);
  const manifest = JSON.parse(readFileSync(resolve(root, 'evidence.json'), 'utf8')) as {
    status: string; sourceSha256: string; target: string; report: string; originalReportBytes: number;
    originalReportSha256: string; gzipSha256: string; shadowArtifactSha256: string;
  };
  expect(manifest.status).toBe('measured'); expect(manifest.report).toBe('P0.json.gz');
  const encoded = readFileSync(resolve(root, manifest.report)); expect(sha256(encoded)).toBe(manifest.gzipSha256);
  const bytes = gunzipSync(encoded, {maxOutputLength: 1024 * 1024});
  expect(bytes.length).toBe(manifest.originalReportBytes); expect(sha256(bytes)).toBe(manifest.originalReportSha256);
  const report = JSON.parse(bytes.toString('utf8')) as NativeSoakReceipt & {
    target: string; buildProfile: string; buildClass: string; workers: number; seed: number;
    metadata: {parameterCount: number; sensorVersion: number; sensorContract: {canonicalJson: string; sha256: string}};
    shadowArtifact: {artifactSha256: string; parameterCount: number};
  };
  expect(report.sourceSha256).toBe(manifest.sourceSha256); expect(report.target).toBe(manifest.target);
  expect(report.buildProfile).toBe('release'); expect(report.buildClass).toBe('test-hooks');
  expect(report.workers).toBe(3); expect(report.seed).toBe(42); expect(report.warmupSteps).toBe(0);
  expect(report.metadata.parameterCount).toBe(25042); expect(report.metadata.sensorVersion).toBe(4);
  const contract = report.metadata.sensorContract;
  expect(sha256(Buffer.concat([Buffer.from('slither.sensor-contract.v1\0'), Buffer.from(contract.canonicalJson)]))).toBe(contract.sha256);
  expect(JSON.parse(contract.canonicalJson) as unknown).toMatchObject({profile: 'motion', bins: 16, inputSize: 264});
  expect(report.shadowArtifact.artifactSha256).toBe(manifest.shadowArtifactSha256);
  expect(report.shadowArtifact.parameterCount).toBe(25042);
  const summary = summarizeNativeSoak(report);
  expect(summary.measuredSimulatedHours).toBe(6); expect(summary.sixHourPressureSoak).toBe(true);
  expect(summary.observed).toBeGreaterThan(12_000_000); expect(summary.drained).toBeGreaterThan(1_000_000);
  expect(report.snapshots).toHaveLength(361);
});

it('retains exact P2 fast-respawn authority across worker counts and slow capture losses', () => {
  const root = resolve(import.meta.dirname, '../../docs/performance/sensor-v4/fast-respawn-pressure-debian-v1');
  const manifest = JSON.parse(readFileSync(resolve(root, 'evidence.json'), 'utf8')) as {
    nativeSourceSha256: string; files: Record<string, {bytes: number; sha256: string; gzipSha256: string}>;
  };
  const files = new Map<string, Buffer>();
  for (const [name, receipt] of Object.entries(manifest.files)) {
    expect(name).toMatch(/^(fast-w[13]\.json|slow-w3\.json|summary\.json|run-pressure\.py)$/u);
    const encoded = readFileSync(resolve(root, name + '.gz')); expect(sha256(encoded)).toBe(receipt.gzipSha256);
    const bytes = gunzipSync(encoded, {maxOutputLength: 4 * 1024 * 1024});
    expect(bytes.length).toBe(receipt.bytes); expect(sha256(bytes)).toBe(receipt.sha256); files.set(name, bytes);
  }
  const summary = JSON.parse(files.get('summary.json')!.toString('utf8')) as {
    measurementComplete: boolean; releaseQualified: boolean; workerAndPressureContinuationExact: boolean;
    nativeSourceSha256: string; scriptSha256: string; reports: Array<{case: string; report: string; sha256: string}>;
  };
  expect(summary).toMatchObject({measurementComplete: true, releaseQualified: false, workerAndPressureContinuationExact: true});
  expect(summary.nativeSourceSha256).toBe(manifest.nativeSourceSha256);
  expect(summary.scriptSha256).toBe(sha256(files.get('run-pressure.py')!));
  const reports: Record<string, unknown>[] = [];
  for (const [index, receipt] of summary.reports.entries()) {
    expect(receipt.case).toBe(['fast-w1', 'fast-w3', 'slow-w3'][index]);
    const bytes = files.get(receipt.report)!; expect(sha256(bytes)).toBe(receipt.sha256);
    const report = JSON.parse(bytes.toString('utf8')) as Record<string, unknown>;
    expect(report).toMatchObject({sourceSha256: manifest.nativeSourceSha256, target: 'x86_64-unknown-linux-gnu',
      admitted: true, population: 55, graphSize: 'large', parameterCount: 494306, warmupSteps: 600, measuredSteps: 3600,
      completedSteps: 4200, baselineCadenceSeconds: 1 / 60, teacherRespawnDelaySeconds: 0.5,
      rayMinimum: 1, rayCap: 512, rayBatch: 131072, sharedBatchBudgetHeld: true});
    expect(report['workers']).toBe(index === 0 ? 1 : 3);
    const capture = report['capture'] as {samplesObserved: number[]; samplesDrained: number[]; droppedSamples: number;
      droppedChunks: number; chunksDrained: number; productionWatermark: number; releasedThrough: number;
      countLimitedDroppedChunks: number; byteLimitedDroppedChunks: number; exactSamplesMatch: boolean | null; queueAccountingMatches: boolean};
    expect(capture.queueAccountingMatches).toBe(true);
    expect(capture.samplesObserved.reduce((a, b) => a + b, 0)).toBe(35640);
    expect(capture.samplesObserved.reduce((a, b) => a + b, 0)).toBe(capture.samplesDrained.reduce((a, b) => a + b, 0) + capture.droppedSamples);
    expect(capture.productionWatermark).toBe(capture.chunksDrained + capture.droppedChunks);
    expect(capture.releasedThrough).toBe(capture.productionWatermark);
    expect(capture.countLimitedDroppedChunks + capture.byteLimitedDroppedChunks).toBe(capture.droppedChunks);
    if (index < 2) {expect(capture.exactSamplesMatch).toBe(true); expect(capture.droppedChunks).toBe(0);}
    else {expect(capture.exactSamplesMatch).toBeNull(); expect(capture.droppedChunks).toBeGreaterThan(0);}
    if (reports.length) {
      for (const field of ['continuationSha256', 'consumerWork', 'raw', 'normalized']) expect(report[field]).toEqual(reports[0]![field]);
      for (const field of ['sourceSampleSha256', 'samplesObserved', 'dueTeacherHistogram', 'productionWatermark']) {
        expect((report['capture'] as Record<string, unknown>)[field]).toEqual((reports[0]!['capture'] as Record<string, unknown>)[field]);
      }
    }
    if (index === 1) expect((report['capture'] as Record<string, unknown>)['drainedSampleSha256'])
      .toEqual((reports[0]!['capture'] as Record<string, unknown>)['drainedSampleSha256']);
    reports.push(report);
  }
  expect(reports).toHaveLength(3);
});
