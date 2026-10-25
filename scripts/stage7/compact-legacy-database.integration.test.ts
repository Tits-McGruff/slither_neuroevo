/** Real SQLite backup and offline VACUUM contract for legacy storage reclamation. */

import { mkdtemp, rm, stat } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve, sep } from 'node:path';
import Database from 'better-sqlite3';
import { describe, expect, it } from 'vitest';
import { validateManagedBackup } from '../managedBackup.ts';
import { compactLegacyDatabase } from './compact-legacy-database.ts';

/** Ensure cleanup cannot escape the specifically created temporary fixture. */
async function removeFixture(path: string): Promise<void> {
  const root = resolve(tmpdir()) + sep;
  if (!resolve(path).startsWith(root) || !path.includes('slither-legacy-compact-')) {
    throw new Error('refusing to remove an unexpected compaction fixture');
  }
  await rm(path, { recursive: true, force: true });
}

describe('offline legacy database compaction', () => {
  it('retains a verified pre-vacuum backup and reclaims deleted BLOB pages', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-legacy-compact-'));
    const databasePath = join(root, 'legacy.sqlite');
    const backupDirectory = join(root, 'backup');
    try {
      const db = new Database(databasePath);
      try {
        db.pragma('journal_mode = WAL');
        db.exec('CREATE TABLE legacy_snapshot (id INTEGER PRIMARY KEY, genome BLOB NOT NULL)');
        const insert = db.prepare('INSERT INTO legacy_snapshot (genome) VALUES (?)');
        const payload = Buffer.alloc(256 * 1024, 0x5a);
        db.transaction(() => {
          for (let index = 0; index < 24; index++) insert.run(payload);
        })();
        db.prepare('DELETE FROM legacy_snapshot WHERE id < 24').run();
        expect(db.pragma('freelist_count', { simple: true })).toBeGreaterThan(0);
      } finally { db.close(); }
      const result = await compactLegacyDatabase({ databasePath, backupDirectory });
      const backup = await validateManagedBackup(backupDirectory);
      const retained = new Database(databasePath, { readonly: true });
      try {
        expect(retained.prepare('SELECT count(*) AS count FROM legacy_snapshot').get()).toEqual({ count: 1 });
        expect(retained.pragma('quick_check', { simple: true })).toBe('ok');
      } finally { retained.close(); }
      expect(result).toMatchObject({ backedUpManagedFiles: 0,
        before: { freelistPages: expect.any(Number) }, after: { freelistPages: 0 } });
      const before = result['before'] as { databaseBytes: number; walBytes: number; shmBytes: number };
      const after = result['after'] as { databaseBytes: number; walBytes: number; shmBytes: number };
      expect(after.databaseBytes + after.walBytes + after.shmBytes)
        .toBeLessThan(before.databaseBytes + before.walBytes + before.shmBytes);
      expect((await stat(databasePath)).size).toBeLessThan(backup.database.bytes);
    } finally { await removeFixture(root); }
  }, 30_000);
});
