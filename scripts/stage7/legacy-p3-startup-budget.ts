/** Exercise full P3 new-run budget admission through supported v2 SQLite conversion. */
import { createHash } from 'node:crypto';
import { existsSync } from 'node:fs';
import { mkdir, readdir, stat, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import Database from 'better-sqlite3';
import WebSocket from 'ws';
import { DEFAULT_CONFIG } from '../../server/config.ts';
import { startRustServer, type RustServer } from '../../server/rustServer.ts';
import { compileGraph } from '../../src/brains/graph/compiler.ts';
import { buildLargeBrainGraph } from '../fixtures/largeBrainGraph.ts';

/** Full supported population for the P3 capacity case. */
const POPULATION = 300;
/** Reproducible source seed; compatibility conversion creates a new run identity. */
const SEED = 0x5a17c0de;
/** Source identity that must never be advertised as exact Rust continuation. */
const LEGACY_RUN_ID = 'legacy-p3-startup-budget-source';

/** Small public health facts used after each real startup attempt. */
interface Health {
  /** HTTP response code, retained independently from the JSON result. */
  status: number;
  /** Whether running Rust authority was published. */
  ok: boolean;
  /** Fault-only startup identity when no authority exists. */
  authority?: string;
  /** Startup fault lifecycle returned by the health-only server. */
  lifecycle?: string;
  /** Public startup rejection. */
  interfaceFault?: string;
  /** Current managed run identity. */
  runId?: string;
  /** Exact committed checkpoint selected by the server. */
  startupCheckpointId?: string;
  /** Source-validated production addon identifier. */
  nativeBuildIdentifier?: string;
  /** Exact current generation. */
  generation?: string;
  /** Durable conversion provenance. */
  legacyConversion?: { sourceSnapshotId: number; sourceFormat: string;
    completeness: string; exactContinuation: boolean };
}

/** Build one bounded packed genome at a time, outside the production startup measurement. */
function createSource(databasePath: string): { parameterCount: number; sourceDigest: string } {
  const graphSpec = buildLargeBrainGraph(147);
  const graph = compileGraph(graphSpec);
  if (graph.totalParams < 400_000) throw new Error('P3 graph omitted its large-brain workload');
  const metadata = JSON.stringify({ formatVersion: 2, boundaryVersion: 1,
    boundaryKind: 'run-start', resumable: true, generation: 1, simulationStep: 0,
    runId: LEGACY_RUN_ID, worldSeed: SEED, configHash: 'legacy-p3-budget-fixture',
    configRevision: 1, archKey: graph.key, graphSpec, populationCount: POPULATION,
    settings: { snakeCount: POPULATION, simSpeed: 1, hiddenLayers: 5,
      neurons1: 256, neurons2: 256, neurons3: 256, neurons4: 256, neurons5: 256 },
    updates: [{ path: 'generationSeconds', value: 60 },
      { path: 'sense.bubbleBins', value: 32 }, { path: 'baselineBots.count', value: 10 },
      { path: 'pelletCountTarget', value: 3500 }],
    rng: {}, allocators: {}, bestFitnessEver: 0, fitnessHistory: [], lastHofEntry: null });
  const database = new Database(databasePath);
  try {
    database.pragma('journal_mode = WAL');
    database.pragma('synchronous = FULL');
    database.exec(`CREATE TABLE population_snapshots (
      id INTEGER PRIMARY KEY, payload_json TEXT, format_version INTEGER,
      boundary_kind TEXT, population_count INTEGER
    ); CREATE TABLE snapshot_genomes (
      snapshot_id INTEGER NOT NULL, slot INTEGER NOT NULL, arch_key TEXT NOT NULL,
      brain_type TEXT NOT NULL, fitness REAL NOT NULL, weight_count INTEGER NOT NULL,
      weights_blob BLOB NOT NULL, weights_checksum TEXT NOT NULL,
      PRIMARY KEY (snapshot_id, slot)
    )`);
    const insert = database.prepare(`INSERT INTO snapshot_genomes
      (snapshot_id, slot, arch_key, brain_type, fitness, weight_count, weights_blob, weights_checksum)
      VALUES (1, ?, ?, 'mlp', ?, ?, ?, ?)`);
    const weights = Buffer.alloc(graph.totalParams * Float32Array.BYTES_PER_ELEMENT);
    let random = SEED;
    database.transaction(() => {
      database.prepare(`INSERT INTO population_snapshots
        (id, payload_json, format_version, boundary_kind, population_count)
        VALUES (1, ?, 2, 'run-start', ?)`).run(metadata, POPULATION);
      for (let slot = 0; slot < POPULATION; slot++) {
        for (let index = 0; index < graph.totalParams; index++) {
          random ^= random << 13;
          random ^= random >>> 17;
          random ^= random << 5;
          weights.writeFloatLE(((random >>> 0) / 0x1_0000_0000 - 0.5) * 0.1, index * 4);
        }
        insert.run(slot, graph.key, POPULATION - slot, graph.totalParams, weights,
          createHash('sha256').update(weights).digest('hex'));
      }
    })();
    database.pragma('wal_checkpoint(TRUNCATE)');
  } finally { database.close(); }
  return { parameterCount: graph.totalParams, sourceDigest: inspectSource(databasePath) };
}

/** Independently hash every retained source row without collecting the population. */
function inspectSource(databasePath: string): string {
  const database = new Database(databasePath, { readonly: true, fileMustExist: true });
  try {
    const parents = database.prepare('SELECT * FROM population_snapshots ORDER BY id').all();
    if (parents.length !== 1) throw new Error('legacy source parent count changed');
    const digest = createHash('sha256').update(JSON.stringify(parents));
    let rows = 0;
    for (const row of database.prepare('SELECT * FROM snapshot_genomes ORDER BY slot').iterate()) {
      const { weights_blob: weights, ...metadata } = row as Record<string, unknown> & { weights_blob: Buffer };
      if (metadata['slot'] !== rows || createHash('sha256').update(weights).digest('hex') !==
          metadata['weights_checksum']) throw new Error(`legacy source genome changed at slot ${rows}`);
      digest.update(JSON.stringify(metadata)).update(weights);
      rows++;
    }
    if (rows !== POPULATION) throw new Error('legacy source population count changed');
    return digest.digest('hex');
  } finally { database.close(); }
}

/** Inspect actual durable records and files after the server and its worker have closed. */
async function durableState(databasePath: string): Promise<Record<string, unknown>> {
  const database = new Database(databasePath, { readonly: true, fileMustExist: true });
  try {
    /** Count only the fixed managed tables named below. */
    const count = (table: string): number =>
      (database.prepare(`SELECT count(*) AS n FROM ${table}`).get() as { n: number }).n;
    const files = await readdir(`${databasePath}.checkpoints`);
    return { activeRows: count('rust_active_run_v1'),
      currentRows: count('rust_checkpoint_v3_current'),
      current: database.prepare('SELECT run_id, checkpoint_id FROM rust_checkpoint_v3_current').all(),
      checkpointRows: count('rust_checkpoint_v3_metadata'), files,
      sourceDigest: inspectSource(databasePath),
      databaseBytes: (await stat(databasePath)).size };
  } finally { database.close(); }
}

/** Read only bounded health JSON from the real HTTP server. */
async function health(port: number): Promise<Health> {
  const response = await fetch(`http://127.0.0.1:${port}/api/health`, {
    signal: AbortSignal.timeout(10_000)
  });
  const value = await response.json() as Omit<Health, 'status'>;
  return { ...value, status: response.status };
}

/** A faulted startup must reject a real WebSocket upgrade with no successful session. */
async function rejectedUpgrade(port: number): Promise<number> {
  const socket = new WebSocket(`ws://127.0.0.1:${port}`);
  return new Promise<number>((done, reject) => {
    let finished = false;
    const timer = setTimeout(() => finish(new Error('faulted startup upgrade did not answer')), 10_000);
    /** Complete the one bounded exchange and release its socket. */
    function finish(error?: Error, status?: number): void {
      if (finished) return;
      finished = true;
      clearTimeout(timer);
      socket.terminate();
      if (error) reject(error);
      else done(status ?? 0);
    }
    socket.on('unexpected-response', (_request, response) => {
      response.resume();
      if (response.statusCode !== 503) finish(new Error(`unexpected upgrade status ${response.statusCode}`));
      else finish(undefined, response.statusCode);
    });
    socket.on('open', () => finish(new Error('faulted startup accepted a WebSocket session')));
    socket.on('error', error => finish(error));
  });
}

/** Verify the converted authority actually advertises the full supported P3 configuration. */
async function workloadWelcome(port: number, parameterCount: number): Promise<Record<string, unknown>> {
  const socket = new WebSocket(`ws://127.0.0.1:${port}`);
  return new Promise<Record<string, unknown>>((done, reject) => {
    let finished = false;
    const timer = setTimeout(() => finish(new Error('converted P3 welcome did not arrive')), 10_000);
    /** Return one small provenance record and release the observation-only connection. */
    function finish(error?: Error, value?: Record<string, unknown>): void {
      if (finished) return;
      finished = true;
      clearTimeout(timer);
      socket.terminate();
      if (error) reject(error);
      else done(value ?? {});
    }
    socket.on('open', () => socket.send(JSON.stringify({ type: 'hello', version: 2, clientType: 'bot' })));
    socket.on('error', error => finish(error));
    socket.on('close', () => finish(new Error('converted P3 connection closed before welcome')));
    socket.on('message', (data, binary) => {
      if (binary || finished) return;
      try {
        const value = JSON.parse(data.toString()) as Record<string, unknown>;
        if (value['type'] !== 'welcome') return;
        const settings = value['settings'] as { core: { snakeCount: number; simSpeed: number };
          updates: Array<{ path: string; value: unknown }> };
        const sensors = value['sensorSpec'] as { sensorCount: number };
        const inference = value['inferenceMode'] as { activeBackend: string;
          activeWorkerCount: number; parameterCount: number };
        if (settings.core.snakeCount !== POPULATION || settings.core.simSpeed !== 1 ||
            sensors.sensorCount !== 147 || inference.activeBackend !== 'native' ||
            inference.activeWorkerCount !== 6 || inference.parameterCount !== parameterCount ||
            !settings.updates.some(update => update.path === 'baselineBots.count' && update.value === 10) ||
            !settings.updates.some(update => update.path === 'pelletCountTarget' && update.value === 3500)) {
          throw new Error('converted startup did not select the actual full P3 workload');
        }
        finish(undefined, { settings, sensorSpec: sensors, inferenceMode: inference,
          runId: value['runId'], configHash: value['configHash'], worldSeed: value['worldSeed'] });
      } catch (error) { finish(error instanceof Error ? error : new Error(String(error))); }
    });
  });
}

/** Measure one asynchronous startup without retaining process or population snapshots. */
async function measuredStartup(databasePath: string, checkpointBudgetMiB: number): Promise<{
  server: RustServer; measurement: Record<string, unknown>
}> {
  const began = performance.now();
  let lastSample = began;
  const initialRssBytes = process.memoryUsage().rss;
  let peakRssBytes = initialRssBytes;
  const delays: number[] = [];
  const timer = setInterval(() => {
    const now = performance.now();
    if (delays.length < 50_000) delays.push(Math.max(0, now - lastSample - 10));
    lastSample = now;
    peakRssBytes = Math.max(peakRssBytes, process.memoryUsage().rss);
  }, 10);
  try {
    const server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, dbPath: databasePath,
      resume: 'latest', checkpointBudgetMiB, rustCalculationWorkers: 6, logLevel: 'error' });
    const ordered = delays.sort((a, b) => a - b);
    return { server, measurement: { wallSeconds: (performance.now() - began) / 1000,
      initialRssBytes,
      sampledPeakRssBytes: Math.max(peakRssBytes, process.memoryUsage().rss),
      eventLoopDelaySamples: ordered.length,
      eventLoopDelayP95Ms: ordered[Math.ceil(ordered.length * 0.95) - 1] ?? null,
      eventLoopDelayMaxMs: ordered.at(-1) ?? null } };
  } finally { clearInterval(timer); }
}

/** Run rejection, adequate-budget conversion and exact managed restart on one new fixture. */
export async function run(databasePath: string): Promise<Record<string, unknown>> {
  if (existsSync(databasePath) || existsSync(`${databasePath}.checkpoints`)) {
    throw new Error('legacy budget fixture requires absent database and managed paths');
  }
  await mkdir(dirname(databasePath), { recursive: true });
  const source = createSource(databasePath);
  const rejected = await measuredStartup(databasePath, 1280);
  let faultHealth: Health;
  let upgradeStatus: number;
  let minimumBytes: number;
  try {
    const match = /^checkpoint budget [0-9]+ bytes .*requires at least ([0-9]+) bytes$/u
      .exec(rejected.server.startupFault ?? '');
    if (!match) throw new Error(`P3 conversion failed for another reason: ${rejected.server.startupFault}`);
    minimumBytes = Number(match[1]);
    if (!Number.isSafeInteger(minimumBytes) || minimumBytes <= 1280 * 1024 * 1024) {
      throw new Error('P3 fixture did not exceed the selected minimum budget');
    }
    faultHealth = await health(rejected.server.port);
    if (faultHealth.status !== 503 || faultHealth.ok || faultHealth.authority !== 'rust' ||
        faultHealth.lifecycle !== 'startup-fault') throw new Error('rejected startup published healthy authority');
    upgradeStatus = await rejectedUpgrade(rejected.server.port);
  } finally { await rejected.server.close(); }
  const rejectedDurable = await durableState(databasePath);
  if (rejectedDurable['activeRows'] !== 0 || rejectedDurable['currentRows'] !== 0 ||
      rejectedDurable['checkpointRows'] !== 0 || (rejectedDurable['files'] as string[]).length !== 0 ||
      rejectedDurable['sourceDigest'] !== source.sourceDigest) {
    throw new Error(`rejected startup changed durable state: ${JSON.stringify(rejectedDurable)}`);
  }
  const retryBudgetMiB = Math.ceil(minimumBytes / (1024 * 1024)) + 16;
  const admitted = await measuredStartup(databasePath, retryBudgetMiB);
  let admittedHealth: Health;
  let welcome: Record<string, unknown>;
  try {
    if (admitted.server.startupFault) throw new Error(`adequate budget rejected: ${admitted.server.startupFault}`);
    admittedHealth = await health(admitted.server.port);
    if (!admittedHealth.ok || admittedHealth.status !== 200 || admittedHealth.runId === LEGACY_RUN_ID ||
        admittedHealth.generation !== '0000000000000001' ||
        admittedHealth.legacyConversion?.sourceFormat !== 'typescript-v2' ||
        admittedHealth.legacyConversion.exactContinuation !== false ||
        admittedHealth.legacyConversion.completeness !== 'population-only') {
      throw new Error('admitted conversion did not expose its new managed run honestly');
    }
    welcome = await workloadWelcome(admitted.server.port, source.parameterCount);
  } finally { await admitted.server.close(); }
  const admittedDurable = await durableState(databasePath);
  // Initial conversion uses the sole-current-run selection convention; explicit run switches
  // publish an active selection row. Require the actual current identity before restarting.
  const current = admittedDurable['current'] as Array<{ run_id: string; checkpoint_id: string }>;
  if (admittedDurable['activeRows'] !== 0 || admittedDurable['currentRows'] !== 1 ||
      current[0]?.run_id !== admittedHealth.runId ||
      current[0]?.checkpoint_id !== admittedHealth.startupCheckpointId ||
      admittedDurable['checkpointRows'] !== 1 || (admittedDurable['files'] as string[]).length !== 1 ||
      admittedDurable['sourceDigest'] !== source.sourceDigest) {
    throw new Error(`conversion did not retain one clean boundary: ${JSON.stringify(admittedDurable)}`);
  }
  const restarted = await measuredStartup(databasePath, retryBudgetMiB);
  let restartHealth: Health;
  try {
    if (restarted.server.startupFault) throw new Error(`converted restart failed: ${restarted.server.startupFault}`);
    restartHealth = await health(restarted.server.port);
    if (!restartHealth.ok || restartHealth.runId !== admittedHealth.runId ||
        restartHealth.startupCheckpointId !== admittedHealth.startupCheckpointId) {
      throw new Error('converted checkpoint did not restart exactly');
    }
  } finally { await restarted.server.close(); }
  if (inspectSource(databasePath) !== source.sourceDigest) throw new Error('restart changed the retained legacy source');
  return { scenario: 'P3', passed: true, population: POPULATION, ...source,
    scope: 'A full-size packed v2 source converts to a new managed generation-one P3 run through normal production startup. Rejection precedes SQLite current-pointer publication and leaves no managed orphan. This does not add P3 settings to --fresh, prove exact legacy continuation, or measure real-time P3 performance. Startup time and timer-delay sampling exclude fixture construction. RSS is cumulative in one process and can include retained fixture or earlier-startup allocations; it is not an isolated production memory gate. No control/health latency is claimed before the listener opens.',
    rejectedBudgetMiB: 1280, minimumBytes, retryBudgetMiB,
    rejected: { health: faultHealth, upgradeStatus, durable: rejectedDurable, measurement: rejected.measurement },
    admitted: { health: admittedHealth, welcome, durable: admittedDurable, measurement: admitted.measurement },
    restart: { health: restartHealth, measurement: restarted.measurement } };
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const args = process.argv.slice(2);
  if (args.length !== 4 || args[0] !== '--db-path' || !args[1] || args[2] !== '--output' || !args[3]) {
    throw new Error('usage: --db-path NEW_DATABASE --output NEW_REPORT');
  }
  const output = resolve(args[3]);
  if (existsSync(output)) throw new Error('report destination already exists');
  void run(resolve(args[1])).then(async result => {
    await mkdir(dirname(output), { recursive: true });
    await writeFile(output, `${JSON.stringify(result, null, 2)}\n`);
    process.stdout.write(`${JSON.stringify(result)}\n`);
  }).catch(error => { process.stderr.write(`${String(error)}\n`); process.exitCode = 1; });
}
