import type { IncomingMessage, ServerResponse } from 'node:http';
import { once } from 'node:events';
import type { World } from '../src/world.ts';
import type { GenomeJSON, HallOfFameEntry, PopulationImportData } from '../src/protocol/messages.ts';
import type { GraphSpec } from '../src/brains/graph/schema.ts';
import { validateSnapshotPayload, type Persistence, type PopulationSnapshotPayload } from './persistence.ts';
import type { Logger } from './logger.ts';
import type { InferenceModeRecord } from './inferenceMode.ts';
import type { SchedulerDiagnostics, SimulationRunIdentity } from '../src/sim/SimCore.ts';
import type { SimulationFaultStatus } from './simServer.ts';
import type { SpatialHashDiagnostics } from '../src/spatialHash.ts';

/** Hard limit for incoming request bodies to avoid memory pressure. */
const MAX_BODY_BYTES = 50 * 1024 * 1024;
/** Upper bound on genome weight array length for resurrect requests. */
const MAX_RESURRECT_WEIGHTS = 2_000_000;

/** Dependencies injected into the HTTP API handler. */
export interface HttpApiDeps {
  /** Returns the current server status for health checks. */
  getStatus: () => {
    tick: number;
    clients: number;
    inferenceMode: InferenceModeRecord;
    scheduler: SchedulerDiagnostics;
    collisionGrid: SpatialHashDiagnostics;
    fault: SimulationFaultStatus;
    run: SimulationRunIdentity;
    configRevision: number;
    configHash: string;
  };
  /** Returns the current world instance, or null if not ready. */
  getWorld: () => World | null;
  /** Imports a population snapshot into the active world. */
  importPopulation: (data: PopulationImportData) => Promise<{
    ok: boolean;
    reason?: string;
    used?: number;
    total?: number;
  }>;
  /** Persistence adapter for snapshots and graph presets. */
  persistence: Persistence;
  /** Persist the active population through the typed non-resumable export path. */
  savePopulationExport: () => number;
  /** Returns the hash of the active server configuration. */
  getConfigHash: () => string;
  /** Returns the active world seed. */
  getWorldSeed: () => number;
  /** Optional logger for error reporting. */
  logger?: Logger | undefined;
}

/**
 * Builds the HTTP handler that serves API requests and health checks.
 * @param deps - API dependencies and persistence adapters.
 * @returns Request handler function.
 */
export function createHttpHandler(deps: HttpApiDeps): (req: IncomingMessage, res: ServerResponse) => void {
  return (req, res) => {
    void handleRequest(req, res, deps);
  };
}

/**
 * Check whether an origin belongs to a loopback or trusted-LAN address shape.
 * This routing check is not authentication or an internet-facing security boundary.
 * @param origin - Origin header to check.
 * @returns True when the origin is local or LAN-shaped.
 */
function isLanOrigin(origin: string | undefined): boolean {
  if (!origin) return false;
  try {
    const { hostname } = new URL(origin);
    if (hostname === 'localhost' || hostname === '127.0.0.1' || hostname === '::1') return true;

    const parts = hostname.split('.').map(Number);
    if (parts.length === 4 && parts.every((part) => !Number.isNaN(part) && part >= 0 && part <= 255)) {
      if (parts[0] === 10) return true;
      if (parts[0] === 172) return true;
      if (parts[0] === 192 && parts[1] === 168) return true;
    }

    if (!hostname.includes('.')) return true;
    return false;
  } catch {
    return false;
  }
}

/**
 * Add CORS headers for browser clients on loopback or a trusted LAN.
 * @param req - Incoming request.
 * @param res - Server response.
 */
function applyCors(req: IncomingMessage, res: ServerResponse): void {
  const origin = req.headers.origin;
  if (origin && isLanOrigin(origin)) {
    res.setHeader('Access-Control-Allow-Origin', origin);
    res.setHeader('Access-Control-Allow-Credentials', 'true');
    res.setHeader('Vary', 'Origin');
  } else {
    res.setHeader('Access-Control-Allow-Origin', '*');
  }
  res.setHeader('Access-Control-Allow-Methods', 'GET, POST, OPTIONS, PUT, DELETE');
  res.setHeader('Access-Control-Allow-Headers', 'Content-Type, Authorization, X-Requested-With, Accept');
  res.setHeader('Access-Control-Max-Age', '86400');
}

/**
 * Routes incoming HTTP requests to the correct handler.
 * @param req - Incoming request.
 * @param res - Server response.
 * @param deps - API dependencies and persistence adapters.
 */
async function handleRequest(
  req: IncomingMessage,
  res: ServerResponse,
  deps: HttpApiDeps
): Promise<void> {
  try {
    applyCors(req, res);
    if (req.method === 'OPTIONS') {
      res.statusCode = 204;
      res.end();
      return;
    }
    await routeRequest(req, res, deps);
  } catch (err) {
    const message = (err as Error).message || 'internal server error';
    deps.logger?.error('http', `Request error: ${message}`);
    // If headers haven't been sent yet, we can send a 500.
    if (!res.headersSent) {
      applyCors(req, res);
      sendJson(res, 500, { ok: false, message });
    } else {
      res.end();
    }
  }
}

/**
 * Check whether a value is a plain object.
 * @param value - Value to inspect.
 * @returns True when value is a non-null object.
 */
function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

/**
 * Check whether a value is a finite number.
 * @param value - Value to inspect.
 * @returns True when value is a finite number.
 */
function isFiniteNumber(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value);
}

/**
 * Validate a genome JSON payload for resurrect requests.
 * @param value - Raw genome payload.
 * @returns True when the payload matches the expected genome shape.
 */
function isGenomeJson(value: unknown): value is GenomeJSON {
  if (!isRecord(value)) return false;
  const archKey = value['archKey'];
  if (typeof archKey !== 'string' || !archKey.trim()) return false;
  const weights = value['weights'];
  if (!Array.isArray(weights)) return false;
  if (weights.length > MAX_RESURRECT_WEIGHTS) return false;
  for (const w of weights) {
    if (!isFiniteNumber(w)) return false;
  }
  if ('brainType' in value && value['brainType'] !== undefined && typeof value['brainType'] !== 'string') {
    return false;
  }
  if ('fitness' in value && value['fitness'] !== undefined && !isFiniteNumber(value['fitness'])) {
    return false;
  }
  return true;
}

/**
 * Routes incoming HTTP requests to handlers (internal implementation).
 */
async function routeRequest(
  req: IncomingMessage,
  res: ServerResponse,
  deps: HttpApiDeps
): Promise<void> {
  const url = new URL(req.url ?? '/', 'http://localhost');
  if (req.method === 'GET' && url.pathname === '/health') {
    const status = deps.getStatus();
    sendJson(res, 200, { ok: true, ...status });
    return;
  }

  if (req.method === 'POST' && url.pathname === '/api/save') {
    try {
      const snapshotId = deps.savePopulationExport();
      sendJson(res, 200, { ok: true, snapshotId });
    } catch (err) {
      sendJson(res, 500, { ok: false, message: (err as Error).message });
    }
    return;
  }

  if (req.method === 'GET' && url.pathname === '/api/export/latest') {
    const snapshotId = deps.persistence.getLatestSnapshotId();
    if (snapshotId === null) {
      sendJson(res, 404, { ok: false, message: 'no snapshots' });
      return;
    }
    await sendJsonChunks(res, deps.persistence.exportSnapshotJsonChunks(snapshotId));
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
    if (payload.cfgHash !== deps.getConfigHash() && !force) {
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
    const result = await deps.importPopulation(importData);
    if (!result.ok) {
      sendJson(res, 400, { ok: false, message: result.reason ?? 'import failed' });
      return;
    }
    sendJson(res, 200, {
      ok: true,
      used: result.used ?? 0,
      total: result.total ?? 0,
      importedWorldSeed: payload.worldSeed,
      activeWorldSeed: deps.getWorldSeed(),
      seedApplied: false,
      seedDisposition: 'metadata-only; active run seed is unchanged'
    });
    return;
  }

  if (req.method === 'POST' && url.pathname === '/api/resurrect') {
    const world = deps.getWorld();
    if (!world) {
      sendJson(res, 503, { ok: false, message: 'world not ready' });
      return;
    }
    const body = await readJsonBody(req, MAX_BODY_BYTES).catch((err: Error) => {
      sendJson(res, 400, { ok: false, message: err.message });
      return null;
    });
    if (!body) return;
    const payload = isRecord(body) && 'genome' in body ? (body as { genome?: unknown }).genome : body;
    if (!isGenomeJson(payload)) {
      sendJson(res, 400, { ok: false, message: 'invalid genome payload' });
      return;
    }
    try {
      const snakeId = world.resurrect(payload);
      sendJson(res, 200, { ok: true, snakeId });
    } catch (err) {
      sendJson(res, 400, { ok: false, message: (err as Error).message });
    }
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

  if (req.method === 'GET' && url.pathname === '/api/hof') {
    const limitRaw = url.searchParams.get('limit');
    const limit = Number.parseInt(limitRaw ?? '50', 10) || 50;
    const entries = deps.persistence.loadHofEntries(limit);
    sendJson(res, 200, { ok: true, hof: entries });
    return;
  }

  if (req.method === 'POST' && url.pathname === '/api/hof') {
    const body = (await readJsonBody(req, MAX_BODY_BYTES).catch((err: Error) => {
      sendJson(res, 400, { ok: false, message: err.message });
      return null;
    })) as { hof?: HallOfFameEntry[] } | null;
    if (!body) return;

    if (!Array.isArray(body.hof)) {
      sendJson(res, 400, { ok: false, message: 'invalid hof payload' });
      return;
    }

    deps.persistence.saveHofEntries(body.hof);

    sendJson(res, 200, { ok: true });
    return;
  }

  res.statusCode = 404;
  res.end('Not found');
}

/**
 * Sends a JSON response with status code and payload.
 * @param res - Server response.
 * @param status - HTTP status code.
 * @param payload - JSON payload to serialize.
 */
function sendJson(res: ServerResponse, status: number, payload: unknown): void {
  res.statusCode = status;
  res.setHeader('Content-Type', 'application/json');
  res.end(JSON.stringify(payload));
}

/**
 * Write a JSON chunk iterable while honoring Node response backpressure.
 * @param res - HTTP response receiving the export.
 * @param chunks - Incremental JSON chunks bounded to one genome at a time.
 */
async function sendJsonChunks(res: ServerResponse, chunks: Iterable<string>): Promise<void> {
  const iterator = chunks[Symbol.iterator]();
  const first = iterator.next();
  res.statusCode = 200;
  res.setHeader('Content-Type', 'application/json');
  let next = first;
  while (!next.done) {
    if (res.destroyed) throw new Error('export client disconnected');
    if (!res.write(next.value)) await once(res, 'drain');
    next = iterator.next();
  }
  res.end();
}

/**
 * Reads a JSON payload with a strict size limit.
 * @param req - Incoming request.
 * @param limitBytes - Maximum allowed payload size.
 * @returns Parsed JSON payload.
 */
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

/**
 * Extracts a snapshot payload from wrapper objects.
 * @param body - Incoming JSON body.
 * @returns Snapshot payload object.
 */
function extractPayload(body: unknown): PopulationSnapshotPayload {
  if (body && typeof body === 'object' && 'payload' in body) {
    const payload = (body as { payload?: PopulationSnapshotPayload }).payload;
    if (payload) return payload;
  }
  return body as PopulationSnapshotPayload;
}
