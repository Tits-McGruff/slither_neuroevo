/** Explicit offline legacy-database compaction after a validated complete backup. */

import { existsSync } from 'node:fs';
import { lstat, mkdir, readdir, stat, statfs } from 'node:fs/promises';
import { dirname, resolve, sep } from 'node:path';
import { performance } from 'node:perf_hooks';
import { pathToFileURL } from 'node:url';
import Database from 'better-sqlite3';
import { createManagedBackup, validateManagedBackup } from '../managedBackup.ts';

/** Extra unallocated bytes kept while a complete SQLite temporary copy is built. */
const OPERATING_RESERVE_BYTES = 512n * 1024n * 1024n;

/** Exact operator-selected paths for a one-shot offline maintenance action. */
export interface LegacyCompactionOptions {
  databasePath: string;
  backupDirectory: string;
}

/** Byte and page measurements around one completed SQLite boundary. */
interface DatabaseSize {
  databaseBytes: number;
  walBytes: number;
  shmBytes: number;
  pageCount: number;
  freelistPages: number;
}

/** Parse explicit paths and the operator's offline confirmation. */
function parseOptions(argv: readonly string[]): LegacyCompactionOptions {
  if (argv.length !== 5 || argv[0] !== '--db-path' || !argv[1] ||
      argv[2] !== '--backup' || !argv[3] || argv[4] !== '--offline') {
    throw new Error('usage: compact-legacy-database.ts --db-path EXISTING_DB --backup NEW_DIR --offline');
  }
  return { databasePath: resolve(argv[1]), backupDirectory: resolve(argv[3]) };
}

/** Return a regular file's physical length or zero for an absent SQLite sidecar. */
async function fileBytes(path: string): Promise<number> {
  try { return (await stat(path)).size; }
  catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') return 0;
    throw error;
  }
}

/** Read exact SQLite file and page totals without materializing legacy blobs. */
async function measureDatabase(path: string, db: Database.Database): Promise<DatabaseSize> {
  return {
    databaseBytes: await fileBytes(path),
    walBytes: await fileBytes(`${path}-wal`),
    shmBytes: await fileBytes(`${path}-shm`),
    pageCount: db.pragma('page_count', { simple: true }) as number,
    freelistPages: db.pragma('freelist_count', { simple: true }) as number
  };
}

/** Read unallocated bytes on the volume containing a path's existing parent. */
async function freeBytes(directory: string): Promise<bigint> {
  const state = await statfs(directory, { bigint: true });
  return state.bavail * state.bsize;
}

/** Ensure the complete backup can be copied before touching the source database. */
async function admitBackup(sourcePath: string, backupDirectory: string): Promise<void> {
  const managedRoot = `${sourcePath}.checkpoints`;
  let estimate = BigInt(await fileBytes(sourcePath)) + BigInt(await fileBytes(`${sourcePath}-wal`));
  if (existsSync(managedRoot)) {
    for (const entry of await readdir(managedRoot, { withFileTypes: true })) {
      if (entry.isFile()) estimate += BigInt(await fileBytes(resolve(managedRoot, entry.name)));
    }
  }
  await mkdir(dirname(backupDirectory), { recursive: true });
  if (await freeBytes(dirname(backupDirectory)) < estimate + OPERATING_RESERVE_BYTES) {
    throw new Error('backup destination lacks source-copy bytes plus the operating reserve');
  }
}

/** Force a stopped WAL database to release completed pages before measuring it. */
function checkpointWal(db: Database.Database): void {
  if (db.pragma('journal_mode', { simple: true }) !== 'wal') return;
  const results = db.pragma('wal_checkpoint(TRUNCATE)') as Array<{
    busy: number; log: number; checkpointed: number
  }>;
  if (results.length !== 1 || results[0]?.busy !== 0) {
    throw new Error('offline compaction cannot obtain an uncontended WAL checkpoint');
  }
}

/** Back up and compact one stopped legacy database, retaining the original backup. */
export async function compactLegacyDatabase(options: LegacyCompactionOptions): Promise<Record<string, unknown>> {
  const databasePath = resolve(options.databasePath);
  const backupDirectory = resolve(options.backupDirectory);
  const managedRoot = `${databasePath}.checkpoints`;
  if (backupDirectory === databasePath || backupDirectory.startsWith(`${managedRoot}${sep}`) ||
      backupDirectory === managedRoot || existsSync(backupDirectory)) {
    throw new Error('backup must be a new directory outside the managed checkpoint root');
  }
  const source = await lstat(databasePath);
  if (!source.isFile() || source.isSymbolicLink()) throw new Error('database must be a regular non-symlink file');
  await admitBackup(databasePath, backupDirectory);
  const started = performance.now();
  const backup = await createManagedBackup({ databasePath, outputDirectory: backupDirectory });
  await validateManagedBackup(backupDirectory);
  if (await freeBytes(dirname(databasePath)) <
      (BigInt(await fileBytes(databasePath)) + BigInt(await fileBytes(`${databasePath}-wal`))) * 2n +
      OPERATING_RESERVE_BYTES) {
    throw new Error('validated backup retained; source volume lacks SQLite temporary-copy headroom');
  }
  const db = new Database(databasePath, { fileMustExist: true, timeout: 5000 });
  try {
    if (db.pragma('quick_check', { simple: true }) !== 'ok') {
      throw new Error('source database failed SQLite quick_check before compaction');
    }
    checkpointWal(db);
    const before = await measureDatabase(databasePath, db);
    db.exec('VACUUM');
    checkpointWal(db);
    if (db.pragma('quick_check', { simple: true }) !== 'ok') {
      throw new Error('compacted database failed SQLite quick_check');
    }
    const after = await measureDatabase(databasePath, db);
    if (after.freelistPages !== 0) throw new Error('VACUUM left reusable legacy pages allocated');
    return { databasePath, backupDirectory, backupDatabaseBytes: backup.database.bytes,
      backedUpManagedFiles: backup.managedFiles.length,
      before, after, elapsedSeconds: (performance.now() - started) / 1000 };
  } finally { db.close(); }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  void compactLegacyDatabase(parseOptions(process.argv.slice(2)))
    .then(result => process.stdout.write(`${JSON.stringify(result, null, 2)}\n`))
    .catch(error => { console.error(error); process.exitCode = 1; });
}
