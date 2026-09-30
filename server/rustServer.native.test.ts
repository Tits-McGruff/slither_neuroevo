import { mkdtemp, rm, readdir, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { request as httpRequest } from 'node:http';
import { gzipSync } from 'node:zlib';
import { expect, it, vi } from 'vitest';
import WebSocket from 'ws';
import Database from 'better-sqlite3';
import { DEFAULT_CONFIG, normalizeConfig } from './config.ts';
import { PlayerActionPump } from '../src/net/playerActionPump.ts';
import { createWsClient, type AssignMsg, type SensorsMsg, type WelcomeMsg, type WsClient } from '../src/net/wsClient.ts';
import { run as runStage6RuntimeProbe } from '../scripts/stage6/runtime-integration-probe.ts';
import { startRustServer } from './rustServer.ts';
import { CheckpointPersistenceClient } from './rustEngine/checkpointPersistenceClient.ts';
import { BackgroundOutputPump } from './rustEngine/backgroundOutput.ts';
import { describeNetworkSuite } from './test/networkSuites.ts';
import { buildStackGraphSpec } from '../src/brains/stackBuilder.ts';
import { compileGraph } from '../src/brains/graph/compiler.ts';
import { CFG_DEFAULT } from '../src/config.ts';
import { DEFAULT_CORE_SETTINGS } from '../src/protocol/settings.ts';
import type { RustQueueDiagnostics } from '../src/protocol/rustBackground.ts';
import type { WsOutboundDiagnostics } from './wsHub.ts';

/** Bounded test inbox for the real Protocol 2 transport. */
interface Peer {
  /** Live local test socket. */
  socket: WebSocket;
  /** Small received protocol packets. */
  packets: Array<Record<string, unknown>>;
  /** Number of binary frame-v1 messages received. */
  frames: number;
  /** Newest copied display frame, retained only for bounded integration assertions. */
  latestFrame?: Buffer;
}

/** Attach listeners before sending a hello so no ready message can be missed. */
async function connect(port: number, clientType: 'ui' | 'bot'): Promise<Peer> {
  const peer: Peer = { socket: new WebSocket(`ws://127.0.0.1:${port}`), packets: [], frames: 0 };
  peer.socket.on('message', (data, binary) => {
    if (binary) { peer.frames++; peer.latestFrame = Buffer.from(data as Buffer); }
    else if (peer.packets.length < 256) peer.packets.push(JSON.parse(data.toString()) as Record<string, unknown>);
  });
  await new Promise<void>((done, reject) => { peer.socket.once('open', done); peer.socket.once('error', reject); });
  peer.socket.send(JSON.stringify({ type: 'hello', version: 2, clientType }));
  return peer;
}

/** Wait on a bounded real transport outcome, preserving diagnostics on failure. */
async function until(peer: Peer, predicate: () => boolean): Promise<void> {
  const deadline = performance.now() + 5000;
  while (!predicate() && performance.now() < deadline) await new Promise<void>(done => setTimeout(done, 10));
  expect(predicate(), JSON.stringify(peer.packets)).toBe(true);
}

/** Poll one real health condition within the existing five-second integration bound. */
async function healthUntil(
  port: number,
  predicate: (health: Record<string, unknown>) => boolean
): Promise<Record<string, unknown>> {
  const deadline = performance.now() + 5000;
  let health: Record<string, unknown> = {};
  while (performance.now() < deadline) {
    health = await (await fetch(`http://127.0.0.1:${port}/api/health`)).json() as Record<string, unknown>;
    if (predicate(health)) return health;
    await new Promise<void>(done => setTimeout(done, 10));
  }
  expect(predicate(health), JSON.stringify(health)).toBe(true);
  return health;
}

/** Read one assigned snake direction from the compact frame-v1 contract. */
function frameDirection(bytes: Buffer | undefined, snakeId: number): number | undefined {
  if (!bytes || bytes.byteLength < 7 * Float32Array.BYTES_PER_ELEMENT) return undefined;
  const value = (index: number): number => bytes.readFloatLE(index * Float32Array.BYTES_PER_ELEMENT);
  const alive = Math.trunc(value(2));
  let offset = 7;
  for (let index = 0; index < alive; index++) {
    if ((offset + 8) * Float32Array.BYTES_PER_ELEMENT > bytes.byteLength) return undefined;
    const id = value(offset);
    const direction = value(offset + 5);
    const points = Math.trunc(value(offset + 7));
    if (id === snakeId) return direction;
    if (points < 0) return undefined;
    offset += 8 + points * 2;
  }
  return undefined;
}

/** Read the first alive snake's public identity and head from frame v1. */
function firstFrameSnake(bytes: Buffer | undefined): { snakeId: number; x: number; y: number } | undefined {
  if (!bytes || bytes.byteLength < 15 * Float32Array.BYTES_PER_ELEMENT) return undefined;
  const value = (index: number): number => bytes.readFloatLE(index * Float32Array.BYTES_PER_ELEMENT);
  if (Math.trunc(value(2)) < 1) return undefined;
  return { snakeId: value(7), x: value(10), y: value(11) };
}

/** Signed shortest angular change from one wrapped direction to another. */
function directionDelta(from: number, to: number): number {
  return Math.atan2(Math.sin(to - from), Math.cos(to - from));
}

describeNetworkSuite('Rust server real sockets', () => {
  it.runIf(process.env['SLITHER_FULL_UPLOAD_TIMEOUT_TEST'] === '1')(
    'rejects a connected chunked import after the full no-progress deadline', async () => {
      const root = await mkdtemp(join(tmpdir(), 'slither-rust-stalled-import-'));
      const dbPath = join(root, 'experiment.sqlite');
      const server = await startRustServer({
        ...DEFAULT_CONFIG, port: 0, resume: 'fresh', seed: 42, dbPath
      });
      const request = httpRequest(`http://127.0.0.1:${server.port}/api/import/archive`, {
        method: 'POST', headers: { 'Content-Type': 'application/vnd.slither-neuroevo.save',
          'Transfer-Encoding': 'chunked' }
      });
      try {
        const before = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as {
          runId: string; startupCheckpointId: string;
        };
        const startedAt = performance.now();
        const terminal = new Promise<{ status: number; body: string }>((resolve, reject) => {
          request.once('error', reject);
          request.once('response', response => {
            const chunks: Buffer[] = [];
            response.on('data', chunk => chunks.push(chunk as Buffer));
            response.once('error', reject);
            response.once('end', () => resolve({ status: response.statusCode ?? 0,
              body: Buffer.concat(chunks).toString() }));
          });
        });
        request.write(Buffer.from('started-but-incomplete'));
        const result = await terminal;
        const elapsedMs = performance.now() - startedAt;
        expect(result.status).toBe(400);
        expect(result.body).toContain('archive upload made no progress for 60000 ms');
        expect(elapsedMs).toBeGreaterThanOrEqual(59_000);
        request.destroy();
        expect((await readdir(`${dbPath}.checkpoints`)).filter(name => name.includes('upload'))).toEqual([]);
        expect(await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json()).toMatchObject({
          ok: true, runId: before.runId, startupCheckpointId: before.startupCheckpointId
        });
      } finally {
        request.destroy();
        await server.close();
        await rm(root, { recursive: true, force: true });
      }
    }, 90_000
  );

  it('rejects a second import while a chunked client is connected and cleans an aborted upload', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-aborted-import-'));
    const dbPath = join(root, 'experiment.sqlite');
    const server = await startRustServer({
      ...DEFAULT_CONFIG, port: 0, resume: 'fresh', seed: 42, dbPath
    });
    const first = httpRequest(`http://127.0.0.1:${server.port}/api/import/archive`, {
      method: 'POST', headers: { 'Content-Type': 'application/vnd.slither-neuroevo.save',
        'Transfer-Encoding': 'chunked' }
    });
    first.on('error', () => { /* Destroying the deliberately incomplete request is expected. */ });
    try {
      const before = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as {
        runId: string; startupCheckpointId: string;
      };
      first.write(Buffer.from('incomplete-archive'));
      const managedDirectory = `${dbPath}.checkpoints`;
      const uploadDeadline = performance.now() + 5000;
      let uploadFiles: string[] = [];
      do {
        uploadFiles = (await readdir(managedDirectory)).filter(name => name.includes('upload.partial'));
        if (uploadFiles.length === 1) break;
        await new Promise<void>(done => setTimeout(done, 10));
      } while (performance.now() < uploadDeadline);
      expect(uploadFiles).toHaveLength(1);

      const busy = await fetch(`http://127.0.0.1:${server.port}/api/import/archive`, {
        method: 'POST', body: 'another-import'
      });
      expect(busy.status).toBe(409);
      expect(await busy.json()).toMatchObject({ ok: false,
        message: 'another archive operation is in progress' });

      first.destroy();
      const cleanupDeadline = performance.now() + 5000;
      let leftovers: string[] = [];
      do {
        leftovers = (await readdir(managedDirectory)).filter(name => name.includes('upload'));
        if (leftovers.length === 0) break;
        await new Promise<void>(done => setTimeout(done, 10));
      } while (performance.now() < cleanupDeadline);
      expect(leftovers).toEqual([]);
      const retryDeadline = performance.now() + 5000;
      let retryStatus = 409;
      do {
        const retry = await fetch(`http://127.0.0.1:${server.port}/api/import/archive`, {
          method: 'POST', body: 'invalid-but-complete-archive'
        });
        retryStatus = retry.status;
        await retry.body?.cancel();
        if (retryStatus !== 409) break;
        await new Promise<void>(done => setTimeout(done, 10));
      } while (performance.now() < retryDeadline);
      expect(retryStatus).toBe(400);
      expect(await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json()).toMatchObject({
        ok: true, runId: before.runId, startupCheckpointId: before.startupCheckpointId
      });
      expect((await readdir(managedDirectory)).filter(name => name.includes('upload'))).toEqual([]);
    } finally {
      first.destroy();
      await server.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 20_000);

  it('creates the first Rust run when resume-latest targets an absent database', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-first-run-'));
    const dbPath = join(root, 'slither.sqlite');
    const server = await startRustServer({
      ...DEFAULT_CONFIG, port: 0, dbPath, resume: 'latest', seed: 91
    });
    try {
      const health = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json();
      expect(health).toMatchObject({ ok: true, authority: 'rust', generation: '0000000000000001' });
      const firstBoundary = BigInt(`0x${(health as { commandServiceBoundaries: string }).commandServiceBoundaries}`);
      await healthUntil(server.port, sample =>
        BigInt(`0x${sample['commandServiceBoundaries'] as string}`) > firstBoundary);
    } finally {
      await server.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 15_000);

  it('passes the selected checkpoint budget through production health and retention', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-budget-'));
    const dbPath = join(root, 'slither.sqlite');
    const server = await startRustServer({
      ...DEFAULT_CONFIG, port: 0, dbPath, resume: 'fresh', seed: 42,
      checkpointBudgetMiB: 2048
    });
    try {
      expect(await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json()).toMatchObject({
        ok: true,
        retention: { automaticByteCap: '0000000080000000' }
      });
    } finally {
      await server.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 15_000);

  it('imports an old browser JSON population as a new Rust run', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-legacy-import-'));
    const dbPath = join(root, 'experiment.sqlite');
    const server = await startRustServer({
      ...DEFAULT_CONFIG, port: 0, resume: 'fresh', seed: 41, dbPath, rustCalculationWorkers: 4
    });
    const peers: Peer[] = [];
    try {
      const before = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as {
        runId: string; nativeBuildIdentifier: string; calculationWorkers: number;
      };
      expect(before.nativeBuildIdentifier).toMatch(/^slither_native\/[0-9A-Za-z.+-]+$/u);
      expect(before.calculationWorkers).toBe(4);
      const viewer = await connect(server.port, 'ui');
      peers.push(viewer);
      await until(viewer, () => viewer.packets.some(packet => packet['type'] === 'welcome'));
      expect(viewer.packets.find(packet => packet['type'] === 'welcome')).toMatchObject({
        inferenceMode: { nativeAddonBuildIdentifier: before.nativeBuildIdentifier,
          requestedMt: true, activeWorkerCount: 4 }
      });
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator' }));
      const weights = new Array<number>(13_458).fill(0);
      const legacyFile = JSON.stringify({
        generation: 37,
        archKey: 'legacy-default-graph',
        worldSeed: 1_234_567,
        settings: { snakeCount: 2, simSpeed: 3, baselineBots: { count: 1 } },
        genomes: [
          { archKey: 'legacy-default-graph', brainType: 'mlp', fitness: 99, weights },
          { archKey: 'legacy-default-graph', brainType: 'mlp', fitness: 50, weights }
        ]
      });
      const response = await fetch(`http://127.0.0.1:${server.port}/api/import/archive`, {
        method: 'POST', headers: { 'Content-Type': 'application/json' }, body: legacyFile
      });
      const imported = await response.json() as {
        ok: boolean; runId: string; generation: string; completedStep: string; checkpointId: string;
      };
      expect({ status: response.status, imported }).toMatchObject({
        status: 200,
        imported: {
          ok: true,
          generation: '0000000000000001',
          completedStep: '0000000000000000'
        }
      });
      expect(imported.runId).not.toBe(before.runId);
      expect(imported.checkpointId).toMatch(/^[0-9a-f]{64}$/u);
      await until(viewer, () => viewer.packets.some(packet =>
        packet['type'] === 'stateReplaced' && packet['reason'] === 'import'));
      expect(viewer.packets.findLast(packet => packet['type'] === 'stateReplaced')).toMatchObject({
        checkpointId: imported.checkpointId,
        welcome: {
          runId: imported.runId,
          worldSeed: 1_234_567,
          inferenceMode: { activeWorkerCount: 4 },
          settings: {
            core: { snakeCount: 2, simSpeed: 3 },
            updates: expect.arrayContaining([{ path: 'baselineBots.count', value: 1 }])
          }
        }
      });
      expect(await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json()).toMatchObject({
        ok: true,
        runId: imported.runId,
        seed: 1_234_567,
        generation: '0000000000000001',
        startupCheckpointId: imported.checkpointId
      });
      const rejected = await fetch(`http://127.0.0.1:${server.port}/api/import/archive`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          generation: 1,
          archKey: 'legacy-default-graph',
          genomes: [{ archKey: 'legacy-default-graph', weights: [0] }]
        })
      });
      expect(rejected.status).toBe(400);
      expect(await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json()).toMatchObject({
        ok: true, runId: imported.runId, startupCheckpointId: imported.checkpointId
      });
      expect((await readdir(`${dbPath}.checkpoints`)).filter(name =>
        name.includes('upload') || name.includes('import-inventory'))).toEqual([]);
    } finally {
      for (const peer of peers) peer.socket.terminate();
      await server.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);

  it('converts the newest TypeScript v2 checkpoint without changing its source rows', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-v2-startup-'));
    const dbPath = join(root, 'experiment.sqlite');
    const core = { ...DEFAULT_CORE_SETTINGS, snakeCount: 2, simSpeed: 2, neurons1: 96, neurons2: 96 };
    const graphSpec = buildStackGraphSpec(core, CFG_DEFAULT);
    const graph = compileGraph(graphSpec);
    expect(graph.totalParams).toBeGreaterThan(64 * 1024 / Float32Array.BYTES_PER_ELEMENT);
    const firstWeights = Buffer.alloc(graph.totalParams * Float32Array.BYTES_PER_ELEMENT);
    const secondWeights = Buffer.from(firstWeights);
    secondWeights.writeFloatLE(0.25, 0);
    const metadata = JSON.stringify({
      formatVersion: 2,
      boundaryVersion: 1,
      boundaryKind: 'generation',
      resumable: true,
      generation: 19,
      simulationStep: 72_000,
      runId: 'typescript-source-run',
      worldSeed: 7_654_321,
      configHash: 'legacy-config',
      configRevision: 4,
      archKey: graph.key,
      graphSpec,
      populationCount: 2,
      settings: core,
      updates: [
        { path: 'baselineBots.count', value: 0 },
        { path: 'foodSpawn.edgeFalloffEnabled', value: 1 }
      ],
      rng: {},
      allocators: {},
      bestFitnessEver: 0,
      fitnessHistory: [],
      lastHofEntry: null
    });
    const sourceDigest = createHash('sha256')
      .update(metadata)
      .update(firstWeights)
      .update(secondWeights)
      .digest('hex');
    const database = new Database(dbPath);
    try {
      database.exec(`
        CREATE TABLE population_snapshots (
          id INTEGER PRIMARY KEY, payload_json TEXT, format_version INTEGER,
          boundary_kind TEXT, population_count INTEGER
        );
        CREATE TABLE snapshot_genomes (
          snapshot_id INTEGER NOT NULL, slot INTEGER NOT NULL, arch_key TEXT NOT NULL,
          brain_type TEXT NOT NULL, fitness REAL NOT NULL, weight_count INTEGER NOT NULL,
          weights_blob BLOB NOT NULL, weights_checksum TEXT NOT NULL,
          PRIMARY KEY (snapshot_id, slot)
        );
      `);
      database.prepare(`INSERT INTO population_snapshots
        (id, payload_json, format_version, boundary_kind, population_count)
        VALUES (1, ?, 2, 'generation', 2)`).run(metadata);
      const insert = database.prepare(`INSERT INTO snapshot_genomes
        (snapshot_id, slot, arch_key, brain_type, fitness, weight_count, weights_blob, weights_checksum)
        VALUES (1, ?, ?, 'mlp', ?, ?, ?, ?)`);
      for (const [slot, weights] of [firstWeights, secondWeights].entries()) {
        insert.run(slot, graph.key, 10 - slot, graph.totalParams, weights,
          createHash('sha256').update(weights).digest('hex'));
      }
    } finally { database.close(); }
    const { seed: _defaultSeed, ...resumeConfig } = DEFAULT_CONFIG;
    let server = await startRustServer({
      ...resumeConfig, port: 0, resume: 'latest', dbPath
    });
    const peers: Peer[] = [];
    try {
      expect(server.startupFault).toBeUndefined();
      const health = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as {
        runId: string; startupCheckpointId: string;
      };
      const legacyConversion = {
        sourceSnapshotId: 1,
        sourceFormat: 'typescript-v2',
        completeness: 'population-only',
        exactContinuation: false
      };
      expect(health).toMatchObject({
        ok: true, seed: 7_654_321, generation: '0000000000000001', legacyConversion
      });
      expect(health.runId).not.toBe('typescript-source-run');
      const viewer = await connect(server.port, 'ui');
      peers.push(viewer);
      await until(viewer, () => viewer.packets.some(packet => packet['type'] === 'welcome'));
      expect(viewer.packets.find(packet => packet['type'] === 'welcome')).toMatchObject({
        worldSeed: 7_654_321,
        settings: { core: { snakeCount: 2, simSpeed: 2 } },
        legacyConversion
      });
      viewer.socket.terminate();
      await server.close();
      server = await startRustServer({
        ...resumeConfig, port: 0, resume: 'latest', dbPath
      });
      expect(server.startupFault).toBeUndefined();
      expect(await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json()).toMatchObject({
        ok: true, runId: health.runId, startupCheckpointId: health.startupCheckpointId,
        legacyConversion
      });
      const retained = new Database(dbPath, { readonly: true });
      try {
        const row = retained.prepare('SELECT payload_json FROM population_snapshots WHERE id = 1')
          .get() as { payload_json: string };
        const blobs = retained.prepare(`SELECT weights_blob FROM snapshot_genomes
          WHERE snapshot_id = 1 ORDER BY slot`).all() as Array<{ weights_blob: Buffer }>;
        expect(createHash('sha256').update(row.payload_json)
          .update(blobs[0]!.weights_blob).update(blobs[1]!.weights_blob).digest('hex')).toBe(sourceDigest);
      } finally { retained.close(); }
    } finally {
      for (const peer of peers) peer.socket.terminate();
      await server.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);

  it.each(['gzip', 'embedded'] as const)(
    'converts a format-zero %s population without changing its source row',
    async (storage) => {
      const root = await mkdtemp(join(tmpdir(), `slither-rust-v0-${storage}-`));
      const dbPath = join(root, 'experiment.sqlite');
      const core = { ...DEFAULT_CORE_SETTINGS, snakeCount: 2, simSpeed: 3 };
      const graphSpec = buildStackGraphSpec(core, CFG_DEFAULT);
      const graph = compileGraph(graphSpec);
      const first = new Array<number>(graph.totalParams).fill(0);
      const second = new Array<number>(graph.totalParams).fill(0);
      second[0] = 0.25;
      const genomes = [first, second].map((weights, slot) => ({
        archKey: graph.key, brainType: 'mlp', fitness: 10 - slot, weights
      }));
      const payload = JSON.stringify({
        generation: 7,
        archKey: graph.key,
        genomes: storage === 'embedded' ? genomes : [],
        cfgHash: 'legacy-config',
        worldSeed: 1_234_567,
        graphSpec,
        ...(storage === 'embedded' ? {
          settings: core,
          updates: [{ path: 'baselineBots.count', value: 0 }]
        } : {})
      });
      const framed = storage === 'gzip' ? gzipSync(Buffer.concat(genomes.flatMap(genome => {
        const encoded = Buffer.from(JSON.stringify(genome));
        const prefix = Buffer.alloc(4);
        prefix.writeUInt32LE(encoded.byteLength);
        return [prefix, encoded];
      }))) : null;
      const sourceDigest = createHash('sha256').update(payload).update(framed ?? Buffer.alloc(0)).digest('hex');
      const database = new Database(dbPath);
      try {
        database.exec(storage === 'gzip' ? `CREATE TABLE population_snapshots (
          id INTEGER PRIMARY KEY, created_at INTEGER, gen INTEGER, payload_json TEXT,
          settings_json TEXT, updates_json TEXT, genomes_blob BLOB
        )` : `CREATE TABLE population_snapshots (
          id INTEGER PRIMARY KEY, created_at INTEGER, gen INTEGER, payload_json TEXT
        )`);
        if (storage === 'gzip') {
          database.prepare(`INSERT INTO population_snapshots
            (id, created_at, gen, payload_json, settings_json, updates_json, genomes_blob)
            VALUES (1, ?, 7, ?, ?, ?, ?)`).run(Date.now(), payload, JSON.stringify(core),
              JSON.stringify([{ path: 'baselineBots.count', value: 0 }]), framed);
        } else {
          database.prepare(`INSERT INTO population_snapshots
            (id, created_at, gen, payload_json) VALUES (1, ?, 7, ?)`).run(Date.now(), payload);
        }
      } finally { database.close(); }

      const { seed: _defaultSeed, ...resumeConfig } = DEFAULT_CONFIG;
      const server = await startRustServer({
        ...resumeConfig, port: 0, resume: 'latest', dbPath
      });
      const peers: Peer[] = [];
      try {
        expect(server.startupFault).toBeUndefined();
        const legacyConversion = {
          sourceSnapshotId: 1,
          sourceFormat: storage === 'gzip' ? 'legacy-gzip' : 'legacy-json',
          completeness: 'population-only',
          exactContinuation: false
        };
        expect(await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json()).toMatchObject({
          ok: true, seed: 1_234_567, generation: '0000000000000001', legacyConversion
        });
        const viewer = await connect(server.port, 'ui');
        peers.push(viewer);
        await until(viewer, () => viewer.packets.some(packet => packet['type'] === 'welcome'));
        expect(viewer.packets.find(packet => packet['type'] === 'welcome')).toMatchObject({
          worldSeed: 1_234_567, settings: { core: { snakeCount: 2, simSpeed: 3 } },
          legacyConversion
        });
        const retained = new Database(dbPath, { readonly: true });
        try {
          const row = retained.prepare(`SELECT payload_json${storage === 'gzip' ? ', genomes_blob' : ''}
            FROM population_snapshots WHERE id = 1`).get() as { payload_json: string; genomes_blob?: Buffer };
          expect(createHash('sha256').update(row.payload_json)
            .update(row.genomes_blob ?? Buffer.alloc(0)).digest('hex')).toBe(sourceDigest);
          expect((retained.prepare('SELECT count(*) AS count FROM rust_checkpoint_v3_current').get() as
            { count: number }).count).toBe(1);
          expect(retained.prepare(`SELECT source_snapshot_id, source_format, completeness
            FROM rust_legacy_conversions_v1`).get()).toEqual({
            source_snapshot_id: 1,
            source_format: legacyConversion.sourceFormat,
            completeness: 'population-only'
          });
        } finally { retained.close(); }
      } finally {
        for (const peer of peers) peer.socket.terminate();
        await server.close();
        await rm(root, { recursive: true, force: true });
      }
    },
    30_000
  );

  it('streams, imports, and atomically activates one exact Rust save', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-export-server-'));
    const dbPath = join(root, 'experiment.sqlite');
    const managedDirectory = `${dbPath}.checkpoints`;
    const server = await startRustServer({
      ...DEFAULT_CONFIG, port: 0, resume: 'fresh', seed: 41, dbPath
    });
    let target: Awaited<ReturnType<typeof startRustServer>> | undefined;
    const peers: Peer[] = [];
    try {
      const health = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as {
        runId: string; startupCheckpointId: string; archiveWork: unknown;
        schedulerDroppedWallMicros: string; schedulerOverloaded: boolean;
      };
      expect(health.archiveWork).toBeNull();
      expect(health.schedulerDroppedWallMicros).toMatch(/^[0-9a-f]{16}$/u);
      expect(typeof health.schedulerOverloaded).toBe('boolean');
      const hallOfFame = await fetch(`http://127.0.0.1:${server.port}/api/hof`);
      expect(hallOfFame.status).toBe(200);
      expect(await hallOfFame.json()).toEqual({ hof: [] });
      const missingWinner = await fetch(`http://127.0.0.1:${server.port}/api/resurrect`, {
        method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ entryId: '0000000000000001' })
      });
      expect(missingWinner.status).toBe(400);
      expect(await missingWinner.json()).toMatchObject({ ok: false });
      const exported = await fetch(`http://127.0.0.1:${server.port}/api/export/latest`);
      expect(exported.status).toBe(200);
      expect(exported.headers.get('content-type')).toBe('application/vnd.slither-neuroevo.save');
      expect(exported.headers.get('content-disposition')).toMatch(
        /^attachment; filename="slither-neuroevo-[0-9a-f]{12}-gen-1-v1\.slither-save"$/u
      );
      expect(exported.headers.get('x-slither-checkpoint-id')).toBe(health.startupCheckpointId);
      expect(exported.headers.get('x-slither-save-root')).toMatch(/^[0-9a-f]{64}$/u);
      const concurrent = await fetch(`http://127.0.0.1:${server.port}/api/export/latest`);
      expect(concurrent.status).toBe(409);
      await concurrent.body?.cancel();
      const bytes = Buffer.from(await exported.arrayBuffer());
      expect(bytes.byteLength).toBe(Number(exported.headers.get('content-length')));
      expect(bytes.subarray(0, 100).toString('utf8').replace(/\0.*$/u, '')).toBe('checkpoint.bin');
      expect(bytes.includes(Buffer.from('checkpoint/checkpoint-v3.ustar\0'))).toBe(false);
      expect(bytes.includes(Buffer.from('graph.bin\0'))).toBe(true);
      expect(bytes.includes(Buffer.from('population/index.bin\0'))).toBe(true);
      expect(bytes.includes(Buffer.from('population/weights.'))).toBe(true);
      expect(bytes.includes(Buffer.from('population/recurrent.'))).toBe(true);
      expect(bytes.includes(Buffer.from('history.bin\0'))).toBe(true);
      expect(bytes.includes(Buffer.from('hof/index.bin\0'))).toBe(true);
      expect(bytes.includes(Buffer.from('hof/weights.f32le\0'))).toBe(true);
      expect(bytes.includes(Buffer.from('manifest.json\0'))).toBe(true);
      const cleanupDeadline = performance.now() + 2000;
      let leftovers: string[] = [];
      do {
        leftovers = (await readdir(managedDirectory)).filter(name =>
          name.includes('export-inventory') || name.includes('slither-save') || name.includes('export-hof')
        );
        if (leftovers.length === 0) break;
        await new Promise<void>(done => setTimeout(done, 10));
      } while (performance.now() < cleanupDeadline);
      expect(leftovers).toEqual([]);
      const afterExport = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as {
        archiveWork: { kind: string; started: boolean; finished: boolean; completedBytes: string };
      };
      expect(afterExport.archiveWork).toMatchObject({ kind: 'export', started: true, finished: true });
      expect(BigInt(`0x${afterExport.archiveWork.completedBytes}`)).toBeGreaterThan(0n);

      const targetDbPath = join(root, 'target.sqlite');
      target = await startRustServer({
        ...DEFAULT_CONFIG, port: 0, resume: 'fresh', seed: 42, dbPath: targetDbPath
      });
      const targetBefore = await (await fetch(`http://127.0.0.1:${target.port}/api/health`)).json() as {
        runId: string; startupCheckpointId: string;
      };
      expect(targetBefore.runId).not.toBe(health.runId);
      const viewer = await connect(target.port, 'ui');
      peers.push(viewer);
      await until(viewer, () => viewer.packets.some(packet => packet['type'] === 'welcome'));
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator' }));
      expect(viewer.packets.find(packet => packet['type'] === 'welcome')).toMatchObject({
        capabilities: { archiveExport: true, archiveImport: true }
      });
      const importedResponse = await fetch(`http://127.0.0.1:${target.port}/api/import/archive`, {
        method: 'POST', headers: { 'Content-Type': 'application/vnd.slither-neuroevo.save' }, body: bytes
      });
      expect(importedResponse.status).toBe(200);
      expect(await importedResponse.json()).toMatchObject({
        ok: true, runId: health.runId, generation: '0000000000000001', checkpointId: health.startupCheckpointId
      });
      const importedHealth = await (await fetch(`http://127.0.0.1:${target.port}/api/health`)).json() as {
        archiveWork: { kind: string; started: boolean; finished: boolean; completedBytes: string };
      };
      expect(importedHealth.archiveWork).toMatchObject({ kind: 'import', started: true, finished: true });
      expect(BigInt(`0x${importedHealth.archiveWork.completedBytes}`)).toBeGreaterThan(0n);
      await until(viewer, () => viewer.packets.some(packet => packet['type'] === 'stateReplaced'));
      expect(viewer.socket.readyState).toBe(WebSocket.OPEN);
      expect(viewer.packets.find(packet => packet['type'] === 'stateReplaced')).toMatchObject({
        reason: 'import', checkpointId: health.startupCheckpointId,
        welcome: { runId: health.runId, worldSeed: 41 }
      });
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator' }));
      expect(await (await fetch(`http://127.0.0.1:${target.port}/api/health`)).json()).toMatchObject({
        ok: true, runId: health.runId, seed: 41, startupCheckpointId: health.startupCheckpointId
      });

      const replay = await fetch(`http://127.0.0.1:${target.port}/api/import/archive`, {
        method: 'POST', headers: { 'Content-Type': 'application/vnd.slither-neuroevo.save' }, body: bytes
      });
      expect(replay.status).toBe(200);
      expect(await replay.json()).toMatchObject({ ok: true, checkpointId: health.startupCheckpointId });

      const futureDatabase = new Database(targetDbPath);
      try {
        futureDatabase.prepare(`INSERT INTO rust_generation_history_v1
          (run_id, generation_hex, checkpoint_id, record_version, record_blob, created_at_ms)
          VALUES (?, '0000000000000002', NULL, 1, ?, ?)`)
          .run(health.runId, Buffer.alloc(56), Date.now());
      } finally { futureDatabase.close(); }
      const requiresBranch = await fetch(`http://127.0.0.1:${target.port}/api/import/archive`, {
        method: 'POST', headers: { 'Content-Type': 'application/vnd.slither-neuroevo.save' }, body: bytes
      });
      const requiresBranchBody = await requiresBranch.json();
      expect({ status: requiresBranch.status, body: requiresBranchBody }).toMatchObject({
        status: 409, body: { ok: false, code: 'IMPORT_REQUIRES_BRANCH' }
      });
      const branchedResponse = await fetch(`http://127.0.0.1:${target.port}/api/import/archive?mode=branch`, {
        method: 'POST', headers: { 'Content-Type': 'application/vnd.slither-neuroevo.save' }, body: bytes
      });
      const branched = await branchedResponse.json() as { runId: string; branched: boolean; sourceRunId: string; message?: string };
      expect({ status: branchedResponse.status, body: branched }).toMatchObject({ status: 200 });
      expect(branched).toMatchObject({ branched: true, sourceRunId: health.runId });
      expect(branched.runId).not.toBe(health.runId);
      expect(await (await fetch(`http://127.0.0.1:${target.port}/api/health`)).json()).toMatchObject({
        ok: true, runId: branched.runId,
        importBranch: { sourceRunId: health.runId, branchRunId: branched.runId,
          sourceGeneration: '0000000000000001', sourceCheckpointId: health.startupCheckpointId }
      });

      await target.close();
      const { seed: _defaultSeed, ...resumeConfig } = DEFAULT_CONFIG;
      target = await startRustServer({
        ...resumeConfig, port: 0, resume: 'latest', dbPath: targetDbPath
      });
      expect(await (await fetch(`http://127.0.0.1:${target.port}/api/health`)).json()).toMatchObject({
        ok: true, runId: branched.runId,
        importBranch: { sourceRunId: health.runId, branchRunId: branched.runId }
      });
      const corrupt = Buffer.from(bytes);
      corrupt[512] = (corrupt[512] ?? 0) ^ 0xff;
      const rejected = await fetch(`http://127.0.0.1:${target.port}/api/import/archive`, {
        method: 'POST', headers: { 'Content-Type': 'application/vnd.slither-neuroevo.save' }, body: corrupt
      });
      expect(rejected.status).toBe(400);
      expect(await (await fetch(`http://127.0.0.1:${target.port}/api/health`)).json()).toMatchObject({
        ok: true, runId: branched.runId, startupCheckpointId: health.startupCheckpointId
      });
      const targetFiles = await readdir(`${targetDbPath}.checkpoints`);
      expect(targetFiles.filter(name => name.includes('upload') || name.includes('import-inventory') ||
        name.includes('import-stage'))).toEqual([]);
    } finally {
      for (const peer of peers) peer.socket.terminate();
      await target?.close();
      await server.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);

  it('faults the old authority when an import commits but its reply is lost', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-lost-import-reply-'));
    const sourceDbPath = join(root, 'source.sqlite');
    const targetDbPath = join(root, 'target.sqlite');
    const source = await startRustServer({ ...DEFAULT_CONFIG, port: 0,
      resume: 'fresh', seed: 41, dbPath: sourceDbPath });
    let target: Awaited<ReturnType<typeof startRustServer>> | undefined;
    let importSpy: ReturnType<typeof vi.spyOn> | undefined;
    try {
      const sourceHealth = await (await fetch(`http://127.0.0.1:${source.port}/api/health`)).json() as {
        runId: string; startupCheckpointId: string;
      };
      const archive = Buffer.from(await (await fetch(
        `http://127.0.0.1:${source.port}/api/export/latest`
      )).arrayBuffer());
      target = await startRustServer({ ...DEFAULT_CONFIG, port: 0,
        resume: 'fresh', seed: 42, dbPath: targetDbPath });
      const oldHealth = await (await fetch(`http://127.0.0.1:${target.port}/api/health`)).json() as {
        runId: string; startupCheckpointId: string;
      };
      const originalCommit = CheckpointPersistenceClient.prototype.commitImport;
      /** Lose one reply after the real worker has committed the new pointer. */
      importSpy = vi.spyOn(CheckpointPersistenceClient.prototype, 'commitImport')
        .mockImplementationOnce(async function (this: CheckpointPersistenceClient,
          ...args: Parameters<CheckpointPersistenceClient['commitImport']>) {
          await originalCommit.apply(this, args);
          throw new Error('injected lost import commit reply');
        });
      const response = await fetch(`http://127.0.0.1:${target.port}/api/import/archive`, {
        method: 'POST', headers: { 'Content-Type': 'application/vnd.slither-neuroevo.save' }, body: archive
      });
      expect(response.status).toBe(503);
      expect(await response.json()).toMatchObject({ ok: false, message: 'injected lost import commit reply' });
      const faulted = await healthUntil(target.port, health => health['ok'] === false);
      expect(faulted).toMatchObject({ runId: oldHealth.runId,
        startupCheckpointId: oldHealth.startupCheckpointId,
        interfaceFault: 'injected lost import commit reply' });
      expect(['stopRequested', 'stopped']).toContain(faulted['lifecycle']);
      const stopped = await healthUntil(target.port, health => health['lifecycle'] === 'stopped');
      await new Promise<void>(done => setTimeout(done, 50));
      expect(await (await fetch(`http://127.0.0.1:${target.port}/api/health`)).json()).toMatchObject({
        completedStep: stopped['completedStep']
      });
      const database = new Database(targetDbPath, { readonly: true });
      try {
        expect(database.prepare('SELECT run_id FROM rust_active_run_v1 WHERE singleton = 1').get())
          .toEqual({ run_id: sourceHealth.runId });
      } finally { database.close(); }
      await target.close();
      target = undefined;
      const { seed: _seed, ...resumeConfig } = DEFAULT_CONFIG;
      target = await startRustServer({ ...resumeConfig, port: 0,
        resume: 'latest', dbPath: targetDbPath });
      expect(await (await fetch(`http://127.0.0.1:${target.port}/api/health`)).json()).toMatchObject({
        ok: true, runId: sourceHealth.runId, startupCheckpointId: sourceHealth.startupCheckpointId
      });
    } finally {
      importSpy?.mockRestore();
      await target?.close();
      await source.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);

  it('keeps a committed New Run when its SQLite reply is lost', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-lost-new-run-reply-'));
    const dbPath = join(root, 'experiment.sqlite');
    let server = await startRustServer({ ...DEFAULT_CONFIG, port: 0,
      resume: 'fresh', seed: 42, dbPath });
    let viewer: Peer | undefined;
    let commitSpy: ReturnType<typeof vi.spyOn> | undefined;
    try {
      viewer = await connect(server.port, 'ui');
      await until(viewer, () => viewer!.packets.some(packet => packet['type'] === 'welcome'));
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator' }));
      const before = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as {
        runId: string; startupCheckpointId: string;
      };
      const originalCommit = CheckpointPersistenceClient.prototype.commit;
      /** Lose one reply after the real worker has activated the new run pointer. */
      commitSpy = vi.spyOn(CheckpointPersistenceClient.prototype, 'commit')
        .mockImplementationOnce(async function (this: CheckpointPersistenceClient,
          ...args: Parameters<CheckpointPersistenceClient['commit']>) {
          await originalCommit.apply(this, args);
          throw new Error('injected lost New Run commit reply');
        });
      viewer.socket.send(JSON.stringify({ type: 'newRun', requestId: 'lost-new-run-reply' }));
      await until(viewer, () => viewer!.packets.some(packet =>
        packet['type'] === 'error' || packet['type'] === 'newRunResult'));
      const faulted = await healthUntil(server.port, health => health['ok'] === false);
      expect(faulted).toMatchObject({ runId: before.runId,
        startupCheckpointId: before.startupCheckpointId,
        interfaceFault: 'fresh replacement checkpoint outcome is unknown; restart from a valid retained checkpoint' });
      await until(viewer, () => viewer!.packets.some(packet => packet['type'] === 'error'));
      expect(viewer.packets.filter(packet => packet['requestId'] === 'lost-new-run-reply')).toEqual([]);
      const database = new Database(dbPath, { readonly: true });
      let committedRunId: string;
      try {
        const current = database.prepare('SELECT run_id FROM rust_active_run_v1 WHERE singleton = 1')
          .get() as { run_id: string };
        committedRunId = current.run_id;
        expect(committedRunId).not.toBe(before.runId);
      } finally { database.close(); }
      viewer.socket.terminate();
      viewer = undefined;
      await server.close();
      commitSpy.mockRestore();
      commitSpy = undefined;
      const { seed: _seed, ...resumeConfig } = DEFAULT_CONFIG;
      server = await startRustServer({ ...resumeConfig, port: 0, resume: 'latest', dbPath });
      expect(await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json()).toMatchObject({
        ok: true, runId: committedRunId, generation: '0000000000000001'
      });
    } finally {
      commitSpy?.mockRestore();
      viewer?.socket.terminate();
      await server.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);

  it('recovers the committed import across both sides of the Rust swap', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-import-swap-fault-'));
    const source = await startRustServer({ ...DEFAULT_CONFIG, port: 0,
      resume: 'fresh', seed: 41, dbPath: join(root, 'source.sqlite') });
    try {
      const sourceHealth = await (await fetch(`http://127.0.0.1:${source.port}/api/health`)).json() as {
        runId: string; startupCheckpointId: string;
      };
      const archive = Buffer.from(await (await fetch(
        `http://127.0.0.1:${source.port}/api/export/latest`
      )).arrayBuffer());
      for (const phase of ['before-swap', 'after-swap'] as const) {
        const dbPath = join(root, `${phase}.sqlite`);
        let target: Awaited<ReturnType<typeof startRustServer>> | undefined;
        let publishSpy: ReturnType<typeof vi.spyOn> | undefined;
        try {
          target = await startRustServer({ ...DEFAULT_CONFIG, port: 0,
            resume: 'fresh', seed: 42, dbPath });
          const oldHealth = await (await fetch(`http://127.0.0.1:${target.port}/api/health`)).json() as {
            runId: string; startupCheckpointId: string;
          };
          const originalPublish = BackgroundOutputPump.prototype.publishPreparedImport;
          /** Fail once immediately before or after the real native publication. */
          publishSpy = vi.spyOn(BackgroundOutputPump.prototype, 'publishPreparedImport')
            .mockImplementationOnce(async function (this: BackgroundOutputPump,
              ...args: Parameters<BackgroundOutputPump['publishPreparedImport']>) {
              if (phase === 'after-swap') await originalPublish.apply(this, args);
              throw new Error(`injected import ${phase} failure`);
            });
          const response = await fetch(`http://127.0.0.1:${target.port}/api/import/archive`, {
            method: 'POST', headers: { 'Content-Type': 'application/vnd.slither-neuroevo.save' }, body: archive
          });
          expect(response.status).toBe(503);
          expect(await response.json()).toMatchObject({ ok: false,
            message: `injected import ${phase} failure` });
          const faulted = await healthUntil(target.port, health => health['ok'] === false);
          expect(faulted).toMatchObject({
            runId: oldHealth.runId, startupCheckpointId: oldHealth.startupCheckpointId,
            interfaceFault: `injected import ${phase} failure`
          });
          expect(['stopRequested', 'stopped']).toContain(faulted['lifecycle']);
          const stopped = await healthUntil(target.port, health => health['lifecycle'] === 'stopped');
          await new Promise<void>(done => setTimeout(done, 50));
          expect(await (await fetch(`http://127.0.0.1:${target.port}/api/health`)).json()).toMatchObject({
            completedStep: stopped['completedStep']
          });
          const database = new Database(dbPath, { readonly: true });
          try {
            expect(database.prepare('SELECT run_id FROM rust_active_run_v1 WHERE singleton = 1').get())
              .toEqual({ run_id: sourceHealth.runId });
          } finally { database.close(); }
          await target.close();
          target = undefined;
          const { seed: _seed, ...resumeConfig } = DEFAULT_CONFIG;
          target = await startRustServer({ ...resumeConfig, port: 0, resume: 'latest', dbPath });
          expect(await (await fetch(`http://127.0.0.1:${target.port}/api/health`)).json()).toMatchObject({
            ok: true, runId: sourceHealth.runId, startupCheckpointId: sourceHealth.startupCheckpointId
          });
        } finally {
          publishSpy?.mockRestore();
          await target?.close();
        }
      }
    } finally {
      await source.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);

  it('keeps a later-generation Rust world ready after rejecting its older exact import', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-later-import-reject-'));
    const dbPath = join(root, 'experiment.sqlite');
    const server = await startRustServer({ ...DEFAULT_CONFIG, port: 0,
      resume: 'fresh', seed: 42, dbPath });
    let viewer: Peer | undefined;
    try {
      viewer = await connect(server.port, 'ui');
      await until(viewer, () => viewer!.packets.some(packet => packet['type'] === 'welcome'));
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator' }));
      viewer.socket.send(JSON.stringify({ type: 'reset', settings: { snakeCount: 12, simSpeed: 1 },
        updates: [{ path: 'generationSeconds', value: 8 }, { path: 'baselineBots.count', value: 0 }] }));
      await until(viewer, () => viewer!.packets.some(packet =>
        packet['type'] === 'stateReplaced' && packet['reason'] === 'reset'));
      const resetHealth = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as {
        runId: string; startupCheckpointId: string;
      };
      const archived = await fetch(`http://127.0.0.1:${server.port}/api/export/latest`);
      expect(archived.status).toBe(200);
      expect(archived.headers.get('x-slither-checkpoint-id')).toBe(resetHealth.startupCheckpointId);
      const archive = Buffer.from(await archived.arrayBuffer());
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator' }));
      viewer.socket.send(JSON.stringify({ type: 'settings', requestId: 'accelerate-generation',
        updates: [{ path: 'simSpeed', value: 12 }] }));
      await until(viewer, () => viewer!.packets.some(packet =>
        packet['type'] === 'settingsApplied' && packet['requestId'] === 'accelerate-generation'));
      expect(viewer.packets.findLast(packet => packet['requestId'] === 'accelerate-generation'))
        .toMatchObject({ applied: true });
      const database = new Database(dbPath, { readonly: true });
      let currentCheckpointId = resetHealth.startupCheckpointId;
      try {
        const deadline = performance.now() + 10_000;
        while (currentCheckpointId === resetHealth.startupCheckpointId && performance.now() < deadline) {
          currentCheckpointId = (database.prepare(`SELECT checkpoint_id FROM rust_checkpoint_v3_current
            WHERE run_id = ?`).get(resetHealth.runId) as { checkpoint_id: string }).checkpoint_id;
          if (currentCheckpointId === resetHealth.startupCheckpointId) {
            await new Promise<void>(done => setTimeout(done, 20));
          }
        }
        expect(currentCheckpointId).not.toBe(resetHealth.startupCheckpointId);
        viewer.socket.send(JSON.stringify({ type: 'settings', requestId: 'hold-later-generation',
          updates: [{ path: 'simSpeed', value: 0.1 }] }));
        await until(viewer, () => viewer!.packets.some(packet =>
          packet['type'] === 'settingsApplied' && packet['requestId'] === 'hold-later-generation'));
        expect(viewer.packets.findLast(packet => packet['requestId'] === 'hold-later-generation'))
          .toMatchObject({ applied: true });
        currentCheckpointId = (database.prepare(`SELECT checkpoint_id FROM rust_checkpoint_v3_current
          WHERE run_id = ?`).get(resetHealth.runId) as { checkpoint_id: string }).checkpoint_id;
        const rejected = await fetch(`http://127.0.0.1:${server.port}/api/import/archive`, {
          method: 'POST', headers: { 'Content-Type': 'application/vnd.slither-neuroevo.save' }, body: archive
        });
        expect(rejected.status).toBe(409);
        expect(await rejected.json()).toMatchObject({ ok: false, code: 'IMPORT_REQUIRES_BRANCH' });
        expect(await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json()).toMatchObject({
          ok: true, runId: resetHealth.runId, lifecycle: 'running'
        });
        expect((database.prepare(`SELECT checkpoint_id FROM rust_checkpoint_v3_current WHERE run_id = ?`)
          .get(resetHealth.runId) as { checkpoint_id: string }).checkpoint_id).toBe(currentCheckpointId);
      } finally { database.close(); }
    } finally {
      viewer?.socket.terminate();
      await server.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);

  it('resumes exact managed IDs and exposes recovery or health-only failure over real HTTP/WebSocket', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-recovery-server-'));
    const dbPath = join(root, 'experiment.sqlite');
    const config = { ...DEFAULT_CONFIG, port: 0, dbPath };
    let server = await startRustServer({ ...config, resume: 'fresh', seed: 42 });
    const peers: Peer[] = [];
    try {
      const initial = await (await fetch(`http://127.0.0.1:${server.port}/health`)).json() as { runId: string; startupCheckpointId: string };
      await server.close();
      const exact = normalizeConfig({ ...config, resume: initial.startupCheckpointId });
      expect(exact.resume).toBe(`sha256:${initial.startupCheckpointId}`);
      server = await startRustServer({ ...exact, port: 0 });
      expect(await (await fetch(`http://127.0.0.1:${server.port}/health`)).json()).toMatchObject({ ok: true, runId: initial.runId });
      await server.close();
      const db = new Database(dbPath);
      try {
        db.pragma('foreign_keys = OFF');
        db.prepare('UPDATE rust_checkpoint_v3_current SET checkpoint_id = ?').run('f'.repeat(64));
      } finally { db.close(); }
      server = await startRustServer({ ...config, resume: 'latest' });
      const health = await (await fetch(`http://127.0.0.1:${server.port}/health`)).json() as { runId: string; recovery: unknown };
      expect(health.runId).not.toBe(initial.runId);
      expect(health.recovery).toMatchObject({ failedRunId: initial.runId, branchRunId: health.runId,
        recoveredCheckpointId: initial.startupCheckpointId, lostCompletedGenerations: null });
      const viewer = await connect(server.port, 'ui'); peers.push(viewer);
      await until(viewer, () => viewer.packets.some(packet => packet['type'] === 'welcome'));
      expect(viewer.packets.find(packet => packet['type'] === 'welcome')).toMatchObject({ recovery: health.recovery });
      viewer.socket.terminate();
      await server.close();
      const files = await readdir(`${dbPath}.checkpoints`);
      for (const file of files.filter(name => name.endsWith('.checkpoint-v3'))) await writeFile(join(`${dbPath}.checkpoints`, file), 'corrupt');
      server = await startRustServer({ ...config, resume: 'latest' });
      expect(server.startupFault).toMatch(/no valid retained/);
      const fault = await fetch(`http://127.0.0.1:${server.port}/api/health`);
      expect(fault.status).toBe(503);
      expect(await fault.json()).toMatchObject({ ok: false, lifecycle: 'startup-fault' });
      const rejected = new WebSocket(`ws://127.0.0.1:${server.port}`);
      await new Promise<void>((done, reject) => {
        rejected.once('unexpected-response', (_request, response) => {
          expect(response.statusCode).toBe(503); response.resume(); rejected.terminate(); done();
        });
        rejected.once('open', () => reject(new Error('faulted startup accepted a game socket')));
        rejected.on('error', () => {});
      });
    } finally {
      for (const peer of peers) peer.socket.terminate();
      await server.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);

  it('serves native welcome, frames, controller observations and same-snake token reclaim', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-server-'));
    const peers: Peer[] = [];
    const server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, resume: 'fresh', seed: 42,
      rustCalculationWorkers: 4, dbPath: join(root, 'experiment.sqlite') });
    try {
      const viewer = await connect(server.port, 'ui'); peers.push(viewer);
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator' }));
      await until(viewer, () => viewer.frames > 0 && viewer.packets.some(packet => packet['type'] === 'stats'));
      expect(viewer.packets.find(packet => packet['type'] === 'welcome')).toMatchObject({ protocolVersion: 2, worldSeed: 42,
        sensorSpec: { sensorCount: 83 }, inferenceMode: { activeBackend: 'native', activeWorkerCount: 4 } });
      viewer.socket.send(JSON.stringify({ type: 'viz', enabled: true }));
      await until(viewer, () => viewer.packets.some(packet => {
        const viz = packet['viz'] as { layers?: unknown[] } | undefined;
        return packet['type'] === 'stats' && viz?.layers?.length === 5;
      }));
      expect(viewer.packets.findLast(packet => packet['viz'])).toMatchObject({
        viz: {
          kind: 'graph', snakeId: expect.any(Number),
          layers: [
            { count: 83, activations: null },
            { count: 64, activations: expect.any(Array) },
            { count: 64, activations: expect.any(Array) },
            { count: 16, activations: expect.any(Array), isRecurrent: true },
            { count: 2, activations: expect.any(Array) }
          ]
        }
      });
      viewer.socket.send(JSON.stringify({ type: 'viz', enabled: false }));
      const statsBeforeDisable = viewer.packets.filter(packet => packet['type'] === 'stats').length;
      await until(viewer, () => viewer.packets.filter(packet => packet['type'] === 'stats').length > statsBeforeDisable);
      expect(viewer.packets.findLast(packet => packet['type'] === 'stats')?.['viz']).toBeUndefined();
      const selected = firstFrameSnake(viewer.latestFrame);
      expect(selected).toBeDefined();
      if (!selected) throw new Error('native frame omitted every alive snake');
      viewer.socket.send(JSON.stringify({ type: 'godMode', requestId: 'native-god-move', action: 'move',
        snakeId: selected.snakeId, x: selected.x * 0.9, y: selected.y * 0.9 }));
      await until(viewer, () => viewer.packets.some(packet => packet['requestId'] === 'native-god-move'));
      expect(viewer.packets.findLast(packet => packet['requestId'] === 'native-god-move')).toMatchObject({
        type: 'godModeResult', action: 'move', snakeId: selected.snakeId, applied: true,
        sequence: expect.any(Number), step: expect.any(Number), x: expect.any(Number), y: expect.any(Number)
      });
      viewer.socket.send(JSON.stringify({ type: 'godMode', requestId: 'native-god-kill', action: 'kill',
        snakeId: selected.snakeId }));
      await until(viewer, () => viewer.packets.some(packet => packet['requestId'] === 'native-god-kill'));
      const killResult = viewer.packets.findLast(packet => packet['requestId'] === 'native-god-kill');
      expect(killResult).toMatchObject({
        type: 'godModeResult', action: 'kill', snakeId: selected.snakeId, applied: true,
        sequence: expect.any(Number), step: expect.any(Number), pelletsDropped: expect.any(Number)
      });
      expect(Number(killResult?.['pelletsDropped'])).toBeGreaterThan(0);
      await until(viewer, () => frameDirection(viewer.latestFrame, selected.snakeId) === undefined);
      viewer.socket.send(JSON.stringify({ type: 'godMode', requestId: 'native-god-missing', action: 'move',
        snakeId: selected.snakeId, x: 0, y: 0 }));
      await until(viewer, () => viewer.packets.some(packet => packet['requestId'] === 'native-god-missing'));
      expect(viewer.packets.findLast(packet => packet['requestId'] === 'native-god-missing')).toMatchObject({
        type: 'godModeResult', action: 'move', snakeId: selected.snakeId, applied: false,
        reason: expect.stringContaining('missing or already dead')
      });
      const bot = await connect(server.port, 'bot'); peers.push(bot);
      bot.socket.send(JSON.stringify({ type: 'join', mode: 'player', name: 'socket-bot' }));
      await until(bot, () => bot.packets.some(packet => packet['type'] === 'sensors'));
      const assignment = bot.packets.find(packet => packet['type'] === 'assign')!;
      const sample = bot.packets.find(packet => packet['type'] === 'sensors')!;
      expect(sample['snakeId']).toBe(assignment['snakeId']);
      expect(sample['sensors']).toHaveLength(83);
      bot.socket.send(JSON.stringify({ type: 'action', snakeId: assignment['snakeId'], tick: sample['tick'], turn: 0.4, boost: 0 }));
      await until(bot, () => bot.packets.some(packet => packet['type'] === 'sensors' && Number(packet['tick']) > Number(sample['tick'])));
      await new Promise<void>(done => { bot.socket.once('close', () => done()); bot.socket.close(); });
      const resumed = await connect(server.port, 'bot'); peers.push(resumed);
      resumed.socket.send(JSON.stringify({ type: 'join', mode: 'player', name: 'socket-bot', resumeToken: assignment['resumeToken'] }));
      await until(resumed, () => resumed.packets.some(packet => packet['type'] === 'sensors'));
      expect(resumed.packets.find(packet => packet['type'] === 'reclaimResult')).toMatchObject({ reclaimed: true, snakeId: assignment['snakeId'] });
      const nextAssignment = resumed.packets.find(packet => packet['type'] === 'assign')!;
      expect(nextAssignment).toMatchObject({ reclaimed: true, snakeId: assignment['snakeId'] });
      expect(nextAssignment['resumeToken']).not.toBe(assignment['resumeToken']);
      /** Match the production latest-action pump so a runner-delayed sample cannot be the only attempt. */
      const actionPump = setInterval(() => {
        const latest = resumed.packets.findLast(packet => packet['type'] === 'sensors');
        if (latest && resumed.socket.readyState === WebSocket.OPEN) {
          resumed.socket.send(JSON.stringify({ type: 'action', snakeId: nextAssignment['snakeId'],
            tick: latest['tick'], turn: 0.4, boost: 0 }));
        }
      }, 10);
      let health: Record<string, unknown>;
      try {
        health = await healthUntil(server.port, value => {
          const telemetry = value['telemetry'] as { trainerAction?: { samples?: number } } | undefined;
          return (telemetry?.trainerAction?.samples ?? 0) > 0;
        });
      } finally {
        clearInterval(actionPump);
      }
      expect(health).toMatchObject({
        ok: true,
        authority: 'rust',
        calculationWorkers: 4,
        seed: 42,
        nativeQueues: {
          inbound: { maxBatches: '0000000000000040', maxCommands: '0000000000000040',
            maxOwnedBytes: '0000000000400000', maxBatchCommands: '0000000000000001',
            maxBatchOwnedBytes: '0000000000100000' },
          output: { maxReliable: '0000000000000020', maxDiscrete: '0000000000000004',
            maxOwnedBytes: '0000000002000000', maxFrames: '0000000000000004', hasReservedFault: false }
        },
        outbound: {
          connections: 2,
          replacedFrames: expect.any(Number),
          reliableFailures: expect.any(Number)
        },
        telemetry: {
          authoritativeSteps: expect.any(Number),
          simulatedWallRatio: expect.any(Number),
          step: { samples: expect.any(Number), p95Ms: expect.any(Number), p99Ms: expect.any(Number) },
          process: { rssBytes: expect.any(Number), eventLoopDelayP95Ms: expect.any(Number) },
          frame: { latestBytes: expect.any(Number), maximumObservedBytes: expect.any(Number) },
          trainerAction: { samples: expect.any(Number) },
          playerAction: { samples: 0 },
          controllerLifecycle: { samples: 2 },
          controllerActivity: {
            player: { freshAssignments: 0, successfulReclaims: 0, appliedActions: 0, appliedDisconnects: 0 },
            trainer: { freshAssignments: 1, successfulReclaims: 1, appliedActions: expect.any(Number),
              appliedDisconnects: expect.any(Number) }
          }
        },
        retention: {
          schemaVersion: 1,
          retained: { latest: { checkpointCount: 1 }, pinned: { checkpointCount: 0 } },
          plannedPrune: { checkpointCount: 0 }
        },
        retentionCleanup: { deletedCheckpointCount: 0, deletedStoredByteCount: '0000000000000000' },
        storage: {
          schemaVersion: 1,
          sqlite: {
            databaseBytes: expect.stringMatching(/^[1-9][0-9]*$/u),
            walBytes: expect.stringMatching(/^[0-9]+$/u),
            pageSizeBytes: expect.stringMatching(/^[1-9][0-9]*$/u),
            pageCount: expect.stringMatching(/^[1-9][0-9]*$/u),
            freelistPageCount: expect.stringMatching(/^[0-9]+$/u),
            usedPageBytes: expect.stringMatching(/^[1-9][0-9]*$/u)
          },
          managed: {
            temporaryBytes: expect.stringMatching(/^[0-9]+$/u),
            temporaryQuotaBytes: expect.stringMatching(/^[1-9][0-9]*$/u),
            freeBytes: expect.stringMatching(/^[1-9][0-9]*$/u),
            operatingReserveBytes: expect.stringMatching(/^[1-9][0-9]*$/u)
          }
        }
      });
      const queues = health['nativeQueues'] as RustQueueDiagnostics;
      for (const group of [queues.inbound, queues.output]) {
        for (const value of Object.values(group)) {
          if (typeof value === 'string') expect(value).toMatch(/^[0-9a-f]{16}$/u);
        }
      }
      expect(BigInt(`0x${queues.inbound.highWaterCommands}`)).toBeGreaterThan(0n);
      expect(BigInt(`0x${queues.inbound.highWaterCommands}`)).toBeLessThanOrEqual(BigInt(`0x${queues.inbound.maxCommands}`));
      expect(BigInt(`0x${queues.inbound.highWaterOwnedBytes}`)).toBeLessThanOrEqual(BigInt(`0x${queues.inbound.maxOwnedBytes}`));
      expect(BigInt(`0x${queues.output.highWaterCount}`)).toBeGreaterThan(0n);
      expect(BigInt(`0x${queues.output.highWaterCount}`)).toBeLessThanOrEqual(
        BigInt(`0x${queues.output.maxReliable}`) + BigInt(`0x${queues.output.maxDiscrete}`) + BigInt(`0x${queues.output.maxFrames}`) + 1n);
      expect(BigInt(`0x${queues.output.highWaterOwnedBytes}`)).toBeLessThanOrEqual(BigInt(`0x${queues.output.maxOwnedBytes}`));
      expect(queues.inbound.faultDiscardedCommands).toBe('0000000000000000');
      expect(queues.output.priorityOverflows).toBe('0000000000000000');
      const activity = (health['telemetry'] as {
        trainerAction: { samples: number };
        controllerActivity: { trainer: { appliedActions: number } };
      });
      expect(activity.trainerAction.samples).toBeGreaterThan(0);
      expect(activity.controllerActivity.trainer.appliedActions).toBeGreaterThan(0);
      const pinResponse = await fetch(`http://127.0.0.1:${server.port}/api/checkpoints/current/pin`, { method: 'POST' });
      expect(pinResponse.status).toBe(200);
      const pinned = await pinResponse.json() as Record<string, unknown>;
      expect(pinned).toMatchObject({ ok: true, checkpointId: expect.stringMatching(/^[0-9a-f]{64}$/u), generation: '0000000000000001' });
      const pinnedHealth = await healthUntil(server.port, value => {
        const retention = value['retention'] as { retained?: { pinned?: { checkpointCount?: number } } } | undefined;
        return retention?.retained?.pinned?.checkpointCount === 1;
      });
      expect(pinnedHealth).toMatchObject({ retention: { retained: { pinned: { checkpointCount: 1 } } } });
      const beforeReset = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as {
        runId: string; seed: number; startupCheckpointId: string;
      };
      const replacementGraph = {
        type: 'graph', nodes: [
          { id: 'input', type: 'Input', outputSize: 51 },
          { id: 'features', type: 'MLP', inputSize: 51, hiddenSizes: [8], outputSize: 8 },
          { id: 'memory', type: 'GRU', inputSize: 8, hiddenSize: 4 },
          { id: 'head', type: 'Dense', inputSize: 4, outputSize: 2 }
        ], edges: [
          { from: 'input', to: 'features' }, { from: 'features', to: 'memory' },
          { from: 'memory', to: 'head' }
        ], outputs: [{ nodeId: 'head' }], outputSize: 2
      };
      const savePresetResponse = await fetch(`http://127.0.0.1:${server.port}/api/graph-presets`, {
        method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ name: 'Compact memory', spec: replacementGraph })
      });
      expect(savePresetResponse.status).toBe(200);
      const savedPreset = await savePresetResponse.json() as { presetId: number };
      expect(savedPreset.presetId).toBeGreaterThan(0);
      expect(await (await fetch(`http://127.0.0.1:${server.port}/api/graph-presets`)).json()).toMatchObject({
        ok: true, presets: [{ id: savedPreset.presetId, name: 'Compact memory', createdAt: expect.any(Number) }]
      });
      expect(await (await fetch(
        `http://127.0.0.1:${server.port}/api/graph-presets/${savedPreset.presetId}`
      )).json()).toMatchObject({
        ok: true, preset: { id: savedPreset.presetId, name: 'Compact memory', spec: replacementGraph }
      });
      viewer.socket.send(JSON.stringify({ type: 'reset', settings: { snakeCount: 12, simSpeed: 2 }, updates: [
        { path: 'worldRadius', value: 4_200 },
        { path: 'generationSeconds', value: 90 },
        { path: 'sense.bubbleBins', value: 8 },
        { path: 'baselineBots.count', value: 3 },
        { path: 'foodSpawn.edgeFalloffEnabled', value: 0 }
      ], graphSpec: replacementGraph }));
      await until(viewer, () => viewer.packets.some(packet => packet['type'] === 'stateReplaced' && packet['reason'] === 'reset'));
      const resetNotice = viewer.packets.findLast(packet => packet['type'] === 'stateReplaced');
      expect(resetNotice).toMatchObject({ reason: 'reset', welcome: { worldSeed: 42,
        graphSpec: replacementGraph, inferenceMode: { parameterCount: 654 }, settings: {
        core: { snakeCount: 12, simSpeed: 2 }, updates: expect.arrayContaining([
          { path: 'worldRadius', value: 4_200 },
          { path: 'generationSeconds', value: 90 },
          { path: 'sense.bubbleBins', value: 8 },
          { path: 'baselineBots.count', value: 3 },
          { path: 'foodSpawn.edgeFalloffEnabled', value: 0 }
        ])
      } } });
      const afterReset = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as {
        runId: string; seed: number; startupCheckpointId: string;
      };
      expect(afterReset).toMatchObject({ seed: beforeReset.seed, calculationWorkers: 4 });
      expect(afterReset.runId).not.toBe(beforeReset.runId);
      expect(afterReset.startupCheckpointId).not.toBe(beforeReset.startupCheckpointId);

      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator' }));
      viewer.socket.send(JSON.stringify({
        type: 'settings', requestId: 'native-settings-rejected',
        updates: [{ path: 'snakeCount', value: 40 }]
      }));
      await until(viewer, () => viewer.packets.some(packet =>
        packet['type'] === 'settingsApplied' && packet['requestId'] === 'native-settings-rejected'));
      expect(viewer.packets.findLast(packet => packet['requestId'] === 'native-settings-rejected'))
        .toMatchObject({ applied: false, updates: [], reason: expect.stringContaining('requires reset') });
      viewer.socket.send(JSON.stringify({
        type: 'settings', requestId: 'native-settings-applied', updates: [
          { path: 'simSpeed', value: 2 },
          { path: 'reward.pointsPerKill', value: 275 },
          { path: 'foodSpawn.edgeFalloffEnabled', value: 0 }
        ]
      }));
      await until(viewer, () => viewer.packets.some(packet =>
        packet['type'] === 'settingsApplied' && packet['requestId'] === 'native-settings-applied'));
      const settingsApplied = viewer.packets.findLast(packet => packet['requestId'] === 'native-settings-applied');
      expect(settingsApplied).toMatchObject({ applied: true, configRevision: 2,
        configHash: expect.stringMatching(/^sha256:[0-9a-f]{64}$/u), sequence: expect.any(Number),
        step: expect.any(Number), updates: [
          { path: 'simSpeed', value: 2 },
          { path: 'reward.pointsPerKill', value: 275 },
          { path: 'foodSpawn.edgeFalloffEnabled', value: 0 }
        ] });
      expect(await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json()).toMatchObject({
        configRevision: 2, configHash: settingsApplied?.['configHash']
      });
      viewer.socket.send(JSON.stringify({ type: 'newRun', requestId: 'native-new-run' }));
      await until(viewer, () => viewer.packets.some(packet =>
        packet['type'] === 'stateReplaced' && packet['reason'] === 'newRun') &&
        viewer.packets.some(packet => packet['type'] === 'newRunResult' && packet['requestId'] === 'native-new-run'));
      const newRunResult = viewer.packets.findLast(packet => packet['type'] === 'newRunResult');
      expect(newRunResult).toMatchObject({ requestId: 'native-new-run', applied: true });
      const afterNewRun = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as {
        runId: string; seed: number; startupCheckpointId: string; configRevision: number; configHash: string;
      };
      expect(afterNewRun).toMatchObject({ runId: newRunResult?.['runId'], seed: newRunResult?.['worldSeed'],
        calculationWorkers: 4 });
      expect(afterNewRun.runId).not.toBe(afterReset.runId);
      expect(afterNewRun).toMatchObject({ configRevision: 1, configHash: settingsApplied?.['configHash'] });
      const newRunNotice = viewer.packets.findLast(packet =>
        packet['type'] === 'stateReplaced' && packet['reason'] === 'newRun');
      expect(newRunNotice).toMatchObject({ welcome: { configHash: settingsApplied?.['configHash'],
        graphSpec: replacementGraph, inferenceMode: { parameterCount: 654 },
        settings: { core: { snakeCount: 12, simSpeed: 2 }, updates: expect.arrayContaining([
          { path: 'baselineBots.count', value: 3 }
        ]) } } });
    } finally {
      for (const peer of peers) peer.socket.terminate();
      await server.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);

  it('serves the built browser and applies its independent latest-action pump through Rust', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-browser-action-'));
    const server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, resume: 'fresh', seed: 73,
      dbPath: join(root, 'experiment.sqlite') });
    let pump: PlayerActionPump | undefined;
    let browser: WsClient | undefined;
    try {
      const page = await fetch(`http://127.0.0.1:${server.port}/`);
      expect(page.status).toBe(200);
      expect(page.headers.get('content-type')).toMatch(/^text\/html/u);
      expect(await page.text()).toContain('<canvas id="c"');

      vi.stubGlobal('WebSocket', WebSocket);
      let welcome: WelcomeMsg | undefined;
      let assignment: AssignMsg | undefined;
      let sample: SensorsMsg | undefined;
      let latestFrame: Buffer | undefined;
      const errors: string[] = [];
      browser = createWsClient({
        onConnected(info) { welcome = info; browser?.sendJoin('player', 'browser-pump'); },
        onDisconnected() {},
        onFrame(frame) { latestFrame = Buffer.from(frame); },
        onStats() {},
        onAssign(message) { assignment = message; },
        // Incoming sensors update observation state only; they never produce an action.
        onSensors(message) { sample = message; },
        onError(message) { errors.push(message.message); }
      });
      browser.connect(`ws://127.0.0.1:${server.port}`);
      const deadline = performance.now() + 5000;
      while ((!welcome || !assignment || !sample) && performance.now() < deadline) await new Promise<void>(done => setTimeout(done, 10));
      expect(welcome).toMatchObject({ protocolVersion: 2, worldSeed: 73, inferenceMode: { activeBackend: 'native' } });
      expect(assignment).toBeDefined();
      expect(sample).toBeDefined();
      if (!assignment || !sample) throw new Error(`browser transport did not assign: ${errors.join('; ')}`);
      const snakeId = assignment.snakeId;
      const clientTick = sample.tick;
      const frameDeadline = performance.now() + 5000;
      while (frameDirection(latestFrame, snakeId) === undefined && performance.now() < frameDeadline) await new Promise<void>(done => setTimeout(done, 10));
      const initialDirection = frameDirection(latestFrame, snakeId);
      expect(initialDirection).toEqual(expect.any(Number));

      let turn = 1;
      let boost = 1;
      const actions: Array<{ turn: number; boost: number }> = [];
      pump = new PlayerActionPump({ cadenceHz: 60, isActive: () => browser?.isConnected() === true,
        buildLatestAction: () => ({ tick: clientTick, snakeId, turn, boost }),
        sendAction: action => { actions.push({ turn: action.turn, boost: action.boost });
          browser?.sendAction(action.tick, action.snakeId, action.turn, action.boost); } });
      // No sensor or frame callback invokes the pump: its own timer and change
      // request are the only producers while incoming state is merely observed.
      pump.start();
      const firstTurnDeadline = performance.now() + 5000;
      while (performance.now() < firstTurnDeadline) {
        const direction = frameDirection(latestFrame, snakeId);
        if (direction !== undefined && initialDirection !== undefined && directionDelta(initialDirection, direction) > 0.02) break;
        await new Promise<void>(done => setTimeout(done, 10));
      }
      const beforeRelease = frameDirection(latestFrame, snakeId)!;
      expect(directionDelta(initialDirection!, beforeRelease)).toBeGreaterThan(0.02);
      turn = -1;
      boost = 0;
      pump.requestImmediate();
      const reverseDeadline = performance.now() + 5000;
      while (performance.now() < reverseDeadline) {
        const direction = frameDirection(latestFrame, snakeId);
        if (actions.some(action => action.turn === -1 && action.boost === 0) &&
            direction !== undefined && directionDelta(beforeRelease, direction) < -0.02) break;
        await new Promise<void>(done => setTimeout(done, 10));
      }
      expect(directionDelta(beforeRelease, frameDirection(latestFrame, snakeId)!)).toBeLessThan(-0.02);
      expect(actions.at(-1)).toEqual({ turn: -1, boost: 0 });
      expect(errors).toEqual([]);
      const health = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as {
        telemetry: { playerAction: { samples: number } };
      };
      expect(health.telemetry.playerAction.samples).toBeGreaterThan(0);
    } finally {
      pump?.stop();
      browser?.disconnect();
      vi.unstubAllGlobals();
      await server.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);

  it('delivers fresh assignment and same-snake reclaim while display frames remain backpressured', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-frame-pressure-'));
    const peers: Peer[] = [];
    const bufferedAmount = Object.getOwnPropertyDescriptor(WebSocket.prototype, 'bufferedAmount')?.get;
    if (!bufferedAmount) throw new Error('real ws transport has no buffered-amount getter');
    /** Induce only the server display admission condition; real TCP JSON sends still execute. */
    const pressure = vi.spyOn(WebSocket.prototype, 'bufferedAmount', 'get').mockImplementation(function (this: WebSocket) {
      return (this as WebSocket & { _isServer: boolean })._isServer
        ? 1024 * 1024 : Number(bufferedAmount.call(this));
    });
    let server: Awaited<ReturnType<typeof startRustServer>> | undefined;
    try {
      server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, resume: 'fresh', seed: 76,
        dbPath: join(root, 'experiment.sqlite') });
      const first = await connect(server.port, 'ui'); peers.push(first);
      first.socket.send(JSON.stringify({ type: 'join', mode: 'player', name: 'frame-pressure-player' }));
      await until(first, () => first.packets.some(packet => packet['type'] === 'assign'));
      const assignment = first.packets.find(packet => packet['type'] === 'assign')!;
      const initial = await healthUntil(server.port, value => {
        const outbound = value['outbound'] as WsOutboundDiagnostics;
        return outbound.replacedFrames >= 3 && outbound.pendingFrames === 1;
      });
      expect(first.frames).toBe(0);
      expect(first.packets.filter(packet => packet['type'] === 'assign')).toHaveLength(1);
      expect((initial['telemetry'] as { controllerActivity: { player: { freshAssignments: number } } })
        .controllerActivity.player.freshAssignments).toBe(1);
      await new Promise<void>(done => { first.socket.once('close', done); first.socket.close(); });
      const resumed = await connect(server.port, 'ui'); peers.push(resumed);
      resumed.socket.send(JSON.stringify({ type: 'join', mode: 'player', name: 'frame-pressure-player',
        resumeToken: assignment['resumeToken'] }));
      await until(resumed, () => resumed.packets.some(packet => packet['type'] === 'assign') &&
        resumed.packets.some(packet => packet['type'] === 'reclaimResult'));
      expect(resumed.packets.filter(packet => packet['type'] === 'assign')).toHaveLength(1);
      expect(resumed.packets.filter(packet => packet['type'] === 'reclaimResult')).toHaveLength(1);
      expect(resumed.packets.find(packet => packet['type'] === 'reclaimResult')).toMatchObject({ reclaimed: true,
        snakeId: assignment['snakeId'] });
      const reclaimed = resumed.packets.find(packet => packet['type'] === 'assign')!;
      expect(reclaimed).toMatchObject({ reclaimed: true, snakeId: assignment['snakeId'] });
      expect(reclaimed['resumeToken']).not.toBe(assignment['resumeToken']);
      const measured = await healthUntil(server.port, value => {
        const outbound = value['outbound'] as WsOutboundDiagnostics;
        return outbound.replacedFrames >= (initial['outbound'] as WsOutboundDiagnostics).replacedFrames + 3 &&
          outbound.pendingFrames === 1 && BigInt(`0x${String(value['completedStep'])}`) > BigInt(`0x${String(initial['completedStep'])}`);
      });
      expect(resumed.frames).toBe(0);
      expect(measured).toMatchObject({ ok: true, outbound: { connections: 1, pendingFrames: 1, reliableFailures: 0 },
        telemetry: { controllerActivity: { player: { freshAssignments: 1, successfulReclaims: 1 } } } });
      const outbound = measured['outbound'] as WsOutboundDiagnostics;
      expect(outbound.highWaterReliableMessagesPerConnection).toBeGreaterThan(0);
      expect(outbound.highWaterReliableMessagesPerConnection).toBeLessThanOrEqual(outbound.maxReliableMessagesPerConnection);
      expect(outbound.highWaterReliableBytesPerConnection).toBeLessThanOrEqual(outbound.maxReliableBytesPerConnection);
      const queues = measured['nativeQueues'] as RustQueueDiagnostics;
      expect(queues.output.priorityOverflows).toBe('0000000000000000');
      expect(queues.inbound.faultDiscardedCommands).toBe('0000000000000000');
      expect(BigInt(`0x${queues.output.highWaterOwnedBytes}`)).toBeLessThanOrEqual(BigInt(`0x${queues.output.maxOwnedBytes}`));
      pressure.mockRestore();
      await until(resumed, () => resumed.frames > 0);
      expect(frameDirection(resumed.latestFrame, Number(assignment['snakeId']))).toBeDefined();
    } finally {
      pressure.mockRestore();
      for (const peer of peers) peer.socket.terminate();
      await server?.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);

  it('applies browser-player input while inbound frames and sensors are paused', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-browser-suppression-'));
    const server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, resume: 'fresh', seed: 74,
      dbPath: join(root, 'experiment.sqlite') });
    try {
      const report = await runStage6RuntimeProbe({
        wsUrl: `ws://127.0.0.1:${server.port}/`,
        durationMs: 4_000,
        reconnectAfterSensors: 2,
        requireGenerationTransition: false,
        playerSuppressionMs: 750,
        requireFrameReplacement: false
      });
      const browserPlayer = report['browserPlayer'] as {
        actionsDuringSuppression: number;
        inboundDuringSuppression: number;
        latestTurn: number;
        latestBoost: number;
      };
      const suppression = report['playerSuppression'] as {
        serverPlayerActionSamplesBefore: number;
        serverPlayerActionSamplesAfter: number;
        recoveredSensors: number;
        recoveredFrames: number;
      };
      expect(browserPlayer.actionsDuringSuppression).toBeGreaterThan(0);
      expect(browserPlayer.inboundDuringSuppression).toBe(0);
      expect(browserPlayer).toMatchObject({ latestTurn: -1, latestBoost: 0 });
      expect(suppression.serverPlayerActionSamplesAfter)
        .toBeGreaterThan(suppression.serverPlayerActionSamplesBefore);
      expect(suppression.recoveredSensors).toBeGreaterThan(0);
      expect(suppression.recoveredFrames).toBeGreaterThan(0);
    } finally {
      await server.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);
});
