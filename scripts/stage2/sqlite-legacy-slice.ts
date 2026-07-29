/**
 * Disposable SQLite legacy-column slice experiment for Stage 2.
 *
 * This probe establishes what the JavaScript-facing `better-sqlite3` API
 * returns when a large legacy BLOB or TEXT column is read through SQLite's
 * `length` plus `substr(CAST(... AS BLOB))` functions. It is deliberately not
 * a production reader, checkpoint schema, conversion path, or proof that
 * SQLite's native implementation never needs a larger temporary allocation.
 */

import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { performance } from 'node:perf_hooks';
import Database from 'better-sqlite3';

/** P0-like compressed payload size used by the short smoke experiment. */
export const DEFAULT_PAYLOAD_BYTES = 2_527_124;
/** Maximum per-column fixture size accepted by this disposable runner. */
export const MAX_PAYLOAD_BYTES = 64 * 1024 * 1024;
/** Default JavaScript-visible slice upper bound. */
export const DEFAULT_CHUNK_BYTES = 64 * 1024;
/** Default bounded wait for the isolated reader child. */
export const DEFAULT_CHILD_TIMEOUT_MS = 60_000;
/** Fixed table key for the representative legacy BLOB value. */
export const BLOB_KIND = 'blob';
/** Fixed table key for the representative legacy TEXT value. */
export const TEXT_KIND = 'text';

/**
 * Query used by the chunk reader for a BLOB legacy column. It must remain a
 * bounded SQLite expression; a full `SELECT payload_blob` is forbidden here.
 */
export const BLOB_SLICE_SQL = `
  SELECT substr(CAST(payload_blob AS BLOB), ?, ?) AS chunk
    FROM legacy_values
   WHERE kind = ?
`;

/**
 * Query used by the chunk reader for a TEXT legacy column. Casting to BLOB
 * makes offsets and lengths unambiguously UTF-8 byte-oriented.
 */
export const TEXT_SLICE_SQL = `
  SELECT substr(CAST(payload_text AS BLOB), ?, ?) AS chunk
    FROM legacy_values
   WHERE kind = ?
`;

/** One logical legacy fixture role. */
export type LegacySliceRole = typeof BLOB_KIND | typeof TEXT_KIND;

/** Result of one bounded JavaScript-facing SQLite slice read. */
export interface SliceReadResult {
  /** Logical fixture role. */
  role: LegacySliceRole;
  /** Length reported by SQLite's byte-oriented expression. */
  logicalBytes: number;
  /** Exact number of returned chunks. */
  chunks: number;
  /** Largest Buffer returned to JavaScript by the slice query. */
  largestReturnedChunkBytes: number;
  /** SHA-256 over the reconstructed logical byte sequence. */
  sha256: string;
  /** Whether every returned chunk stayed within the requested cap. */
  allChunksBounded: boolean;
  /** Wall duration for this synchronous database read. */
  durationMs: number;
}

/** Result of creating the two-column disposable fixture. */
export interface LegacySliceFixture {
  /** Disposable database path. */
  databasePath: string;
  /** One logical byte count per column. */
  payloadBytes: number;
  /** BLOB logical SHA-256. */
  blobSha256: string;
  /** TEXT UTF-8 byte SHA-256. */
  textSha256: string;
  /** Main database size after creation. */
  databaseBytes: number;
  /** SQLite database text encoding recorded for this fixture. */
  sqliteEncoding: string;
}

/** Reader child result returned to the parent as JSON. */
interface ReaderChildResult {
  /** Child result schema. */
  schema: 'slither-stage2-sqlite-legacy-slice-child';
  /** Schema revision. */
  version: 1;
  /** Explicit constraint on what this measurement proves. */
  caveat: string;
  /** Database path read by this child only. */
  databasePath: string;
  /** Requested JavaScript-visible chunk cap. */
  chunkBytes: number;
  /** Read results by role. */
  reads: Record<LegacySliceRole, SliceReadResult>;
  /** Process memory before/after plus whole-role boundary samples. */
  memory: MemorySamples;
  /** Checks that make accidental full-column selection visible. */
  assertions: {
    chunkQueriesUseBoundedSubstrCast: boolean;
    chunkPathContainsNoFullColumnSelect: boolean;
    exactBlob: boolean;
    exactText: boolean;
    boundedBlob: boolean;
    boundedText: boolean;
    sqliteEncodingIsUtf8: boolean;
  };
  /** Deliberately conservative interpretation of this probe. */
  conclusion: {
    /** This probe does not observe or prove native transient allocations. */
    boundedNativeAllocationProved: false;
    /** This probe alone never approves a production reader implementation. */
    productionQueryPathAuthorized: false;
    /** Required future design choice before production legacy reads. */
    requiredProductionDecision: string;
  };
}

/** Process memory snapshots sampled only at whole-role boundaries. */
interface MemorySamples {
  /** Memory before the first read. */
  before: NodeJS.MemoryUsage;
  /** Memory after the final read. */
  after: NodeJS.MemoryUsage;
  /** Largest RSS sampled before/after a complete role read, not a transient peak. */
  sampledBoundaryMaximumRssBytes: number;
  /** Largest heap use sampled before/after a complete role read, not a transient peak. */
  sampledBoundaryMaximumHeapUsedBytes: number;
  /** Largest external total sampled before/after a complete role read, not a transient peak. */
  sampledBoundaryMaximumExternalBytes: number;
}

/** Public parent-runner options. */
interface RunnerOptions {
  /** Exact byte length generated independently for BLOB and TEXT. */
  payloadBytes: number;
  /** Maximum returned Buffer length for the chunk reader. */
  chunkBytes: number;
  /** Optional destination for the parent artifact. */
  outputPath: string | null;
  /** Upper bound for the fresh-process reader invocation. */
  childTimeoutMs: number;
  /** Internal child-only database path. */
  internalReadDatabasePath: string | null;
  /** Internal child-only expected BLOB checksum. */
  expectedBlobSha256: string | null;
  /** Internal child-only expected TEXT checksum. */
  expectedTextSha256: string | null;
}

/**
 * Parse one bounded positive byte count.
 * @param value - Command-line value.
 * @param option - Option name used in an error.
 * @param maximum - Inclusive allowed maximum.
 * @returns Parsed safe byte count.
 */
function parseBytes(value: string | undefined, option: string, maximum: number): number {
  const parsed = Number.parseInt(value ?? '', 10);
  if (!Number.isSafeInteger(parsed) || parsed < 1 || parsed > maximum) {
    throw new RangeError(`${option} must be an integer from 1 to ${maximum}`);
  }
  return parsed;
}

/**
 * Parse public and internal command-line options.
 * @param argv - Arguments after the TypeScript file path.
 * @returns Validated runner settings.
 */
function parseOptions(argv: readonly string[]): RunnerOptions {
  const options: RunnerOptions = {
    payloadBytes: DEFAULT_PAYLOAD_BYTES,
    chunkBytes: DEFAULT_CHUNK_BYTES,
    outputPath: null,
    childTimeoutMs: DEFAULT_CHILD_TIMEOUT_MS,
    internalReadDatabasePath: null,
    expectedBlobSha256: null,
    expectedTextSha256: null
  };
  for (let index = 0; index < argv.length; index++) {
    const option = argv[index];
    const value = argv[index + 1];
    switch (option) {
      case '--bytes':
        options.payloadBytes = parseBytes(value, option, MAX_PAYLOAD_BYTES);
        index++;
        break;
      case '--chunk-bytes':
        options.chunkBytes = parseBytes(value, option, MAX_PAYLOAD_BYTES);
        index++;
        break;
      case '--output':
        if (!value) throw new Error('--output requires a path');
        options.outputPath = path.resolve(value);
        index++;
        break;
      case '--child-timeout-ms':
        options.childTimeoutMs = parseBytes(value, option, 10 * 60 * 1000);
        if (options.childTimeoutMs < 1_000) {
          throw new RangeError('--child-timeout-ms must be at least 1000');
        }
        index++;
        break;
      case '--internal-read':
        if (!value) throw new Error('--internal-read requires a path');
        options.internalReadDatabasePath = path.resolve(value);
        index++;
        break;
      case '--expected-blob-sha256':
        if (!value) throw new Error('--expected-blob-sha256 requires a value');
        options.expectedBlobSha256 = value;
        index++;
        break;
      case '--expected-text-sha256':
        if (!value) throw new Error('--expected-text-sha256 requires a value');
        options.expectedTextSha256 = value;
        index++;
        break;
      default:
        throw new Error(`Unknown option ${option ?? '<missing>'}`);
    }
  }
  const internalValues = [
    options.internalReadDatabasePath,
    options.expectedBlobSha256,
    options.expectedTextSha256
  ];
  const internalCount = internalValues.filter(value => value !== null).length;
  if (internalCount !== 0 && internalCount !== internalValues.length) {
    throw new Error('internal reader requires database path and both expected checksums');
  }
  if (options.internalReadDatabasePath !== null && options.outputPath !== null) {
    throw new Error('internal reader cannot write a parent artifact');
  }
  return options;
}

/**
 * Build deterministic, non-compressibility-dependent BLOB bytes.
 * @param length - Requested byte count.
 * @returns Exact byte fixture.
 */
export function deterministicBlob(length: number): Buffer {
  const output = Buffer.allocUnsafe(length);
  let state = 0x9e3779b9;
  for (let index = 0; index < output.length; index++) {
    state ^= state << 13;
    state ^= state >>> 17;
    state ^= state << 5;
    output[index] = state & 0xff;
  }
  return output;
}

/**
 * Build deterministic valid JSON text of an exact UTF-8 byte length. The
 * JSON string contains two-, three-, and four-byte code points. SQLite slicing
 * happens only after `CAST(... AS BLOB)`, so this fixture detects accidental
 * character-oriented slicing and chunks deliberately split encoded code points.
 * @param length - Requested UTF-8 byte count.
 * @returns Exact valid UTF-8 JSON fixture.
 */
export function deterministicText(length: number): string {
  const prefix = '{"format":"stage2","note":"µ€😀","population_weights":"';
  const suffix = '"}';
  const fixedBytes = Buffer.byteLength(prefix, 'utf8') + Buffer.byteLength(suffix, 'utf8');
  if (length < fixedBytes + 4) {
    throw new RangeError(`text fixture requires at least ${fixedBytes + 4} UTF-8 bytes`);
  }
  let remainingBytes = length - fixedBytes;
  const fourByteCharacters = Math.floor(remainingBytes / 4);
  let weights = '😀'.repeat(fourByteCharacters);
  remainingBytes -= fourByteCharacters * 4;
  if (remainingBytes === 3) weights += '€';
  else if (remainingBytes === 2) weights += 'é';
  else if (remainingBytes === 1) weights += 'x';
  const text = `${prefix}${weights}${suffix}`;
  if (Buffer.byteLength(text, 'utf8') !== length) {
    throw new Error(`text fixture retained ${Buffer.byteLength(text, 'utf8')} UTF-8 bytes; expected ${length}`);
  }
  JSON.parse(text);
  return text;
}

/**
 * Confirm the source query strings remain bounded `substr(CAST(... AS BLOB))`
 * expressions and do not name a full payload column in their SELECT list.
 * @returns Static source-shape assertions retained in the artifact.
 */
export function assertBoundedSliceQueryShape(): {
  chunkQueriesUseBoundedSubstrCast: boolean;
  chunkPathContainsNoFullColumnSelect: boolean;
} {
  const queries = [BLOB_SLICE_SQL, TEXT_SLICE_SQL];
  const chunkQueriesUseBoundedSubstrCast = queries.every(query =>
    /SELECT\s+substr\(CAST\(payload_(blob|text)\s+AS\s+BLOB\),\s*\?,\s*\?\)\s+AS\s+chunk/i.test(query)
  );
  const chunkPathContainsNoFullColumnSelect = queries.every(query =>
    !/SELECT\s+payload_(blob|text)\b/i.test(query)
  );
  if (!chunkQueriesUseBoundedSubstrCast || !chunkPathContainsNoFullColumnSelect) {
    throw new Error('chunk path query shape is not bounded SQLite substr(CAST(... AS BLOB))');
  }
  return { chunkQueriesUseBoundedSubstrCast, chunkPathContainsNoFullColumnSelect };
}

/**
 * Generate two representative legacy columns in a new disposable database.
 * The caller owns cleanup of the already verified parent temporary directory.
 * @param databasePath - New database file beneath an owned temp root.
 * @param payloadBytes - Exact independent BLOB and TEXT logical byte length.
 * @returns Fixture identity and logical checksums.
 */
export function createLegacySliceFixture(
  databasePath: string,
  payloadBytes: number
): LegacySliceFixture {
  if (fs.existsSync(databasePath)) throw new Error(`Refusing to overwrite fixture: ${databasePath}`);
  const blob = deterministicBlob(payloadBytes);
  const text = deterministicText(payloadBytes);
  const textBytes = Buffer.from(text, 'utf8');
  if (textBytes.length !== payloadBytes) throw new Error('text fixture did not retain its byte length');
  const database = new Database(databasePath);
  try {
    const sqliteEncoding = database.pragma('encoding', { simple: true }) as string;
    if (sqliteEncoding.toUpperCase() !== 'UTF-8') {
      throw new Error(`expected SQLite UTF-8 database encoding; received ${sqliteEncoding}`);
    }
    database.exec(`
      CREATE TABLE legacy_values (
        kind TEXT PRIMARY KEY NOT NULL,
        payload_blob BLOB,
        payload_text TEXT
      )
    `);
    const insert = database.prepare(
      'INSERT INTO legacy_values(kind, payload_blob, payload_text) VALUES (?, ?, ?)'
    );
    const write = database.transaction(() => {
      insert.run(BLOB_KIND, blob, null);
      insert.run(TEXT_KIND, null, text);
    });
    write();
    return {
      databasePath,
      payloadBytes,
      blobSha256: createHash('sha256').update(blob).digest('hex'),
      textSha256: createHash('sha256').update(textBytes).digest('hex'),
      databaseBytes: fs.statSync(databasePath).size,
      sqliteEncoding
    };
  } finally {
    database.close();
  }
}

/**
 * Read one legacy column solely via bounded SQLite BLOB slices.
 *
 * The per-chunk return check proves `better-sqlite3` did not hand JavaScript a
 * larger Buffer for this query. It cannot prove SQLite's C implementation did
 * not allocate more memory internally while evaluating `substr`.
 *
 * @param database - Open fixture database.
 * @param role - Selected BLOB or TEXT role.
 * @param chunkBytes - Maximum returned JavaScript Buffer length.
 * @returns Exact bounded reconstruction result.
 */
export function readLegacyColumnSlices(
  database: Database.Database,
  role: LegacySliceRole,
  chunkBytes: number
): SliceReadResult {
  if (!Number.isSafeInteger(chunkBytes) || chunkBytes < 1) {
    throw new RangeError('chunkBytes must be a positive safe integer');
  }
  const column = role === BLOB_KIND ? 'payload_blob' : 'payload_text';
  const lengthSql = `
    SELECT length(CAST(${column} AS BLOB)) AS logical_bytes
      FROM legacy_values
     WHERE kind = ?
  `;
  const lengthRow = database.prepare(lengthSql).get(role) as { logical_bytes: number | null } | undefined;
  const logicalBytes = lengthRow?.logical_bytes;
  if (!Number.isSafeInteger(logicalBytes) || logicalBytes < 0) {
    throw new Error(`missing or invalid ${role} logical byte length`);
  }
  const query = database.prepare(role === BLOB_KIND ? BLOB_SLICE_SQL : TEXT_SLICE_SQL);
  const hash = createHash('sha256');
  let bytes = 0;
  let chunks = 0;
  let largestReturnedChunkBytes = 0;
  const started = performance.now();
  for (let offset = 0; offset < logicalBytes; offset += chunkBytes) {
    const expected = Math.min(chunkBytes, logicalBytes - offset);
    const row = query.get(offset + 1, expected, role) as { chunk: Buffer | null } | undefined;
    if (!row || !Buffer.isBuffer(row.chunk)) {
      throw new Error(`${role} slice at byte ${offset} did not return a Buffer`);
    }
    const chunk = row.chunk;
    if (chunk.length !== expected || chunk.length > chunkBytes) {
      throw new Error(
        `${role} slice at byte ${offset} returned ${chunk.length} bytes; expected ${expected} at most ${chunkBytes}`
      );
    }
    hash.update(chunk);
    bytes += chunk.length;
    chunks++;
    largestReturnedChunkBytes = Math.max(largestReturnedChunkBytes, chunk.length);
  }
  if (bytes !== logicalBytes) {
    throw new Error(`${role} slice reconstruction ended at ${bytes} bytes; expected ${logicalBytes}`);
  }
  return {
    role,
    logicalBytes,
    chunks,
    largestReturnedChunkBytes,
    sha256: hash.digest('hex'),
    allChunksBounded: largestReturnedChunkBytes <= chunkBytes,
    durationMs: Number((performance.now() - started).toFixed(6))
  };
}

/**
 * Record process memory only at a whole-role boundary. This intentionally does
 * not claim to observe allocations that begin and end within SQLite's native
 * `substr` evaluation.
 * @param samples - Mutable whole-role boundary tracker.
 */
function sampleMemoryBoundary(samples: MemorySamples): void {
  const memory = process.memoryUsage();
  samples.sampledBoundaryMaximumRssBytes = Math.max(samples.sampledBoundaryMaximumRssBytes, memory.rss);
  samples.sampledBoundaryMaximumHeapUsedBytes = Math.max(
    samples.sampledBoundaryMaximumHeapUsedBytes,
    memory.heapUsed
  );
  samples.sampledBoundaryMaximumExternalBytes = Math.max(
    samples.sampledBoundaryMaximumExternalBytes,
    memory.external
  );
}

/**
 * Read both roles in a fresh child process context and return its measurements.
 * @param databasePath - Existing disposable fixture database.
 * @param chunkBytes - Maximum JavaScript-visible slice size.
 * @param expectedBlobSha256 - Parent-generated BLOB logical digest.
 * @param expectedTextSha256 - Parent-generated TEXT logical digest.
 * @returns Child evidence result.
 */
function runInternalReader(
  databasePath: string,
  chunkBytes: number,
  expectedBlobSha256: string,
  expectedTextSha256: string
): ReaderChildResult {
  const queryShape = assertBoundedSliceQueryShape();
  const database = new Database(databasePath, { readonly: true, fileMustExist: true });
  const before = process.memoryUsage();
  const memory: MemorySamples = {
    before,
    after: before,
    sampledBoundaryMaximumRssBytes: before.rss,
    sampledBoundaryMaximumHeapUsedBytes: before.heapUsed,
    sampledBoundaryMaximumExternalBytes: before.external
  };
  try {
    const sqliteEncoding = database.pragma('encoding', { simple: true }) as string;
    const blob = readLegacyColumnSlices(database, BLOB_KIND, chunkBytes);
    sampleMemoryBoundary(memory);
    const text = readLegacyColumnSlices(database, TEXT_KIND, chunkBytes);
    sampleMemoryBoundary(memory);
    memory.after = process.memoryUsage();
    sampleMemoryBoundary(memory);
    const result: ReaderChildResult = {
      schema: 'slither-stage2-sqlite-legacy-slice-child',
      version: 1,
      caveat: 'Exact hashes and returned Buffer caps prove the JavaScript-facing chunk result. Boundary-only memory samples do not prove SQLite or better-sqlite3 native internals never allocate a full value while evaluating substr. This result does not authorize this query path for production.',
      databasePath,
      chunkBytes,
      reads: { blob, text },
      memory,
      assertions: {
        ...queryShape,
        exactBlob: blob.sha256 === expectedBlobSha256,
        exactText: text.sha256 === expectedTextSha256,
        boundedBlob: blob.allChunksBounded,
        boundedText: text.allChunksBounded,
        sqliteEncodingIsUtf8: sqliteEncoding.toUpperCase() === 'UTF-8'
      },
      conclusion: {
        boundedNativeAllocationProved: false,
        productionQueryPathAuthorized: false,
        requiredProductionDecision: 'Use a separately reviewed native incremental-BLOB path, or document and enforce a stricter legacy compatibility ceiling before production use.'
      }
    };
    if (!Object.values(result.assertions).every(Boolean)) {
      throw new Error('SQLite slice child assertions failed');
    }
    return result;
  } finally {
    database.close();
  }
}

/**
 * Confirm a temporary path is a direct child of the platform temp directory
 * and carries the experiment's fixed prefix before recursive cleanup.
 * @param temporaryRoot - Candidate directory created by mkdtemp.
 * @returns Absolute validated path.
 */
export function verifyTemporaryRoot(temporaryRoot: string): string {
  const resolvedRoot = path.resolve(temporaryRoot);
  const resolvedTemp = path.resolve(os.tmpdir());
  if (
    path.dirname(resolvedRoot) !== resolvedTemp ||
    !path.basename(resolvedRoot).startsWith('slither-stage2-sqlite-slice-')
  ) {
    throw new Error(`Refusing to clean unexpected temporary root: ${resolvedRoot}`);
  }
  return resolvedRoot;
}

/**
 * Return current source identity without modifying the repository.
 * @returns Commit identity and worktree state.
 */
function sourceIdentity(): { commit: string; dirty: boolean } {
  const commit = spawnSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' });
  const status = spawnSync('git', ['status', '--porcelain'], { encoding: 'utf8' });
  return {
    commit: commit.status === 0 ? commit.stdout.trim() : 'unavailable',
    dirty: status.status !== 0 || status.stdout.trim().length > 0
  };
}

/**
 * Return the installed better-sqlite3 package version.
 * @returns Installed package version or an explicit unavailable marker.
 */
function betterSqlite3Version(): string {
  const require = createRequire(import.meta.url);
  try {
    return (require('better-sqlite3/package.json') as { version: string }).version;
  } catch {
    return 'unavailable';
  }
}

/**
 * Read the SQLite library identity without retaining an extra database handle.
 * @param databasePath - Existing disposable database path.
 * @returns SQLite library version reported by the opened database.
 */
function sqliteVersion(databasePath: string): string {
  const database = new Database(databasePath, { readonly: true, fileMustExist: true });
  try {
    return (database.prepare('SELECT sqlite_version() AS version').get() as { version: string }).version;
  } finally {
    database.close();
  }
}

/**
 * Spawn a clean Node process for read-path memory measurement.
 * @param fixture - Parent-created database and logical digests.
 * @param chunkBytes - Requested JavaScript-visible bound.
 * @param timeoutMs - Bounded wait for the reader child.
 * @returns Parsed child result.
 */
function invokeReaderChild(
  fixture: LegacySliceFixture,
  chunkBytes: number,
  timeoutMs: number
): ReaderChildResult {
  const scriptPath = fileURLToPath(import.meta.url);
  const tsxCli = path.resolve('node_modules', 'tsx', 'dist', 'cli.mjs');
  const child = spawnSync(process.execPath, [
    tsxCli,
    scriptPath,
    '--internal-read', fixture.databasePath,
    '--chunk-bytes', String(chunkBytes),
    '--expected-blob-sha256', fixture.blobSha256,
    '--expected-text-sha256', fixture.textSha256
  ], { encoding: 'utf8', maxBuffer: 1024 * 1024, timeout: timeoutMs });
  if (child.error?.code === 'ETIMEDOUT') {
    throw new Error(`SQLite slice reader child timed out after ${timeoutMs} ms`);
  }
  if (child.error) {
    throw new Error(`SQLite slice reader child could not start: ${child.error.message}`);
  }
  if (child.status !== 0) {
    throw new Error(`slice reader child failed: ${child.stderr || child.stdout || `status ${child.status}`}`);
  }
  try {
    return JSON.parse(child.stdout) as ReaderChildResult;
  } catch (error) {
    throw new Error(`slice reader child emitted invalid JSON: ${(error as Error).message}`);
  }
}

/**
 * Atomically create a new artifact without overwriting a user-selected file.
 * @param outputPath - Explicit destination.
 * @param result - JSON-safe artifact.
 */
function writeArtifact(outputPath: string, result: Record<string, unknown>): void {
  fs.mkdirSync(path.dirname(outputPath), { recursive: true });
  try {
    fs.writeFileSync(outputPath, `${JSON.stringify(result, null, 2)}\n`, {
      encoding: 'utf8',
      flag: 'wx'
    });
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'EEXIST') {
      throw new Error(`Refusing to overwrite artifact: ${outputPath}`);
    }
    throw error;
  }
}

/**
 * Run the parent fixture creator and isolated-reader measurement.
 * @param options - Validated public runner settings.
 * @returns Complete parent evidence artifact.
 */
function runParentExperiment(options: RunnerOptions): Record<string, unknown> {
  const temporaryRoot = verifyTemporaryRoot(
    fs.mkdtempSync(path.join(os.tmpdir(), 'slither-stage2-sqlite-slice-'))
  );
  const databasePath = path.join(temporaryRoot, 'legacy-columns.db');
  try {
    const fixture = createLegacySliceFixture(databasePath, options.payloadBytes);
    const reader = invokeReaderChild(fixture, options.chunkBytes, options.childTimeoutMs);
    return {
      schema: 'slither-stage2-sqlite-legacy-slice',
      version: 1,
      evidenceClass: 'new measured result',
      caveat: 'Disposable fixture and fresh reader child only. Exact reconstruction plus returned chunk caps prove the JavaScript-facing result, not bounded allocation inside SQLite or better-sqlite3 native code. This result does not satisfy proof of bounded native allocation and does not authorize this query path for production. This is not a production legacy reader, migration, checkpoint schema, export path, or database benchmark.',
      source: sourceIdentity(),
      environment: {
        capturedAt: new Date().toISOString(),
        platform: process.platform,
        architecture: process.arch,
        osType: os.type(),
        osRelease: os.release(),
        node: process.version,
        v8: process.versions.v8,
        sqlite: sqliteVersion(databasePath),
        betterSqlite3: betterSqlite3Version()
      },
      fixture: {
        payloadBytesPerColumn: fixture.payloadBytes,
        totalLogicalBytes: fixture.payloadBytes * 2,
        blobSha256: fixture.blobSha256,
        textSha256: fixture.textSha256,
        databaseBytes: fixture.databaseBytes,
        sqliteEncoding: fixture.sqliteEncoding,
        contentNote: 'deterministic high-entropy BLOB and deterministic valid JSON UTF-8 TEXT with multibyte code points; no owner data'
      },
      readerProcess: {
        timeoutMs: options.childTimeoutMs,
        timeoutPolicy: 'bounded child process; timeout fails with a clear diagnosis and the verified disposable root is removed in finally'
      },
      result: reader
    };
  } finally {
    fs.rmSync(temporaryRoot, { recursive: true, force: true });
  }
}

/** Execute the command-line experiment. */
function main(): void {
  const options = parseOptions(process.argv.slice(2));
  if (options.internalReadDatabasePath !== null) {
    const result = runInternalReader(
      options.internalReadDatabasePath,
      options.chunkBytes,
      options.expectedBlobSha256 as string,
      options.expectedTextSha256 as string
    );
    process.stdout.write(`${JSON.stringify(result)}\n`);
    return;
  }
  const result = runParentExperiment(options);
  if (options.outputPath) {
    writeArtifact(options.outputPath, result);
    console.info(`[stage2.sqlite-legacy-slice] wrote ${options.outputPath}`);
  } else {
    process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  try {
    main();
  } catch (error) {
    console.error(error);
    process.exitCode = 1;
  }
}
