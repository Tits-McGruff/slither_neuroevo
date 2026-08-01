/** Read-only Stage 2 inventory for current SQLite persistence artifacts. */

import { createHash } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import Database from 'better-sqlite3';

/** Maximum Hall-of-Fame rows described individually in one inventory artifact. */
const HALL_OF_FAME_DETAIL_LIMIT = 100;

/** Command-line options for the database inventory. */
interface DatabaseOptions {
  /** Existing database path. */
  databasePath: string;
  /** Optional JSON artifact destination. */
  outputPath: string | null;
}

/** SQLite table column returned by PRAGMA table_info. */
interface TableColumn {
  /** Column position. */
  cid: number;
  /** Column name. */
  name: string;
  /** Declared SQL type. */
  type: string;
  /** Non-null flag. */
  notnull: 0 | 1;
  /** Default expression. */
  dflt_value: unknown;
  /** Primary-key position. */
  pk: number;
}

/**
 * Parse command-line options.
 * @param argv - Arguments after script path.
 * @returns Validated paths.
 */
function parseOptions(argv: readonly string[]): DatabaseOptions {
  let databasePath = path.resolve('data', 'slither.db');
  let outputPath: string | null = null;
  for (let index = 0; index < argv.length; index++) {
    const option = argv[index];
    const value = argv[index + 1];
    switch (option) {
      case '--db':
        if (!value) throw new Error('--db requires a path');
        databasePath = path.resolve(value);
        index++;
        break;
      case '--output':
        if (!value) throw new Error('--output requires a path');
        outputPath = path.resolve(value);
        index++;
        break;
      default:
        throw new Error(`Unknown option ${option ?? '<missing>'}`);
    }
  }
  return { databasePath, outputPath };
}

/**
 * Return a file's size, or zero when absent.
 * @param filePath - Candidate path.
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
 * Hash a potentially multi-gigabyte file without materializing it in memory.
 * @param filePath - Existing file to hash.
 * @returns Lowercase SHA-256 digest.
 */
function sha256File(filePath: string): string {
  const hash = createHash('sha256');
  const buffer = Buffer.allocUnsafe(1024 * 1024);
  const file = fs.openSync(filePath, 'r');
  try {
    let bytesRead = 0;
    do {
      bytesRead = fs.readSync(file, buffer, 0, buffer.length, null);
      if (bytesRead > 0) hash.update(buffer.subarray(0, bytesRead));
    } while (bytesRead > 0);
  } finally {
    fs.closeSync(file);
  }
  return hash.digest('hex');
}

/**
 * Quote one SQLite identifier.
 * @param identifier - Trusted identifier returned by SQLite itself.
 * @returns Double-quoted SQL identifier.
 */
function quoteIdentifier(identifier: string): string {
  return `"${identifier.replaceAll('"', '""')}"`;
}

/**
 * Read source identity without changing repository state.
 * @returns Commit and dirty-worktree flag.
 */
function sourceIdentity(): { commit: string; dirty: boolean } {
  const env = { ...process.env, GIT_OPTIONAL_LOCKS: '0' };
  const commit = spawnSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8', env });
  const status = spawnSync('git', ['status', '--porcelain'], { encoding: 'utf8', env });
  return {
    commit: commit.status === 0 ? commit.stdout.trim() : 'unavailable',
    dirty: status.status !== 0 || status.stdout.trim().length > 0
  };
}

/**
 * Return the declared columns of one table.
 * @param db - Read-only database.
 * @param table - Existing table name returned by SQLite.
 * @returns Ordered column descriptions.
 */
function tableColumns(db: Database.Database, table: string): TableColumn[] {
  return db.prepare(`PRAGMA table_info(${quoteIdentifier(table)})`).all() as TableColumn[];
}

/**
 * Select an optional column without requiring a schema migration.
 * @param columns - Columns present in the inspected table.
 * @param name - Desired source and result name.
 * @returns SQL projection returning the column or null.
 */
function optionalColumn(columns: ReadonlySet<string>, name: string): string {
  const quoted = quoteIdentifier(name);
  return columns.has(name) ? quoted : `NULL AS ${quoted}`;
}

/**
 * Select the byte length of an optional variable-width column.
 * @param columns - Columns present in the inspected table.
 * @param name - Desired source column.
 * @param alias - Result column name.
 * @returns SQL projection returning the byte length or null.
 */
function optionalLengthColumn(
  columns: ReadonlySet<string>,
  name: string,
  alias: string
): string {
  const quotedAlias = quoteIdentifier(alias);
  return columns.has(name)
    ? `LENGTH(${quoteIdentifier(name)}) AS ${quotedAlias}`
    : `NULL AS ${quotedAlias}`;
}

/**
 * Query whether a named table exists.
 * @param db - Read-only database.
 * @param table - Table name.
 * @returns True when present.
 */
function hasTable(db: Database.Database, table: string): boolean {
  return Boolean(db.prepare(
    `SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = ?`
  ).get(table));
}

/**
 * Inventory one database without changing its logical contents. SQLite can
 * still create WAL shared-state sidecars for a read-only connection, so owner
 * artifacts must be copied before this probe is run.
 * @param databasePath - Existing disposable inspection copy.
 * @returns Machine-readable inventory.
 */
function inventoryDatabase(databasePath: string): Record<string, unknown> {
  if (!fs.existsSync(databasePath)) throw new Error(`Database does not exist: ${databasePath}`);
  const stat = fs.statSync(databasePath);
  const db = new Database(databasePath, { readonly: true, fileMustExist: true });
  try {
    db.pragma('query_only = ON');
    const tableRows = db.prepare(
      `SELECT name, sql
         FROM sqlite_schema
        WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
        ORDER BY name`
    ).all() as Array<{ name: string; sql: string }>;
    const tables = tableRows.map(row => {
      const columns = tableColumns(db, row.name);
      const count = (db.prepare(
        `SELECT COUNT(*) AS count FROM ${quoteIdentifier(row.name)}`
      ).get() as { count: number }).count;
      const variableColumns = columns.filter(column => {
        const type = column.type.toUpperCase();
        return type.includes('TEXT') || type.includes('BLOB') || type.length === 0;
      });
      const logical = variableColumns.map(column => {
        const result = db.prepare(
          `SELECT COALESCE(SUM(LENGTH(${quoteIdentifier(column.name)})), 0) AS bytes
             FROM ${quoteIdentifier(row.name)}`
        ).get() as { bytes: number };
        return { column: column.name, declaredType: column.type, bytes: result.bytes };
      });
      return {
        name: row.name,
        rowCount: count,
        columns,
        variableLengthLogicalBytes: logical,
        schemaSql: row.sql
      };
    });
    let dbstat: unknown = { available: false };
    try {
      dbstat = {
        available: true,
        objects: db.prepare(
          `SELECT name,
                  COUNT(*) AS pages,
                  SUM(pgsize) AS page_bytes,
                  SUM(payload) AS payload_bytes,
                  SUM(unused) AS unused_bytes
             FROM dbstat
            GROUP BY name
            ORDER BY name`
        ).all()
      };
    } catch (error) {
      dbstat = {
        available: false,
        reason: error instanceof Error ? error.message : String(error)
      };
    }
    const snapshotColumns = hasTable(db, 'population_snapshots')
      ? tableColumns(db, 'population_snapshots')
      : [];
    const snapshotColumnNames = new Set(snapshotColumns.map(column => column.name));
    const currentSnapshots = snapshotColumns.length > 0
      ? db.prepare(
        `SELECT ${optionalColumn(snapshotColumnNames, 'id')},
                ${optionalColumn(snapshotColumnNames, 'created_at')},
                ${optionalColumn(snapshotColumnNames, 'gen')},
                ${optionalColumn(snapshotColumnNames, 'format_version')},
                ${optionalColumn(snapshotColumnNames, 'boundary_kind')},
                ${optionalColumn(snapshotColumnNames, 'population_count')},
                ${optionalLengthColumn(snapshotColumnNames, 'payload_json', 'payload_json_bytes')},
                ${optionalLengthColumn(snapshotColumnNames, 'settings_json', 'settings_json_bytes')},
                ${optionalLengthColumn(snapshotColumnNames, 'updates_json', 'updates_json_bytes')},
                ${optionalLengthColumn(snapshotColumnNames, 'genomes_blob', 'legacy_blob_bytes')}
           FROM population_snapshots
          ORDER BY ${snapshotColumnNames.has('id') ? quoteIdentifier('id') : 'rowid'}`
      ).all()
      : [];
    const snapshotGenomes = hasTable(db, 'snapshot_genomes')
      ? db.prepare(
        `SELECT snapshot_id,
                COUNT(*) AS genome_count,
                SUM(weight_count) AS weight_count,
                SUM(LENGTH(weights_blob)) AS weight_bytes,
                MIN(weight_count) AS min_weights_per_genome,
                MAX(weight_count) AS max_weights_per_genome
           FROM snapshot_genomes
          GROUP BY snapshot_id
          ORDER BY snapshot_id`
      ).all()
      : [];
    const hallOfFameTotalRows = hasTable(db, 'hof_entries')
      ? (db.prepare('SELECT COUNT(*) AS count FROM hof_entries').get() as { count: number }).count
      : 0;
    const hallOfFameRows = hallOfFameTotalRows > 0
      ? db.prepare(
        `SELECT id, created_at, gen, seed, fitness, points, length,
                LENGTH(genome_json) AS genome_json_bytes,
                CASE WHEN json_valid(genome_json)
                     THEN json_extract(genome_json, '$.archKey') END AS arch_key,
                CASE WHEN json_valid(genome_json)
                     THEN json_extract(genome_json, '$.brainType') END AS brain_type,
                CASE WHEN json_valid(genome_json)
                     THEN json_array_length(genome_json, '$.weights') END AS weight_count,
                COALESCE(json_valid(genome_json), 0) AS json_valid
           FROM hof_entries
          ORDER BY id
          LIMIT ?`
      ).all(HALL_OF_FAME_DETAIL_LIMIT)
      : [];
    return {
      schema: 'slither-stage2-database-baseline',
      version: 2,
      evidenceClass: 'new measured result',
      accessMode: 'read-only',
      sourcePreservationRequirement:
        'Run against a copied database: SQLite may create -wal/-shm shared-state sidecars even for a query-only connection.',
      source: sourceIdentity(),
      artifact: {
        path: databasePath,
        sha256: sha256File(databasePath),
        sizeBytes: stat.size,
        createdAt: stat.birthtime.toISOString(),
        modifiedAt: stat.mtime.toISOString(),
        walBytes: fileSize(`${databasePath}-wal`),
        shmBytes: fileSize(`${databasePath}-shm`)
      },
      sqlite: {
        libraryVersion: (db.prepare('SELECT sqlite_version() AS version').get() as { version: string }).version,
        journalMode: (db.pragma('journal_mode', { simple: true }) as string),
        pageSize: db.pragma('page_size', { simple: true }) as number,
        pageCount: db.pragma('page_count', { simple: true }) as number,
        freelistCount: db.pragma('freelist_count', { simple: true }) as number,
        autoVacuum: db.pragma('auto_vacuum', { simple: true }) as number
      },
      tables,
      dbstat,
      snapshotStorageShape: {
        parentColumns: snapshotColumns.map(column => column.name),
        hasFormatVersionColumn: snapshotColumnNames.has('format_version'),
        hasLegacyCombinedBlobColumn: snapshotColumnNames.has('genomes_blob'),
        hasPerGenomeChildTable: hasTable(db, 'snapshot_genomes')
      },
      currentSnapshots,
      snapshotGenomes,
      hallOfFameDetails: {
        limit: HALL_OF_FAME_DETAIL_LIMIT,
        totalRows: hallOfFameTotalRows,
        returnedRows: hallOfFameRows.length,
        omittedRows: Math.max(0, hallOfFameTotalRows - hallOfFameRows.length),
        rows: hallOfFameRows
      }
    };
  } finally {
    db.close();
  }
}

/** Execute the CLI. */
function main(): void {
  const options = parseOptions(process.argv.slice(2));
  const result = inventoryDatabase(options.databasePath);
  const json = `${JSON.stringify(result, null, 2)}\n`;
  if (options.outputPath) {
    fs.mkdirSync(path.dirname(options.outputPath), { recursive: true });
    fs.writeFileSync(options.outputPath, json, 'utf8');
    console.info(`[stage2.database] wrote ${options.outputPath}`);
  } else {
    process.stdout.write(json);
  }
}

try {
  main();
} catch (error) {
  console.error(error);
  process.exitCode = 1;
}
