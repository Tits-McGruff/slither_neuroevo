/** Accelerated production checkpoint-v3 volume fixture for the approved P0/P2/P3 workloads. */

import { existsSync } from 'node:fs';
import { mkdir, readdir, stat } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import Database from 'better-sqlite3';
import WebSocket from 'ws';
import { DEFAULT_CONFIG } from '../../server/config.ts';
import { CheckpointPersistenceClient } from '../../server/rustEngine/checkpointPersistenceClient.ts';
import { startRustServer } from '../../server/rustServer.ts';
import { buildLargeBrainGraph } from '../stage2/fixtures.ts';

/** Approved workload names represented by this measured fixture. */
type Scenario = 'P0' | 'P2' | 'P3';

/** Validated invocation values. */
interface Options {
  scenario: Scenario;
  databasePath: string;
  generations: number;
  seed: number;
  rustWorkers: number;
  resumeExisting: boolean;
}

/** One physical allocation sample including live SQLite sidecars. */
interface StorageSample {
  managedFiles: number;
  managedBytes: number;
  databaseBytes: number;
  walBytes: number;
  shmBytes: number;
  totalBytes: number;
}

/** Bounded production health fields needed for the accelerated loop. */
interface FixtureHealth {
  ok: boolean;
  runId: string;
  seed: number;
  generation: string;
  completedStep: string;
  startupCheckpointId: string;
  loopState?: string;
  commandServiceBoundaries?: string;
  coordinatorWorkBytes?: string;
  faultCode?: string;
}

/** Immutable checkpoint protected from automatic pruning during the fixture. */
interface FixturePin {
  ok: boolean;
  checkpointId: string;
  generation: string;
}

/** Parse one named unsigned integer without silently accepting partial text. */
function unsigned(value: string | undefined, name: string, maximum: number): number {
  if (!value || !/^(?:0|[1-9][0-9]*)$/u.test(value)) throw new Error(`${name} must be an integer`);
  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed) || parsed < 1 || parsed > maximum) {
    throw new RangeError(`${name} must be from 1 to ${maximum}`);
  }
  return parsed;
}

/** Require an absent disposable destination and explicit workload selection. */
function parseOptions(argv: readonly string[]): Options {
  const values = new Map<string, string>();
  let resumeExisting = false;
  for (let index = 0; index < argv.length; index++) {
    const name = argv[index];
    if (name === '--resume') {
      if (resumeExisting) throw new Error('duplicate --resume option');
      resumeExisting = true;
      continue;
    }
    const value = argv[++index];
    if (!name || !value || !['--scenario', '--db-path', '--generations', '--seed', '--rust-workers'].includes(name) ||
        values.has(name)) throw new Error(`invalid or duplicate fixture option: ${name}`);
    values.set(name, value);
  }
  const scenario = values.get('--scenario');
  if (scenario !== 'P0' && scenario !== 'P2' && scenario !== 'P3') {
    throw new Error('--scenario must be P0, P2 or P3');
  }
  const db = values.get('--db-path');
  if (!db) throw new Error('--db-path is required');
  const databasePath = resolve(db);
  const exists = existsSync(databasePath) && existsSync(`${databasePath}.checkpoints`);
  if (resumeExisting ? !exists : existsSync(databasePath) || existsSync(`${databasePath}.checkpoints`)) {
    throw new Error(`fixture destination has the wrong existence state: ${databasePath}`);
  }
  return {
    scenario,
    databasePath,
    generations: unsigned(values.get('--generations') ?? '480', '--generations', 1000),
    seed: unsigned(values.get('--seed') ?? '1511506142', '--seed', 0xffff_ffff),
    rustWorkers: unsigned(values.get('--rust-workers') ?? '5', '--rust-workers', 7),
    resumeExisting
  };
}

/** Read a strictly scalar Protocol 2 reply from a nonbinary WebSocket message. */
function packet(data: WebSocket.RawData): Record<string, unknown> {
  const value: unknown = JSON.parse(data.toString());
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('invalid server packet');
  return value as Record<string, unknown>;
}

/** Establish the exact workload via Reset and then accelerate only the step request rate. */
async function configureWorkload(port: number, options: Options): Promise<void> {
  const large = options.scenario !== 'P0';
  const snakeCount = options.scenario === 'P3' ? 300 : 55;
  const graphSpec = large ? buildLargeBrainGraph(147) : null;
  const socket = new WebSocket(`ws://127.0.0.1:${port}`);
  await new Promise<void>((resolveReady, reject) => {
    let finished = false;
    const timeout = setTimeout(() => finish(new Error('workload reset or speed application timed out')), 120_000);
    /** Complete this one command exchange and close its spectator connection. */
    const finish = (error?: Error): void => {
      if (finished) return;
      finished = true;
      clearTimeout(timeout);
      socket.terminate();
      if (error) reject(error);
      else resolveReady();
    };
    socket.on('open', () => socket.send(JSON.stringify({ type: 'hello', version: 2, clientType: 'ui' })));
    socket.on('error', error => finish(error));
    socket.on('close', () => finish(new Error('workload socket closed early')));
    socket.on('message', (data, binary) => {
      if (binary || finished) return;
      try {
        const message = packet(data);
        if (message['type'] === 'welcome') {
          socket.send(JSON.stringify({ type: 'join', mode: 'spectator' }));
          socket.send(JSON.stringify({
            type: 'reset',
            settings: {
              snakeCount, simSpeed: 1,
              ...(large ? { hiddenLayers: 5, neurons1: 256, neurons2: 256, neurons3: 256,
                neurons4: 256, neurons5: 256 } : {})
            },
            updates: [
              { path: 'generationSeconds', value: 8 },
              { path: 'sense.bubbleBins', value: large ? 32 : 16 },
              { path: 'baselineBots.count', value: 10 },
              { path: 'pelletCountTarget', value: 3500 }
            ],
            graphSpec
          }));
        } else if (message['type'] === 'stateReplaced' && message['reason'] === 'reset') {
          const welcome = message['welcome'] as { inferenceMode?: { parameterCount?: number };
            sensorSpec?: { sensorCount?: number }; settings?: {
              core?: { snakeCount?: number }; updates?: Array<{ path?: string; value?: unknown }>
            } } | undefined;
          if (welcome?.settings?.core?.snakeCount !== snakeCount ||
              !welcome.settings.updates?.some(update =>
                update.path === 'generationSeconds' && update.value === 8) ||
              !welcome.settings.updates.some(update =>
                update.path === 'baselineBots.count' && update.value === 10) ||
              !welcome.settings.updates.some(update =>
                update.path === 'pelletCountTarget' && update.value === 3500)) {
            throw new Error('Rust reset did not select the approved population, round or world settings');
          }
          if (large && (welcome?.sensorSpec?.sensorCount !== 147 ||
              (welcome.inferenceMode?.parameterCount ?? 0) < 400_000)) {
            throw new Error('Rust reset did not select the approved large-brain sensor and graph');
          }
          socket.send(JSON.stringify({ type: 'join', mode: 'spectator' }));
          socket.send(JSON.stringify({ type: 'settings', requestId: 'fixture-speed',
            updates: [{ path: 'simSpeed', value: 12 }] }));
        } else if (message['type'] === 'settingsApplied' && message['requestId'] === 'fixture-speed') {
          if (message['applied'] !== true) throw new Error('Rust rejected fixture speed');
          finish();
        } else if (message['type'] === 'error') {
          finish(new Error(`Rust rejected fixture setup: ${String(message['message'])}`));
        }
      } catch (error) { finish(error instanceof Error ? error : new Error(String(error))); }
    });
  });
}

/** Return a file's allocation length or zero for an absent SQLite sidecar. */
async function fileBytes(path: string): Promise<number> {
  try { return (await stat(path)).size; }
  catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') return 0;
    throw error;
  }
}

/** Measure real managed files and SQLite/WAL sidecars without reading population contents. */
async function sampleStorage(databasePath: string): Promise<StorageSample> {
  const managedRoot = `${databasePath}.checkpoints`;
  let managedBytes = 0;
  let managedFiles = 0;
  for (const entry of await readdir(managedRoot, { withFileTypes: true })) {
    if (!entry.isFile()) continue;
    let length: number;
    try { length = (await stat(resolve(managedRoot, entry.name))).size; }
    catch (error) {
      if ((error as NodeJS.ErrnoException).code === 'ENOENT') continue;
      throw error;
    }
    managedFiles++;
    managedBytes += length;
  }
  const databaseBytes = await fileBytes(databasePath);
  const walBytes = await fileBytes(`${databasePath}-wal`);
  const shmBytes = await fileBytes(`${databasePath}-shm`);
  return { managedFiles, managedBytes, databaseBytes, walBytes, shmBytes,
    totalBytes: managedBytes + databaseBytes + walBytes + shmBytes };
}

/** Query only compact durable metadata once authority has stopped. */
function inspectMetadata(databasePath: string): Record<string, unknown> {
  const database = new Database(databasePath, { readonly: true });
  try {
    const count = (table: string): number => (database.prepare(`SELECT count(*) AS n FROM ${table}`).get() as { n: number }).n;
    const retention = database.prepare(`SELECT retention_kind AS kind, count(*) AS checkpoints
      FROM rust_checkpoint_retention_v1 GROUP BY retention_kind ORDER BY retention_kind`).all();
    return {
      checkpointRows: count('rust_checkpoint_v3_metadata'),
      historyRows: count('rust_generation_history_v1'),
      hallOfFameRows: count('rust_hall_of_fame_v1'),
      retention
    };
  } finally { database.close(); }
}

/** Prove managed files exactly match live checkpoint and Hall-of-Fame references. */
async function auditManagedFiles(databasePath: string): Promise<Record<string, unknown>> {
  const database = new Database(databasePath, { readonly: true, fileMustExist: true });
  const expected = new Set<string>();
  try {
    const checkpoints = database.prepare(`SELECT metadata.relative_filename AS filename
      FROM rust_checkpoint_v3_metadata AS metadata
      JOIN rust_checkpoint_retention_v1 AS retention USING (checkpoint_id)
      WHERE retention.retention_kind IN ('automatic', 'pinned')`).all() as Array<{ filename: string }>;
    const winners = database.prepare(`SELECT DISTINCT weights.relative_filename AS filename
      FROM rust_hall_of_fame_weights_v1 AS weights
      JOIN rust_hall_of_fame_v1 AS hall ON hall.weights_sha256 = weights.logical_sha256`).all() as Array<{ filename: string }>;
    for (const row of [...checkpoints, ...winners]) expected.add(row.filename);
  } finally { database.close(); }
  const actual = new Set<string>();
  for (const entry of await readdir(`${databasePath}.checkpoints`, { withFileTypes: true })) {
    if (!entry.isFile()) throw new Error(`managed directory contains a non-file: ${entry.name}`);
    actual.add(entry.name);
  }
  const missing = [...expected].filter(filename => !actual.has(filename));
  const unreferenced = [...actual].filter(filename => !expected.has(filename));
  if (missing.length > 0 || unreferenced.length > 0) {
    throw new Error(`managed file references mismatch: ${missing.length} missing, ${unreferenced.length} unreferenced`);
  }
  return { referencedFiles: expected.size, physicalFiles: actual.size,
    missingFiles: missing.length, unreferencedFiles: unreferenced.length };
}

/** Protect one early checkpoint so the long fixture exercises owner-pinned retention. */
async function pinCheckpoint(port: number): Promise<FixturePin> {
  const response = await fetch(`http://127.0.0.1:${port}/api/checkpoints/current/pin`, {
    method: 'POST', signal: AbortSignal.timeout(60_000)
  });
  const pinned = await response.json() as FixturePin;
  if (!response.ok || pinned.ok !== true || !/^[0-9a-f]{64}$/u.test(pinned.checkpointId) ||
      !/^[0-9a-f]{16}$/u.test(pinned.generation)) {
    throw new Error('Rust fixture failed to pin its early checkpoint');
  }
  return pinned;
}

/** Execute one fresh production-path workload and retain its disposable evidence on disk. */
export async function run(options: Options): Promise<Record<string, unknown>> {
  await mkdir(dirname(options.databasePath), { recursive: true });
  const server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, dbPath: options.databasePath,
    resume: options.resumeExisting ? 'latest' : 'fresh',
    ...(!options.resumeExisting ? { seed: options.seed } : {}),
    rustCalculationWorkers: options.rustWorkers, logLevel: 'error' });
  if (server.startupFault) {
    await server.close();
    throw new Error(`Rust fixture startup failed: ${server.startupFault}`);
  }
  let peak: StorageSample | undefined;
  let lastGeneration = 0;
  let startedGeneration: number | undefined;
  let pinnedCheckpoint: FixturePin | undefined;
  const startedAt = performance.now();
  let finalHealth: FixtureHealth | undefined;
  let sampling = false;
  let samplingFailure: unknown;
  let sampler: Promise<void> | undefined;
  try {
    if (!options.resumeExisting) {
      await configureWorkload(server.port, options);
      pinnedCheckpoint = await pinCheckpoint(server.port);
    }
    sampling = true;
    sampler = (async () => {
      while (sampling) {
        try {
          const storage = await sampleStorage(options.databasePath);
          if (!peak || storage.totalBytes > peak.totalBytes) peak = storage;
        } catch (error) {
          samplingFailure = error;
          return;
        }
        await new Promise<void>(done => setTimeout(done, 100));
      }
    })();
    for (;;) {
      if (samplingFailure) throw samplingFailure;
      const response = await fetch(`http://127.0.0.1:${server.port}/api/health`, {
        signal: AbortSignal.timeout(5000)
      });
      const health = await response.json() as FixtureHealth;
      if (!health.ok) throw new Error(`Rust fixture fault: ${health.faultCode ?? 'unknown'}`);
      if (health.seed !== options.seed) throw new Error('resumed fixture seed differs from the requested fixture');
      const generation = Number.parseInt(health.generation, 16);
      if (!Number.isSafeInteger(generation) || generation < 1) throw new Error('invalid Rust generation');
      startedGeneration ??= generation;
      const storage = await sampleStorage(options.databasePath);
      if (!peak || storage.totalBytes > peak.totalBytes) peak = storage;
      if (options.scenario === 'P3' && generation === 1) {
        process.stderr.write(`p3-step=${health.completedStep} state=${health.loopState} boundaries=${health.commandServiceBoundaries} work=${health.coordinatorWorkBytes}\n`);
      }
      if (generation > lastGeneration && (generation === 1 || generation % 20 === 1 ||
          generation >= options.generations + 1)) {
        process.stderr.write(`generation=${generation} stored=${storage.totalBytes} managed=${storage.managedBytes}\n`);
      }
      lastGeneration = generation;
      if (generation >= options.generations + 1) { finalHealth = health; break; }
      await new Promise<void>(done => setTimeout(done, 1000));
    }
  } finally {
    sampling = false;
    await sampler;
    await server.close();
  }
  if (samplingFailure) throw samplingFailure;
  const finalStorage = await sampleStorage(options.databasePath);
  const persistence = new CheckpointPersistenceClient({ databasePath: options.databasePath,
    managedRootPath: `${options.databasePath}.checkpoints`, existingOnly: true });
  let retention;
  try { retention = await persistence.inspectRetention(); }
  finally { await persistence.close(); }
  if (Number.parseInt(finalHealth?.generation ?? '', 16) < options.generations + 1 ||
      retention.plannedPrune.checkpointCount !== 0) {
    throw new Error('fixture stopped without every generation commit and retention cleanup');
  }
  if (BigInt(`0x${retention.automaticStoredByteCount}`) >
      BigInt(`0x${retention.automaticByteCap}`)) {
    throw new Error('automatic retained checkpoint bytes exceeded the selected budget');
  }
  const metadata = inspectMetadata(options.databasePath);
  const managedFileAudit = await auditManagedFiles(options.databasePath);
  if (Number(metadata['historyRows']) < options.generations ||
      Number(metadata['hallOfFameRows']) < options.generations ||
      Number(metadata['checkpointRows']) < options.generations + 2) {
    throw new Error('fixture omitted durable generation history, Hall of Fame, or checkpoint metadata');
  }
  const peakExcludingPinned = (peak?.totalBytes ?? 0) - Number.parseInt(retention.pinnedStoredByteCount, 16);
  const physicalBudgetMet = BigInt(peakExcludingPinned) <= BigInt(`0x${retention.automaticByteCap}`);
  return { scenario: options.scenario, seed: options.seed, requestedCompletedGenerations: options.generations,
    resumedFromExisting: options.resumeExisting, startedGeneration, pinnedCheckpoint,
    runId: finalHealth?.runId, generation: finalHealth?.generation,
    completedStep: finalHealth?.completedStep, elapsedWallSeconds: (performance.now() - startedAt) / 1000,
    peakObservedStorage: peak, peakObservedExcludingPinnedBytes: peakExcludingPinned,
    physicalBudgetMet, finalStorage,
    finalPhysicalExcludingPinnedBytes: finalStorage.totalBytes - Number.parseInt(retention.pinnedStoredByteCount, 16),
    retention, metadata, managedFileAudit };
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  void run(parseOptions(process.argv.slice(2)))
    .then(result => {
      process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
      if (result['physicalBudgetMet'] !== true) process.exitCode = 1;
    })
    .catch(error => { console.error(error); process.exitCode = 1; });
}
