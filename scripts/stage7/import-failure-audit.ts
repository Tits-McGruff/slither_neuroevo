/** Capture compact production health, SQLite metadata and immutable source hashes around a browser import. */
import { createHash } from 'node:crypto';
import { createReadStream, existsSync } from 'node:fs';
import { readdir, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import Database from 'better-sqlite3';

/** Hash one managed file incrementally without copying its population into JavaScript. */
async function fileHash(path: string): Promise<string> {
  const hash = createHash('sha256');
  for await (const chunk of createReadStream(path)) hash.update(chunk as Buffer);
  return hash.digest('hex');
}

/** Read only the named loopback authority's small health response. */
async function health(port: number): Promise<Record<string, unknown>> {
  const response = await fetch(`http://127.0.0.1:${port}/api/health`, { signal: AbortSignal.timeout(5000) });
  const result = await response.json() as Record<string, unknown>;
  if (!response.ok || result['ok'] !== true) throw new Error('production health is not healthy');
  return result;
}

/** Require an existing task database, explicit loopback port and absent output file. */
async function main(argv: readonly string[]): Promise<void> {
  if (argv.length !== 6 || argv[0] !== '--db-path' || argv[2] !== '--port' || argv[4] !== '--output' ||
      !argv[1] || !argv[3] || !argv[5] || !/^[1-9][0-9]*$/u.test(argv[3])) {
    throw new Error('usage: --db-path TASK_DB --port PORT --output NEW_JSON');
  }
  const databasePath = resolve(argv[1]);
  const port = Number(argv[3]);
  const output = resolve(argv[5]);
  if (!existsSync(databasePath) || existsSync(output) || !Number.isSafeInteger(port) || port < 1024 || port > 65535) {
    throw new Error('existing database, valid port and absent output required');
  }
  const before = await health(port);
  const database = new Database(databasePath, { readonly: true, fileMustExist: true });
  const metadata = database.transaction(() => {
    const tables = database.prepare(`SELECT name FROM sqlite_master
      WHERE type = 'table' AND name LIKE 'rust_%' ORDER BY name`).all() as Array<{ name: string }>;
    return tables.map(({ name }) => {
      const hash = createHash('sha256');
      let rows = 0;
      let bytes = 0;
      for (const row of database.prepare(`SELECT * FROM "${name.replaceAll('"', '""')}" ORDER BY rowid`).iterate()) {
        if (++rows > 2000) throw new Error('audit requires a compact fixture with at most 2000 rows per table');
        const encoded = JSON.stringify(row);
        bytes += Buffer.byteLength(encoded);
        if (bytes > 8 * 1024 * 1024) throw new Error('audit metadata exceeds eight MiB per table');
        hash.update(encoded).update('\n');
      }
      return { table: name, rows, bytes, sha256: hash.digest('hex') };
    });
  });
  let tables: ReturnType<typeof metadata>;
  try { tables = metadata.deferred(); }
  finally { database.close(); }
  const managedDirectory = `${databasePath}.checkpoints`;
  const names = (await readdir(managedDirectory)).sort();
  const files: Array<{ name: string; sha256: string }> = [];
  for (const name of names) {
    if (/^[0-9a-f]{64}\.(?:checkpoint-v3|hof-weights-v1)$/u.test(name)) {
      files.push({ name, sha256: await fileHash(resolve(managedDirectory, name)) });
    }
  }
  const after = await health(port);
  if (before['runId'] !== after['runId'] || before['generation'] !== after['generation']) {
    throw new Error('authority changed generation/run during audit; capture a new sample');
  }
  await writeFile(output, JSON.stringify({ schemaVersion: 1, healthBefore: before, healthAfter: after,
    tables, files, otherManagedNames: names.filter(name => !files.some(file => file.name === name)) }, null, 2) + '\n',
  { flag: 'wx' });
  console.log(JSON.stringify({ output, runId: after['runId'], generation: after['generation'], files: files.length }));
}

void main(process.argv.slice(2)).catch(error => { console.error(String(error)); process.exitCode = 1; });
