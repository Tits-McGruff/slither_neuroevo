import type { IncomingMessage, ServerResponse } from 'node:http';
import type { World } from '../src/world.ts';
import type { HallOfFameEntry, PopulationImportData } from '../src/protocol/messages.ts';
import type { GraphSpec } from '../src/brains/graph/schema.ts';
import { validateSnapshotPayload, type Persistence, type PopulationSnapshotPayload } from './persistence.ts';
import { buildCoreSettingsSnapshot, buildSettingsUpdatesSnapshot } from './settingsSnapshot.ts';

/** Hard limit for incoming request bodies to avoid memory pressure. */
const MAX_BODY_BYTES = 50 * 1024 * 1024;

/** Dependencies injected into the HTTP API handler. */
export interface HttpApiDeps {
  /** Returns the current server status for health checks. */
  getStatus: () => { tick: number; clients: number };
  /** Returns the current world instance, or null if not ready. */
  getWorld: () => World | null;
  /** Imports a population snapshot into the active world. */
  importPopulation: (data: PopulationImportData) => {
    ok: boolean;
    reason?: string;
    used?: number;
    total?: number;
  };
  /** Persistence adapter for snapshots and graph presets. */
  persistence: Persistence;
  /** Hash of the active server configuration. */
  cfgHash: string;
  /** Seed used to initialize the world. */
  worldSeed: number;
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
 * Check if a given origin corresponds to a LAN or local environment.
 * @param origin - Origin header to check.
 * @returns True if the origin is on the LAN.
 */
function isLanOrigin(origin: string | undefined): boolean {
  if (!origin) return false;
  try {
    const { hostname } = new URL(origin);
    // Localhost and loopback
    if (hostname === 'localhost' || hostname === '127.0.0.1' || hostname === '::1') return true;

    // Check for IPv4 private ranges:
    // 10.0.0.0 - 10.255.255.255
    // 172.16.0.0 - 172.31.255.255 (also allowing all 172.x for common container/VM bridges)
    // 192.168.0.0 - 192.168.255.255
    const parts = hostname.split('.').map(Number);
    if (parts.length === 4 && parts.every((p) => !isNaN(p) && p >= 0 && p <= 255)) {
      if (parts[0] === 10) return true;
      if (parts[0] === 172) return true;
      if (parts[0] === 192 && parts[1] === 168) return true;
    }

    // Hostnames without dots are typically local network machine names
    if (!hostname.includes('.')) return true;

    return false;
  } catch {
    return false;
  }
}

/**
 * Adds CORS headers for browser clients.
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
    // If not a whitelisted LAN origin, default to allow-all (*) for non-credentialed requests
    // but log a warning if it's a credentialed request from outside.
    res.setHeader('Access-Control-Allow-Origin', '*');
  }
  res.setHeader('Access-Control-Allow-Methods', 'GET,POST,OPTIONS');
  res.setHeader('Access-Control-Allow-Headers', 'Content-Type');
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

    body.hof.forEach((entry) => {
      deps.persistence.saveHofEntry(entry);
    });

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

/**
 * Builds a snapshot payload from the active world and config metadata.
 * @param world - World instance used for export.
 * @param cfgHash - Active configuration hash.
 * @param worldSeed - Seed used for world initialization.
 * @returns Snapshot payload to persist.
 */
function buildSnapshotPayload(
  world: World,
  cfgHash: string,
  worldSeed: number
): PopulationSnapshotPayload {
  const exportData = world.exportPopulation();
  const settings = buildCoreSettingsSnapshot(world);
  const updates = buildSettingsUpdatesSnapshot();
  return {
    ...exportData,
    cfgHash,
    worldSeed,
    settings,
    updates
  };
}
