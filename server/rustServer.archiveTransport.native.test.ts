/** Real HTTP archive framing and wire-limit acceptance with an unchanged prior experiment. */
import { createHash } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { mkdtemp, readdir, rm, stat, statfs } from 'node:fs/promises';
import { request, type ClientRequest } from 'node:http';
import { connect, type Socket } from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import Database from 'better-sqlite3';
import { expect, it } from 'vitest';
import WebSocket from 'ws';
import { DEFAULT_CONFIG } from './config.ts';
import { startRustServer, type RustServer } from './rustServer.ts';
import { P0_ARCHIVE_UPLOAD_LIMIT } from './rustEngine/archiveUpload.ts';
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
      name.includes('upload') || name.includes('slither-save') || name.includes('export-inventory'));
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
    await action({ server, databasePath, managedDirectory, archive, identity,
      metadata: metadata(databasePath), files: await files(managedDirectory), sockets, requests });
  } finally {
    for (const client of requests) client.destroy();
    for (const client of sockets) client.destroy();
    socket?.terminate();
    await server?.close();
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

describeNetworkSuite('Rust archive HTTP framing', () => {
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
