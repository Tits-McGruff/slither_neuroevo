/** Schema setup on the existing worker-owned SQLite connection. */
import type Database from 'better-sqlite3';
/** Add owner pin classification and backfill every Stage 3/6A file as automatic. */
export function initializeRetentionSchema(database: ReturnType<typeof Database>): void {
  database.transaction(() => {
    const existing = database.prepare(`SELECT sql FROM sqlite_schema
      WHERE type = 'table' AND name = 'rust_checkpoint_retention_v1'`).get() as { sql: string | null } | undefined;
    if (existing && (!existing.sql?.includes("'pruning'") || !existing.sql.includes("'pruned'"))) {
      database.exec(`
        ALTER TABLE rust_checkpoint_retention_v1 RENAME TO rust_checkpoint_retention_v1_old;
        CREATE TABLE rust_checkpoint_retention_v1 (
          checkpoint_id TEXT PRIMARY KEY NOT NULL REFERENCES rust_checkpoint_v3_metadata(checkpoint_id),
          retention_kind TEXT NOT NULL CHECK(retention_kind IN ('automatic', 'pinned', 'pruning', 'pruned')),
          classified_at_ms INTEGER NOT NULL
        );
        INSERT INTO rust_checkpoint_retention_v1 SELECT * FROM rust_checkpoint_retention_v1_old;
        DROP TABLE rust_checkpoint_retention_v1_old;
      `);
    }
    database.exec(`
      CREATE TABLE IF NOT EXISTS rust_checkpoint_retention_v1 (
        checkpoint_id TEXT PRIMARY KEY NOT NULL REFERENCES rust_checkpoint_v3_metadata(checkpoint_id),
        retention_kind TEXT NOT NULL CHECK(retention_kind IN ('automatic', 'pinned', 'pruning', 'pruned')),
        classified_at_ms INTEGER NOT NULL
      );
    `);
    database.prepare(`INSERT OR IGNORE INTO rust_checkpoint_retention_v1 (
      checkpoint_id, retention_kind, classified_at_ms
    ) SELECT checkpoint_id, 'automatic', created_at_ms FROM rust_checkpoint_v3_metadata
    `).run();
  }).immediate();
}

/** Add bounded recovery provenance without rewriting any existing checkpoint/history rows. */
export function initializeRecoverySchema(database: ReturnType<typeof Database>): void {
  database.transaction(() => database.exec(`
    CREATE TABLE IF NOT EXISTS rust_recovery_branches_v1 (
      branch_run_id TEXT PRIMARY KEY NOT NULL,
      operation_id TEXT UNIQUE NOT NULL,
      source_run_id TEXT NOT NULL,
      recovered_checkpoint_id TEXT NOT NULL REFERENCES rust_checkpoint_v3_metadata(checkpoint_id),
      history_through_generation_hex TEXT NOT NULL,
      provenance_json TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS rust_import_branches_v1 (
      branch_run_id TEXT PRIMARY KEY NOT NULL,
      operation_id TEXT UNIQUE NOT NULL,
      source_run_id TEXT NOT NULL,
      recovered_checkpoint_id TEXT NOT NULL REFERENCES rust_checkpoint_v3_metadata(checkpoint_id),
      history_through_generation_hex TEXT NOT NULL,
      provenance_json TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS rust_active_run_v1 (
      singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
      run_id TEXT NOT NULL REFERENCES rust_checkpoint_v3_current(run_id)
    );
  `)).immediate();
}

/**
 * Create the minimal checkpoint metadata, compact history, and per-run pointer tables.
 * @param database - Worker-owned synchronous SQLite connection.
 */
export function initializeSchema(database: ReturnType<typeof Database>): void {
  database.exec(`
    CREATE TABLE IF NOT EXISTS rust_checkpoint_v3_metadata (
      checkpoint_id TEXT PRIMARY KEY NOT NULL,
      operation_id TEXT NOT NULL UNIQUE,
      run_id TEXT NOT NULL,
      transition_epoch TEXT NOT NULL,
      generation_hex TEXT NOT NULL,
      completed_step_hex TEXT NOT NULL,
      boundary_kind TEXT NOT NULL,
      checkpoint_format_version_hex TEXT NOT NULL,
      state_version_hex TEXT NOT NULL,
      graph_layout_version_hex TEXT NOT NULL,
      managed_root TEXT NOT NULL,
      relative_filename TEXT NOT NULL UNIQUE,
      logical_root_sha256 TEXT NOT NULL UNIQUE,
      stored_byte_count_hex TEXT NOT NULL,
      decoded_byte_count_hex TEXT NOT NULL,
      role_count_hex TEXT NOT NULL,
      population_count_hex TEXT NOT NULL,
      weight_count_hex TEXT NOT NULL,
      recurrent_state_count_hex TEXT NOT NULL,
      weights_encoding TEXT NOT NULL,
      recurrent_state_encoding TEXT NOT NULL,
      graph_layout_sha256 TEXT NOT NULL,
      write_validation_policy TEXT NOT NULL,
      descriptor_json TEXT NOT NULL,
      created_at_ms INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS rust_checkpoint_v3_current (
      run_id TEXT PRIMARY KEY NOT NULL,
      checkpoint_id TEXT NOT NULL REFERENCES rust_checkpoint_v3_metadata(checkpoint_id),
      transition_epoch TEXT NOT NULL,
      operation_id TEXT NOT NULL UNIQUE
    );
    CREATE TABLE IF NOT EXISTS rust_generation_history_v1 (
      run_id TEXT NOT NULL,
      generation_hex TEXT NOT NULL,
      checkpoint_id TEXT REFERENCES rust_checkpoint_v3_metadata(checkpoint_id),
      record_version INTEGER NOT NULL,
      record_blob BLOB NOT NULL CHECK(length(record_blob) = 56),
      created_at_ms INTEGER NOT NULL,
      PRIMARY KEY (run_id, generation_hex)
    );
    CREATE TABLE IF NOT EXISTS rust_generation_completion_v1 (
      run_id TEXT NOT NULL,
      generation_hex TEXT NOT NULL,
      checkpoint_id TEXT REFERENCES rust_checkpoint_v3_metadata(checkpoint_id) ON DELETE SET NULL,
      record_blob BLOB NOT NULL CHECK(length(record_blob) = 32),
      PRIMARY KEY (run_id, generation_hex)
    );
    CREATE TABLE IF NOT EXISTS rust_generation_work_v1 (
      run_id TEXT NOT NULL,
      generation_hex TEXT NOT NULL,
      checkpoint_id TEXT REFERENCES rust_checkpoint_v3_metadata(checkpoint_id) ON DELETE SET NULL,
      record_json TEXT NOT NULL CHECK(length(record_json) <= 8192),
      PRIMARY KEY (run_id, generation_hex)
    );
    CREATE TABLE IF NOT EXISTS rust_generation_experiment_v1 (
      run_id TEXT NOT NULL,
      generation_hex TEXT NOT NULL,
      checkpoint_id TEXT REFERENCES rust_checkpoint_v3_metadata(checkpoint_id) ON DELETE SET NULL,
      record_blob BLOB NOT NULL CHECK(length(record_blob) = 768),
      PRIMARY KEY (run_id, generation_hex)
    );
    CREATE TABLE IF NOT EXISTS rust_hall_of_fame_weights_v1 (
      logical_sha256 TEXT PRIMARY KEY NOT NULL,
      relative_filename TEXT NOT NULL UNIQUE,
      encoding TEXT NOT NULL,
      stored_byte_count_hex TEXT NOT NULL,
      decoded_byte_count_hex TEXT NOT NULL,
      weight_count_hex TEXT NOT NULL,
      created_at_ms INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS rust_hall_of_fame_v1 (
      run_id TEXT NOT NULL,
      generation_hex TEXT NOT NULL,
      checkpoint_id TEXT REFERENCES rust_checkpoint_v3_metadata(checkpoint_id),
      record_version INTEGER NOT NULL,
      record_blob BLOB NOT NULL CHECK(length(record_blob) = 56),
      weights_sha256 TEXT REFERENCES rust_hall_of_fame_weights_v1(logical_sha256),
      genome_sha256 TEXT,
      fitness_value REAL NOT NULL,
      pinned INTEGER NOT NULL DEFAULT 0 CHECK(pinned IN (0, 1)),
      weight_state TEXT NOT NULL CHECK(weight_state IN ('selected', 'unselected', 'legacy')),
      created_at_ms INTEGER NOT NULL,
      PRIMARY KEY (run_id, generation_hex)
    );
  `);
}

/** Add the content-addressed winner-weight store without rewriting older Hall-of-Fame rows. */
export function initializeHallOfFameWeightsSchema(
  database: ReturnType<typeof Database>,
  rebuildHallOfFameRetention: (runId: string) => void
): void {
  database.transaction(() => {
    database.exec(`
      CREATE TABLE IF NOT EXISTS rust_hall_of_fame_weights_v1 (
        logical_sha256 TEXT PRIMARY KEY NOT NULL,
        relative_filename TEXT NOT NULL UNIQUE,
        encoding TEXT NOT NULL,
        stored_byte_count_hex TEXT NOT NULL,
        decoded_byte_count_hex TEXT NOT NULL,
        weight_count_hex TEXT NOT NULL,
        created_at_ms INTEGER NOT NULL
      )
    `);
    let columns = database.prepare('PRAGMA table_info(rust_hall_of_fame_v1)').all() as Array<{ name: string }>;
    if (!columns.some(column => column.name === 'weights_sha256')) {
      database.exec(`ALTER TABLE rust_hall_of_fame_v1 ADD COLUMN weights_sha256 TEXT
        REFERENCES rust_hall_of_fame_weights_v1(logical_sha256)`);
    }
    if (!columns.some(column => column.name === 'fitness_value')) {
      database.exec('ALTER TABLE rust_hall_of_fame_v1 ADD COLUMN fitness_value REAL');
    }
    if (!columns.some(column => column.name === 'genome_sha256')) {
      database.exec('ALTER TABLE rust_hall_of_fame_v1 ADD COLUMN genome_sha256 TEXT');
    }
    if (!columns.some(column => column.name === 'pinned')) {
      database.exec(`ALTER TABLE rust_hall_of_fame_v1 ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0
        CHECK(pinned IN (0, 1))`);
    }
    if (!columns.some(column => column.name === 'weight_state')) {
      database.exec(`ALTER TABLE rust_hall_of_fame_v1 ADD COLUMN weight_state TEXT
        CHECK(weight_state IN ('selected', 'unselected', 'legacy'))`);
    }
    columns = database.prepare('PRAGMA table_info(rust_hall_of_fame_v1)').all() as Array<{ name: string }>;
    if (!['weights_sha256', 'genome_sha256', 'fitness_value', 'pinned', 'weight_state']
      .every(name => columns.some(column => column.name === name))) {
      throw new Error('Hall-of-Fame schema migration did not create every retention field');
    }
    type MigrationRow = {
      run_id: string; generation_hex: string; record_blob: Buffer; weights_sha256: string | null;
      genome_sha256: string | null; fitness_value: number | null; weight_state: string | null;
    };
    const firstPage = database.prepare(`SELECT run_id, generation_hex, record_blob, weights_sha256,
      genome_sha256, fitness_value, weight_state FROM rust_hall_of_fame_v1
      ORDER BY run_id, generation_hex LIMIT 256`);
    const nextPage = database.prepare(`SELECT run_id, generation_hex, record_blob, weights_sha256,
      genome_sha256, fitness_value, weight_state FROM rust_hall_of_fame_v1
      WHERE run_id > ? OR (run_id = ? AND generation_hex > ?)
      ORDER BY run_id, generation_hex LIMIT 256`);
    const update = database.prepare(`UPDATE rust_hall_of_fame_v1
      SET genome_sha256 = ?, fitness_value = ?, weight_state = ?
      WHERE run_id = ? AND generation_hex = ?`);
    let rows = firstPage.all() as MigrationRow[];
    while (rows.length > 0) {
      for (const row of rows) {
        if (!Buffer.isBuffer(row.record_blob) || row.record_blob.length !== 56) {
          throw new Error('Hall-of-Fame migration found a malformed compact record');
        }
        const fitness = row.record_blob.readDoubleLE(32);
        if (!Number.isFinite(fitness)) throw new Error('Hall-of-Fame migration found non-finite fitness');
        const weightState = row.weight_state ?? (row.weights_sha256 === null ? 'legacy' : 'selected');
        if (!['selected', 'unselected', 'legacy'].includes(weightState)) {
          throw new Error('Hall-of-Fame migration found an invalid weight state');
        }
        const genomeSha256 = row.genome_sha256 ?? row.weights_sha256;
        if (weightState !== 'legacy' && genomeSha256 === null) {
          throw new Error('Hall-of-Fame migration found a retained row without genome identity');
        }
        if (row.genome_sha256 !== genomeSha256 || row.fitness_value !== fitness ||
            row.weight_state !== weightState) {
          update.run(genomeSha256, fitness, weightState, row.run_id, row.generation_hex);
        }
      }
      const last = rows[rows.length - 1]!;
      rows = nextPage.all(last.run_id, last.run_id, last.generation_hex) as MigrationRow[];
    }
    ensureHallOfFameIndexes(database);
    const runs = database.prepare('SELECT DISTINCT run_id FROM rust_hall_of_fame_v1')
      .all() as Array<{ run_id: string }>;
    for (const run of runs) rebuildHallOfFameRetention(run.run_id);
  }).immediate();
}

/** Restore selection and reference indexes after either Hall-of-Fame schema path. */
function ensureHallOfFameIndexes(database: ReturnType<typeof Database>): void {
  database.exec(`
    CREATE INDEX IF NOT EXISTS rust_hof_genome_rank_v1
      ON rust_hall_of_fame_v1(run_id, genome_sha256, pinned, fitness_value DESC, generation_hex);
    CREATE INDEX IF NOT EXISTS rust_hof_selected_v1
      ON rust_hall_of_fame_v1(run_id, weight_state, pinned, generation_hex);
    CREATE INDEX IF NOT EXISTS rust_hof_checkpoint_state_v1
      ON rust_hall_of_fame_v1(checkpoint_id, weight_state);
    CREATE INDEX IF NOT EXISTS rust_hof_weights_reference_v1
      ON rust_hall_of_fame_v1(weights_sha256);
  `);
}

/** Permit imported history to retain its exact keys without inventing one
 * managed checkpoint reference per historical generation. */
export function initializeImportableHistorySchema(database: ReturnType<typeof Database>): void {
  database.transaction(() => {
    const history = database.prepare(`SELECT sql FROM sqlite_schema
      WHERE type = 'table' AND name = 'rust_generation_history_v1'`).get() as { sql: string };
    if (/checkpoint_id\s+TEXT\s+NOT NULL\s+UNIQUE/iu.test(history.sql)) {
      database.exec(`
        ALTER TABLE rust_generation_history_v1 RENAME TO rust_generation_history_v1_old;
        CREATE TABLE rust_generation_history_v1 (
          run_id TEXT NOT NULL,
          generation_hex TEXT NOT NULL,
          checkpoint_id TEXT REFERENCES rust_checkpoint_v3_metadata(checkpoint_id),
          record_version INTEGER NOT NULL,
          record_blob BLOB NOT NULL CHECK(length(record_blob) = 56),
          created_at_ms INTEGER NOT NULL,
          PRIMARY KEY (run_id, generation_hex)
        );
        INSERT INTO rust_generation_history_v1
          (run_id, generation_hex, checkpoint_id, record_version, record_blob, created_at_ms)
        SELECT run_id, generation_hex, checkpoint_id, record_version, record_blob, created_at_ms
          FROM rust_generation_history_v1_old;
        DROP TABLE rust_generation_history_v1_old;
      `);
    }
    const hall = database.prepare(`SELECT sql FROM sqlite_schema
      WHERE type = 'table' AND name = 'rust_hall_of_fame_v1'`).get() as { sql: string };
    if (/checkpoint_id\s+TEXT\s+NOT NULL\s+UNIQUE/iu.test(hall.sql)) {
      database.exec(`
        ALTER TABLE rust_hall_of_fame_v1 RENAME TO rust_hall_of_fame_v1_old;
        CREATE TABLE rust_hall_of_fame_v1 (
          run_id TEXT NOT NULL,
          generation_hex TEXT NOT NULL,
          checkpoint_id TEXT REFERENCES rust_checkpoint_v3_metadata(checkpoint_id),
          record_version INTEGER NOT NULL,
          record_blob BLOB NOT NULL CHECK(length(record_blob) = 56),
          weights_sha256 TEXT REFERENCES rust_hall_of_fame_weights_v1(logical_sha256),
          genome_sha256 TEXT,
          fitness_value REAL NOT NULL,
          pinned INTEGER NOT NULL DEFAULT 0 CHECK(pinned IN (0, 1)),
          weight_state TEXT NOT NULL CHECK(weight_state IN ('selected', 'unselected', 'legacy')),
          created_at_ms INTEGER NOT NULL,
          PRIMARY KEY (run_id, generation_hex)
        );
        INSERT INTO rust_hall_of_fame_v1
          (run_id, generation_hex, checkpoint_id, record_version, record_blob,
           weights_sha256, genome_sha256, fitness_value, pinned, weight_state, created_at_ms)
        SELECT run_id, generation_hex, checkpoint_id, record_version, record_blob,
          weights_sha256, genome_sha256, fitness_value, pinned, weight_state, created_at_ms
          FROM rust_hall_of_fame_v1_old;
        DROP TABLE rust_hall_of_fame_v1_old;
      `);
    }
    ensureHallOfFameIndexes(database);
  }).immediate();
}

/** Preserve conversion provenance independently of expiring prior-run resume pointers. */
export function initializeLegacyConversionSchema(database: ReturnType<typeof Database>): void {
  const createTable = `
    CREATE TABLE IF NOT EXISTS rust_legacy_conversions_v1 (
      run_id TEXT PRIMARY KEY NOT NULL,
      source_snapshot_id INTEGER NOT NULL CHECK(source_snapshot_id > 0),
      source_format TEXT NOT NULL CHECK(source_format IN ('typescript-v2', 'legacy-gzip', 'legacy-json')),
      completeness TEXT NOT NULL CHECK(completeness = 'population-only'),
      created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0)
    );
  `;
  database.transaction(() => {
    const foreignKeys = database.prepare('PRAGMA foreign_key_list(rust_legacy_conversions_v1)').all() as
      Array<{ table: string; from: string; to: string }>;
    if (foreignKeys.some(key => key.table === 'rust_checkpoint_v3_current' && key.from === 'run_id' && key.to === 'run_id')) {
      // Conversion history has the same lifetime as compact checkpoint metadata,
      // while retention intentionally detaches obsolete current pointers.
      database.exec('ALTER TABLE rust_legacy_conversions_v1 RENAME TO rust_legacy_conversions_v1_old');
      database.exec(createTable);
      database.exec(`INSERT INTO rust_legacy_conversions_v1 (
        run_id, source_snapshot_id, source_format, completeness, created_at_ms
      ) SELECT run_id, source_snapshot_id, source_format, completeness, created_at_ms
        FROM rust_legacy_conversions_v1_old;
        DROP TABLE rust_legacy_conversions_v1_old;`);
    } else {
      database.exec(createTable);
    }
  }).immediate();
}

/** Add the small legacy-compatible graph-preset table to dedicated Rust databases. */
export function initializeGraphPresetSchema(database: ReturnType<typeof Database>): void {
  database.exec(`
    CREATE TABLE IF NOT EXISTS graph_presets (
      id INTEGER PRIMARY KEY,
      created_at INTEGER NOT NULL,
      name TEXT NOT NULL,
      spec_json TEXT NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_graph_presets_name ON graph_presets(name);
  `);
  database.prepare('SELECT id, created_at, name, spec_json FROM graph_presets LIMIT 0').all();
}
