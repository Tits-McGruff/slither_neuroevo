/** Real HTTP archive framing and wire-limit acceptance with an unchanged prior experiment. */
import { createHash, randomBytes } from 'node:crypto';
import { execFile } from 'node:child_process';
import { createRequire } from 'node:module';
import { createReadStream } from 'node:fs';
import { mkdir, mkdtemp, readdir, readFile, readlink, rm, stat, statfs, unlink, writeFile } from 'node:fs/promises';
import { createServer, request, Server, type ClientRequest, type IncomingMessage, type ServerResponse } from 'node:http';
import { connect, type Socket } from 'node:net';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { promisify } from 'node:util';
import { constants as zstdConstants, createZstdDecompress, zstdCompressSync, zstdDecompressSync } from 'node:zlib';
import Database from 'better-sqlite3';
import { expect, it, onTestFinished, vi } from 'vitest';
import WebSocket from 'ws';
import { DEFAULT_CONFIG } from './config.ts';
import { startRustServer, type RustServer } from './rustServer.ts';
import { P0_ARCHIVE_UPLOAD_LIMIT } from './rustEngine/archiveUpload.ts';
import * as archiveUpload from './rustEngine/archiveUpload.ts';
import { BackgroundOutputPump } from './rustEngine/backgroundOutput.ts';
import { CheckpointPersistenceClient } from './rustEngine/checkpointPersistenceClient.ts';
import { loadExperimentalFreshRunSession } from './rustEngine/experimentalFreshRunSession.ts';
import * as freshRunSessions from './rustEngine/experimentalFreshRunSession.ts';
import { buildLargeBrainGraph } from '../scripts/stage2/fixtures.ts';
import { compileGraph } from '../src/brains/graph/compiler.ts';
import type { GraphSpec } from '../src/brains/graph/schema.ts';
import { admitDiskOperation, CHECKPOINT_DISK_ADMISSION_REQUEST } from './rustEngine/diskAdmission.ts';
import * as diskAdmission from './rustEngine/diskAdmission.ts';
import { describeNetworkSuite } from './test/networkSuites.ts';
import type { AssignMsg, SensorsMsg } from './protocol.ts';
import type { ExperimentalRuntimeTelemetrySnapshot } from './rustEngine/runtimeTelemetry.ts';
import type { ExperimentalRunningAuthorityNativeHandle } from './rustEngine/backgroundRuntime.ts';

/** Bounded native mount commands used only by the explicitly selected Linux quota fixture. */
const runFile = promisify(execFile);

/** Original HTTP dispatch, captured before a timed-out fixture can install an observer. */
const originalHttpEmit = Server.prototype.emit;

/** Change the task's private tmpfs quota without allocating its advertised capacity. */
async function quota(directory: string, bytes: bigint): Promise<void> {
  // Do not reparse host-mapped uid/gid options from mountinfo inside the user namespace.
  await runFile('mount', ['--options-mode', 'replace', '--types', 'tmpfs', '--options',
    `remount,size=${bytes},uid=0,gid=0`, 'slither-a7-quota', directory], { timeout: 5000 });
}

/** One task-owned authority and its durable pre-request evidence. */
interface Fixture {
  /** Actual production server with a source-identified mandatory addon. */
  server: RustServer;
  /** Private SQLite path owned only by this test. */
  databasePath: string;
  /** Private managed directory owned only by this test. */
  managedDirectory: string;
  /** Small valid archive used to distinguish transport rejection from codec rejection. */
  archive: Buffer;
  /** Prior public identity; simulation steps may continue. */
  identity: Record<string, unknown>;
  /** Every retained Rust metadata row before the request. */
  metadata: unknown;
  /** All managed filenames and exact stored-byte digests before the request. */
  files: Array<{ filename: string; sha256: string }>;
  /** Actual client sockets that must close before fixture deletion. */
  sockets: Set<Socket>;
  /** Actual HTTP clients that must close before fixture deletion. */
  requests: Set<ClientRequest>;
  /** Additional real controllers that close before the private fixture is removed. */
  peers: Set<WebSocket>;
}

/** Hash managed files incrementally; no population-sized buffer is constructed. */
async function files(directory: string): Promise<Fixture['files']> {
  const result: Fixture['files'] = [];
  for (const filename of (await readdir(directory)).sort()) {
    const hash = createHash('sha256');
    for await (const bytes of createReadStream(join(directory, filename))) hash.update(bytes);
    result.push({ filename, sha256: hash.digest('hex') });
  }
  return result;
}

/** Read every Rust metadata table through a task-owned read-only SQLite connection. */
function metadata(path: string): Array<{ name: string; rows: unknown[] }> {
  const database = new Database(path, { readonly: true });
  try {
    const tables = database.prepare(`SELECT name FROM sqlite_master
      WHERE type = 'table' AND name LIKE 'rust_%' ORDER BY name`).all() as Array<{ name: string }>;
    return tables.map(({ name }) => ({ name, rows: database.prepare(
      `SELECT * FROM "${name.replaceAll('"', '""')}" ORDER BY rowid`
    ).all() }));
  } finally { database.close(); }
}

/** Read actual health without retaining display frames or population state. */
async function health(server: RustServer): Promise<Record<string, unknown>> {
  const response = await fetch(`http://127.0.0.1:${server.port}/api/health`);
  expect(response.status).toBe(200);
  return await response.json() as Record<string, unknown>;
}

/** Wait for lease/spool cleanup while preserving a bounded failure diagnostic. */
async function noTransferScratch(directory: string): Promise<void> {
  const deadline = performance.now() + 5000;
  let leftovers: string[] = [];
  do {
    leftovers = (await readdir(directory)).filter(name =>
      name.includes('upload') || name.includes('slither-save') || name.includes('export-inventory') ||
      name.includes('import-'));
    if (leftovers.length === 0) return;
    await new Promise<void>(done => setTimeout(done, 10));
  } while (performance.now() < deadline);
  expect(leftovers).toEqual([]);
}

/** Preserve a generation boundary during the transfer without pausing the running game. */
async function slow(socket: WebSocket): Promise<void> {
  await new Promise<void>((done, reject) => {
    const timeout = setTimeout(() => reject(new Error('transport fixture settings timed out')), 5000);
    socket.on('message', (bytes, binary) => {
      if (binary) return;
      const message = JSON.parse(bytes.toString()) as Record<string, unknown>;
      if (message['type'] === 'welcome') {
        socket.send(JSON.stringify({ type: 'join', mode: 'spectator' }));
        socket.send(JSON.stringify({ type: 'settings', requestId: 'transport-speed',
          updates: [{ path: 'simSpeed', value: 0.1 }] }));
      }
      if (message['type'] === 'error') {
        clearTimeout(timeout);
        reject(new Error(`transport fixture rejected: ${String(message['message'])}`));
      }
      if (message['type'] === 'settingsApplied' && message['requestId'] === 'transport-speed') {
        clearTimeout(timeout);
        if (message['applied'] === true) done();
        else reject(new Error('transport fixture settings rejected'));
      }
    });
    socket.once('error', error => { clearTimeout(timeout); reject(error); });
    socket.once('open', () => socket.send(JSON.stringify({ type: 'hello', version: 2, clientType: 'ui' })));
  });
}

/** Create, close and remove one exact fixture; owner databases are never opened. */
async function experiment(action: (fixture: Fixture) => Promise<void>, privateQuota = false): Promise<void> {
  const root = await mkdtemp(join(tmpdir(), 'slither-archive-transport-'));
  const databasePath = join(root, 'experiment.sqlite');
  const managedDirectory = `${databasePath}.checkpoints`;
  let server: RustServer | undefined;
  let fixture: Fixture | undefined;
  let socket: WebSocket | undefined;
  const sockets = new Set<Socket>();
  const requests = new Set<ClientRequest>();
  const peers = new Set<WebSocket>();
  let mounted = false;
  try {
    if (privateQuota) {
      // Refuse host-root or shared-namespace execution before invoking mount.
      expect(process.platform).toBe('linux');
      expect(await readFile('/proc/self/uid_map', 'utf8')).toMatch(/^\s*0\s+[1-9]\d*\s+1\s*$/mu);
      const parentNamespace = process.env['SLITHER_PARENT_MOUNT_NAMESPACE'];
      expect(parentNamespace).toMatch(/^mnt:\[\d+\]$/u);
      expect(await readlink('/proc/self/ns/mnt')).not.toBe(parentNamespace);
      await mkdir(managedDirectory);
      await runFile('mount', ['--types', 'tmpfs', '--options', 'size=3G,nosuid,nodev,mode=0700',
        'slither-a7-quota', managedDirectory], { timeout: 5000 });
      mounted = true;
      expect((await statfs(managedDirectory, { bigint: true })).type).toBe(0x01021994n);
    }
    server = await startRustServer({ ...DEFAULT_CONFIG, host: '127.0.0.1', port: 0,
      resume: 'fresh', seed: 42, dbPath: databasePath });
    expect(server.startupFault).toBeUndefined();
    socket = new WebSocket(`ws://127.0.0.1:${server.port}`);
    await slow(socket);
    const response = await fetch(`http://127.0.0.1:${server.port}/api/export/latest`);
    expect(response.status).toBe(200);
    const archive = Buffer.from(await response.arrayBuffer());
    expect(archive.byteLength).toBeLessThan(4 * 1024 * 1024);
    await noTransferScratch(managedDirectory);
    const before = await health(server);
    const identityKeys = ['runId', 'seed', 'generation', 'worldEpoch', 'configHash', 'startupCheckpointId'];
    for (const key of identityKeys) expect(before[key], `health omitted ${key}`).toBeDefined();
    const identity = Object.fromEntries(identityKeys.map(key => [key, before[key]]));
    fixture = { server, databasePath, managedDirectory, archive, identity,
      metadata: metadata(databasePath), files: await files(managedDirectory), sockets, requests, peers };
    await action(fixture);
  } finally {
    for (const client of requests) client.destroy();
    for (const client of sockets) client.destroy();
    for (const peer of peers) peer.terminate();
    socket?.terminate();
    await (fixture?.server ?? server)?.close();
    if (mounted) await runFile('umount', [managedDirectory], { timeout: 5000 });
    await rm(root, { recursive: true, force: true });
  }
}

/** Assert rejection preserved the entire saved experiment and its active world identity. */
async function preserved(fixture: Fixture): Promise<void> {
  await noTransferScratch(fixture.managedDirectory);
  expect(metadata(fixture.databasePath)).toEqual(fixture.metadata);
  expect(await files(fixture.managedDirectory)).toEqual(fixture.files);
  expect(await health(fixture.server)).toMatchObject({ ok: true, ...fixture.identity });
}

/** Prove the server finished disconnected work by completing a fresh real export. */
async function exportAfterDisconnect(fixture: Fixture): Promise<void> {
  const deadline = performance.now() + 5000;
  while (performance.now() < deadline) {
    const response = await fetch(`http://127.0.0.1:${fixture.server.port}/api/export/latest`,
      { signal: AbortSignal.timeout(5000) });
    if (response.status === 409) {
      expect(await response.json()).toMatchObject({ ok: false,
        message: 'another persistence operation is in progress' });
      await new Promise<void>(done => setTimeout(done, 10));
      continue;
    }
    expect(response.status).toBe(200);
    expect(Buffer.from(await response.arrayBuffer()).equals(fixture.archive)).toBe(true);
    return;
  }
  throw new Error('disconnected import did not release its busy gate');
}

/** Build a supported legacy population whose import creates a distinct candidate and run. */
function legacyPopulation(): Buffer {
  return Buffer.from(JSON.stringify({ generation: 37, archKey: 'legacy-default-graph',
    worldSeed: 1234567, settings: { snakeCount: 2, simSpeed: 0.1 },
    genomes: [1, 2].map(fitness => ({ archKey: 'legacy-default-graph', brainType: 'mlp',
      fitness, weights: new Array<number>(13458).fill(0) })) }));
}

/** Locate extents only in the canonical small archive produced by this test's real exporter. */
function archiveEntries(archive: Buffer): Array<{ header: number; data: number; size: number; end: number }> {
  const entries: Array<{ header: number; data: number; size: number; end: number }> = [];
  let header = 0;
  while (header < archive.byteLength - 1024) {
    expect(archive.subarray(header + 257, header + 265).toString()).toBe('ustar\0' + '00');
    const size = Number.parseInt(archive.subarray(header + 124, header + 136).toString().replaceAll('\0', '').trim(), 8);
    expect(Number.isSafeInteger(size) && size >= 0).toBe(true);
    const data = header + 512;
    const end = data + Math.ceil(size / 512) * 512;
    expect(end).toBeLessThanOrEqual(archive.byteLength - 1024);
    entries.push({ header, data, size, end });
    header = end;
  }
  expect(entries).toHaveLength(9);
  expect(header + 1024).toBe(archive.byteLength);
  expect(archive.subarray(header).every(byte => byte === 0)).toBe(true);
  return entries;
}

/** Update a deliberately changed USTAR header so tests reach type/path validation beyond checksum. */
function archiveHeaderChecksum(bytes: Buffer, header: number): void {
  bytes.fill(0x20, header + 148, header + 156);
  const checksum = bytes.subarray(header, header + 512).reduce((sum, byte) => sum + byte, 0);
  bytes.write(checksum.toString(8).padStart(6, '0') + '\0 ', header + 148, 8, 'ascii');
}

/** Manifest fields needed to independently rehash a small semantic failure fixture. */
interface SemanticRole {
  /** Ordered logical role name. */
  role: string;
  /** Stored role codec; these fixtures change raw binary roles only. */
  encoding: string;
  /** Exact stored length as a fixed-width unsigned hexadecimal value. */
  storedBytesHex: string;
  /** Exact logical length with the same fixed-width encoding. */
  decodedBytesHex: string;
  /** Hash of the decoded role bytes. */
  logicalSha256: string;
}

/** Selected manifest fields; JSON parsing retains all other production fields. */
interface SemanticManifest {
  /** Save root over all eight ordered roles. */
  logicalRootSha256: string;
  /** Nested checkpoint root referenced by the save. */
  checkpointLogicalRootSha256: string;
  /** Complete ordered save roles. */
  roles: SemanticRole[];
  /** Complete nested manifest, retaining unchanged configuration/layout/counts. */
  checkpointManifest: {
    /** Root over the five checkpoint roles. */
    logicalRootSha256: string;
    /** Total stored bytes across those roles. */
    roleStoredBytesHex: string;
    /** Total decoded bytes across those roles. */
    roleDecodedBytesHex: string;
    /** Mirrored checkpoint roles. */
    roles: SemanticRole[];
  };
}

/** Independently implement the published ordered-role digest, checked against real exports. */
function semanticRoot(domain: string, roles: SemanticRole[]): string {
  const hash = createHash('sha256').update(domain);
  const count = Buffer.alloc(4);
  count.writeUInt32LE(roles.length);
  hash.update(count);
  for (const role of roles) {
    const name = Buffer.from(role.role);
    const nameLength = Buffer.alloc(2);
    nameLength.writeUInt16LE(name.byteLength);
    const logicalLength = Buffer.alloc(8);
    logicalLength.writeBigUInt64LE(BigInt(`0x${role.decodedBytesHex}`));
    hash.update(nameLength).update(name).update(logicalLength).update(Buffer.from(role.logicalSha256, 'hex'));
  }
  return hash.digest('hex');
}

/** Repack a canonical small save with honest hashes, lengths and both logical roots. */
function semanticArchive(archive: Buffer, roleIndex?: number, changed?: Buffer): Buffer {
  const entries = archiveEntries(archive);
  const bodies: Buffer[] = entries.map(entry => Buffer.from(archive.subarray(entry.data, entry.data + entry.size)));
  const manifest = JSON.parse(bodies[8]!.toString()) as SemanticManifest;
  const checkpointDomain = 'slither-neuroevo-logical-checkpoint-root\0v1\0';
  const saveDomain = 'slither-neuroevo-save-root\0v1\0';
  expect(semanticRoot(checkpointDomain, manifest.checkpointManifest.roles)).toBe(manifest.checkpointLogicalRootSha256);
  expect(semanticRoot(saveDomain, manifest.roles)).toBe(manifest.logicalRootSha256);
  if (roleIndex !== undefined) {
    expect(changed).toBeDefined();
    expect(manifest.roles[roleIndex]!.encoding).toBe('raw-binary-v1');
    bodies[roleIndex] = changed!;
    const role = manifest.roles[roleIndex]!;
    role.storedBytesHex = changed!.byteLength.toString(16).padStart(16, '0');
    role.decodedBytesHex = role.storedBytesHex;
    role.logicalSha256 = createHash('sha256').update(changed!).digest('hex');
    manifest.checkpointManifest.roles[roleIndex] = { ...role };
  }
  for (const [field, roleField] of [['roleStoredBytesHex', 'storedBytesHex'],
    ['roleDecodedBytesHex', 'decodedBytesHex']] as const) {
    manifest.checkpointManifest[field] = manifest.checkpointManifest.roles.reduce((sum, role) =>
      sum + BigInt(`0x${role[roleField]}`), 0n).toString(16).padStart(16, '0');
  }
  manifest.checkpointManifest.logicalRootSha256 = semanticRoot(checkpointDomain, manifest.checkpointManifest.roles);
  manifest.checkpointLogicalRootSha256 = manifest.checkpointManifest.logicalRootSha256;
  manifest.logicalRootSha256 = semanticRoot(saveDomain, manifest.roles);
  bodies[8] = Buffer.from(JSON.stringify(manifest));
  const chunks: Buffer[] = [];
  for (const [index, body] of bodies.entries()) {
    const header = Buffer.from(archive.subarray(entries[index]!.header, entries[index]!.data));
    header.write(body.byteLength.toString(8).padStart(11, '0') + '\0', 124, 12, 'ascii');
    archiveHeaderChecksum(header, 0);
    chunks.push(header, body, Buffer.alloc((512 - body.byteLength % 512) % 512));
  }
  chunks.push(Buffer.alloc(1024));
  return Buffer.concat(chunks);
}

/** Change actual Concat input ordering while retaining the encoded original compiled identity. */
function reversedConcatRole(role: Buffer): Buffer {
  const changed = Buffer.from(role);
  /** Encode the checkpoint's length-prefixed UTF-8 node identifier. */
  const text = (value: string): Buffer => {
    const bytes = Buffer.from(value);
    const size = Buffer.alloc(4);
    size.writeUInt32LE(bytes.byteLength);
    return Buffer.concat([size, bytes]);
  };
  for (const port of [0n, 1n]) {
    const ports = Buffer.alloc(18);
    ports[0] = 1;
    ports[9] = 1;
    ports.writeBigInt64LE(port, 1);
    ports.writeBigInt64LE(port, 10);
    const edge = Buffer.concat([text('split'), text('concat'), ports]);
    const offset = changed.indexOf(edge);
    expect(offset).toBeGreaterThanOrEqual(0);
    expect(changed.indexOf(edge, offset + 1)).toBe(-1);
    changed.writeBigInt64LE(1n - port, offset + edge.byteLength - 8);
  }
  return changed;
}

/** Valid equal-total-width graphs whose explicit merge order changes their weight interpretation. */
function orderingGraph(reversed = false): GraphSpec {
  return {
    type: 'graph',
    nodes: [{ id: 'input', type: 'Input', outputSize: 83 },
      { id: 'split', type: 'Split', outputSizes: [41, 42] }, { id: 'concat', type: 'Concat' },
      { id: 'head', type: 'Dense', inputSize: 83, outputSize: 2 }],
    edges: [{ from: 'input', to: 'split', fromPort: 0 },
      { from: 'split', to: 'concat', fromPort: 0, toPort: reversed ? 1 : 0 },
      { from: 'split', to: 'concat', fromPort: 1, toPort: reversed ? 0 : 1 },
      { from: 'concat', to: 'head', fromPort: 0 }],
    outputs: [{ nodeId: 'head', port: 0 }], outputSize: 2
  };
}

/** Admit an evolved checkpoint and prove the independent repacker's valid control imports. */
async function evolvedArchiveFixture(fixture: Fixture, mode: 'ordering' | 'streaming' = 'ordering'): Promise<void> {
  const streaming = mode === 'streaming';
  const viewer = new WebSocket(`ws://127.0.0.1:${fixture.server.port}`);
  fixture.peers.add(viewer);
  await slow(viewer);
  const reset = new Promise<void>((done, reject) => viewer.on('message', (bytes, binary) => {
    if (binary) return;
    const packet = JSON.parse(bytes.toString()) as Record<string, unknown>;
    if (packet['type'] === 'error') reject(new Error(String(packet['message'])));
    if (packet['type'] === 'stateReplaced' && packet['reason'] === 'reset') done();
  }));
  viewer.send(JSON.stringify({ type: 'reset', settings: { snakeCount: streaming ? 300 : 12, simSpeed: 12 },
    updates: [{ path: 'generationSeconds', value: 8 }, { path: 'baselineBots.count', value: 2 },
      { path: 'pelletCountTarget', value: 100 }, ...(streaming ? [{ path: 'worldRadius', value: 10000 }] : [])],
    graphSpec: streaming ? null : orderingGraph() }));
  await bounded(reset, 'ordering fixture reset did not commit');
  viewer.send(JSON.stringify({ type: 'join', mode: 'spectator' }));
  const rejoined = new Promise<Buffer>(done => viewer.once('pong', done));
  viewer.ping('ordering-fixture-rejoin');
  expect((await bounded(rejoined, 'ordering fixture rejoin was not received')).toString()).toBe('ordering-fixture-rejoin');
  const deadline = performance.now() + 5000;
  let generation = 1n;
  while (generation < 2n && performance.now() < deadline) {
    const current = await health(fixture.server);
    expect(current['ok']).toBe(true);
    generation = BigInt(`0x${current['generation'] as string}`);
    if (generation < 2n) await new Promise<void>(done => setTimeout(done, 10));
  }
  expect(generation).toBeGreaterThanOrEqual(2n);
  const slowed = new Promise<void>((done, reject) => viewer.on('message', (bytes, binary) => {
    if (binary) return;
    const packet = JSON.parse(bytes.toString()) as Record<string, unknown>;
    if (packet['type'] === 'error') reject(new Error(String(packet['message'])));
    if (packet['type'] === 'settingsApplied' && packet['requestId'] === 'ordering-fixture-speed') {
      if (packet['applied'] === true) done();
      else reject(new Error('ordering fixture speed was rejected'));
    }
  }));
  viewer.send(JSON.stringify({ type: 'settings', requestId: 'ordering-fixture-speed',
    updates: [{ path: 'simSpeed', value: 0.1 }] }));
  await bounded(slowed, 'ordering fixture did not slow after evolution');
  viewer.terminate();
  const response = await fetch(`http://127.0.0.1:${fixture.server.port}/api/export/latest`);
  expect(response.status).toBe(200);
  fixture.archive = Buffer.from(await response.arrayBuffer());
  expect(fixture.archive.byteLength).toBeLessThan((streaming ? 64 : 4) * 1024 * 1024);
  if (streaming) expect(fixture.archive.byteLength).toBeGreaterThan(8 * 1024 * 1024);
  await noTransferScratch(fixture.managedDirectory);
  const repacked = semanticArchive(fixture.archive);
  const imported = await fetch(`http://127.0.0.1:${fixture.server.port}/api/import/archive`,
    { method: 'POST', body: new Uint8Array(repacked), signal: AbortSignal.timeout(5000) });
  expect(imported.status, await imported.clone().text()).toBe(200);
  expect(await imported.json()).toMatchObject({ ok: true, branched: false });
  await noTransferScratch(fixture.managedDirectory);
  const current = await health(fixture.server);
  fixture.identity = Object.fromEntries(Object.keys(fixture.identity).map(key => [key, current[key]]));
  const rows = metadata(fixture.databasePath);
  expect(rows.find(table => table.name === 'rust_generation_history_v1')!.rows.length).toBeGreaterThan(0);
  expect(rows.find(table => table.name === 'rust_hall_of_fame_v1')!.rows.length).toBeGreaterThan(0);
  fixture.metadata = rows;
  fixture.files = await files(fixture.managedDirectory);
  expect(fixture.files.some(file => file.filename.endsWith('.hof-weights-v1'))).toBe(true);
}

/** A valid dictionary-free frame with no content-size field and 128 repeated 128-KiB blocks. */
function expansionFrame(): Buffer {
  const blocks = 128;
  const frame = Buffer.alloc(6 + blocks * 4);
  Buffer.from([0x28, 0xb5, 0x2f, 0xfd, 0x00, 0x50]).copy(frame);
  for (let index = 0; index < blocks; index++) {
    frame.writeUIntLE((131072 << 3) | 2 | (index === blocks - 1 ? 1 : 0), 6 + index * 4, 3);
  }
  return frame;
}

/** Independently prove the expansion fixture is a valid stream without retaining its output. */
async function expansionBytes(frame: Buffer): Promise<number> {
  const decoder = createZstdDecompress({ chunkSize: 64 * 1024,
    params: { [zstdConstants.ZSTD_d_windowLogMax]: 20 } });
  decoder.end(frame);
  let count = 0;
  for await (const bytes of decoder) count += (bytes as Buffer).byteLength;
  return count;
}

/** Locate a payload-bit change that a separate bounded decoder accepts with different output. */
function alteredDecodedFrame(frame: Buffer, decodedBytes: number): Buffer {
  const original = zstdDecompressSync(frame, { maxOutputLength: decodedBytes });
  expect(original.byteLength).toBe(decodedBytes);
  for (let offset = frame.byteLength - 1; offset >= Math.max(16, frame.byteLength - 128); offset--) {
    const changed = Buffer.from(frame);
    changed[offset] = changed[offset]! ^ 1;
    let decoded: Buffer;
    try { decoded = zstdDecompressSync(changed, { maxOutputLength: decodedBytes }); }
    catch { continue; }
    if (decoded.byteLength === decodedBytes && !decoded.equals(original)) return changed;
  }
  throw new Error('could not construct a valid changed-bit numeric fixture');
}

/** Export independently valid Rust state that deliberately collides with an existing run/generation key. */
async function conflictingArchive(fixture: Fixture): Promise<Buffer> {
  const databasePath = join(dirname(fixture.databasePath), 'conflicting.sqlite');
  const managedDirectory = `${databasePath}.checkpoints`;
  await mkdir(managedDirectory);
  await admitDiskOperation(managedDirectory, CHECKPOINT_DISK_ADMISSION_REQUEST);
  const persistence = new CheckpointPersistenceClient({ databasePath, managedRootPath: managedDirectory });
  try {
    const nativeRequire = createRequire(import.meta.url);
    const session = await loadExperimentalFreshRunSession({ nativeManifestDirectory: resolve('native'),
      loadBinding: () => nativeRequire(resolve('native/index.js')),
      runId: String(fixture.identity['runId']), seed: 77,
      memoryCeilingBytes: 4n * 1024n ** 3n, calculationWorkers: 1, persistence, managedDirectory });
    await session.initialize();
    const committed = await session.commitPendingRunStart(randomBytes(16).toString('hex'));
    expect(committed.descriptor.generation).toBe(fixture.identity['generation']);
    expect(committed.checkpointId).not.toBe(fixture.identity['startupCheckpointId']);
  } finally { await persistence.close(); }
  const server = await startRustServer({ ...DEFAULT_CONFIG, host: '127.0.0.1', port: 0,
    resume: 'latest', dbPath: databasePath });
  try {
    expect(server.startupFault).toBeUndefined();
    expect(await health(server)).toMatchObject({ runId: fixture.identity['runId'], seed: 77,
      generation: fixture.identity['generation'] });
    const response = await fetch(`http://127.0.0.1:${server.port}/api/export/latest`);
    expect(response.status).toBe(200);
    const bytes = Buffer.from(await response.arrayBuffer());
    expect(bytes.byteLength).toBeLessThan(4 * 1024 * 1024);
    return bytes;
  } finally { await server.close(); }
}

/** Produce and independently re-import a small valid archive using the approved large-brain graph. */
async function largeBrainArchive(fixture: Fixture): Promise<Buffer> {
  const source = await startRustServer({ ...DEFAULT_CONFIG, host: '127.0.0.1', port: 0,
    resume: 'fresh', seed: 99, dbPath: join(dirname(fixture.databasePath), 'large-brain.sqlite') });
  let viewer: WebSocket | undefined;
  try {
    expect(source.startupFault).toBeUndefined();
    viewer = new WebSocket(`ws://127.0.0.1:${source.port}`);
    await slow(viewer);
    const reset = new Promise<void>((done, reject) => {
      viewer!.on('message', (bytes, binary) => {
        if (binary) return;
        const message = JSON.parse(bytes.toString()) as Record<string, unknown>;
        if (message['type'] === 'error') reject(new Error(String(message['message'])));
        if (message['type'] === 'stateReplaced' && message['reason'] === 'reset') done();
      });
      viewer!.once('error', reject);
    });
    viewer.send(JSON.stringify({ type: 'reset', graphSpec: buildLargeBrainGraph(147),
      settings: { snakeCount: 2, simSpeed: 0.1 }, updates: [
        { path: 'sense.bubbleBins', value: 32 }, { path: 'baselineBots.count', value: 0 }
      ] }));
    await bounded(reset, 'large-brain source did not complete reset');
    const exported = await fetch(`http://127.0.0.1:${source.port}/api/export/latest`);
    expect(exported.status).toBe(200);
    const bytes = Buffer.from(await exported.arrayBuffer());
    expect(bytes.byteLength).toBeLessThan(4 * 1024 * 1024);
    await noTransferScratch(`${join(dirname(fixture.databasePath), 'large-brain.sqlite')}.checkpoints`);
    const imported = await fetch(`http://127.0.0.1:${source.port}/api/import/archive`,
      { method: 'POST', body: new Uint8Array(bytes), signal: AbortSignal.timeout(5000) });
    expect(imported.status).toBe(200);
    expect(await imported.json()).toMatchObject({ ok: true, branched: false });
    return bytes;
  } finally { viewer?.terminate(); await source.close(); }
}

/** Prove cancellation or publication releases the world to complete further fixed steps. */
async function advancing(fixture: Fixture): Promise<void> {
  const initial = await health(fixture.server);
  const before = BigInt(`0x${initial['completedStep'] as string}`);
  const deadline = performance.now() + 5000;
  let completed = before;
  while (completed <= before && performance.now() < deadline) {
    await new Promise<void>(done => setTimeout(done, 10));
    const current = await health(fixture.server);
    expect(current['ok']).toBe(true);
    completed = BigInt(`0x${current['completedStep'] as string}`);
  }
  expect(completed).toBeGreaterThan(before);
}

/** Exchange raw bytes so Node's client does not normalize deliberately malformed framing. */
async function raw(fixture: Fixture, headers: string[], body: Buffer = Buffer.alloc(0), endInput = false): Promise<string> {
  return await new Promise<string>((done, reject) => {
    const socket = connect({ host: '127.0.0.1', port: fixture.server.port });
    fixture.sockets.add(socket);
    const chunks: Buffer[] = [];
    const timeout = setTimeout(() => { socket.destroy(); reject(new Error('raw request did not close')); }, 5000);
    socket.on('data', bytes => chunks.push(bytes));
    socket.once('error', error => { clearTimeout(timeout); reject(error); });
    socket.once('close', () => { clearTimeout(timeout); done(Buffer.concat(chunks).toString()); });
    socket.once('connect', () => {
      const bytes = Buffer.concat([
        Buffer.from(['POST /api/import/archive HTTP/1.1', 'Host: localhost', 'Connection: close',
          'Content-Type: application/vnd.slither-neuroevo.save', ...headers, '', ''].join('\r\n')), body
      ]);
      if (endInput) socket.end(bytes);
      else socket.write(bytes);
    });
  });
}

/** Submit one fixed client chunk and release all backpressure listeners after each outcome. */
async function writeChunk(client: ClientRequest, chunk: Buffer): Promise<void> {
  if (client.write(chunk)) return;
  await new Promise<void>((done, reject) => {
    /** Remove this write's listeners; previous chunks cannot accumulate error callbacks. */
    const cleanup = (): void => {
      client.off('drain', drained);
      client.off('error', failed);
      client.off('close', closed);
    };
    /** Resume only after the actual client write queue drains. */
    const drained = (): void => { cleanup(); done(); };
    /** Surface the actual socket failure while freeing the write's listeners. */
    const failed = (error: Error): void => { cleanup(); reject(error); };
    /** Termination must wake a producer waiting for drain. */
    const closed = (): void => failed(new Error('upload connection closed during a write'));
    client.once('drain', drained);
    client.once('error', failed);
    client.once('close', closed);
  });
}

/** Await a real boundary with a deadline that is cleared on every terminal outcome. */
async function bounded<T>(promise: Promise<T>, description: string): Promise<T> {
  let timeout: NodeJS.Timeout | undefined;
  try {
    return await Promise.race([promise, new Promise<never>((_done, reject) => {
      timeout = setTimeout(() => reject(new Error(description)), 5000);
    })]);
  } finally { if (timeout) clearTimeout(timeout); }
}

/** Release a held boundary on test timeout as well as normal completion, exactly once. */
function testCleanup(action: () => void): () => void {
  let cleaned = false;
  /** A delayed finally block cannot restore global methods belonging to the next test. */
  const cleanup = (): void => {
    if (cleaned) return;
    cleaned = true;
    action();
  };
  onTestFinished(cleanup);
  return cleanup;
}

/** Observe the actual server response closing before success, without changing HTTP dispatch. */
function observeDisconnect(server: Pick<RustServer, 'port'>, endpoint = '/api/import/archive'):
  { closed: Promise<boolean>; restore: () => void } {
  let disconnected!: (beforeFinish: boolean) => void;
  const closed = new Promise<boolean>(done => { disconnected = done; });
  /** Forward directly to Node, never through an earlier fixture's still-installed wrapper. */
  const dispatch = function(
    this: Server, event: string | symbol, ...args: unknown[]
  ): boolean {
    if (event === 'request') {
      const incoming = args[0] as IncomingMessage;
      const response = args[1] as ServerResponse;
      if (incoming.socket.localPort === server.port && incoming.url === endpoint) {
        response.once('close', () => disconnected(!response.writableFinished));
      }
    }
    return Reflect.apply(originalHttpEmit, this, [event, ...args]) as boolean;
  };
  Server.prototype.emit = dispatch;
  /** Late cleanup must not remove a newer fixture's observer after a test deadline. */
  const restore = (): void => {
    if (Server.prototype.emit === dispatch) Server.prototype.emit = originalHttpEmit;
  };
  return { closed, restore: testCleanup(restore) };
}

/** One actual controller's bounded protocol inbox and newest delivered observation. */
interface ControllerPeer {
  /** Real Protocol 2 socket. */
  socket: WebSocket;
  /** Latest Rust-issued lease assignment. */
  assignment?: AssignMsg;
  /** Latest delivered sample for that assignment. */
  sample?: SensorsMsg;
  /** Reliable lifecycle/error packets, excluding repetitive frames and statistics. */
  packets: Array<Record<string, unknown>>;
}

/** Poll an observable outcome without retaining a pending task after its deadline. */
async function outcome(predicate: () => boolean, description: string): Promise<void> {
  const deadline = performance.now() + 5000;
  while (!predicate() && performance.now() < deadline) await new Promise<void>(done => setTimeout(done, 10));
  expect(predicate(), description).toBe(true);
}

/** Acquire an actual player or trainer lease, retaining only the newest sensor sample. */
async function controller(fixture: Fixture, kind: 'ui' | 'bot'): Promise<ControllerPeer> {
  const socket = new WebSocket(`ws://127.0.0.1:${fixture.server.port}`);
  fixture.peers.add(socket);
  const peer: ControllerPeer = { socket, packets: [] };
  socket.on('error', error => peer.packets.push({ type: 'error', message: error.message }));
  socket.on('open', () => socket.send(JSON.stringify({ type: 'hello', version: 2, clientType: kind })));
  socket.on('message', (bytes, binary) => {
    if (binary) return;
    const packet = JSON.parse(bytes.toString()) as Record<string, unknown>;
    if (packet['type'] === 'welcome') socket.send(JSON.stringify({ type: 'join', mode: 'player', name: `Import-${kind}` }));
    else if (packet['type'] === 'assign') { peer.assignment = packet as unknown as AssignMsg; delete peer.sample; }
    else if (packet['type'] === 'stateReplaced') { delete peer.assignment; delete peer.sample; }
    else if (packet['type'] === 'sensors' && packet['snakeId'] === peer.assignment?.snakeId) {
      peer.sample = packet as unknown as SensorsMsg;
    }
    if (['assign', 'reclaimResult', 'stateReplaced', 'newRunResult', 'error'].includes(String(packet['type'])) && peer.packets.length < 128) {
      peer.packets.push(packet);
    }
  });
  await outcome(() => !!peer.assignment && !!peer.sample, `controller startup failed: ${kind}`);
  expect(peer.packets.filter(packet => packet['type'] === 'error')).toEqual([]);
  return peer;
}

/** Read delivered v3 heading while respecting the wrap at plus/minus pi. */
function headingChange(before: SensorsMsg, after: SensorsMsg): number {
  const first = Math.atan2(before.sensors[0]!, before.sensors[1]!);
  const last = Math.atan2(after.sensors[0]!, after.sensors[1]!);
  return Math.atan2(Math.sin(last - first), Math.cos(last - first));
}

/** Deliver held input through actual sockets, proving server receipt with ordered round trips. */
async function heldInput(peers: ControllerPeer[], replacePlayer = true): Promise<void> {
  await Promise.all(peers.map(async (peer, index) => {
    // Players may replace unsent input; trainers retain their one action per observation boundary.
    if (index === 0 && replacePlayer) peer.socket.send(JSON.stringify({ type: 'action',
      snakeId: peer.assignment!.snakeId, tick: peer.sample!.tick, turn: 1, boost: 0 }));
    peer.socket.send(JSON.stringify({ type: 'action',
      snakeId: peer.assignment!.snakeId, tick: peer.sample!.tick, turn: -1, boost: 0 }));
    const echo = new Promise<Buffer>(done => peer.socket.once('pong', bytes => done(bytes)));
    peer.socket.ping(`held-input-${index}`);
    expect((await bounded(echo, 'held input socket round trip failed')).toString()).toBe(`held-input-${index}`);
  }));
}

describeNetworkSuite('Rust archive HTTP framing', () => {
  it('keeps the newer HTTP disconnect observer when an older fixture cleans up late', async () => {
    let requests = 0;
    const server = createServer((_incoming, response) => {
      requests++;
      response.writeHead(400, { 'Connection': 'close' });
      response.end('fixture import rejected');
    });
    let previous: ReturnType<typeof observeDisconnect> | undefined;
    let current: ReturnType<typeof observeDisconnect> | undefined;
    try {
      await new Promise<void>((done, reject) => {
        server.once('error', reject);
        server.listen(0, '127.0.0.1', () => { server.off('error', reject); done(); });
      });
      const address = server.address();
      if (!address || typeof address === 'string') throw new Error('observer fixture has no bound port');
      previous = observeDisconnect({ port: address.port });
      current = observeDisconnect({ port: address.port });
      previous.restore();
      previous.restore();
      const response = await fetch(`http://127.0.0.1:${address.port}/api/import/archive`, {
        method: 'POST', body: 'invalid', signal: AbortSignal.timeout(5000)
      });
      expect(response.status).toBe(400);
      expect(await response.text()).toBe('fixture import rejected');
      expect(await bounded(current.closed, 'newer observer lost the actual HTTP close')).toBe(false);
      expect(requests).toBe(1);
    } finally {
      previous?.restore(); current?.restore();
      await new Promise<void>((done, reject) => server.close(error => error ? reject(error) : done()));
    }
    expect(Server.prototype.emit).toBe(originalHttpEmit);
  });

  it('delivers the whole resource rejection and closes a connected unfinished upload without spooling', async () => {
    await experiment(async fixture => {
      const admission = vi.spyOn(diskAdmission, 'admitDiskOperation')
        .mockImplementation(async (_directory, request) => diskAdmission.evaluateDiskAdmission(request,
          diskAdmission.ARCHIVE_TEMP_QUOTA_BYTES, 32n * 1024n ** 3n));
      const spool = vi.spyOn(archiveUpload, 'spoolArchiveUpload');
      try {
        const started = performance.now();
        const response = await raw(fixture, [`Content-Length: ${fixture.archive.byteLength}`],
          fixture.archive.subarray(0, 64 * 1024));
        expect(response).toMatch(/^HTTP\/1\.1 400 /u);
        expect(performance.now() - started).toBeLessThan(2500);
        const [headers, body] = response.split('\r\n\r\n');
        const length = Number(headers!.match(/Content-Length: (\d+)/iu)?.[1]);
        expect(length).toBeGreaterThan(0);
        expect(Buffer.byteLength(body!)).toBe(length);
        expect(JSON.parse(body!)).toMatchObject({ ok: false, message: expect.stringContaining('temporary bytes, above') });
        expect(admission).toHaveBeenCalledOnce();
        expect(spool).not.toHaveBeenCalled();
        await preserved(fixture);
        await advancing(fixture);
      } finally { admission.mockRestore(); spool.mockRestore(); }
    });
  }, 10_000);

  it.each(([
    ['import upload', 1], ['import preparation', 2], ['export', 1]
  ] as const).flatMap(([boundary, attempt]) =>
    (['temporary quota', 'SQLite/WAL allowance', 'operating reserve'] as const)
      .map(resource => ({ boundary, attempt, resource }))))(
    'preserves the game when $resource rejects $boundary', async ({ boundary, attempt, resource }) => {
      await experiment(async fixture => {
        const originalAdmit = diskAdmission.admitDiskOperation;
        const sourceHash = createHash('sha256').update(fixture.archive).digest('hex');
        const operation = boundary === 'export' ? 'export' : 'import';
        let seen = 0;
        let refusal: diskAdmission.DiskAdmissionError | undefined;
        const spool = vi.spyOn(archiveUpload, 'spoolArchiveUpload');
        const stage = vi.spyOn(BackgroundOutputPump.prototype, 'stagePreparedImport');
        const commit = vi.spyOn(CheckpointPersistenceClient.prototype, 'commitImport');
        const release = vi.spyOn(CheckpointPersistenceClient.prototype, 'releaseExportLease');
        /** Inject only scarce-resource readings; run the production admission arithmetic and transfer path. */
        const admission = vi.spyOn(diskAdmission, 'admitDiskOperation')
          .mockImplementation(async (directory, request) => {
            if (request.operation !== operation || ++seen !== attempt) return originalAdmit(directory, request);
            expect(directory).toBe(fixture.managedDirectory);
            const actual = await diskAdmission.inspectManagedDisk(directory);
            if (boundary === 'import preparation') {
              expect(actual.tempByteCount).toBe(BigInt(fixture.archive.byteLength));
              const upload = (await readdir(directory)).filter(name => name.endsWith('.upload.ready'));
              expect(upload).toHaveLength(1);
              const retained = await files(directory);
              expect(retained.find(file => file.filename === upload[0])?.sha256).toBe(sourceHash);
              expect(request.sourceSpoolBytes).toBe(0n);
              expect(request.candidateSpoolBytes + request.finalManagedBytes).toBeGreaterThan(0n);
            } else if (boundary === 'import upload') {
              expect(actual.tempByteCount).toBe(0n);
              expect(request.sourceSpoolBytes).toBe(BigInt(fixture.archive.byteLength));
            } else {
              expect(actual.tempByteCount).toBeGreaterThan(0n); // The real export inventory is already leased.
              expect(request.candidateSpoolBytes).toBeGreaterThan(0n);
            }
            const plannedTemp = request.sourceSpoolBytes + request.candidateSpoolBytes;
            const existingTemp = resource === 'temporary quota'
              ? diskAdmission.ARCHIVE_TEMP_QUOTA_BYTES - plannedTemp + 1n : actual.tempByteCount;
            const payloadBytes = existingTemp + plannedTemp + request.finalManagedBytes;
            const freeBytes = resource === 'temporary quota' ? 32n * 1024n ** 3n :
              payloadBytes + (resource === 'SQLite/WAL allowance'
                ? diskAdmission.OPERATING_DISK_RESERVE_BYTES : diskAdmission.SQLITE_WAL_ALLOWANCE_BYTES);
            try { return diskAdmission.evaluateDiskAdmission(request, existingTemp, freeBytes); }
            catch (error) {
              expect(error).toBeInstanceOf(diskAdmission.DiskAdmissionError);
              refusal = error as diskAdmission.DiskAdmissionError;
              throw error;
            }
          });
        try {
          for (let repetition = 0; repetition < (boundary === 'import upload' ? 12 : 1); repetition++) {
            seen = 0;
            const response = await fetch(`http://127.0.0.1:${fixture.server.port}/api/${operation === 'export' ? 'export/latest' : 'import/archive'}`,
              operation === 'export' ? { signal: AbortSignal.timeout(5000) } :
                { method: 'POST', body: new Uint8Array(fixture.archive), signal: AbortSignal.timeout(5000) })
              .catch(error => { throw new Error(`resource rejection response failed (${repetition})`, { cause: error }); });
            expect(response.status).toBe(operation === 'export' ? 500 : 400);
            expect(refusal).toBeDefined();
            expect(refusal?.code).toBe(resource === 'temporary quota' ? 'TEMP_QUOTA' : 'FREE_DISK');
            expect(await response.json()).toMatchObject({ ok: false, message: refusal!.message });
            await preserved(fixture);
          }
          expect(seen).toBe(attempt);
          expect(spool).toHaveBeenCalledTimes(boundary === 'import preparation' ? 1 : 0);
          expect(stage).not.toHaveBeenCalled();
          expect(commit).not.toHaveBeenCalled();
          await preserved(fixture);
          await advancing(fixture);
          expect(release).toHaveBeenCalledTimes(operation === 'export' ? 1 : 0);
          expect(createHash('sha256').update(fixture.archive).digest('hex')).toBe(sourceHash);
          admission.mockRestore();
          const retry = await fetch(`http://127.0.0.1:${fixture.server.port}/api/export/latest`)
            .catch(error => { throw new Error('export retry response failed', { cause: error }); });
          expect(retry.status).toBe(200);
          expect(Buffer.from(await retry.arrayBuffer())).toEqual(fixture.archive);
          await preserved(fixture);
        } finally {
          admission.mockRestore(); spool.mockRestore(); stage.mockRestore(); commit.mockRestore(); release.mockRestore();
        }
      });
    }, 20_000
  );

  it.runIf(process.platform === 'linux' && process.env['SLITHER_PRIVATE_QUOTA_TEST'] === '1').each([
    ['import upload', 'import', 1], ['import preparation', 'import', 2], ['export encoding', 'export', 1]
  ] as const)('preserves the game after real private-filesystem exhaustion during %s',
    async (_boundary, operation, attempt) => {
      await experiment(async fixture => {
        const peers = await Promise.all([controller(fixture, 'ui'), controller(fixture, 'bot')]);
        const baselines = peers.map(peer => ({ assignment: { ...peer.assignment! }, sample: peer.sample! }));
        const activity = ((await health(fixture.server))['telemetry'] as ExperimentalRuntimeTelemetrySnapshot).controllerActivity;
        const originalAdmit = diskAdmission.admitDiskOperation;
        let admitted = 0;
        let constrained = false;
        let beforeFree: bigint | undefined;
        const stage = vi.spyOn(BackgroundOutputPump.prototype, 'stagePreparedImport');
        const commit = vi.spyOn(CheckpointPersistenceClient.prototype, 'commitImport');
        /** Reduce actual filesystem capacity only after successful production admission. */
        const admission = vi.spyOn(diskAdmission, 'admitDiskOperation')
          .mockImplementation(async (directory, request) => {
            const decision = await originalAdmit(directory, request);
            if (request.operation === operation && ++admitted === attempt) {
              expect(directory).toBe(fixture.managedDirectory);
              const capacity = await statfs(directory, { bigint: true });
              beforeFree = capacity.bavail * capacity.bsize;
              expect(beforeFree).toBeGreaterThan(decision.requiredFreeBytes);
              await quota(directory, (capacity.blocks - capacity.bfree) * capacity.bsize);
              constrained = true;
              expect((await statfs(directory, { bigint: true })).bavail).toBe(0n);
              await heldInput(peers);
            }
            return decision;
          });
        try {
          const response = await fetch(`http://127.0.0.1:${fixture.server.port}/api/${operation === 'export'
            ? 'export/latest' : 'import/archive'}`, operation === 'export'
            ? { signal: AbortSignal.timeout(5000) }
            : { method: 'POST', body: new Uint8Array(fixture.archive), signal: AbortSignal.timeout(5000) });
          expect(response.status).toBe(operation === 'export' ? 500 : 400);
          const rejected = await response.json() as { ok: boolean; message: string };
          expect(rejected.ok).toBe(false);
          expect(rejected.message).toMatch(/ENOSPC|no space left on device|os error 28/iu);
          expect(constrained).toBe(true);
          expect(admitted).toBe(attempt);
          expect(stage).not.toHaveBeenCalled();
          expect(commit).not.toHaveBeenCalled();
          await quota(fixture.managedDirectory, 3n * 1024n ** 3n);
          await preserved(fixture);
          await advancing(fixture);
          await outcome(() => peers.every((peer, index) => peer.sample!.tick > baselines[index]!.sample.tick &&
            headingChange(baselines[index]!.sample, peer.sample!) < -0.01),
          'steering delivered during disk exhaustion did not reach the preserved world');
          for (const [index, peer] of peers.entries()) {
            expect(peer.socket.readyState).toBe(WebSocket.OPEN);
            expect(peer.assignment).toEqual(baselines[index]!.assignment);
            expect(peer.packets.filter(packet => ['stateReplaced', 'error'].includes(String(packet['type'])))).toEqual([]);
          }
          const applied = ((await health(fixture.server))['telemetry'] as ExperimentalRuntimeTelemetrySnapshot).controllerActivity;
          expect(applied.player.appliedActions).toBeGreaterThanOrEqual(activity.player.appliedActions + 1);
          expect(applied.trainer.appliedActions).toBe(activity.trainer.appliedActions + 1);
          admission.mockRestore();
          const retry = await fetch(`http://127.0.0.1:${fixture.server.port}/api/${operation === 'export'
            ? 'export/latest' : 'import/archive'}`, operation === 'export'
            ? { signal: AbortSignal.timeout(5000) }
            : { method: 'POST', body: new Uint8Array(fixture.archive), signal: AbortSignal.timeout(5000) });
          expect(retry.status).toBe(200);
          if (operation === 'export') {
            expect(Buffer.from(await retry.arrayBuffer())).toEqual(fixture.archive);
            await preserved(fixture);
          } else {
            expect(await retry.json()).toMatchObject({ ok: true, runId: fixture.identity['runId'] });
            await noTransferScratch(fixture.managedDirectory);
            expect(metadata(fixture.databasePath)).toEqual(
              (fixture.metadata as Array<{ name: string; rows: unknown[] }>).map(table =>
                table.name === 'rust_active_run_v1' ? { ...table,
                  rows: [{ singleton: 1, run_id: fixture.identity['runId'] }] } : table));
            expect(await files(fixture.managedDirectory)).toEqual(fixture.files);
            const replaced = await health(fixture.server);
            expect(replaced).toMatchObject({ ok: true, runId: fixture.identity['runId'],
              seed: fixture.identity['seed'], generation: fixture.identity['generation'] });
            expect(BigInt(`0x${replaced['worldEpoch'] as string}`))
              .toBeGreaterThan(BigInt(`0x${fixture.identity['worldEpoch'] as string}`));
          }
          await advancing(fixture);
        } finally {
          admission.mockRestore(); stage.mockRestore(); commit.mockRestore();
          if (constrained) await quota(fixture.managedDirectory, 3n * 1024n ** 3n);
        }
      }, true);
    }, 20_000);

  it('rejects independently valid import and reset exceeding native state memory before replacement', async () => {
    const ceiling = 1400n * 1024n ** 2n;
    const originalCreate = freshRunSessions.createExperimentalFreshRunSession;
    /** Change only the target's native admission budget; retain the production constructor. */
    const construction = vi.spyOn(freshRunSessions, 'createExperimentalFreshRunSession')
      .mockImplementation(options => originalCreate({ ...options,
        memoryCeilingBytes: options.seed === 42 ? ceiling : options.memoryCeilingBytes }));
    try {
      await experiment(async fixture => {
        const candidate = await largeBrainArchive(fixture);
        const candidateHash = createHash('sha256').update(candidate).digest('hex');
        const stage = vi.spyOn(BackgroundOutputPump.prototype, 'stagePreparedImport');
        const commit = vi.spyOn(CheckpointPersistenceClient.prototype, 'commitImport');
        try {
          const response = await fetch(`http://127.0.0.1:${fixture.server.port}/api/import/archive`,
            { method: 'POST', body: new Uint8Array(candidate), signal: AbortSignal.timeout(5000) });
          expect(response.status).toBe(400);
          const rejected = await response.json() as { ok: boolean; message: string };
          expect(rejected).toMatchObject({ ok: false });
          expect(rejected.message).toMatch(/checkpoint state admission failed: state requires an estimated \d+ bytes/iu);
          expect(rejected.message).toContain(`exceeding the ${ceiling}-byte ceiling`);
          expect(stage).not.toHaveBeenCalled();
          expect(commit).not.toHaveBeenCalled();
          await preserved(fixture);
          await advancing(fixture);
          expect(createHash('sha256').update(candidate).digest('hex')).toBe(candidateHash);
          const peer = await controller(fixture, 'ui');
          peer.socket.send(JSON.stringify({ type: 'reset', graphSpec: buildLargeBrainGraph(147),
            settings: { snakeCount: 2, simSpeed: 0.1 }, updates: [
              { path: 'sense.bubbleBins', value: 32 }, { path: 'baselineBots.count', value: 0 }
            ] }));
          await outcome(() => peer.packets.some(packet => packet['type'] === 'error' &&
            String(packet['message']).includes(`${ceiling}-byte ceiling`)),
          'oversized reset did not reject the retained native ceiling');
          expect(peer.packets.filter(packet => packet['type'] === 'stateReplaced')).toEqual([]);
          expect(stage).not.toHaveBeenCalled();
          expect(commit).not.toHaveBeenCalled();
          await preserved(fixture);
          await advancing(fixture);
          const retry = await fetch(`http://127.0.0.1:${fixture.server.port}/api/import/archive`,
            { method: 'POST', body: new Uint8Array(fixture.archive), signal: AbortSignal.timeout(5000) });
          expect(retry.status).toBe(200);
          expect(await retry.json()).toMatchObject({ ok: true, runId: fixture.identity['runId'] });
          await noTransferScratch(fixture.managedDirectory);
          expect(metadata(fixture.databasePath)).toEqual(
            (fixture.metadata as Array<{ name: string; rows: unknown[] }>).map(table =>
              table.name === 'rust_active_run_v1' ? { ...table,
                rows: [{ singleton: 1, run_id: fixture.identity['runId'] }] } : table));
          expect(await files(fixture.managedDirectory)).toEqual(fixture.files);
          const replaced = await health(fixture.server);
          expect(replaced).toMatchObject({ ok: true, runId: fixture.identity['runId'],
            seed: fixture.identity['seed'], generation: fixture.identity['generation'] });
          expect(BigInt(`0x${replaced['worldEpoch'] as string}`))
            .toBeGreaterThan(BigInt(`0x${fixture.identity['worldEpoch'] as string}`));
          await advancing(fixture);
        } finally { stage.mockRestore(); commit.mockRestore(); }
      });
    } finally { construction.mockRestore(); }
  }, 30_000);

  it.each(['export-hof-weights.partial', 'slither-save.partial', 'slither-save.ready'] as const)(
    'preserves an existing %s when actual export creation or publication fails', async suffix => {
      await experiment(async fixture => {
        const originalAcquire = CheckpointPersistenceClient.prototype.acquireCurrentExportLease;
        const originalRelease = CheckpointPersistenceClient.prototype.releaseExportLease;
        const earlierBytes = Buffer.from('pre-existing task evidence: preserve these exact bytes');
        let earlierPath: string | undefined;
        let released = false;
        /** Create the collision after the genuine lease, before native encoding starts. */
        const acquiring = vi.spyOn(CheckpointPersistenceClient.prototype, 'acquireCurrentExportLease')
          .mockImplementationOnce(async function(this: CheckpointPersistenceClient, ...args) {
            const lease = await originalAcquire.apply(this, args);
            earlierPath = join(fixture.managedDirectory, `.${lease.operationId}.${suffix}`);
            await writeFile(earlierPath, earlierBytes, { flag: 'wx' });
            return lease;
          });
        /** Join real cleanup before examining the collision and immutable source files. */
        const release = vi.spyOn(CheckpointPersistenceClient.prototype, 'releaseExportLease')
          .mockImplementation(async function(this: CheckpointPersistenceClient, ...args) {
            await originalRelease.apply(this, args);
            released = true;
          });
        try {
          const response = await fetch(`http://127.0.0.1:${fixture.server.port}/api/export/latest`,
            { signal: AbortSignal.timeout(5000) });
          expect(response.status).toBe(500);
          expect(response.headers.get('content-disposition')).toBeNull();
          expect(await response.json()).toMatchObject({ ok: false, message: expect.any(String) });
          const deadline = performance.now() + 5000;
          while (!released && performance.now() < deadline) await new Promise<void>(done => setTimeout(done, 10));
          expect(released).toBe(true);
          expect(release).toHaveBeenCalledOnce();
          expect(await readFile(earlierPath!)).toEqual(earlierBytes);
          await unlink(earlierPath!);
          earlierPath = undefined;
          await preserved(fixture);
          await advancing(fixture);
          acquiring.mockRestore();
          const retry = await fetch(`http://127.0.0.1:${fixture.server.port}/api/export/latest`);
          expect(retry.status).toBe(200);
          expect(Buffer.from(await retry.arrayBuffer())).toEqual(fixture.archive);
          await preserved(fixture);
        } finally {
          acquiring.mockRestore(); release.mockRestore();
          if (earlierPath) await unlink(earlierPath).catch(error => {
            if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error;
          });
        }
      });
    }, 20_000
  );

  it.each(['import', 'reset', 'newRun'] as const)(
    'preserves player and trainer leases and held input after a failed staged %s', async kind => {
    await experiment(async fixture => {
      const peers = await Promise.all([controller(fixture, 'ui'), controller(fixture, 'bot')]);
      let reached!: () => void;
      let release!: () => void;
      const staged = new Promise<void>(done => { reached = done; });
      const released = new Promise<void>(done => { release = done; });
      const originalStage = BackgroundOutputPump.prototype.stagePreparedImport;
      const stage = vi.spyOn(BackgroundOutputPump.prototype, 'stagePreparedImport')
        .mockImplementationOnce(async function(this: BackgroundOutputPump) {
          await originalStage.call(this);
          reached();
          await released;
        });
      const observation = observeDisconnect(fixture.server);
      const cleanup = testCleanup(() => {
        release();
        stage.mockRestore();
        observation.restore();
      });
      try {
        let client: ClientRequest | undefined;
        if (kind === 'import') {
          const body = legacyPopulation();
          client = request(`http://127.0.0.1:${fixture.server.port}/api/import/archive`, {
            method: 'POST', headers: { 'Content-Length': body.byteLength }
          });
          fixture.requests.add(client);
          client.on('error', () => {});
          client.end(body);
        } else {
          const database = new Database(fixture.databasePath);
          try {
            database.exec(`CREATE TRIGGER reject_replacement_activation
              BEFORE INSERT ON rust_active_run_v1 BEGIN
                SELECT RAISE(ABORT, 'injected controller preservation rejection');
              END`);
          } finally { database.close(); }
          peers[0]!.socket.send(JSON.stringify(kind === 'reset' ? { type: 'reset' } :
            { type: 'newRun', requestId: 'held-controller-rejection' }));
        }
        await bounded(staged, 'import never held the old world');
        const before = await health(fixture.server);
        const activity = (before['telemetry'] as ExperimentalRuntimeTelemetrySnapshot).controllerActivity;
        const baselines = peers.map(peer => ({ assignment: { ...peer.assignment! }, sample: peer.sample! }));
        await heldInput(peers);
        if (client) {
          client.destroy();
          expect(await bounded(observation.closed, 'server never observed disconnect')).toBe(true);
        }
        release();
        if (kind !== 'import') {
          await outcome(() => peers[0]!.packets.some(packet => kind === 'reset' ?
            packet['type'] === 'error' && String(packet['message']).includes('injected controller preservation rejection') :
            packet['type'] === 'newRunResult' && packet['applied'] === false &&
              String(packet['reason']).includes('injected controller preservation rejection')), 'replacement did not reject its actual transaction');
        }
        await preserved(fixture);
        await outcome(() => peers.every((peer, index) => peer.sample!.tick > baselines[index]!.sample.tick &&
          headingChange(baselines[index]!.sample, peer.sample!) < -0.01), 'held steering was not applied after cancellation');
        for (const [index, peer] of peers.entries()) {
          expect(peer.socket.readyState).toBe(WebSocket.OPEN);
          expect(peer.assignment).toEqual(baselines[index]!.assignment);
          expect(peer.packets.filter(packet => packet['type'] === 'stateReplaced')).toEqual([]);
          expect(peer.packets.filter(packet => packet['type'] === 'error')).toHaveLength(kind === 'reset' && index === 0 ? 1 : 0);
        }
        const after = await health(fixture.server);
        const applied = (after['telemetry'] as ExperimentalRuntimeTelemetrySnapshot).controllerActivity;
        expect(applied.player.appliedActions).toBe(activity.player.appliedActions + 1);
        expect(applied.trainer.appliedActions).toBe(activity.trainer.appliedActions + 1);
      } finally {
        cleanup();
      }
    });
  }, 15_000);

  it.each(['import', 'reset', 'newRun'] as const)(
    'discards held player/trainer input and old tokens after committed %s until explicit rejoin', async kind => {
    await experiment(async fixture => {
      const peers = await Promise.all([controller(fixture, 'ui'), controller(fixture, 'bot')]);
      const baselines = peers.map(peer => ({ assignment: { ...peer.assignment! }, sample: peer.sample! }));
      const activity = ((await health(fixture.server))['telemetry'] as ExperimentalRuntimeTelemetrySnapshot).controllerActivity;
      let reached!: () => void;
      let release!: () => void;
      const staged = new Promise<void>(done => { reached = done; });
      const released = new Promise<void>(done => { release = done; });
      const originalStage = BackgroundOutputPump.prototype.stagePreparedImport;
      const stage = vi.spyOn(BackgroundOutputPump.prototype, 'stagePreparedImport')
        .mockImplementationOnce(async function(this: BackgroundOutputPump) {
          await originalStage.call(this);
          reached();
          await released;
        });
      let response: Promise<Response> | undefined;
      try {
        if (kind === 'import') response = fetch(`http://127.0.0.1:${fixture.server.port}/api/import/archive`, {
          method: 'POST', body: new Uint8Array(legacyPopulation())
        });
        else peers[0]!.socket.send(JSON.stringify(kind === 'reset' ? { type: 'reset' } :
          { type: 'newRun', requestId: 'held-controller-publication' }));
        await bounded(staged, 'replacement never held the old world');
        await heldInput(peers);
        release();
        if (response) {
          const imported = await bounded(response, 'import never responded');
          expect(imported.status).toBe(200);
          expect(await imported.json()).toMatchObject({ ok: true });
        }
        await outcome(() => peers.every(peer => peer.packets.some(packet =>
          packet['type'] === 'stateReplaced' && packet['reason'] === kind)), 'replacement notice was not delivered');
        expect((await health(fixture.server))['worldEpoch']).not.toBe(fixture.identity['worldEpoch']);
        expect(await files(fixture.managedDirectory)).toEqual(expect.arrayContaining(fixture.files));
        for (const [index, peer] of peers.entries()) {
          expect(peer.socket.readyState).toBe(WebSocket.OPEN);
          expect(peer.assignment).toBeUndefined();
          expect(peer.sample).toBeUndefined();
          expect(peer.packets.filter(packet => packet['type'] === 'stateReplaced')).toHaveLength(1);
          peer.socket.send(JSON.stringify({ type: 'join', mode: 'player', name: `Import-${index === 0 ? 'ui' : 'bot'}`,
            resumeToken: baselines[index]!.assignment.resumeToken }));
          // A rejected old-token join supplies no lease, even if public snake IDs happen to repeat.
          peer.socket.send(JSON.stringify({ type: 'action', snakeId: baselines[index]!.assignment.snakeId,
            tick: baselines[index]!.sample.tick, turn: 1, boost: 1 }));
        }
        await outcome(() => peers.every(peer => peer.packets.some(packet =>
          packet['type'] === 'reclaimResult' && packet['reclaimed'] === false)), 'old tokens were not rejected');
        await advancing(fixture);
        const unassigned = ((await health(fixture.server))['telemetry'] as ExperimentalRuntimeTelemetrySnapshot).controllerActivity;
        expect(unassigned.player.appliedActions).toBe(activity.player.appliedActions);
        expect(unassigned.trainer.appliedActions).toBe(activity.trainer.appliedActions);
        for (const [index, peer] of peers.entries()) {
          expect(peer.assignment).toBeUndefined();
          peer.socket.send(JSON.stringify({ type: 'join', mode: 'player', name: `Import-${index === 0 ? 'ui' : 'bot'}` }));
        }
        await outcome(() => peers.every(peer => !!peer.assignment && !!peer.sample), 'explicit rejoin did not assign new leases');
        const samples = peers.map(peer => peer.sample!);
        for (const [index, peer] of peers.entries()) {
          expect(peer.assignment!.resumeToken).not.toBe(baselines[index]!.assignment.resumeToken);
          expect(peer.packets.filter(packet => packet['type'] === 'assign')).toHaveLength(2);
          expect(peer.packets.filter(packet => packet['type'] === 'error')).toEqual([]);
        }
        await heldInput(peers);
        await outcome(() => peers.every((peer, index) => peer.sample!.tick > samples[index]!.tick &&
          headingChange(samples[index]!, peer.sample!) < -0.01), 'rejoined controllers could not steer');
        const rejoined = ((await health(fixture.server))['telemetry'] as ExperimentalRuntimeTelemetrySnapshot).controllerActivity;
        // The new player input may be admitted separately because the world is running again.
        expect(rejoined.player.appliedActions).toBeGreaterThan(activity.player.appliedActions);
        expect(rejoined.trainer.appliedActions).toBe(activity.trainer.appliedActions + 1);
      } finally {
        release();
        stage.mockRestore();
        await response?.catch(() => {});
      }
    });
  }, 15_000);

  it.each(['after source selection', 'after disk admission'] as const)(
    'skips native export work when the client disconnects %s', async boundary => {
      await experiment(async fixture => {
        await evolvedArchiveFixture(fixture);
        const sourcePath = join(dirname(fixture.databasePath), 'user-original.save');
        await writeFile(sourcePath, fixture.archive, { flag: 'wx' });
        const peers = await Promise.all([controller(fixture, 'ui'), controller(fixture, 'bot')]);
        const before = await health(fixture.server);
        const activity = (before['telemetry'] as ExperimentalRuntimeTelemetrySnapshot).controllerActivity;
        const baselines = peers.map(peer => ({ assignment: { ...peer.assignment! }, sample: peer.sample! }));
        // Production startup has already validated this exact source-identified addon.
        const binding = createRequire(import.meta.url)(resolve('native/index.js')) as {
          ExperimentalRunningAuthority: { prototype: ExperimentalRunningAuthorityNativeHandle };
        };
        expect(binding.ExperimentalRunningAuthority).toBeTypeOf('function');
        const prepare = vi.spyOn(binding.ExperimentalRunningAuthority.prototype, 'prepareExportArchive');
        const reached = Promise.withResolvers<void>();
        const gate = Promise.withResolvers<void>();
        const originalAcquire = CheckpointPersistenceClient.prototype.acquireCurrentExportLease;
        const acquire = vi.spyOn(CheckpointPersistenceClient.prototype, 'acquireCurrentExportLease')
          .mockImplementationOnce(async function(this: CheckpointPersistenceClient) {
            const lease = await originalAcquire.call(this);
            if (boundary === 'after source selection') { reached.resolve(); await gate.promise; }
            return lease;
          });
        const originalAdmit = diskAdmission.admitDiskOperation;
        const admission = vi.spyOn(diskAdmission, 'admitDiskOperation').mockImplementationOnce(async (...args) => {
          const decision = await originalAdmit(...args);
          if (boundary === 'after disk admission') { reached.resolve(); await gate.promise; }
          return decision;
        });
        const release = vi.spyOn(CheckpointPersistenceClient.prototype, 'releaseExportLease');
        const observation = observeDisconnect(fixture.server, '/api/export/latest');
        const client = request(`http://127.0.0.1:${fixture.server.port}/api/export/latest`);
        fixture.requests.add(client);
        let receivedHeaders = false;
        client.on('error', () => { /* Deliberately cancelled before any response headers. */ });
        client.on('response', response => { receivedHeaders = true; response.resume(); });
        const cleanup = testCleanup(() => {
          gate.resolve(); client.destroy(); observation.restore();
          prepare.mockRestore(); acquire.mockRestore(); admission.mockRestore(); release.mockRestore();
        });
        try {
          client.end();
          await bounded(reached.promise, 'export did not reach its actual preparation boundary');
          await heldInput(peers, false);
          client.destroy();
          expect(await bounded(observation.closed, 'server did not observe early export cancellation')).toBe(true);
          gate.resolve();
          await outcome(() => release.mock.calls.length === 1, 'early cancellation did not release its source');
          await release.mock.results[0]!.value;
          expect(receivedHeaders).toBe(false);
          expect(prepare).not.toHaveBeenCalled();
          if (boundary === 'after source selection') expect(admission).not.toHaveBeenCalled();
          else expect(admission).toHaveBeenCalledOnce();
          await preserved(fixture);
          await outcome(() => peers.every((peer, index) => peer.sample!.tick > baselines[index]!.sample.tick &&
            headingChange(baselines[index]!.sample, peer.sample!) < -0.01), 'steering during early cancellation was lost');
          for (const [index, peer] of peers.entries()) {
            expect(peer.socket.readyState).toBe(WebSocket.OPEN);
            expect(peer.assignment).toEqual(baselines[index]!.assignment);
            expect(peer.packets.filter(packet => packet['type'] === 'assign')).toHaveLength(1);
            expect(peer.packets.filter(packet => ['error', 'stateReplaced'].includes(String(packet['type'])))).toEqual([]);
          }
          const after = await health(fixture.server);
          const applied = (after['telemetry'] as ExperimentalRuntimeTelemetrySnapshot).controllerActivity;
          expect(applied.player.appliedActions).toBe(activity.player.appliedActions + 1);
          expect(applied.trainer.appliedActions).toBe(activity.trainer.appliedActions + 1);
          expect(BigInt(`0x${after['completedStep'] as string}`)).toBeGreaterThan(BigInt(`0x${before['completedStep'] as string}`));
          expect((await readFile(sourcePath)).equals(fixture.archive)).toBe(true);
          await exportAfterDisconnect(fixture);
          await outcome(() => release.mock.calls.length === 2, 'retry did not release its source');
          await release.mock.results[1]!.value;
          expect(prepare).toHaveBeenCalledOnce();
          await preserved(fixture);
          expect((await readFile(sourcePath)).equals(fixture.archive)).toBe(true);
        } finally { cleanup(); }
      });
    }, 15_000
  );

  it.each(['during body delivery', 'while waiting for drain'] as const)(
    'cleans an evolved export cancelled %s and preserves live controllers', async boundary => {
      await experiment(async fixture => {
        await evolvedArchiveFixture(fixture, 'streaming');
        const sourcePath = join(dirname(fixture.databasePath), 'user-original.save');
        await writeFile(sourcePath, fixture.archive, { flag: 'wx' });
        const peers = await Promise.all([controller(fixture, 'ui'), controller(fixture, 'bot')]);
        const before = await health(fixture.server);
        const activity = (before['telemetry'] as ExperimentalRuntimeTelemetrySnapshot).controllerActivity;
        const baselines = peers.map(peer => ({ assignment: { ...peer.assignment! }, sample: peer.sample! }));
        let actualResponse: ServerResponse | undefined;
        let closedBeforeFinish: boolean | undefined;
        let clientResponse: IncomingMessage | undefined;
        let receivedBytes = 0;
        let reached!: () => void;
        const bodyReceived = new Promise<void>(done => { reached = done; });
        let serverClosed!: () => void;
        const disconnected = new Promise<void>(done => { serverClosed = done; });
        /** Observe this server's actual response; no writes, buffering or drain events are fabricated. */
        const dispatch = function(this: Server, event: string | symbol, ...args: unknown[]): boolean {
          if (event === 'request') {
            const incoming = args[0] as IncomingMessage;
            const response = args[1] as ServerResponse;
            if (incoming.socket.localPort === fixture.server.port && incoming.url === '/api/export/latest') {
              actualResponse = response;
              response.once('close', () => {
                closedBeforeFinish = !response.writableFinished;
                serverClosed();
              });
            }
          }
          return Reflect.apply(originalHttpEmit, this, [event, ...args]) as boolean;
        };
        Server.prototype.emit = dispatch;
        const release = vi.spyOn(CheckpointPersistenceClient.prototype, 'releaseExportLease');
        const client = request(`http://127.0.0.1:${fixture.server.port}/api/export/latest`);
        fixture.requests.add(client);
        client.on('error', () => { /* This test deliberately disconnects an unfinished response. */ });
        client.on('response', response => {
          clientResponse = response;
          response.on('error', () => { /* The deliberate cancellation may report ECONNRESET. */ });
          response.on('data', (bytes: Buffer) => {
            receivedBytes += bytes.byteLength;
            response.pause();
            reached();
          });
        });
        const cleanup = testCleanup(() => {
          client.destroy();
          clientResponse?.destroy();
          release.mockRestore();
          if (Server.prototype.emit === dispatch) Server.prototype.emit = originalHttpEmit;
        });
        try {
          client.end();
          await bounded(bodyReceived, 'download did not deliver body bytes');
          expect(clientResponse!.statusCode).toBe(200);
          expect(receivedBytes).toBeGreaterThan(0);
          expect(receivedBytes).toBeLessThan(fixture.archive.byteLength);
          expect(actualResponse!.writableFinished).toBe(false);
          if (boundary === 'while waiting for drain') {
            await outcome(() => actualResponse!.writableNeedDrain && !actualResponse!.writableFinished,
              'real download did not wait for socket drain');
          }
          await heldInput(peers, false);
          expect(actualResponse!.writableFinished).toBe(false);
          if (boundary === 'while waiting for drain') {
            expect(actualResponse!.writableNeedDrain).toBe(true);
            expect(actualResponse!.writableLength).toBeGreaterThan(0);
          }
          clientResponse!.destroy();
          client.destroy();
          await bounded(disconnected, 'server did not observe mid-body cancellation');
          expect(closedBeforeFinish).toBe(true);
          await noTransferScratch(fixture.managedDirectory);
          await outcome(() => release.mock.calls.length === 1, 'cancelled download did not release its lease');
          await release.mock.results[0]!.value;
          await preserved(fixture);
          await outcome(() => peers.every((peer, index) => peer.sample!.tick > baselines[index]!.sample.tick &&
            headingChange(baselines[index]!.sample, peer.sample!) < -0.01), 'steering during cancelled export was lost');
          for (const [index, peer] of peers.entries()) {
            expect(peer.socket.readyState).toBe(WebSocket.OPEN);
            expect(peer.assignment).toEqual(baselines[index]!.assignment);
            expect(peer.packets.filter(packet => packet['type'] === 'assign')).toHaveLength(1);
            expect(peer.packets.filter(packet => ['error', 'stateReplaced'].includes(String(packet['type'])))).toEqual([]);
          }
          const after = await health(fixture.server);
          const applied = (after['telemetry'] as ExperimentalRuntimeTelemetrySnapshot).controllerActivity;
          expect(applied.player.appliedActions).toBe(activity.player.appliedActions + 1);
          expect(applied.trainer.appliedActions).toBe(activity.trainer.appliedActions + 1);
          expect(BigInt(`0x${after['completedStep'] as string}`)).toBeGreaterThan(BigInt(`0x${before['completedStep'] as string}`));
          expect((await readFile(sourcePath)).equals(fixture.archive)).toBe(true);
          await exportAfterDisconnect(fixture);
          await outcome(() => release.mock.calls.length === 2, 'retry download did not release its lease');
          expect(release).toHaveBeenCalledTimes(2);
          await release.mock.results[1]!.value;
          await preserved(fixture);
          expect((await readFile(sourcePath)).equals(fixture.archive)).toBe(true);
        } finally { cleanup(); }
      });
    }, 20_000
  );

  it.each([
    ['incompatible Concat input ordering', 'GRAPH_IDENTITY'],
    ['missing population record', 'INDEX_LENGTH'],
    ['missing dense population slot', 'INDEX_DENSE']
  ] as const)('rejects a correctly hashed save with %s before replacement', async (fault, code) => {
    await experiment(async fixture => {
      await evolvedArchiveFixture(fixture);
      const entries = archiveEntries(fixture.archive);
      const graph = compileGraph(orderingGraph());
      const reversed = compileGraph(orderingGraph(true));
      expect(reversed.totalParams).toBe(graph.totalParams);
      expect(reversed.key).not.toBe(graph.key);
      const roleIndex = code === 'GRAPH_IDENTITY' ? 1 : 2;
      const entry = entries[roleIndex]!;
      let role: Buffer = Buffer.from(fixture.archive.subarray(entry.data, entry.data + entry.size));
      if (code === 'GRAPH_IDENTITY') role = reversedConcatRole(role);
      else {
        const recordBytes = role.readUInt32LE(12);
        expect(recordBytes).toBe(104);
        expect(role.readBigUInt64LE(16)).toBe(12n);
        expect(role.byteLength).toBe(40 + 12 * recordBytes);
        if (code === 'INDEX_LENGTH') role = role.subarray(0, role.byteLength - recordBytes);
        else role.writeUInt32LE(0, 40 + recordBytes);
      }
      const damaged = semanticArchive(fixture.archive, roleIndex, role);
      const sourcePath = join(dirname(fixture.databasePath), 'user-original.save');
      await writeFile(sourcePath, fixture.archive, { flag: 'wx' });
      const sourceHash = createHash('sha256').update(fixture.archive).digest('hex');
      const peers = await Promise.all([controller(fixture, 'ui'), controller(fixture, 'bot')]);
      const before = await health(fixture.server);
      const activity = (before['telemetry'] as ExperimentalRuntimeTelemetrySnapshot).controllerActivity;
      const baselines = peers.map(peer => ({ assignment: { ...peer.assignment! }, sample: peer.sample! }));
      const stage = vi.spyOn(BackgroundOutputPump.prototype, 'stagePreparedImport');
      const commit = vi.spyOn(CheckpointPersistenceClient.prototype, 'commitImport');
      const originalSpool = archiveUpload.spoolArchiveUpload;
      const spool = vi.spyOn(archiveUpload, 'spoolArchiveUpload').mockImplementationOnce(async options => {
        const result = await originalSpool(options);
        await heldInput(peers, false);
        return result;
      });
      const cleanup = testCleanup(() => { spool.mockRestore(); stage.mockRestore(); commit.mockRestore(); });
      try {
        const response = await fetch(`http://127.0.0.1:${fixture.server.port}/api/import/archive`, {
          method: 'POST', body: new Uint8Array(damaged), signal: AbortSignal.timeout(5000) });
        expect(response.status).toBe(400);
        expect(await response.json(), fault).toMatchObject({ ok: false,
          message: expect.stringContaining(`checkpoint ${code}:`) });
        expect(spool).toHaveBeenCalledOnce();
        expect(stage).not.toHaveBeenCalled();
        expect(commit).not.toHaveBeenCalled();
        await preserved(fixture);
        await outcome(() => peers.every((peer, index) => peer.sample!.tick > baselines[index]!.sample.tick &&
          headingChange(baselines[index]!.sample, peer.sample!) < -0.01), 'steering during invalid import was lost');
        for (const [index, peer] of peers.entries()) {
          expect(peer.socket.readyState).toBe(WebSocket.OPEN);
          expect(peer.assignment).toEqual(baselines[index]!.assignment);
          expect(peer.packets.filter(packet => packet['type'] === 'assign')).toHaveLength(1);
          expect(peer.packets.filter(packet => ['error', 'stateReplaced'].includes(String(packet['type'])))).toEqual([]);
        }
        const after = await health(fixture.server);
        const applied = (after['telemetry'] as ExperimentalRuntimeTelemetrySnapshot).controllerActivity;
        expect(applied.player.appliedActions).toBe(activity.player.appliedActions + 1);
        expect(applied.trainer.appliedActions).toBe(activity.trainer.appliedActions + 1);
        expect(BigInt(`0x${after['completedStep'] as string}`)).toBeGreaterThan(BigInt(`0x${before['completedStep'] as string}`));
        expect(createHash('sha256').update(await readFile(sourcePath)).digest('hex')).toBe(sourceHash);
        expect(createHash('sha256').update(fixture.archive).digest('hex')).toBe(sourceHash);
        await exportAfterDisconnect(fixture);
        await preserved(fixture);
      } finally { cleanup(); }
    });
  }, 15_000);

  it.each(['nonzero role padding', 'nonzero second end block', 'hidden header name bytes',
    'continuous-file entry', 'symbolic-link entry', 'device entry', 'sparse entry', 'PAX entry',
    'duplicate role path', 'parent traversal path', 'missing role', 'unsupported save version',
    'corrupted logical root', 'false decoded byte count', 'corrupted graph entry'] as const)(
    'rejects %s in the uploaded save container before replacement', async fault => {
      await experiment(async fixture => {
        const entries = archiveEntries(fixture.archive);
        let damaged = Buffer.from(fixture.archive);
        if (fault === 'nonzero role padding') {
          const entry = entries.find(value => value.size % 512 !== 0)!;
          expect(entry).toBeDefined();
          damaged[entry.data + entry.size] = 1;
        } else if (fault === 'nonzero second end block') damaged[damaged.byteLength - 1] = 1;
        else if (fault === 'missing role') damaged = Buffer.concat([damaged.subarray(entries[0]!.end)]);
        else if (fault === 'corrupted graph entry') damaged[entries[1]!.data] = damaged[entries[1]!.data]! ^ 1;
        else if (fault === 'unsupported save version' || fault === 'corrupted logical root' ||
            fault === 'false decoded byte count') {
          const entry = entries[8]!;
          const manifest = JSON.parse(damaged.subarray(entry.data, entry.data + entry.size).toString()) as {
            archiveVersion: number; logicalRootSha256: string; roles: Array<{ decodedBytesHex: string }>;
          };
          if (fault === 'unsupported save version') manifest.archiveVersion = 2;
          else if (fault === 'corrupted logical root') {
            manifest.logicalRootSha256 = (manifest.logicalRootSha256.startsWith('0') ? '1' : '0') +
              manifest.logicalRootSha256.slice(1);
          } else manifest.roles[3]!.decodedBytesHex = 'ffffffffffffffff';
          const bytes = Buffer.from(JSON.stringify(manifest));
          expect(bytes.byteLength).toBe(entry.size);
          bytes.copy(damaged, entry.data);
        }
        else {
          const header = fault === 'duplicate role path' ? entries[1]!.header : entries[0]!.header;
          if (fault === 'hidden header name bytes') {
            const nul = damaged.indexOf(0, header);
            expect(nul).toBeLessThan(header + 99);
            damaged[nul + 1] = 0x78;
          } else if (fault === 'parent traversal path' || fault === 'duplicate role path') {
            const name = fault === 'parent traversal path' ? '../outside'
              : damaged.subarray(0, 100).toString().split('\0')[0]!;
            damaged.fill(0, header, header + 100);
            damaged.write(name, header, 'ascii');
          } else {
            const type = { 'continuous-file entry': '7', 'symbolic-link entry': '2',
              'device entry': '3', 'sparse entry': 'S', 'PAX entry': 'x' }[fault];
            damaged[header + 156] = type.charCodeAt(0);
          }
          archiveHeaderChecksum(damaged, header);
        }
        const originalHash = createHash('sha256').update(fixture.archive).digest('hex');
        const response = await fetch(`http://127.0.0.1:${fixture.server.port}/api/import/archive`, {
          method: 'POST', headers: { 'Content-Type': 'application/vnd.slither-neuroevo.save' }, body: damaged
        });
        expect(response.status, await response.text()).toBe(400);
        await preserved(fixture);
        expect(createHash('sha256').update(fixture.archive).digest('hex')).toBe(originalHash);
      });
    }
  );

  it.each([
    ['oversized decoder window', 'ZSTD_WINDOW'],
    ['hidden-size expansion stream', 'ZSTD_CONTENT_SIZE'],
    ['false frame output size', 'ZSTD_CONTENT_SIZE'],
    ['dictionary-dependent frame', 'ZSTD_FRAME'],
    ['oversized shuffled block', 'SHUFFLED_LIMIT'],
    ['oversized compressed block', 'SHUFFLED_LIMIT'],
    ['truncated frame header', 'ZSTD_FRAME'],
    ['extra compressed frame', 'ZSTD_FRAME'],
    ['changed decoded bits', 'LOGICAL_ROLE_SHA256']
  ] as const)('rejects %s inside a compressed population before replacement', async (fault, code) => {
    await experiment(async fixture => {
      const entries = archiveEntries(fixture.archive);
      const entry = entries[3]!;
      const manifest = JSON.parse(fixture.archive.subarray(entries[8]!.data,
        entries[8]!.data + entries[8]!.size).toString()) as { roles: Array<{ encoding: string }> };
      expect(manifest.roles[3]!.encoding).toBe('f32le-shuffle4-zstd-v1');
      expect(fixture.archive.subarray(entry.data, entry.data + 4).toString()).toBe('SFZ1');
      const floats = fixture.archive.readUInt32LE(entry.data + 4);
      const size = fixture.archive.readUInt32LE(entry.data + 8);
      expect(floats * 4).toBe(1024 * 1024);
      const originalFrame = fixture.archive.subarray(entry.data + 12, entry.data + 12 + size);
      const damaged = Buffer.from(fixture.archive);
      let replacement: Buffer | undefined;
      if (fault === 'oversized shuffled block') damaged.writeUInt32LE(floats + 1, entry.data + 4);
      else if (fault === 'oversized compressed block') damaged.writeUInt32LE(0xffff_ffff, entry.data + 8);
      else if (fault === 'truncated frame header') replacement = originalFrame.subarray(0, 3);
      else if (fault === 'extra compressed frame') replacement = Buffer.concat([
        originalFrame, zstdCompressSync(Buffer.alloc(4))
      ]);
      else if (fault === 'changed decoded bits') replacement = alteredDecodedFrame(originalFrame, floats * 4);
      else if (fault === 'hidden-size expansion stream') {
        replacement = expansionFrame();
        expect(replacement.byteLength).toBe(518);
        expect(await expansionBytes(replacement)).toBe(16 * 1024 * 1024);
      } else {
        const dictionary = fault === 'dictionary-dependent frame';
        replacement = Buffer.alloc(dictionary ? 11 : 10);
        Buffer.from([0x28, 0xb5, 0x2f, 0xfd, dictionary ? 0x81 : 0x80,
          fault === 'oversized decoder window' ? 0x58 : 0x50]).copy(replacement);
        if (dictionary) replacement[6] = 1;
        replacement.writeUInt32LE(floats * 4 + (fault === 'false frame output size' ? 1 : 0), dictionary ? 7 : 6);
      }
      if (replacement) {
        expect(replacement.byteLength + 12).toBeLessThan(entry.size);
        damaged.writeUInt32LE(replacement.byteLength, entry.data + 8);
        replacement.copy(damaged, entry.data + 12);
      }
      const sourceHash = createHash('sha256').update(fixture.archive).digest('hex');
      const stage = vi.spyOn(BackgroundOutputPump.prototype, 'stagePreparedImport');
      const commit = vi.spyOn(CheckpointPersistenceClient.prototype, 'commitImport');
      try {
        const response = await fetch(`http://127.0.0.1:${fixture.server.port}/api/import/archive`, {
          method: 'POST', body: damaged, signal: AbortSignal.timeout(5000) });
        expect(response.status).toBe(400);
        expect(await response.json()).toMatchObject({ ok: false, message: expect.stringContaining(`checkpoint ${code}:`) });
        expect(stage).not.toHaveBeenCalled();
        expect(commit).not.toHaveBeenCalled();
        await preserved(fixture);
        await advancing(fixture);
        expect(createHash('sha256').update(fixture.archive).digest('hex')).toBe(sourceHash);
      } finally { stage.mockRestore(); commit.mockRestore(); }
    });
  });

  it('rejects a valid same-run same-generation archive with different immutable content', async () => {
    await experiment(async fixture => {
      const body = await conflictingArchive(fixture);
      const originalHash = createHash('sha256').update(body).digest('hex');
      const response = await fetch(`http://127.0.0.1:${fixture.server.port}/api/import/archive`, {
        method: 'POST', body: new Uint8Array(body), signal: AbortSignal.timeout(5000) });
      expect(response.status, await response.clone().text()).toBe(400);
      expect(await response.json()).toMatchObject({ ok: false, message: expect.stringContaining('generation identity conflicts') });
      await preserved(fixture);
      await advancing(fixture);
      expect(createHash('sha256').update(body).digest('hex')).toBe(originalHash);
    });
  });

  it.each(['afterSpool', 'afterPreparation', 'afterStage', 'beforeCommit'] as const)(
    'preserves the experiment when the import client disconnects %s', async boundary => {
      await experiment(async fixture => {
        let reached!: () => void;
        let release!: () => void;
        const paused = new Promise<void>(done => { reached = done; });
        const released = new Promise<void>(done => { release = done; });
        /** Hold the actual completed boundary until the server observes the client disconnect. */
        const hold = async (): Promise<void> => { reached(); await released; };
        const originalSpool = archiveUpload.spoolArchiveUpload;
        const originalStage = BackgroundOutputPump.prototype.stagePreparedImport;
        const originalSelect = CheckpointPersistenceClient.prototype.selectStartup;
        const boundarySpy = boundary === 'afterSpool'
          ? vi.spyOn(archiveUpload, 'spoolArchiveUpload').mockImplementationOnce(async options => {
            const result = await originalSpool(options);
            await hold();
            return result;
          })
          : boundary === 'beforeCommit'
            ? vi.spyOn(CheckpointPersistenceClient.prototype, 'selectStartup')
              .mockImplementationOnce(async function(this: CheckpointPersistenceClient, ...args) {
                const result = await originalSelect.apply(this, args);
                await hold();
                return result;
              })
            : vi.spyOn(BackgroundOutputPump.prototype, 'stagePreparedImport')
            .mockImplementationOnce(async function(this: BackgroundOutputPump) {
              if (boundary === 'afterPreparation') await hold();
              await originalStage.call(this);
              if (boundary === 'afterStage') await hold();
            });
        const observation = observeDisconnect(fixture.server);
        const cleanup = testCleanup(() => {
          release();
          boundarySpy.mockRestore();
          observation.restore();
        });
        const body = legacyPopulation();
        const sourceHash = createHash('sha256').update(body).digest('hex');
        try {
          const client = request(`http://127.0.0.1:${fixture.server.port}/api/import/archive`, {
            method: 'POST', headers: { 'Content-Length': body.byteLength }
          });
          fixture.requests.add(client);
          client.on('error', () => { /* The deliberate disconnect may reset the client socket. */ });
          client.end(body);
          await bounded(paused, `import never reached ${boundary}`);
          if (boundary !== 'afterSpool') {
            const checkpoints = (await readdir(fixture.managedDirectory)).filter(name => name.endsWith('.checkpoint-v3'));
            expect(checkpoints.length).toBeGreaterThan(
              fixture.files.filter(file => file.filename.endsWith('.checkpoint-v3')).length);
          }
          client.destroy();
          expect(await bounded(observation.closed, 'server never observed disconnect')).toBe(true);
          release();
          await preserved(fixture);
          expect(createHash('sha256').update(body).digest('hex')).toBe(sourceHash);
          await advancing(fixture);
        } finally {
          cleanup();
        }
      });
    }
  );

  it.each(['afterCommit', 'afterSwap'] as const)(
    'finishes an import disconnected %s and resumes that same checkpoint on restart', async boundary => {
    await experiment(async fixture => {
      let reached!: () => void;
      let release!: () => void;
      const committed = new Promise<void>(done => { reached = done; });
      const released = new Promise<void>(done => { release = done; });
      const originalCommit = CheckpointPersistenceClient.prototype.commitImport;
      const originalPublish = BackgroundOutputPump.prototype.publishPreparedImport;
      let durable: Awaited<ReturnType<typeof originalCommit>> | undefined;
      const commit = vi.spyOn(CheckpointPersistenceClient.prototype, 'commitImport')
        .mockImplementationOnce(async function(this: CheckpointPersistenceClient, ...args) {
          durable = await originalCommit.apply(this, args);
          if (boundary === 'afterCommit') { reached(); await released; }
          return durable;
        });
      const publish = boundary === 'afterSwap'
        ? vi.spyOn(BackgroundOutputPump.prototype, 'publishPreparedImport')
          .mockImplementationOnce(async function(this: BackgroundOutputPump, ...args) {
            const result = await originalPublish.apply(this, args);
            reached();
            await released;
            return result;
          })
        : undefined;
      const observation = observeDisconnect(fixture.server);
      const cleanup = testCleanup(() => {
        release();
        commit.mockRestore();
        publish?.mockRestore();
        observation.restore();
      });
      // A supported legacy population gives this import a new run and a distinct durable pointer.
      const body = legacyPopulation();
      const sourceHash = createHash('sha256').update(body).digest('hex');
      try {
        const client = request(`http://127.0.0.1:${fixture.server.port}/api/import/archive`, {
          method: 'POST', headers: { 'Content-Length': body.byteLength, 'Content-Type': 'application/json' }
        });
        fixture.requests.add(client);
        client.on('error', () => { /* The deliberate disconnect may reset the client socket. */ });
        client.end(body);
        await bounded(committed, `import never reached ${boundary}`);
        expect(durable!.descriptor.runId).not.toBe(fixture.identity['runId']);
        client.destroy();
        expect(await bounded(observation.closed, 'server never observed disconnect')).toBe(true);
        release();
        await noTransferScratch(fixture.managedDirectory);
        const published = await health(fixture.server);
        expect(published).toMatchObject({ ok: true, runId: durable!.descriptor.runId,
          startupCheckpointId: durable!.checkpointId });
        expect(BigInt(`0x${published['worldEpoch'] as string}`))
          .toBeGreaterThan(BigInt(`0x${fixture.identity['worldEpoch'] as string}`));
        const database = new Database(fixture.databasePath, { readonly: true });
        try {
          expect(database.prepare(`SELECT active.run_id, current.checkpoint_id FROM rust_active_run_v1 AS active
            JOIN rust_checkpoint_v3_current AS current ON current.run_id = active.run_id`).get())
            .toEqual({ run_id: durable!.descriptor.runId, checkpoint_id: durable!.checkpointId });
        } finally { database.close(); }
        expect(await files(fixture.managedDirectory)).toEqual(expect.arrayContaining(fixture.files));
        expect(createHash('sha256').update(body).digest('hex')).toBe(sourceHash);
        await advancing(fixture);
        await fixture.server.close();
        fixture.server = await startRustServer({ ...DEFAULT_CONFIG, host: '127.0.0.1', port: 0,
          resume: 'latest', dbPath: fixture.databasePath });
        expect(fixture.server.startupFault).toBeUndefined();
        expect(await health(fixture.server)).toMatchObject({ ok: true, runId: durable!.descriptor.runId,
          startupCheckpointId: durable!.checkpointId });
        const exported = await fetch(`http://127.0.0.1:${fixture.server.port}/api/export/latest`);
        expect(exported.status).toBe(200);
        expect(exported.headers.get('x-slither-checkpoint-id')).toBe(durable!.checkpointId);
        expect((await exported.arrayBuffer()).byteLength).toBeGreaterThan(1024);
        await noTransferScratch(fixture.managedDirectory);
      } finally {
        cleanup();
      }
    });
  });

  it('cancels an upload disconnected during disk admission before opening its spool', async () => {
    await experiment(async fixture => {
      const entered = Promise.withResolvers<void>();
      const resume = Promise.withResolvers<void>();
      const originalAdmit = diskAdmission.admitDiskOperation;
      /** Hold actual admission after its filesystem reading until the peer closes. */
      const admission = vi.spyOn(diskAdmission, 'admitDiskOperation').mockImplementation(async (directory, operation) => {
        const result = await originalAdmit(directory, operation);
        if (operation.operation === 'import') {
          entered.resolve();
          await resume.promise;
        }
        return result;
      });
      const spool = vi.spyOn(archiveUpload, 'spoolArchiveUpload');
      const preparation = vi.spyOn(BackgroundOutputPump.prototype, 'stagePreparedImport');
      const commit = vi.spyOn(CheckpointPersistenceClient.prototype, 'commitImport');
      const disconnected = raw(fixture, ['Content-Length: 100'], Buffer.from('incomplete'), true);
      try {
        await bounded(entered.promise, 'upload did not reach disk admission');
        expect(await disconnected).not.toContain('200 OK');
        const busy = await fetch(`http://127.0.0.1:${fixture.server.port}/api/export/latest`);
        expect(busy.status).toBe(409);
        await busy.arrayBuffer();
        resume.resolve();
        await exportAfterDisconnect(fixture);
        expect(spool).not.toHaveBeenCalled();
        expect(preparation).not.toHaveBeenCalled();
        expect(commit).not.toHaveBeenCalled();
        await preserved(fixture);
        await advancing(fixture);
      } finally {
        resume.resolve();
        try { await disconnected; }
        finally { admission.mockRestore(); spool.mockRestore(); preparation.mockRestore(); commit.mockRestore(); }
      }
    });
  }, 20_000);

  it.each([
    ['over-limit declared length', [`Content-Length: ${P0_ARCHIVE_UPLOAD_LIMIT + 1n}`], Buffer.alloc(0), false],
    ['noncanonical declared length', ['Content-Length: 01'], Buffer.from('x'), false],
    ['conflicting length and chunked framing', ['Content-Length: 1', 'Transfer-Encoding: chunked'], Buffer.from('0\r\n\r\n'), false],
    ['empty unframed upload', [], Buffer.alloc(0), false],
    ['truncated declared body', ['Content-Length: 100'], Buffer.from('incomplete'), true]
  ] as const)('preserves the experiment after %s', async (_name, headers, body, endInput) => {
    await experiment(async fixture => {
      const response = await raw(fixture, [...headers], body, endInput);
      expect(response).not.toContain('200 OK');
      if (!endInput) expect(response).toMatch(/^HTTP\/1\.1 400 /u);
      await exportAfterDisconnect(fixture);
      await preserved(fixture);
    });
  }, 20_000);

  it('rejects a valid archive followed by an extra byte beyond Content-Length before replacement', async () => {
    await experiment(async fixture => {
      const response = await raw(fixture, [`Content-Length: ${fixture.archive.byteLength}`],
        Buffer.concat([fixture.archive, Buffer.from('x')]));
      expect(response).not.toContain('200 OK');
      await preserved(fixture);
    });
  });

  it('imports a valid bounded chunked save without Content-Length', async () => {
    await experiment(async fixture => {
      const response = await fetch(`http://127.0.0.1:${fixture.server.port}/api/import/archive`, {
        method: 'POST', headers: { 'Content-Type': 'application/vnd.slither-neuroevo.save' },
        body: (async function* () { yield fixture.archive; })() as unknown as BodyInit,
        duplex: 'half'
      } as RequestInit & { duplex: 'half' });
      expect(response.status).toBe(200);
      expect(await response.json()).toMatchObject({ ok: true, runId: fixture.identity['runId'] });
      await noTransferScratch(fixture.managedDirectory);
      expect(await files(fixture.managedDirectory)).toEqual(fixture.files);
      expect(await health(fixture.server)).toMatchObject({ ok: true, runId: fixture.identity['runId'] });
    });
  });

  it.runIf(process.env['SLITHER_FULL_UPLOAD_LIMIT_TEST'] === '1')(
    'rejects a chunked body crossing the actual four-GiB wire limit and removes its spool', async () => {
      await experiment(async fixture => {
        const space = await statfs(fixture.managedDirectory, { bigint: true });
        expect(space.bavail * space.bsize).toBeGreaterThan(12n * 1024n ** 3n);
        expect(P0_ARCHIVE_UPLOAD_LIMIT).toBe(4n * 1024n ** 3n);
        const client = request(`http://127.0.0.1:${fixture.server.port}/api/import/archive`, {
          method: 'POST', headers: { 'Content-Type': 'application/vnd.slither-neuroevo.save',
            'Transfer-Encoding': 'chunked', 'Connection': 'close' }
        });
        fixture.requests.add(client);
        const terminal = new Promise<{ status: number; body: string }>((done, reject) => {
          client.once('error', reject);
          client.once('response', response => {
            const chunks: Buffer[] = [];
            response.on('data', bytes => chunks.push(bytes as Buffer));
            response.once('error', reject);
            response.once('end', () => done({ status: response.statusCode ?? 0,
              body: Buffer.concat(chunks).toString() }));
          });
        });
        // Observe a terminal rejection immediately, including while backpressure holds the writer.
        void terminal.catch(() => {});
        const chunk = Buffer.alloc(1024 * 1024, 'x');
        const before = await health(fixture.server);
        const started = performance.now();
        let maximumSpoolBytes = 0;
        for (let written = 0n; written <= P0_ARCHIVE_UPLOAD_LIMIT; written += BigInt(chunk.byteLength)) {
          await writeChunk(client, chunk);
          if ((written + BigInt(chunk.byteLength)) % (64n * 1024n ** 2n) === 0n) {
            const partials = (await readdir(fixture.managedDirectory)).filter(name => name.endsWith('.upload.partial'));
            expect(partials).toHaveLength(1);
            maximumSpoolBytes = Math.max(maximumSpoolBytes,
              (await stat(join(fixture.managedDirectory, partials[0]!))).size);
          }
        }
        client.end();
        const result = await terminal;
        expect(result.status).toBe(400);
        expect(result.body).toContain(`archive upload exceeded the ${P0_ARCHIVE_UPLOAD_LIMIT}-byte limit`);
        expect(maximumSpoolBytes).toBeGreaterThan(Number(P0_ARCHIVE_UPLOAD_LIMIT) - 32 * 1024 ** 2);
        client.destroy();
        await preserved(fixture);
        const after = await health(fixture.server);
        expect(BigInt(`0x${after['completedStep'] as string}`))
          .toBeGreaterThan(BigInt(`0x${before['completedStep'] as string}`));
        console.log(JSON.stringify({ fullUploadLimitBytes: P0_ARCHIVE_UPLOAD_LIMIT.toString(),
          submittedBodyBytes: (P0_ARCHIVE_UPLOAD_LIMIT + BigInt(chunk.byteLength)).toString(),
          maximumSpoolBytes, wallMs: performance.now() - started, status: result.status }));
      });
    }, 180_000
  );
});
