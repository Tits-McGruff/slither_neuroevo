/** Isolate ordinary attachment handling from the game with bounded, request-specific bytes. */
import { createHash } from 'node:crypto';
import { existsSync } from 'node:fs';
import { appendFile, mkdir } from 'node:fs/promises';
import { createServer } from 'node:http';
import { resolve } from 'node:path';
import { Readable } from 'node:stream';
import { pipeline } from 'node:stream/promises';

/** Run a five-minute loopback-only probe in an explicit, previously absent directory. */
async function run(): Promise<void> {
  const args = process.argv.slice(2);
  if (args.length !== 4 || args[0] !== '--output-root' || args[2] !== '--port' ||
      !args[1] || !/^[1-9][0-9]*$/u.test(args[3] ?? '')) {
    throw new Error('usage: --output-root NEW_DIRECTORY --port PORT');
  }
  const outputRoot = resolve(args[1]);
  const port = Number(args[3]);
  if (existsSync(outputRoot) || port < 1024 || port > 65535) {
    throw new Error('output directory must be absent and port must be from 1024 to 65535');
  }
  await mkdir(outputRoot, { recursive: true });
  /** Serialized compact evidence writes, never retaining downloaded bodies. */
  let records = Promise.resolve();
  /** Record real transport events in occurrence order. */
  const record = (phase: string, detail: Record<string, unknown>): void => {
    const line = JSON.stringify({ at: new Date().toISOString(), phase, ...detail }) + '\n';
    records = records.then(() => appendFile(resolve(outputRoot, 'events.jsonl'), line));
  };
  /** Every GET selects a different immutable payload and attachment name. */
  let requestNumber = 0;
  /** Bounded payload exceeds the large-download gate without any simulation or database. */
  const chunks = 52;
  const chunkBytes = 1024 * 1024;
  /** The production export UI uses exactly this hidden attachment-link pattern. */
  const page = `<!doctype html><html lang="en"><meta charset="utf-8">
<title>Ordinary download probe</title><h1>Ordinary download probe</h1>
<p>52 MiB of synthetic bytes. No game, population, or database.</p>
<button id="download">Download request-specific bytes</button>
<button id="save">Download synthetic save attachment</button>
<script>
for (const [id, url] of [['download', '/download.bin'], ['save', '/synthetic.slither-save']]) {
document.getElementById(id).addEventListener('click', () => {
  const link = document.createElement('a');
  link.href = url;
  link.download = '';
  link.hidden = true;
  document.body.append(link);
  link.click();
  link.remove();
});
}
</script></html>`;
  const server = createServer((incoming, response) => {
    if (incoming.method === 'GET' && incoming.url === '/') {
      response.writeHead(200, { 'content-type': 'text/html; charset=utf-8', 'cache-control': 'no-store' });
      response.end(page);
      return;
    }
    if (incoming.method !== 'GET' ||
        !['/download.bin', '/synthetic.slither-save'].includes(incoming.url ?? '')) {
      response.writeHead(404); response.end(); return;
    }
    const requestId = ++requestNumber;
    const saveType = incoming.url === '/synthetic.slither-save';
    const filename = saveType ? `synthetic-request-${requestId}.slither-save` : `request-${requestId}.bin`;
    const chunk = Buffer.alloc(chunkBytes, requestId % 256);
    const expectedDigest = createHash('sha256');
    for (let index = 0; index < chunks; index++) expectedDigest.update(chunk);
    record('request', { requestId, filename, expectedBytes: chunks * chunkBytes,
      expectedSha256: expectedDigest.digest('hex'), headers: incoming.headers });
    /** Hash generated stream chunks; only a finished response proves complete delivery. */
    let generatedBytes = 0;
    const generatedDigest = createHash('sha256');
    response.once('finish', () => record('finish', { requestId, generatedBytes,
      generatedSha256: generatedDigest.digest('hex') }));
    response.once('close', () => record('close', { requestId, finished: response.writableFinished,
      generatedBytes }));
    response.writeHead(200, { 'content-type': saveType ? 'application/vnd.slither-neuroevo.save' : 'application/octet-stream',
      'content-disposition': `attachment; filename="${filename}"`,
      'content-length': chunks * chunkBytes, 'cache-control': 'no-store' });
    record('headers', { requestId, filename });
    /** One reused buffer plus stream backpressure bounds the server's memory. */
    async function* body(): AsyncGenerator<Buffer> {
      for (let index = 0; index < chunks; index++) {
        if (response.destroyed) return;
        generatedBytes += chunk.byteLength; generatedDigest.update(chunk);
        yield chunk;
      }
    }
    void pipeline(Readable.from(body()), response).catch((error: unknown) => {
      record('transferError', { requestId, message: error instanceof Error ? error.message : String(error) });
    });
  });
  await new Promise<void>((accept, reject) => {
    server.once('error', reject);
    server.listen(port, '127.0.0.1', accept);
  });
  /** All probe sockets are owned by this server and end at shutdown. */
  let closing = false;
  const stop = (): void => {
    if (closing) return;
    closing = true;
    server.closeAllConnections(); server.close();
  };
  const deadline = setTimeout(stop, 5 * 60 * 1000);
  const stopPoll = setInterval(() => {
    if (existsSync(resolve(outputRoot, 'stop'))) stop();
  }, 250);
  process.once('SIGINT', stop); process.once('SIGTERM', stop);
  console.log(`DOWNLOAD_PROBE_READY http://127.0.0.1:${port}/`);
  await new Promise<void>((accept) => server.once('close', accept));
  clearTimeout(deadline); clearInterval(stopPoll);
  process.removeListener('SIGINT', stop); process.removeListener('SIGTERM', stop);
  record('stopped', { requestNumber });
  await records;
}

void run().catch((error: unknown) => { console.error(error); process.exitCode = 1; });
