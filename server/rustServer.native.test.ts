import { mkdir, mkdtemp, rm, readdir, readFile, stat, writeFile } from 'node:fs/promises';
import { createReadStream } from 'node:fs';
import { createHash, randomUUID } from 'node:crypto';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { request as httpRequest, Server, type IncomingMessage, type ServerResponse } from 'node:http';
import { gzipSync } from 'node:zlib';
import { expect, it, vi } from 'vitest';
import WebSocket from 'ws';
import Database from 'better-sqlite3';
import { DEFAULT_CONFIG, normalizeConfig, parseConfig } from './config.ts';
import { PlayerActionPump } from '../src/net/playerActionPump.ts';
import { createWsClient, type AssignMsg, type SensorsMsg, type WelcomeMsg, type WsClient } from '../src/net/wsClient.ts';
import { run as runStage6RuntimeProbe } from '../scripts/stage6/runtime-integration-probe.ts';
import { measureTurnResponses } from '../scripts/stage7/lan-turn-response.ts';
import { PlayerReconnectExchange } from '../scripts/stage7/player-reconnect-exchange.ts';
import { startRustServer } from './rustServer.ts';
import { WsHub } from './wsHub.ts';
import { CheckpointPersistenceClient } from './rustEngine/checkpointPersistenceClient.ts';
import { BackgroundOutputPump } from './rustEngine/backgroundOutput.ts';
import { loadExperimentalFreshRunSession } from './rustEngine/experimentalFreshRunSession.ts';
import type { ManagedLegacySnapshotFormat } from './rustEngine/checkpointPersistenceProtocol.ts';
import { admitDiskOperation, CHECKPOINT_DISK_ADMISSION_REQUEST } from './rustEngine/diskAdmission.ts';
import type { RustArchiveWorkProgress } from './rustEngine/backgroundRuntime.ts';
import { describeNetworkSuite } from './test/networkSuites.ts';
import { buildStackGraphSpec } from '../src/brains/stackBuilder.ts';
import { compileGraph, graphKey } from '../src/brains/graph/compiler.ts';
import { CFG_DEFAULT } from '../src/config.ts';
import { DEFAULT_CORE_SETTINGS } from '../src/protocol/settings.ts';
import type { RustQueueDiagnostics } from '../src/protocol/rustBackground.ts';
import type { WsOutboundDiagnostics } from './wsHub.ts';
import { BROWSER_CAMERA_SETTING_PATHS, buildSettingsUI, applyValuesToSlidersFromCFG } from '../src/settings.ts';
import type { SettingsUpdate } from '../src/protocol/settings.ts';

/** Bounded test inbox for the real Protocol 2 transport. */
interface Peer {
  /** Live local test socket. */
  socket: WebSocket;
  /** Small received protocol packets. */
  packets: Array<Record<string, unknown>>;
  /** Number of binary frame-v1 messages received. */
  frames: number;
  /** Latest replacement acknowledgement survives intentional inbox clearing. */
  rejoinToken?: string;
  /** Newest copied display frame, retained only for bounded integration assertions. */
  latestFrame?: Buffer;
}

/** Attach listeners before sending a hello so no ready message can be missed. */
async function connect(port: number, clientType: 'ui' | 'bot', origin?: string): Promise<Peer> {
  const peer: Peer = { socket: new WebSocket(`ws://127.0.0.1:${port}`,
    origin === undefined ? {} : { origin }), packets: [], frames: 0 };
  peer.socket.on('message', (data, binary) => {
    if (binary) { peer.frames++; peer.latestFrame = Buffer.from(data as Buffer); }
    else {
      const packet = JSON.parse(data.toString()) as Record<string, unknown>;
      if (packet['type'] === 'stateReplaced') peer.rejoinToken = String(packet['rejoinToken']);
      if (peer.packets.length < 256) peer.packets.push(packet);
    }
  });
  await new Promise<void>((done, reject) => { peer.socket.once('open', done); peer.socket.once('error', reject); });
  peer.socket.send(JSON.stringify({ type: 'hello', version: 2, clientType }));
  return peer;
}

/** Wait on a bounded real transport outcome, preserving diagnostics on failure. */
async function until(peer: Peer, predicate: () => boolean): Promise<void> {
  const deadline = performance.now() + 5000;
  while (!predicate() && performance.now() < deadline) await new Promise<void>(done => setTimeout(done, 10));
  expect(predicate(), JSON.stringify(peer.packets)).toBe(true);
}

/** Poll one real health condition within the existing five-second integration bound. */
async function healthUntil(
  port: number,
  predicate: (health: Record<string, unknown>) => boolean
): Promise<Record<string, unknown>> {
  const deadline = performance.now() + 5000;
  let health: Record<string, unknown> = {};
  while (performance.now() < deadline) {
    health = await (await fetch(`http://127.0.0.1:${port}/api/health`)).json() as Record<string, unknown>;
    if (predicate(health)) return health;
    await new Promise<void>(done => setTimeout(done, 10));
  }
  expect(predicate(health), JSON.stringify(health)).toBe(true);
  return health;
}

/** Wait for publication and its final retention refresh before requesting another replacement. */
async function replacementUntil(peer: Peer, port: number, reason: 'reset' | 'newRun'): Promise<WelcomeMsg> {
  await until(peer, () => peer.packets.some(packet => packet['type'] === 'stateReplaced' && packet['reason'] === reason));
  const welcome = peer.packets.findLast(packet => packet['type'] === 'stateReplaced' && packet['reason'] === reason)!['welcome'] as WelcomeMsg;
  await healthUntil(port, health => (health['retention'] as { activeRunId: string }).activeRunId === welcome.runId);
  return welcome;
}

/** Read one assigned snake direction from the compact frame-v1 contract. */
function frameDirection(bytes: Buffer | undefined, snakeId: number): number | undefined {
  if (!bytes || bytes.byteLength < 7 * Float32Array.BYTES_PER_ELEMENT) return undefined;
  const value = (index: number): number => bytes.readFloatLE(index * Float32Array.BYTES_PER_ELEMENT);
  const alive = Math.trunc(value(2));
  let offset = 7;
  for (let index = 0; index < alive; index++) {
    if ((offset + 8) * Float32Array.BYTES_PER_ELEMENT > bytes.byteLength) return undefined;
    const id = value(offset);
    const direction = value(offset + 5);
    const points = Math.trunc(value(offset + 7));
    if (id === snakeId) return direction;
    if (points < 0) return undefined;
    offset += 8 + points * 2;
  }
  return undefined;
}

/** Read the first alive snake's public identity and head from frame v1. */
function firstFrameSnake(bytes: Buffer | undefined): { snakeId: number; x: number; y: number } | undefined {
  if (!bytes || bytes.byteLength < 15 * Float32Array.BYTES_PER_ELEMENT) return undefined;
  const value = (index: number): number => bytes.readFloatLE(index * Float32Array.BYTES_PER_ELEMENT);
  if (Math.trunc(value(2)) < 1) return undefined;
  return { snakeId: value(7), x: value(10), y: value(11) };
}

/** Signed shortest angular change from one wrapped direction to another. */
function directionDelta(from: number, to: number): number {
  return Math.atan2(Math.sin(to - from), Math.cos(to - from));
}

/** Hash a large original save with bounded memory before and after a transfer test. */
async function archiveFileSha256(path: string): Promise<string> {
  const hash = createHash('sha256');
  for await (const bytes of createReadStream(path)) hash.update(bytes);
  return hash.digest('hex');
}

/** Hash every original fixture row, including all scalar columns and exact stored BLOB bytes. */
function legacyRecords(databasePath: string, originalTables?: readonly string[]): {
  /** Original table names, excluding schema added by writable startup. */
  tables: string[];
  /** Digest of every original row and column, including BLOB bytes. */
  sha256: string;
} {
  const database = new Database(databasePath, { readonly: true });
  try {
    const tables = originalTables ? [...originalTables] : (database.prepare(`SELECT name FROM sqlite_master
      WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name`).all() as Array<{ name: string }>).map(row => row.name);
    const hash = createHash('sha256');
    for (const name of tables) {
      hash.update(JSON.stringify({ name, rows: database.prepare(
        `SELECT * FROM "${name.replaceAll('"', '""')}" ORDER BY rowid`).raw().all() }));
    }
    return { tables, sha256: hash.digest('hex') };
  } finally { database.close(); }
}

/** Include reference companion tables so preservation also covers retained owner records. */
function legacyCompanions(database: ReturnType<typeof Database>, graphJson: string, archKey: string, weightCount: number): void {
  database.exec(`CREATE TABLE hof_entries (id INTEGER PRIMARY KEY, created_at INTEGER, gen INTEGER,
    seed INTEGER, fitness REAL, points REAL, length REAL, genome_json TEXT, UNIQUE(gen, seed, fitness));
    CREATE TABLE players (id TEXT PRIMARY KEY, name TEXT, created_at INTEGER);
    CREATE TABLE graph_presets (id INTEGER PRIMARY KEY, created_at INTEGER, name TEXT, spec_json TEXT);`);
  database.prepare('INSERT INTO hof_entries VALUES (1, 123456, 6, 1234567, 99, 42, 17, ?)')
    .run(JSON.stringify({ archKey, brainType: 'mlp', fitness: 99, weights: new Array<number>(weightCount).fill(0) }));
  database.prepare('INSERT INTO players VALUES (?, ?, 123456)').run('owner-player', 'Retained Ω player');
  database.prepare('INSERT INTO graph_presets VALUES (1, 123456, ?, ?)').run('Retained Ω graph', graphJson);
}

/** Run the real read-only Rust converter with a separate durable destination, preserving source bytes. */
async function readOnlyLegacyConversion(root: string, databasePath: string, sourceFormat: ManagedLegacySnapshotFormat): Promise<void> {
  const sourceHash = await archiveFileSha256(databasePath);
  const sourceRows = legacyRecords(databasePath);
  const destination = join(root, 'read-only-conversion.sqlite');
  const managedDirectory = `${destination}.checkpoints`;
  await mkdir(managedDirectory);
  await admitDiskOperation(managedDirectory, CHECKPOINT_DISK_ADMISSION_REQUEST);
  const persistence = new CheckpointPersistenceClient({ databasePath: destination, managedRootPath: managedDirectory });
  try {
    const nativeRequire = createRequire(import.meta.url);
    const session = await loadExperimentalFreshRunSession({ nativeManifestDirectory: resolve('native'),
      loadBinding: () => nativeRequire(resolve('native/index.js')), runId: randomUUID(), seed: 0,
      memoryCeilingBytes: 4n * 1024n * 1024n * 1024n, calculationWorkers: 1,
      persistence, managedDirectory });
    const converted = await session.initializeFromLegacySqlite(databasePath, 1);
    expect(session.startupMetadata()).toMatchObject({ seed: 0, legacyConversion: {
      version: 1, sourceFormat, sourceSnapshotId: 1, completeness: 'population-only', exactContinuation: false
    } });
    expect(converted).toMatchObject({ generation: '0000000000000001', completedStep: '0000000000000000',
      snakeCount: '0000000000000000', checkpointPublished: false });
    const durable = await session.commitPendingRunStart('81'.repeat(16), {
      snapshotId: 1, sourceFormat, completeness: 'population-only'
    });
    expect(durable.descriptor.populationCount).toBe('0000000000000002');
    expect(await persistence.selectStartup()).toMatchObject({ runId: durable.runId,
      descriptor: { logicalRootSha256: durable.checkpointId },
      legacyConversion: { snapshotId: 1, sourceFormat, completeness: 'population-only' } });
    expect((await readdir(managedDirectory)).filter(name => name.endsWith('.checkpoint-v3')))
      .toEqual([`${durable.checkpointId}.checkpoint-v3`]);
  } finally { await persistence.close(); }
  expect(await archiveFileSha256(databasePath)).toBe(sourceHash);
  expect(legacyRecords(databasePath, sourceRows.tables)).toEqual(sourceRows);
}

/** Read the small manifest from a bounded, actually exported nine-role test archive. */
function legacyExportManifest(bytes: Buffer): {
  archiveKind: string; legacyConversion?: import('../src/protocol/rustBackground.ts').RustLegacyConversionNotice;
  runId: string; checkpointLogicalRootSha256: string; logicalRootSha256: string;
  roles: Array<{ role: string; logicalSha256: string }>;
} {
  expect(bytes.byteLength).toBeLessThan(4 * 1024 * 1024);
  let cursor = 0;
  for (let entry = 0; entry < 9; entry++) {
    const name = bytes.subarray(cursor, cursor + 100).toString().split('\0')[0];
    const length = Number.parseInt(bytes.subarray(cursor + 124, cursor + 136).toString().replace(/\0.*$/su, '').trim(), 8);
    expect(Number.isSafeInteger(length) && length >= 0).toBe(true);
    if (entry === 8) {
      expect(name).toBe('manifest.json');
      return JSON.parse(bytes.subarray(cursor + 512, cursor + 512 + length).toString());
    }
    cursor += 512 + Math.ceil(length / 512) * 512;
  }
  throw new Error('legacy conversion export omitted its manifest');
}

/** Re-export and re-import a converted population through real archive HTTP, checking its original Float32 digest. */
async function migratedArchiveRoundTrip(root: string, server: Awaited<ReturnType<typeof startRustServer>>, weightsSha256?: string): Promise<void> {
  const sourceHealth = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as Record<string, unknown>;
  const exported = await fetch(`http://127.0.0.1:${server.port}/api/export/latest`);
  expect(exported.status).toBe(200);
  const archive = Buffer.from(await exported.arrayBuffer());
  const originalHash = createHash('sha256').update(archive).digest('hex');
  const manifest = legacyExportManifest(archive);
  expect(manifest.archiveKind).toBe('legacy-population-import');
  expect(manifest.legacyConversion).toEqual(sourceHealth['legacyConversion']);
  expect(manifest.legacyConversion).toMatchObject({ version: 1, completeness: 'population-only', exactContinuation: false });
  if (weightsSha256 !== undefined) expect(manifest.roles.find(role => role.role === 'population-weights')?.logicalSha256).toBe(weightsSha256);
  expect(exported.headers.get('x-slither-checkpoint-id')).toBe(manifest.checkpointLogicalRootSha256);
  const targetPath = join(root, 'archive-round-trip.sqlite');
  let target = await startRustServer({ ...DEFAULT_CONFIG, port: 0, seed: 99, resume: 'fresh', dbPath: targetPath });
  try {
    expect(target.startupFault).toBeUndefined();
    const imported = await fetch(`http://127.0.0.1:${target.port}/api/import/archive`, {
      method: 'POST', body: new Uint8Array(archive)
    });
    expect(imported.status, await imported.clone().text()).toBe(200);
    expect(await imported.json()).toMatchObject({ ok: true, runId: manifest.runId,
      checkpointId: manifest.checkpointLogicalRootSha256, saveLogicalRootSha256: manifest.logicalRootSha256,
      legacyConversion: manifest.legacyConversion });
    await target.close();
    target = await startRustServer({ ...DEFAULT_CONFIG, port: 0, resume: 'latest', dbPath: targetPath });
    expect(target.startupFault).toBeUndefined();
    expect(await (await fetch(`http://127.0.0.1:${target.port}/api/health`)).json()).toMatchObject({
      ok: true, runId: manifest.runId, seed: sourceHealth['seed'], legacyConversion: manifest.legacyConversion });
    const reexported = await fetch(`http://127.0.0.1:${target.port}/api/export/latest`);
    expect(reexported.status).toBe(200);
    const roundTrip = legacyExportManifest(Buffer.from(await reexported.arrayBuffer()));
    expect(roundTrip).toEqual(manifest);
    expect(createHash('sha256').update(archive).digest('hex')).toBe(originalHash);
  } finally { await target.close(); }
}

/** Replace only the final manifest in a private fixture, retaining valid USTAR framing. */
function rewriteLegacyManifest(archive: Buffer, mutate: (manifest: Record<string, unknown>) => void): Buffer {
  let cursor = 0;
  while (cursor + 512 <= archive.length) {
    const header = archive.subarray(cursor, cursor + 512);
    const name = header.subarray(0, 100).toString().split('\0', 1)[0];
    const length = Number.parseInt(header.subarray(124, 136).toString().replace(/\0.*$/u, '').trim(), 8);
    if (name === 'manifest.json') {
      const manifest = JSON.parse(archive.subarray(cursor + 512, cursor + 512 + length).toString()) as Record<string, unknown>;
      mutate(manifest);
      const payload = Buffer.from(JSON.stringify(manifest));
      const rewritten = Buffer.from(header);
      rewritten.fill(0, 124, 136);
      rewritten.write(payload.length.toString(8).padStart(11, '0'), 124, 'ascii');
      rewritten.fill(32, 148, 156);
      const checksum = rewritten.reduce((sum, byte) => sum + byte, 0);
      rewritten.write(`${checksum.toString(8).padStart(6, '0')}\0 `, 148, 'ascii');
      return Buffer.concat([archive.subarray(0, cursor), rewritten, payload,
        Buffer.alloc((512 - payload.length % 512) % 512 + 1024)]);
    }
    cursor += 512 + Math.ceil(length / 512) * 512;
  }
  throw new Error('fixture manifest not found');
}

describeNetworkSuite('Rust server real sockets', () => {
  it('enforces browser origins on production HTTP, WebSockets and startup-fault health', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-browser-origin-'));
    const peers: Peer[] = [];
    let server: Awaited<ReturnType<typeof startRustServer>> | undefined;
    try {
      const config = { ...DEFAULT_CONFIG, port: 0, resume: 'fresh' as const, seed: 42,
        rustCalculationWorkers: 1, dbPath: join(root, 'experiment.sqlite') };
      server = await startRustServer(config);
      const url = `http://127.0.0.1:${server.port}`;
      const before = await (await fetch(`${url}/api/health`)).json() as Record<string, unknown>;
      for (const path of ['/api/export/latest', '/api/import/archive', '/api/checkpoints/current/pin', '/api/graph-presets']) {
        const response = await fetch(`${url}${path}`, { method: path.includes('export') ? 'GET' : 'POST',
          headers: { Origin: 'http://evil.test', 'Content-Type': 'text/plain' } });
        expect(response.status).toBe(403);
        expect(response.headers.has('Access-Control-Allow-Origin')).toBe(false);
        await response.text();
      }
      const image = await fetch(`${url}/api/export/latest`, {
        headers: { 'Sec-Fetch-Site': 'cross-site', 'Sec-Fetch-Dest': 'image' }
      });
      expect(image.status).toBe(403);
      await image.text();
      const rejected = new WebSocket(`ws://127.0.0.1:${server.port}`, { origin: 'http://evil.test' });
      await new Promise<void>((done, reject) => {
        rejected.once('open', () => { rejected.terminate(); reject(new Error('untrusted upgrade accepted')); });
        rejected.once('error', error => {
          try { expect(error.message).toContain('403'); done(); } catch (failure) { reject(failure); }
        });
      });
      for (const origin of ['http://localhost:5173', url, undefined]) {
        const peer = await connect(server.port, 'ui', origin);
        peers.push(peer);
        peer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
          rejoinToken: peer.rejoinToken }));
        await until(peer, () => peer.frames > 0);
      }
      const after = await (await fetch(`${url}/api/health`, { headers: { Origin: 'http://localhost:5173' } })).json() as Record<string, unknown>;
      expect(after['runId']).toBe(before['runId']);
      expect(after['configHash']).toBe(before['configHash']);
      for (const peer of peers) peer.socket.terminate();
      peers.length = 0;
      await server.close();
      server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, resume: `sha256:${'ab'.repeat(32)}`,
        rustCalculationWorkers: 1, dbPath: join(root, 'missing.sqlite') });
      expect(server.startupFault).toBeDefined();
      const faultUrl = `http://127.0.0.1:${server.port}/api/health`;
      const forbidden = await fetch(faultUrl, { headers: { Origin: 'http://evil.test' } });
      expect(forbidden.status).toBe(403);
      await forbidden.text();
      const health = await fetch(faultUrl, { headers: { Origin: 'http://localhost:5173' } });
      expect(health.status).toBe(503);
      expect(health.headers.get('Access-Control-Allow-Origin')).toBe('http://localhost:5173');
      expect(await health.json()).toMatchObject({ lifecycle: 'startup-fault' });
    } finally {
      for (const peer of peers) peer.socket.terminate();
      await server?.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 20_000);

  it.each([
    { inferenceBackend: 'js' as const },
    { mtEnabled: true },
    { mtWorkers: 4 }
  ])('rejects reference backend/pool configuration before production startup: %j', async reference => {
    await expect(startRustServer({ ...DEFAULT_CONFIG, ...reference })).rejects.toThrow(
      /use --rust-workers for Rust or npm run server:reference/u
    );
  });

  it.each(['CLI', 'environment', 'TOML'])('rejects a production tick override from %s before creating state', async source => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-tick-override-'));
    const configPath = join(root, 'server.toml');
    try {
      await writeFile(configPath, source === 'TOML' ? 'tickRateHz = 30\n' : '');
      const config = parseConfig(['--config', configPath, ...(source === 'CLI' ? ['--tick', '30'] : [])],
        source === 'environment' ? { TICK_RATE: '30' } : {});
      expect(config.tickRateHz).toBe(30);
      await expect(startRustServer({ ...config, port: 0, dbPath: join(root, 'missing.sqlite') })).rejects.toThrow(
        'Rust startup requires tickRateHz=60'
      );
      expect(await readdir(root)).toEqual(['server.toml']);
    } finally { await rm(root, { recursive: true, force: true }); }
  });

  it('keeps the process session stable through live settings, reconnect, reset and New Run', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-process-session-'));
    const dbPath = join(root, 'experiment.sqlite');
    let server: Awaited<ReturnType<typeof startRustServer>> | undefined;
    const peers: Peer[] = [];
    try {
      server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, resume: 'fresh', seed: 42,
        rustCalculationWorkers: 1, dbPath });
      const viewer = await connect(server.port, 'ui'); peers.push(viewer);
      await until(viewer, () => viewer.packets.some(packet => packet['type'] === 'welcome'));
      const initial = viewer.packets.find(packet => packet['type'] === 'welcome')!;
      const sessionId = initial['sessionId'];
      expect(sessionId).toEqual(expect.any(String));
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
        rejoinToken: viewer.rejoinToken }));
      viewer.socket.send(JSON.stringify({ type: 'settings', requestId: 'session-live-settings',
        updates: [{ path: 'simSpeed', value: 0.1 }] }));
      await until(viewer, () => viewer.packets.some(packet => packet['type'] === 'settingsApplied' && packet['applied'] === true));
      const reconnected = await connect(server.port, 'ui'); peers.push(reconnected);
      await until(reconnected, () => reconnected.packets.some(packet => packet['type'] === 'welcome'));
      expect(reconnected.packets.find(packet => packet['type'] === 'welcome')).toMatchObject({
        sessionId, runId: initial['runId'], configRevision: 2, settings: { core: { simSpeed: 0.1 } }
      });
      const beforeReset = await healthUntil(server.port, health => Number(BigInt(`0x${health['completedStep'] as string}`)) >= 5);
      const stepsBeforeReset = (beforeReset['telemetry'] as { authoritativeSteps: number }).authoritativeSteps;
      viewer.socket.send(JSON.stringify({ type: 'reset' }));
      const reset = await replacementUntil(viewer, server.port, 'reset');
      expect(reset.sessionId).toBe(sessionId);
      expect(reset.runId).not.toBe(initial['runId']);
      const afterReset = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as {
        telemetry: { authoritativeSteps: number };
      };
      expect(afterReset.telemetry.authoritativeSteps).toBeGreaterThanOrEqual(stepsBeforeReset);
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
        rejoinToken: viewer.rejoinToken }));
      viewer.socket.send(JSON.stringify({ type: 'newRun', requestId: 'session-new-run' }));
      const newRun = await replacementUntil(viewer, server.port, 'newRun');
      expect(newRun.sessionId).toBe(sessionId);
      expect(newRun.runId).not.toBe(reset.runId);
      const afterNewRun = await healthUntil(server.port, health =>
        (health['telemetry'] as { authoritativeSteps: number }).authoritativeSteps > afterReset.telemetry.authoritativeSteps);
      expect((afterNewRun['telemetry'] as { authoritativeSteps: number }).authoritativeSteps).toBeGreaterThan(stepsBeforeReset);
      for (const peer of peers) peer.socket.terminate();
      peers.length = 0;
      await server.close();
      server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, resume: 'latest', rustCalculationWorkers: 1, dbPath });
      expect(server.startupFault).toBeUndefined();
      const restarted = await connect(server.port, 'ui'); peers.push(restarted);
      await until(restarted, () => restarted.packets.some(packet => packet['type'] === 'welcome'));
      const resumedWelcome = restarted.packets.find(packet => packet['type'] === 'welcome')!;
      expect(resumedWelcome['sessionId']).not.toBe(sessionId);
      expect(resumedWelcome['runId']).toBe(newRun.runId);
    } finally {
      for (const peer of peers) peer.socket.terminate();
      await server?.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 15_000);

  it('clears custom graphs and preserves truthful stack controls through reset, reconnect and resume', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-stack-reset-'));
    const dbPath = join(root, 'experiment.sqlite');
    let server: Awaited<ReturnType<typeof startRustServer>> | undefined;
    const peers: Peer[] = [];
    const originalRetention = CheckpointPersistenceClient.prototype.inspectRetention;
    const retention = vi.spyOn(CheckpointPersistenceClient.prototype, 'inspectRetention').mockImplementation(async function(
      this: CheckpointPersistenceClient
    ) {
      const result = await originalRetention.call(this);
      // Keep publication visibly ahead of completion, reproducing the loaded-runner ordering.
      await new Promise<void>(done => setTimeout(done, 40));
      return result;
    });
    try {
      server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, resume: 'fresh', seed: 42,
        rustCalculationWorkers: 1, dbPath });
      const viewer = await connect(server.port, 'ui'); peers.push(viewer);
      await until(viewer, () => viewer.packets.some(packet => packet['type'] === 'welcome'));
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
        rejoinToken: viewer.rejoinToken }));
      await until(viewer, () => viewer.frames > 0);
      const custom = buildStackGraphSpec(DEFAULT_CORE_SETTINGS,
        { brain: { ...CFG_DEFAULT.brain, useMlp: false, stack: { gru: 0, lstm: 0, rru: 0 } } });
      custom.nodes[1]!.id = 'custom-output';
      custom.edges[0]!.to = 'custom-output';
      custom.outputs[0]!.nodeId = 'custom-output';
      viewer.socket.send(JSON.stringify({ type: 'reset', graphSpec: custom,
        settings: { snakeCount: 3, simSpeed: 0.1 }, updates: [{ path: 'baselineBots.count', value: 0 }] }));
      await replacementUntil(viewer, server.port, 'reset');
      expect(viewer.packets.findLast(packet => packet['type'] === 'stateReplaced')).toMatchObject({ welcome: { graphSpec: custom } });
      viewer.packets.length = 0;
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
        rejoinToken: viewer.rejoinToken }));
      viewer.socket.send(JSON.stringify({ type: 'reset' }));
      await replacementUntil(viewer, server.port, 'reset');
      expect(viewer.packets.findLast(packet => packet['type'] === 'stateReplaced')).toMatchObject({ welcome: { graphSpec: custom } });

      const core = { ...DEFAULT_CORE_SETTINGS, snakeCount: 3, simSpeed: 0.1, hiddenLayers: 3,
        neurons1: 23, neurons2: 17, neurons3: 11 };
      const brain = { ...CFG_DEFAULT.brain, inSize: 51, gruHidden: 12, lstmHidden: 20, rruHidden: 24,
        stack: { gru: 1, lstm: 1, rru: 1 } };
      const expectedGraph = buildStackGraphSpec(core, { brain });
      const updates = [
        { path: 'sense.bubbleBins', value: 8 }, { path: 'brain.useMlp', value: 1 },
        { path: 'brain.stack.gru', value: 1 }, { path: 'brain.stack.lstm', value: 1 }, { path: 'brain.stack.rru', value: 1 },
        { path: 'brain.gruHidden', value: 12 }, { path: 'brain.lstmHidden', value: 20 }, { path: 'brain.rruHidden', value: 24 }
      ];
      viewer.packets.length = 0;
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
        rejoinToken: viewer.rejoinToken }));
      viewer.socket.send(JSON.stringify({ type: 'reset', graphSpec: null, settings: core, updates }));
      const welcome = await replacementUntil(viewer, server.port, 'reset');
      expect(welcome).toMatchObject({ graphSpec: expectedGraph, settings: {
        core: { hiddenLayers: 3, neurons1: 23, neurons2: 17, neurons3: 11 }, updates: expect.arrayContaining(updates)
      } });
      expect(graphKey(welcome.graphSpec!)).not.toBe(graphKey(custom));

      // Omitted graph plus the browser's unchanged controls remains a valid reset.
      viewer.packets.length = 0;
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
        rejoinToken: viewer.rejoinToken }));
      viewer.socket.send(JSON.stringify({ type: 'reset', settings: welcome.settings.core, updates }));
      await replacementUntil(viewer, server.port, 'reset');
      expect(viewer.packets.findLast(packet => packet['type'] === 'stateReplaced')).toMatchObject({ welcome: { graphSpec: expectedGraph } });

      const reconnect = await connect(server.port, 'ui'); peers.push(reconnect);
      await until(reconnect, () => reconnect.packets.some(packet => packet['type'] === 'welcome'));
      expect(reconnect.packets.find(packet => packet['type'] === 'welcome')).toMatchObject({ graphSpec: expectedGraph,
        settings: welcome.settings });
      for (const peer of peers) peer.socket.terminate();
      peers.length = 0;
      await server.close();
      server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, resume: 'latest', rustCalculationWorkers: 1, dbPath });
      expect(server.startupFault).toBeUndefined();
      const resumed = await connect(server.port, 'ui'); peers.push(resumed);
      await until(resumed, () => resumed.packets.some(packet => packet['type'] === 'welcome'));
      expect(resumed.packets.find(packet => packet['type'] === 'welcome')).toMatchObject({ graphSpec: expectedGraph,
        settings: welcome.settings });
    } finally {
      retention.mockRestore();
      for (const peer of peers) peer.socket.terminate();
      await server?.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 20_000);

  it('admits 300 complete long initial bodies through the normal reset boundary', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-long-start-'));
    let server: Awaited<ReturnType<typeof startRustServer>> | undefined;
    let viewer: Peer | undefined;
    try {
      server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, resume: 'fresh',
        seed: 1511506142, dbPath: join(root, 'experiment.sqlite') });
      viewer = await connect(server.port, 'ui');
      await until(viewer, () => viewer!.packets.some(packet => packet['type'] === 'welcome'));
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
        rejoinToken: viewer.rejoinToken }));
      await until(viewer, () => viewer!.frames > 0);
      const graphSpec = { type: 'graph',
        nodes: [{ id: 'input', type: 'Input', outputSize: 83 },
          { id: 'output', type: 'Dense', inputSize: 83, outputSize: 2 }],
        edges: [{ from: 'input', to: 'output' }], outputs: [{ nodeId: 'output' }], outputSize: 2 };
      viewer.socket.send(JSON.stringify({ type: 'reset', graphSpec,
        settings: { snakeCount: 300, simSpeed: 0.1 }, updates: [
          { path: 'worldRadius', value: 10000 },
          { path: 'snakeStartLen', value: 140 },
          { path: 'snakeSpacing', value: 3 },
          // This checks admission geometry; limit movement before a loaded runner observes it.
          { path: 'snakeBaseSpeed', value: 30 },
          { path: 'snakeBoostSpeed', value: 40 },
          { path: 'snakeRadius', value: 3 },
          { path: 'snakeThicknessScale', value: 0 },
          { path: 'collision.skipSegments', value: 30 },
          { path: 'baselineBots.count', value: 0 }
        ] }));
      await until(viewer, () => viewer!.packets.some(packet =>
        packet['type'] === 'stateReplaced' && packet['reason'] === 'reset'));
      delete viewer.latestFrame;
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
        rejoinToken: viewer.rejoinToken }));
      await until(viewer, () => viewer!.latestFrame?.readFloatLE(4) === 300);
      const bytes = viewer.latestFrame!;
      expect(bytes.readFloatLE(0)).toBe(1);
      expect(bytes.readFloatLE(8)).toBe(300);
      expect(bytes.readFloatLE(12)).toBe(10000);
      let offset = 7;
      let bodyPoints = 0;
      for (let snake = 0; snake < 300; snake++) {
        const points = bytes.readFloatLE((offset + 7) * 4);
        expect(Number.isSafeInteger(points)).toBe(true);
        expect(points).toBeGreaterThanOrEqual(140);
        for (let point = 0; point < points; point++) {
          const x = bytes.readFloatLE((offset + 8 + point * 2) * 4);
          const y = bytes.readFloatLE((offset + 9 + point * 2) * 4);
          expect(x * x + y * y).toBeLessThan(10000 ** 2);
        }
        bodyPoints += points;
        offset += 8 + points * 2;
      }
      expect(bodyPoints).toBeGreaterThanOrEqual(42000);
      expect(await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json())
        .toMatchObject({ ok: true, lifecycle: 'running' });
    } finally {
      viewer?.socket.terminate();
      await server?.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 20_000);

  it.runIf(process.env['SLITHER_FULL_UPLOAD_TIMEOUT_TEST'] === '1')(
    'rejects a connected chunked import after the full no-progress deadline', async () => {
      const root = await mkdtemp(join(tmpdir(), 'slither-rust-stalled-import-'));
      const dbPath = join(root, 'experiment.sqlite');
      const server = await startRustServer({
        ...DEFAULT_CONFIG, port: 0, resume: 'fresh', seed: 42, dbPath
      });
      const request = httpRequest(`http://127.0.0.1:${server.port}/api/import/archive`, {
        method: 'POST', headers: { 'Content-Type': 'application/vnd.slither-neuroevo.save',
          'Transfer-Encoding': 'chunked' }
      });
      try {
        const before = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as {
          runId: string; startupCheckpointId: string;
        };
        const startedAt = performance.now();
        const terminal = new Promise<{ status: number; body: string }>((resolve, reject) => {
          request.once('error', reject);
          request.once('response', response => {
            const chunks: Buffer[] = [];
            response.on('data', chunk => chunks.push(chunk as Buffer));
            response.once('error', reject);
            response.once('end', () => resolve({ status: response.statusCode ?? 0,
              body: Buffer.concat(chunks).toString() }));
          });
        });
        request.write(Buffer.from('started-but-incomplete'));
        const result = await terminal;
        const elapsedMs = performance.now() - startedAt;
        expect(result.status).toBe(400);
        expect(result.body).toContain('archive upload made no progress for 60000 ms');
        expect(elapsedMs).toBeGreaterThanOrEqual(59_000);
        request.destroy();
        expect((await readdir(`${dbPath}.checkpoints`)).filter(name => name.includes('upload'))).toEqual([]);
        expect(await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json()).toMatchObject({
          ok: true, runId: before.runId, startupCheckpointId: before.startupCheckpointId
        });
      } finally {
        request.destroy();
        await server.close();
        await rm(root, { recursive: true, force: true });
      }
    }, 90_000
  );

  it('rejects a second import while a chunked client is connected and cleans an aborted upload', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-aborted-import-'));
    const dbPath = join(root, 'experiment.sqlite');
    const server = await startRustServer({
      ...DEFAULT_CONFIG, port: 0, resume: 'fresh', seed: 42, dbPath
    });
    const first = httpRequest(`http://127.0.0.1:${server.port}/api/import/archive`, {
      method: 'POST', headers: { 'Content-Type': 'application/vnd.slither-neuroevo.save',
        'Transfer-Encoding': 'chunked' }
    });
    first.on('error', () => { /* Destroying the deliberately incomplete request is expected. */ });
    try {
      const before = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as {
        runId: string; startupCheckpointId: string;
      };
      first.write(Buffer.from('incomplete-archive'));
      const managedDirectory = `${dbPath}.checkpoints`;
      const uploadDeadline = performance.now() + 5000;
      let uploadFiles: string[] = [];
      do {
        uploadFiles = (await readdir(managedDirectory)).filter(name => name.includes('upload.partial'));
        if (uploadFiles.length === 1) break;
        await new Promise<void>(done => setTimeout(done, 10));
      } while (performance.now() < uploadDeadline);
      expect(uploadFiles).toHaveLength(1);

      const busy = await fetch(`http://127.0.0.1:${server.port}/api/import/archive`, {
        method: 'POST', body: 'another-import'
      });
      expect(busy.status).toBe(409);
      expect(await busy.json()).toMatchObject({ ok: false,
        message: 'another archive operation is in progress' });

      first.destroy();
      const cleanupDeadline = performance.now() + 5000;
      let leftovers: string[] = [];
      do {
        leftovers = (await readdir(managedDirectory)).filter(name => name.includes('upload'));
        if (leftovers.length === 0) break;
        await new Promise<void>(done => setTimeout(done, 10));
      } while (performance.now() < cleanupDeadline);
      expect(leftovers).toEqual([]);
      const retryDeadline = performance.now() + 5000;
      let retryStatus = 409;
      do {
        const retry = await fetch(`http://127.0.0.1:${server.port}/api/import/archive`, {
          method: 'POST', body: 'invalid-but-complete-archive'
        });
        retryStatus = retry.status;
        await retry.body?.cancel();
        if (retryStatus !== 409) break;
        await new Promise<void>(done => setTimeout(done, 10));
      } while (performance.now() < retryDeadline);
      expect(retryStatus).toBe(400);
      expect(await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json()).toMatchObject({
        ok: true, runId: before.runId, startupCheckpointId: before.startupCheckpointId
      });
      expect((await readdir(managedDirectory)).filter(name => name.includes('upload'))).toEqual([]);
    } finally {
      first.destroy();
      await server.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 20_000);

  it.each(['latest', `sha256:${'ab'.repeat(32)}`] as const)('faults explicit resume %s against a missing database without creating state', async resume => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-missing-resume-'));
    const server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, resume,
      rustCalculationWorkers: 1, dbPath: join(root, 'missing.sqlite') });
    try {
      expect(server.startupFault).toContain('database does not exist');
      const response = await fetch(`http://127.0.0.1:${server.port}/api/health`);
      expect(response.status).toBe(503);
      expect(await response.json()).toMatchObject({ ok: false, lifecycle: 'startup-fault' });
      const socket = new WebSocket(`ws://127.0.0.1:${server.port}`);
      await new Promise<void>((done, reject) => {
        socket.once('open', () => reject(new Error('missing resume accepted a game connection')));
        socket.once('error', error => {
          try { expect(error.message).toContain('503'); done(); } catch (failure) { reject(failure); }
        });
      });
      expect(await readdir(root)).toEqual([]);
    } finally {
      await server.close();
      await rm(root, { recursive: true, force: true });
    }
  });

  it('creates the first Rust run only under automatic startup and resumes its existing database', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-first-run-'));
    const dbPath = join(root, 'slither.sqlite');
    let server = await startRustServer({
      ...DEFAULT_CONFIG, port: 0, dbPath, seed: 91, rustCalculationWorkers: 1
    });
    try {
      const health = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json();
      expect(health).toMatchObject({ ok: true, authority: 'rust', generation: '0000000000000001' });
      const firstBoundary = BigInt(`0x${(health as { commandServiceBoundaries: string }).commandServiceBoundaries}`);
      await healthUntil(server.port, sample =>
        BigInt(`0x${sample['commandServiceBoundaries'] as string}`) > firstBoundary);
      await server.close();
      server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, dbPath, rustCalculationWorkers: 1 });
      expect(await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json()).toMatchObject({
        ok: true, runId: (health as { runId: string }).runId, seed: 91
      });
    } finally {
      await server.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 15_000);

  it('appends a seeded fresh run to a managed store and preserves prior checkpoints, history and Hall of Fame', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-fresh-append-'));
    const dbPath = join(root, 'slither.sqlite');
    let server: Awaited<ReturnType<typeof startRustServer>> | undefined;
    let viewer: Peer | undefined;
    try {
      server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, dbPath, resume: 'fresh', seed: 41, rustCalculationWorkers: 1 });
      expect(server.startupFault).toBeUndefined();
      expect((await fetch(`http://127.0.0.1:${server.port}/api/checkpoints/current/pin`, { method: 'POST' })).status).toBe(200);
      viewer = await connect(server.port, 'ui');
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
        rejoinToken: viewer.rejoinToken }));
      viewer.socket.send(JSON.stringify({ type: 'reset', settings: { snakeCount: 3, simSpeed: 0.1 },
        updates: [{ path: 'generationSeconds', value: 8 }, { path: 'pelletCountTarget', value: 100 },
          { path: 'baselineBots.count', value: 0 }],
        graphSpec: buildStackGraphSpec({ hiddenLayers: 1, neurons1: 2, neurons2: 2, neurons3: 2, neurons4: 2, neurons5: 2 },
          { brain: { inSize: 83, outSize: 2, useMlp: false } }) }));
      const previous = await replacementUntil(viewer, server.port, 'reset');
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
        rejoinToken: viewer.rejoinToken }));
      expect((await fetch(`http://127.0.0.1:${server.port}/api/checkpoints/current/pin`, { method: 'POST' })).status).toBe(200);
      viewer.socket.send(JSON.stringify({ type: 'settings', requestId: 'fresh-append-accelerate',
        updates: [{ path: 'simSpeed', value: 12 }] }));
      await until(viewer, () => viewer!.packets.some(packet => packet['requestId'] === 'fresh-append-accelerate' && packet['applied'] === true));
      await healthUntil(server.port, sample => BigInt(`0x${sample['generation'] as string}`) >= 2n);
      viewer.socket.send(JSON.stringify({ type: 'settings', requestId: 'fresh-append-slow',
        updates: [{ path: 'simSpeed', value: 0.1 }] }));
      await until(viewer, () => viewer!.packets.some(packet => packet['requestId'] === 'fresh-append-slow' && packet['applied'] === true));
      viewer.socket.terminate(); viewer = undefined;
      await server.close(); server = undefined;
      /** Read exact retained rows for the old lineage without materializing its population. */
      const oldRecords = (): unknown[] => {
        const database = new Database(dbPath, { readonly: true });
        try { return ['rust_checkpoint_v3_metadata', 'rust_checkpoint_v3_current', 'rust_generation_history_v1', 'rust_hall_of_fame_v1']
          .map(table => database.prepare(`SELECT * FROM ${table} WHERE run_id = ? ORDER BY rowid`).all(previous.runId)); }
        finally { database.close(); }
      };
      const retained = oldRecords();
      expect((retained[2] as unknown[]).length).toBeGreaterThan(0);
      expect((retained[3] as unknown[]).length).toBeGreaterThan(0);
      const oldFiles = new Map<string, string>();
      const inventory = new Database(dbPath, { readonly: true });
      let filenames: string[];
      try {
        filenames = (inventory.prepare(`SELECT relative_filename AS filename FROM rust_checkpoint_v3_metadata
          UNION SELECT relative_filename AS filename FROM rust_hall_of_fame_weights_v1`)
          .all() as Array<{ filename: string }>).map(row => row.filename);
      } finally { inventory.close(); }
      for (const file of filenames) {
        oldFiles.set(file, createHash('sha256').update(await readFile(join(`${dbPath}.checkpoints`, file))).digest('hex'));
      }
      expect(oldFiles.size).toBeGreaterThan(1);
      server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, dbPath, resume: 'fresh', seed: 99, rustCalculationWorkers: 1 });
      expect(server.startupFault).toBeUndefined();
      const fresh = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as {
        runId: string; startupCheckpointId: string; retention: { activeRunId: string };
      };
      expect(fresh).toMatchObject({ ok: true, seed: 99, generation: '0000000000000001' });
      expect(fresh.runId).not.toBe(previous.runId);
      expect(fresh.retention.activeRunId).toBe(fresh.runId);
      expect(oldRecords()).toEqual(retained);
      for (const [file, hash] of oldFiles) {
        expect(createHash('sha256').update(await readFile(join(`${dbPath}.checkpoints`, file))).digest('hex')).toBe(hash);
      }
      const database = new Database(dbPath, { readonly: true });
      try { expect(database.prepare('SELECT run_id FROM rust_active_run_v1 WHERE singleton = 1').get()).toEqual({ run_id: fresh.runId }); }
      finally { database.close(); }
      await server.close(); server = undefined;
      server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, dbPath, resume: 'latest', rustCalculationWorkers: 1 });
      expect(await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json()).toMatchObject({
        ok: true, seed: 99, runId: fresh.runId, startupCheckpointId: fresh.startupCheckpointId
      });
    } finally {
      viewer?.socket.terminate();
      await server?.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 20_000);

  it.each(['legacy', 'unrelated'])('rejects fresh startup of an existing %s database without modifying it', async kind => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-fresh-incompatible-'));
    const dbPath = join(root, 'owner.sqlite');
    let server: Awaited<ReturnType<typeof startRustServer>> | undefined;
    try {
      const database = new Database(dbPath);
      try {
        if (kind === 'legacy') {
          database.exec('CREATE TABLE population_snapshots (id INTEGER PRIMARY KEY, payload_json TEXT NOT NULL)');
          database.prepare('INSERT INTO population_snapshots VALUES (1, ?)').run('retained legacy population');
        } else {
          database.exec('CREATE TABLE owner_records (value TEXT NOT NULL)');
          database.prepare('INSERT INTO owner_records VALUES (?)').run('retained owner record');
        }
      } finally { database.close(); }
      const before = await readFile(dbPath);
      server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, dbPath, resume: 'fresh', seed: 99 });
      expect(server.startupFault).toBeDefined();
      expect((await fetch(`http://127.0.0.1:${server.port}/api/health`)).status).toBe(503);
      expect(await readFile(dbPath)).toEqual(before);
      expect(await readdir(root)).toEqual(['owner.sqlite']);
    } finally {
      await server?.close();
      await rm(root, { recursive: true, force: true });
    }
  });
  it('passes the selected checkpoint budget through production health and retention', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-budget-'));
    const dbPath = join(root, 'slither.sqlite');
    const server = await startRustServer({
      ...DEFAULT_CONFIG, port: 0, dbPath, resume: 'fresh', seed: 42,
      checkpointBudgetMiB: 2048
    });
    try {
      expect(await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json()).toMatchObject({
        ok: true,
        retention: { automaticByteCap: '0000000080000000' }
      });
    } finally {
      await server.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 15_000);

  it('imports an old browser JSON population as a new Rust run', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-legacy-import-'));
    const dbPath = join(root, 'experiment.sqlite');
    const server = await startRustServer({
      ...DEFAULT_CONFIG, port: 0, resume: 'fresh', seed: 41, dbPath, rustCalculationWorkers: 4
    });
    const peers: Peer[] = [];
    try {
      const before = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as {
        runId: string; nativeBuildIdentifier: string; calculationWorkers: number;
      };
      expect(before.nativeBuildIdentifier).toMatch(/^slither_native\/[0-9A-Za-z.+-]+$/u);
      expect(before.calculationWorkers).toBe(4);
      const viewer = await connect(server.port, 'ui');
      peers.push(viewer);
      await until(viewer, () => viewer.packets.some(packet => packet['type'] === 'welcome'));
      expect(viewer.packets.find(packet => packet['type'] === 'welcome')).toMatchObject({
        inferenceMode: { nativeAddonBuildIdentifier: before.nativeBuildIdentifier,
          requestedMt: true, activeWorkerCount: 4 }
      });
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
        rejoinToken: viewer.rejoinToken }));
      const weights = new Array<number>(13_458).fill(0);
      const legacyFile = JSON.stringify({
        runId: 'browser-source-run',
        generation: 37,
        archKey: 'legacy-default-graph',
        worldSeed: 1_234_567,
        settings: { snakeCount: 2, simSpeed: 3, baselineBots: { count: 1 } },
        genomes: [
          { archKey: 'legacy-default-graph', brainType: 'mlp', fitness: 99, weights },
          { archKey: 'legacy-default-graph', brainType: 'mlp', fitness: 50, weights }
        ]
      });
      const response = await fetch(`http://127.0.0.1:${server.port}/api/import/archive`, {
        method: 'POST', headers: { 'Content-Type': 'application/json' }, body: legacyFile
      });
      const imported = await response.json() as {
        ok: boolean; runId: string; generation: string; completedStep: string; checkpointId: string;
      };
      expect({ status: response.status, imported }).toMatchObject({
        status: 200,
        imported: {
          ok: true,
          generation: '0000000000000001',
          completedStep: '0000000000000000'
        }
      });
      expect(imported.runId).not.toBe(before.runId);
      expect(imported.checkpointId).toMatch(/^[0-9a-f]{64}$/u);
      const currentHealth = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as { seed: number };
      await until(viewer, () => viewer.packets.some(packet =>
        packet['type'] === 'stateReplaced' && packet['reason'] === 'import'));
      expect(viewer.packets.findLast(packet => packet['type'] === 'stateReplaced')).toMatchObject({
        checkpointId: imported.checkpointId,
        welcome: {
          runId: imported.runId,
          worldSeed: currentHealth.seed,
          legacyConversion: { version: 1, sourceFormat: 'browser-json', sourceRunId: 'browser-source-run',
            sourceGeneration: '0000000000000025', sourceSeed: 1_234_567,
            sourceSha256: createHash('sha256').update(legacyFile).digest('hex'), completeness: 'population-only', exactContinuation: false },
          inferenceMode: { activeWorkerCount: 4 },
          settings: {
            core: { snakeCount: 2, simSpeed: 3 },
            updates: expect.arrayContaining([{ path: 'baselineBots.count', value: 1 }])
          }
        }
      });
      expect(await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json()).toMatchObject({
        ok: true,
        runId: imported.runId,
        seed: currentHealth.seed,
        generation: '0000000000000001',
        startupCheckpointId: imported.checkpointId
      });
      const rejected = await fetch(`http://127.0.0.1:${server.port}/api/import/archive`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          generation: 1,
          archKey: 'legacy-default-graph',
          genomes: [{ archKey: 'legacy-default-graph', weights: [0] }]
        })
      });
      expect(rejected.status).toBe(400);
      expect(await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json()).toMatchObject({
        ok: true, runId: imported.runId, startupCheckpointId: imported.checkpointId
      });
      expect((await readdir(`${dbPath}.checkpoints`)).filter(name =>
        name.includes('upload') || name.includes('import-inventory'))).toEqual([]);
      await migratedArchiveRoundTrip(root, server, createHash('sha256')
        .update(Buffer.alloc(weights.length * 2 * Float32Array.BYTES_PER_ELEMENT)).digest('hex'));
    } finally {
      for (const peer of peers) peer.socket.terminate();
      await server.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);

  it.each(['reset', 'newRun'] as const)('retains legacy origin through evolution and restart, then clears it on %s', async replacement => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-origin-lineage-'));
    let server: Awaited<ReturnType<typeof startRustServer>> | undefined;
    let viewer: Peer | undefined;
    let releaseSpy: ReturnType<typeof vi.spyOn> | undefined;
    try {
      server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, resume: 'fresh', seed: 42, dbPath: join(root, 'source.sqlite') });
      const legacyFile = JSON.stringify({ generation: 37, worldSeed: 1234567, archKey: 'legacy-default-graph',
        settings: { snakeCount: 2, simSpeed: 12, baselineBots: { count: 0 } },
        updates: [{ path: 'generationSeconds', value: 8 }],
        genomes: [0, 1].map(() => ({ archKey: 'legacy-default-graph', weights: new Array<number>(13_458).fill(0) })) });
      const imported = await fetch(`http://127.0.0.1:${server.port}/api/import/archive`, { method: 'POST', body: legacyFile });
      expect(imported.status, await imported.clone().text()).toBe(200);
      const result = await imported.json() as { legacyConversion: unknown; runId: string };
      await healthUntil(server.port, health => BigInt(`0x${health['generation'] as string}`) >= 3n);
      viewer = await connect(server.port, 'ui');
      await until(viewer, () => viewer!.packets.some(packet => packet['type'] === 'welcome'));
      expect(viewer.packets.find(packet => packet['type'] === 'welcome')).toMatchObject({ legacyConversion: result.legacyConversion });
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
        rejoinToken: viewer.rejoinToken }));
      viewer.socket.send(JSON.stringify({ type: 'settings', requestId: 'hold-origin', updates: [{ path: 'simSpeed', value: 0.1 }] }));
      await until(viewer, () => viewer!.packets.some(packet => packet['type'] === 'settingsApplied' && packet['requestId'] === 'hold-origin'));
      expect(await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json()).toMatchObject({ legacyConversion: result.legacyConversion });
      let released = false;
      const originalRelease = CheckpointPersistenceClient.prototype.releaseExportLease;
      releaseSpy = vi.spyOn(CheckpointPersistenceClient.prototype, 'releaseExportLease')
        .mockImplementationOnce(async function(this: CheckpointPersistenceClient, operationId) {
          await originalRelease.call(this, operationId);
          released = true;
        });
      const exported = await fetch(`http://127.0.0.1:${server.port}/api/export/latest`);
      expect(exported.status).toBe(200);
      const archive = Buffer.from(await exported.arrayBuffer());
      await until(viewer, () => released);
      releaseSpy.mockRestore();
      releaseSpy = undefined;
      const retainedRows = legacyRecords(join(root, 'source.sqlite'));
      const managedDirectory = join(root, 'source.sqlite.checkpoints');
      /** Observe every durable file without materializing population data. */
      const retainedFiles = async (): Promise<Array<{ filename: string; sha256: string }>> =>
        Promise.all((await readdir(managedDirectory)).sort().map(async filename => ({
          filename, sha256: await archiveFileSha256(join(managedDirectory, filename))
        })));
      const durableFiles = await retainedFiles();
      for (const mutate of [
        (manifest: Record<string, unknown>) => { manifest['archiveKind'] = 'exact-generation-boundary-v1'; },
        (manifest: Record<string, unknown>) => { delete manifest['legacyConversion']; manifest['archiveKind'] = 'exact-generation-boundary-v1'; },
        (manifest: Record<string, unknown>) => { (manifest['legacyConversion'] as Record<string, unknown>)['sourceSeed'] = 1234568; }
      ]) {
        const rejected = await fetch(`http://127.0.0.1:${server.port}/api/import/archive`, {
          method: 'POST', body: new Uint8Array(rewriteLegacyManifest(archive, mutate)) });
        expect(rejected.status, await rejected.clone().text()).toBe(400);
        expect(await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json()).toMatchObject({
          ok: true, runId: result.runId, legacyConversion: result.legacyConversion });
        expect(legacyRecords(join(root, 'source.sqlite'))).toEqual(retainedRows);
        expect(await retainedFiles()).toEqual(durableFiles);
      }
      await migratedArchiveRoundTrip(root, server);
      viewer.socket.send(JSON.stringify({ type: replacement, ...(replacement === 'newRun' ? { requestId: 'clear-origin' } : {}) }));
      await until(viewer, () => viewer!.packets.some(packet => packet['type'] === 'stateReplaced' && packet['reason'] === replacement));
      expect(viewer.packets.findLast(packet => packet['type'] === 'stateReplaced')?.['welcome']).not.toHaveProperty('legacyConversion');
      expect(await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json()).not.toHaveProperty('legacyConversion');
      const freshExport = await fetch(`http://127.0.0.1:${server.port}/api/export/latest`);
      expect(legacyExportManifest(Buffer.from(await freshExport.arrayBuffer()))).toMatchObject({ archiveKind: 'exact-generation-boundary-v1' });
    } finally {
      releaseSpy?.mockRestore();
      viewer?.socket.terminate();
      await server?.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);

  it('converts the newest TypeScript v2 checkpoint and repeatedly resets without changing its source rows', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-v2-startup-'));
    const dbPath = join(root, 'experiment.sqlite');
    const core = { ...DEFAULT_CORE_SETTINGS, snakeCount: 2, simSpeed: 2, neurons1: 96, neurons2: 96 };
    const graphSpec = buildStackGraphSpec(core, CFG_DEFAULT);
    const graph = compileGraph(graphSpec);
    expect(graph.totalParams).toBeGreaterThan(64 * 1024 / Float32Array.BYTES_PER_ELEMENT);
    const firstWeights = Buffer.alloc(graph.totalParams * Float32Array.BYTES_PER_ELEMENT);
    const secondWeights = Buffer.from(firstWeights);
    secondWeights.writeFloatLE(0.25, 0);
    const metadata = JSON.stringify({
      formatVersion: 2,
      boundaryVersion: 1,
      boundaryKind: 'generation',
      resumable: true,
      generation: 19,
      simulationStep: 72_000,
      runId: 'typescript-source-run',
      worldSeed: 7_654_321,
      configHash: 'legacy-config',
      configRevision: 4,
      archKey: graph.key,
      graphSpec,
      populationCount: 2,
      settings: core,
      updates: [
        { path: 'baselineBots.count', value: 0 },
        { path: 'foodSpawn.edgeFalloffEnabled', value: 1 }
      ],
      rng: {},
      allocators: {},
      bestFitnessEver: 0,
      fitnessHistory: [],
      lastHofEntry: null
    });
    const sourceDigest = createHash('sha256')
      .update(metadata)
      .update(firstWeights)
      .update(secondWeights)
      .digest('hex');
    const database = new Database(dbPath);
    try {
      database.exec(`
        CREATE TABLE population_snapshots (
          id INTEGER PRIMARY KEY, payload_json TEXT, format_version INTEGER,
          boundary_kind TEXT, population_count INTEGER
        );
        CREATE TABLE snapshot_genomes (
          snapshot_id INTEGER NOT NULL, slot INTEGER NOT NULL, arch_key TEXT NOT NULL,
          brain_type TEXT NOT NULL, fitness REAL NOT NULL, weight_count INTEGER NOT NULL,
          weights_blob BLOB NOT NULL, weights_checksum TEXT NOT NULL,
          PRIMARY KEY (snapshot_id, slot)
        );
      `);
      database.prepare(`INSERT INTO population_snapshots
        (id, payload_json, format_version, boundary_kind, population_count)
        VALUES (1, ?, 2, 'generation', 2)`).run(metadata);
      const insert = database.prepare(`INSERT INTO snapshot_genomes
        (snapshot_id, slot, arch_key, brain_type, fitness, weight_count, weights_blob, weights_checksum)
        VALUES (1, ?, ?, 'mlp', ?, ?, ?, ?)`);
      for (const [slot, weights] of [firstWeights, secondWeights].entries()) {
        insert.run(slot, graph.key, 10 - slot, graph.totalParams, weights,
          createHash('sha256').update(weights).digest('hex'));
      }
      legacyCompanions(database, JSON.stringify(graphSpec), graph.key, graph.totalParams);
    } finally { database.close(); }
    const originalRecords = legacyRecords(dbPath);
    await readOnlyLegacyConversion(root, dbPath, 'typescript-v2').catch(async error => {
      await rm(root, { recursive: true, force: true });
      throw error;
    });
    const { seed: _defaultSeed, ...resumeConfig } = DEFAULT_CONFIG;
    let server = await startRustServer({
      ...resumeConfig, port: 0, resume: 'latest', dbPath
    });
    const peers: Peer[] = [];
    try {
      expect(server.startupFault).toBeUndefined();
      const health = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as {
        runId: string; startupCheckpointId: string; seed: number;
      };
      const legacyConversion = {
        version: 1, sourceRunId: 'typescript-source-run', sourceGeneration: '0000000000000013', sourceSeed: 7_654_321,
        sourceSnapshotId: 1,
        sourceFormat: 'typescript-v2',
        completeness: 'population-only',
        exactContinuation: false
      };
      expect(health).toMatchObject({
        ok: true, seed: expect.any(Number), generation: '0000000000000001', legacyConversion
      });
      expect(health.runId).not.toBe('typescript-source-run');
      const viewer = await connect(server.port, 'ui');
      peers.push(viewer);
      await until(viewer, () => viewer.packets.some(packet => packet['type'] === 'welcome'));
      expect(viewer.packets.find(packet => packet['type'] === 'welcome')).toMatchObject({
        worldSeed: health.seed,
        settings: { core: { snakeCount: 2, simSpeed: 2 } },
        legacyConversion
      });
      viewer.socket.terminate();
      await server.close();
      server = await startRustServer({
        ...resumeConfig, port: 0, resume: 'latest', dbPath
      });
      expect(server.startupFault).toBeUndefined();
      expect(await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json()).toMatchObject({
        ok: true, runId: health.runId, startupCheckpointId: health.startupCheckpointId,
        legacyConversion
      });
      await migratedArchiveRoundTrip(root, server, createHash('sha256')
        .update(firstWeights).update(secondWeights).digest('hex'));
      expect(legacyRecords(dbPath, originalRecords.tables)).toEqual(originalRecords);
      const retained = new Database(dbPath, { readonly: true });
      try {
        const row = retained.prepare('SELECT payload_json FROM population_snapshots WHERE id = 1')
          .get() as { payload_json: string };
        const blobs = retained.prepare(`SELECT weights_blob FROM snapshot_genomes
          WHERE snapshot_id = 1 ORDER BY slot`).all() as Array<{ weights_blob: Buffer }>;
        expect(createHash('sha256').update(row.payload_json)
          .update(blobs[0]!.weights_blob).update(blobs[1]!.weights_blob).digest('hex')).toBe(sourceDigest);
      } finally { retained.close(); }
      const resetPeer = await connect(server.port, 'ui');
      peers.push(resetPeer);
      resetPeer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator' }));
      let activeRunId = health.runId;
      for (let index = 0; index < 5; index++) {
        resetPeer.packets.length = 0;
        resetPeer.socket.send(JSON.stringify({ type: 'reset', settings: { simSpeed: 0.1 },
          updates: [{ path: 'worldRadius', value: 4200 + index * 50 }] }));
        const replaced = await replacementUntil(resetPeer, server.port, 'reset');
        expect(replaced.worldSeed).toBe(health.seed);
        expect(replaced.runId).not.toBe(activeRunId);
        expect(replaced).not.toHaveProperty('legacyConversion');
        expect(replaced.settings.updates).toContainEqual({ path: 'worldRadius', value: 4200 + index * 50 });
        expect(resetPeer.packets.filter(packet => packet['type'] === 'error')).toEqual([]);
        activeRunId = replaced.runId;
        resetPeer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator', rejoinToken: resetPeer.rejoinToken }));
      }
      resetPeer.socket.terminate();
      await server.close();
      server = await startRustServer({ ...resumeConfig, port: 0, resume: 'latest', dbPath });
      expect(server.startupFault).toBeUndefined();
      expect(await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json())
        .toMatchObject({ ok: true, runId: activeRunId, seed: health.seed });
      expect(legacyRecords(dbPath, originalRecords.tables)).toEqual(originalRecords);
      const afterResets = new Database(dbPath, { readonly: true });
      try {
        expect(afterResets.prepare('PRAGMA foreign_key_check').all()).toEqual([]);
        expect(afterResets.prepare('SELECT run_id FROM rust_checkpoint_v3_current WHERE run_id = ?').get(health.runId))
          .toBeUndefined();
        expect(afterResets.prepare('SELECT source_snapshot_id, source_format, completeness FROM rust_legacy_conversions_v1 WHERE run_id = ?')
          .get(health.runId)).toEqual({ source_snapshot_id: 1, source_format: 'typescript-v2', completeness: 'population-only' });
      } finally { afterResets.close(); }
    } finally {
      for (const peer of peers) peer.socket.terminate();
      await server.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);

  it.each((['gzip', 'embedded'] as const).flatMap(storage =>
    (['absent', 'null', 'zero'] as const).map(format => ({ storage, format }))))(
    'converts a $storage population with $format format column without changing any source row',
    async ({ storage, format }) => {
      const root = await mkdtemp(join(tmpdir(), `slither-rust-v0-${storage}-`));
      const dbPath = join(root, 'experiment.sqlite');
      const core = { ...DEFAULT_CORE_SETTINGS, snakeCount: 2, simSpeed: 3 };
      const graphSpec = buildStackGraphSpec(core, CFG_DEFAULT);
      const graph = compileGraph(graphSpec);
      const first = new Array<number>(graph.totalParams).fill(0);
      const second = new Array<number>(graph.totalParams).fill(0);
      second[0] = 0.25;
      const genomes = [first, second].map((weights, slot) => ({
        archKey: graph.key, brainType: 'mlp', fitness: 10 - slot, weights
      }));
      const payload = JSON.stringify({
        generation: 7,
        archKey: graph.key,
        genomes: storage === 'embedded' ? genomes : [],
        cfgHash: 'legacy-config',
        worldSeed: 1_234_567,
        graphSpec,
        ...(storage === 'embedded' ? {
          settings: core,
          updates: [{ path: 'baselineBots.count', value: 0 }]
        } : {})
      });
      const framed = storage === 'gzip' ? gzipSync(Buffer.concat(genomes.flatMap(genome => {
        const encoded = Buffer.from(JSON.stringify(genome));
        const prefix = Buffer.alloc(4);
        prefix.writeUInt32LE(encoded.byteLength);
        return [prefix, encoded];
      }))) : null;
      const sourceDigest = createHash('sha256').update(payload).update(framed ?? Buffer.alloc(0)).digest('hex');
      const database = new Database(dbPath);
      try {
        database.exec(storage === 'gzip' ? `CREATE TABLE population_snapshots (
          id INTEGER PRIMARY KEY, created_at INTEGER, gen INTEGER, payload_json TEXT,
          settings_json TEXT, updates_json TEXT, genomes_blob BLOB
        )` : `CREATE TABLE population_snapshots (
          id INTEGER PRIMARY KEY, created_at INTEGER, gen INTEGER, payload_json TEXT
        )`);
        if (format !== 'absent') database.exec('ALTER TABLE population_snapshots ADD COLUMN format_version INTEGER');
        if (storage === 'gzip') {
          database.prepare(`INSERT INTO population_snapshots
            (id, created_at, gen, payload_json, settings_json, updates_json, genomes_blob)
            VALUES (1, ?, 7, ?, ?, ?, ?)`).run(Date.now(), payload, JSON.stringify(core),
              JSON.stringify([{ path: 'baselineBots.count', value: 0 }]), framed);
        } else {
          database.prepare(`INSERT INTO population_snapshots
            (id, created_at, gen, payload_json) VALUES (1, ?, 7, ?)`).run(Date.now(), payload);
        }
        if (format !== 'absent') database.prepare('UPDATE population_snapshots SET format_version = ? WHERE id = 1')
          .run(format === 'zero' ? 0 : null);
        legacyCompanions(database, JSON.stringify(graphSpec), graph.key, graph.totalParams);
      } finally { database.close(); }
      const originalRecords = legacyRecords(dbPath);
      await readOnlyLegacyConversion(root, dbPath, storage === 'gzip' ? 'legacy-gzip' : 'legacy-json').catch(async error => {
        await rm(root, { recursive: true, force: true });
        throw error;
      });

      const { seed: _defaultSeed, ...resumeConfig } = DEFAULT_CONFIG;
      const server = await startRustServer({
        ...resumeConfig, port: 0, resume: 'latest', dbPath
      });
      const peers: Peer[] = [];
      try {
        expect(server.startupFault).toBeUndefined();
        const legacyConversion = {
          version: 1, sourceGeneration: '0000000000000007', sourceSeed: 1_234_567,
          sourceSnapshotId: 1,
          sourceFormat: storage === 'gzip' ? 'legacy-gzip' : 'legacy-json',
          completeness: 'population-only',
          exactContinuation: false
        };
        expect(await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json()).toMatchObject({
          ok: true, seed: expect.any(Number), generation: '0000000000000001', legacyConversion
        });
        const viewer = await connect(server.port, 'ui');
        peers.push(viewer);
        await until(viewer, () => viewer.packets.some(packet => packet['type'] === 'welcome'));
        expect(viewer.packets.find(packet => packet['type'] === 'welcome')).toMatchObject({
          worldSeed: expect.any(Number), settings: { core: { snakeCount: 2, simSpeed: 3 } },
          legacyConversion
        });
        const retained = new Database(dbPath, { readonly: true });
        try {
          const row = retained.prepare(`SELECT payload_json${storage === 'gzip' ? ', genomes_blob' : ''}
            FROM population_snapshots WHERE id = 1`).get() as { payload_json: string; genomes_blob?: Buffer };
          expect(createHash('sha256').update(row.payload_json)
            .update(row.genomes_blob ?? Buffer.alloc(0)).digest('hex')).toBe(sourceDigest);
          expect((retained.prepare('SELECT count(*) AS count FROM rust_checkpoint_v3_current').get() as
            { count: number }).count).toBe(1);
          expect(retained.prepare(`SELECT source_snapshot_id, source_format, completeness
            FROM rust_legacy_conversions_v1`).get()).toEqual({
            source_snapshot_id: 1,
            source_format: legacyConversion.sourceFormat,
            completeness: 'population-only'
          });
        } finally { retained.close(); }
        const packedWeights = Buffer.alloc(graph.totalParams * 2 * Float32Array.BYTES_PER_ELEMENT);
        packedWeights.writeFloatLE(0.25, graph.totalParams * Float32Array.BYTES_PER_ELEMENT);
        await migratedArchiveRoundTrip(root, server, createHash('sha256').update(packedWeights).digest('hex'));
        expect(legacyRecords(dbPath, originalRecords.tables)).toEqual(originalRecords);
      } finally {
        for (const peer of peers) peer.socket.terminate();
        await server.close();
        await rm(root, { recursive: true, force: true });
      }
    },
    30_000
  );

  it.each([false, true])('streams, imports, and atomically activates one exact Rust save (phase trace=%s)', async trace => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-export-server-'));
    const dbPath = join(root, 'experiment.sqlite');
    const managedDirectory = `${dbPath}.checkpoints`;
    const server = await startRustServer({
      ...DEFAULT_CONFIG, port: 0, resume: 'fresh', seed: 41, dbPath
    });
    let target: Awaited<ReturnType<typeof startRustServer>> | undefined;
    const peers: Peer[] = [];
    try {
      vi.stubEnv('SLITHER_TRACE_ARCHIVE_PHASES', trace ? '1' : '0');
      const health = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as {
        runId: string; startupCheckpointId: string; archiveWork: unknown;
        schedulerDroppedWallMicros: string; schedulerOverloaded: boolean;
      };
      expect(health.archiveWork).toBeNull();
      expect(health.schedulerDroppedWallMicros).toMatch(/^[0-9a-f]{16}$/u);
      expect(typeof health.schedulerOverloaded).toBe('boolean');
      const hallOfFame = await fetch(`http://127.0.0.1:${server.port}/api/hof`);
      expect(hallOfFame.status).toBe(200);
      expect(await hallOfFame.json()).toEqual({ hof: [] });
      const missingWinner = await fetch(`http://127.0.0.1:${server.port}/api/resurrect`, {
        method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ entryId: '0000000000000001' })
      });
      expect(missingWinner.status).toBe(400);
      expect(await missingWinner.json()).toMatchObject({ ok: false });
      const exported = await fetch(`http://127.0.0.1:${server.port}/api/export/latest`);
      expect(exported.status).toBe(200);
      expect(exported.headers.get('content-type')).toBe('application/vnd.slither-neuroevo.save');
      expect(exported.headers.get('content-disposition')).toMatch(
        /^attachment; filename="slither-neuroevo-[0-9a-f]{12}-gen-1-v1\.slither-save"$/u
      );
      expect(exported.headers.get('x-slither-checkpoint-id')).toBe(health.startupCheckpointId);
      expect(exported.headers.get('x-slither-save-root')).toMatch(/^[0-9a-f]{64}$/u);
      const concurrent = await fetch(`http://127.0.0.1:${server.port}/api/export/latest`);
      expect(concurrent.status).toBe(409);
      await concurrent.body?.cancel();
      const bytes = Buffer.from(await exported.arrayBuffer());
      expect(bytes.byteLength).toBe(Number(exported.headers.get('content-length')));
      expect(bytes.subarray(0, 100).toString('utf8').replace(/\0.*$/u, '')).toBe('checkpoint.bin');
      expect(bytes.includes(Buffer.from('checkpoint/checkpoint-v3.ustar\0'))).toBe(false);
      expect(bytes.includes(Buffer.from('graph.bin\0'))).toBe(true);
      expect(bytes.includes(Buffer.from('population/index.bin\0'))).toBe(true);
      expect(bytes.includes(Buffer.from('population/weights.'))).toBe(true);
      expect(bytes.includes(Buffer.from('population/recurrent.'))).toBe(true);
      expect(bytes.includes(Buffer.from('history.bin\0'))).toBe(true);
      expect(bytes.includes(Buffer.from('hof/index.bin\0'))).toBe(true);
      expect(bytes.includes(Buffer.from('hof/weights.f32le\0'))).toBe(true);
      expect(bytes.includes(Buffer.from('manifest.json\0'))).toBe(true);
      const cleanupDeadline = performance.now() + 2000;
      let leftovers: string[] = [];
      do {
        leftovers = (await readdir(managedDirectory)).filter(name =>
          name.includes('export-inventory') || name.includes('slither-save') || name.includes('export-hof')
        );
        if (leftovers.length === 0) break;
        await new Promise<void>(done => setTimeout(done, 10));
      } while (performance.now() < cleanupDeadline);
      expect(leftovers).toEqual([]);
      const afterExport = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as {
        archiveWork: RustArchiveWorkProgress;
      };
      expect(afterExport.archiveWork).toMatchObject({ kind: 'export', started: true, finished: true });
      expect(BigInt(`0x${afterExport.archiveWork.completedBytes}`)).toBeGreaterThan(0n);
      if (trace) {
        const timings = afterExport.archiveWork.phaseTrace!;
        expect(timings.truncated).toBe(false);
        expect(timings.intervals.length).toBeLessThanOrEqual(4096);
        expect(timings.rssSamplerStarted).toBe(true);
        expect(BigInt(`0x${timings.requestedRssSampleIntervalMicros}`)).toBe(2000n);
        for (const interval of timings.intervals) {
          expect(BigInt(`0x${interval.startRssBytes}`)).toBeGreaterThan(0n);
          expect(BigInt(`0x${interval.finishRssBytes}`)).toBeGreaterThan(0n);
          expect(BigInt(`0x${interval.rssSamples}`)).toBeGreaterThanOrEqual(2n);
          expect(BigInt(`0x${interval.sampledPeakRssBytes}`)).toBeGreaterThanOrEqual(BigInt(`0x${interval.startRssBytes}`));
          expect(BigInt(`0x${interval.sampledPeakRssBytes}`)).toBeGreaterThanOrEqual(BigInt(`0x${interval.finishRssBytes}`));
        }
        expect(timings.intervals.every(interval => interval.finishedMicros !== undefined)).toBe(true);
        const source = timings.intervals.find(interval => interval.phase === 'export-source-population')!;
        const write = timings.intervals.find(interval => interval.phase === 'temporary-file-write')!;
        const validation = timings.intervals.find(interval => interval.phase === 'validation')!;
        expect(BigInt(`0x${source.finishedMicros}`)).toBeLessThanOrEqual(BigInt(`0x${write.startedMicros}`));
        expect(BigInt(`0x${write.finishedMicros}`)).toBeLessThanOrEqual(BigInt(`0x${validation.startedMicros}`));
      } else expect(afterExport.archiveWork.phaseTrace).toBeUndefined();

      const targetDbPath = join(root, 'target.sqlite');
      target = await startRustServer({
        ...DEFAULT_CONFIG, port: 0, resume: 'fresh', seed: 42, dbPath: targetDbPath
      });
      const targetBefore = await (await fetch(`http://127.0.0.1:${target.port}/api/health`)).json() as {
        runId: string; startupCheckpointId: string;
      };
      expect(targetBefore.runId).not.toBe(health.runId);
      const viewer = await connect(target.port, 'ui');
      peers.push(viewer);
      await until(viewer, () => viewer.packets.some(packet => packet['type'] === 'welcome'));
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
        rejoinToken: viewer.rejoinToken }));
      expect(viewer.packets.find(packet => packet['type'] === 'welcome')).toMatchObject({
        capabilities: { archiveExport: true, archiveImport: true }
      });
      const importedResponse = await fetch(`http://127.0.0.1:${target.port}/api/import/archive`, {
        method: 'POST', headers: { 'Content-Type': 'application/vnd.slither-neuroevo.save' }, body: bytes
      });
      expect(importedResponse.status).toBe(200);
      expect(await importedResponse.json()).toMatchObject({
        ok: true, runId: health.runId, generation: '0000000000000001', checkpointId: health.startupCheckpointId
      });
      const importedHealth = await (await fetch(`http://127.0.0.1:${target.port}/api/health`)).json() as {
        archiveWork: RustArchiveWorkProgress;
      };
      expect(importedHealth.archiveWork).toMatchObject({ kind: 'import', started: true, finished: true });
      expect(BigInt(`0x${importedHealth.archiveWork.completedBytes}`)).toBeGreaterThan(0n);
      if (trace) {
        const timings = importedHealth.archiveWork.phaseTrace!;
        expect(timings.truncated).toBe(false);
        expect(timings.rssSamplerStarted).toBe(true);
        for (const interval of timings.intervals) {
          expect(BigInt(`0x${interval.startRssBytes}`)).toBeGreaterThan(0n);
          expect(BigInt(`0x${interval.finishRssBytes}`)).toBeGreaterThan(0n);
          expect(BigInt(`0x${interval.rssSamples}`)).toBeGreaterThanOrEqual(2n);
        }
        expect(timings.intervals.every(interval => interval.finishedMicros !== undefined)).toBe(true);
        expect(new Set(timings.intervals.map(interval => interval.phase))).toEqual(new Set([
          'import', 'validation', 'temporary-file-write', 'numeric-decode', 'checkpoint-restore',
          'managed-publication', 'candidate-construction'
        ]));
      } else expect(importedHealth.archiveWork.phaseTrace).toBeUndefined();
      await until(viewer, () => viewer.packets.some(packet => packet['type'] === 'stateReplaced'));
      expect(viewer.socket.readyState).toBe(WebSocket.OPEN);
      expect(viewer.packets.find(packet => packet['type'] === 'stateReplaced')).toMatchObject({
        reason: 'import', checkpointId: health.startupCheckpointId,
        welcome: { runId: health.runId, worldSeed: 41 }
      });
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
        rejoinToken: viewer.rejoinToken }));
      expect(await (await fetch(`http://127.0.0.1:${target.port}/api/health`)).json()).toMatchObject({
        ok: true, runId: health.runId, seed: 41, startupCheckpointId: health.startupCheckpointId
      });

      const replay = await fetch(`http://127.0.0.1:${target.port}/api/import/archive`, {
        method: 'POST', headers: { 'Content-Type': 'application/vnd.slither-neuroevo.save' }, body: bytes
      });
      expect(replay.status).toBe(200);
      expect(await replay.json()).toMatchObject({ ok: true, checkpointId: health.startupCheckpointId });

      const futureDatabase = new Database(targetDbPath);
      try {
        futureDatabase.prepare(`INSERT INTO rust_generation_history_v1
          (run_id, generation_hex, checkpoint_id, record_version, record_blob, created_at_ms)
          VALUES (?, '0000000000000002', NULL, 1, ?, ?)`)
          .run(health.runId, Buffer.alloc(56), Date.now());
      } finally { futureDatabase.close(); }
      const requiresBranch = await fetch(`http://127.0.0.1:${target.port}/api/import/archive`, {
        method: 'POST', headers: { 'Content-Type': 'application/vnd.slither-neuroevo.save' }, body: bytes
      });
      const requiresBranchBody = await requiresBranch.json();
      expect({ status: requiresBranch.status, body: requiresBranchBody }).toMatchObject({
        status: 409, body: { ok: false, code: 'IMPORT_REQUIRES_BRANCH' }
      });
      const branchedResponse = await fetch(`http://127.0.0.1:${target.port}/api/import/archive?mode=branch`, {
        method: 'POST', headers: { 'Content-Type': 'application/vnd.slither-neuroevo.save' }, body: bytes
      });
      const branched = await branchedResponse.json() as { runId: string; branched: boolean; sourceRunId: string; message?: string };
      expect({ status: branchedResponse.status, body: branched }).toMatchObject({ status: 200 });
      expect(branched).toMatchObject({ branched: true, sourceRunId: health.runId });
      expect(branched.runId).not.toBe(health.runId);
      expect(await (await fetch(`http://127.0.0.1:${target.port}/api/health`)).json()).toMatchObject({
        ok: true, runId: branched.runId,
        importBranch: { sourceRunId: health.runId, branchRunId: branched.runId,
          sourceGeneration: '0000000000000001', sourceCheckpointId: health.startupCheckpointId }
      });

      await target.close();
      const { seed: _defaultSeed, ...resumeConfig } = DEFAULT_CONFIG;
      target = await startRustServer({
        ...resumeConfig, port: 0, resume: 'latest', dbPath: targetDbPath
      });
      expect(await (await fetch(`http://127.0.0.1:${target.port}/api/health`)).json()).toMatchObject({
        ok: true, runId: branched.runId,
        importBranch: { sourceRunId: health.runId, branchRunId: branched.runId }
      });
      const corrupt = Buffer.from(bytes);
      corrupt[512] = (corrupt[512] ?? 0) ^ 0xff;
      const rejected = await fetch(`http://127.0.0.1:${target.port}/api/import/archive`, {
        method: 'POST', headers: { 'Content-Type': 'application/vnd.slither-neuroevo.save' }, body: corrupt
      });
      expect(rejected.status).toBe(400);
      expect(await (await fetch(`http://127.0.0.1:${target.port}/api/health`)).json()).toMatchObject({
        ok: true, runId: branched.runId, startupCheckpointId: health.startupCheckpointId
      });
      const targetFiles = await readdir(`${targetDbPath}.checkpoints`);
      expect(targetFiles.filter(name => name.includes('upload') || name.includes('import-inventory') ||
        name.includes('import-stage'))).toEqual([]);
    } finally {
      for (const peer of peers) peer.socket.terminate();
      await target?.close();
      await server.close();
      await rm(root, { recursive: true, force: true });
      vi.unstubAllEnvs();
    }
  }, 30_000);

  it.each(['before headers', 'after headers'] as const)(
    'releases a cancelled export %s and admits a fresh direct request', async boundary => {
      const root = await mkdtemp(join(tmpdir(), 'slither-rust-export-cancel-'));
      const dbPath = join(root, 'experiment.sqlite');
      const managedDirectory = `${dbPath}.checkpoints`;
      const server = await startRustServer({ ...DEFAULT_CONFIG, port: 0,
        resume: 'fresh', seed: 41, dbPath });
      /** Hold only the real selected lease so cancellation precedes preparation deterministically. */
      const gate = Promise.withResolvers<void>();
      /** Observe the actual persistence lease rather than guessing from request timing. */
      const selected = Promise.withResolvers<Awaited<ReturnType<CheckpointPersistenceClient['acquireCurrentExportLease']>>>();
      const originalAcquire = CheckpointPersistenceClient.prototype.acquireCurrentExportLease;
      const acquire = vi.spyOn(CheckpointPersistenceClient.prototype, 'acquireCurrentExportLease')
        .mockImplementationOnce(async function(this: CheckpointPersistenceClient) {
          const lease = await originalAcquire.call(this);
          selected.resolve(lease);
          if (boundary === 'before headers') await gate.promise;
          return lease;
        });
      const release = vi.spyOn(CheckpointPersistenceClient.prototype, 'releaseExportLease');
      /** Observe real server termination so an already-complete response cannot masquerade as cancellation. */
      let cancelledResponseClosedBeforeFinish: boolean | undefined;
      let observedExport = false;
      const originalEmit = Server.prototype.emit;
      const dispatch = vi.spyOn(Server.prototype, 'emit').mockImplementation(function(
        this: Server, event: string | symbol, ...args: unknown[]
      ): boolean {
        if (event === 'request') {
          const incoming = args[0] as IncomingMessage;
          const response = args[1] as ServerResponse;
          if (!observedExport && incoming.socket.localPort === server.port &&
              incoming.url === '/api/export/latest') {
            observedExport = true;
            response.once('close', () => {
              cancelledResponseClosedBeforeFinish = !response.writableFinished;
            });
          }
        }
        return Reflect.apply(originalEmit, this, [event, ...args]) as boolean;
      });
      const request = httpRequest(`http://127.0.0.1:${server.port}/api/export/latest`);
      /** Client close is observed independently of server cleanup. */
      const closed = new Promise<void>(done => request.once('close', done));
      let receivedHeaders = false;
      request.on('error', () => { /* Destroying the cancelled client may report ECONNRESET. */ });
      request.on('response', response => {
        receivedHeaders = true;
        expect(response.statusCode).toBe(200);
        response.destroy();
      });
      /** Snapshot every durable metadata row; export must not change the experiment. */
      const metadata = (): unknown => {
        const database = new Database(dbPath, { readonly: true });
        try {
          const tables = database.prepare(`SELECT name FROM sqlite_master
            WHERE type = 'table' AND name LIKE 'rust_%' ORDER BY name`).all() as Array<{ name: string }>;
          return tables.map(({ name }) => ({ name, rows: database.prepare(
            `SELECT * FROM "${name.replaceAll('"', '""')}" ORDER BY rowid`
          ).all() }));
        } finally { database.close(); }
      };
      /** Check exact source file bytes as well as filenames and SQLite references. */
      const managedHashes = async (files: readonly string[]): Promise<string[]> => Promise.all(
        files.map(async name => createHash('sha256').update(await readFile(join(managedDirectory, name))).digest('hex'))
      );
      try {
        const before = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as {
          runId: string; generation: string; startupCheckpointId: string; configHash: string; seed: number;
        };
        const beforeMetadata = metadata();
        const beforeFiles = (await readdir(managedDirectory)).sort();
        const beforeHashes = await managedHashes(beforeFiles);
        request.end();
        const lease = await selected.promise;
        expect(lease.descriptor.logicalRootSha256).toBe(before.startupCheckpointId);
        if (boundary === 'before headers') request.destroy();
        await closed;
        expect(receivedHeaders).toBe(boundary === 'after headers');
        gate.resolve();

        const deadline = performance.now() + 2000;
        let files: string[];
        do {
          files = (await readdir(managedDirectory)).sort();
          if (release.mock.calls.length === 1 && JSON.stringify(files) === JSON.stringify(beforeFiles)) break;
          await new Promise<void>(done => setTimeout(done, 10));
        } while (performance.now() < deadline);
        expect(files!).toEqual(beforeFiles);
        expect(cancelledResponseClosedBeforeFinish).toBe(true);
        expect(release).toHaveBeenCalledExactlyOnceWith(lease.operationId);
        await release.mock.results[0]!.value;
        expect(metadata()).toEqual(beforeMetadata);
        expect(await managedHashes(beforeFiles)).toEqual(beforeHashes);
        expect(await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json()).toMatchObject({
          ok: true, runId: before.runId, generation: before.generation,
          startupCheckpointId: before.startupCheckpointId, configHash: before.configHash, seed: before.seed
        });

        const fresh = await fetch(`http://127.0.0.1:${server.port}/api/export/latest`);
        expect(fresh.status).toBe(200);
        expect(fresh.headers.get('x-slither-checkpoint-id')).toBe(before.startupCheckpointId);
        expect(fresh.headers.get('content-disposition')).toContain('-gen-1-v1.slither-save');
        expect((await fresh.arrayBuffer()).byteLength).toBe(Number(fresh.headers.get('content-length')));
        const retryDeadline = performance.now() + 2000;
        do {
          files = (await readdir(managedDirectory)).sort();
          if (release.mock.calls.length === 2 && JSON.stringify(files) === JSON.stringify(beforeFiles)) break;
          await new Promise<void>(done => setTimeout(done, 10));
        } while (performance.now() < retryDeadline);
        expect(files!).toEqual(beforeFiles);
        expect(acquire).toHaveBeenCalledTimes(2);
        expect(release).toHaveBeenCalledTimes(2);
        await release.mock.results[1]!.value;
        expect(metadata()).toEqual(beforeMetadata);
        expect(await managedHashes(beforeFiles)).toEqual(beforeHashes);
      } finally {
        request.destroy();
        gate.resolve();
        try { await server.close(); }
        finally {
          dispatch.mockRestore();
          acquire.mockRestore();
          release.mockRestore();
          await rm(root, { recursive: true, force: true });
        }
      }
    }, 20_000
  );

  it.runIf(process.env['SLITHER_FULL_DOWNLOAD_TIMEOUT_TEST'] === '1')(
    'releases a connected large download after the full no-progress deadline', async () => {
      const archivePath = process.env['SLITHER_DOWNLOAD_TIMEOUT_ARCHIVE'];
      if (!archivePath) throw new Error('SLITHER_DOWNLOAD_TIMEOUT_ARCHIVE must name a verified save over 50 MiB');
      const archive = await stat(archivePath);
      expect(archive.isFile()).toBe(true);
      expect(archive.size).toBeGreaterThan(50 * 1024 * 1024);
      const originalSha256 = await archiveFileSha256(archivePath);
      const root = await mkdtemp(join(tmpdir(), 'slither-rust-stalled-export-'));
      const dbPath = join(root, 'experiment.sqlite');
      const managedDirectory = `${dbPath}.checkpoints`;
      const server = await startRustServer({ ...DEFAULT_CONFIG, port: 0,
        resume: 'fresh', seed: 41, dbPath, rustCalculationWorkers: 6 });
      /** Observe server-side close even while the client refuses to drain buffered bytes. */
      const terminal = Promise.withResolvers<{ finished: boolean; elapsedSinceProgressMs: number }>();
      let observedExport = false;
      let lastProgressAt = 0;
      const originalEmit = Server.prototype.emit;
      const dispatch = vi.spyOn(Server.prototype, 'emit').mockImplementation(function(
        this: Server, event: string | symbol, ...args: unknown[]
      ): boolean {
        if (event === 'request') {
          const incoming = args[0] as IncomingMessage;
          const response = args[1] as ServerResponse;
          if (!observedExport && incoming.socket.localPort === server.port &&
              incoming.url === '/api/export/latest') {
            observedExport = true;
            const originalWrite = response.write;
            response.write = function(...values: unknown[]): boolean {
              lastProgressAt = performance.now();
              return Reflect.apply(originalWrite, this, values) as boolean;
            };
            response.on('drain', () => { lastProgressAt = performance.now(); });
            response.once('close', () => terminal.resolve({ finished: response.writableFinished,
              elapsedSinceProgressMs: performance.now() - lastProgressAt }));
          }
        }
        return Reflect.apply(originalEmit, this, [event, ...args]) as boolean;
      });
      const release = vi.spyOn(CheckpointPersistenceClient.prototype, 'releaseExportLease');
      const request = httpRequest(`http://127.0.0.1:${server.port}/api/export/latest`);
      request.on('error', () => { /* The timed-out server may reset the blocked client. */ });
      /** Original headers arrive before deliberately stopping all client body reads. */
      const headers = Promise.withResolvers<{ checkpointId: string; contentLength: number }>();
      request.once('response', response => {
        response.pause();
        response.on('error', () => { /* Expected when the server ends an idle download. */ });
        headers.resolve({ checkpointId: String(response.headers['x-slither-checkpoint-id']),
          contentLength: Number(response.headers['content-length']) });
      });
      try {
        const imported = await fetch(`http://127.0.0.1:${server.port}/api/import/archive`, {
          method: 'POST', headers: { 'Content-Type': 'application/vnd.slither-neuroevo.save',
            'Content-Length': String(archive.size) },
          body: createReadStream(archivePath) as unknown as BodyInit, duplex: 'half'
        } as RequestInit & { duplex: 'half' });
        expect(imported.status).toBe(200);
        const receipt = await imported.json() as { ok: boolean; runId: string; checkpointId: string };
        expect(receipt.ok).toBe(true);
        const before = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as {
          runId: string; configHash: string; completedStep: string;
        };
        request.end();
        const advertised = await headers.promise;
        expect(advertised.checkpointId).toBe(receipt.checkpointId);
        expect(advertised.contentLength).toBeGreaterThan(50 * 1024 * 1024);
        const result = await terminal.promise;
        expect(result.finished).toBe(false);
        expect(result.elapsedSinceProgressMs).toBeGreaterThanOrEqual(59_000);
        expect(result.elapsedSinceProgressMs).toBeLessThan(70_000);
        // Keep the non-reading client connected until the actual server timeout is observed.
        request.destroy();
        const cleanupDeadline = performance.now() + 2000;
        let leftovers: string[];
        do {
          leftovers = (await readdir(managedDirectory)).filter(name =>
            name.includes('export-inventory') || name.includes('slither-save') || name.includes('export-hof'));
          if (release.mock.calls.length === 1 && leftovers.length === 0) break;
          await new Promise<void>(done => setTimeout(done, 10));
        } while (performance.now() < cleanupDeadline);
        expect(leftovers!).toEqual([]);
        expect(release).toHaveBeenCalledTimes(1);
        await release.mock.results[0]!.value;
        const after = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as {
          completedStep: string; generation: string;
        };
        expect(after).toMatchObject({ ok: true, runId: before.runId, configHash: before.configHash });
        expect(BigInt(`0x${after.completedStep}`)).toBeGreaterThan(BigInt(`0x${before.completedStep}`));
        expect(await archiveFileSha256(archivePath)).toBe(originalSha256);
        console.log(JSON.stringify({ idleDownload: result, advertisedBytes: advertised.contentLength,
          generationAfterTimeout: after.generation, originalArchiveSha256: originalSha256 }));
      } finally {
        request.destroy();
        try { await server.close(); }
        finally {
          dispatch.mockRestore();
          release.mockRestore();
          await rm(root, { recursive: true, force: true });
        }
      }
    }, 90_000
  );

  it('faults the old authority when an import commits but its reply is lost', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-lost-import-reply-'));
    const sourceDbPath = join(root, 'source.sqlite');
    const targetDbPath = join(root, 'target.sqlite');
    const source = await startRustServer({ ...DEFAULT_CONFIG, port: 0,
      resume: 'fresh', seed: 41, dbPath: sourceDbPath });
    let target: Awaited<ReturnType<typeof startRustServer>> | undefined;
    let importSpy: ReturnType<typeof vi.spyOn> | undefined;
    try {
      const sourceHealth = await (await fetch(`http://127.0.0.1:${source.port}/api/health`)).json() as {
        runId: string; startupCheckpointId: string;
      };
      const archive = Buffer.from(await (await fetch(
        `http://127.0.0.1:${source.port}/api/export/latest`
      )).arrayBuffer());
      target = await startRustServer({ ...DEFAULT_CONFIG, port: 0,
        resume: 'fresh', seed: 42, dbPath: targetDbPath });
      const oldHealth = await (await fetch(`http://127.0.0.1:${target.port}/api/health`)).json() as {
        runId: string; startupCheckpointId: string;
      };
      const originalCommit = CheckpointPersistenceClient.prototype.commitImport;
      /** Lose one reply after the real worker has committed the new pointer. */
      importSpy = vi.spyOn(CheckpointPersistenceClient.prototype, 'commitImport')
        .mockImplementationOnce(async function (this: CheckpointPersistenceClient,
          ...args: Parameters<CheckpointPersistenceClient['commitImport']>) {
          await originalCommit.apply(this, args);
          throw new Error('injected lost import commit reply');
        });
      const response = await fetch(`http://127.0.0.1:${target.port}/api/import/archive`, {
        method: 'POST', headers: { 'Content-Type': 'application/vnd.slither-neuroevo.save' }, body: archive
      });
      expect(response.status).toBe(503);
      expect(await response.json()).toMatchObject({ ok: false, message: 'injected lost import commit reply' });
      const faulted = await healthUntil(target.port, health => health['ok'] === false);
      expect(faulted).toMatchObject({ runId: oldHealth.runId,
        startupCheckpointId: oldHealth.startupCheckpointId,
        interfaceFault: 'injected lost import commit reply' });
      expect(['stopRequested', 'stopped']).toContain(faulted['lifecycle']);
      const stopped = await healthUntil(target.port, health => health['lifecycle'] === 'stopped');
      await new Promise<void>(done => setTimeout(done, 50));
      expect(await (await fetch(`http://127.0.0.1:${target.port}/api/health`)).json()).toMatchObject({
        completedStep: stopped['completedStep']
      });
      const database = new Database(targetDbPath, { readonly: true });
      try {
        expect(database.prepare('SELECT run_id FROM rust_active_run_v1 WHERE singleton = 1').get())
          .toEqual({ run_id: sourceHealth.runId });
      } finally { database.close(); }
      await target.close();
      target = undefined;
      const { seed: _seed, ...resumeConfig } = DEFAULT_CONFIG;
      target = await startRustServer({ ...resumeConfig, port: 0,
        resume: 'latest', dbPath: targetDbPath });
      expect(await (await fetch(`http://127.0.0.1:${target.port}/api/health`)).json()).toMatchObject({
        ok: true, runId: sourceHealth.runId, startupCheckpointId: sourceHealth.startupCheckpointId
      });
    } finally {
      importSpy?.mockRestore();
      await target?.close();
      await source.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);

  it('reclaims a rejected New Run candidate before resuming the unchanged old authority', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-rejected-new-run-'));
    const dbPath = join(root, 'experiment.sqlite');
    const server = await startRustServer({ ...DEFAULT_CONFIG, port: 0,
      resume: 'fresh', seed: 42, dbPath });
    let viewer: Peer | undefined;
    const cleanup = vi.spyOn(CheckpointPersistenceClient.prototype, 'reclaimManagedOrphans');
    /** Preserve every managed byte and metadata row across a real SQLite rejection. */
    const snapshot = async (): Promise<unknown> => {
      const database = new Database(dbPath, { readonly: true });
      try {
        const tables = database.prepare(`SELECT name FROM sqlite_master
          WHERE type = 'table' AND name LIKE 'rust_%' ORDER BY name`).all() as Array<{ name: string }>;
        const names = (await readdir(`${dbPath}.checkpoints`)).sort();
        return { tables: tables.map(({ name }) => ({ name,
          rows: database.prepare(`SELECT * FROM "${name.replaceAll('"', '""')}" ORDER BY rowid`).all() })),
        files: await Promise.all(names.map(async name => ({ name,
          hash: createHash('sha256').update(await readFile(join(`${dbPath}.checkpoints`, name))).digest('hex') }))) };
      } finally { database.close(); }
    };
    try {
      viewer = await connect(server.port, 'ui');
      await until(viewer, () => viewer!.packets.some(packet => packet['type'] === 'welcome'));
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
        rejoinToken: viewer.rejoinToken }));
      const failureDatabase = new Database(dbPath);
      try {
        failureDatabase.exec(`CREATE TRIGGER reject_new_run_activation
          BEFORE INSERT ON rust_active_run_v1 BEGIN
            SELECT RAISE(ABORT, 'injected actual New Run transaction rejection');
          END`);
      } finally { failureDatabase.close(); }
      const before = await snapshot();
      const beforeHealth = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as {
        runId: string; worldEpoch: string; seed: number;
      };
      viewer.socket.send(JSON.stringify({ type: 'newRun', requestId: 'rejected-new-run' }));
      await until(viewer, () => viewer!.packets.some(packet => packet['requestId'] === 'rejected-new-run'));
      expect(viewer.packets.find(packet => packet['requestId'] === 'rejected-new-run')).toMatchObject({
        type: 'newRunResult', applied: false, reason: expect.stringContaining('injected actual New Run transaction rejection')
      });
      expect(cleanup).toHaveBeenCalledOnce();
      expect(await cleanup.mock.results[0]!.value).toMatchObject({ completed: true,
        deletedCheckpointCount: '0000000000000001', deletedHallOfFameCount: '0000000000000000' });
      expect(await snapshot()).toEqual(before);
      expect(await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json()).toMatchObject({
        ok: true, runId: beforeHealth.runId, worldEpoch: beforeHealth.worldEpoch, seed: beforeHealth.seed
      });
      const recoveryDatabase = new Database(dbPath);
      try { recoveryDatabase.exec('DROP TRIGGER reject_new_run_activation'); }
      finally { recoveryDatabase.close(); }
      viewer.socket.send(JSON.stringify({ type: 'newRun', requestId: 'new-run-valid-retry' }));
      await until(viewer, () => viewer!.packets.some(packet => packet['requestId'] === 'new-run-valid-retry'));
      expect(viewer.packets.find(packet => packet['requestId'] === 'new-run-valid-retry')).toMatchObject({
        type: 'newRunResult', applied: true
      });
      expect(cleanup).toHaveBeenCalledOnce();
    } finally {
      cleanup.mockRestore();
      viewer?.socket.close();
      await server.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 20_000);

  it('keeps a committed New Run when its SQLite reply is lost', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-lost-new-run-reply-'));
    const dbPath = join(root, 'experiment.sqlite');
    let server = await startRustServer({ ...DEFAULT_CONFIG, port: 0,
      resume: 'fresh', seed: 42, dbPath });
    let viewer: Peer | undefined;
    let commitSpy: ReturnType<typeof vi.spyOn> | undefined;
    try {
      viewer = await connect(server.port, 'ui');
      await until(viewer, () => viewer!.packets.some(packet => packet['type'] === 'welcome'));
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
        rejoinToken: viewer.rejoinToken }));
      const before = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as {
        runId: string; startupCheckpointId: string;
      };
      const originalCommit = CheckpointPersistenceClient.prototype.commit;
      /** Lose one reply after the real worker has activated the new run pointer. */
      commitSpy = vi.spyOn(CheckpointPersistenceClient.prototype, 'commit')
        .mockImplementationOnce(async function (this: CheckpointPersistenceClient,
          ...args: Parameters<CheckpointPersistenceClient['commit']>) {
          await originalCommit.apply(this, args);
          throw new Error('injected lost New Run commit reply');
        });
      viewer.socket.send(JSON.stringify({ type: 'newRun', requestId: 'lost-new-run-reply' }));
      await until(viewer, () => viewer!.packets.some(packet =>
        packet['type'] === 'error' || packet['type'] === 'newRunResult'));
      const faulted = await healthUntil(server.port, health => health['ok'] === false);
      expect(faulted).toMatchObject({ runId: before.runId,
        startupCheckpointId: before.startupCheckpointId,
        interfaceFault: 'fresh replacement checkpoint outcome is unknown; restart from a valid retained checkpoint' });
      await until(viewer, () => viewer!.packets.some(packet => packet['type'] === 'error'));
      expect(viewer.packets.filter(packet => packet['requestId'] === 'lost-new-run-reply')).toEqual([]);
      const database = new Database(dbPath, { readonly: true });
      let committedRunId: string;
      try {
        const current = database.prepare('SELECT run_id FROM rust_active_run_v1 WHERE singleton = 1')
          .get() as { run_id: string };
        committedRunId = current.run_id;
        expect(committedRunId).not.toBe(before.runId);
      } finally { database.close(); }
      viewer.socket.terminate();
      viewer = undefined;
      await server.close();
      commitSpy.mockRestore();
      commitSpy = undefined;
      const { seed: _seed, ...resumeConfig } = DEFAULT_CONFIG;
      server = await startRustServer({ ...resumeConfig, port: 0, resume: 'latest', dbPath });
      expect(await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json()).toMatchObject({
        ok: true, runId: committedRunId, generation: '0000000000000001'
      });
    } finally {
      commitSpy?.mockRestore();
      viewer?.socket.terminate();
      await server.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);

  it('recovers the committed import across both sides of the Rust swap', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-import-swap-fault-'));
    const source = await startRustServer({ ...DEFAULT_CONFIG, port: 0,
      resume: 'fresh', seed: 41, dbPath: join(root, 'source.sqlite') });
    try {
      const sourceHealth = await (await fetch(`http://127.0.0.1:${source.port}/api/health`)).json() as {
        runId: string; startupCheckpointId: string;
      };
      const archive = Buffer.from(await (await fetch(
        `http://127.0.0.1:${source.port}/api/export/latest`
      )).arrayBuffer());
      for (const phase of ['before-swap', 'after-swap'] as const) {
        const dbPath = join(root, `${phase}.sqlite`);
        let target: Awaited<ReturnType<typeof startRustServer>> | undefined;
        let publishSpy: ReturnType<typeof vi.spyOn> | undefined;
        try {
          target = await startRustServer({ ...DEFAULT_CONFIG, port: 0,
            resume: 'fresh', seed: 42, dbPath });
          const oldHealth = await (await fetch(`http://127.0.0.1:${target.port}/api/health`)).json() as {
            runId: string; startupCheckpointId: string;
          };
          const originalPublish = BackgroundOutputPump.prototype.publishPreparedImport;
          /** Fail once immediately before or after the real native publication. */
          publishSpy = vi.spyOn(BackgroundOutputPump.prototype, 'publishPreparedImport')
            .mockImplementationOnce(async function (this: BackgroundOutputPump,
              ...args: Parameters<BackgroundOutputPump['publishPreparedImport']>) {
              if (phase === 'after-swap') await originalPublish.apply(this, args);
              throw new Error(`injected import ${phase} failure`);
            });
          const response = await fetch(`http://127.0.0.1:${target.port}/api/import/archive`, {
            method: 'POST', headers: { 'Content-Type': 'application/vnd.slither-neuroevo.save' }, body: archive
          });
          expect(response.status).toBe(503);
          expect(await response.json()).toMatchObject({ ok: false,
            message: `injected import ${phase} failure` });
          const faulted = await healthUntil(target.port, health => health['ok'] === false);
          expect(faulted).toMatchObject({
            runId: oldHealth.runId, startupCheckpointId: oldHealth.startupCheckpointId,
            interfaceFault: `injected import ${phase} failure`
          });
          expect(['stopRequested', 'stopped']).toContain(faulted['lifecycle']);
          const stopped = await healthUntil(target.port, health => health['lifecycle'] === 'stopped');
          await new Promise<void>(done => setTimeout(done, 50));
          expect(await (await fetch(`http://127.0.0.1:${target.port}/api/health`)).json()).toMatchObject({
            completedStep: stopped['completedStep']
          });
          const database = new Database(dbPath, { readonly: true });
          try {
            expect(database.prepare('SELECT run_id FROM rust_active_run_v1 WHERE singleton = 1').get())
              .toEqual({ run_id: sourceHealth.runId });
          } finally { database.close(); }
          await target.close();
          target = undefined;
          const { seed: _seed, ...resumeConfig } = DEFAULT_CONFIG;
          target = await startRustServer({ ...resumeConfig, port: 0, resume: 'latest', dbPath });
          expect(await (await fetch(`http://127.0.0.1:${target.port}/api/health`)).json()).toMatchObject({
            ok: true, runId: sourceHealth.runId, startupCheckpointId: sourceHealth.startupCheckpointId
          });
        } finally {
          publishSpy?.mockRestore();
          await target?.close();
        }
      }
    } finally {
      await source.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);

  it('imports evolved Hall-of-Fame objects and reuses them on an exact retry', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-import-evolved-hof-'));
    const sourcePath = join(root, 'source.sqlite');
    const targetPath = join(root, 'target.sqlite');
    const source = await startRustServer({ ...DEFAULT_CONFIG, port: 0,
      resume: 'fresh', seed: 41, dbPath: sourcePath });
    let target: Awaited<ReturnType<typeof startRustServer>> | undefined;
    let viewer: Peer | undefined;
    let targetViewer: Peer | undefined;
    let cleanupSpy: ReturnType<typeof vi.spyOn> | undefined;
    let stageSpy: ReturnType<typeof vi.spyOn> | undefined;
    let releaseSpy: ReturnType<typeof vi.spyOn> | undefined;
    try {
      let initialExportReleased = false;
      const releaseExportLease = CheckpointPersistenceClient.prototype.releaseExportLease;
      // Download completion precedes lease release; Reset must wait for the real release reply.
      releaseSpy = vi.spyOn(CheckpointPersistenceClient.prototype, 'releaseExportLease')
        .mockImplementationOnce(async function(this: CheckpointPersistenceClient, operationId) {
          await releaseExportLease.call(this, operationId);
          initialExportReleased = true;
        });
      const initialExport = await fetch(`http://127.0.0.1:${source.port}/api/export/latest`);
      expect(initialExport.status).toBe(200);
      const initialCheckpointId = initialExport.headers.get('x-slither-checkpoint-id');
      const initialArchive = Buffer.from(await initialExport.arrayBuffer());
      viewer = await connect(source.port, 'ui');
      await until(viewer, () => viewer!.packets.some(packet => packet['type'] === 'welcome'));
      await until(viewer, () => initialExportReleased);
      releaseSpy.mockRestore();
      releaseSpy = undefined;
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
        rejoinToken: viewer.rejoinToken }));
      viewer.socket.send(JSON.stringify({ type: 'reset', settings: { snakeCount: 12, simSpeed: 12 },
        updates: [{ path: 'generationSeconds', value: 8 }, { path: 'baselineBots.count', value: 0 }] }));
      await until(viewer, () => viewer!.packets.some(packet =>
        packet['type'] === 'stateReplaced' && packet['reason'] === 'reset'));
      await healthUntil(source.port, health => BigInt(`0x${health['generation'] as string}`) >= 2n);
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
        rejoinToken: viewer.rejoinToken }));
      viewer.socket.send(JSON.stringify({ type: 'settings', requestId: 'hold-hof-fixture',
        updates: [{ path: 'simSpeed', value: 0.1 }] }));
      await until(viewer, () => viewer!.packets.some(packet =>
        packet['type'] === 'settingsApplied' && packet['requestId'] === 'hold-hof-fixture'));
      const exported = await fetch(`http://127.0.0.1:${source.port}/api/export/latest`);
      expect(exported.status).toBe(200);
      const checkpointId = exported.headers.get('x-slither-checkpoint-id');
      const archive = Buffer.from(await exported.arrayBuffer());
      const sourceDatabase = new Database(sourcePath, { readonly: true });
      let expectedElites: Array<{ relative_filename: string }>;
      try {
        expectedElites = sourceDatabase.prepare(`SELECT DISTINCT weights.relative_filename
          FROM rust_hall_of_fame_weights_v1 AS weights
          JOIN rust_hall_of_fame_v1 AS hall ON hall.weights_sha256 = weights.logical_sha256`).all() as
          Array<{ relative_filename: string }>;
      } finally { sourceDatabase.close(); }
      expect(expectedElites.length).toBeGreaterThan(0);
      target = await startRustServer({ ...DEFAULT_CONFIG, port: 0,
        resume: 'fresh', seed: 42, dbPath: targetPath });
      const managedDirectory = `${targetPath}.checkpoints`;
      /** Capture every durable metadata table and managed file in this small fixture. */
      const snapshot = async (): Promise<unknown> => {
        const database = new Database(targetPath, { readonly: true });
        try {
          const tables = database.prepare(`SELECT name FROM sqlite_master
            WHERE type = 'table' AND name LIKE 'rust_%' ORDER BY name`).all() as Array<{ name: string }>;
          const names = (await readdir(managedDirectory)).sort();
          return { tables: tables.map(({ name }) => ({ name,
            rows: database.prepare(`SELECT * FROM "${name.replaceAll('"', '""')}" ORDER BY rowid`).all() })),
          files: await Promise.all(names.map(async name => ({ name,
            hash: createHash('sha256').update(await readFile(join(managedDirectory, name))).digest('hex') }))) };
        } finally { database.close(); }
      };
      const failureDatabase = new Database(targetPath);
      try {
        failureDatabase.exec(`CREATE TRIGGER reject_import_activation
          BEFORE INSERT ON rust_active_run_v1 BEGIN
            SELECT RAISE(ABORT, 'injected actual import transaction rejection');
          END`);
      } finally { failureDatabase.close(); }
      const beforeFailure = await snapshot();
      const previousAuthority = await (await fetch(`http://127.0.0.1:${target.port}/api/health`)).json() as {
        runId: string; worldEpoch: string; generation: string;
      };
      const originalCleanup = CheckpointPersistenceClient.prototype.reclaimManagedOrphans;
      /** Observe real permanent files and a held world across the actual reference scan. */
      cleanupSpy = vi.spyOn(CheckpointPersistenceClient.prototype, 'reclaimManagedOrphans')
        .mockImplementationOnce(async function(this: CheckpointPersistenceClient) {
          const files = await readdir(managedDirectory);
          expect(files).toEqual(expect.arrayContaining([`${checkpointId}.checkpoint-v3`,
            ...expectedElites.map(elite => elite.relative_filename)]));
          const held = await (await fetch(`http://127.0.0.1:${target!.port}/api/health`)).json() as {
            completedStep: string; worldEpoch: string;
          };
          const result = await originalCleanup.call(this);
          expect(result).toMatchObject({ completed: true, deletedCheckpointCount: '0000000000000001' });
          expect(BigInt(`0x${result.deletedHallOfFameCount}`)).toBe(BigInt(expectedElites.length));
          expect(await (await fetch(`http://127.0.0.1:${target!.port}/api/health`)).json()).toMatchObject({
            completedStep: held.completedStep, worldEpoch: held.worldEpoch
          });
          return result;
        });
      const transactionRejected = await fetch(`http://127.0.0.1:${target.port}/api/import/archive`, {
        method: 'POST', headers: { 'Content-Type': 'application/vnd.slither-neuroevo.save' }, body: archive
      });
      expect(transactionRejected.status).toBe(400);
      expect(await transactionRejected.json()).toMatchObject({ ok: false,
        message: expect.stringContaining('injected actual import transaction rejection') });
      expect(cleanupSpy).toHaveBeenCalledOnce();
      cleanupSpy.mockRestore();
      expect(await snapshot()).toEqual(beforeFailure);
      expect(await (await fetch(`http://127.0.0.1:${target.port}/api/health`)).json()).toMatchObject({
        ok: true, runId: previousAuthority.runId, worldEpoch: previousAuthority.worldEpoch,
        generation: previousAuthority.generation
      });
      const recoveryDatabase = new Database(targetPath);
      try { recoveryDatabase.exec('DROP TRIGGER reject_import_activation'); }
      finally { recoveryDatabase.close(); }
      for (let attempt = 0; attempt < 2; attempt++) {
        const imported = await fetch(`http://127.0.0.1:${target.port}/api/import/archive`, {
          method: 'POST', headers: { 'Content-Type': 'application/vnd.slither-neuroevo.save' }, body: archive
        });
        expect(imported.status).toBe(200);
        expect(await imported.json()).toMatchObject({ ok: true, checkpointId });
        targetViewer ??= await connect(target.port, 'ui');
        targetViewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
          rejoinToken: targetViewer.rejoinToken }));
        const requestId = `hold-imported-hof-${attempt}`;
        targetViewer.socket.send(JSON.stringify({ type: 'settings', requestId,
          updates: [{ path: 'simSpeed', value: 0.1 }] }));
        await until(targetViewer, () => targetViewer!.packets.some(packet =>
          packet['type'] === 'settingsApplied' && packet['requestId'] === requestId));
        for (const elite of expectedElites) {
          expect(await readFile(join(`${targetPath}.checkpoints`, elite.relative_filename)))
            .toEqual(await readFile(join(`${sourcePath}.checkpoints`, elite.relative_filename)));
        }
        const names = await readdir(`${targetPath}.checkpoints`);
        expect(names.filter(name => name.includes('import-') || name.endsWith('.partial'))).toEqual([]);
      }
      const targetDatabase = new Database(targetPath, { readonly: true });
      try {
        expect(targetDatabase.prepare('SELECT count(*) AS count FROM rust_hall_of_fame_v1').get())
          .toEqual({ count: expectedElites.length });
      } finally { targetDatabase.close(); }
      let retryExportReleased = false;
      // Observe the actual cleanup reply before hashing immutable files; body
      // delivery alone does not join deletion of the export inventory.
      releaseSpy = vi.spyOn(CheckpointPersistenceClient.prototype, 'releaseExportLease')
        .mockImplementationOnce(async function(this: CheckpointPersistenceClient, operationId) {
          await releaseExportLease.call(this, operationId);
          retryExportReleased = true;
        });
      const retryExport = await fetch(`http://127.0.0.1:${target.port}/api/export/latest`);
      expect(retryExport.status).toBe(200);
      expect(retryExport.headers.get('x-slither-checkpoint-id')).toBe(checkpointId);
      expect(Buffer.from(await retryExport.arrayBuffer())).toEqual(archive);
      await until(targetViewer!, () => retryExportReleased);
      releaseSpy.mockRestore();
      releaseSpy = undefined;

      const winnersResponse = await fetch(`http://127.0.0.1:${target.port}/api/hof`);
      expect(winnersResponse.status).toBe(200);
      const winners = await winnersResponse.json() as { hof: Array<{ entryId: string }> };
      expect(winners.hof.length).toBe(expectedElites.length);
      const beforeResurrection = await snapshot();
      const resurrectedResponse = await fetch(`http://127.0.0.1:${target.port}/api/resurrect`, {
        method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ entryId: winners.hof[0]!.entryId })
      });
      expect(resurrectedResponse.status).toBe(200);
      const resurrected = await resurrectedResponse.json() as { ok: boolean; snakeId: number };
      expect(resurrected).toMatchObject({ ok: true, snakeId: expect.any(Number) });
      await until(targetViewer!, () => frameDirection(targetViewer!.latestFrame, resurrected.snakeId) !== undefined);
      expect(await snapshot()).toEqual(beforeResurrection);

      // A same-name corrupt elite must be preserved as evidence and reject the
      // import before any new inventory or immutable object becomes permanent.
      const elitePath = join(managedDirectory, expectedElites[0]!.relative_filename);
      const originalElite = await readFile(elitePath);
      const damaged = Buffer.from(originalElite);
      damaged[0] = damaged[0]! ^ 1;
      await writeFile(elitePath, damaged);
      const before = await snapshot();
      const beforeHealth = await (await fetch(`http://127.0.0.1:${target.port}/api/health`)).json() as {
        runId: string; worldEpoch: string; generation: string;
      };
      const rejected = await fetch(`http://127.0.0.1:${target.port}/api/import/archive`, {
        method: 'POST', headers: { 'Content-Type': 'application/vnd.slither-neuroevo.save' }, body: archive
      });
      expect(rejected.status).toBe(400);
      expect(await rejected.json()).toMatchObject({ ok: false,
        message: expect.stringContaining('existing immutable object differs') });
      expect(await snapshot()).toEqual(before);
      expect(await (await fetch(`http://127.0.0.1:${target.port}/api/health`)).json()).toMatchObject({
        ok: true, runId: beforeHealth.runId, worldEpoch: beforeHealth.worldEpoch, generation: beforeHealth.generation
      });
      await writeFile(elitePath, originalElite);
      const recovered = await fetch(`http://127.0.0.1:${target.port}/api/export/latest`);
      expect(recovered.status).toBe(200);
      expect(Buffer.from(await recovered.arrayBuffer())).toEqual(archive);
      const exportCleanupDeadline = performance.now() + 2000;
      let exportScratch: string[];
      do {
        exportScratch = (await readdir(managedDirectory)).filter(name =>
          name.endsWith('.slither-save.ready') || name.endsWith('.export-inventory-v1'));
        if (!exportScratch.length) break;
        await new Promise<void>(done => setTimeout(done, 10));
      } while (performance.now() < exportCleanupDeadline);
      expect(exportScratch!).toEqual([]);
      // Settle the preceding corruption case's deferred scan independently,
      // so the next rejection must schedule its own cleanup.
      const beforeSettlement = await (await fetch(`http://127.0.0.1:${target.port}/api/health`)).json() as {
        generation: string;
      };
      cleanupSpy = vi.spyOn(CheckpointPersistenceClient.prototype, 'reclaimManagedOrphans');
      targetViewer!.socket.send(JSON.stringify({ type: 'settings', requestId: 'settle-corruption-cleanup',
        updates: [{ path: 'simSpeed', value: 12 }] }));
      await healthUntil(target.port, health =>
        BigInt(`0x${health['generation'] as string}`) > BigInt(`0x${beforeSettlement.generation}`));
      expect(cleanupSpy).toHaveBeenCalledOnce();
      expect(await cleanupSpy.mock.results[0]!.value).toMatchObject({ completed: true });
      cleanupSpy.mockRestore();
      targetViewer!.socket.send(JSON.stringify({ type: 'settings', requestId: 'hold-deferred-cleanup-fixture',
        updates: [{ path: 'simSpeed', value: 0.1 }] }));
      await until(targetViewer!, () => targetViewer!.packets.some(packet =>
        packet['type'] === 'settingsApplied' && packet['requestId'] === 'hold-deferred-cleanup-fixture'));

      // Preparation has produced permanent files, but a busy/rejected stage
      // still leaves the old game running. Reclaim at its next durable boundary.
      stageSpy = vi.spyOn(BackgroundOutputPump.prototype, 'stagePreparedImport')
        .mockRejectedValueOnce(new Error('injected pre-stage rejection'));
      const unstaged = await fetch(`http://127.0.0.1:${target.port}/api/import/archive`, {
        method: 'POST', headers: { 'Content-Type': 'application/vnd.slither-neuroevo.save' }, body: initialArchive
      });
      expect(unstaged.status).toBe(400);
      expect(await unstaged.json()).toMatchObject({ ok: false, message: 'injected pre-stage rejection' });
      stageSpy.mockRestore();
      const deferredFile = join(managedDirectory, `${initialCheckpointId}.checkpoint-v3`);
      expect((await stat(deferredFile)).isFile()).toBe(true);
      const unchanged = await (await fetch(`http://127.0.0.1:${target.port}/api/health`)).json() as {
        runId: string; generation: string;
      };
      expect(unchanged).toMatchObject({ ok: true, runId: beforeHealth.runId });
      cleanupSpy = vi.spyOn(CheckpointPersistenceClient.prototype, 'reclaimManagedOrphans');
      targetViewer!.socket.send(JSON.stringify({ type: 'settings', requestId: 'reach-orphan-cleanup-boundary',
        updates: [{ path: 'simSpeed', value: 12 }] }));
      await healthUntil(target.port, health =>
        BigInt(`0x${health['generation'] as string}`) > BigInt(`0x${unchanged.generation}`));
      expect(cleanupSpy).toHaveBeenCalledOnce();
      expect(await readdir(managedDirectory)).not.toContain(`${initialCheckpointId}.checkpoint-v3`);
      cleanupSpy.mockRestore();
      const validRetry = await fetch(`http://127.0.0.1:${target.port}/api/import/archive`, {
        method: 'POST', headers: { 'Content-Type': 'application/vnd.slither-neuroevo.save' }, body: initialArchive
      });
      expect(validRetry.status).toBe(200);
      expect(await validRetry.json()).toMatchObject({ ok: true, checkpointId: initialCheckpointId });
    } finally {
      releaseSpy?.mockRestore();
      stageSpy?.mockRestore();
      cleanupSpy?.mockRestore();
      targetViewer?.socket.close();
      viewer?.socket.close();
      await target?.close();
      await source.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);

  it('keeps a later-generation Rust world ready after rejecting its older exact import', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-later-import-reject-'));
    const dbPath = join(root, 'experiment.sqlite');
    const server = await startRustServer({ ...DEFAULT_CONFIG, port: 0,
      resume: 'fresh', seed: 42, dbPath });
    let viewer: Peer | undefined;
    try {
      viewer = await connect(server.port, 'ui');
      await until(viewer, () => viewer!.packets.some(packet => packet['type'] === 'welcome'));
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
        rejoinToken: viewer.rejoinToken }));
      viewer.socket.send(JSON.stringify({ type: 'reset', settings: { snakeCount: 12, simSpeed: 1 },
        updates: [{ path: 'generationSeconds', value: 8 }, { path: 'baselineBots.count', value: 0 }] }));
      await until(viewer, () => viewer!.packets.some(packet =>
        packet['type'] === 'stateReplaced' && packet['reason'] === 'reset'));
      const resetHealth = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as {
        runId: string; startupCheckpointId: string;
      };
      const archived = await fetch(`http://127.0.0.1:${server.port}/api/export/latest`);
      expect(archived.status).toBe(200);
      expect(archived.headers.get('x-slither-checkpoint-id')).toBe(resetHealth.startupCheckpointId);
      const archive = Buffer.from(await archived.arrayBuffer());
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
        rejoinToken: viewer.rejoinToken }));
      viewer.socket.send(JSON.stringify({ type: 'settings', requestId: 'accelerate-generation',
        updates: [{ path: 'simSpeed', value: 12 }] }));
      await until(viewer, () => viewer!.packets.some(packet =>
        packet['type'] === 'settingsApplied' && packet['requestId'] === 'accelerate-generation'));
      expect(viewer.packets.findLast(packet => packet['requestId'] === 'accelerate-generation'))
        .toMatchObject({ applied: true });
      const database = new Database(dbPath, { readonly: true });
      let currentCheckpointId = resetHealth.startupCheckpointId;
      try {
        const deadline = performance.now() + 10_000;
        while (currentCheckpointId === resetHealth.startupCheckpointId && performance.now() < deadline) {
          currentCheckpointId = (database.prepare(`SELECT checkpoint_id FROM rust_checkpoint_v3_current
            WHERE run_id = ?`).get(resetHealth.runId) as { checkpoint_id: string }).checkpoint_id;
          if (currentCheckpointId === resetHealth.startupCheckpointId) {
            await new Promise<void>(done => setTimeout(done, 20));
          }
        }
        expect(currentCheckpointId).not.toBe(resetHealth.startupCheckpointId);
        viewer.socket.send(JSON.stringify({ type: 'settings', requestId: 'hold-later-generation',
          updates: [{ path: 'simSpeed', value: 0.1 }] }));
        await until(viewer, () => viewer!.packets.some(packet =>
          packet['type'] === 'settingsApplied' && packet['requestId'] === 'hold-later-generation'));
        expect(viewer.packets.findLast(packet => packet['requestId'] === 'hold-later-generation'))
          .toMatchObject({ applied: true });
        currentCheckpointId = (database.prepare(`SELECT checkpoint_id FROM rust_checkpoint_v3_current
          WHERE run_id = ?`).get(resetHealth.runId) as { checkpoint_id: string }).checkpoint_id;
        const rejected = await fetch(`http://127.0.0.1:${server.port}/api/import/archive`, {
          method: 'POST', headers: { 'Content-Type': 'application/vnd.slither-neuroevo.save' }, body: archive
        });
        expect(rejected.status).toBe(409);
        expect(await rejected.json()).toMatchObject({ ok: false, code: 'IMPORT_REQUIRES_BRANCH' });
        expect(await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json()).toMatchObject({
          ok: true, runId: resetHealth.runId, lifecycle: 'running'
        });
        expect((database.prepare(`SELECT checkpoint_id FROM rust_checkpoint_v3_current WHERE run_id = ?`)
          .get(resetHealth.runId) as { checkpoint_id: string }).checkpoint_id).toBe(currentCheckpointId);
      } finally { database.close(); }
    } finally {
      viewer?.socket.terminate();
      await server.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);

  it('resumes exact managed IDs and exposes recovery or health-only failure over real HTTP/WebSocket', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-recovery-server-'));
    const dbPath = join(root, 'experiment.sqlite');
    const config = { ...DEFAULT_CONFIG, port: 0, dbPath };
    let server = await startRustServer({ ...config, resume: 'fresh', seed: 42 });
    const peers: Peer[] = [];
    try {
      const initial = await (await fetch(`http://127.0.0.1:${server.port}/health`)).json() as { runId: string; startupCheckpointId: string };
      await server.close();
      const exact = normalizeConfig({ ...config, resume: initial.startupCheckpointId });
      expect(exact.resume).toBe(`sha256:${initial.startupCheckpointId}`);
      server = await startRustServer({ ...exact, port: 0 });
      expect(await (await fetch(`http://127.0.0.1:${server.port}/health`)).json()).toMatchObject({ ok: true, runId: initial.runId });
      await server.close();
      const db = new Database(dbPath);
      try {
        db.pragma('foreign_keys = OFF');
        db.prepare('UPDATE rust_checkpoint_v3_current SET checkpoint_id = ?').run('f'.repeat(64));
      } finally { db.close(); }
      server = await startRustServer({ ...config, resume: 'latest' });
      const health = await (await fetch(`http://127.0.0.1:${server.port}/health`)).json() as { runId: string; recovery: unknown };
      expect(health.runId).not.toBe(initial.runId);
      expect(health.recovery).toMatchObject({ failedRunId: initial.runId, branchRunId: health.runId,
        recoveredCheckpointId: initial.startupCheckpointId, lostCompletedGenerations: null });
      const viewer = await connect(server.port, 'ui'); peers.push(viewer);
      await until(viewer, () => viewer.packets.some(packet => packet['type'] === 'welcome'));
      expect(viewer.packets.find(packet => packet['type'] === 'welcome')).toMatchObject({ recovery: health.recovery });
      viewer.socket.terminate();
      await server.close();
      const files = await readdir(`${dbPath}.checkpoints`);
      for (const file of files.filter(name => name.endsWith('.checkpoint-v3'))) await writeFile(join(`${dbPath}.checkpoints`, file), 'corrupt');
      server = await startRustServer({ ...config, resume: 'latest' });
      expect(server.startupFault).toMatch(/no valid retained/);
      const fault = await fetch(`http://127.0.0.1:${server.port}/api/health`);
      expect(fault.status).toBe(503);
      expect(await fault.json()).toMatchObject({ ok: false, lifecycle: 'startup-fault' });
      const rejected = new WebSocket(`ws://127.0.0.1:${server.port}`);
      await new Promise<void>((done, reject) => {
        rejected.once('unexpected-response', (_request, response) => {
          expect(response.statusCode).toBe(503); response.resume(); rejected.terminate(); done();
        });
        rejected.once('open', () => reject(new Error('faulted startup accepted a game socket')));
        rejected.on('error', () => {});
      });
    } finally {
      for (const peer of peers) peer.socket.terminate();
      await server.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);

  it('durably resets Rust with every authoritative input generated by the browser settings panel', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-panel-reset-'));
    const server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, resume: 'fresh', seed: 42,
      dbPath: join(root, 'experiment.sqlite') });
    let viewer: Peer | undefined;
    try {
      viewer = await connect(server.port, 'ui');
      await until(viewer, () => viewer!.packets.some(packet => packet['type'] === 'welcome'));
      const original = viewer.packets.find(packet => packet['type'] === 'welcome') as unknown as WelcomeMsg;
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator' }));
      /** Capture real generated controls with only the DOM methods used by the panel builder. */
      const inputs: HTMLInputElement[] = [];
      vi.stubGlobal('document', {
        createElement() {
          const element = { dataset: {}, value: '', checked: false, appendChild() {} } as unknown as HTMLInputElement;
          inputs.push(element);
          return element;
        },
        getElementById() { return null; }
      });
      let updates: SettingsUpdate[];
      try {
        const panel = { appendChild() {}, querySelectorAll: () => inputs.filter(input => input.dataset['path']) } as unknown as HTMLElement;
        buildSettingsUI(panel);
        applyValuesToSlidersFromCFG(panel);
        updates = inputs.filter(input => input.dataset['path'] && !BROWSER_CAMERA_SETTING_PATHS.has(input.dataset['path']))
          .map(input => ({ path: input.dataset['path'] as SettingsUpdate['path'],
            value: input.type === 'checkbox' ? Number(input.checked) : Number(input.value) }));
      } finally { vi.unstubAllGlobals(); }
      expect(updates.length).toBeGreaterThan(60);
      const radius = updates.find(update => update.path === 'worldRadius')!;
      radius.value = 4200;
      const graphSpec = buildStackGraphSpec({ ...DEFAULT_CORE_SETTINGS, hiddenLayers: 1, neurons1: 8 }, CFG_DEFAULT);
      viewer.socket.send(JSON.stringify({ type: 'reset',
        settings: { ...DEFAULT_CORE_SETTINGS, snakeCount: 12, hiddenLayers: 1, neurons1: 8 }, updates, graphSpec }));
      const replaced = await replacementUntil(viewer, server.port, 'reset');
      expect(replaced.worldSeed).toBe(original.worldSeed);
      expect(replaced.runId).not.toBe(original.runId);
      expect(replaced.settings).toMatchObject({ core: { snakeCount: 12 },
        updates: expect.arrayContaining([{ path: 'worldRadius', value: 4200 }]) });
      const db = new Database(join(root, 'experiment.sqlite'), { readonly: true });
      try {
        expect(db.prepare('SELECT checkpoint_id FROM rust_checkpoint_v3_current WHERE run_id = ?')
          .get(replaced.runId)).toMatchObject({ checkpoint_id: expect.stringMatching(/^[0-9a-f]{64}$/u) });
      } finally { db.close(); }
      expect(viewer.packets.filter(packet => packet['type'] === 'error')).toEqual([]);
    } finally {
      viewer?.socket.terminate();
      await server.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 15_000);

  it('serves native welcome, frames, controller observations and same-snake token reclaim', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-server-'));
    const peers: Peer[] = [];
    const server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, resume: 'fresh', seed: 42,
      rustCalculationWorkers: 4, dbPath: join(root, 'experiment.sqlite') });
    try {
      const viewer = await connect(server.port, 'ui'); peers.push(viewer);
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
        rejoinToken: viewer.rejoinToken }));
      await until(viewer, () => viewer.frames > 0 && viewer.packets.some(packet => packet['type'] === 'stats'));
      expect(viewer.packets.find(packet => packet['type'] === 'welcome')).toMatchObject({ protocolVersion: 2, worldSeed: 42,
        sensorSpec: { sensorCount: 83 }, inferenceMode: { activeBackend: 'native', activeWorkerCount: 4 } });
      viewer.socket.send(JSON.stringify({ type: 'viz', enabled: true }));
      await until(viewer, () => viewer.packets.some(packet => {
        const viz = packet['viz'] as { layers?: unknown[] } | undefined;
        return packet['type'] === 'stats' && viz?.layers?.length === 5;
      }));
      expect(viewer.packets.findLast(packet => packet['viz'])).toMatchObject({
        viz: {
          kind: 'graph', snakeId: expect.any(Number),
          layers: [
            { count: 83, activations: null },
            { count: 64, activations: expect.any(Array) },
            { count: 64, activations: expect.any(Array) },
            { count: 16, activations: expect.any(Array), isRecurrent: true },
            { count: 2, activations: expect.any(Array) }
          ]
        }
      });
      viewer.socket.send(JSON.stringify({ type: 'viz', enabled: false }));
      const statsBeforeDisable = viewer.packets.filter(packet => packet['type'] === 'stats').length;
      await until(viewer, () => viewer.packets.filter(packet => packet['type'] === 'stats').length > statsBeforeDisable);
      expect(viewer.packets.findLast(packet => packet['type'] === 'stats')?.['viz']).toBeUndefined();
      const selected = firstFrameSnake(viewer.latestFrame);
      expect(selected).toBeDefined();
      if (!selected) throw new Error('native frame omitted every alive snake');
      viewer.socket.send(JSON.stringify({ type: 'godMode', requestId: 'native-god-move', action: 'move',
        snakeId: selected.snakeId, x: selected.x * 0.9, y: selected.y * 0.9 }));
      await until(viewer, () => viewer.packets.some(packet => packet['requestId'] === 'native-god-move'));
      expect(viewer.packets.findLast(packet => packet['requestId'] === 'native-god-move')).toMatchObject({
        type: 'godModeResult', action: 'move', snakeId: selected.snakeId, applied: true,
        sequence: expect.any(Number), step: expect.any(Number), x: expect.any(Number), y: expect.any(Number)
      });
      viewer.socket.send(JSON.stringify({ type: 'godMode', requestId: 'native-god-kill', action: 'kill',
        snakeId: selected.snakeId }));
      await until(viewer, () => viewer.packets.some(packet => packet['requestId'] === 'native-god-kill'));
      const killResult = viewer.packets.findLast(packet => packet['requestId'] === 'native-god-kill');
      expect(killResult).toMatchObject({
        type: 'godModeResult', action: 'kill', snakeId: selected.snakeId, applied: true,
        sequence: expect.any(Number), step: expect.any(Number), pelletsDropped: expect.any(Number)
      });
      expect(Number(killResult?.['pelletsDropped'])).toBeGreaterThan(0);
      await until(viewer, () => frameDirection(viewer.latestFrame, selected.snakeId) === undefined);
      viewer.socket.send(JSON.stringify({ type: 'godMode', requestId: 'native-god-missing', action: 'move',
        snakeId: selected.snakeId, x: 0, y: 0 }));
      await until(viewer, () => viewer.packets.some(packet => packet['requestId'] === 'native-god-missing'));
      expect(viewer.packets.findLast(packet => packet['requestId'] === 'native-god-missing')).toMatchObject({
        type: 'godModeResult', action: 'move', snakeId: selected.snakeId, applied: false,
        reason: expect.stringContaining('missing or already dead')
      });
      const bot = await connect(server.port, 'bot'); peers.push(bot);
      bot.socket.send(JSON.stringify({ type: 'join', rejoinToken: bot.rejoinToken, mode: 'player', name: 'socket-bot' }));
      await until(bot, () => bot.packets.some(packet => packet['type'] === 'sensors'));
      const assignment = bot.packets.find(packet => packet['type'] === 'assign')!;
      const sample = bot.packets.find(packet => packet['type'] === 'sensors')!;
      expect(sample['snakeId']).toBe(assignment['snakeId']);
      expect(sample['sensors']).toHaveLength(83);
      bot.socket.send(JSON.stringify({ type: 'action', snakeId: assignment['snakeId'], tick: sample['tick'], turn: 0.4, boost: 0 }));
      await until(bot, () => bot.packets.some(packet => packet['type'] === 'sensors' && Number(packet['tick']) > Number(sample['tick'])));
      await new Promise<void>(done => { bot.socket.once('close', () => done()); bot.socket.close(); });
      const resumed = await connect(server.port, 'bot'); peers.push(resumed);
      resumed.socket.send(JSON.stringify({ type: 'join', rejoinToken: resumed.rejoinToken, mode: 'player', name: 'socket-bot', resumeToken: assignment['resumeToken'] }));
      await until(resumed, () => resumed.packets.some(packet => packet['type'] === 'sensors'));
      expect(resumed.packets.find(packet => packet['type'] === 'reclaimResult')).toMatchObject({ reclaimed: true, snakeId: assignment['snakeId'] });
      const nextAssignment = resumed.packets.find(packet => packet['type'] === 'assign')!;
      expect(nextAssignment).toMatchObject({ reclaimed: true, snakeId: assignment['snakeId'] });
      expect(nextAssignment['resumeToken']).not.toBe(assignment['resumeToken']);
      /** Match the production latest-action pump so a runner-delayed sample cannot be the only attempt. */
      const actionPump = setInterval(() => {
        const latest = resumed.packets.findLast(packet => packet['type'] === 'sensors');
        if (latest && resumed.socket.readyState === WebSocket.OPEN) {
          resumed.socket.send(JSON.stringify({ type: 'action', snakeId: nextAssignment['snakeId'],
            tick: latest['tick'], turn: 0.4, boost: 0 }));
        }
      }, 10);
      let health: Record<string, unknown>;
      try {
        health = await healthUntil(server.port, value => {
          const telemetry = value['telemetry'] as { trainerAction?: { samples?: number } } | undefined;
          return (telemetry?.trainerAction?.samples ?? 0) > 0;
        });
      } finally {
        clearInterval(actionPump);
      }
      expect(health).toMatchObject({
        ok: true,
        authority: 'rust',
        calculationWorkers: 4,
        seed: 42,
        nativeQueues: {
          inbound: { maxBatches: '0000000000000040', maxCommands: '0000000000000040',
            maxOwnedBytes: '0000000000400000', maxBatchCommands: '0000000000000001',
            maxBatchOwnedBytes: '0000000000100000' },
          output: { maxReliable: '0000000000000020', maxDiscrete: '0000000000000004',
            maxOwnedBytes: '0000000002000000', maxFrames: '0000000000000004', hasReservedFault: false }
        },
        outbound: {
          connections: 2,
          replacedFrames: expect.any(Number),
          reliableFailures: expect.any(Number)
        },
        telemetry: {
          authoritativeSteps: expect.any(Number),
          simulatedWallRatio: expect.any(Number),
          step: { samples: expect.any(Number), p95Ms: expect.any(Number), p99Ms: expect.any(Number) },
          process: { rssBytes: expect.any(Number), eventLoopDelayP95Ms: expect.any(Number) },
          frame: { latestBytes: expect.any(Number), maximumObservedBytes: expect.any(Number) },
          trainerAction: { samples: expect.any(Number) },
          playerAction: { samples: 0 },
          controllerLifecycle: { samples: 2 },
          controllerActivity: {
            player: { freshAssignments: 0, successfulReclaims: 0, appliedActions: 0, appliedDisconnects: 0 },
            trainer: { freshAssignments: 1, successfulReclaims: 1, appliedActions: expect.any(Number),
              appliedDisconnects: expect.any(Number) }
          }
        },
        retention: {
          schemaVersion: 1,
          retained: { latest: { checkpointCount: 1 }, pinned: { checkpointCount: 0 } },
          plannedPrune: { checkpointCount: 0 }
        },
        retentionCleanup: { deletedCheckpointCount: 0, deletedStoredByteCount: '0000000000000000' },
        storage: {
          schemaVersion: 1,
          sqlite: {
            databaseBytes: expect.stringMatching(/^[1-9][0-9]*$/u),
            walBytes: expect.stringMatching(/^[0-9]+$/u),
            pageSizeBytes: expect.stringMatching(/^[1-9][0-9]*$/u),
            pageCount: expect.stringMatching(/^[1-9][0-9]*$/u),
            freelistPageCount: expect.stringMatching(/^[0-9]+$/u),
            usedPageBytes: expect.stringMatching(/^[1-9][0-9]*$/u)
          },
          managed: {
            temporaryBytes: expect.stringMatching(/^[0-9]+$/u),
            temporaryQuotaBytes: expect.stringMatching(/^[1-9][0-9]*$/u),
            freeBytes: expect.stringMatching(/^[1-9][0-9]*$/u),
            operatingReserveBytes: expect.stringMatching(/^[1-9][0-9]*$/u)
          }
        }
      });
      const queues = health['nativeQueues'] as RustQueueDiagnostics;
      for (const group of [queues.inbound, queues.output]) {
        for (const value of Object.values(group)) {
          if (typeof value === 'string') expect(value).toMatch(/^[0-9a-f]{16}$/u);
        }
      }
      expect(BigInt(`0x${queues.inbound.highWaterCommands}`)).toBeGreaterThan(0n);
      expect(BigInt(`0x${queues.inbound.highWaterCommands}`)).toBeLessThanOrEqual(BigInt(`0x${queues.inbound.maxCommands}`));
      expect(BigInt(`0x${queues.inbound.highWaterOwnedBytes}`)).toBeLessThanOrEqual(BigInt(`0x${queues.inbound.maxOwnedBytes}`));
      expect(BigInt(`0x${queues.output.highWaterCount}`)).toBeGreaterThan(0n);
      expect(BigInt(`0x${queues.output.highWaterCount}`)).toBeLessThanOrEqual(
        BigInt(`0x${queues.output.maxReliable}`) + BigInt(`0x${queues.output.maxDiscrete}`) + BigInt(`0x${queues.output.maxFrames}`) + 1n);
      expect(BigInt(`0x${queues.output.highWaterOwnedBytes}`)).toBeLessThanOrEqual(BigInt(`0x${queues.output.maxOwnedBytes}`));
      expect(queues.inbound.faultDiscardedCommands).toBe('0000000000000000');
      expect(queues.output.priorityOverflows).toBe('0000000000000000');
      const activity = (health['telemetry'] as {
        trainerAction: { samples: number };
        controllerActivity: { trainer: { appliedActions: number } };
      });
      expect(activity.trainerAction.samples).toBeGreaterThan(0);
      expect(activity.controllerActivity.trainer.appliedActions).toBeGreaterThan(0);
      const pinResponse = await fetch(`http://127.0.0.1:${server.port}/api/checkpoints/current/pin`, { method: 'POST' });
      expect(pinResponse.status).toBe(200);
      const pinned = await pinResponse.json() as Record<string, unknown>;
      expect(pinned).toMatchObject({ ok: true, checkpointId: expect.stringMatching(/^[0-9a-f]{64}$/u), generation: '0000000000000001' });
      const pinnedHealth = await healthUntil(server.port, value => {
        const retention = value['retention'] as { retained?: { pinned?: { checkpointCount?: number } } } | undefined;
        return retention?.retained?.pinned?.checkpointCount === 1;
      });
      expect(pinnedHealth).toMatchObject({ retention: { retained: { pinned: { checkpointCount: 1 } } } });
      const beforeReset = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as {
        runId: string; seed: number; startupCheckpointId: string;
      };
      const replacementGraph = {
        type: 'graph', nodes: [
          { id: 'input', type: 'Input', outputSize: 51 },
          { id: 'features', type: 'MLP', inputSize: 51, hiddenSizes: [8], outputSize: 8 },
          { id: 'memory', type: 'GRU', inputSize: 8, hiddenSize: 4 },
          { id: 'head', type: 'Dense', inputSize: 4, outputSize: 2 }
        ], edges: [
          { from: 'input', to: 'features' }, { from: 'features', to: 'memory' },
          { from: 'memory', to: 'head' }
        ], outputs: [{ nodeId: 'head' }], outputSize: 2
      };
      const savePresetResponse = await fetch(`http://127.0.0.1:${server.port}/api/graph-presets`, {
        method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ name: 'Compact memory', spec: replacementGraph })
      });
      expect(savePresetResponse.status).toBe(200);
      const savedPreset = await savePresetResponse.json() as { presetId: number };
      expect(savedPreset.presetId).toBeGreaterThan(0);
      expect(await (await fetch(`http://127.0.0.1:${server.port}/api/graph-presets`)).json()).toMatchObject({
        ok: true, presets: [{ id: savedPreset.presetId, name: 'Compact memory', createdAt: expect.any(Number) }]
      });
      expect(await (await fetch(
        `http://127.0.0.1:${server.port}/api/graph-presets/${savedPreset.presetId}`
      )).json()).toMatchObject({
        ok: true, preset: { id: savedPreset.presetId, name: 'Compact memory', spec: replacementGraph }
      });
      viewer.socket.send(JSON.stringify({ type: 'reset', settings: { snakeCount: 12, simSpeed: 2 }, updates: [
        { path: 'worldRadius', value: 4_200 },
        { path: 'generationSeconds', value: 90 },
        { path: 'sense.bubbleBins', value: 8 },
        { path: 'baselineBots.count', value: 3 },
        { path: 'foodSpawn.edgeFalloffEnabled', value: 0 }
      ], graphSpec: replacementGraph }));
      await until(viewer, () => viewer.packets.some(packet => packet['type'] === 'stateReplaced' && packet['reason'] === 'reset'));
      const resetNotice = viewer.packets.findLast(packet => packet['type'] === 'stateReplaced');
      expect(resetNotice).toMatchObject({ reason: 'reset', welcome: { worldSeed: 42,
        graphSpec: replacementGraph, inferenceMode: { parameterCount: 654 }, settings: {
        core: { snakeCount: 12, simSpeed: 2 }, updates: expect.arrayContaining([
          { path: 'worldRadius', value: 4_200 },
          { path: 'generationSeconds', value: 90 },
          { path: 'sense.bubbleBins', value: 8 },
          { path: 'baselineBots.count', value: 3 },
          { path: 'foodSpawn.edgeFalloffEnabled', value: 0 }
        ])
      } } });
      const afterReset = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as {
        runId: string; seed: number; startupCheckpointId: string;
      };
      expect(afterReset).toMatchObject({ seed: beforeReset.seed, calculationWorkers: 4 });
      expect(afterReset.runId).not.toBe(beforeReset.runId);
      expect(afterReset.startupCheckpointId).not.toBe(beforeReset.startupCheckpointId);

      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
        rejoinToken: viewer.rejoinToken }));
      viewer.socket.send(JSON.stringify({
        type: 'settings', requestId: 'native-settings-rejected',
        updates: [{ path: 'snakeCount', value: 40 }]
      }));
      await until(viewer, () => viewer.packets.some(packet =>
        packet['type'] === 'settingsApplied' && packet['requestId'] === 'native-settings-rejected'));
      expect(viewer.packets.findLast(packet => packet['requestId'] === 'native-settings-rejected'))
        .toMatchObject({ applied: false, updates: [], reason: expect.stringContaining('requires reset') });
      viewer.socket.send(JSON.stringify({
        type: 'settings', requestId: 'native-settings-applied', updates: [
          { path: 'simSpeed', value: 2 },
          { path: 'reward.pointsPerKill', value: 275 },
          { path: 'foodSpawn.edgeFalloffEnabled', value: 0 }
        ]
      }));
      await until(viewer, () => viewer.packets.some(packet =>
        packet['type'] === 'settingsApplied' && packet['requestId'] === 'native-settings-applied'));
      const settingsApplied = viewer.packets.findLast(packet => packet['requestId'] === 'native-settings-applied');
      expect(settingsApplied).toMatchObject({ applied: true, configRevision: 2,
        configHash: expect.stringMatching(/^sha256:[0-9a-f]{64}$/u), sequence: expect.any(Number),
        step: expect.any(Number), updates: [
          { path: 'simSpeed', value: 2 },
          { path: 'reward.pointsPerKill', value: 275 },
          { path: 'foodSpawn.edgeFalloffEnabled', value: 0 }
        ] });
      expect(await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json()).toMatchObject({
        configRevision: 2, configHash: settingsApplied?.['configHash']
      });
      viewer.socket.send(JSON.stringify({ type: 'newRun', requestId: 'native-new-run' }));
      await until(viewer, () => viewer.packets.some(packet =>
        packet['type'] === 'stateReplaced' && packet['reason'] === 'newRun') &&
        viewer.packets.some(packet => packet['type'] === 'newRunResult' && packet['requestId'] === 'native-new-run'));
      const newRunResult = viewer.packets.findLast(packet => packet['type'] === 'newRunResult');
      expect(newRunResult).toMatchObject({ requestId: 'native-new-run', applied: true });
      const afterNewRun = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as {
        runId: string; seed: number; startupCheckpointId: string; configRevision: number; configHash: string;
      };
      expect(afterNewRun).toMatchObject({ runId: newRunResult?.['runId'], seed: newRunResult?.['worldSeed'],
        calculationWorkers: 4 });
      expect(afterNewRun.runId).not.toBe(afterReset.runId);
      expect(afterNewRun).toMatchObject({ configRevision: 1, configHash: settingsApplied?.['configHash'] });
      const newRunNotice = viewer.packets.findLast(packet =>
        packet['type'] === 'stateReplaced' && packet['reason'] === 'newRun');
      expect(newRunNotice).toMatchObject({ welcome: { configHash: settingsApplied?.['configHash'],
        graphSpec: replacementGraph, inferenceMode: { parameterCount: 654 },
        settings: { core: { snakeCount: 12, simSpeed: 2 }, updates: expect.arrayContaining([
          { path: 'baselineBots.count', value: 3 }
        ]) } } });
    } finally {
      for (const peer of peers) peer.socket.terminate();
      await server.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);

  it('keeps same-snake reclaim distinct from fresh joins after generation changes and failed successor sends', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-reclaim-generation-'));
    const server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, resume: 'fresh', seed: 91,
      rustCalculationWorkers: 4, dbPath: join(root, 'experiment.sqlite') });
    const peers: Peer[] = [];
    let sendSpy: ReturnType<typeof vi.spyOn> | undefined;
    try {
      const viewer = await connect(server.port, 'ui'); peers.push(viewer);
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
        rejoinToken: viewer.rejoinToken }));
      viewer.socket.send(JSON.stringify({ type: 'reset', settings: { snakeCount: 3, simSpeed: 0.1 },
        updates: [{ path: 'generationSeconds', value: 8 }, { path: 'pelletCountTarget', value: 100 },
          { path: 'baselineBots.count', value: 0 }],
        graphSpec: buildStackGraphSpec({ hiddenLayers: 1, neurons1: 2, neurons2: 2,
          neurons3: 2, neurons4: 2, neurons5: 2 },
        { brain: { inSize: 83, outSize: 2, useMlp: false } }) }));
      await until(viewer, () => viewer.packets.some(packet => packet['type'] === 'stateReplaced'));
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
        rejoinToken: viewer.rejoinToken }));
      /** Drive the exact measurement exchange through real result/assignment packets. */
      const claim = async (prior?: { snakeId: number; token: string }): Promise<{ peer: Peer; exchange: PlayerReconnectExchange }> => {
        const peer = await connect(server.port, 'ui'); peers.push(peer);
        const exchange = new PlayerReconnectExchange(prior?.snakeId, prior?.token);
        peer.socket.on('message', (data, binary) => {
          if (binary || exchange.record.failure) return;
          try {
            const message = JSON.parse(data.toString()) as Record<string, unknown>;
            if (exchange.consume(message)) peer.socket.send(JSON.stringify({
              type: 'join', mode: 'player', name: 'GenerationReclaimProbe' }));
          } catch { /* The exchange retains its failed invariant for the bounded assertion below. */ }
        });
        peer.socket.send(JSON.stringify({ type: 'join', rejoinToken: peer.rejoinToken, mode: 'player', name: 'GenerationReclaimProbe',
          ...(prior ? { resumeToken: prior.token } : {}) }));
        await until(peer, () => exchange.ready || exchange.record.failure !== undefined);
        expect(exchange.record.failure).toBeUndefined();
        expect(exchange.ready).toBe(true);
        return { peer, exchange };
      };
      const first = await claim();
      expect(first.exchange.record.outcome).toBe('fresh');
      await new Promise<void>(done => { first.peer.socket.once('close', done); first.peer.socket.close(); });
      const original = first.peer.packets.findLast(packet => packet['type'] === 'assign')!;
      const same = await claim({ snakeId: Number(original['snakeId']), token: String(original['resumeToken']) });
      expect(same.exchange.record.outcome).toBe('sameSnakeReclaim');
      await new Promise<void>(done => { same.peer.socket.once('close', done); same.peer.socket.close(); });
      const before = same.peer.packets.findLast(packet => packet['type'] === 'assign')!;
      const initial = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as { generation: string };
      viewer.socket.send(JSON.stringify({ type: 'settings', requestId: 'advance-reclaim-generation',
        updates: [{ path: 'simSpeed', value: 12 }] }));
      await until(viewer, () => viewer.packets.some(packet =>
        packet['type'] === 'settingsApplied' && packet['requestId'] === 'advance-reclaim-generation'));
      expect(viewer.packets.findLast(packet => packet['requestId'] === 'advance-reclaim-generation'))
        .toMatchObject({ applied: true });
      await healthUntil(server.port, value => BigInt(`0x${String(value['generation'])}`) > BigInt(`0x${initial.generation}`));
      const replacement = await claim({ snakeId: Number(before['snakeId']), token: String(before['resumeToken']) });
      expect(replacement.exchange.record.outcome).toBe('freshAfterRejectedReclaim');
      expect(replacement.exchange.record.packets[0]).toMatchObject({ type: 'reclaimResult', reclaimed: false });
      expect(replacement.exchange.record.packets.at(-1)?.snakeId).not.toBe(Number(before['snakeId']));
      expect(replacement.exchange.record.freshJoinRequested).toBe(true);

      viewer.socket.send(JSON.stringify({ type: 'settings', requestId: 'hold-successor-send',
        updates: [{ path: 'simSpeed', value: 0.1 }] }));
      await until(viewer, () => viewer.packets.some(packet => packet['requestId'] === 'hold-successor-send'));
      expect(viewer.packets.findLast(packet => packet['requestId'] === 'hold-successor-send'))
        .toMatchObject({ applied: true });
      const held = replacement.peer.packets.findLast(packet => packet['type'] === 'assign')!;
      const heldGeneration = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as { generation: string };
      const originalSend = WsHub.prototype.sendJsonTo;
      let failedSuccessorId: number | undefined;
      // Reject exactly the first new-snake send at the real transport boundary.
      // Rust must publish its failed-send outcome; the old token cannot bind it.
      sendSpy = vi.spyOn(WsHub.prototype, 'sendJsonTo').mockImplementation(function (this: WsHub, connection, message) {
        if (message.type === 'assign' && message.reclaimed !== true &&
            message.snakeId !== Number(held['snakeId']) && failedSuccessorId === undefined) {
          failedSuccessorId = message.snakeId;
          return false;
        }
        return originalSend.call(this, connection, message);
      });
      viewer.socket.send(JSON.stringify({ type: 'settings', requestId: 'advance-successor-send',
        updates: [{ path: 'simSpeed', value: 12 }] }));
      await healthUntil(server.port, value => failedSuccessorId !== undefined &&
        BigInt(`0x${String(value['generation'])}`) > BigInt(`0x${heldGeneration.generation}`));
      sendSpy.mockRestore(); sendSpy = undefined;
      await new Promise<void>(done => { replacement.peer.socket.once('close', done); replacement.peer.socket.close(); });
      const recovered = await claim({ snakeId: Number(held['snakeId']), token: String(held['resumeToken']) });
      expect(recovered.exchange.record.outcome).toBe('freshAfterRejectedReclaim');
      expect(recovered.exchange.record.packets[0]).toMatchObject({ type: 'reclaimResult', reclaimed: false });
      expect(recovered.exchange.record.requestedSnakeId).not.toBe(failedSuccessorId);
    } finally {
      sendSpy?.mockRestore();
      for (const peer of peers) peer.socket.terminate();
      await server.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 15_000);

  it('correlates client steering reversals with real Rust observations for player and bot leases', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-lan-turn-'));
    const server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, resume: 'fresh', seed: 83,
      dbPath: join(root, 'experiment.sqlite') });
    try {
      for (const kind of ['ui', 'bot'] as const) {
        const report = await measureTurnResponses(`ws://127.0.0.1:${server.port}`, kind, 4);
        expect(report.failure).toBeUndefined();
        expect(report.welcome?.inferenceMode.activeBackend).toBe('native');
        expect(report.trials).toHaveLength(4);
        expect(report.trials.map(trial => trial.requestedTurn)).toEqual([-1, 1, -1, 1]);
        for (const trial of report.trials) {
          expect(trial.failure).toBeUndefined();
          expect(trial.observedTick).toBeGreaterThan(trial.clientTick);
          expect(trial.latencyUpperBoundMs).toBeGreaterThanOrEqual(0);
          expect(trial.latencyUpperBoundMs).toBeLessThan(500);
        }
        // The 100 ms performance limit is evaluated in retained target-platform
        // reports; this integration gate verifies actual control/observation correlation.
      }
    } finally {
      await server.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 15_000);

  it('serves the built browser and applies its independent latest-action pump through Rust', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-browser-action-'));
    const server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, resume: 'fresh', seed: 73,
      dbPath: join(root, 'experiment.sqlite') });
    let pump: PlayerActionPump | undefined;
    let browser: WsClient | undefined;
    try {
      const page = await fetch(`http://127.0.0.1:${server.port}/`);
      expect(page.status).toBe(200);
      expect(page.headers.get('content-type')).toMatch(/^text\/html/u);
      expect(await page.text()).toContain('<canvas id="c"');

      vi.stubGlobal('WebSocket', WebSocket);
      let welcome: WelcomeMsg | undefined;
      let assignment: AssignMsg | undefined;
      let sample: SensorsMsg | undefined;
      let latestFrame: Buffer | undefined;
      const errors: string[] = [];
      browser = createWsClient({
        onConnected(info) { welcome = info; browser?.sendJoin('player', 'browser-pump'); },
        onDisconnected() {},
        onFrame(frame) { latestFrame = Buffer.from(frame); },
        onStats() {},
        onAssign(message) { assignment = message; },
        // Incoming sensors update observation state only; they never produce an action.
        onSensors(message) { sample = message; },
        onError(message) { errors.push(message.message); }
      });
      browser.connect(`ws://127.0.0.1:${server.port}`);
      const deadline = performance.now() + 5000;
      while ((!welcome || !assignment || !sample) && performance.now() < deadline) await new Promise<void>(done => setTimeout(done, 10));
      expect(welcome).toMatchObject({ protocolVersion: 2, worldSeed: 73, inferenceMode: { activeBackend: 'native' } });
      expect(assignment).toBeDefined();
      expect(sample).toBeDefined();
      if (!assignment || !sample) throw new Error(`browser transport did not assign: ${errors.join('; ')}`);
      const snakeId = assignment.snakeId;
      const clientTick = sample.tick;
      const frameDeadline = performance.now() + 5000;
      while (frameDirection(latestFrame, snakeId) === undefined && performance.now() < frameDeadline) await new Promise<void>(done => setTimeout(done, 10));
      const initialDirection = frameDirection(latestFrame, snakeId);
      expect(initialDirection).toEqual(expect.any(Number));

      let turn = 1;
      let boost = 1;
      const actions: Array<{ turn: number; boost: number }> = [];
      pump = new PlayerActionPump({ cadenceHz: 60, isActive: () => browser?.isConnected() === true,
        buildLatestAction: () => ({ tick: clientTick, snakeId, turn, boost }),
        sendAction: action => { actions.push({ turn: action.turn, boost: action.boost });
          browser?.sendAction(action.tick, action.snakeId, action.turn, action.boost); } });
      // No sensor or frame callback invokes the pump: its own timer and change
      // request are the only producers while incoming state is merely observed.
      pump.start();
      const firstTurnDeadline = performance.now() + 5000;
      while (performance.now() < firstTurnDeadline) {
        const direction = frameDirection(latestFrame, snakeId);
        if (direction !== undefined && initialDirection !== undefined && directionDelta(initialDirection, direction) > 0.02) break;
        await new Promise<void>(done => setTimeout(done, 10));
      }
      const beforeRelease = frameDirection(latestFrame, snakeId)!;
      expect(directionDelta(initialDirection!, beforeRelease)).toBeGreaterThan(0.02);
      turn = -1;
      boost = 0;
      pump.requestImmediate();
      const reverseDeadline = performance.now() + 5000;
      while (performance.now() < reverseDeadline) {
        const direction = frameDirection(latestFrame, snakeId);
        if (actions.some(action => action.turn === -1 && action.boost === 0) &&
            direction !== undefined && directionDelta(beforeRelease, direction) < -0.02) break;
        await new Promise<void>(done => setTimeout(done, 10));
      }
      expect(directionDelta(beforeRelease, frameDirection(latestFrame, snakeId)!)).toBeLessThan(-0.02);
      expect(actions.at(-1)).toEqual({ turn: -1, boost: 0 });
      expect(errors).toEqual([]);
      const health = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as {
        telemetry: { playerAction: { samples: number } };
      };
      expect(health.telemetry.playerAction.samples).toBeGreaterThan(0);
    } finally {
      pump?.stop();
      browser?.disconnect();
      vi.unstubAllGlobals();
      await server.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);

  it('delivers fresh assignment and same-snake reclaim while display frames remain backpressured', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-frame-pressure-'));
    const peers: Peer[] = [];
    const sendErrors = vi.spyOn(console, 'error');
    const bufferedAmount = Object.getOwnPropertyDescriptor(WebSocket.prototype, 'bufferedAmount')?.get;
    if (!bufferedAmount) throw new Error('real ws transport has no buffered-amount getter');
    /** Induce only the server display admission condition; real TCP JSON sends still execute. */
    const pressure = vi.spyOn(WebSocket.prototype, 'bufferedAmount', 'get').mockImplementation(function (this: WebSocket) {
      return (this as WebSocket & { _isServer: boolean })._isServer
        ? 1024 * 1024 : Number(bufferedAmount.call(this));
    });
    let server: Awaited<ReturnType<typeof startRustServer>> | undefined;
    try {
      server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, resume: 'fresh', seed: 76,
        dbPath: join(root, 'experiment.sqlite') });
      const first = await connect(server.port, 'ui'); peers.push(first);
      first.socket.send(JSON.stringify({ type: 'join', rejoinToken: first.rejoinToken, mode: 'player', name: 'frame-pressure-player' }));
      await until(first, () => first.packets.some(packet => packet['type'] === 'assign'));
      const assignment = first.packets.find(packet => packet['type'] === 'assign')!;
      const initial = await healthUntil(server.port, value => {
        const outbound = value['outbound'] as WsOutboundDiagnostics;
        return outbound.replacedFrames >= 3 && outbound.pendingFrames === 1;
      });
      expect(first.frames).toBe(0);
      expect((initial['outbound'] as WsOutboundDiagnostics).reliableFailures).toBe(0);
      expect(first.packets.filter(packet => packet['type'] === 'assign')).toHaveLength(1);
      expect((initial['telemetry'] as { controllerActivity: { player: { freshAssignments: number } } })
        .controllerActivity.player.freshAssignments).toBe(1);
      await new Promise<void>(done => { first.socket.once('close', done); first.socket.close(); });
      await healthUntil(server.port, value => (value['outbound'] as WsOutboundDiagnostics).connections === 0);
      const resumed = await connect(server.port, 'ui'); peers.push(resumed);
      resumed.socket.send(JSON.stringify({ type: 'join', rejoinToken: resumed.rejoinToken, mode: 'player', name: 'frame-pressure-player',
        resumeToken: assignment['resumeToken'] }));
      await until(resumed, () => resumed.packets.some(packet => packet['type'] === 'assign') &&
        resumed.packets.some(packet => packet['type'] === 'reclaimResult'));
      expect(resumed.packets.filter(packet => packet['type'] === 'assign')).toHaveLength(1);
      expect(resumed.packets.filter(packet => packet['type'] === 'reclaimResult')).toHaveLength(1);
      expect(resumed.packets.find(packet => packet['type'] === 'reclaimResult')).toMatchObject({ reclaimed: true,
        snakeId: assignment['snakeId'] });
      const reclaimed = resumed.packets.find(packet => packet['type'] === 'assign')!;
      expect(reclaimed).toMatchObject({ reclaimed: true, snakeId: assignment['snakeId'] });
      expect(reclaimed['resumeToken']).not.toBe(assignment['resumeToken']);
      const measured = await healthUntil(server.port, value => {
        const outbound = value['outbound'] as WsOutboundDiagnostics;
        return outbound.replacedFrames >= (initial['outbound'] as WsOutboundDiagnostics).replacedFrames + 3 &&
          outbound.pendingFrames === 1 && BigInt(`0x${String(value['completedStep'])}`) > BigInt(`0x${String(initial['completedStep'])}`);
      });
      expect(resumed.frames).toBe(0);
      expect(measured).toMatchObject({ ok: true, outbound: { connections: 1, pendingFrames: 1 },
        telemetry: { controllerActivity: { player: { freshAssignments: 1, successfulReclaims: 1 } } } });
      const outbound = measured['outbound'] as WsOutboundDiagnostics;
      // A racing observation may fail on the deliberately closed first peer.
      // Preserve the lifetime count and reject any failure on the reclaimed peer.
      const reliableErrors = sendErrors.mock.calls.filter(([label]) => label === '[ws.reliable_send_failed]');
      expect(outbound.reliableFailures).toBe(reliableErrors.length);
      for (const [, details] of reliableErrors) {
        expect(details).toMatchObject({ connId: 1, reason: expect.stringMatching(/not open|closed/iu) });
      }
      expect(outbound.highWaterReliableMessagesPerConnection).toBeGreaterThan(0);
      expect(outbound.highWaterReliableMessagesPerConnection).toBeLessThanOrEqual(outbound.maxReliableMessagesPerConnection);
      expect(outbound.highWaterReliableBytesPerConnection).toBeLessThanOrEqual(outbound.maxReliableBytesPerConnection);
      const queues = measured['nativeQueues'] as RustQueueDiagnostics;
      expect(queues.output.priorityOverflows).toBe('0000000000000000');
      expect(queues.inbound.faultDiscardedCommands).toBe('0000000000000000');
      expect(BigInt(`0x${queues.output.highWaterOwnedBytes}`)).toBeLessThanOrEqual(BigInt(`0x${queues.output.maxOwnedBytes}`));
      pressure.mockRestore();
      await until(resumed, () => resumed.frames > 0);
      expect(frameDirection(resumed.latestFrame, Number(assignment['snakeId']))).toBeDefined();
    } finally {
      pressure.mockRestore();
      sendErrors.mockRestore();
      for (const peer of peers) peer.socket.terminate();
      await server?.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);

  it('applies browser-player input while inbound frames and sensors are paused', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-browser-suppression-'));
    const server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, resume: 'fresh', seed: 74,
      dbPath: join(root, 'experiment.sqlite') });
    try {
      const report = await runStage6RuntimeProbe({
        wsUrl: `ws://127.0.0.1:${server.port}/`,
        durationMs: 4_000,
        reconnectAfterSensors: 2,
        requireGenerationTransition: false,
        playerSuppressionMs: 750,
        requireFrameReplacement: false
      });
      const browserPlayer = report['browserPlayer'] as {
        actionsDuringSuppression: number;
        inboundDuringSuppression: number;
        latestTurn: number;
        latestBoost: number;
      };
      const suppression = report['playerSuppression'] as {
        serverPlayerActionSamplesBefore: number;
        serverPlayerActionSamplesAfter: number;
        recoveredSensors: number;
        recoveredFrames: number;
      };
      expect(browserPlayer.actionsDuringSuppression).toBeGreaterThan(0);
      expect(browserPlayer.inboundDuringSuppression).toBe(0);
      expect(browserPlayer).toMatchObject({ latestTurn: -1, latestBoost: 0 });
      expect(suppression.serverPlayerActionSamplesAfter)
        .toBeGreaterThan(suppression.serverPlayerActionSamplesBefore);
      expect(suppression.recoveredSensors).toBeGreaterThan(0);
      expect(suppression.recoveredFrames).toBeGreaterThan(0);
    } finally {
      await server.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);
});
