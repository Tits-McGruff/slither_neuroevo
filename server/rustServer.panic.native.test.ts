/** Real release-addon worker panic through the production HTTP/WebSocket router. */
import { createRequire } from 'node:module';
import { createHash } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { once } from 'node:events';
import { spawn, type ChildProcess } from 'node:child_process';
import { createInterface } from 'node:readline';
import { mkdtemp, readdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { expect, it, vi, type MockInstance } from 'vitest';
import WebSocket from 'ws';
import Database from 'better-sqlite3';
import { DEFAULT_CONFIG } from './config.ts';
import { startRustServer, type RustServer } from './rustServer.ts';
import { validateExperimentalFreshRunBinding } from './rustEngine/experimentalFreshRunSession.ts';
import { computeNativeSourceIdentity } from './rustEngine/nativeSourceIdentity.ts';
import { configurePanicFixture, loadPanicBinding, panicFixtureFrameSteps, startPreparedPanicRuntime } from './test/panicRuntime.ts';
import { describeNetworkSuite } from './test/networkSuites.ts';
import { fixtureArchiveDownload } from './test/archiveDownload.ts';
import type { RustBackgroundHealth } from '../src/protocol/rustBackground.ts';
import type { AssignMsg, SensorsMsg } from './protocol.ts';
import type { ExperimentalRuntimeTelemetrySnapshot } from './rustEngine/runtimeTelemetry.ts';
import * as diskAdmission from './rustEngine/diskAdmission.ts';

vi.mock('./rustEngine/experimentalStartup.ts', async importOriginal => {
  const original = await importOriginal<typeof import('./rustEngine/experimentalStartup.ts')>();
  const fixture = await import('./test/panicRuntime.ts');
  return { ...original, createExperimentalServerRuntime: fixture.createPanicTestRuntime };
});

/** CommonJS production addon loader, independent of the feature addon fixture. */
const require = createRequire(import.meta.url);

/** Cold Node/tsx bootstrap, native loading and durable fixture startup have their own budget.
 * Live fault observation and stop/join deadlines below remain five seconds.
 */
const FIXTURE_STARTUP_TIMEOUT_MS = 10_000;

/** Health fields added by the actual HTTP router to the native protocol. */
interface Health extends RustBackgroundHealth {
  /** Honest success/fault status. */
  ok: boolean;
  /** Active durable lineage. */
  runId: string;
  /** Checkpoint selected before any failed step. */
  startupCheckpointId: string;
  /** Native or interface fault reported to the browser. */
  interfaceFault?: string;
  /** Production scalar counters for player and trainer input. */
  telemetry: ExperimentalRuntimeTelemetrySnapshot;
  /** Current authoritative settings identity. */
  configHash: string;
}

/** Poll a real HTTP outcome within the existing five-second integration deadline. */
async function healthUntil(port: number, predicate: (health: Health) => boolean): Promise<Health> {
  const deadline = performance.now() + 5000;
  let health: Health | undefined;
  do {
    health = await (await fetch(`http://127.0.0.1:${port}/api/health`)).json() as Health;
    if (predicate(health)) return health;
    await new Promise<void>(done => setTimeout(done, 10));
  } while (performance.now() < deadline);
  throw new Error(`HTTP condition not reached: ${JSON.stringify(health)}`);
}

/** Inspect the real current pointer and row counts independently from the persistence worker. */
function durableState(databasePath: string): unknown {
  const db = new Database(databasePath, { readonly: true });
  try {
    return {
      current: db.prepare('SELECT * FROM rust_checkpoint_v3_current ORDER BY run_id').all(),
      checkpoints: db.prepare('SELECT * FROM rust_checkpoint_v3_metadata ORDER BY checkpoint_id').all()
    };
  } finally { db.close(); }
}

/** Inspect all compact Rust metadata, including history, graphs, Hall of Fame and leases. */
function archiveMetadata(databasePath: string): Array<{ name: string; rows: unknown[] }> {
  const db = new Database(databasePath, { readonly: true });
  try {
    const tables = db.prepare(`SELECT name FROM sqlite_master WHERE type = 'table'
      AND name LIKE 'rust_%' ORDER BY name`).all() as Array<{ name: string }>;
    return tables.map(({ name }) => ({ name, rows: db.prepare(
      `SELECT * FROM "${name.replaceAll('"', '""')}" ORDER BY rowid`).all() }));
  } finally { db.close(); }
}

/** Hash the actual retained files without buffering their populations. */
async function archiveFiles(directory: string): Promise<Array<{ filename: string; sha256: string }>> {
  const result = [];
  for (const filename of (await readdir(directory)).sort()) {
    const hash = createHash('sha256');
    for await (const bytes of createReadStream(join(directory, filename))) hash.update(bytes);
    result.push({ filename, sha256: hash.digest('hex') });
  }
  return result;
}

/** Wait for actual export lease and scratch cleanup, preserving a hard failure deadline. */
async function exportClean(directory: string): Promise<void> {
  const deadline = performance.now() + 5000;
  let scratch: string[] = [];
  do {
    scratch = (await readdir(directory)).filter(name => name.includes('slither-save') ||
      name.includes('export-') || name.includes('import-') || name.includes('upload'));
    if (scratch.length === 0) return;
    await new Promise<void>(done => setTimeout(done, 10));
  } while (performance.now() < deadline);
  expect(scratch).toEqual([]);
}

/** One real external controller's current assignment and delivered observation. */
interface ExportController {
  /** Actual Protocol 2 connection. */
  socket: WebSocket;
  /** Latest Rust-issued lease identity. */
  assignment?: AssignMsg;
  /** Latest delivered observation for that assignment. */
  sample?: SensorsMsg;
  /** Bounded reliable packet evidence, excluding recurring stats and frames. */
  packets: Array<Record<string, unknown>>;
}

/** Await a socket outcome without retaining a task beyond its five-second deadline. */
async function controllerUntil(predicate: () => boolean, description: string | (() => string)): Promise<void> {
  const deadline = performance.now() + 5000;
  while (!predicate() && performance.now() < deadline) await new Promise<void>(done => setTimeout(done, 10));
  expect(predicate(), typeof description === 'string' ? description : description()).toBe(true);
}

/** Join a player or trainer through the actual production hub and Rust controller boundary. */
async function exportController(port: number, kind: 'ui' | 'bot', peers: WebSocket[]): Promise<ExportController> {
  const socket = new WebSocket(`ws://127.0.0.1:${port}`);
  peers.push(socket);
  const peer: ExportController = { socket, packets: [] };
  socket.on('error', error => peer.packets.push({ type: 'error', message: error.message }));
  socket.on('open', () => socket.send(JSON.stringify({ type: 'hello', version: 2, clientType: kind })));
  socket.on('message', (bytes, binary) => {
    if (binary) return;
    const packet = JSON.parse(bytes.toString()) as Record<string, unknown>;
    if (packet['type'] === 'welcome') socket.send(JSON.stringify({ type: 'join', mode: 'player', name: `Export-${kind}` }));
    if (packet['type'] === 'assign') { peer.assignment = packet as unknown as AssignMsg; delete peer.sample; }
    if (packet['type'] === 'stateReplaced') { delete peer.assignment; delete peer.sample; }
    if (packet['type'] === 'sensors' && packet['snakeId'] === peer.assignment?.snakeId) peer.sample = packet as unknown as SensorsMsg;
    if (['assign', 'settingsApplied', 'stateReplaced', 'error'].includes(String(packet['type'])) && peer.packets.length < 64) peer.packets.push(packet);
  });
  await controllerUntil(() => !!peer.assignment && !!peer.sample, `export controller did not join: ${kind}`);
  expect(peer.packets.filter(packet => packet['type'] === 'error')).toEqual([]);
  return peer;
}

/** Measure delivered heading using sensor-v3 sine/cosine and circular subtraction. */
function exportHeadingChange(before: SensorsMsg, after: SensorsMsg): number {
  const first = Math.atan2(before.sensors[0]!, before.sensors[1]!);
  const last = Math.atan2(after.sensors[0]!, after.sensors[1]!);
  return Math.atan2(Math.sin(last - first), Math.cos(last - first));
}

/** Launch the explicit supervisor fixture in its own real Node process. */
async function fixtureProcess(databasePath: string): Promise<{ child: ChildProcess; port: number }> {
  const child = spawn(process.execPath, ['--import', 'tsx',
    resolve('scripts/stage8/supervised-panic-server.ts')], {
    env: { ...process.env, SLITHER_PANIC_FIXTURE_DB: databasePath, SLITHER_PANIC_FIXTURE_PORT: '0' },
    stdio: ['ignore', 'pipe', 'pipe', 'ipc']
  });
  let didClose = false;
  child.once('close', () => { didClose = true; });
  let errors = '';
  child.stderr?.on('data', data => { errors = `${errors}${String(data)}`.slice(-8192); });
  const lines = createInterface({ input: child.stdout! });
  try {
    const port = await new Promise<number>((done, reject) => {
      const timeout = setTimeout(() => reject(new Error(`fixture startup timed out: ${errors}`)),
        FIXTURE_STARTUP_TIMEOUT_MS);
      child.once('error', error => {
        clearTimeout(timeout);
        reject(error);
      });
      child.once('exit', (code, signal) => {
        clearTimeout(timeout);
        reject(new Error(`fixture exited (${code}/${signal}): ${errors}`));
      });
      lines.on('line', line => {
        if (!line.startsWith('{')) return;
        const value = JSON.parse(line) as { fixture?: string; port?: number };
        if (value.fixture !== 'supervised-calculation-panic' || !value.port) return;
        clearTimeout(timeout);
        done(value.port);
      });
    });
    return { child, port };
  } catch (error) {
    // Rejection must join the child before the caller removes its SQLite files
    // or starts another fixture. Sending kill alone leaves cleanup racing exit.
    if (!didClose) {
      const closed = once(child, 'close', { signal: AbortSignal.timeout(5000) });
      child.kill();
      await closed;
    }
    throw error;
  }
  finally { lines.close(); }
}

/** Ask the child to join its real engine/worker before process termination. */
async function stopFixture(child: ChildProcess | undefined): Promise<void> {
  if (!child || child.exitCode !== null || child.signalCode !== null) return;
  const exited = once(child, 'close', { signal: AbortSignal.timeout(5000) });
  child.send('stop');
  try { expect((await exited)[0]).toBe(0); }
  finally { if (child.exitCode === null && child.signalCode === null) child.kill(); }
}

describeNetworkSuite('Rust server caught calculation panic', () => {
  it('keeps production provenance strict and exposes no production panic trigger', () => {
    const production = require(resolve('native/index.js')) as {
      nativeAddonBuildClass(): string;
      ExperimentalRunningAuthority: { prototype: Record<string, unknown> };
    };
    expect(production.nativeAddonBuildClass()).toBe('production');
    expect(production.ExperimentalRunningAuthority.prototype['armCalculationPanicForTest']).toBeUndefined();
    expect(production.ExperimentalRunningAuthority.prototype['armExportFailureForTest']).toBeUndefined();
    const hooks = loadPanicBinding();
    expect(() => validateExperimentalFreshRunBinding(hooks, computeNativeSourceIdentity(resolve('native'))))
      .toThrow(/production build class/);
  });

  it.each([
    { mode: 1, boundary: 'USTAR completion', diagnosis: /injected archive end-block write error/u },
    { mode: 2, boundary: 'completed file length', diagnosis: /EXPORT_ARCHIVE_LENGTH/u },
    { mode: 3, boundary: 'full post-write validation', diagnosis: /IMPORT_CHECKPOINT_ROLE.*SHA-256/u }
  ])('preserves an evolved game and controller input after export fails at $boundary', async ({ mode, diagnosis }) => {
    const root = await mkdtemp(join(tmpdir(), 'slither-export-failure-'));
    const databasePath = join(root, 'metadata.sqlite');
    const directory = `${databasePath}.checkpoints`;
    const peers: WebSocket[] = [];
    let server: RustServer | undefined;
    let admission: MockInstance<typeof diskAdmission.admitDiskOperation> | undefined;
    try {
      configurePanicFixture(false, false);
      server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, resume: 'fresh', seed: 42,
        dbPath: databasePath, rustCalculationWorkers: 2 });
      expect(server.startupFault).toBeUndefined();
      const actual = startPreparedPanicRuntime();
      expect(() => actual.armExportFailureForTest(0)).toThrow(/mode must be/u);
      expect(() => actual.armExportFailureForTest(4)).toThrow(/mode must be/u);
      const bootstrap = await exportController(server.port, 'ui', peers);
      bootstrap.socket.send(JSON.stringify({ type: 'reset', settings: { snakeCount: 12, simSpeed: 12 },
        updates: [{ path: 'generationSeconds', value: 8 }, { path: 'baselineBots.count', value: 2 },
          { path: 'pelletCountTarget', value: 100 }] }));
      await controllerUntil(() => bootstrap.packets.some(packet => packet['type'] === 'stateReplaced'), 'export fixture did not reset');
      // Reset invalidates the old join. Rejoin through the real hub before changing live settings.
      bootstrap.socket.send(JSON.stringify({ type: 'join', mode: 'spectator' }));
      const rejoined = once(bootstrap.socket, 'pong', { signal: AbortSignal.timeout(5000) });
      bootstrap.socket.ping('export-fixture-rejoin');
      await rejoined;
      await healthUntil(server.port, value => value.ok && BigInt(`0x${value.generation}`) >= 2n);
      bootstrap.socket.send(JSON.stringify({ type: 'settings', requestId: 'export-failure-speed',
        updates: [{ path: 'simSpeed', value: 0.1 }] }));
      await controllerUntil(() => bootstrap.packets.some(packet => packet['type'] === 'settingsApplied' &&
        packet['requestId'] === 'export-failure-speed' && packet['applied'] === true), () =>
        `export fixture did not slow: ${JSON.stringify(bootstrap.packets.map(packet => ({
          type: packet['type'], reason: packet['reason'], message: packet['message'],
          requestId: packet['requestId'], applied: packet['applied'] })))}`);
      bootstrap.socket.terminate();
      const controllers = await Promise.all([exportController(server.port, 'ui', peers), exportController(server.port, 'bot', peers)]);
      const baselineResponse = await fixtureArchiveDownload(server.port, 'baseline');
      expect(baselineResponse.status).toBe(200);
      const baselineArchive = Buffer.from(await baselineResponse.arrayBuffer());
      expect(baselineArchive.byteLength).toBeLessThan(4 * 1024 * 1024);
      const userFile = join(root, 'retained-owner-copy.slither-save');
      await writeFile(userFile, baselineArchive);
      await exportClean(directory);
      const before = await healthUntil(server.port, value => value.ok);
      expect(BigInt(`0x${before.generation}`)).toBeGreaterThanOrEqual(2n);
      const metadataBefore = archiveMetadata(databasePath);
      expect(metadataBefore.find(table => table.name === 'rust_generation_history_v1')!.rows.length).toBeGreaterThan(0);
      expect(metadataBefore.find(table => table.name === 'rust_hall_of_fame_v1')!.rows.length).toBeGreaterThan(0);
      const filesBefore = await archiveFiles(directory);
      expect(filesBefore.some(file => file.filename.endsWith('.hof-weights-v1'))).toBe(true);
      const assignments = controllers.map(peer => ({ ...peer.assignment! }));
      const samples = controllers.map(peer => peer.sample!);
      const originalAdmit = diskAdmission.admitDiskOperation;
      admission = vi.spyOn(diskAdmission, 'admitDiskOperation').mockImplementationOnce(async (...args) => {
        const result = await originalAdmit(...args);
        await Promise.all(controllers.map(async peer => {
          peer.socket.send(JSON.stringify({ type: 'action', snakeId: peer.assignment!.snakeId,
            tick: peer.sample!.tick, turn: -1, boost: 0 }));
          const receipt = once(peer.socket, 'pong', { signal: AbortSignal.timeout(5000) });
          peer.socket.ping('export-failure-input');
          expect((await receipt)[0].toString()).toBe('export-failure-input');
        }));
        return result;
      });
      actual.armExportFailureForTest(mode);
      expect(() => actual.armExportFailureForTest(mode)).toThrow(/already armed/u);
      const failed = await fixtureArchiveDownload(server.port, 'injected failure');
      expect(failed.status).toBe(500);
      expect(failed.headers.has('content-disposition')).toBe(false);
      expect(failed.headers.has('x-slither-checkpoint-id')).toBe(false);
      expect(await failed.json()).toMatchObject({ ok: false, message: expect.stringMatching(diagnosis) });
      expect(admission).toHaveBeenCalledOnce();
      admission.mockRestore();
      await exportClean(directory);
      expect(archiveMetadata(databasePath)).toEqual(metadataBefore);
      expect(await archiveFiles(directory)).toEqual(filesBefore);
      expect(await readFile(userFile)).toEqual(baselineArchive);
      const after = await healthUntil(server.port, value => value.ok &&
        BigInt(`0x${value.completedStep}`) > BigInt(`0x${before.completedStep}`) &&
        value.telemetry.controllerActivity.player.appliedActions === before.telemetry.controllerActivity.player.appliedActions + 1 &&
        value.telemetry.controllerActivity.trainer.appliedActions === before.telemetry.controllerActivity.trainer.appliedActions + 1);
      expect(after).toMatchObject({ runId: before.runId, generation: before.generation,
        worldEpoch: before.worldEpoch, configHash: before.configHash, startupCheckpointId: before.startupCheckpointId });
      await controllerUntil(() => controllers.every((peer, index) => peer.sample!.tick > samples[index]!.tick &&
        exportHeadingChange(samples[index]!, peer.sample!) < -0.01), 'queued steering did not reach the unchanged game');
      for (const [index, peer] of controllers.entries()) {
        expect(peer.socket.readyState).toBe(WebSocket.OPEN);
        expect(peer.assignment).toEqual(assignments[index]);
        expect(peer.packets.filter(packet => ['stateReplaced', 'error'].includes(String(packet['type'])))).toEqual([]);
      }
      const retry = await fixtureArchiveDownload(server.port, 'retry');
      expect(retry.status).toBe(200);
      expect(Buffer.from(await retry.arrayBuffer())).toEqual(baselineArchive);
      await exportClean(directory);
      expect(archiveMetadata(databasePath)).toEqual(metadataBefore);
      expect(await archiveFiles(directory)).toEqual(filesBefore);
      expect(await readFile(userFile)).toEqual(baselineArchive);
    } finally {
      admission?.mockRestore();
      for (const peer of peers) peer.terminate();
      try { await server?.close(); }
      finally {
        configurePanicFixture(false);
        await rm(root, { recursive: true, force: true, maxRetries: 5, retryDelay: 50 });
      }
    }
  }, 20_000);

  it('contains a real Rayon panic, preserves health and its committed save, and restarts exactly', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-server-panic-'));
    const dbPath = join(root, 'metadata.sqlite');
    let server: RustServer | undefined;
    let peer: WebSocket | undefined;
    try {
      configurePanicFixture(true);
      server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, resume: 'fresh', seed: 42,
        dbPath, rustCalculationWorkers: 2 });
      expect(server.startupFault).toBeUndefined();
      const before = await healthUntil(server.port, value => value.ok && value.completedStep === '0000000000000000');
      const databaseBefore = durableState(dbPath);
      const filesBefore = (await readdir(`${dbPath}.checkpoints`)).sort();
      const checkpointPath = join(`${dbPath}.checkpoints`, `${before.startupCheckpointId}.checkpoint-v3`);
      const checkpointBefore = await readFile(checkpointPath);
      const messages: Array<Record<string, unknown>> = [];
      let frames = 0;
      peer = new WebSocket(`ws://127.0.0.1:${server.port}`);
      peer.on('message', (data, binary) => {
        if (binary) frames++;
        else messages.push(JSON.parse(data.toString()) as Record<string, unknown>);
      });
      await new Promise<void>((done, reject) => { peer?.once('open', done); peer?.once('error', reject); });
      peer.send(JSON.stringify({ type: 'hello', version: 2, clientType: 'ui' }));
      const welcomeDeadline = performance.now() + 5000;
      while (!messages.some(message => message['type'] === 'welcome') && performance.now() < welcomeDeadline) {
        await new Promise<void>(done => setTimeout(done, 10));
      }
      expect(messages.some(message => message['type'] === 'welcome')).toBe(true);
      peer.send(JSON.stringify({ type: 'join', mode: 'spectator' }));
      // WebSocket messages and ping are parsed in order. Its pong proves that
      // the real hub has processed join before the native failure can broadcast.
      const joined = once(peer, 'pong', { signal: AbortSignal.timeout(5000) });
      peer.ping();
      await joined;
      messages.length = 0;
      frames = 0;
      const actual = startPreparedPanicRuntime();
      const faulted = await healthUntil(server.port, value => !value.ok);
      expect(faulted.interfaceFault).toMatch(/inference.*partition 1|partition 1.*inference/);
      expect(faulted).toMatchObject({ runId: before.runId,
        startupCheckpointId: before.startupCheckpointId, completedStep: before.completedStep,
        generation: before.generation, schedulerCompletedSteps: '0000000000000000' });
      expect((await fetch(`http://127.0.0.1:${server.port}/api/health`)).status).toBe(503);
      expect(() => actual.armCalculationPanicForTest()).toThrow(/before coordinator start/);
      expect(() => actual.submitVisualization('0000000000000001', true)).toThrow();
      peer.send(JSON.stringify({ type: 'settings', requestId: 'after-panic',
        updates: [{ path: 'simSpeed', value: 2 }] }));
      const rejectionDeadline = performance.now() + 5000;
      while (!messages.some(message => message['type'] === 'settingsApplied' &&
          message['requestId'] === 'after-panic') &&
          performance.now() < rejectionDeadline) {
        await new Promise<void>(done => setTimeout(done, 10));
      }
      expect(messages.find(message => message['type'] === 'settingsApplied' &&
        message['requestId'] === 'after-panic')).toMatchObject({
        applied: false, updates: [], reason: faulted.interfaceFault
      });
      await new Promise<void>(done => setTimeout(done, 100));
      // The valid step-zero startup frame may precede the failed calculation.
      // Every frame copied by the real router must still describe that boundary.
      expect(panicFixtureFrameSteps().length).toBeGreaterThanOrEqual(frames);
      expect(panicFixtureFrameSteps().every(step => step === before.completedStep)).toBe(true);
      expect(actual.latestDisplay()?.completedStep).toBe(before.completedStep);
      expect(messages.some(message => message['type'] === 'error')).toBe(true);
      expect(messages.filter(message => message['type'] === 'stats')
        .every(message => message['tick'] === 0)).toBe(true);
      expect(messages.some(message => (message['type'] === 'settingsApplied' && message['applied'] === true) ||
        (message['type'] === 'newRunResult' && message['applied'] === true))).toBe(false);
      expect(durableState(dbPath)).toEqual(databaseBefore);
      expect((await readdir(`${dbPath}.checkpoints`)).sort()).toEqual(filesBefore);
      expect(await readFile(checkpointPath)).toEqual(checkpointBefore);
      peer.terminate(); peer = undefined;
      await server.close(); server = undefined;
      configurePanicFixture(false);
      server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, resume: 'latest',
        dbPath, rustCalculationWorkers: 2 });
      expect(server.startupFault).toBeUndefined();
      startPreparedPanicRuntime();
      const resumed = await healthUntil(server.port, value => value.ok && BigInt(`0x${value.completedStep}`) >= 2n);
      expect(resumed).toMatchObject({ runId: before.runId, startupCheckpointId: before.startupCheckpointId,
        generation: before.generation });
      expect(durableState(dbPath)).toEqual(databaseBefore);
    } finally {
      peer?.terminate();
      try { await server?.close(); }
      finally {
        configurePanicFixture(false);
        await rm(root, { recursive: true, force: true });
      }
    }
  }, 15_000);

  it('runs the supervisor fixture in a separate process and restores after explicit stop', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-server-panic-process-'));
    const databasePath = join(root, 'metadata.sqlite');
    let child: ChildProcess | undefined;
    try {
      const first = await fixtureProcess(databasePath);
      child = first.child;
      const failed = await healthUntil(first.port, value => !value.ok);
      expect(failed.interfaceFault).toMatch(/inference.*partition 1/);
      expect(failed.completedStep).toBe('0000000000000000');
      const retained = durableState(databasePath);
      await stopFixture(child); child = undefined;
      const second = await fixtureProcess(databasePath);
      child = second.child;
      const restored = await healthUntil(second.port,
        value => value.ok && BigInt(`0x${value.completedStep}`) >= 2n);
      expect(restored).toMatchObject({ runId: failed.runId,
        startupCheckpointId: failed.startupCheckpointId, generation: failed.generation });
      expect(durableState(databasePath)).toEqual(retained);
    } finally {
      try { await stopFixture(child); }
      // Windows can briefly retain the SQLite shared-memory mapping after
      // process close. Retry only this owned fixture removal, within 750 ms.
      finally { await rm(root, { recursive: true, force: true, maxRetries: 5, retryDelay: 50 }); }
    }
  }, 30_000);
});
