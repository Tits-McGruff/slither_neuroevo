import type { SavedBrainLibrary } from './savedBrainLibrary.ts';
import type { DistillationShadows } from './distillationShadows.ts';
import { MAX_BRAIN_ARTIFACT_BYTES } from '../../src/protocol/brainArtifacts.ts';
import type { IncomingMessage, ServerResponse } from 'node:http';
import { readJsonBody } from '../readJsonBody.ts';
import { DISTILLATION_ZERO, distillationInteger } from './distillationProtocol.ts';
import type { DistillationSpoolClient } from './distillationSpoolClient.ts';

/** Native arm/stop hooks use ordered authority commands, never actor join/action. */
export interface DistillationCaptureControl {
  /** Bounded student controls; absent when no native authority is connected. */
  shadows?: Pick<DistillationShadows, 'activate' | 'stop' | 'metrics'>;
  /** Admit one explicit capture request and return its dataset identity. */
  start(value: unknown): Promise<unknown>;
  /** Seal the current matching epoch without waiting for trainer consumption. */
  stop(captureEpoch: string): Promise<unknown>;
}

/** Dedicated bounded dataset transport; callers retain ordinary origin admission. */
export class DistillationHttpRouter {
  /** Retain download buffers until response completion/cancellation. */
  private downloads = 0;

  /** Bound complete in-flight routes, including control/readiness waits before worker requests. */
  private activeRequests = 0;

  /** One portable artifact upload owns bounded request bytes at a time. */
  private candidateUploads = 0;

  /** Keep filesystem/database work in the separate asynchronous worker interface. */
  constructor(
    private readonly spool: DistillationSpoolClient,
    private readonly control?: DistillationCaptureControl,
    private readonly library?: SavedBrainLibrary
  ) {}

  /** Handle only the dedicated namespace; unknown routes never fall through to game APIs. */
  handle(request: IncomingMessage, response: ServerResponse, url: URL): boolean {
    if (!url.pathname.startsWith('/api/distillation/v1/')) return false;
    if (this.activeRequests >= 32) {
      this.json(response, 429, {error: 'distillation HTTP request admission exhausted'}); return true;
    }
    this.activeRequests += 1;
    void this.route(request, response, url).catch(error => {
      if (response.destroyed || response.writableEnded) return;
      if (response.headersSent) { response.destroy(); return; }
      const reason = error instanceof Error ? error.message : String(error);
      const status = /outcome remains pending|native shadow applied; receipt persistence failed/u.test(reason) ? 503 :
        /not found|does not exist/u.test(reason) ? 404 : /unavailable|expired/u.test(reason) ? 410 :
        /quota|exhausted|already active|busy/u.test(reason) ? 429 : /worker|timed out|closing|outcome remains pending/u.test(reason) ? 503 : 400;
      this.json(response, status, { error: reason.slice(0, 1024) });
    }).finally(() => { this.activeRequests -= 1; });
    return true;
  }

  /** Read/lease/download paths return bounded immutable descriptors or opaque bytes. */
  private async route(request: IncomingMessage, response: ServerResponse, url: URL): Promise<void> {
    const path = url.pathname.slice('/api/distillation/v1/'.length);
    const method = request.method;
    if (path === 'library' || path.startsWith('library/')) {
      if (!this.library) throw new Error('saved brain library unavailable');
      /** Library records have a separately admitted full-graph metadata bound. */
      const json = (status: number, value: unknown): void => this.json(response, status, value, 1200 * 1024);
      if (method === 'GET' && path === 'library') {
        const limit = url.searchParams.get('limit') ?? '25';
        if (!/^[0-9]{1,2}$/u.test(limit)) throw new TypeError('invalid saved brain page limit');
        json(200, {...this.library.status(), page: await this.library.list(url.searchParams.get('after'),
          distillationInteger(Number(limit), 1, 25, 'saved brain page limit'))}); return;
      }
      if (method === 'POST' && path === 'library/import') {
        const library = this.library;
        json(201, await this.artifactUpload(request, bytes => library.importArtifact(bytes, {name: url.searchParams.get('name')}))); return;
      }
      const saved = /^library\/([0-9a-f]{64})(?:\/(save|spawn|export))?$/u.exec(path);
      if (saved) {
        const id = saved[1]!;
        if (method === 'GET' && !saved[2]) { json(200, await this.library.read(id)); return; }
        if (method === 'DELETE' && !saved[2]) { json(200, await this.library.delete(id)); return; }
        if (method === 'POST' && saved[2] === 'save') { json(201, await this.library.saveCandidate(id, await readJsonBody(request, 1024))); return; }
        if (method === 'POST' && saved[2] === 'spawn') { json(200, await this.library.spawn(id, await readJsonBody(request, 4096))); return; }
        if (method === 'GET' && saved[2] === 'export') {
          const snapshot = await this.library.export(id);
          let settled = false;
          /** Own the immutable export through both ordinary completion and peer cancellation. */
          const finish = (): void => {
            if (settled) return;
            settled = true;
            void snapshot.release().catch(() => response.destroy());
          };
          if (response.destroyed) { finish(); return; }
          response.once('finish', finish); response.once('close', finish);
          response.setTimeout(30_000, () => response.destroy());
          try {
            response.writeHead(200, {'Content-Type': 'application/octet-stream',
              'Content-Length': snapshot.bytes.byteLength, 'Content-Disposition': `attachment; filename="${id}.slither-brain"`,
              ETag: `"${id}"`, 'Cache-Control': 'no-store'});
            response.end(snapshot.bytes);
          } catch (error) { finish(); throw error; }
          return;
        }
      }
      json(404, {error: 'saved brain route not found'}); return;
    }
    if (method === 'GET' && path === 'shadows') {
      if (!this.control?.shadows) throw new Error('shadow control unavailable');
      this.json(response, 200, this.control.shadows.metrics()); return;
    }
    if (method === 'GET' && path === 'shadows/events') {
      this.json(response, 200, await this.spool.shadowEvents(url.searchParams.get('after') ?? DISTILLATION_ZERO, this.limit(url))); return;
    }
    const shadow = /^shadows\/([0-3])\/(activate|stop)$/u.exec(path);
    if (method === 'POST' && shadow) {
      if (!this.control?.shadows) throw new Error('shadow control unavailable');
      const body = await readJsonBody(request, 4096);
      const slot = Number(shadow[1]);
      this.json(response, 200, shadow[2] === 'activate' ? await this.control.shadows.activate(slot, body) : await this.control.shadows.stop(slot, body)); return;
    }
    const candidate = /^candidates\/([0-9a-f]{64})$/u.exec(path);
    if (candidate && method === 'POST') {
      this.json(response, 201, await this.artifactUpload(request, bytes => this.spool.ingestCandidate(candidate[1]!, bytes)));
      return;
    }
    if (candidate && method === 'GET') { this.json(response, 200, await this.spool.candidate(candidate[1]!)); return; }
    if (method === 'GET' && path === 'storage') {
      this.json(response, 200, await this.spool.storagePolicy()); return;
    }
    if (method === 'GET' && path === 'datasets') {
      const after = url.searchParams.get('after') ?? '0'.repeat(32);
      this.json(response, 200, await this.spool.datasets(after, this.limit(url)));
      return;
    }
    if (method === 'POST' && path === 'capture/start') {
      if (!this.control) throw new Error('capture control unavailable');
      this.json(response, 201, await this.control.start(await readJsonBody(request, 16 * 1024)));
      return;
    }
    const stop = /^capture\/([0-9a-f]{32})\/stop$/u.exec(path);
    if (method === 'POST' && stop) {
      if (!this.control) throw new Error('capture control unavailable');
      this.json(response, 200, await this.control.stop(stop[1]!)); return;
    }
    const dataset = /^datasets\/([0-9a-f]{32})\/manifest$/u.exec(path);
    if (method === 'GET' && dataset) { this.json(response, 200, await this.spool.dataset(dataset[1]!)); return; }
    const listing = /^datasets\/([0-9a-f]{32})\/chunks$/u.exec(path);
    if (method === 'GET' && listing) {
      this.json(response, 200, await this.spool.list(listing[1]!, url.searchParams.get('after') ?? DISTILLATION_ZERO, this.limit(url)));
      return;
    }
    const object = /^datasets\/([0-9a-f]{32})\/chunks\/([0-9a-f]{16})$/u.exec(path);
    if (method === 'GET' && object) {
      if (this.downloads >= 4) throw new Error('download concurrency exhausted');
      const lease = url.searchParams.get('lease');
      if (!lease) throw new TypeError('download requires storage lease');
      this.downloads += 1;
      let settled = false;
      /** Release response-owned bytes exactly once, including a cancelled download. */
      const finish = (): void => { if (!settled) { settled = true; this.downloads -= 1; } };
      response.once('finish', finish); response.once('close', finish);
      response.setTimeout(30_000, () => response.destroy());
      try {
        const chunk = await this.spool.download(object[1]!, object[2]!, lease);
        if (response.destroyed) { finish(); return; }
        response.writeHead(200, { 'Content-Type': 'application/octet-stream', 'Content-Length': chunk.bytes.byteLength,
          ETag: `"${chunk.descriptor.sha256}"`, 'X-Slither-Chunk-Sha256': chunk.descriptor.sha256,
          'Cache-Control': 'private, immutable, max-age=31536000' });
        response.end(chunk.bytes);
      } catch (error) { finish(); throw error; }
      return;
    }
    const leases = /^datasets\/([0-9a-f]{32})\/leases$/u.exec(path);
    if (method === 'POST' && leases) {
      const body = this.bodyRecord(await readJsonBody(request, 16 * 1024), ['sequences', 'durationMs']);
      if (!Array.isArray(body['sequences']) || body['sequences'].some(value => typeof value !== 'string')) throw new TypeError('invalid lease sequences');
      const duration = distillationInteger(body['durationMs'], 1, 300_000, 'lease duration');
      this.json(response, 201, await this.spool.lease(leases[1]!, body['sequences'] as string[], duration)); return;
    }
    const renew = /^leases\/([0-9a-f]{32})\/renew$/u.exec(path);
    if (method === 'POST' && renew) {
      const body = this.bodyRecord(await readJsonBody(request, 1024), ['durationMs']);
      const expiresAt = await this.spool.renewLease(renew[1]!, distillationInteger(body['durationMs'], 1, 300_000, 'lease duration'));
      this.json(response, 200, { expiresAt }); return;
    }
    const release = /^leases\/([0-9a-f]{32})$/u.exec(path);
    if (method === 'DELETE' && release) { await this.spool.releaseLease(release[1]!); this.json(response, 200, { released: true }); return; }
    this.json(response, 404, { error: 'distillation route not found' });
  }

  /** Candidate uploads and explicit library imports share one bounded opaque ingress owner. */
  private async artifactUpload(request: IncomingMessage, consume: (bytes: Buffer) => Promise<unknown>): Promise<unknown> {
    if (this.candidateUploads >= 1) throw new Error('portable brain upload concurrency exhausted');
    this.candidateUploads += 1;
    try {
      const type = request.headers['content-type']?.split(';')[0]?.trim();
      const length = request.headers['content-length'];
      if (type !== 'application/octet-stream' || typeof length !== 'string' || !/^[0-9]{1,9}$/u.test(length)) throw new TypeError('portable brain upload requires exact binary content length');
      const expected = distillationInteger(Number(length), 88, MAX_BRAIN_ARTIFACT_BYTES, 'portable brain bytes');
      request.setTimeout(30_000, () => request.destroy());
      const chunks: Buffer[] = [];
      let received = 0;
      for await (const value of request) {
        const chunk = value as Buffer;
        received += chunk.byteLength;
        if (received > expected) throw new TypeError('portable brain upload exceeds declared bytes');
        chunks.push(chunk);
      }
      if (received !== expected || request.aborted) throw new Error('portable brain upload incomplete');
      const bytes = Buffer.concat(chunks, received);
      chunks.length = 0;
      return await consume(bytes);
    } finally { this.candidateUploads -= 1; }
  }

  /** Require an integer page limit rather than forgiving partial numeric strings. */
  private limit(url: URL): number {
    const value = url.searchParams.get('limit') ?? '100';
    if (!/^[0-9]{1,3}$/u.test(value)) throw new TypeError('invalid page limit');
    return distillationInteger(Number(value), 1, 256, 'page limit');
  }

  /** Strict small request records cannot smuggle alternate command/payload fields. */
  private bodyRecord(value: unknown, keys: string[]): Record<string, unknown> {
    if (!value || typeof value !== 'object' || Array.isArray(value) || Object.keys(value).length !== keys.length ||
        keys.some(key => !Object.hasOwn(value, key))) throw new TypeError('invalid distillation request fields');
    return value as Record<string, unknown>;
  }

  /** Bound metadata responses independently of binary transfer ownership. */
  private json(response: ServerResponse, status: number, value: unknown, maxBytes = 1024 * 1024): void {
    if (response.destroyed) return;
    const encoded = JSON.stringify(value);
    if (Buffer.byteLength(encoded) > maxBytes) throw new Error('distillation metadata response limit');
    response.writeHead(status, { 'Content-Type': 'application/json', 'Cache-Control': 'no-store' });
    response.end(encoded);
  }
}
