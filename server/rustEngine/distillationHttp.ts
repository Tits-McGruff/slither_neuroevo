import { MAX_BRAIN_ARTIFACT_BYTES } from '../../src/protocol/brainArtifacts.ts';
import type { IncomingMessage, ServerResponse } from 'node:http';
import { readJsonBody } from '../readJsonBody.ts';
import { DISTILLATION_ZERO, distillationInteger } from './distillationProtocol.ts';
import type { DistillationSpoolClient } from './distillationSpoolClient.ts';

/** Native arm/stop hooks use ordered authority commands, never actor join/action. */
export interface DistillationCaptureControl {
  /** Admit one explicit capture request and return its dataset identity. */
  start(value: unknown): Promise<unknown>;
  /** Seal the current matching epoch without waiting for trainer consumption. */
  stop(captureEpoch: string): Promise<unknown>;
}

/** Dedicated bounded dataset transport; callers retain ordinary origin admission. */
export class DistillationHttpRouter {
  /** Retain download buffers until response completion/cancellation. */
  private downloads = 0;

  /** One candidate upload owns bounded request bytes at a time. */
  private candidateUploads = 0;

  /** Keep filesystem/database work in the separate asynchronous worker interface. */
  constructor(
    private readonly spool: DistillationSpoolClient,
    private readonly control?: DistillationCaptureControl
  ) {}

  /** Handle only the dedicated namespace; unknown routes never fall through to game APIs. */
  handle(request: IncomingMessage, response: ServerResponse, url: URL): boolean {
    if (!url.pathname.startsWith('/api/distillation/v1/')) return false;
    void this.route(request, response, url).catch(error => {
      if (response.destroyed || response.writableEnded) return;
      if (response.headersSent) { response.destroy(); return; }
      const reason = error instanceof Error ? error.message : String(error);
      const status = /not found/u.test(reason) ? 404 : /unavailable|expired/u.test(reason) ? 410 :
        /quota|exhausted|already active/u.test(reason) ? 429 : /worker|timed out|closing/u.test(reason) ? 503 : 400;
      this.json(response, status, { error: reason.slice(0, 1024) });
    });
    return true;
  }

  /** Read/lease/download paths return bounded immutable descriptors or opaque bytes. */
  private async route(request: IncomingMessage, response: ServerResponse, url: URL): Promise<void> {
    const path = url.pathname.slice('/api/distillation/v1/'.length);
    const method = request.method;
    const candidate = /^candidates\/([0-9a-f]{64})$/u.exec(path);
    if (candidate && method === 'POST') {
      if (this.candidateUploads >= 1) throw new Error('candidate upload concurrency exhausted');
      this.candidateUploads += 1;
      try {
        const type = request.headers['content-type']?.split(';')[0]?.trim();
        const length = request.headers['content-length'];
        if (type !== 'application/octet-stream' || typeof length !== 'string' || !/^[0-9]{1,9}$/u.test(length)) throw new TypeError('candidate upload requires exact binary content length');
        const expected = distillationInteger(Number(length), 88, MAX_BRAIN_ARTIFACT_BYTES, 'candidate bytes');
        request.setTimeout(30_000, () => request.destroy());
        const chunks: Buffer[] = [];
        let received = 0;
        for await (const value of request) {
          const chunk = value as Buffer;
          received += chunk.byteLength;
          if (received > expected) throw new TypeError('candidate upload exceeds declared bytes');
          chunks.push(chunk);
        }
        if (received !== expected || request.aborted) throw new Error('candidate upload incomplete');
        const bytes = Buffer.concat(chunks, received);
        chunks.length = 0;
        this.json(response, 201, await this.spool.ingestCandidate(candidate[1]!, bytes));
      } finally { this.candidateUploads -= 1; }
      return;
    }
    if (candidate && method === 'GET') { this.json(response, 200, await this.spool.candidate(candidate[1]!)); return; }
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
  private json(response: ServerResponse, status: number, value: unknown): void {
    if (response.destroyed) return;
    const encoded = JSON.stringify(value);
    if (Buffer.byteLength(encoded) > 1024 * 1024) throw new Error('distillation metadata response limit');
    response.writeHead(status, { 'Content-Type': 'application/json', 'Cache-Control': 'no-store' });
    response.end(encoded);
  }
}
