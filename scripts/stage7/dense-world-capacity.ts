/** Prepare a long-body capacity case through supported legacy startup and ordinary Rust growth. */
import { createHash } from 'node:crypto';
import { existsSync } from 'node:fs';
import { mkdir, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import Database from 'better-sqlite3';
import WebSocket from 'ws';
import { DEFAULT_CONFIG } from '../../server/config.ts';
import { startRustServer } from '../../server/rustServer.ts';
import { compileGraph } from '../../src/brains/graph/compiler.ts';
import type { GraphSpec } from '../../src/brains/graph/types.ts';

/** Required actual segment count for the historical spatial-capacity boundary. */
const REQUIRED_SEGMENTS = 200_000;
/** Crowded preparation stays within a bounded per-snake length that can fit the larger arena. */
const CROWDED_MAXIMUM_LENGTH = 1000;
/** Supported stress settings, retained verbatim in the report. */
const UPDATES = [
  { path: 'generationSeconds', value: 480 },
  { path: 'observer.earlyEndMinSeconds', value: 50 },
  { path: 'observer.earlyEndAliveThreshold', value: 1 },
  { path: 'worldRadius', value: 800 },
  { path: 'pelletCountTarget', value: 25000 },
  { path: 'pelletSpawnPerSecond', value: 3500 },
  { path: 'foodValue', value: 8 },
  { path: 'growPerFood', value: 10 },
  { path: 'baselineBots.count', value: 0 },
  { path: 'snakeBaseSpeed', value: 30 },
  { path: 'snakeBoostSpeed', value: 40 },
  { path: 'snakeTurnRate', value: 14 },
  { path: 'snakeRadius', value: 3 },
  { path: 'snakeRadiusMax', value: 4 },
  { path: 'snakeThicknessScale', value: 0 },
  { path: 'snakeSpacing', value: 3 },
  { path: 'snakeStartLen', value: 5 },
  { path: 'snakeMaxLen', value: 100000 },
  { path: 'snakeSizeSpeedPenalty', value: 0 },
  { path: 'snakeBoostSizePenalty', value: 0 },
  { path: 'collision.skipSegments', value: 30 },
  { path: 'collision.hitScale', value: 0.45 },
  { path: 'boost.pointsCostPerSecond', value: 0 },
  { path: 'boost.lenLossPerPoint', value: 0 }
];

/** Scalar facts decoded from one actual binary frame without retaining body arrays. */
interface FrameCounts {
  /** Generation encoded by the published frame. */
  generation: number;
  /** All active snakes represented by the frame. */
  alive: number;
  /** Sum of actual body coordinates in this frame. */
  bodyPoints: number;
  /** Sum of adjacent-point segments, excluding each body's initial point. */
  bodySegments: number;
  /** Longest actual represented body. */
  longestBody: number;
  /** Actual represented pellets. */
  pellets: number;
  /** Exact binary transport size. */
  bytes: number;
  /** Conservative collision segments with both endpoints inside the arena, when audited. */
  inArenaCollisionSegments?: number;
  /** Number of represented points outside the arena, when audited. */
  outsideArenaPoints?: number;
}

/** Validate and count every variable-length frame record, including the final pellet section. */
export function frameCounts(bytes: Buffer, auditGeometry = false): FrameCounts {
  if (bytes.length < 32 || bytes.length % 4 !== 0) throw new Error('invalid capacity frame size');
  const floats = bytes.length / 4;
  /** Read an exact non-negative bounded integer field. */
  const integer = (offset: number): number => {
    if (offset >= floats) throw new Error('truncated capacity frame');
    const value = bytes.readFloatLE(offset * 4);
    if (!Number.isSafeInteger(value) || value < 0) throw new Error('invalid capacity frame count');
    return value;
  };
  const generation = integer(0);
  const alive = integer(2);
  let offset = 7;
  let bodyPoints = 0;
  let bodySegments = 0;
  let longestBody = 0;
  let inArenaCollisionSegments = 0;
  let outsideArenaPoints = 0;
  const worldRadiusSquared = bytes.readFloatLE(3 * 4) ** 2;
  for (let snake = 0; snake < alive; snake++) {
    const points = integer(offset + 7);
    if (auditGeometry) {
      if (offset + 8 + points * 2 >= floats) throw new Error('truncated audited capacity body');
      let previousInside = false;
      for (let point = 0; point < points; point++) {
        const coordinate = (offset + 8 + point * 2) * 4;
        const x = bytes.readFloatLE(coordinate);
        const y = bytes.readFloatLE(coordinate + 4);
        if (!Number.isFinite(x) || !Number.isFinite(y)) throw new Error('nonfinite capacity body');
        const inside = x * x + y * y <= worldRadiusSquared;
        if (!inside) outsideArenaPoints++;
        // The fixture skips the first 30 points for head/body collision indexing.
        if (point >= 30 && previousInside && inside) inArenaCollisionSegments++;
        previousInside = inside;
      }
    }
    offset += 8 + points * 2;
    if (offset >= floats) throw new Error('truncated capacity body');
    bodyPoints += points;
    bodySegments += Math.max(0, points - 1);
    longestBody = Math.max(longestBody, points);
  }
  const pellets = integer(offset);
  if (offset + 1 + pellets * 5 !== floats) throw new Error('invalid capacity pellet section');
  return { generation, alive, bodyPoints, bodySegments, longestBody, pellets, bytes: bytes.length,
    ...(auditGeometry ? { inArenaCollisionSegments, outsideArenaPoints } : {}) };
}

/** Write one bounded genome at a time for normal population-only SQLite conversion. */
function createSource(databasePath: string, browserLan = false): Record<string, unknown> {
  const graphSpec: GraphSpec = { type: 'graph',
    nodes: [{ id: 'input', type: 'Input', outputSize: 83 },
      { id: 'output', type: 'Dense', inputSize: 83, outputSize: 2 }],
    edges: [{ from: 'input', to: 'output' }], outputs: [{ nodeId: 'output' }], outputSize: 2 };
  const graph = compileGraph(graphSpec);
  if (graph.totalParams !== 168) throw new Error('capacity graph layout changed');
  const crowdedOverrides: Record<string, number> = { worldRadius: 10000, snakeBaseSpeed: 650,
    snakeBoostSpeed: 650, snakeStartLen: 5, snakeMaxLen: CROWDED_MAXIMUM_LENGTH };
  const updates = browserLan ? UPDATES.map(update => ({ ...update,
    value: crowdedOverrides[update.path] ?? update.value })) : UPDATES;
  const metadata = { formatVersion: 2, boundaryVersion: 1, boundaryKind: 'run-start',
    resumable: true, generation: 1, simulationStep: 0, worldSeed: 1511506142,
    runId: 'dense-world-capacity-source', configHash: 'dense-world-capacity-fixture', configRevision: 1,
    archKey: graph.key, graphSpec, populationCount: 300,
    settings: { snakeCount: 300, simSpeed: browserLan ? 1 : 12 }, updates,
    rng: {}, allocators: {}, bestFitnessEver: 0, fitnessHistory: [], lastHofEntry: null };
  const database = new Database(databasePath);
  try {
    database.pragma('journal_mode = WAL');
    database.pragma('synchronous = FULL');
    database.exec(`CREATE TABLE population_snapshots (
      id INTEGER PRIMARY KEY, payload_json TEXT, format_version INTEGER,
      boundary_kind TEXT, population_count INTEGER
    ); CREATE TABLE snapshot_genomes (
      snapshot_id INTEGER NOT NULL, slot INTEGER NOT NULL, arch_key TEXT NOT NULL,
      brain_type TEXT NOT NULL, fitness REAL NOT NULL, weight_count INTEGER NOT NULL,
      weights_blob BLOB NOT NULL, weights_checksum TEXT NOT NULL,
      PRIMARY KEY (snapshot_id, slot)
    )`);
    const insert = database.prepare(`INSERT INTO snapshot_genomes
      (snapshot_id, slot, arch_key, brain_type, fitness, weight_count, weights_blob, weights_checksum)
      VALUES (1, ?, ?, 'mlp', 0, ?, ?, ?)`);
    database.transaction(() => {
      database.prepare(`INSERT INTO population_snapshots
        (id, payload_json, format_version, boundary_kind, population_count)
        VALUES (1, ?, 2, 'run-start', 300)`).run(JSON.stringify(metadata));
      for (let slot = 0; slot < 300; slot++) {
        const weights = Buffer.alloc(graph.totalParams * 4);
        // Dense row-major weights followed by the two biases. Turn toward nearest food
        // and away from low-clearance body/wall bins; keep boost off while bodies grow.
        weights.writeFloatLE(4 + slot / 300, 14 * 4);
        for (let bin = 0; bin < 16; bin++) {
          const steering = Math.sin(-Math.PI + (bin + 0.5) * Math.PI / 8);
          weights.writeFloatLE(steering, (35 + bin) * 4);
          weights.writeFloatLE(steering, (51 + bin) * 4);
        }
        weights.writeFloatLE(-4, 167 * 4);
        insert.run(slot, graph.key, graph.totalParams, weights,
          createHash('sha256').update(weights).digest('hex'));
      }
    })();
    database.pragma('wal_checkpoint(TRUNCATE)');
  } finally { database.close(); }
  return metadata;
}

/** Observe complete production frames, keeping an admitted dense world available for a bounded UI capture. */
async function observe(port: number, seconds: number, requiredHoldSeconds: number, auditGeometry: boolean): Promise<Record<string, unknown>> {
  const socket = new WebSocket(`ws://127.0.0.1:${port}`, { maxPayload: 64 * 1024 * 1024 });
  return new Promise<Record<string, unknown>>((done, reject) => {
    const began = performance.now();
    let stopReason = 'wall-time-limit';
    let finished = false;
    let frames = 0;
    let peak: FrameCounts | null = null;
    let welcome: Record<string, unknown> | null = null;
    const packetTypes: Record<string, number> = {};
    let lastSample = -5;
    let capacityBegan: number | undefined;
    let capacityHeldSeconds = 0;
    let healthPending = false;
    const healthSamples: Array<Record<string, unknown>> = [];
    const samples: Array<FrameCounts & { wallSeconds: number }> = [];
    const timer = setTimeout(() => finish(), seconds * 1000);
    // Bracket the actual observer stream with live bounded queue/frame diagnostics.
    const healthTimer = setInterval(() => {
      if (finished || healthPending) return;
      healthPending = true;
      void fetch(`http://127.0.0.1:${port}/api/health`, {
        signal: AbortSignal.timeout(5000)
      }).then(async response => {
        const health = await response.json() as Record<string, unknown>;
        if (!response.ok || !health['ok']) throw new Error(`capacity runtime fault: ${JSON.stringify(health)}`);
        if (!finished) {
          healthSamples.push({ wallSeconds: (performance.now() - began) / 1000,
            completedStep: health['completedStep'], outbound: health['outbound'],
            nativeQueues: health['nativeQueues'], telemetry: health['telemetry'] });
        }
      }).catch(error => finish(error instanceof Error ? error : new Error(String(error))))
        .finally(() => { healthPending = false; });
    }, 10_000);
    /** Release the observer socket and settle once, retaining a failed capacity attempt honestly. */
    function finish(error?: Error): void {
      if (finished) return;
      finished = true;
      clearTimeout(timer);
      clearInterval(healthTimer);
      socket.terminate();
      if (error) reject(error);
      else if (!welcome || frames === 0) reject(new Error('capacity observer received no live frames'));
      else done({ frames, packetTypes, healthSamples, wallSeconds: (performance.now() - began) / 1000, welcome,
        peak, samples, reachedRequiredBodySegments:
          (peak?.inArenaCollisionSegments ?? peak?.bodySegments ?? 0) > REQUIRED_SEGMENTS,
        requiredSegmentScope: auditGeometry ? 'both endpoints inside arena, after 30-point head skip' : 'all adjacent body points',
        capacityHeldSeconds, requiredHoldSeconds, stopReason });
    }
    socket.on('open', () => socket.send(JSON.stringify({ type: 'hello', version: 2, clientType: 'ui' })));
    socket.on('error', error => finish(error));
    socket.on('close', () => finish(new Error('capacity socket closed before its window ended')));
    socket.on('message', (data, binary) => {
      if (finished) return;
      try {
        if (binary) {
          const bytes = Buffer.isBuffer(data) ? data : Array.isArray(data) ? Buffer.concat(data) : Buffer.from(data);
          const counts = frameCounts(bytes, auditGeometry);
          const wallSeconds = (performance.now() - began) / 1000;
          frames++;
          const qualifyingSegments = counts.inArenaCollisionSegments ?? counts.bodySegments;
          if (!peak || qualifyingSegments > (peak.inArenaCollisionSegments ?? peak.bodySegments)) peak = counts;
          if (qualifyingSegments > REQUIRED_SEGMENTS) {
            capacityBegan ??= wallSeconds;
            capacityHeldSeconds = wallSeconds - capacityBegan;
          } else {
            capacityBegan = undefined;
            capacityHeldSeconds = 0;
          }
          if (wallSeconds - lastSample >= 5) {
            samples.push({ ...counts, wallSeconds });
            lastSample = wallSeconds;
            process.stdout.write(`${JSON.stringify({ wallSeconds, ...counts })}\n`);
          }
          // Ordinary capacity uses five seconds; browser mode keeps real physics
          // advancing long enough for separate foreground follow/overview captures.
          if (capacityHeldSeconds >= requiredHoldSeconds) {
            stopReason = 'required-dense-hold-complete';
            finish();
          } else if (auditGeometry && counts.generation === 1 &&
              counts.alive * (CROWDED_MAXIMUM_LENGTH - 30) <= REQUIRED_SEGMENTS) {
            // With no baseline/external spawns, this generation's survivor count
            // can only fall. Do not spend the remaining window on an impossible hold.
            stopReason = 'first-generation-survivors-cannot-reach-in-arena-boundary';
            finish();
          }
        } else {
          const packet = JSON.parse(data.toString()) as Record<string, unknown>;
          const type = String(packet['type']);
          packetTypes[type] = (packetTypes[type] ?? 0) + 1;
          if (packet['type'] === 'welcome') {
            welcome = packet;
            socket.send(JSON.stringify({ type: 'join', mode: 'spectator' }));
            socket.send(JSON.stringify({ type: 'view', mode: 'overview', viewW: 1200, viewH: 900 }));
          } else if (packet['type'] === 'error') finish(new Error(String(packet['message'])));
        }
      } catch (error) { finish(error instanceof Error ? error : new Error(String(error))); }
    });
  });
}

/** Prepare an absent disposable database; optional LAN exposure supports separate browser evidence. */
export async function run(databasePath: string, seconds: number, browserLan = false): Promise<Record<string, unknown>> {
  if (!Number.isSafeInteger(seconds) || seconds < 1 || seconds > (browserLan ? 900 : 600)) {
    throw new Error('capacity window exceeds its bounded mode limit');
  }
  if (existsSync(databasePath) || existsSync(`${databasePath}.checkpoints`)) {
    throw new Error('capacity fixture requires absent database and managed paths');
  }
  await mkdir(dirname(databasePath), { recursive: true });
  const source = createSource(databasePath, browserLan);
  const host = browserLan ? '0.0.0.0' : '127.0.0.1';
  const requiredHoldSeconds = browserLan ? 180 : 5;
  const server = await startRustServer({ ...DEFAULT_CONFIG, host, port: browserLan ? 5180 : 0, dbPath: databasePath,
    resume: 'latest', rustCalculationWorkers: 6, logLevel: 'error' });
  try {
    if (server.startupFault) throw new Error(`capacity startup failed: ${server.startupFault}`);
    process.stdout.write(`${JSON.stringify({ capacityServerReady: true, host, port: server.port,
      requiredHoldSeconds, maximumWallSeconds: seconds })}\n`);
    const result = await observe(server.port, seconds, requiredHoldSeconds, browserLan);
    const response = await fetch(`http://127.0.0.1:${server.port}/api/health`, {
      signal: AbortSignal.timeout(10_000)
    });
    const health = await response.json() as Record<string, unknown>;
    if (!response.ok || !health['ok']) throw new Error(`capacity runtime fault: ${JSON.stringify(health)}`);
    return { source, ...result, host, port: server.port, browserLan, health,
      scope: browserLan
        ? 'Crowded-world preparation through supported 300-snake, 5-point initial bodies, 10000-radius arena, 1000-point maximum bodies, 650-speed movement and 1x requested simulation. Normal production sensing, collisions and deaths. Every observed frame audits actual in-arena collision segments after the 30-point head skip; only those segments qualify the hold. No body injection or hooks addon. Host contention and observation overhead must be accounted for; this report alone does not qualify browser drawing or final P4 acceptance.'
        : 'Capacity preparation only through supported slow-motion/high-growth settings at 12x requested speed. Normal production sensing, physics, food and deaths; no body injection or hooks addon. Straight tail extension can place most represented points outside the arena. Counts prove storage/frame capacity, not crowded-world collision, browser rendering or final P4 acceptance.' };
  } finally { await server.close(); }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const args = process.argv.slice(2);
  const browserLan = args[6] === '--browser-lan';
  if (args.length !== (browserLan ? 7 : 6) || args[0] !== '--db-path' || !args[1] || args[2] !== '--seconds' ||
      !/^[1-9][0-9]*$/u.test(args[3] ?? '') || Number(args[3]) > (browserLan ? 900 : 600) ||
      args[4] !== '--output' || !args[5]) {
    throw new Error('usage: --db-path NEW --seconds 1..600 --output NEW [--browser-lan (up to 900 seconds)]');
  }
  const output = resolve(args[5]);
  if (existsSync(output)) throw new Error('capacity report destination exists');
  void run(resolve(args[1]), Number(args[3]), browserLan).then(async result => {
    await mkdir(dirname(output), { recursive: true });
    await writeFile(output, `${JSON.stringify(result, null, 2)}\n`);
    process.stdout.write(`${JSON.stringify({ reachedRequiredBodySegments: result['reachedRequiredBodySegments'],
      peak: result['peak'], frames: result['frames'] })}\n`);
  }).catch(error => { process.stderr.write(`${String(error)}\n`); process.exitCode = 1; });
}
