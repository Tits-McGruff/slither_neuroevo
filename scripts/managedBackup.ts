import { createHash } from 'node:crypto';
import {
  constants,
  closeSync,
  copyFileSync,
  createReadStream,
  existsSync,
  fsyncSync,
  lstatSync,
  mkdirSync,
  openSync,
  readFileSync,
  realpathSync,
  renameSync,
  rmSync,
  statSync,
  writeFileSync
} from 'node:fs';
import { basename, dirname, join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import Database from 'better-sqlite3';

/** Versioned directory-backup manifest identifier. */
export const MANAGED_BACKUP_FORMAT = 'slither-managed-backup-v1' as const;

/** One immutable managed file retained by a database snapshot. */
export interface ManagedBackupFile {
  name: string;
  bytes: number;
  sha256: string;
}

/** Self-validating database-plus-managed-files backup description. */
export interface ManagedBackupManifest {
  format: typeof MANAGED_BACKUP_FORMAT;
  createdAt: string;
  database: ManagedBackupFile;
  managedFiles: ManagedBackupFile[];
}

/** Options for producing one consistent online backup set. */
export interface CreateManagedBackupOptions {
  databasePath: string;
  outputDirectory: string;
  maxAttempts?: number;
}

/** Options for restoring a validated backup to a new database path. */
export interface RestoreManagedBackupOptions {
  backupDirectory: string;
  databasePath: string;
}

/** Minimal immutable-file row read from the copied SQLite snapshot. */
interface InventoryRow {
  relative_filename: string;
  stored_byte_count_hex: string;
}

/** Error indicating that pruning changed the selected file set during copy. */
class BackupRaceError extends Error {}

/** Strict direct-child final object naming contract. */
const MANAGED_FINAL_NAME = /^[0-9a-f]{64}\.(?:checkpoint-v3|hof-weights-v1)$/u;

/** Manifest file stored at the root of every backup directory. */
const MANIFEST_NAME = 'manifest.json';

/** Database filename inside a portable backup directory. */
const BACKUP_DATABASE_NAME = 'slither.db';

/** Managed-object subdirectory inside a portable backup directory. */
const BACKUP_MANAGED_DIRECTORY = 'managed';

/** Parse an exact unsigned 64-bit hexadecimal byte count into a safe file size. */
function parseStoredBytes(value: string): number {
  if (!/^[0-9a-f]{16}$/u.test(value)) throw new Error('backup inventory contains an invalid byte count');
  const bytes = BigInt(`0x${value}`);
  if (bytes > BigInt(Number.MAX_SAFE_INTEGER)) throw new Error('backup inventory file is too large for this host');
  return Number(bytes);
}

/** Hash one file without materializing it in JavaScript memory. */
async function hashFile(path: string): Promise<string> {
  const hash = createHash('sha256');
  const stream = createReadStream(path);
  for await (const chunk of stream) hash.update(chunk as Buffer);
  return hash.digest('hex');
}

/** Close an open file descriptor even when fsync fails. */
function flushAndCloseFile(path: string): void {
  const handle = openSync(path, 'r+');
  try { fsyncSync(handle); }
  finally { closeSync(handle); }
}

/** Return whether one SQLite table exists in a snapshot. */
function hasTable(database: Database.Database, name: string): boolean {
  return database.prepare(`SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?`).get(name) !== undefined;
}

/** Read the immutable object inventory from one internally consistent database backup. */
function readSnapshotInventory(snapshotPath: string): Array<{ name: string; bytes: number }> {
  const database = new Database(snapshotPath, { readonly: true, fileMustExist: true });
  try {
    const rows: InventoryRow[] = [];
    if (hasTable(database, 'rust_checkpoint_v3_metadata') &&
        hasTable(database, 'rust_checkpoint_retention_v1')) {
      rows.push(...database.prepare(`SELECT metadata.relative_filename, metadata.stored_byte_count_hex
        FROM rust_checkpoint_v3_metadata AS metadata
        JOIN rust_checkpoint_retention_v1 AS retention USING(checkpoint_id)
        WHERE retention.retention_kind IN ('automatic', 'pinned')
        ORDER BY metadata.relative_filename`).all() as InventoryRow[]);
    }
    if (hasTable(database, 'rust_hall_of_fame_weights_v1') &&
        hasTable(database, 'rust_hall_of_fame_v1')) {
      rows.push(...database.prepare(`SELECT DISTINCT weights.relative_filename, weights.stored_byte_count_hex
        FROM rust_hall_of_fame_weights_v1 AS weights
        JOIN rust_hall_of_fame_v1 AS hall ON hall.weights_sha256 = weights.logical_sha256
        ORDER BY weights.relative_filename`).all() as InventoryRow[]);
    }
    const inventory = new Map<string, number>();
    for (const row of rows) {
      if (!MANAGED_FINAL_NAME.test(row.relative_filename) || basename(row.relative_filename) !== row.relative_filename) {
        throw new Error('backup inventory contains an unsafe managed filename');
      }
      const bytes = parseStoredBytes(row.stored_byte_count_hex);
      const existing = inventory.get(row.relative_filename);
      if (existing !== undefined && existing !== bytes) {
        throw new Error('backup inventory gives one managed object conflicting sizes');
      }
      inventory.set(row.relative_filename, bytes);
    }
    return [...inventory].map(([name, bytes]) => ({ name, bytes })).sort((a, b) => a.name.localeCompare(b.name));
  } finally {
    database.close();
  }
}

/** Copy one immutable direct child while rejecting symlinks, size changes, and pruning races. */
async function copyManagedFile(
  sourceRoot: string,
  targetRoot: string,
  entry: { name: string; bytes: number }
): Promise<ManagedBackupFile> {
  const source = join(sourceRoot, entry.name);
  const target = join(targetRoot, entry.name);
  try {
    const before = lstatSync(source);
    if (before.isSymbolicLink() || !before.isFile() || before.size !== entry.bytes ||
        dirname(realpathSync(source)) !== sourceRoot) {
      throw new BackupRaceError(`managed object changed during backup: ${entry.name}`);
    }
    copyFileSync(source, target, constants.COPYFILE_EXCL);
    const copied = lstatSync(target);
    if (!copied.isFile() || copied.isSymbolicLink() || copied.size !== entry.bytes) {
      throw new BackupRaceError(`managed object copy is incomplete: ${entry.name}`);
    }
    flushAndCloseFile(target);
    return { name: entry.name, bytes: entry.bytes, sha256: await hashFile(target) };
  } catch (error) {
    if (error instanceof BackupRaceError || (error instanceof Error && 'code' in error &&
        ['ENOENT', 'ESTALE'].includes(String((error as NodeJS.ErrnoException).code)))) {
      throw new BackupRaceError(`managed object disappeared during backup: ${entry.name}`);
    }
    throw error;
  }
}

/** Validate a parsed manifest before trusting any paths from it. */
function parseManifest(value: unknown): ManagedBackupManifest {
  if (!value || typeof value !== 'object') throw new Error('backup manifest must be an object');
  const candidate = value as Partial<ManagedBackupManifest>;
  if (candidate.format !== MANAGED_BACKUP_FORMAT || typeof candidate.createdAt !== 'string' ||
      !candidate.database || candidate.database.name !== BACKUP_DATABASE_NAME ||
      !Array.isArray(candidate.managedFiles)) {
    throw new Error('backup manifest has an unsupported shape or version');
  }
  const files = [candidate.database, ...candidate.managedFiles];
  const names = new Set<string>();
  for (const [index, file] of files.entries()) {
    if (!file || typeof file.name !== 'string' || !Number.isSafeInteger(file.bytes) || file.bytes < 0 ||
        !/^[0-9a-f]{64}$/u.test(file.sha256) || basename(file.name) !== file.name ||
        (index > 0 && !MANAGED_FINAL_NAME.test(file.name)) || names.has(file.name)) {
      throw new Error('backup manifest contains an invalid file entry');
    }
    names.add(file.name);
  }
  return candidate as ManagedBackupManifest;
}

/** Validate every byte count and digest in an existing backup directory. */
export async function validateManagedBackup(backupDirectory: string): Promise<ManagedBackupManifest> {
  const root = resolve(backupDirectory);
  const manifest = parseManifest(JSON.parse(readFileSync(join(root, MANIFEST_NAME), 'utf8')));
  for (const [index, file] of [manifest.database, ...manifest.managedFiles].entries()) {
    const path = index === 0 ? join(root, file.name) : join(root, BACKUP_MANAGED_DIRECTORY, file.name);
    const stats = lstatSync(path);
    if (!stats.isFile() || stats.isSymbolicLink() || stats.size !== file.bytes ||
        await hashFile(path) !== file.sha256) {
      throw new Error(`backup validation failed for ${file.name}`);
    }
  }
  return manifest;
}

/** Create one consistent SQLite-plus-managed-object backup while the server may be running. */
export async function createManagedBackup(options: CreateManagedBackupOptions): Promise<ManagedBackupManifest> {
  const databasePath = resolve(options.databasePath);
  const managedRoot = resolve(`${databasePath}.checkpoints`);
  const output = resolve(options.outputDirectory);
  const maxAttempts = options.maxAttempts ?? 3;
  if (!Number.isInteger(maxAttempts) || maxAttempts < 1 || maxAttempts > 10) {
    throw new RangeError('backup maxAttempts must be an integer from 1 through 10');
  }
  if (!existsSync(databasePath)) throw new Error(`database does not exist: ${databasePath}`);
  if (existsSync(output)) throw new Error(`backup output already exists: ${output}`);
  mkdirSync(dirname(output), { recursive: true });

  for (let attempt = 1; attempt <= maxAttempts; attempt++) {
    const temporary = `${output}.partial-${process.pid}-${attempt}`;
    rmSync(temporary, { recursive: true, force: true });
    mkdirSync(join(temporary, BACKUP_MANAGED_DIRECTORY), { recursive: true });
    try {
      const snapshot = join(temporary, BACKUP_DATABASE_NAME);
      const source = new Database(databasePath, { readonly: true, fileMustExist: true });
      try { await source.backup(snapshot); }
      finally { source.close(); }

      const inventory = readSnapshotInventory(snapshot);
      if (inventory.length > 0) {
        const rootStats = lstatSync(managedRoot);
        if (!rootStats.isDirectory() || rootStats.isSymbolicLink()) {
          throw new BackupRaceError('managed checkpoint root is unavailable during backup');
        }
      }
      const managedFiles: ManagedBackupFile[] = [];
      for (const entry of inventory) {
        managedFiles.push(await copyManagedFile(
          managedRoot,
          join(temporary, BACKUP_MANAGED_DIRECTORY),
          entry
        ));
      }
      flushAndCloseFile(snapshot);
      const database = {
        name: BACKUP_DATABASE_NAME,
        bytes: statSync(snapshot).size,
        sha256: await hashFile(snapshot)
      };
      const manifest: ManagedBackupManifest = {
        format: MANAGED_BACKUP_FORMAT,
        createdAt: new Date().toISOString(),
        database,
        managedFiles
      };
      const manifestPath = join(temporary, MANIFEST_NAME);
      writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`, { flag: 'wx' });
      flushAndCloseFile(manifestPath);
      await validateManagedBackup(temporary);
      renameSync(temporary, output);
      return manifest;
    } catch (error) {
      rmSync(temporary, { recursive: true, force: true });
      if (error instanceof BackupRaceError && attempt < maxAttempts) {
        await delay(100 * attempt);
        continue;
      }
      throw error;
    }
  }
  throw new Error('backup did not complete');
}

/** Restore a validated backup set to a new, absent database and managed-directory pair. */
export async function restoreManagedBackup(options: RestoreManagedBackupOptions): Promise<ManagedBackupManifest> {
  const backup = resolve(options.backupDirectory);
  const databasePath = resolve(options.databasePath);
  const managedRoot = resolve(`${databasePath}.checkpoints`);
  if (existsSync(databasePath) || existsSync(managedRoot)) {
    throw new Error('restore target database and managed directory must both be absent');
  }
  const manifest = await validateManagedBackup(backup);
  mkdirSync(dirname(databasePath), { recursive: true });
  const temporaryDatabase = `${databasePath}.restore-${process.pid}`;
  const temporaryManaged = `${managedRoot}.restore-${process.pid}`;
  rmSync(temporaryDatabase, { force: true });
  rmSync(temporaryManaged, { recursive: true, force: true });
  mkdirSync(temporaryManaged);
  try {
    copyFileSync(join(backup, manifest.database.name), temporaryDatabase, constants.COPYFILE_EXCL);
    flushAndCloseFile(temporaryDatabase);
    for (const file of manifest.managedFiles) {
      const target = join(temporaryManaged, file.name);
      copyFileSync(join(backup, BACKUP_MANAGED_DIRECTORY, file.name), target, constants.COPYFILE_EXCL);
      flushAndCloseFile(target);
    }
    renameSync(temporaryManaged, managedRoot);
    renameSync(temporaryDatabase, databasePath);
    return manifest;
  } catch (error) {
    rmSync(temporaryDatabase, { force: true });
    if (!existsSync(managedRoot)) rmSync(temporaryManaged, { recursive: true, force: true });
    throw error;
  }
}
