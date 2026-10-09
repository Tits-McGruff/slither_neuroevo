/** Produce fresh and evolved P0/P1/P2/P3 saves through the unchanged production Rust authority. */
import { createWriteStream, existsSync } from 'node:fs';
import { mkdir, realpath, rm, stat, statfs, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { Writable } from 'node:stream';
import Database from 'better-sqlite3';
import WebSocket from 'ws';
import { DEFAULT_CONFIG } from '../../server/config.ts';
import { startRustServer } from '../../server/rustServer.ts';
import { buildLargeBrainGraph } from '../stage2/fixtures.ts';

/** Workload names and an absent, explicitly selected scratch destination. */
interface Options {
  /** Approved population and graph size. */
  scenario: 'P0' | 'P1' | 'P2' | 'P3';
  /** Directory created by this invocation only. */
  outputRoot: string;
}

/** Scalar identity read from the actual durable current pointer. */
interface Identity {
  /** Active experiment identity. */
  runId: string;
  /** Selected immutable checkpoint. */
  checkpointId: string;
  /** Exact boundary generation. */
  generation: string;
}

/** Require an explicit workload and an absent task directory. */
function options(): Options {
  const [scenarioFlag, scenario, rootFlag, root, ...extra] = process.argv.slice(2);
  if (scenarioFlag !== '--scenario' || !['P0', 'P1', 'P2', 'P3'].includes(scenario ?? '') ||
      rootFlag !== '--output-root' || !root || extra.length) {
    throw new Error('usage: --scenario P0|P1|P2|P3 --output-root NEW_DIRECTORY');
  }
  const outputRoot = resolve(root);
  if (existsSync(outputRoot)) throw new Error('output directory already exists');
  return { scenario: scenario as Options['scenario'], outputRoot };
}

/** Exchange one bounded reliable command; binary display frames are ignored. */
async function exchange(socket: WebSocket, command: Record<string, unknown>,
  matches: (message: Record<string, unknown>) => boolean): Promise<Record<string, unknown>> {
  return new Promise((resolveReply, reject) => {
    const timeout = setTimeout(() => finish(new Error('production command timed out')), 180_000);
    /** Remove only this exchange's listeners before resolving or rejecting. */
    function finish(error?: Error, message?: Record<string, unknown>): void {
      clearTimeout(timeout);
      socket.off('message', receive);
      socket.off('error', failed);
      socket.off('close', closed);
      if (error) reject(error);
      else resolveReply(message!);
    }
    /** Decode the small reliable message, without retaining binary world frames. */
    function receive(data: WebSocket.RawData, binary: boolean): void {
      if (binary) return;
      try {
        const message = JSON.parse(data.toString()) as Record<string, unknown>;
        if (message['type'] === 'error') finish(new Error(String(message['message'])));
        else if (matches(message)) finish(undefined, message);
      } catch (error) { finish(error instanceof Error ? error : new Error(String(error))); }
    }
    /** Reject a failed socket. */
    function failed(error: Error): void { finish(error); }
    /** Reject an interrupted command. */
    function closed(): void { finish(new Error('production socket closed')); }
    socket.on('message', receive);
    socket.on('error', failed);
    socket.on('close', closed);
    socket.send(JSON.stringify(command));
  });
}

/** Apply the supported simulation rate and require its authoritative acknowledgement. */
async function speed(socket: WebSocket, value: number): Promise<void> {
  const requestId = `codec-speed-${value}`;
  const reply = await exchange(socket, { type: 'settings', requestId,
    updates: [{ path: 'simSpeed', value }] }, message =>
    message['type'] === 'settingsApplied' && message['requestId'] === requestId);
  if (reply['applied'] !== true) throw new Error('Rust rejected the requested rate');
}

/** Read only SQLite scalar metadata; no population rows or files are loaded in Node. */
function identity(databasePath: string): Identity {
  const database = new Database(databasePath, { readonly: true, fileMustExist: true });
  try {
    const row = database.prepare(`SELECT current.run_id AS runId,
      current.checkpoint_id AS checkpointId, metadata.generation_hex AS generation
      FROM rust_active_run_v1 AS active
      JOIN rust_checkpoint_v3_current AS current ON current.run_id = active.run_id
      JOIN rust_checkpoint_v3_metadata AS metadata ON metadata.checkpoint_id = current.checkpoint_id
      WHERE active.singleton = 1`).get() as Identity | undefined;
    if (!row || !/^[0-9a-f]{64}$/u.test(row.checkpointId)) throw new Error('missing current checkpoint');
    return row;
  } finally { database.close(); }
}

/** Stream one real production export and verify its selected checkpoint and exact length. */
async function save(port: number, databasePath: string, path: string): Promise<Record<string, unknown>> {
  const selected = identity(databasePath);
  const started = performance.now();
  const response = await fetch(`http://127.0.0.1:${port}/api/export/latest`, {
    signal: AbortSignal.timeout(180_000)
  });
  if (!response.ok || !response.body || response.headers.get('x-slither-checkpoint-id') !== selected.checkpointId) {
    throw new Error(`production export failed or selected another checkpoint: ${response.status}`);
  }
  await response.body.pipeTo(Writable.toWeb(createWriteStream(path, { flags: 'wx' })));
  const bytes = (await stat(path)).size;
  if (bytes !== Number(response.headers.get('content-length'))) throw new Error('export length mismatch');
  return { ...selected, bytes, exportWallMs: performance.now() - started,
    saveRoot: response.headers.get('x-slither-save-root') };
}

/** Verify the exact task parent and remove its stopped database, sidecars and managed files. */
async function removeDatabase(outputRoot: string, createdRoot: string, databasePath: string): Promise<void> {
  if (await realpath(outputRoot) !== createdRoot || databasePath !== resolve(createdRoot, 'fixture.db')) {
    throw new Error('cleanup parent changed');
  }
  await rm(`${databasePath}.checkpoints`, { recursive: true, force: true });
  for (const path of [databasePath, `${databasePath}-wal`, `${databasePath}-shm`]) await rm(path, { force: true });
}

/** Advance actual physics, controls and evolution, then preserve only the two save fixtures. */
async function run(request: Options): Promise<void> {
  await mkdir(request.outputRoot, { recursive: true });
  const createdRoot = await realpath(request.outputRoot);
  const filesystem = await statfs(request.outputRoot, { bigint: true });
  const freeBytes = filesystem.bavail * filesystem.bsize;
  if (freeBytes < 12n * 1024n ** 3n) throw new Error('codec fixture requires 12 GiB free');
  const databasePath = resolve(createdRoot, 'fixture.db');
  const started = performance.now();
  const server = await startRustServer({ ...DEFAULT_CONFIG, host: '127.0.0.1', port: 0,
    dbPath: databasePath, seed: 1511506142, resume: 'fresh', rustCalculationWorkers: 6, logLevel: 'error' });
  let socket: WebSocket | undefined;
  try {
    if (server.startupFault) throw new Error(`Rust startup fault: ${server.startupFault}`);
    socket = new WebSocket(`ws://127.0.0.1:${server.port}`);
    const connection = socket;
    await new Promise<void>((done, reject) => {
      const timeout = setTimeout(() => { connection.terminate(); reject(new Error('socket open timeout')); }, 10_000);
      connection.once('open', () => { clearTimeout(timeout); done(); });
      connection.once('error', error => { clearTimeout(timeout); reject(error); });
    });
    await exchange(socket, { type: 'hello', version: 2, clientType: 'ui' }, message => message['type'] === 'welcome');
    socket.send(JSON.stringify({ type: 'join', mode: 'spectator' }));
    const large = request.scenario === 'P2' || request.scenario === 'P3';
    const snakeCount = request.scenario === 'P1' || request.scenario === 'P3' ? 300 : 55;
    const reset = await exchange(socket, { type: 'reset', settings: { snakeCount, simSpeed: 0.1,
      ...(large ? { hiddenLayers: 5, neurons1: 256, neurons2: 256, neurons3: 256, neurons4: 256, neurons5: 256 } : {}) },
      updates: [{ path: 'generationSeconds', value: 8 }, { path: 'sense.bubbleBins', value: large ? 32 : 16 },
        { path: 'baselineBots.count', value: 10 }, { path: 'pelletCountTarget', value: 3500 }],
      graphSpec: large ? buildLargeBrainGraph(147) : null }, message =>
      message['type'] === 'stateReplaced' && message['reason'] === 'reset');
    const welcome = reset['welcome'] as { inferenceMode: { parameterCount: number };
      sensorSpec: { sensorCount: number }; settings: { core: { snakeCount: number };
        updates: Array<{ path: string; value: unknown }> } };
    if (welcome.settings.core.snakeCount !== snakeCount ||
        welcome.sensorSpec.sensorCount !== (large ? 147 : 83) ||
        welcome.inferenceMode.parameterCount !== (large ? 402_914 : 13_458) ||
        ![{ path: 'generationSeconds', value: 8 }, { path: 'baselineBots.count', value: 10 },
          { path: 'pelletCountTarget', value: 3500 }].every(expected => welcome.settings.updates.some(
            actual => actual.path === expected.path && actual.value === expected.value))) {
      throw new Error('workload was not accepted');
    }
    socket.send(JSON.stringify({ type: 'join', mode: 'spectator', rejoinToken: reset['rejoinToken'] }));
    const fresh = await save(server.port, databasePath, resolve(request.outputRoot, 'fresh.slither-save'));
    if (fresh['generation'] !== '0000000000000001') throw new Error('fresh export passed generation one');
    process.stderr.write(`${request.scenario} fresh=${fresh['bytes']}B\n`);
    await speed(socket, 12);
    let reported = 1;
    for (;;) {
      if (performance.now() - started > 600_000) throw new Error('codec fixture exceeded ten minutes');
      const response = await fetch(`http://127.0.0.1:${server.port}/api/health`, { signal: AbortSignal.timeout(10_000) });
      const health = await response.json() as { ok: boolean; generation: string; completedStep: string; interfaceFault?: string };
      if (!response.ok || !health.ok) throw new Error(`production fault: ${health.interfaceFault}`);
      const generation = Number.parseInt(health.generation, 16);
      if (generation !== reported) {
        process.stderr.write(`${request.scenario} generation=${generation} step=${health.completedStep}\n`);
        reported = generation;
      }
      if (generation >= 13) break;
      await new Promise<void>(done => setTimeout(done, 1000));
    }
    await speed(socket, 0.1);
    const evolved = await save(server.port, databasePath, resolve(request.outputRoot, 'evolved.slither-save'));
    if (Number.parseInt(String(evolved['generation']), 16) < 13 || evolved['runId'] !== fresh['runId']) {
      throw new Error('evolved export has the wrong durable identity');
    }
    await writeFile(resolve(request.outputRoot, 'fixture.json'), JSON.stringify({ scenario: request.scenario,
      freeBytes: freeBytes.toString(), snakeCount, parameterCount: welcome.inferenceMode.parameterCount,
      rustWorkers: 6, generationSeconds: 8, fresh, evolved,
      wallSeconds: (performance.now() - started) / 1000,
      scope: 'Unchanged production Rust physics, sensors, inference, evolution and HTTP export; accelerated rate only. Export wall time is not isolated Rust codec timing.' }, null, 2) + '\n', { flag: 'wx' });
  } finally {
    socket?.terminate();
    await server.close();
    await removeDatabase(request.outputRoot, createdRoot, databasePath);
  }
}

void run(options()).catch(error => { console.error(error); process.exitCode = 1; });
