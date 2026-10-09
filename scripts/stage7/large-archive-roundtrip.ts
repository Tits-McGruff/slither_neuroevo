/** Stream a real P3 archive over 50 MiB through export, import, and restart. */

import { createReadStream, createWriteStream, existsSync } from 'node:fs';
import { link, mkdir, readdir, stat } from 'node:fs/promises';
import { resolve } from 'node:path';
import { Writable } from 'node:stream';
import { pathToFileURL } from 'node:url';
import Database from 'better-sqlite3';
import { DEFAULT_CONFIG } from '../../server/config.ts';
import { startRustServer } from '../../server/rustServer.ts';

/** Exact source checkpoint identity selected from durable SQLite metadata. */
interface Source {
  runId: string;
  checkpointId: string;
  generation: string;
}

/** Require a source fixture and a new output directory. */
function options(argv: readonly string[]): { sourcePath: string; outputRoot: string } {
  if (argv.length !== 4 || argv[0] !== '--source-db' || argv[2] !== '--output-root' ||
      !argv[1] || !argv[3]) {
    throw new Error('usage: --source-db EXISTING_DB --output-root NEW_DIRECTORY');
  }
  const sourcePath = resolve(argv[1]);
  const outputRoot = resolve(argv[3]);
  if (!existsSync(sourcePath) || !existsSync(`${sourcePath}.checkpoints`)) {
    throw new Error('source database and managed directory must exist');
  }
  if (existsSync(outputRoot)) throw new Error(`output already exists: ${outputRoot}`);
  return { sourcePath, outputRoot };
}

/** Identify the active current checkpoint without loading population data. */
function sourceIdentity(databasePath: string): Source {
  const database = new Database(databasePath, { readonly: true, fileMustExist: true });
  try {
    const row = database.prepare(`SELECT current.run_id AS runId,
      current.checkpoint_id AS checkpointId, metadata.generation_hex AS generation
      FROM rust_active_run_v1 AS active
      JOIN rust_checkpoint_v3_current AS current ON current.run_id = active.run_id
      JOIN rust_checkpoint_v3_metadata AS metadata ON metadata.checkpoint_id = current.checkpoint_id
      WHERE active.singleton = 1`).get() as Source | undefined;
    if (!row || !/^[0-9a-f]{64}$/u.test(row.checkpointId)) {
      throw new Error('source fixture has no valid active checkpoint');
    }
    return row;
  } finally { database.close(); }
}

/** Copy only the compact database and hard-link immutable managed files. */
async function isolatedSource(sourcePath: string, databasePath: string): Promise<void> {
  const database = new Database(sourcePath, { readonly: true, fileMustExist: true });
  try { await database.backup(databasePath); }
  finally { database.close(); }
  const managedPath = `${databasePath}.checkpoints`;
  await mkdir(managedPath);
  for (const entry of await readdir(`${sourcePath}.checkpoints`, { withFileTypes: true })) {
    if (!entry.isFile() || !/^[0-9a-f]{64}\.(?:checkpoint-v3|hof-weights-v1)$/u.test(entry.name)) continue;
    await link(resolve(`${sourcePath}.checkpoints`, entry.name), resolve(managedPath, entry.name));
  }
}

/** Read the small identity fields from one running production server. */
async function health(port: number): Promise<{ ok: boolean; runId: string;
  startupCheckpointId: string; generation: string }> {
  const response = await fetch(`http://127.0.0.1:${port}/api/health`, {
    signal: AbortSignal.timeout(10_000)
  });
  const value = await response.json() as { ok: boolean; runId: string;
    startupCheckpointId: string; generation: string };
  if (!response.ok || !value.ok) throw new Error(`production health failed: ${response.status}`);
  return value;
}

/** Run one direct archive export without a population-sized JS buffer. */
async function exportArchive(port: number, checkpointId: string, archivePath: string): Promise<{
  archiveBytes: number; saveRoot: string
}> {
  const response = await fetch(`http://127.0.0.1:${port}/api/export/latest`, {
    signal: AbortSignal.timeout(600_000)
  });
  const saveRoot = response.headers.get('x-slither-save-root') ?? '';
  if (!response.ok || !response.body || response.headers.get('x-slither-checkpoint-id') !== checkpointId ||
      !/^[0-9a-f]{64}$/u.test(saveRoot)) {
    throw new Error(`direct export failed or selected another checkpoint: ${response.status}`);
  }
  await response.body.pipeTo(Writable.toWeb(createWriteStream(archivePath)));
  const archiveBytes = (await stat(archivePath)).size;
  if (archiveBytes <= 50 * 1024 * 1024 ||
      archiveBytes !== Number(response.headers.get('content-length'))) {
    throw new Error(`archive did not exceed 50 MiB or match its declared length: ${archiveBytes}`);
  }
  return { archiveBytes, saveRoot };
}

/** Send the original archive file directly as one bounded HTTP request body. */
async function importArchive(port: number, archivePath: string, archiveBytes: number): Promise<{
  ok: boolean; runId: string; generation: string; checkpointId: string
}> {
  const response = await fetch(`http://127.0.0.1:${port}/api/import/archive`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/vnd.slither-neuroevo.save',
      'Content-Length': String(archiveBytes) },
    body: createReadStream(archivePath), duplex: 'half', signal: AbortSignal.timeout(600_000)
  });
  const result = await response.json() as { ok: boolean; runId: string;
    generation: string; checkpointId: string; message?: string };
  if (!response.ok || !result.ok) {
    throw new Error(`direct import failed: ${response.status} ${result.message ?? JSON.stringify(result)}`);
  }
  return result;
}

/** Execute the production round trip and retain its file and copied databases. */
export async function run(sourcePath: string, outputRoot: string): Promise<Record<string, unknown>> {
  if (existsSync(outputRoot)) throw new Error(`output already exists: ${outputRoot}`);
  await mkdir(outputRoot, { recursive: true });
  const sourceCopyPath = resolve(outputRoot, 'source.sqlite');
  const targetPath = resolve(outputRoot, 'target.sqlite');
  const archivePath = resolve(outputRoot, 'export.slither-save');
  await isolatedSource(sourcePath, sourceCopyPath);
  const selected = sourceIdentity(sourceCopyPath);
  const source = await startRustServer({ ...DEFAULT_CONFIG, port: 0, dbPath: sourceCopyPath,
    resume: `sha256:${selected.checkpointId}`, logLevel: 'error' });
  let archive: Awaited<ReturnType<typeof exportArchive>>;
  try {
    if (source.startupFault) throw new Error(`source startup failed: ${source.startupFault}`);
    const before = await health(source.port);
    if (before.runId !== selected.runId || before.startupCheckpointId !== selected.checkpointId) {
      throw new Error('source server selected a different checkpoint');
    }
    archive = await exportArchive(source.port, selected.checkpointId, archivePath);
  } finally { await source.close(); }
  let target = await startRustServer({ ...DEFAULT_CONFIG, port: 0, dbPath: targetPath,
    resume: 'fresh', seed: 42, logLevel: 'error' });
  let imported: Awaited<ReturnType<typeof importArchive>>;
  try {
    if (target.startupFault) throw new Error(`target startup failed: ${target.startupFault}`);
    imported = await importArchive(target.port, archivePath, archive.archiveBytes);
    const after = await health(target.port);
    if (imported.runId !== selected.runId || imported.checkpointId !== selected.checkpointId ||
        imported.generation !== selected.generation || after.runId !== selected.runId ||
        after.startupCheckpointId !== selected.checkpointId) {
      throw new Error(`import did not activate the selected checkpoint: ${JSON.stringify({ imported, after })}`);
    }
  } finally { await target.close(); }
  target = await startRustServer({ ...DEFAULT_CONFIG, port: 0, dbPath: targetPath,
    resume: 'latest', logLevel: 'error' });
  try {
    if (target.startupFault) throw new Error(`imported restart failed: ${target.startupFault}`);
    const resumed = await health(target.port);
    if (resumed.runId !== selected.runId || resumed.startupCheckpointId !== selected.checkpointId) {
      throw new Error('imported checkpoint did not survive restart');
    }
  } finally { await target.close(); }
  return { sourcePath, outputRoot, selected, archive, imported, restartVerified: true };
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const parsed = options(process.argv.slice(2));
  void run(parsed.sourcePath, parsed.outputRoot)
    .then(result => process.stdout.write(`${JSON.stringify(result)}\n`))
    .catch(error => { process.stderr.write(`${String(error)}\n`); process.exitCode = 1; });
}
