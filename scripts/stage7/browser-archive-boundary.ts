/** Trace a real browser export while its exact selected checkpoint ages by one generation. */
import { createHash } from 'node:crypto';
import { appendFileSync, createReadStream, existsSync } from 'node:fs';
import { mkdir, stat, statfs } from 'node:fs/promises';
import { Server, type IncomingMessage, type ServerResponse } from 'node:http';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { DEFAULT_CONFIG } from '../../server/config.ts';
import { startRustServer } from '../../server/rustServer.ts';
import { CheckpointPersistenceClient } from '../../server/rustEngine/checkpointPersistenceClient.ts';

/** Explicit new workspace and existing immutable save for this bounded browser fixture. */
interface Options {
  /** Original validated save, streamed into a new production authority. */
  archivePath: string;
  /** Absent persistent directory containing only this diagnostic's output. */
  outputRoot: string;
  /** Deliberate loopback port used by the browser. */
  port: number;
}

/** Read only the small production health fields needed to observe durable advancement. */
async function health(port: number): Promise<{ ok: boolean; runId: string; generation: string }> {
  const response = await fetch(`http://127.0.0.1:${port}/api/health`, {
    signal: AbortSignal.timeout(5000)
  });
  const value = await response.json() as { ok: boolean; runId: string; generation: string };
  if (!response.ok || !value.ok) throw new Error(`production health failed: ${response.status}`);
  return value;
}

/** Require a new explicit workspace and bounded loopback port. */
function options(argv: readonly string[]): Options {
  if (argv.length !== 6 || argv[0] !== '--archive-path' || argv[2] !== '--output-root' ||
      argv[4] !== '--port' || !argv[1] || !argv[3] || !/^[1-9][0-9]*$/u.test(argv[5] ?? '')) {
    throw new Error('usage: --archive-path FILE --output-root NEW_DIRECTORY --port PORT');
  }
  const archivePath = resolve(argv[1]);
  const outputRoot = resolve(argv[3]);
  const port = Number(argv[5]);
  if (existsSync(outputRoot) || !Number.isSafeInteger(port) || port < 1024 || port > 65535) {
    throw new Error('output directory must be absent and port must be from 1024 to 65535');
  }
  return { archivePath, outputRoot, port };
}

/** Serve unchanged production archive bytes with a deterministic post-selection scheduling delay. */
export async function run(request: Options): Promise<void> {
  if (existsSync(request.outputRoot)) throw new Error('output directory already exists');
  const archive = await stat(request.archivePath);
  if (!archive.isFile() || archive.size <= 50 * 1024 * 1024) throw new Error('large regular save required');
  await mkdir(request.outputRoot, { recursive: true });
  const filesystem = await statfs(request.outputRoot, { bigint: true });
  const freeBytes = filesystem.bavail * filesystem.bsize;
  if (freeBytes < 10n * 1024n ** 3n) throw new Error('fixture filesystem has less than 10 GiB free');
  /** Compact request, selected-checkpoint, transition and actual response-body records. */
  const record = (phase: string, detail: Record<string, unknown>): void => {
    appendFileSync(resolve(request.outputRoot, 'events.jsonl'),
      JSON.stringify({ at: new Date().toISOString(), phase, ...detail }) + '\n');
  };
  record('filesystem', { freeBytes: freeBytes.toString() });
  /** Original transport dispatcher, restored when this one process closes. */
  const originalEmit = Server.prototype.emit;
  /** Original production lease acquisition, preserving the selected immutable descriptor. */
  const originalAcquire = CheckpointPersistenceClient.prototype.acquireCurrentExportLease;
  /** Whether shutdown has been requested by a signal, stop file or hard lifetime. */
  let closing = false;
  /** Number of requests observed by the browser fixture, independent of generation. */
  let requestNumber = 0;
  Server.prototype.emit = function(event: string | symbol, ...args: unknown[]): boolean {
    if (event === 'request') {
      const incoming = args[0] as IncomingMessage;
      const response = args[1] as ServerResponse;
      if (incoming.method === 'GET' && incoming.url === '/api/export/latest') {
        const requestId = ++requestNumber;
        const digest = createHash('sha256');
        let bodyBytes = 0;
        record('request', { requestId, method: incoming.method, url: incoming.url,
          userAgent: incoming.headers['user-agent'],
          range: incoming.headers['range'] ?? null, ifRange: incoming.headers['if-range'] ?? null,
          accept: incoming.headers['accept'] ?? null,
          fetchDestination: incoming.headers['sec-fetch-dest'] ?? null,
          fetchMode: incoming.headers['sec-fetch-mode'] ?? null,
          fetchSite: incoming.headers['sec-fetch-site'] ?? null });
        const originalWrite = response.write;
        const originalEnd = response.end;
        const originalWriteHead = response.writeHead;
        /** Hash original outgoing chunks without retaining or replacing archive data. */
        const observe = (chunk: unknown, encoding: unknown): void => {
          if (typeof chunk === 'string') {
            const bytes = Buffer.from(chunk, typeof encoding === 'string' ? encoding as BufferEncoding : 'utf8');
            digest.update(bytes); bodyBytes += bytes.length;
          } else if (Buffer.isBuffer(chunk) || chunk instanceof Uint8Array) {
            digest.update(chunk); bodyBytes += chunk.byteLength;
          }
        };
        response.write = function(...values: unknown[]): boolean {
          observe(values[0], values[1]);
          return Reflect.apply(originalWrite, this, values) as boolean;
        };
        response.end = function(...values: unknown[]): ServerResponse {
          observe(values[0], values[1]);
          return Reflect.apply(originalEnd, this, values) as ServerResponse;
        };
        response.writeHead = function(...values: unknown[]): ServerResponse {
          record('headers', { requestId, arguments: values });
          return Reflect.apply(originalWriteHead, this, values) as ServerResponse;
        };
        response.once('finish', () => record('finish', { requestId, bodyBytes, bodySha256: digest.digest('hex') }));
        response.once('close', () => record('close', { requestId, finished: response.writableFinished }));
      }
    }
    return Reflect.apply(originalEmit, this, [event, ...args]) as boolean;
  };
  const server = await startRustServer({ ...DEFAULT_CONFIG, host: '127.0.0.1', port: request.port,
    dbPath: resolve(request.outputRoot, 'experiment.sqlite'), resume: 'fresh', seed: 1511506142,
    rustCalculationWorkers: 6, logLevel: 'error' });
  if (server.startupFault) { await server.close(); throw new Error(server.startupFault); }
  try {
    const response = await fetch(`http://127.0.0.1:${server.port}/api/import/archive`, {
      method: 'POST', headers: { 'Content-Type': 'application/vnd.slither-neuroevo.save',
        // Node fetch accepts this AsyncIterable body; DOM RequestInit omits that extension.
        'Content-Length': String(archive.size) }, body: createReadStream(request.archivePath) as unknown as BodyInit,
      duplex: 'half', signal: AbortSignal.timeout(120_000)
    } as RequestInit & { duplex: 'half' });
    const receipt = await response.json() as { ok: boolean } & Record<string, unknown>;
    if (!response.ok || !receipt.ok) throw new Error(`fixture import failed: ${JSON.stringify(receipt)}`);
    record('fixture-import', { receipt });
    CheckpointPersistenceClient.prototype.acquireCurrentExportLease = async function() {
      const lease = await originalAcquire.call(this);
      record('selected', { operationId: lease.operationId, runId: lease.runId,
        generation: lease.descriptor.generation, completedStep: lease.descriptor.completedStep,
        checkpointId: lease.descriptor.logicalRootSha256 });
      console.log(`EXPORT_SELECTED generation=${BigInt(`0x${lease.descriptor.generation}`)}; waiting for next durable generation`);
      const deadline = performance.now() + 120_000;
      try {
        for (;;) {
          if (closing) throw new Error('fixture closing while checkpoint is leased');
          const current = await health(server.port);
          if (current.runId !== lease.runId) throw new Error('unexpected fixture run replacement');
          if (BigInt(`0x${current.generation}`) > BigInt(`0x${lease.descriptor.generation}`)) {
            record('advanced-before-preparation', { operationId: lease.operationId,
              generation: current.generation, selectedGeneration: lease.descriptor.generation });
            console.log(`EXPORT_ADVANCED generation=${BigInt(`0x${current.generation}`)}; preparing the original leased checkpoint`);
            return lease;
          }
          if (performance.now() >= deadline) throw new Error('next durable generation exceeded the bounded fixture wait');
          await new Promise<void>(done => setTimeout(done, 100));
        }
      } catch (error) {
        await this.releaseExportLease(lease.operationId).catch(() => {});
        throw error;
      }
    };
    /** Stop this diagnostic without adding routes or controls to the production server. */
    const close = async (reason: string): Promise<void> => {
      if (closing) return;
      closing = true;
      record('stopping', { reason });
      clearInterval(stopPoll); clearTimeout(lifetime);
      CheckpointPersistenceClient.prototype.acquireCurrentExportLease = originalAcquire;
      Server.prototype.emit = originalEmit;
      await server.close();
      record('stopped', { reason });
    };
    /** Task-owned stop marker, checked independently of browser success or failure. */
    const stopPoll = setInterval(() => {
      if (existsSync(resolve(request.outputRoot, 'stop'))) void close('stop marker');
    }, 500);
    /** Hard twenty-minute limit prevents abandoned diagnostic games. */
    const lifetime = setTimeout(() => void close('twenty-minute lifetime'), 1_200_000);
    process.once('SIGINT', () => void close('SIGINT')); process.once('SIGTERM', () => void close('SIGTERM'));
    console.log(`BOUNDARY_UI_READY http://127.0.0.1:${server.port}/?server=ws://127.0.0.1:${server.port}`);
  } catch (error) {
    CheckpointPersistenceClient.prototype.acquireCurrentExportLease = originalAcquire;
    Server.prototype.emit = originalEmit;
    await server.close();
    throw error;
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  void run(options(process.argv.slice(2))).catch(error => {
    console.error(String(error)); process.exitCode = 1;
  });
}
