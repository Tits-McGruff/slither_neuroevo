/** Real HTTP archive framing and wire-limit acceptance with an unchanged prior experiment. */
import { createHash, randomBytes } from 'node:crypto';
import { createRequire } from 'node:module';
import { createReadStream } from 'node:fs';
import { mkdir, mkdtemp, readdir, rm, stat, statfs } from 'node:fs/promises';
import { request, Server, type ClientRequest, type IncomingMessage, type ServerResponse } from 'node:http';
import { connect, type Socket } from 'node:net';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { constants as zstdConstants, createZstdDecompress, zstdCompressSync, zstdDecompressSync } from 'node:zlib';
import Database from 'better-sqlite3';
import { expect, it, vi } from 'vitest';
import WebSocket from 'ws';
import { DEFAULT_CONFIG } from './config.ts';
import { startRustServer, type RustServer } from './rustServer.ts';
import { P0_ARCHIVE_UPLOAD_LIMIT } from './rustEngine/archiveUpload.ts';
import * as archiveUpload from './rustEngine/archiveUpload.ts';
import { BackgroundOutputPump } from './rustEngine/backgroundOutput.ts';
import { CheckpointPersistenceClient } from './rustEngine/checkpointPersistenceClient.ts';
import { loadExperimentalFreshRunSession } from './rustEngine/experimentalFreshRunSession.ts';
import { admitDiskOperation, CHECKPOINT_DISK_ADMISSION_REQUEST } from './rustEngine/diskAdmission.ts';
import * as diskAdmission from './rustEngine/diskAdmission.ts';
import { describeNetworkSuite } from './test/networkSuites.ts';
import type { AssignMsg, SensorsMsg } from './protocol.ts';
import type { ExperimentalRuntimeTelemetrySnapshot } from './rustEngine/runtimeTelemetry.ts';

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
function metadata(path: string): unknown {
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
async function experiment(action: (fixture: Fixture) => Promise<void>): Promise<void> {
  const root = await mkdtemp(join(tmpdir(), 'slither-archive-transport-'));
  const databasePath = join(root, 'experiment.sqlite');
  const managedDirectory = `${databasePath}.checkpoints`;
  let server: RustServer | undefined;
  let fixture: Fixture | undefined;
  let socket: WebSocket | undefined;
  const sockets = new Set<Socket>();
  const requests = new Set<ClientRequest>();
  const peers = new Set<WebSocket>();
  try {
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

/** Observe the actual server response closing before success, without changing HTTP dispatch. */
function observeDisconnect(fixture: Fixture): { closed: Promise<boolean>; restore: () => void } {
  let disconnected!: (beforeFinish: boolean) => void;
  const closed = new Promise<boolean>(done => { disconnected = done; });
  const originalEmit = Server.prototype.emit;
  const dispatch = vi.spyOn(Server.prototype, 'emit').mockImplementation(function(
    this: Server, event: string | symbol, ...args: unknown[]
  ): boolean {
    if (event === 'request') {
      const incoming = args[0] as IncomingMessage;
      const response = args[1] as ServerResponse;
      if (incoming.socket.localPort === fixture.server.port && incoming.url === '/api/import/archive') {
        response.once('close', () => disconnected(!response.writableFinished));
      }
    }
    return Reflect.apply(originalEmit, this, [event, ...args]) as boolean;
  });
  return { closed, restore: () => dispatch.mockRestore() };
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
async function heldInput(peers: ControllerPeer[]): Promise<void> {
  await Promise.all(peers.map(async (peer, index) => {
    // Players may replace unsent input; trainers retain their one action per observation boundary.
    if (index === 0) peer.socket.send(JSON.stringify({ type: 'action',
      snakeId: peer.assignment!.snakeId, tick: peer.sample!.tick, turn: 1, boost: 0 }));
    peer.socket.send(JSON.stringify({ type: 'action',
      snakeId: peer.assignment!.snakeId, tick: peer.sample!.tick, turn: -1, boost: 0 }));
    const echo = new Promise<Buffer>(done => peer.socket.once('pong', bytes => done(bytes)));
    peer.socket.ping(`held-input-${index}`);
    expect((await bounded(echo, 'held input socket round trip failed')).toString()).toBe(`held-input-${index}`);
  }));
}

describeNetworkSuite('Rust archive HTTP framing', () => {
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
      const observation = observeDisconnect(fixture);
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
        release();
        stage.mockRestore();
        observation.restore();
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
        const observation = observeDisconnect(fixture);
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
          release();
          boundarySpy.mockRestore();
          observation.restore();
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
      const observation = observeDisconnect(fixture);
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
        release();
        commit.mockRestore();
        publish?.mockRestore();
        observation.restore();
      }
    });
  });

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
      await preserved(fixture);
    });
  });

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
