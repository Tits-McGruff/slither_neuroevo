/** Bounded opaque fixtures and metadata checks for full-catalog live qualification. */
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { open, realpath } from 'node:fs/promises';
import { basename, dirname, join } from 'node:path';
import type { SavedBrainPageV1, SavedBrainSummaryV1 } from '../../server/protocol.ts';

/** One externally generated manual portable original; only Rust interprets its payload. */
export interface LiveLibraryFixtureRecord {
  /** Exact complete original identity. */
  brainId: string;
  /** Exact canonical numeric identity checked again by Rust. */
  weightsSha256: string;
  /** Controlled input below the fixture root. */
  relativeFilename: string;
  /** Explicit synthetic/manual display label. */
  name: string;
  /** Exact original length as canonical u64 hex. */
  artifactByteCount: string;
  /** Full large Motion/B32 canonical parameter count. */
  parameterCount: number;
}
/** A complete 100-record catalog plus a distinct valid overflow probe. */
export interface LiveLibraryFixture {
  /** Canonical fixture root, independent of the disposable live database. */
  directory: string;
  /** Exact admitted index bytes. */
  indexSha256: string;
  /** Dense ordinal input ownership. */
  records: readonly LiveLibraryFixtureRecord[];
}
/** Scalar facts sufficient to compare catalog membership across cold restart. */
export interface LiveCatalogReceipt {
  /** Exactly four maximum-sized metadata pages. */
  pages: number;
  /** Full distinct original count. */
  recordCount: number;
  /** Sum of encoded portable original bytes, in decimal. */
  originalBytes: string;
  /** Stable identity/metadata digest, excluding full graph and model buffers. */
  metadataSha256: string;
  /** Exact sorted portable identities. */
  brainIds: string[];
}

/** Read one ordinary file without allocating beyond a fixed admitted byte ceiling. */
async function boundedBytes(path: string, maximum: number): Promise<Buffer> {
  const file = await open(path, 'r');
  try {
    const metadata = await file.stat();
    assert(metadata.isFile() && metadata.size > 0 && metadata.size <= maximum, 'bounded ordinary fixture file');
    const buffer = Buffer.alloc(metadata.size + 1);
    let count = 0;
    while (count < buffer.length) {
      const {bytesRead} = await file.read(buffer, count, buffer.length - count, count);
      if (bytesRead === 0) break;
      count += bytesRead;
    }
    assert.equal(count, metadata.size, 'fixture length changed');
    return buffer.subarray(0, count);
  } finally {await file.close();}
}

/** Admit an exact dense index before any upload or database construction. */
export async function loadLiveLibraryFixture(index: string): Promise<LiveLibraryFixture> {
  const path = await realpath(index);
  assert.equal(basename(path), 'index.json'); assert.equal(basename(dirname(path)), 'library-inputs');
  const bytes = await boundedBytes(path, 128 * 1024);
  const value = JSON.parse(bytes.toString('utf8')) as {schemaVersion: unknown; records: unknown};
  assert(value && Object.keys(value).sort().join(',') === 'records,schemaVersion' && value.schemaVersion === 1);
  assert(Array.isArray(value.records) && value.records.length === 101, '100 originals plus overflow required');
  const records = value.records as LiveLibraryFixtureRecord[];
  for (const [ordinal, record] of records.entries()) {
    assert(record && Object.keys(record).sort().join(',') === 'artifactByteCount,brainId,name,parameterCount,relativeFilename,weightsSha256');
    assert.match(record.brainId, /^[0-9a-f]{64}$/u); assert.match(record.weightsSha256, /^[0-9a-f]{64}$/u);
    assert.match(record.artifactByteCount, /^[0-9a-f]{16}$/u);
    const length = BigInt('0x' + record.artifactByteCount);
    assert(length >= 88n && length <= 8n * 1024n ** 2n, 'bounded large manual original');
    assert.equal(record.parameterCount, 494306, 'full large graph required');
    assert.equal(record.relativeFilename, 'library-inputs/' + String(ordinal).padStart(3, '0') + '.slither-brain');
    assert(typeof record.name === 'string' && record.name.length > 0 && record.name.length <= 80);
  }
  assert.equal(new Set(records.map(record => record.brainId)).size, 101, 'distinct originals required');
  assert.equal(new Set(records.map(record => record.weightsSha256)).size, 101, 'distinct weights required');
  return {directory: dirname(dirname(path)), indexSha256: createHash('sha256').update(bytes).digest('hex'), records};
}

/** Confine and hash one opaque original immediately before its real HTTP/native admission. */
export async function readLiveLibraryArtifact(fixture: LiveLibraryFixture, ordinal: number): Promise<Buffer> {
  assert(Number.isSafeInteger(ordinal) && ordinal >= 0 && ordinal < 101);
  const record = fixture.records[ordinal]!;
  const path = await realpath(join(fixture.directory, record.relativeFilename));
  assert.equal(dirname(path), join(fixture.directory, 'library-inputs'), 'fixture path escapes controlled input directory');
  const bytes = await boundedBytes(path, 8 * 1024 ** 2);
  assert.equal(BigInt(bytes.byteLength), BigInt('0x' + record.artifactByteCount));
  assert.equal(createHash('sha256').update(bytes).digest('hex'), record.brainId, 'fixture original changed');
  return bytes;
}

/** Verify actual bounded catalog pages and retain no graph/parameter payload in the receipt. */
export async function readLiveCapacityCatalog(page: (after: string | null) => Promise<SavedBrainPageV1>): Promise<LiveCatalogReceipt> {
  const records: SavedBrainSummaryV1[] = [];
  let after: string | null = null; let pages = 0;
  do {
    const result = await page(after); pages++;
    assert(pages <= 4 && result.entries.length === 25, 'four full bounded catalog pages required');
    for (const record of result.entries) {
      assert.match(record.brainId, /^[0-9a-f]{64}$/u);
      assert.equal(record.artifact.parameterCount, 494306); assert.equal(record.artifact.inputSize, 504);
      assert.equal(record.artifact.sourceKind, 'manual');
      assert(record.brainId > (records.at(-1)?.brainId ?? ''), 'catalog order/identity changed');
      records.push(record);
    }
    const expected = records.length < 100 ? records.at(-1)!.brainId : null;
    assert.equal(result.next, expected, 'catalog cursor must match complete membership');
    after = result.next;
  } while (after !== null);
  assert.equal(records.length, 100);
  const compact = records.map(record => ({brainId: record.brainId, name: record.name, createdAtMs: record.createdAtMs,
    artifactByteCount: record.artifactByteCount, artifactRelativeFilename: record.artifactRelativeFilename,
    weights: record.weights, artifact: record.artifact}));
  return {pages, recordCount: records.length,
    originalBytes: records.reduce((sum, record) => sum + BigInt('0x' + record.artifactByteCount), 0n).toString(),
    metadataSha256: createHash('sha256').update(JSON.stringify(compact)).digest('hex'),
    brainIds: records.map(record => record.brainId)};
}
