import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import Database from 'better-sqlite3';
import { describe, expect, it } from 'vitest';
import { validateCheckpointDatabaseSchema } from './checkpointDatabaseSchema.ts';

function createOriginalManagedSchema(database: Database.Database): void {
  database.exec(`
    CREATE TABLE rust_checkpoint_v3_metadata (
      checkpoint_id TEXT PRIMARY KEY NOT NULL,
      operation_id TEXT NOT NULL UNIQUE,
      run_id TEXT NOT NULL,
      transition_epoch TEXT NOT NULL,
      generation_hex TEXT NOT NULL,
      completed_step_hex TEXT NOT NULL,
      descriptor_json TEXT NOT NULL
    );
    CREATE TABLE rust_checkpoint_v3_current (
      run_id TEXT PRIMARY KEY NOT NULL,
      checkpoint_id TEXT NOT NULL,
      transition_epoch TEXT NOT NULL,
      operation_id TEXT NOT NULL
    );
    CREATE TABLE rust_generation_history_v1 (
      run_id TEXT NOT NULL,
      generation_hex TEXT NOT NULL,
      checkpoint_id TEXT,
      record_version INTEGER NOT NULL,
      record_blob BLOB NOT NULL,
      created_at_ms INTEGER NOT NULL,
      PRIMARY KEY (run_id, generation_hex)
    );
    CREATE TABLE rust_hall_of_fame_v1 (
      run_id TEXT NOT NULL,
      generation_hex TEXT NOT NULL,
      checkpoint_id TEXT,
      record_version INTEGER NOT NULL,
      record_blob BLOB NOT NULL,
      created_at_ms INTEGER NOT NULL,
      PRIMARY KEY (run_id, generation_hex)
    );
  `);
}

describe('validateCheckpointDatabaseSchema', () => {
  it('upgrades missing managed generation sidecars only on a writable connection', () => {
    const directory = mkdtempSync(join(tmpdir(), 'slither-checkpoint-schema-'));
    const databasePath = join(directory, 'managed.db');
    try {
      const seed = new Database(databasePath);
      try {
        createOriginalManagedSchema(seed);
      } finally {
        seed.close();
      }

      const readonly = new Database(databasePath, { readonly: true, fileMustExist: true });
      try {
        expect(validateCheckpointDatabaseSchema(readonly)).toBe('managed');
        expect(readonly.prepare(`SELECT name FROM sqlite_schema
          WHERE type = 'table' AND name IN ('rust_generation_completion_v1', 'rust_generation_experiment_v1')
          ORDER BY name`).all()).toEqual([]);
      } finally {
        readonly.close();
      }

      const writable = new Database(databasePath, { fileMustExist: true });
      try {
        expect(validateCheckpointDatabaseSchema(writable)).toBe('managed');
        expect(writable.prepare(`SELECT name FROM sqlite_schema
          WHERE type = 'table' AND name IN ('rust_generation_completion_v1', 'rust_generation_experiment_v1')
          ORDER BY name`).all()).toEqual([
          { name: 'rust_generation_completion_v1' },
          { name: 'rust_generation_experiment_v1' }
        ]);
        expect(() => writable.prepare(`INSERT INTO rust_generation_completion_v1
          (run_id, generation_hex, checkpoint_id, record_blob) VALUES (?, ?, NULL, ?)`)
          .run('run', '0000000000000001', Buffer.alloc(31))).toThrow();
        expect(() => writable.prepare(`INSERT INTO rust_generation_experiment_v1
          (run_id, generation_hex, checkpoint_id, record_blob) VALUES (?, ?, NULL, ?)`)
          .run('run', '0000000000000001', Buffer.alloc(767))).toThrow();
      } finally {
        writable.close();
      }
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  });

  it('rejects an incompatible pre-existing sidecar instead of hiding it behind IF NOT EXISTS', () => {
    const database = new Database(':memory:');
    try {
      createOriginalManagedSchema(database);
      database.exec('CREATE TABLE rust_generation_experiment_v1 (run_id TEXT NOT NULL)');
      expect(() => validateCheckpointDatabaseSchema(database)).toThrow(/generation_hex|no such column/u);
      expect(database.prepare(`SELECT count(*) AS count FROM sqlite_schema
        WHERE type = 'table' AND name = 'rust_generation_completion_v1'`).get()).toEqual({ count: 0 });
    } finally {
      database.close();
    }
  });
});
