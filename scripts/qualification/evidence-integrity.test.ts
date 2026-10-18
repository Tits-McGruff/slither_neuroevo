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
  ['pellet-counting-windows-v1', 'P1-w5-comparison.json']
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

it('binds loaded replay RAM to genuine Rust codec bytes and exact independent restore', () => {
  const root = resolve(import.meta.dirname, '../../docs/performance/distillation/replay-capacity-windows-v1');
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
    expect(report.environment.platform).toMatch(/^Windows-/u);
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
