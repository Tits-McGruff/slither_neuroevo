/** Real HTTP archive framing and wire-limit acceptance with an unchanged prior experiment. */
import { createHash } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { mkdtemp, readdir, rm, stat, statfs } from 'node:fs/promises';
import { request, Server, type ClientRequest, type IncomingMessage, type ServerResponse } from 'node:http';
import { connect, type Socket } from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import Database from 'better-sqlite3';
import { expect, it, vi } from 'vitest';
import WebSocket from 'ws';
import { DEFAULT_CONFIG } from './config.ts';
import { startRustServer, type RustServer } from './rustServer.ts';
import { P0_ARCHIVE_UPLOAD_LIMIT } from './rustEngine/archiveUpload.ts';
import * as archiveUpload from './rustEngine/archiveUpload.ts';
import { BackgroundOutputPump } from './rustEngine/backgroundOutput.ts';
import { CheckpointPersistenceClient } from './rustEngine/checkpointPersistenceClient.ts';
import { describeNetworkSuite } from './test/networkSuites.ts';

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
      metadata: metadata(databasePath), files: await files(managedDirectory), sockets, requests };
    await action(fixture);
  } finally {
    for (const client of requests) client.destroy();
    for (const client of sockets) client.destroy();
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
async function raw(fixture: Fixture, headers: string[], body = Buffer.alloc(0), endInput = false): Promise<string> {
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

describeNetworkSuite('Rust archive HTTP framing', () => {
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
