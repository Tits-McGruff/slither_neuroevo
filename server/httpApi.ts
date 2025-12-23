import type { IncomingMessage, ServerResponse } from 'node:http';
import type { World } from '../src/world.ts';
import type { PopulationImportData } from '../src/protocol/messages.ts';
import type { GraphSpec } from '../src/brains/graph/schema.ts';
import { validateSnapshotPayload, type Persistence, type PopulationSnapshotPayload } from './persistence.ts';

const MAX_BODY_BYTES = 50 * 1024 * 1024;

export interface HttpApiDeps {
  getStatus: () => { tick: number; clients: number };
  getWorld: () => World | null;
  importPopulation: (data: PopulationImportData) => {
    ok: boolean;
    reason?: string;
    used?: number;
    total?: number;
  };
  persistence: Persistence;
  cfgHash: string;
  worldSeed: number;
}

export function createHttpHandler(deps: HttpApiDeps): (req: IncomingMessage, res: ServerResponse) => void {
  return (req, res) => {
    void handleRequest(req, res, deps);
  };
}

function applyCors(req: IncomingMessage, res: ServerResponse): void {
  const origin = req.headers.origin;
  if (origin) {
    res.setHeader('Access-Control-Allow-Origin', origin);
    res.setHeader('Vary', 'Origin');
  } else {
    res.setHeader('Access-Control-Allow-Origin', '*');
  }
  res.setHeader('Access-Control-Allow-Methods', 'GET,POST,OPTIONS');
  res.setHeader('Access-Control-Allow-Headers', 'Content-Type');
}

async function handleRequest(
  req: IncomingMessage,
  res: ServerResponse,
  deps: HttpApiDeps
): Promise<void> {
  applyCors(req, res);
  if (req.method === 'OPTIONS') {
    res.statusCode = 204;
    res.end();
    return;
  }
  const url = new URL(req.url ?? '/', 'http://localhost');
  if (req.method === 'GET' && url.pathname === '/health') {
    const status = deps.getStatus();
    sendJson(res, 200, { ok: true, tick: status.tick, clients: status.clients });
    return;
  }

  if (req.method === 'POST' && url.pathname === '/api/save') {
    const world = deps.getWorld();
    if (!world) {
      sendJson(res, 503, { ok: false, message: 'world not ready' });
      return;
    }
    try {
      const snapshot = buildSnapshotPayload(world, deps.cfgHash, deps.worldSeed);
      const snapshotId = deps.persistence.saveSnapshot(snapshot);
      sendJson(res, 200, { ok: true, snapshotId });
    } catch (err) {
      sendJson(res, 500, { ok: false, message: (err as Error).message });
    }
    return;
  }

  if (req.method === 'GET' && url.pathname === '/api/export/latest') {
    const snapshot = deps.persistence.loadLatestSnapshot();
    if (!snapshot) {
      sendJson(res, 404, { ok: false, message: 'no snapshots' });
      return;
    }
    sendJson(res, 200, snapshot);
    return;
  }

  if (req.method === 'POST' && url.pathname === '/api/import') {
    const body = await readJsonBody(req, MAX_BODY_BYTES).catch((err: Error) => {
      sendJson(res, 400, { ok: false, message: err.message });
      return null;
    });
    if (!body) return;

    const force = Boolean((body as { force?: boolean }).force) || url.searchParams.get('force') === '1';
    const payload = extractPayload(body);
    try {
      validateSnapshotPayload(payload);
    } catch (err) {
      sendJson(res, 400, { ok: false, message: (err as Error).message });
      return;
    }
    if (payload.cfgHash !== deps.cfgHash && !force) {
      sendJson(res, 409, {
        ok: false,
        message: 'cfgHash mismatch; pass force=true to override'
      });
      return;
    }
    const importData: PopulationImportData = {
      generation: payload.generation,
      archKey: payload.archKey,
      genomes: payload.genomes
    };
    const result = deps.importPopulation(importData);
    if (!result.ok) {
      sendJson(res, 400, { ok: false, message: result.reason ?? 'import failed' });
      return;
    }
    sendJson(res, 200, { ok: true, used: result.used ?? 0, total: result.total ?? 0 });
    return;
  }

  if (req.method === 'GET' && url.pathname === '/api/graph-presets') {
    const limitRaw = url.searchParams.get('limit');
    const parsedLimit = Number(limitRaw);
    const limit = Number.isFinite(parsedLimit)
      ? Math.min(200, Math.max(1, parsedLimit))
      : 50;
    const presets = deps.persistence.listGraphPresets(limit);
    sendJson(res, 200, { ok: true, presets });
    return;
  }

  if (req.method === 'GET' && url.pathname.startsWith('/api/graph-presets/')) {
    const idRaw = url.pathname.split('/').pop() ?? '';
    const id = Number(idRaw);
    if (!Number.isFinite(id)) {
      sendJson(res, 400, { ok: false, message: 'preset id must be a number' });
      return;
    }
    try {
      const preset = deps.persistence.loadGraphPreset(id);
      if (!preset) {
        sendJson(res, 404, { ok: false, message: 'preset not found' });
        return;
      }
      sendJson(res, 200, { ok: true, preset });
    } catch (err) {
      sendJson(res, 400, { ok: false, message: (err as Error).message });
    }
    return;
  }

  if (req.method === 'POST' && url.pathname === '/api/graph-presets') {
    const body = await readJsonBody(req, MAX_BODY_BYTES).catch((err: Error) => {
      sendJson(res, 400, { ok: false, message: err.message });
      return null;
    });
    if (!body) return;
    const name = typeof (body as { name?: unknown }).name === 'string'
      ? (body as { name: string }).name.trim()
      : '';
    if (!name) {
      sendJson(res, 400, { ok: false, message: 'preset name is required' });
      return;
    }
    const spec = (body as { spec?: GraphSpec }).spec;
    if (!spec || typeof spec !== 'object') {
      sendJson(res, 400, { ok: false, message: 'preset spec is required' });
      return;
    }
    try {
      const presetId = deps.persistence.saveGraphPreset(name, spec);
      sendJson(res, 200, { ok: true, presetId });
    } catch (err) {
      sendJson(res, 400, { ok: false, message: (err as Error).message });
    }
    return;
  }

  res.statusCode = 404;
  res.end('Not found');
}

function sendJson(res: ServerResponse, status: number, payload: unknown): void {
  res.statusCode = status;
  res.setHeader('Content-Type', 'application/json');
  res.end(JSON.stringify(payload));
}

async function readJsonBody(req: IncomingMessage, limitBytes: number): Promise<unknown> {
  const chunks: Buffer[] = [];
  let total = 0;
  for await (const chunk of req) {
    const buf = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk as ArrayBuffer);
    total += buf.length;
    if (total > limitBytes) {
      throw new Error('payload too large');
    }
    chunks.push(buf);
  }
  const text = Buffer.concat(chunks).toString('utf8');
  if (!text) return {};
  return JSON.parse(text) as unknown;
}

function extractPayload(body: unknown): PopulationSnapshotPayload {
  if (body && typeof body === 'object' && 'payload' in body) {
    const payload = (body as { payload?: PopulationSnapshotPayload }).payload;
    if (payload) return payload;
  }
  return body as PopulationSnapshotPayload;
}

function buildSnapshotPayload(
  world: World,
  cfgHash: string,
  worldSeed: number
): PopulationSnapshotPayload {
  const exportData = world.exportPopulation();
  return {
    ...exportData,
    cfgHash,
    worldSeed
  };
}
