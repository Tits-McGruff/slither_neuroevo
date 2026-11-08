import Database from 'better-sqlite3';
import { createHash,randomUUID } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { mkdir,mkdtemp,readdir,readFile,rm,writeFile } from 'node:fs/promises';
import { request as httpRequest } from 'node:http';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { dirname,join,resolve } from 'node:path';
import { expect,it } from 'vitest';
import WebSocket from 'ws';
import { type WelcomeMsg } from '../src/net/wsClient.ts';
import type { SelectedEntity } from '../src/protocol/entitySelection.ts';
import { CheckpointPersistenceClient } from './rustEngine/checkpointPersistenceClient.ts';
import type { ManagedLegacySnapshotFormat } from './rustEngine/checkpointPersistenceProtocol.ts';
import { admitDiskOperation,CHECKPOINT_DISK_ADMISSION_REQUEST } from './rustEngine/diskAdmission.ts';
import { loadExperimentalFreshRunSession } from './rustEngine/experimentalFreshRunSession.ts';
import { startRustServer } from './rustServer.ts';
import { fixtureArchiveDownload } from './test/archiveDownload.ts';
import { RUST_TEST_CONFIG as DEFAULT_CONFIG,rustWorkersForTest } from './test/rustConfig.ts';
/** Available parallel count for scenarios that characterize four Rust workers. */
export const PARALLEL_WORKERS = rustWorkersForTest(4);
/** Browser metadata reports zero active pool workers when calculation is serial. */
export const ACTIVE_WORKERS = PARALLEL_WORKERS > 1 ? PARALLEL_WORKERS : 0;

/** Bounded test inbox for the real Protocol 2 transport. */
export interface Peer {
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
export async function connect(port: number, clientType: 'ui' | 'bot', origin?: string, reward = false): Promise<Peer> {
  const peer: Peer = { socket: new WebSocket(`ws://127.0.0.1:${port}`,
    origin === undefined ? {} : { origin }), packets: [], frames: 0 };
  peer.socket.on('message', (data, binary) => {
    if (binary && reward) {
      const bytes = Buffer.from(data as Buffer);
      expect(bytes.subarray(0, 4).toString('ascii')).toBe('SNS2');
      expect(bytes.readUInt8(4)).toBe(2);
      const count = bytes.readUInt16LE(6);
      const start = 44 + count * 4;
      const rewardBytes = bytes.readUInt32LE(start);
      expect(rewardBytes).toBeLessThanOrEqual(4096);
      expect(bytes.length).toBe(start + 4 + rewardBytes);
      if (peer.packets.length < 256) peer.packets.push({ type: 'sensors',
        snakeId: bytes.readUInt32LE(16), tick: Number(bytes.readBigUInt64LE(8)),
        sensors: Array.from({ length: count }, (_, index) => bytes.readFloatLE(44 + index * 4)),
        reward: JSON.parse(bytes.subarray(start + 4).toString('utf8')) });
    }
    else if (binary) { peer.frames++; peer.latestFrame = Buffer.from(data as Buffer); }
    else {
      const packet = JSON.parse(data.toString()) as Record<string, unknown>;
      if (packet['type'] === 'stateReplaced') peer.rejoinToken = String(packet['rejoinToken']);
      if (peer.packets.length < 256) peer.packets.push(packet);
    }
  });
  await new Promise<void>((done, reject) => { peer.socket.once('open', done); peer.socket.once('error', reject); });
  peer.socket.send(JSON.stringify({ type: 'hello', version: 2, clientType,
    ...(reward ? { rewardVersion: 1, binarySensorsVersion: 2, binarySensors: true } : {}) }));
  return peer;
}

/** Wait on a bounded real transport outcome, preserving diagnostics on failure. */
export async function until(peer: Peer, predicate: () => boolean): Promise<void> {
  const deadline = performance.now() + 5000;
  while (!predicate() && performance.now() < deadline) await new Promise<void>(done => setTimeout(done, 10));
  expect(predicate(), JSON.stringify(peer.packets)).toBe(true);
}

/** Read small fixture payloads over real HTTP on any OS-assigned port.
 * Node fetch applies browser unsafe-port blocking even to disposable local servers.
 */
export async function manualFixtureHttp(url: string, body?: Uint8Array, contentType = 'application/octet-stream'): Promise<{ status: number; bytes: Buffer }> {
  return new Promise((done, reject) => {
    // Cold native work can outlive the server's idle keep-alive timeout. Each
    // fixture transfer owns a fresh socket instead of racing reuse of a closed one.
    const request = httpRequest(url, { method: body ? 'POST' : 'GET', agent: false,
      headers: body ? {'Content-Type': contentType, 'Content-Length': body.byteLength} : {} }, response => {
      const chunks: Buffer[] = [];
      response.on('data', (chunk: Buffer) => chunks.push(chunk));
      response.once('error', reject);
      response.once('end', () => done({ status: response.statusCode ?? 0, bytes: Buffer.concat(chunks) }));
    });
    request.once('error', error => reject(new Error(`fixture HTTP request failed: ${url}`, {cause: error})));
    request.end(body);
  });
}

/** Poll one real health condition within the existing five-second integration bound. */
export async function healthUntil(
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
export async function replacementUntil(peer: Peer, port: number, reason: 'reset' | 'newRun'): Promise<WelcomeMsg> {
  await until(peer, () => peer.packets.some(packet => packet['type'] === 'stateReplaced' && packet['reason'] === reason));
  const welcome = peer.packets.findLast(packet => packet['type'] === 'stateReplaced' && packet['reason'] === reason)!['welcome'] as WelcomeMsg;
  await healthUntil(port, health => (health['retention'] as { activeRunId: string }).activeRunId === welcome.runId);
  return welcome;
}

/** Read one assigned snake direction from the compact frame-v1 contract. */
export function frameDirection(bytes: Buffer | undefined, snakeId: number): number | undefined {
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
export function firstFrameSnake(bytes: Buffer | undefined): { snakeId: number; x: number; y: number } | undefined {
  if (!bytes || bytes.byteLength < 15 * Float32Array.BYTES_PER_ELEMENT) return undefined;
  const value = (index: number): number => bytes.readFloatLE(index * Float32Array.BYTES_PER_ELEMENT);
  if (Math.trunc(value(2)) < 1) return undefined;
  return { snakeId: value(7), x: value(10), y: value(11) };
}

/** Select a real pellet with clearance from other bodies for controlled food transport coverage. */
export function rewardFixturePellet(bytes: Buffer, controlledId: number): { x: number; y: number } {
  const read = (index: number): number => bytes.readFloatLE(index * 4);
  const occupied: Array<{ x: number; y: number }> = [];
  let cursor = 7;
  for (let snake = 0; snake < read(2); snake++) {
    const id = read(cursor);
    const count = read(cursor + 7);
    for (let point = 0; point < count; point++) {
      if (id !== controlledId) occupied.push({ x: read(cursor + 8 + point * 2),
        y: read(cursor + 9 + point * 2) });
    }
    cursor += 8 + count * 2;
  }
  const count = read(cursor++);
  for (let pellet = 0; pellet < count; pellet++, cursor += 5) {
    const position = { x: read(cursor), y: read(cursor + 1) };
    if (Math.hypot(position.x, position.y) < read(3) - 300 &&
        occupied.every(body => Math.hypot(position.x - body.x, position.y - body.y) > 100)) return position;
  }
  throw new Error('reward fixture has no safe authoritative pellet');
}

/** Remove only the task-owned reward fixture after validating its absolute target. */
export async function cleanupRewardFixture(root: string): Promise<void> {
  if (dirname(resolve(root)) !== resolve(tmpdir()) || !root.startsWith(join(tmpdir(), 'slither-rust-reward-'))) {
    throw new Error('unsafe reward fixture cleanup');
  }
  await rm(root, { recursive: true, force: true });
}

/** Signed shortest angular change from one wrapped direction to another. */
export function directionDelta(from: number, to: number): number {
  return Math.atan2(Math.sin(to - from), Math.cos(to - from));
}

/** Hash a large original save with bounded memory before and after a transfer test. */
export async function archiveFileSha256(path: string): Promise<string> {
  const hash = createHash('sha256');
  for await (const bytes of createReadStream(path)) hash.update(bytes);
  return hash.digest('hex');
}

/** Hash every original fixture row, including all scalar columns and exact stored BLOB bytes. */
export function legacyRecords(databasePath: string, originalTables?: readonly string[]): {
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
export function legacyCompanions(database: ReturnType<typeof Database>, graphJson: string, archKey: string, weightCount: number): void {
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
export async function readOnlyLegacyConversion(root: string, databasePath: string, sourceFormat: ManagedLegacySnapshotFormat): Promise<void> {
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

/** Read the small manifest from a bounded, actually exported versioned test archive. */
export function legacyExportManifest(bytes: Buffer): {
  archiveKind: string; legacyConversion?: import('../src/protocol/rustBackground.ts').RustLegacyConversionNotice;
  runId: string; checkpointLogicalRootSha256: string; logicalRootSha256: string;
  roles: Array<{ role: string; logicalSha256: string }>;
} {
  expect(bytes.byteLength).toBeLessThan(4 * 1024 * 1024);
  let cursor = 0;
  for (let entry = 0; entry < 10; entry++) {
    const name = bytes.subarray(cursor, cursor + 100).toString().split('\0')[0];
    const length = Number.parseInt(bytes.subarray(cursor + 124, cursor + 136).toString().replace(/\0.*$/su, '').trim(), 8);
    expect(Number.isSafeInteger(length) && length >= 0).toBe(true);
    if (name === 'manifest.json') {
      const manifest = JSON.parse(bytes.subarray(cursor + 512, cursor + 512 + length).toString()) as { archiveVersion: number };
      expect([1, 2]).toContain(manifest.archiveVersion);
      expect(entry).toBe(manifest.archiveVersion === 2 ? 9 : 8);
      return JSON.parse(bytes.subarray(cursor + 512, cursor + 512 + length).toString());
    }
    cursor += 512 + Math.ceil(length / 512) * 512;
  }
  throw new Error('legacy conversion export omitted its manifest');
}

/** Re-export and re-import a converted population through real archive HTTP, checking its original Float32 digest. */
export async function migratedArchiveRoundTrip(root: string, server: Awaited<ReturnType<typeof startRustServer>>, weightsSha256?: string): Promise<void> {
  const sourceHealth = JSON.parse((await manualFixtureHttp(`http://127.0.0.1:${server.port}/api/health`)).bytes.toString()) as Record<string, unknown>;
  const exported = await fixtureArchiveDownload(server.port, 'legacy conversion source');
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
    const imported = await manualFixtureHttp(`http://127.0.0.1:${target.port}/api/import/archive`, archive);
    expect(imported.status, imported.bytes.toString()).toBe(200);
    expect(JSON.parse(imported.bytes.toString())).toMatchObject({ ok: true, runId: manifest.runId,
      checkpointId: manifest.checkpointLogicalRootSha256, saveLogicalRootSha256: manifest.logicalRootSha256,
      legacyConversion: manifest.legacyConversion });
    await target.close();
    target = await startRustServer({ ...DEFAULT_CONFIG, port: 0, resume: 'latest', dbPath: targetPath });
    expect(target.startupFault).toBeUndefined();
    expect(JSON.parse((await manualFixtureHttp(`http://127.0.0.1:${target.port}/api/health`)).bytes.toString())).toMatchObject({
      ok: true, runId: manifest.runId, seed: sourceHealth['seed'], legacyConversion: manifest.legacyConversion });
    const reexported = await fixtureArchiveDownload(target.port, 'legacy conversion re-export');
    expect(reexported.status).toBe(200);
    const roundTrip = legacyExportManifest(Buffer.from(await reexported.arrayBuffer()));
    expect(roundTrip).toEqual(manifest);
    expect(createHash('sha256').update(archive).digest('hex')).toBe(originalHash);
  } finally { await target.close(); }
}

/** Replace only the final manifest in a private fixture, retaining valid USTAR framing. */
export function rewriteLegacyManifest(archive: Buffer, mutate: (manifest: Record<string, unknown>) => void): Buffer {
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

/** Resolve a real native identity from the peer's latest observed welcome and stats. */
export async function selectPhysical(peer: Peer, requestId: string, target: { snakeId: number } | { populationSlot: number }): Promise<SelectedEntity> {
  const replacement = peer.packets.findLast(packet => packet['type'] === 'stateReplaced');
  const welcome = (replacement?.['welcome'] ?? peer.packets.findLast(packet => packet['type'] === 'welcome')) as WelcomeMsg;
  if (!welcome) throw new Error('selection fixture has no welcome');
  const stats = peer.packets.findLast(packet => packet['type'] === 'stats');
  const generation = stats?.['gen'] ?? (peer.latestFrame ? peer.latestFrame.readFloatLE(0) : 1);
  peer.socket.send(JSON.stringify({ type: 'selectEntity', requestId, expectedSessionId: welcome.sessionId,
    expectedRunId: welcome.runId, expectedGeneration: generation, ...target }));
  await until(peer, () => peer.packets.some(packet => packet['requestId'] === requestId));
  const result = peer.packets.findLast(packet => packet['requestId'] === requestId)!;
  expect(result).toMatchObject({ type: 'entitySelected', found: true });
  return result['entity'] as SelectedEntity;
}

/** Verify this task-owned scratch path before recursive cleanup. */
export async function cleanupInspectorFixture(root: string): Promise<void> {
  if (dirname(resolve(root)) !== resolve(tmpdir()) || !root.startsWith(join(tmpdir(), 'slither-rust-inspector-'))) {
    throw new Error('unsafe inspector fixture cleanup');
  }
  await rm(root, {recursive: true, force: true});
}

/** Remove only the exact quota test's independently created temporary root. */
export async function cleanupQuotaFixture(root: string): Promise<void> {
  if (dirname(resolve(root)) !== resolve(tmpdir()) || !root.startsWith(join(tmpdir(), 'slither-spool-startup-'))) {
    throw new Error('unsafe quota-test cleanup');
  }
  await rm(root, { recursive: true, force: true });
}

it('rejects diagnostic disk quota before opening owner database or starting native authority', async () => {
  const root = await mkdtemp(join(tmpdir(), 'slither-spool-startup-'));
  const database = join(root, 'owner.db');
  try {
    await writeFile(database, 'preserved owner bytes');
    for (const quota of [0, 65, Number.NaN]) {
      let unexpected: Awaited<ReturnType<typeof startRustServer>> | undefined;
      try {
        await expect((async () => {
          unexpected = await startRustServer({ ...DEFAULT_CONFIG, port: 0, dbPath: database, distillationSpoolGiB: quota });
        })()).rejects.toThrow(/distillationSpoolGiB/u);
      } finally { await unexpected?.close(); }
      expect(await readdir(root)).toEqual(['owner.db']);
      expect(await readFile(database, 'utf8')).toBe('preserved owner bytes');
    }
  } finally {
    await cleanupQuotaFixture(root);
  }
});
