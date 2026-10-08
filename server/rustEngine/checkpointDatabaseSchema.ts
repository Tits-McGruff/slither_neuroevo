import type Database from 'better-sqlite3';

/** Reject unrelated or incomplete databases before changing journal settings or schema. */
export function validateCheckpointDatabaseSchema(database: ReturnType<typeof Database>): 'managed' | 'legacy' {
  const rows = database.prepare(`SELECT name FROM sqlite_schema
    WHERE type = 'table' AND name NOT LIKE 'sqlite_%'`).all() as Array<{ name: string }>;
  const tables = new Set(rows.map(row => row.name));
  const managed = ['rust_checkpoint_v3_metadata', 'rust_checkpoint_v3_current',
    'rust_generation_history_v1', 'rust_hall_of_fame_v1'];
  if (managed.every(table => tables.has(table))) {
    // Preparing these fixed reads also rejects incompatible columns without DDL.
    database.prepare('SELECT checkpoint_id, operation_id, run_id, transition_epoch, generation_hex, completed_step_hex, descriptor_json FROM rust_checkpoint_v3_metadata LIMIT 0').all();
    database.prepare('SELECT run_id, checkpoint_id, transition_epoch, operation_id FROM rust_checkpoint_v3_current LIMIT 0').all();
    for (const table of ['rust_generation_history_v1', 'rust_hall_of_fame_v1']) {
      database.prepare(`SELECT run_id, generation_hex, checkpoint_id, record_version, record_blob, created_at_ms FROM ${table} LIMIT 0`).all();
    }
    return 'managed';
  }
  if (tables.has('population_snapshots')) {
    const parentColumns = new Set((database.prepare('PRAGMA table_info(population_snapshots)').all() as
      Array<{ name: string }>).map(column => column.name));
    if (!['id', 'payload_json'].every(column => parentColumns.has(column))) {
      throw new Error('legacy population_snapshots is missing required base columns');
    }
    if (tables.has('snapshot_genomes')) {
      database.prepare(`SELECT snapshot_id, slot, arch_key, brain_type, fitness, weight_count,
        weights_blob, weights_checksum FROM snapshot_genomes LIMIT 0`).all();
    }
    return 'legacy';
  }
  throw new Error('resume requires a managed or TypeScript v2 checkpoint database');
}

