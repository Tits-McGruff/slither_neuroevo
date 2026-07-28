/** Narrow disposable SQLite BLOB-volume experiment approved for Stage 2. */

import { createHash } from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { performance } from 'node:perf_hooks';
import { spawnSync } from 'node:child_process';
import Database from 'better-sqlite3';

/** Default BLOB chunk size for the disposable measurement. */
const DEFAULT_CHUNK_BYTES = 1024 * 1024;
/** Representative measured P0 shuffled-Zstandard payload bytes. */
const DEFAULT_PAYLOAD_BYTES = 2_527_124;

/** SQLite experiment options. */
interface SqliteExperimentOptions {
  /** Deterministic payload size. */
  payloadBytes: number;
  /** Row chunk size. */
  chunkBytes: number;
  /** Optional output artifact. */
  outputPath: string | null;
}

/** File and page state at one experiment boundary. */
interface StorageState {
  /** Main database bytes. */
  databaseBytes: number;
  /** WAL bytes. */
  walBytes: number;
  /** Shared-memory bytes. */
  shmBytes: number;
  /** Main database page count. */
  pageCount: number;
  /** Reusable page count. */
  freelistCount: number;
}

/**
 * Parse one bounded integer.
 * @param value - Raw CLI value.
 * @param option - Option name.
 * @returns Parsed value.
 */
function parseInteger(value: string | undefined, option: string): number {
  const parsed = Number.parseInt(value ?? '', 10);
  if (!Number.isSafeInteger(parsed) || parsed < 1 || parsed > 2_000_000_000) {
    throw new Error(`${option} must be an integer from 1 to 2000000000`);
  }
  return parsed;
}

/**
 * Parse command-line options.
 * @param argv - Arguments after script path.
 * @returns Validated experiment settings.
 */
function parseOptions(argv: readonly string[]): SqliteExperimentOptions {
  const result: SqliteExperimentOptions = {
    payloadBytes: DEFAULT_PAYLOAD_BYTES,
    chunkBytes: DEFAULT_CHUNK_BYTES,
    outputPath: null
  };
  for (let index = 0; index < argv.length; index++) {
    const option = argv[index];
    const value = argv[index + 1];
    switch (option) {
      case '--bytes':
        result.payloadBytes = parseInteger(value, option);
        index++;
        break;
      case '--chunk-bytes':
        result.chunkBytes = parseInteger(value, option);
        index++;
        break;
      case '--output':
        if (!value) throw new Error('--output requires a path');
        result.outputPath = path.resolve(value);
        index++;
        break;
      default:
        throw new Error(`Unknown option ${option ?? '<missing>'}`);
    }
  }
  return result;
}

/**
 * Return file bytes, or zero for an absent WAL/SHM file.
 * @param filePath - File path.
 * @returns File size.
 */
function fileSize(filePath: string): number {
  try {
    return fs.statSync(filePath).size;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') return 0;
    throw error;
  }
}

/**
 * Fill a deterministic high-entropy byte volume. Content entropy does not
 * affect SQLite BLOB storage; the size represents a previously compressed
 * checkpoint payload.
 * @param length - Payload bytes.
 * @returns Reproducible bytes.
 */
function deterministicPayload(length: number): Buffer {
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
 * Read source identity.
 * @returns Commit and dirty flag.
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
 * Capture file/page state.
 * @param db - Open experiment database.
 * @param databasePath - Main file path.
 * @returns Storage state.
 */
function captureStorage(db: Database.Database, databasePath: string): StorageState {
  return {
    databaseBytes: fileSize(databasePath),
    walBytes: fileSize(`${databasePath}-wal`),
    shmBytes: fileSize(`${databasePath}-shm`),
    pageCount: db.pragma('page_count', { simple: true }) as number,
    freelistCount: db.pragma('freelist_count', { simple: true }) as number
  };
}

/**
 * Measure a synchronous operation and the delay it imposes on a zero-delay
 * timer scheduled immediately before it.
 * @param operation - Blocking operation.
 * @returns Duration, timer delay, and result.
 */
async function measureBlocking<T>(
  operation: () => T
): Promise<{ operationMs: number; timerDelayMs: number; value: T }> {
  const timerScheduled = performance.now();
  let timerResolved!: (delay: number) => void;
  const timer = new Promise<number>(resolve => {
    timerResolved = resolve;
  });
  setTimeout(() => timerResolved(performance.now() - timerScheduled), 0);
  const started = performance.now();
  const value = operation();
  const operationMs = performance.now() - started;
  const timerDelayMs = await timer;
  return {
    operationMs: Number(operationMs.toFixed(6)),
    timerDelayMs: Number(timerDelayMs.toFixed(6)),
    value
  };
}

/**
 * Run the explicitly non-production SQLite comparison.
 * @param options - Validated payload and chunk sizes.
 * @returns Evidence object.
 */
async function runExperiment(options: SqliteExperimentOptions): Promise<Record<string, unknown>> {
  const payload = deterministicPayload(options.payloadBytes);
  const payloadSha256 = createHash('sha256').update(payload).digest('hex');
  const temporaryRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'slither-stage2-sqlite-volume-'));
  const databasePath = path.join(temporaryRoot, 'experiment.db');
  const db = new Database(databasePath);
  try {
    db.pragma('journal_mode = WAL');
    db.pragma('synchronous = FULL');
    db.pragma('wal_autocheckpoint = 0');
    db.exec(`
      CREATE TABLE payload_chunks (
        sequence INTEGER PRIMARY KEY,
        payload BLOB NOT NULL
      )
    `);
    db.pragma('wal_checkpoint(TRUNCATE)');
    const baseline = captureStorage(db, databasePath);
    const insert = db.prepare(
      'INSERT INTO payload_chunks(sequence, payload) VALUES (?, ?)'
    );
    const insertAll = db.transaction(() => {
      let sequence = 0;
      for (let offset = 0; offset < payload.length; offset += options.chunkBytes) {
        insert.run(sequence++, payload.subarray(offset, Math.min(payload.length, offset + options.chunkBytes)));
      }
      return sequence;
    });
    const inserted = await measureBlocking(insertAll);
    const afterInsertCommit = captureStorage(db, databasePath);
    const insertCheckpoint = await measureBlocking(
      () => db.pragma('wal_checkpoint(TRUNCATE)')
    );
    const afterInsertCheckpoint = captureStorage(db, databasePath);

    const read = await measureBlocking(() => {
      const hash = createHash('sha256');
      let bytes = 0;
      let rows = 0;
      for (const row of db.prepare(
        'SELECT payload FROM payload_chunks ORDER BY sequence'
      ).iterate() as Iterable<{ payload: Buffer }>) {
        hash.update(row.payload);
        bytes += row.payload.length;
        rows++;
      }
      return { bytes, rows, sha256: hash.digest('hex') };
    });

    const deleted = await measureBlocking(
      () => db.transaction(() => db.prepare('DELETE FROM payload_chunks').run())()
    );
    const afterDeleteCommit = captureStorage(db, databasePath);
    const deleteCheckpoint = await measureBlocking(
      () => db.pragma('wal_checkpoint(TRUNCATE)')
    );
    const afterDeleteCheckpoint = captureStorage(db, databasePath);

    const reinserted = await measureBlocking(insertAll);
    const afterReuseCommit = captureStorage(db, databasePath);
    const reuseCheckpoint = await measureBlocking(
      () => db.pragma('wal_checkpoint(TRUNCATE)')
    );
    const afterReuseCheckpoint = captureStorage(db, databasePath);

    db.transaction(() => db.prepare('DELETE FROM payload_chunks').run())();
    db.pragma('wal_checkpoint(TRUNCATE)');
    const beforeVacuum = captureStorage(db, databasePath);
    const vacuum = await measureBlocking(() => db.exec('VACUUM'));
    db.pragma('wal_checkpoint(TRUNCATE)');
    const afterVacuum = captureStorage(db, databasePath);

    return {
      schema: 'slither-stage2-sqlite-byte-volume',
      version: 1,
      evidenceClass: 'new measured result',
      caveat: 'Disposable byte-volume experiment only. It is not a checkpoint schema, reader, backup, pruning, recovery, export, or competing production persistence implementation.',
      source: sourceIdentity(),
      environment: {
        capturedAt: new Date().toISOString(),
        platform: process.platform,
        architecture: process.arch,
        osType: os.type(),
        osRelease: os.release(),
        node: process.version,
        sqlite: (db.prepare('SELECT sqlite_version() AS version').get() as { version: string }).version
      },
      fixture: {
        payloadBytes: payload.length,
        payloadSha256,
        chunkBytes: options.chunkBytes,
        expectedChunks: Math.ceil(payload.length / options.chunkBytes),
        contentNote: 'deterministic high-entropy bytes representing an already-compressed payload volume'
      },
      result: {
        baseline,
        insert: {
          operationMs: inserted.operationMs,
          timerDelayMs: inserted.timerDelayMs,
          rows: inserted.value,
          afterCommit: afterInsertCommit,
          walToPayloadRatio: Number((afterInsertCommit.walBytes / payload.length).toFixed(8)),
          checkpointMs: insertCheckpoint.operationMs,
          checkpointTimerDelayMs: insertCheckpoint.timerDelayMs,
          afterCheckpoint: afterInsertCheckpoint
        },
        read: {
          operationMs: read.operationMs,
          timerDelayMs: read.timerDelayMs,
          ...read.value,
          bitExact: read.value.sha256 === payloadSha256
        },
        deletion: {
          operationMs: deleted.operationMs,
          timerDelayMs: deleted.timerDelayMs,
          changedRows: deleted.value.changes,
          afterCommit: afterDeleteCommit,
          checkpointMs: deleteCheckpoint.operationMs,
          checkpointTimerDelayMs: deleteCheckpoint.timerDelayMs,
          afterCheckpoint: afterDeleteCheckpoint
        },
        pageReuse: {
          operationMs: reinserted.operationMs,
          timerDelayMs: reinserted.timerDelayMs,
          rows: reinserted.value,
          afterCommit: afterReuseCommit,
          checkpointMs: reuseCheckpoint.operationMs,
          checkpointTimerDelayMs: reuseCheckpoint.timerDelayMs,
          afterCheckpoint: afterReuseCheckpoint,
          addedMainPagesVersusFirstInsert:
            afterReuseCheckpoint.pageCount - afterInsertCheckpoint.pageCount
        },
        vacuum: {
          before: beforeVacuum,
          operationMs: vacuum.operationMs,
          timerDelayMs: vacuum.timerDelayMs,
          after: afterVacuum
        }
      }
    };
  } finally {
    db.close();
    fs.rmSync(temporaryRoot, { recursive: true, force: true });
  }
}

/** Execute the CLI. */
async function main(): Promise<void> {
  const options = parseOptions(process.argv.slice(2));
  const result = await runExperiment(options);
  const json = `${JSON.stringify(result, null, 2)}\n`;
  if (options.outputPath) {
    fs.mkdirSync(path.dirname(options.outputPath), { recursive: true });
    fs.writeFileSync(options.outputPath, json, 'utf8');
    console.info(`[stage2.sqlite-volume] wrote ${options.outputPath}`);
  } else {
    process.stdout.write(json);
  }
}

main().catch(error => {
  console.error(error);
  process.exitCode = 1;
});
