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
