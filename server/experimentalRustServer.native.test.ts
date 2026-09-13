import { mkdtemp, rm, readdir, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { expect, it, vi } from 'vitest';
import WebSocket from 'ws';
import Database from 'better-sqlite3';
import { DEFAULT_CONFIG, normalizeConfig } from './config.ts';
import { PlayerActionPump } from '../src/net/playerActionPump.ts';
import { createWsClient, type AssignMsg, type SensorsMsg, type WelcomeMsg, type WsClient } from '../src/net/wsClient.ts';
import { run as runStage6RuntimeProbe } from '../scripts/stage6/runtime-integration-probe.ts';
import { startExperimentalRustServer } from './experimentalRustServer.ts';
import { describeNetworkSuite } from './test/networkSuites.ts';

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

describeNetworkSuite('experimental Rust server real sockets', () => {
  it('streams, imports, and atomically activates one exact Rust save', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-export-server-'));
    const dbPath = join(root, 'experiment.sqlite');
    const managedDirectory = `${dbPath}.checkpoints`;
    const server = await startExperimentalRustServer({
      ...DEFAULT_CONFIG, port: 0, resume: 'fresh', seed: 41, dbPath
    });
    let target: Awaited<ReturnType<typeof startExperimentalRustServer>> | undefined;
    const peers: Peer[] = [];
    try {
      const health = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as {
        runId: string; startupCheckpointId: string;
      };
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

      const targetDbPath = join(root, 'target.sqlite');
      target = await startExperimentalRustServer({
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
      target = await startExperimentalRustServer({
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

  it('resumes exact managed IDs and exposes recovery or health-only failure over real HTTP/WebSocket', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-recovery-server-'));
    const dbPath = join(root, 'experiment.sqlite');
    const config = { ...DEFAULT_CONFIG, port: 0, dbPath };
    let server = await startExperimentalRustServer({ ...config, resume: 'fresh', seed: 42 });
    const peers: Peer[] = [];
    try {
      const initial = await (await fetch(`http://127.0.0.1:${server.port}/health`)).json() as { runId: string; startupCheckpointId: string };
      await server.close();
      const exact = normalizeConfig({ ...config, resume: initial.startupCheckpointId });
      expect(exact.resume).toBe(`sha256:${initial.startupCheckpointId}`);
      server = await startExperimentalRustServer({ ...exact, port: 0 });
      expect(await (await fetch(`http://127.0.0.1:${server.port}/health`)).json()).toMatchObject({ ok: true, runId: initial.runId });
      await server.close();
      const db = new Database(dbPath);
      try {
        db.pragma('foreign_keys = OFF');
        db.prepare('UPDATE rust_checkpoint_v3_current SET checkpoint_id = ?').run('f'.repeat(64));
      } finally { db.close(); }
      server = await startExperimentalRustServer({ ...config, resume: 'latest' });
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
      server = await startExperimentalRustServer({ ...config, resume: 'latest' });
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
    const server = await startExperimentalRustServer({ ...DEFAULT_CONFIG, port: 0, resume: 'fresh', seed: 42, dbPath: join(root, 'experiment.sqlite') });
    try {
      const viewer = await connect(server.port, 'ui'); peers.push(viewer);
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator' }));
      await until(viewer, () => viewer.frames > 0 && viewer.packets.some(packet => packet['type'] === 'stats'));
      expect(viewer.packets.find(packet => packet['type'] === 'welcome')).toMatchObject({ protocolVersion: 2, worldSeed: 42,
        sensorSpec: { sensorCount: 83 }, inferenceMode: { activeBackend: 'native', activeWorkerCount: 0 } });
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
        seed: 42,
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
        retentionCleanup: { deletedCheckpointCount: 0, deletedStoredByteCount: '0000000000000000' }
      });
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
      viewer.socket.send(JSON.stringify({ type: 'reset', settings: { simSpeed: 2 }, updates: [
        { path: 'worldRadius', value: 4_200 },
        { path: 'generationSeconds', value: 90 },
        { path: 'sense.bubbleBins', value: 8 },
        { path: 'foodSpawn.edgeFalloffEnabled', value: 0 }
      ], graphSpec: replacementGraph }));
      await until(viewer, () => viewer.packets.some(packet => packet['type'] === 'stateReplaced' && packet['reason'] === 'reset'));
      const resetNotice = viewer.packets.findLast(packet => packet['type'] === 'stateReplaced');
      expect(resetNotice).toMatchObject({ reason: 'reset', welcome: { worldSeed: 42,
        graphSpec: replacementGraph, inferenceMode: { parameterCount: 654 }, settings: {
        core: { simSpeed: 2 }, updates: expect.arrayContaining([
          { path: 'worldRadius', value: 4_200 },
          { path: 'generationSeconds', value: 90 },
          { path: 'sense.bubbleBins', value: 8 },
          { path: 'foodSpawn.edgeFalloffEnabled', value: 0 }
        ])
      } } });
      const afterReset = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as {
        runId: string; seed: number; startupCheckpointId: string;
      };
      expect(afterReset).toMatchObject({ seed: beforeReset.seed });
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
      expect(afterNewRun).toMatchObject({ runId: newRunResult?.['runId'], seed: newRunResult?.['worldSeed'] });
      expect(afterNewRun.runId).not.toBe(afterReset.runId);
      expect(afterNewRun).toMatchObject({ configRevision: 1, configHash: settingsApplied?.['configHash'] });
      const newRunNotice = viewer.packets.findLast(packet =>
        packet['type'] === 'stateReplaced' && packet['reason'] === 'newRun');
      expect(newRunNotice).toMatchObject({ welcome: { configHash: settingsApplied?.['configHash'],
        graphSpec: replacementGraph, inferenceMode: { parameterCount: 654 },
        settings: { core: { simSpeed: 2 } } } });
    } finally {
      for (const peer of peers) peer.socket.terminate();
      await server.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);

  it('serves the built browser and applies its independent latest-action pump through Rust', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-browser-action-'));
    const server = await startExperimentalRustServer({ ...DEFAULT_CONFIG, port: 0, resume: 'fresh', seed: 73,
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

  it('applies browser-player input while inbound frames and sensors are paused', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-browser-suppression-'));
    const server = await startExperimentalRustServer({ ...DEFAULT_CONFIG, port: 0, resume: 'fresh', seed: 74,
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
