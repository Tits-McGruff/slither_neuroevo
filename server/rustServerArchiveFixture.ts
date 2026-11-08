/** Real HTTP archive framing and wire-limit acceptance with an unchanged prior experiment. */
import Database from 'better-sqlite3';
import { execFile } from 'node:child_process';
import { createHash,randomBytes } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { mkdir,mkdtemp,readdir,readFile,readlink,rm,statfs } from 'node:fs/promises';
import { request,Server,type ClientRequest,type IncomingMessage,type ServerResponse } from 'node:http';
import { createRequire } from 'node:module';
import { connect,type Socket } from 'node:net';
import { tmpdir } from 'node:os';
import { dirname,join,resolve } from 'node:path';
import { promisify } from 'node:util';
import { createZstdDecompress,constants as zstdConstants,zstdDecompressSync } from 'node:zlib';
import { expect,onTestFinished,vi } from 'vitest';
import WebSocket from 'ws';
import { buildLargeBrainGraph } from '../scripts/fixtures/largeBrainGraph.ts';
import type { GraphSpec } from '../src/brains/graph/schema.ts';
import type { AssignMsg,SensorsMsg } from './protocol.ts';
import { CheckpointPersistenceClient } from './rustEngine/checkpointPersistenceClient.ts';
import { admitDiskOperation,CHECKPOINT_DISK_ADMISSION_REQUEST } from './rustEngine/diskAdmission.ts';
import { loadExperimentalFreshRunSession } from './rustEngine/experimentalFreshRunSession.ts';
import { startRustServer,type RustServer } from './rustServer.ts';
import { ARCHIVE_PREPARATION_TIMEOUT_MS,fixtureArchiveDownload } from './test/archiveDownload.ts';
import { RUST_TEST_CONFIG as DEFAULT_CONFIG } from './test/rustConfig.ts';

/** Bounded native mount commands used only by the explicitly selected Linux quota fixture. */
export const runFile = promisify(execFile);

/** Original HTTP dispatch, captured before a timed-out fixture can install an observer. */
export const originalHttpEmit = Server.prototype.emit;

/** Whole-test budget for a real server, fresh durable checkpoint, baseline export and shutdown.
 * Five seconds is reserved for individual rejection/cleanup boundaries below;
 * it is not a valid budget for all of the fixture's disk operations combined.
 */
export const ARCHIVE_FIXTURE_TIMEOUT_MS = 20_000;
/** A rejection fixture also verifies a second successful archive preparation after cleanup. */
export const ARCHIVE_RETRY_FIXTURE_TIMEOUT_MS = ARCHIVE_FIXTURE_TIMEOUT_MS + ARCHIVE_PREPARATION_TIMEOUT_MS;

/** Change the task's private tmpfs quota without allocating its advertised capacity. */
export async function quota(directory: string, bytes: bigint): Promise<void> {
  // Do not reparse host-mapped uid/gid options from mountinfo inside the user namespace.
  await runFile('mount', ['--options-mode', 'replace', '--types', 'tmpfs', '--options',
    `remount,size=${bytes},uid=0,gid=0`, 'slither-a7-quota', directory], { timeout: 5000 });
}

/** One task-owned authority and its durable pre-request evidence. */
export interface Fixture {
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
  /** Every retained Rust metadata table before the request. */
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
export async function files(directory: string): Promise<Fixture['files']> {
  const result: Fixture['files'] = [];
  for (const filename of (await readdir(directory)).sort()) {
    const hash = createHash('sha256');
    for await (const bytes of createReadStream(join(directory, filename))) hash.update(bytes);
    result.push({ filename, sha256: hash.digest('hex') });
  }
  return result;
}

/** Read every Rust metadata table through a task-owned read-only SQLite connection. */
export function metadata(path: string): Array<{ name: string; rows: unknown[] }> {
  const database = new Database(path, { readonly: true });
  try {
    const tables = database.prepare(`SELECT name FROM sqlite_master
      WHERE type = 'table' AND name LIKE 'rust_%' ORDER BY name`).all() as Array<{ name: string }>;
    return tables.map(({ name }) => ({ name, rows: database.prepare(
      `SELECT * FROM "${name.replaceAll('"', '""')}" ORDER BY rowid`
    ).all() }));
  } finally { database.close(); }
}

/** Bounded loopback fixture transport, independent of Fetch's browser unsafe-port list. */
export async function loopbackResponse(url: string, init: {method?: string; headers?: Record<string, string>; body?: Buffer} = {}): Promise<Response> {
  const target = new URL(url);
  if (target.protocol !== 'http:' || target.hostname !== '127.0.0.1') throw new Error('fixture requires loopback HTTP');
  return await new Promise<Response>((done, reject) => {
    const client = request(target, {method: init.method ?? 'GET', headers: {
      ...init.headers, ...(init.body === undefined ? {} : {'Content-Length': init.body.byteLength})
    }}, response => {
      const chunks: Buffer[] = []; let length = 0;
      response.on('data', (chunk: Buffer) => {
        length += chunk.byteLength;
        if (length > 8 * 1024 * 1024) response.destroy(new Error('loopback fixture response exceeds eight MiB'));
        else chunks.push(chunk);
      });
      response.once('error', reject);
      response.once('end', () => {
        const headers = new Headers();
        for (const [key, value] of Object.entries(response.headers)) if (value !== undefined) headers.set(key, String(value));
        done(new Response(new Uint8Array(Buffer.concat(chunks)), {status: response.statusCode ?? 500, headers}));
      });
    });
    client.once('error', reject);
    client.setTimeout(10_000, () => client.destroy(new Error('loopback fixture request stalled')));
    client.end(init.body);
  });
}

/** Read actual health without retaining display frames or population state. */
export async function health(server: RustServer): Promise<Record<string, unknown>> {
  const response = await loopbackResponse(`http://127.0.0.1:${server.port}/api/health`);
  expect(response.status).toBe(200);
  return await response.json() as Record<string, unknown>;
}

/** Wait for lease/spool cleanup while preserving a bounded failure diagnostic. */
export async function noTransferScratch(directory: string): Promise<void> {
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
export async function slow(socket: WebSocket): Promise<void> {
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
export async function experiment(action: (fixture: Fixture) => Promise<void>, privateQuota = false): Promise<void> {
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
    const release = vi.spyOn(CheckpointPersistenceClient.prototype, 'releaseExportLease');
    let archive: Buffer;
    try {
      const response = await fixtureArchiveDownload(server.port, 'transport fixture baseline');
      expect(response.status, response.status === 200 ? undefined : await response.text()).toBe(200);
      archive = Buffer.from(await response.arrayBuffer());
      // Motion/B16 doubles the default weight payload; stay within the bounded
      // eight-MiB loopback reader while retaining the full ordinary population.
      expect(archive.byteLength).toBeLessThan(8 * 1024 * 1024);
      // Body completion and ready-file removal precede the worker's actual
      // lease-release acknowledgement. Observe that real completion before
      // testing another operation; keep the existing five-second deadline.
      await vi.waitFor(() => expect(release).toHaveBeenCalledTimes(1), { timeout: 5000, interval: 10 });
      await release.mock.results[0]!.value;
      await new Promise<void>(done => setImmediate(done));
    } finally { release.mockRestore(); }
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
export async function preserved(fixture: Fixture): Promise<void> {
  await noTransferScratch(fixture.managedDirectory);
  expect(metadata(fixture.databasePath)).toEqual(fixture.metadata);
  expect(await files(fixture.managedDirectory)).toEqual(fixture.files);
  expect(await health(fixture.server)).toMatchObject({ ok: true, ...fixture.identity });
}

/** Prove the server finished disconnected work by completing a fresh real export. */
export async function exportAfterDisconnect(fixture: Fixture): Promise<void> {
  const deadline = performance.now() + 5000;
  while (performance.now() < deadline) {
    const response = await fixtureArchiveDownload(fixture.server.port, 'export after disconnect');
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
export function legacyPopulation(): Buffer {
  return Buffer.from(JSON.stringify({ generation: 37, archKey: 'legacy-default-graph',
    worldSeed: 1234567, settings: { snakeCount: 2, simSpeed: 0.1 },
    genomes: [1, 2].map(fitness => ({ archKey: 'legacy-default-graph', brainType: 'mlp',
      fitness, weights: new Array<number>(13458).fill(0) })) }));
}

/** Locate extents only in the canonical small archive produced by this test's real exporter. */
export function archiveEntries(archive: Buffer): Array<{ header: number; data: number; size: number; end: number }> {
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
  expect(entries.length).toBeGreaterThanOrEqual(9);
  expect(entries.length).toBeLessThanOrEqual(111);
  const final = entries.at(-1)!;
  expect(archive.subarray(final.header, final.header + 100).toString().split('\0')[0]).toBe('manifest.json');
  const manifest = JSON.parse(archive.subarray(final.data, final.data + final.size).toString()) as { archiveVersion: number; recordsVersion?: number; savedBrains?: unknown[]; roles: unknown[] };
  expect([1, 2, 3, 4]).toContain(manifest.archiveVersion);
  const recordsVersion = manifest.archiveVersion === 4 ? manifest.recordsVersion! : manifest.archiveVersion;
  expect([1, 2, 3]).toContain(recordsVersion);
  const selected = manifest.archiveVersion === 4 ? manifest.savedBrains!.length : 0;
  expect(entries).toHaveLength(recordsVersion + 8 + selected);
  if (recordsVersion >= 2) expect(manifest.roles[8]).toMatchObject({ role: 'generation-completion', path: 'generation-completion.bin', encoding: 'raw-generation-completion-v1', recordSize: 40 });
  if (recordsVersion === 3) expect(manifest.roles[9]).toMatchObject({ role: 'generation-experiments', path: 'generation-experiments.bin', encoding: 'raw-generation-experiment-v1', recordSize: 776 });
  expect(manifest.roles).toHaveLength(entries.length - 1);
  expect(header + 1024).toBe(archive.byteLength);
  expect(archive.subarray(header).every(byte => byte === 0)).toBe(true);
  return entries;
}

/** Update a deliberately changed USTAR header so tests reach type/path validation beyond checksum. */
export function archiveHeaderChecksum(bytes: Buffer, header: number): void {
  bytes.fill(0x20, header + 148, header + 156);
  const checksum = bytes.subarray(header, header + 512).reduce((sum, byte) => sum + byte, 0);
  bytes.write(checksum.toString(8).padStart(6, '0') + '\0 ', header + 148, 8, 'ascii');
}

/** Manifest fields needed to independently rehash a small semantic failure fixture. */
export interface SemanticRole {
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
export interface SemanticManifest {
  /** Save root over the versioned ordered roles. */
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
export function semanticRoot(domain: string, roles: SemanticRole[]): string {
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
export function semanticArchive(archive: Buffer, roleIndex?: number, changed?: Buffer): Buffer {
  const entries = archiveEntries(archive);
  const bodies: Buffer[] = entries.map(entry => Buffer.from(archive.subarray(entry.data, entry.data + entry.size)));
  const manifestIndex = bodies.length - 1;
  const manifest = JSON.parse(bodies[manifestIndex]!.toString()) as SemanticManifest;
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
  bodies[manifestIndex] = Buffer.from(JSON.stringify(manifest));
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
export function reversedConcatRole(role: Buffer): Buffer {
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
export function orderingGraph(reversed = false): GraphSpec {
  return {
    type: 'graph',
    nodes: [{ id: 'input', type: 'Input', outputSize: 264 },
      { id: 'split', type: 'Split', outputSizes: [131, 133] }, { id: 'concat', type: 'Concat' },
      { id: 'head', type: 'Dense', inputSize: 264, outputSize: 2 }],
    edges: [{ from: 'input', to: 'split', fromPort: 0 },
      { from: 'split', to: 'concat', fromPort: 0, toPort: reversed ? 1 : 0 },
      { from: 'split', to: 'concat', fromPort: 1, toPort: reversed ? 0 : 1 },
      { from: 'concat', to: 'head', fromPort: 0 }],
    outputs: [{ nodeId: 'head', port: 0 }], outputSize: 2
  };
}

/** Admit an evolved checkpoint and prove the independent repacker's valid control imports. */
export async function evolvedArchiveFixture(fixture: Fixture, mode: 'ordering' | 'streaming' = 'ordering'): Promise<void> {
  const streaming = mode === 'streaming';
  const viewer = new WebSocket(`ws://127.0.0.1:${fixture.server.port}`);
  fixture.peers.add(viewer);
  await slow(viewer);
  let rejoinToken: unknown;
  const reset = new Promise<void>((done, reject) => viewer.on('message', (bytes, binary) => {
    if (binary) return;
    const packet = JSON.parse(bytes.toString()) as Record<string, unknown>;
    if (packet['type'] === 'error') reject(new Error(String(packet['message'])));
    if (packet['type'] === 'stateReplaced') { rejoinToken = packet['rejoinToken']; if (packet['reason'] === 'reset') done(); }
  }));
  viewer.send(JSON.stringify({ type: 'reset', settings: { snakeCount: 12, simSpeed: 12 },
    updates: [{ path: 'generationSeconds', value: 8 }, { path: 'baselineBots.count', value: 2 },
      { path: 'pelletCountTarget', value: 100 }, ...(streaming ? [{ path: 'worldRadius', value: 10000 }] : [])],
    // A few large brains provide the real >8 MiB response required for socket
    // backpressure without making this transport fixture a 300-snake sensor test.
    graphSpec: streaming ? buildLargeBrainGraph(264) : orderingGraph() }));
  await bounded(reset, 'ordering fixture reset did not commit');
  viewer.send(JSON.stringify({ type: 'join', mode: 'spectator', rejoinToken }));
  const rejoined = new Promise<Buffer>(done => viewer.once('pong', done));
  viewer.ping('ordering-fixture-rejoin');
  expect((await bounded(rejoined, 'ordering fixture rejoin was not received')).toString()).toBe('ordering-fixture-rejoin');
  // The streaming fixture executes 480 real large-brain steps and commits generation
  // two. Detect a stalled engine or checkpoint within five seconds of its last
  // progress; the unchanged whole-test deadline bounds total setup and cancellation.
  let deadline = performance.now() + 5000;
  let generation = 1n;
  let completedStep = 0n;
  while (generation < 2n && performance.now() < deadline) {
    const current = await health(fixture.server);
    expect(current['ok']).toBe(true);
    const nextStep = BigInt(`0x${current['completedStep'] as string}`);
    expect(nextStep).toBeGreaterThanOrEqual(completedStep);
    if (streaming && nextStep > completedStep) deadline = performance.now() + 5000;
    completedStep = nextStep;
    generation = BigInt(`0x${current['generation'] as string}`);
    if (generation < 2n) await new Promise<void>(done => setTimeout(done, 10));
  }
  expect(generation, `evolution stopped at step ${completedStep}`).toBeGreaterThanOrEqual(2n);
  if (streaming) expect(completedStep).toBeGreaterThanOrEqual(480n);
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
  const release = vi.spyOn(CheckpointPersistenceClient.prototype, 'releaseExportLease');
  try {
    const response = await fetch(`http://127.0.0.1:${fixture.server.port}/api/export/latest`);
    expect(response.status).toBe(200);
    fixture.archive = Buffer.from(await response.arrayBuffer());
    expect(fixture.archive.byteLength).toBeLessThan((streaming ? 64 : 4) * 1024 * 1024);
    if (streaming) expect(fixture.archive.byteLength).toBeGreaterThan(8 * 1024 * 1024);
    // File cleanup precedes the real worker acknowledgement. Observe that same
    // lease-release boundary as the base fixture before importing this archive.
    await outcome(() => release.mock.calls.length === 1, 'evolved fixture export did not release its lease');
    await bounded(release.mock.results[0]!.value, 'evolved fixture export release was not acknowledged');
    await new Promise<void>(done => setImmediate(done));
  } finally { release.mockRestore(); }
  await noTransferScratch(fixture.managedDirectory);
  const repacked = semanticArchive(fixture.archive);
  const previousToken = rejoinToken;
  const imported = await fetch(`http://127.0.0.1:${fixture.server.port}/api/import/archive`,
    { method: 'POST', body: new Uint8Array(repacked), signal: AbortSignal.timeout(5000) });
  expect(imported.status, await imported.clone().text()).toBe(200);
  expect(await imported.json()).toMatchObject({ ok: true, branched: false });
  // Exact checkpoints retain the rate at their boundary; the live slowdown above is not saved.
  // Rejoin the replacement and explicitly slow it before observing preserved files/controllers.
  await outcome(() => rejoinToken !== previousToken, 'import replacement notice was not received');
  viewer.send(JSON.stringify({ type: 'join', mode: 'spectator', rejoinToken }));
  const slowedReplacement = new Promise<void>((done, reject) => {
    /** Require the replacement world's actual settings acknowledgement. */
    const receive = (bytes: WebSocket.RawData, binary: boolean): void => {
      if (binary) return;
      const packet = JSON.parse(bytes.toString()) as Record<string, unknown>;
      if (packet['type'] === 'error') {
        viewer.off('message', receive);
        reject(new Error(String(packet['message'])));
      }
      if (packet['type'] === 'settingsApplied' && packet['requestId'] === 'ordering-replacement-speed') {
        viewer.off('message', receive);
        if (packet['applied'] === true) done();
        else reject(new Error('ordering replacement speed was rejected'));
      }
    };
    viewer.on('message', receive);
  });
  viewer.send(JSON.stringify({ type: 'settings', requestId: 'ordering-replacement-speed',
    updates: [{ path: 'simSpeed', value: 0.1 }] }));
  await bounded(slowedReplacement, 'ordering replacement did not slow after import');
  viewer.terminate();
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
export function expansionFrame(): Buffer {
  const blocks = 128;
  const frame = Buffer.alloc(6 + blocks * 4);
  Buffer.from([0x28, 0xb5, 0x2f, 0xfd, 0x00, 0x50]).copy(frame);
  for (let index = 0; index < blocks; index++) {
    frame.writeUIntLE((131072 << 3) | 2 | (index === blocks - 1 ? 1 : 0), 6 + index * 4, 3);
  }
  return frame;
}

/** Independently prove the expansion fixture is a valid stream without retaining its output. */
export async function expansionBytes(frame: Buffer): Promise<number> {
  const decoder = createZstdDecompress({ chunkSize: 64 * 1024,
    params: { [zstdConstants.ZSTD_d_windowLogMax]: 20 } });
  decoder.end(frame);
  let count = 0;
  for await (const bytes of decoder) count += (bytes as Buffer).byteLength;
  return count;
}

/** Locate a payload-bit change that a separate bounded decoder accepts with different output. */
export function alteredDecodedFrame(frame: Buffer, decodedBytes: number): Buffer {
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
export async function conflictingArchive(fixture: Fixture): Promise<Buffer> {
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
    expect(bytes.byteLength).toBeLessThan(8 * 1024 * 1024);
    return bytes;
  } finally { await server.close(); }
}

/** Produce and independently re-import a small valid archive using the approved large-brain graph. */
export async function largeBrainArchive(fixture: Fixture): Promise<Buffer> {
  const source = await startRustServer({ ...DEFAULT_CONFIG, host: '127.0.0.1', port: 0,
    resume: 'fresh', seed: 99, dbPath: join(dirname(fixture.databasePath), 'large-brain.sqlite') });
  let viewer: WebSocket | undefined;
  try {
    expect(source.startupFault).toBeUndefined();
    viewer = new WebSocket(`ws://127.0.0.1:${source.port}`);
    await slow(viewer);
    let rejoinToken: unknown;
    const reset = new Promise<string>((done, reject) => {
      viewer!.on('message', (bytes, binary) => {
        if (binary) return;
        const message = JSON.parse(bytes.toString()) as Record<string, unknown>;
        if (message['type'] === 'error') reject(new Error(String(message['message'])));
        if (message['type'] === 'stateReplaced' && message['reason'] === 'reset') {
          rejoinToken = message['rejoinToken'];
          done((message['welcome'] as { runId: string }).runId);
        }
      });
      viewer!.once('error', reject);
    });
    viewer.send(JSON.stringify({ type: 'reset', graphSpec: buildLargeBrainGraph(147),
      settings: { snakeCount: 2, simSpeed: 0.1 }, updates: [
        { path: 'sense.bubbleBins', value: 32 }, { path: 'baselineBots.count', value: 0 },
        {path: 'sense.layoutVersion', value: 3}, {path: 'baselineBots.algorithm', value: 1}
      ] }));
    const runId = await bounded(reset, 'large-brain source did not complete reset');
    viewer.send(JSON.stringify({ type: 'join', mode: 'spectator', rejoinToken }));
    // The reliable replacement notice precedes the final retention read.
    // Wait for that reset boundary before starting another persistence operation.
    const deadline = performance.now() + 5000;
    let current = await health(source);
    while ((current['retention'] as { activeRunId: string }).activeRunId !== runId &&
        performance.now() < deadline) {
      await new Promise<void>(done => setTimeout(done, 10));
      current = await health(source);
    }
    expect(current['ok']).toBe(true);
    expect((current['retention'] as { activeRunId: string }).activeRunId).toBe(runId);
    const exported = await fetch(`http://127.0.0.1:${source.port}/api/export/latest`);
    expect(exported.status).toBe(200);
    const bytes = Buffer.from(await exported.arrayBuffer());
    expect(bytes.byteLength).toBeLessThan(8 * 1024 * 1024);
    await noTransferScratch(`${join(dirname(fixture.databasePath), 'large-brain.sqlite')}.checkpoints`);
    const imported = await fetch(`http://127.0.0.1:${source.port}/api/import/archive`,
      { method: 'POST', body: new Uint8Array(bytes), signal: AbortSignal.timeout(5000) });
    expect(imported.status).toBe(200);
    expect(await imported.json()).toMatchObject({ ok: true, branched: false });
    return bytes;
  } finally { viewer?.terminate(); await source.close(); }
}

/** Prove cancellation or publication releases the world to complete further fixed steps. */
export async function advancing(fixture: Fixture): Promise<void> {
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
export async function raw(fixture: Fixture, headers: string[], body: Buffer = Buffer.alloc(0), endInput = false): Promise<string> {
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
export async function writeChunk(client: ClientRequest, chunk: Buffer): Promise<void> {
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
export async function bounded<T>(promise: Promise<T>, description: string, deadlineMs = 5000): Promise<T> {
  let timeout: NodeJS.Timeout | undefined;
  try {
    return await Promise.race([promise, new Promise<never>((_done, reject) => {
      timeout = setTimeout(() => reject(new Error(description)), deadlineMs);
    })]);
  } finally { if (timeout) clearTimeout(timeout); }
}

/** Release a held boundary on test timeout as well as normal completion, exactly once. */
export function testCleanup(action: () => void): () => void {
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
export function observeDisconnect(server: Pick<RustServer, 'port'>, endpoint = '/api/import/archive'):
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
export interface ControllerPeer {
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
export async function outcome(predicate: () => boolean, description: string): Promise<void> {
  const deadline = performance.now() + 5000;
  while (!predicate() && performance.now() < deadline) await new Promise<void>(done => setTimeout(done, 10));
  expect(predicate(), description).toBe(true);
}

/** Acquire an actual player or trainer lease, retaining only the newest sensor sample. */
export async function controller(fixture: Fixture, kind: 'ui' | 'bot', name = `Import-${kind}`): Promise<ControllerPeer> {
  const socket = new WebSocket(`ws://127.0.0.1:${fixture.server.port}`);
  fixture.peers.add(socket);
  const peer: ControllerPeer = { socket, packets: [] };
  socket.on('error', error => peer.packets.push({ type: 'error', message: error.message }));
  socket.on('open', () => socket.send(JSON.stringify({ type: 'hello', version: 2, clientType: kind })));
  socket.on('message', (bytes, binary) => {
    if (binary) return;
    const packet = JSON.parse(bytes.toString()) as Record<string, unknown>;
    if (packet['type'] === 'welcome') socket.send(JSON.stringify({ type: 'join', mode: 'player', name }));
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
export function headingChange(before: SensorsMsg, after: SensorsMsg): number {
  const first = Math.atan2(before.sensors[0]!, before.sensors[1]!);
  const last = Math.atan2(after.sensors[0]!, after.sensors[1]!);
  return Math.atan2(Math.sin(last - first), Math.cos(last - first));
}

/** Deliver held input through actual sockets, proving server receipt with ordered round trips. */
export async function heldInput(peers: ControllerPeer[], replacePlayer = true): Promise<void> {
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
