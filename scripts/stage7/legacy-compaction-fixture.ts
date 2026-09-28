/** Disposable overnight-sized old-BLOB SQLite volume and offline compaction fixture. */

import { existsSync } from 'node:fs';
import { mkdir } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import Database from 'better-sqlite3';
import { compactLegacyDatabase } from './compact-legacy-database.ts';

/** Reproducible legacy volume matching the measured P0 old-population payload. */
const PAYLOAD_BYTES = 2_527_124;
/** Eight hours of 60-second rounds. */
const GENERATIONS = 480;

/** Read two explicit absent disposable destinations. */
function paths(argv: readonly string[]): { databasePath: string; backupDirectory: string } {
  if (argv.length !== 4 || argv[0] !== '--db-path' || !argv[1] ||
      argv[2] !== '--backup' || !argv[3]) {
    throw new Error('usage: legacy-compaction-fixture.ts --db-path NEW_DB --backup NEW_DIR');
  }
  const databasePath = resolve(argv[1]);
  const backupDirectory = resolve(argv[3]);
  if (existsSync(databasePath) || existsSync(backupDirectory)) {
    throw new Error('legacy fixture database and backup must both be absent');
  }
  return { databasePath, backupDirectory };
}

/** Write old combined BLOB rows, delete their migrated predecessors, and compact. */
export async function runLegacyCompactionFixture(databasePath: string, backupDirectory: string): Promise<Record<string, unknown>> {
  await mkdir(dirname(databasePath), { recursive: true });
  const db = new Database(databasePath);
  try {
    db.pragma('journal_mode = WAL');
    db.exec(`CREATE TABLE population_snapshots (
      id INTEGER PRIMARY KEY, created_at TEXT NOT NULL, gen INTEGER NOT NULL,
      payload_json TEXT NOT NULL, genomes_blob BLOB NOT NULL
    )`);
    const insert = db.prepare(`INSERT INTO population_snapshots
      (created_at, gen, payload_json, genomes_blob) VALUES (?, ?, '{}', ?)`);
    const payload = Buffer.alloc(PAYLOAD_BYTES, 0x5a);
    for (let first = 1; first <= GENERATIONS; first += 20) {
      db.transaction(() => {
        for (let generation = first; generation < Math.min(first + 20, GENERATIONS + 1); generation++) {
          insert.run('2026-09-28T00:00:00.000Z', generation, payload);
        }
      })();
      db.pragma('wal_checkpoint(TRUNCATE)');
      if (first === 1 || first % 100 === 1) process.stderr.write(`inserted=${Math.min(first + 19, GENERATIONS)}\n`);
    }
    const beforeDelete = db.prepare('SELECT count(*) AS count FROM population_snapshots').get() as { count: number };
    if (beforeDelete.count !== GENERATIONS) throw new Error('legacy fixture omitted population rows');
    db.prepare('DELETE FROM population_snapshots WHERE gen < ?').run(GENERATIONS);
    db.pragma('wal_checkpoint(TRUNCATE)');
    if ((db.pragma('freelist_count', { simple: true }) as number) === 0) {
      throw new Error('legacy fixture did not leave reusable pages');
    }
  } finally { db.close(); }
  const result = await compactLegacyDatabase({ databasePath, backupDirectory });
  const retained = new Database(databasePath, { readonly: true });
  try {
    const row = retained.prepare('SELECT count(*) AS count, max(gen) AS generation FROM population_snapshots')
      .get() as { count: number; generation: number };
    if (row.count !== 1 || row.generation !== GENERATIONS) {
      throw new Error('compaction changed the retained legacy population row');
    }
  } finally { retained.close(); }
  return { fixtureGenerations: GENERATIONS, payloadBytes: PAYLOAD_BYTES,
    nominalLegacyPopulationBytes: GENERATIONS * PAYLOAD_BYTES, ...result };
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    const options = paths(process.argv.slice(2));
    void runLegacyCompactionFixture(options.databasePath, options.backupDirectory)
      .then(result => process.stdout.write(`${JSON.stringify(result, null, 2)}\n`))
      .catch(error => { console.error(error); process.exitCode = 1; });
  } catch (error) { console.error(error); process.exitCode = 1; }
}
