/** Read-only Stage 2 inventory for current SQLite persistence artifacts. */

import { createHash } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import Database from 'better-sqlite3';

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
  const commit = spawnSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' });
  const status = spawnSync('git', ['status', '--porcelain'], { encoding: 'utf8' });
  return {
    commit: commit.status === 0 ? commit.stdout.trim() : 'unavailable',
    dirty: status.status !== 0 || status.stdout.trim().length > 0
  };
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
 * Inventory one current database without changing pages or journal state.
 * @param databasePath - Existing database.
 * @returns Machine-readable inventory.
 */
function inventoryDatabase(databasePath: string): Record<string, unknown> {
  if (!fs.existsSync(databasePath)) throw new Error(`Database does not exist: ${databasePath}`);
  const stat = fs.statSync(databasePath);
  const db = new Database(databasePath, { readonly: true, fileMustExist: true });
  try {
    const tableRows = db.prepare(
      `SELECT name, sql
         FROM sqlite_schema
        WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
        ORDER BY name`
    ).all() as Array<{ name: string; sql: string }>;
    const tables = tableRows.map(row => {
      const columns = db.prepare(`PRAGMA table_info(${quoteIdentifier(row.name)})`).all() as TableColumn[];
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
    const currentSnapshots = hasTable(db, 'population_snapshots')
      ? db.prepare(
        `SELECT id, created_at, gen, format_version, boundary_kind,
                population_count, LENGTH(payload_json) AS payload_json_bytes,
                LENGTH(settings_json) AS settings_json_bytes,
                LENGTH(updates_json) AS updates_json_bytes,
                LENGTH(genomes_blob) AS legacy_blob_bytes
           FROM population_snapshots
          ORDER BY id`
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
    return {
      schema: 'slither-stage2-database-baseline',
      version: 1,
      evidenceClass: 'new measured result',
      accessMode: 'read-only',
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
      currentSnapshots,
      snapshotGenomes
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
