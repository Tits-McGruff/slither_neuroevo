import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, unlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import Database from 'better-sqlite3';
import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  createManagedBackup,
  restoreManagedBackup,
  validateManagedBackup
} from './managedBackup.ts';

/** Temporary roots created by this test file. */
const temporaryRoots: string[] = [];

/** Allocate one isolated filesystem root. */
function temporaryRoot(): string {
  const root = mkdtempSync(join(tmpdir(), 'slither-backup-'));
  temporaryRoots.push(root);
  return root;
}

/** Convert one safe byte count to the persistence schema's fixed-width hexadecimal form. */
function byteCount(value: number): string {
  return value.toString(16).padStart(16, '0');
}

afterEach(() => {
  for (const root of temporaryRoots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe('managed production backup', () => {
  it('selects a retained prior-run current only in the restored copy', async () => {
    const root = temporaryRoot();
    const databasePath = join(root, 'source.db');
    const backupRoot = join(root, 'backup');
    const restoredPath = join(root, 'restored.db');
    const olderId = '1'.repeat(64);
    const activeId = '2'.repeat(64);
    const db = new Database(databasePath);
    try {
      db.exec(`CREATE TABLE rust_checkpoint_v3_metadata (
          checkpoint_id TEXT PRIMARY KEY, run_id TEXT NOT NULL,
          relative_filename TEXT NOT NULL, stored_byte_count_hex TEXT NOT NULL
        );
        CREATE TABLE rust_checkpoint_retention_v1 (
          checkpoint_id TEXT PRIMARY KEY, retention_kind TEXT NOT NULL
        );
        CREATE TABLE rust_checkpoint_v3_current (run_id TEXT PRIMARY KEY, checkpoint_id TEXT NOT NULL);
        CREATE TABLE rust_active_run_v1 (singleton INTEGER PRIMARY KEY, run_id TEXT NOT NULL);
        INSERT INTO rust_active_run_v1 VALUES (1, 'active');`);
      mkdirSync(`${databasePath}.checkpoints`);
      for (const [id, runId] of [[olderId, 'older'], [activeId, 'active']]) {
        const filename = `${id}.checkpoint-v3`;
        db.prepare('INSERT INTO rust_checkpoint_v3_metadata VALUES (?, ?, ?, ?)')
          .run(id, runId, filename, byteCount(4));
        db.prepare('INSERT INTO rust_checkpoint_retention_v1 VALUES (?, ?)').run(id, 'automatic');
        db.prepare('INSERT INTO rust_checkpoint_v3_current VALUES (?, ?)').run(runId, id);
        writeFileSync(join(`${databasePath}.checkpoints`, filename), Buffer.from('data'));
      }
    } finally { db.close(); }
    await createManagedBackup({ databasePath, outputDirectory: backupRoot });
    await restoreManagedBackup({ backupDirectory: backupRoot, databasePath: restoredPath,
      checkpointId: olderId });
    const source = new Database(databasePath, { readonly: true });
    const restored = new Database(restoredPath, { readonly: true });
    try {
      expect(source.prepare('SELECT run_id FROM rust_active_run_v1 WHERE singleton = 1').get())
        .toEqual({ run_id: 'active' });
      expect(restored.prepare('SELECT run_id FROM rust_active_run_v1 WHERE singleton = 1').get())
        .toEqual({ run_id: 'older' });
    } finally { source.close(); restored.close(); }
    const rejectedPath = join(root, 'rejected.db');
    await expect(restoreManagedBackup({ backupDirectory: backupRoot, databasePath: rejectedPath,
      checkpointId: '3'.repeat(64) })).rejects.toThrow(/not a backed-up retained per-run current/u);
    expect(existsSync(rejectedPath)).toBe(false);
  });

  it('retries from a new SQLite snapshot when pruning removes a selected file', async () => {
    const root = temporaryRoot();
    const databasePath = join(root, 'source.db');
    const managedRoot = `${databasePath}.checkpoints`;
    const backupRoot = join(root, 'backup');
    const restoredPath = join(root, 'restored.db');
    const retainedName = `${'1'.repeat(64)}.checkpoint-v3`;
    const pruningName = `${'2'.repeat(64)}.checkpoint-v3`;
    const database = new Database(databasePath);
    database.pragma('journal_mode = WAL');
    database.exec(`
      CREATE TABLE rust_checkpoint_v3_metadata (
        checkpoint_id TEXT PRIMARY KEY,
        relative_filename TEXT NOT NULL,
        stored_byte_count_hex TEXT NOT NULL
      );
      CREATE TABLE rust_checkpoint_retention_v1 (
        checkpoint_id TEXT PRIMARY KEY,
        retention_kind TEXT NOT NULL
      );
    `);
    mkdirSync(managedRoot);
    for (const [id, name] of [['retained', retainedName], ['pruning', pruningName]]) {
      database.prepare('INSERT INTO rust_checkpoint_v3_metadata VALUES (?, ?, ?)')
        .run(id, name, byteCount(4));
      database.prepare('INSERT INTO rust_checkpoint_retention_v1 VALUES (?, ?)')
        .run(id, 'automatic');
      writeFileSync(join(managedRoot, name), Buffer.from('data'));
    }

    const originalBackup = Database.prototype.backup;
    let snapshots = 0;
    const backupSpy = vi.spyOn(Database.prototype, 'backup').mockImplementation(async function(destinationFile: string) {
      const result = await originalBackup.call(this, destinationFile);
      snapshots++;
      if (snapshots === 1) {
        database.prepare('UPDATE rust_checkpoint_retention_v1 SET retention_kind = ? WHERE checkpoint_id = ?')
          .run('pruned', 'pruning');
        unlinkSync(join(managedRoot, pruningName));
      }
      return result;
    });
    try {
      const manifest = await createManagedBackup({ databasePath, outputDirectory: backupRoot });
      expect(snapshots).toBe(2);
      expect(manifest.managedFiles.map(file => file.name)).toEqual([retainedName]);
      expect((await validateManagedBackup(backupRoot)).managedFiles).toEqual(manifest.managedFiles);
      expect(readdirSync(root).filter(name => name.startsWith('backup.partial-'))).toEqual([]);
      await restoreManagedBackup({ backupDirectory: backupRoot, databasePath: restoredPath });
      expect(readFileSync(join(`${restoredPath}.checkpoints`, retainedName))).toEqual(Buffer.from('data'));
      expect(existsSync(join(`${restoredPath}.checkpoints`, pruningName))).toBe(false);
      const restored = new Database(restoredPath, { readonly: true });
      try {
        expect(restored.prepare('SELECT retention_kind FROM rust_checkpoint_retention_v1 WHERE checkpoint_id = ?')
          .pluck().get('pruning')).toBe('pruned');
      } finally { restored.close(); }

      unlinkSync(join(managedRoot, retainedName));
      const missingBackup = join(root, 'missing-backup');
      await expect(createManagedBackup({ databasePath, outputDirectory: missingBackup, maxAttempts: 2 }))
        .rejects.toThrow(/managed object disappeared during backup/u);
      expect(existsSync(missingBackup)).toBe(false);
      expect(readdirSync(root).filter(name => name.startsWith('missing-backup.partial-'))).toEqual([]);
    } finally {
      backupSpy.mockRestore();
      database.close();
    }
  });

  it('backs up a live SQLite snapshot with exactly its retained immutable files and restores it', async () => {
    const root = temporaryRoot();
    const databasePath = join(root, 'source.db');
    const managedRoot = `${databasePath}.checkpoints`;
    const backupRoot = join(root, 'backup');
    const restoredPath = join(root, 'restored.db');
    const automaticName = `${'1'.repeat(64)}.checkpoint-v3`;
    const prunedName = `${'2'.repeat(64)}.checkpoint-v3`;
    const hallName = `${'3'.repeat(64)}.hof-weights-v1`;
    const automaticBytes = Buffer.from('automatic-checkpoint');
    const hallBytes = Buffer.from('hall-of-fame-weights');
    const database = new Database(databasePath);
    database.pragma('journal_mode = WAL');
    database.exec(`
      CREATE TABLE rust_checkpoint_v3_metadata (
        checkpoint_id TEXT PRIMARY KEY,
        relative_filename TEXT NOT NULL,
        stored_byte_count_hex TEXT NOT NULL
      );
      CREATE TABLE rust_checkpoint_retention_v1 (
        checkpoint_id TEXT PRIMARY KEY,
        retention_kind TEXT NOT NULL
      );
      CREATE TABLE rust_hall_of_fame_weights_v1 (
        logical_sha256 TEXT PRIMARY KEY,
        relative_filename TEXT NOT NULL,
        stored_byte_count_hex TEXT NOT NULL
      );
      CREATE TABLE rust_hall_of_fame_v1 (weights_sha256 TEXT);
    `);
    database.prepare('INSERT INTO rust_checkpoint_v3_metadata VALUES (?, ?, ?)')
      .run('automatic', automaticName, byteCount(automaticBytes.length));
    database.prepare('INSERT INTO rust_checkpoint_retention_v1 VALUES (?, ?)')
      .run('automatic', 'automatic');
    database.prepare('INSERT INTO rust_checkpoint_v3_metadata VALUES (?, ?, ?)')
      .run('pruned', prunedName, byteCount(99));
    database.prepare('INSERT INTO rust_checkpoint_retention_v1 VALUES (?, ?)')
      .run('pruned', 'pruned');
    database.prepare('INSERT INTO rust_hall_of_fame_weights_v1 VALUES (?, ?, ?)')
      .run('hall', hallName, byteCount(hallBytes.length));
    database.prepare('INSERT INTO rust_hall_of_fame_v1 VALUES (?)').run('hall');
    rmSync(managedRoot, { recursive: true, force: true });
    mkdirSync(managedRoot);
    writeFileSync(join(managedRoot, automaticName), automaticBytes);
    writeFileSync(join(managedRoot, hallName), hallBytes);

    try {
      const manifest = await createManagedBackup({ databasePath, outputDirectory: backupRoot });
      expect(manifest.managedFiles.map(file => file.name)).toEqual([automaticName, hallName]);
      expect(await validateManagedBackup(backupRoot)).toEqual(manifest);
      await restoreManagedBackup({ backupDirectory: backupRoot, databasePath: restoredPath });
      expect(readFileSync(join(`${restoredPath}.checkpoints`, automaticName))).toEqual(automaticBytes);
      expect(readFileSync(join(`${restoredPath}.checkpoints`, hallName))).toEqual(hallBytes);
      const restored = new Database(restoredPath, { readonly: true });
      try {
        expect(restored.prepare(
          'SELECT retention_kind FROM rust_checkpoint_retention_v1 WHERE checkpoint_id = ?'
        ).pluck().get('pruned')).toBe('pruned');
      } finally {
        restored.close();
      }

      const manifestPath = join(backupRoot, 'manifest.json');
      const incomplete = { ...manifest, managedFiles: manifest.managedFiles.filter(file => file.name !== hallName) };
      writeFileSync(manifestPath, JSON.stringify(incomplete));
      await expect(validateManagedBackup(backupRoot))
        .rejects.toThrow('backup manifest does not match the SQLite managed-file inventory');
      writeFileSync(manifestPath, JSON.stringify(manifest));
      writeFileSync(join(backupRoot, 'managed', prunedName), Buffer.from('harmless orphan'));
      expect(await validateManagedBackup(backupRoot)).toEqual(manifest);

      writeFileSync(join(backupRoot, 'managed', automaticName), Buffer.from('corrupt'));
      await expect(validateManagedBackup(backupRoot)).rejects.toThrow('backup validation failed');
    } finally {
      database.close();
    }
  });
});
