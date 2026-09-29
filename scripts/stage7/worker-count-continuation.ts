/** Compare one exact generation continuation under one and five Rust workers. */

import { existsSync } from 'node:fs';
import { link, mkdir, readdir } from 'node:fs/promises';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import Database from 'better-sqlite3';
import { DEFAULT_CONFIG } from '../../server/config.ts';
import { startRustServer } from '../../server/rustServer.ts';

/** Exact identity of the active current checkpoint in a stopped fixture. */
interface Boundary {
  /** Persisted run shared by every copy. */
  runId: string;
  /** Current generation as fixed-width hexadecimal text. */
  generation: string;
  /** Content identity of the current population checkpoint. */
  checkpointId: string;
}

/** Small production health projection needed to await one full generation. */
interface Health {
  /** False after any native or interface fault. */
  ok: boolean;
  /** Native lifecycle name. */
  lifecycle: string;
  /** Exact active run. */
  runId: string;
  /** Exact generation. */
  generation: string;
  /** Complete step counter. */
  completedStep: string;
  /** Dropped scheduler time. */
  schedulerDroppedWallMicros: string;
  /** Public failure detail, when available. */
  interfaceFault?: string;
}

/** Parse one source fixture and require a fresh output root. */
function options(argv: readonly string[]): { sourcePath: string; outputRoot: string } {
  if (argv.length !== 4 || argv[0] !== '--source-db' || argv[2] !== '--output-root' ||
      !argv[1] || !argv[3]) {
    throw new Error('usage: --source-db STOPPED_FIXTURE_DB --output-root NEW_DIRECTORY');
  }
  const sourcePath = resolve(argv[1]);
  const outputRoot = resolve(argv[3]);
  if (!existsSync(sourcePath) || !existsSync(`${sourcePath}.checkpoints`)) {
    throw new Error('source database and managed directory must exist');
  }
  if (existsSync(outputRoot)) throw new Error(`output already exists: ${outputRoot}`);
  return { sourcePath, outputRoot };
}

/** Query the active SQLite pointer without opening its population file. */
function activeBoundary(databasePath: string): Boundary {
  const database = new Database(databasePath, { readonly: true, fileMustExist: true });
  try {
    const row = database.prepare(`SELECT current.run_id AS runId,
      current.checkpoint_id AS checkpointId, metadata.generation_hex AS generation
      FROM rust_active_run_v1 AS active
      JOIN rust_checkpoint_v3_current AS current ON current.run_id = active.run_id
      JOIN rust_checkpoint_v3_metadata AS metadata ON metadata.checkpoint_id = current.checkpoint_id
      WHERE active.singleton = 1`).get() as Boundary | undefined;
    if (!row || !/^[0-9a-f]{64}$/u.test(row.checkpointId)) {
      throw new Error('fixture has no valid active managed checkpoint');
    }
    return row;
  } finally { database.close(); }
}

/** Copy metadata consistently and hard-link only immutable managed objects. */
async function copyFixture(sourcePath: string, targetPath: string): Promise<void> {
  const source = new Database(sourcePath, { readonly: true, fileMustExist: true });
  try { await source.backup(targetPath); }
  finally { source.close(); }
  const sourceManaged = `${sourcePath}.checkpoints`;
  const targetManaged = `${targetPath}.checkpoints`;
  await mkdir(targetManaged);
  for (const entry of await readdir(sourceManaged, { withFileTypes: true })) {
    if (!entry.isFile() || !/^[0-9a-f]{64}\.(?:checkpoint-v3|hof-weights-v1)$/u.test(entry.name)) continue;
    await link(resolve(sourceManaged, entry.name), resolve(targetManaged, entry.name));
  }
}

/** Read current native health with a bounded local HTTP request. */
async function health(port: number): Promise<Health> {
  const response = await fetch(`http://127.0.0.1:${port}/api/health`, {
    signal: AbortSignal.timeout(5_000)
  });
  const value = await response.json() as Health;
  if (!response.ok || !value.ok) {
    throw new Error(`Rust continuation faulted: ${value.interfaceFault ?? value.lifecycle}`);
  }
  return value;
}

/** Advance exactly one complete generation under one requested worker count. */
async function continueGeneration(databasePath: string, workers: number, source: Boundary): Promise<{
  workers: number; boundary: Boundary; wallSeconds: number; completedSteps: string;
  droppedWallMicros: string
}> {
  const server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, dbPath: databasePath,
    resume: 'latest', rustCalculationWorkers: workers, logLevel: 'error' });
  if (server.startupFault) {
    await server.close();
    throw new Error(`Rust ${workers}-worker restart failed: ${server.startupFault}`);
  }
  let first: Health;
  let last: Health;
  const startedAt = performance.now();
  try {
    first = await health(server.port);
    if (first.runId !== source.runId || first.generation !== source.generation) {
      throw new Error(`${workers}-worker copy resumed another source boundary`);
    }
    const targetGeneration = BigInt(`0x${source.generation}`) + 1n;
    const deadline = performance.now() + 300_000;
    for (;;) {
      last = await health(server.port);
      const generation = BigInt(`0x${last.generation}`);
      if (generation > targetGeneration) throw new Error('continuation advanced beyond the target generation');
      if (generation === targetGeneration) break;
      if (performance.now() >= deadline) {
        throw new Error(`${workers}-worker continuation did not finish within five minutes`);
      }
      await new Promise<void>(done => setTimeout(done, 250));
    }
  } finally { await server.close(); }
  const boundary = activeBoundary(databasePath);
  if (boundary.runId !== source.runId ||
      BigInt(`0x${boundary.generation}`) !== BigInt(`0x${source.generation}`) + 1n) {
    throw new Error(`${workers}-worker copy did not commit the next population`);
  }
  return { workers, boundary, wallSeconds: (performance.now() - startedAt) / 1000,
    completedSteps: (BigInt(`0x${last.completedStep}`) - BigInt(`0x${first.completedStep}`)).toString(),
    droppedWallMicros: (BigInt(`0x${last.schedulerDroppedWallMicros}`) -
      BigInt(`0x${first.schedulerDroppedWallMicros}`)).toString() };
}

/** Run two isolated continuations sequentially to avoid target-VM contention. */
export async function run(sourcePath: string, outputRoot: string): Promise<Record<string, unknown>> {
  if (existsSync(outputRoot)) throw new Error(`output already exists: ${outputRoot}`);
  await mkdir(outputRoot, { recursive: true });
  const source = activeBoundary(sourcePath);
  const results: Array<Awaited<ReturnType<typeof continueGeneration>>> = [];
  for (const workers of [1, 5]) {
    const databasePath = resolve(outputRoot, `workers-${workers}.sqlite`);
    await copyFixture(sourcePath, databasePath);
    if (JSON.stringify(activeBoundary(databasePath)) !== JSON.stringify(source)) {
      throw new Error(`${workers}-worker copy changed its source identity`);
    }
    const result = await continueGeneration(databasePath, workers, source);
    results.push(result);
    process.stderr.write(`workers=${workers} generation=${BigInt(`0x${result.boundary.generation}`)} checkpoint=${result.boundary.checkpointId}\n`);
  }
  return { sourcePath, outputRoot, source, results,
    exactNextCheckpointMatch: results[0]!.boundary.checkpointId === results[1]!.boundary.checkpointId };
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const request = options(process.argv.slice(2));
  void run(request.sourcePath, request.outputRoot)
    .then(result => process.stdout.write(`${JSON.stringify(result)}\n`))
    .catch(error => { process.stderr.write(`${String(error)}\n`); process.exitCode = 1; });
}
