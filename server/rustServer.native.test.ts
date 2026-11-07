import { mkdtemp,rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { expect,it,vi } from 'vitest';
import WebSocket from 'ws';
import { run as runStage6RuntimeProbe } from '../scripts/stage6/runtime-integration-probe.ts';
import { measureTurnResponses } from '../scripts/stage7/lan-turn-response.ts';
import { PlayerReconnectExchange } from '../scripts/stage7/player-reconnect-exchange.ts';
import { buildStackGraphSpec } from '../src/brains/stackBuilder.ts';
import type { SelectedEntity } from '../src/protocol/entitySelection.ts';
import type { RustQueueDiagnostics } from '../src/protocol/rustBackground.ts';
import { startRustServer } from './rustServer.ts';
import { ACTIVE_WORKERS,connect,firstFrameSnake,frameDirection,healthUntil,PARALLEL_WORKERS,replacementUntil,selectPhysical,until,type Peer } from './rustServerFixture.ts';
import { describeNetworkSuite } from './test/networkSuites.ts';
import { RUST_TEST_CONFIG as DEFAULT_CONFIG } from './test/rustConfig.ts';
import type { WsOutboundDiagnostics } from './wsHub.ts';
import { WsHub } from './wsHub.ts';

describeNetworkSuite('Rust server real sockets', () => {

  it('enforces browser origins on production HTTP, WebSockets and startup-fault health', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-browser-origin-'));
    const peers: Peer[] = [];
    let server: Awaited<ReturnType<typeof startRustServer>> | undefined;
    try {
      const config = { ...DEFAULT_CONFIG, port: 0, resume: 'fresh' as const, seed: 42,
        rustCalculationWorkers: 1, dbPath: join(root, 'experiment.sqlite') };
      server = await startRustServer(config);
      const url = `http://127.0.0.1:${server.port}`;
      const before = await (await fetch(`${url}/api/health`)).json() as Record<string, unknown>;
      for (const path of ['/api/export/latest', '/api/import/archive', '/api/checkpoints/current/pin', '/api/graph-presets', '/api/distillation/v1/library']) {
        const response = await fetch(`${url}${path}`, { method: path.includes('export') ? 'GET' : 'POST',
          headers: { Origin: 'http://evil.test', 'Content-Type': 'text/plain' } });
        expect(response.status).toBe(403);
        expect(response.headers.has('Access-Control-Allow-Origin')).toBe(false);
        await response.text();
      }
      const image = await fetch(`${url}/api/export/latest`, {
        headers: { 'Sec-Fetch-Site': 'cross-site', 'Sec-Fetch-Dest': 'image' }
      });
      expect(image.status).toBe(403);
      await image.text();
      const rejected = new WebSocket(`ws://127.0.0.1:${server.port}`, { origin: 'http://evil.test' });
      await new Promise<void>((done, reject) => {
        rejected.once('open', () => { rejected.terminate(); reject(new Error('untrusted upgrade accepted')); });
        rejected.once('error', error => {
          try { expect(error.message).toContain('403'); done(); } catch (failure) { reject(failure); }
        });
      });
      for (const origin of ['http://localhost:5173', url, undefined]) {
        const peer = await connect(server.port, 'ui', origin);
        peers.push(peer);
        peer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
          rejoinToken: peer.rejoinToken }));
        await until(peer, () => peer.frames > 0);
      }
      const catalog = await fetch(`${url}/api/distillation/v1/library`, {headers: {Origin: 'http://localhost:5173'}});
      expect(catalog.status).toBe(200);
      expect(await catalog.json()).toMatchObject({schemaVersion: 1, processEpoch: expect.stringMatching(/^[0-9a-f]{32}$/u),
        scope: {expectedRunId: before['runId']}, page: {entries: [], next: null}});
      const after = await (await fetch(`${url}/api/health`, { headers: { Origin: 'http://localhost:5173' } })).json() as Record<string, unknown>;
      expect(after['runId']).toBe(before['runId']);
      expect(after['configHash']).toBe(before['configHash']);
      for (const peer of peers) peer.socket.terminate();
      peers.length = 0;
      await server.close();
      server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, resume: `sha256:${'ab'.repeat(32)}`,
        rustCalculationWorkers: 1, dbPath: join(root, 'missing.sqlite') });
      expect(server.startupFault).toBeDefined();
      const faultUrl = `http://127.0.0.1:${server.port}/api/health`;
      const forbidden = await fetch(faultUrl, { headers: { Origin: 'http://evil.test' } });
      expect(forbidden.status).toBe(403);
      await forbidden.text();
      const health = await fetch(faultUrl, { headers: { Origin: 'http://localhost:5173' } });
      expect(health.status).toBe(503);
      expect(health.headers.get('Access-Control-Allow-Origin')).toBe('http://localhost:5173');
      expect(await health.json()).toMatchObject({ lifecycle: 'startup-fault' });
    } finally {
      for (const peer of peers) peer.socket.terminate();
      await server?.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 20_000);

  it('keeps the process session stable through live settings, reconnect, reset and New Run', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-process-session-'));
    const dbPath = join(root, 'experiment.sqlite');
    let server: Awaited<ReturnType<typeof startRustServer>> | undefined;
    const peers: Peer[] = [];
    try {
      server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, resume: 'fresh', seed: 42,
        rustCalculationWorkers: 1, dbPath });
      const viewer = await connect(server.port, 'ui'); peers.push(viewer);
      await until(viewer, () => viewer.packets.some(packet => packet['type'] === 'welcome'));
      const initial = viewer.packets.find(packet => packet['type'] === 'welcome')!;
      const sessionId = initial['sessionId'];
      expect(sessionId).toEqual(expect.any(String));
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
        rejoinToken: viewer.rejoinToken }));
      viewer.socket.send(JSON.stringify({ type: 'settings', requestId: 'session-live-settings',
        updates: [{ path: 'simSpeed', value: 0.1 }] }));
      await until(viewer, () => viewer.packets.some(packet => packet['type'] === 'settingsApplied' && packet['applied'] === true));
      const reconnected = await connect(server.port, 'ui'); peers.push(reconnected);
      await until(reconnected, () => reconnected.packets.some(packet => packet['type'] === 'welcome'));
      expect(reconnected.packets.find(packet => packet['type'] === 'welcome')).toMatchObject({
        sessionId, runId: initial['runId'], configRevision: 2, settings: { core: { simSpeed: 0.1 } }
      });
      const beforeReset = await healthUntil(server.port, health => Number(BigInt(`0x${health['completedStep'] as string}`)) >= 5);
      const stepsBeforeReset = (beforeReset['telemetry'] as { authoritativeSteps: number }).authoritativeSteps;
      viewer.socket.send(JSON.stringify({ type: 'reset' }));
      const reset = await replacementUntil(viewer, server.port, 'reset');
      expect(reset.sessionId).toBe(sessionId);
      expect(reset.runId).not.toBe(initial['runId']);
      const afterReset = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as {
        telemetry: { authoritativeSteps: number };
      };
      expect(afterReset.telemetry.authoritativeSteps).toBeGreaterThanOrEqual(stepsBeforeReset);
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
        rejoinToken: viewer.rejoinToken }));
      viewer.socket.send(JSON.stringify({ type: 'newRun', requestId: 'session-new-run' }));
      const newRun = await replacementUntil(viewer, server.port, 'newRun');
      expect(newRun.sessionId).toBe(sessionId);
      expect(newRun.runId).not.toBe(reset.runId);
      const afterNewRun = await healthUntil(server.port, health =>
        (health['telemetry'] as { authoritativeSteps: number }).authoritativeSteps > afterReset.telemetry.authoritativeSteps);
      expect((afterNewRun['telemetry'] as { authoritativeSteps: number }).authoritativeSteps).toBeGreaterThan(stepsBeforeReset);
      for (const peer of peers) peer.socket.terminate();
      peers.length = 0;
      await server.close();
      server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, resume: 'latest', rustCalculationWorkers: 1, dbPath });
      expect(server.startupFault).toBeUndefined();
      const restarted = await connect(server.port, 'ui'); peers.push(restarted);
      await until(restarted, () => restarted.packets.some(packet => packet['type'] === 'welcome'));
      const resumedWelcome = restarted.packets.find(packet => packet['type'] === 'welcome')!;
      expect(resumedWelcome['sessionId']).not.toBe(sessionId);
      expect(resumedWelcome['runId']).toBe(newRun.runId);
    } finally {
      for (const peer of peers) peer.socket.terminate();
      await server?.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 15_000);

  it('serves native welcome, frames, controller observations and same-snake token reclaim', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-server-'));
    const peers: Peer[] = [];
    const server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, resume: 'fresh', seed: 42,
      rustCalculationWorkers: PARALLEL_WORKERS, dbPath: join(root, 'experiment.sqlite') });
    try {
      const viewer = await connect(server.port, 'ui'); peers.push(viewer);
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
        rejoinToken: viewer.rejoinToken }));
      await until(viewer, () => viewer.frames > 0 && viewer.packets.some(packet => packet['type'] === 'stats'));
      expect(viewer.packets.find(packet => packet['type'] === 'welcome')).toMatchObject({ protocolVersion: 2, worldSeed: 42,
        sensorSpec: { layoutVersion: 'v4', profile: 'motion', sensorCount: 264 }, inferenceMode: { activeBackend: 'native', activeWorkerCount: ACTIVE_WORKERS } });
      viewer.socket.send(JSON.stringify({ type: 'viz', enabled: true }));
      await until(viewer, () => viewer.packets.some(packet => {
        const viz = packet['viz'] as { layers?: unknown[] } | undefined;
        return packet['type'] === 'stats' && viz?.layers?.length === 5;
      }));
      expect(viewer.packets.findLast(packet => packet['viz'])).toMatchObject({
        viz: {
          kind: 'graph', snakeId: expect.any(Number),
          layers: [
            { count: 264, activations: null },
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
        snakeId: selected.snakeId, selectionToken: (await selectPhysical(viewer, 'native-select', { snakeId: selected.snakeId })).token, x: selected.x * 0.9, y: selected.y * 0.9 }));
      await until(viewer, () => viewer.packets.some(packet => packet['requestId'] === 'native-god-move'));
      expect(viewer.packets.findLast(packet => packet['requestId'] === 'native-god-move')).toMatchObject({
        type: 'godModeResult', action: 'move', snakeId: selected.snakeId, applied: true,
        sequence: expect.any(Number), step: expect.any(Number), x: expect.any(Number), y: expect.any(Number)
      });
      viewer.socket.send(JSON.stringify({ type: 'godMode', requestId: 'native-god-kill', action: 'kill',
        snakeId: selected.snakeId, selectionToken: (viewer.packets.findLast(packet => packet['requestId'] === 'native-select')!['entity'] as SelectedEntity).token }));
      await until(viewer, () => viewer.packets.some(packet => packet['requestId'] === 'native-god-kill'));
      const killResult = viewer.packets.findLast(packet => packet['requestId'] === 'native-god-kill');
      expect(killResult).toMatchObject({
        type: 'godModeResult', action: 'kill', snakeId: selected.snakeId, applied: true,
        sequence: expect.any(Number), step: expect.any(Number), pelletsDropped: expect.any(Number)
      });
      expect(Number(killResult?.['pelletsDropped'])).toBeGreaterThan(0);
      await until(viewer, () => frameDirection(viewer.latestFrame, selected.snakeId) === undefined);
      viewer.socket.send(JSON.stringify({ type: 'godMode', requestId: 'native-god-missing', action: 'move',
        snakeId: selected.snakeId, selectionToken: (viewer.packets.findLast(packet => packet['requestId'] === 'native-select')!['entity'] as SelectedEntity).token, x: 0, y: 0 }));
      await until(viewer, () => viewer.packets.some(packet => packet['requestId'] === 'native-god-missing'));
      expect(viewer.packets.findLast(packet => packet['requestId'] === 'native-god-missing')).toMatchObject({
        type: 'godModeResult', action: 'move', snakeId: selected.snakeId, applied: false,
        reason: expect.stringContaining('no longer the current physical life')
      });
      const bot = await connect(server.port, 'bot'); peers.push(bot);
      bot.socket.send(JSON.stringify({ type: 'join', rejoinToken: bot.rejoinToken, mode: 'player', name: 'socket-bot' }));
      await until(bot, () => bot.packets.some(packet => packet['type'] === 'sensors'));
      const assignment = bot.packets.find(packet => packet['type'] === 'assign')!;
      const sample = bot.packets.find(packet => packet['type'] === 'sensors')!;
      expect(sample['snakeId']).toBe(assignment['snakeId']);
      expect(sample['sensors']).toHaveLength(264);
      bot.socket.send(JSON.stringify({ type: 'action', snakeId: assignment['snakeId'], tick: sample['tick'], turn: 0.4, boost: 0 }));
      await until(bot, () => bot.packets.some(packet => packet['type'] === 'sensors' && Number(packet['tick']) > Number(sample['tick'])));
      await new Promise<void>(done => { bot.socket.once('close', () => done()); bot.socket.close(); });
      const resumed = await connect(server.port, 'bot'); peers.push(resumed);
      resumed.socket.send(JSON.stringify({ type: 'join', rejoinToken: resumed.rejoinToken, mode: 'player', name: 'socket-bot', resumeToken: assignment['resumeToken'] }));
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
        calculationWorkers: PARALLEL_WORKERS,
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
        { path: 'sense.layoutVersion', value: 3 }, {path: 'baselineBots.algorithm', value: 1},
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
      expect(afterReset).toMatchObject({ seed: beforeReset.seed, calculationWorkers: PARALLEL_WORKERS });
      expect(afterReset.runId).not.toBe(beforeReset.runId);
      expect(afterReset.startupCheckpointId).not.toBe(beforeReset.startupCheckpointId);

      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
        rejoinToken: viewer.rejoinToken }));
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
        calculationWorkers: PARALLEL_WORKERS });
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

  it('keeps same-snake reclaim distinct from fresh joins after generation changes and failed successor sends', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-reclaim-generation-'));
    const server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, resume: 'fresh', seed: 91,
      rustCalculationWorkers: PARALLEL_WORKERS, dbPath: join(root, 'experiment.sqlite') });
    const peers: Peer[] = [];
    let sendSpy: ReturnType<typeof vi.spyOn> | undefined;
    try {
      const viewer = await connect(server.port, 'ui'); peers.push(viewer);
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
        rejoinToken: viewer.rejoinToken }));
      viewer.socket.send(JSON.stringify({ type: 'reset', settings: { snakeCount: 3, simSpeed: 0.1 },
        updates: [{ path: 'generationSeconds', value: 8 }, { path: 'pelletCountTarget', value: 100 },
          { path: 'baselineBots.count', value: 0 }],
        graphSpec: buildStackGraphSpec({ hiddenLayers: 1, neurons1: 2, neurons2: 2,
          neurons3: 2, neurons4: 2, neurons5: 2 },
        { brain: { inSize: 264, outSize: 2, useMlp: false } }) }));
      await until(viewer, () => viewer.packets.some(packet => packet['type'] === 'stateReplaced'));
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
        rejoinToken: viewer.rejoinToken }));
      /** Drive the exact measurement exchange through real result/assignment packets. */
      const claim = async (prior?: { snakeId: number; token: string }): Promise<{ peer: Peer; exchange: PlayerReconnectExchange }> => {
        const peer = await connect(server.port, 'ui'); peers.push(peer);
        const exchange = new PlayerReconnectExchange(prior?.snakeId, prior?.token);
        peer.socket.on('message', (data, binary) => {
          if (binary || exchange.record.failure) return;
          try {
            const message = JSON.parse(data.toString()) as Record<string, unknown>;
            if (exchange.consume(message)) peer.socket.send(JSON.stringify({
              type: 'join', mode: 'player', name: 'GenerationReclaimProbe' }));
          } catch { /* The exchange retains its failed invariant for the bounded assertion below. */ }
        });
        peer.socket.send(JSON.stringify({ type: 'join', rejoinToken: peer.rejoinToken, mode: 'player', name: 'GenerationReclaimProbe',
          ...(prior ? { resumeToken: prior.token } : {}) }));
        await until(peer, () => exchange.ready || exchange.record.failure !== undefined);
        expect(exchange.record.failure).toBeUndefined();
        expect(exchange.ready).toBe(true);
        return { peer, exchange };
      };
      const first = await claim();
      expect(first.exchange.record.outcome).toBe('fresh');
      await new Promise<void>(done => { first.peer.socket.once('close', done); first.peer.socket.close(); });
      const original = first.peer.packets.findLast(packet => packet['type'] === 'assign')!;
      const same = await claim({ snakeId: Number(original['snakeId']), token: String(original['resumeToken']) });
      expect(same.exchange.record.outcome).toBe('sameSnakeReclaim');
      await new Promise<void>(done => { same.peer.socket.once('close', done); same.peer.socket.close(); });
      const before = same.peer.packets.findLast(packet => packet['type'] === 'assign')!;
      const initial = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as { generation: string };
      viewer.socket.send(JSON.stringify({ type: 'settings', requestId: 'advance-reclaim-generation',
        updates: [{ path: 'simSpeed', value: 12 }] }));
      await until(viewer, () => viewer.packets.some(packet =>
        packet['type'] === 'settingsApplied' && packet['requestId'] === 'advance-reclaim-generation'));
      expect(viewer.packets.findLast(packet => packet['requestId'] === 'advance-reclaim-generation'))
        .toMatchObject({ applied: true });
      await healthUntil(server.port, value => BigInt(`0x${String(value['generation'])}`) > BigInt(`0x${initial.generation}`));
      const replacement = await claim({ snakeId: Number(before['snakeId']), token: String(before['resumeToken']) });
      expect(replacement.exchange.record.outcome).toBe('freshAfterRejectedReclaim');
      expect(replacement.exchange.record.packets[0]).toMatchObject({ type: 'reclaimResult', reclaimed: false });
      expect(replacement.exchange.record.packets.at(-1)?.snakeId).not.toBe(Number(before['snakeId']));
      expect(replacement.exchange.record.freshJoinRequested).toBe(true);

      viewer.socket.send(JSON.stringify({ type: 'settings', requestId: 'hold-successor-send',
        updates: [{ path: 'simSpeed', value: 0.1 }] }));
      await until(viewer, () => viewer.packets.some(packet => packet['requestId'] === 'hold-successor-send'));
      expect(viewer.packets.findLast(packet => packet['requestId'] === 'hold-successor-send'))
        .toMatchObject({ applied: true });
      const held = replacement.peer.packets.findLast(packet => packet['type'] === 'assign')!;
      const heldGeneration = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as { generation: string };
      const originalSend = WsHub.prototype.sendJsonTo;
      let failedSuccessorId: number | undefined;
      // Reject exactly the first new-snake send at the real transport boundary.
      // Rust must publish its failed-send outcome; the old token cannot bind it.
      sendSpy = vi.spyOn(WsHub.prototype, 'sendJsonTo').mockImplementation(function (this: WsHub, connection, message) {
        if (message.type === 'assign' && message.reclaimed !== true &&
            message.snakeId !== Number(held['snakeId']) && failedSuccessorId === undefined) {
          failedSuccessorId = message.snakeId;
          return false;
        }
        return originalSend.call(this, connection, message);
      });
      viewer.socket.send(JSON.stringify({ type: 'settings', requestId: 'advance-successor-send',
        updates: [{ path: 'simSpeed', value: 12 }] }));
      await healthUntil(server.port, value => failedSuccessorId !== undefined &&
        BigInt(`0x${String(value['generation'])}`) > BigInt(`0x${heldGeneration.generation}`));
      sendSpy.mockRestore(); sendSpy = undefined;
      await new Promise<void>(done => { replacement.peer.socket.once('close', done); replacement.peer.socket.close(); });
      const recovered = await claim({ snakeId: Number(held['snakeId']), token: String(held['resumeToken']) });
      expect(recovered.exchange.record.outcome).toBe('freshAfterRejectedReclaim');
      expect(recovered.exchange.record.packets[0]).toMatchObject({ type: 'reclaimResult', reclaimed: false });
      expect(recovered.exchange.record.requestedSnakeId).not.toBe(failedSuccessorId);
    } finally {
      sendSpy?.mockRestore();
      for (const peer of peers) peer.socket.terminate();
      await server.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 15_000);

  it('correlates client steering reversals with real Rust observations for player and bot leases', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-lan-turn-'));
    const server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, resume: 'fresh', seed: 83,
      dbPath: join(root, 'experiment.sqlite') });
    try {
      for (const kind of ['ui', 'bot'] as const) {
        const report = await measureTurnResponses(`ws://127.0.0.1:${server.port}`, kind, 4);
        expect(report.failure).toBeUndefined();
        expect(report.welcome?.inferenceMode.activeBackend).toBe('native');
        expect(report.trials).toHaveLength(4);
        expect(report.trials.map(trial => trial.requestedTurn)).toEqual([-1, 1, -1, 1]);
        for (const trial of report.trials) {
          expect(trial.failure).toBeUndefined();
          expect(trial.observedTick).toBeGreaterThan(trial.clientTick);
          expect(trial.latencyUpperBoundMs).toBeGreaterThanOrEqual(0);
          expect(trial.latencyUpperBoundMs).toBeLessThan(500);
        }
        // The 100 ms performance limit is evaluated in retained target-platform
        // reports; this integration gate verifies actual control/observation correlation.
      }
    } finally {
      await server.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 15_000);

  it('delivers fresh assignment and same-snake reclaim while display frames remain backpressured', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-frame-pressure-'));
    const peers: Peer[] = [];
    const sendErrors = vi.spyOn(console, 'error');
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
      first.socket.send(JSON.stringify({ type: 'join', rejoinToken: first.rejoinToken, mode: 'player', name: 'frame-pressure-player' }));
      await until(first, () => first.packets.some(packet => packet['type'] === 'assign'));
      const assignment = first.packets.find(packet => packet['type'] === 'assign')!;
      const initial = await healthUntil(server.port, value => {
        const outbound = value['outbound'] as WsOutboundDiagnostics;
        return outbound.replacedFrames >= 3 && outbound.pendingFrames === 1;
      });
      expect(first.frames).toBe(0);
      expect((initial['outbound'] as WsOutboundDiagnostics).reliableFailures).toBe(0);
      expect(first.packets.filter(packet => packet['type'] === 'assign')).toHaveLength(1);
      expect((initial['telemetry'] as { controllerActivity: { player: { freshAssignments: number } } })
        .controllerActivity.player.freshAssignments).toBe(1);
      await new Promise<void>(done => { first.socket.once('close', done); first.socket.close(); });
      await healthUntil(server.port, value => (value['outbound'] as WsOutboundDiagnostics).connections === 0);
      const resumed = await connect(server.port, 'ui'); peers.push(resumed);
      resumed.socket.send(JSON.stringify({ type: 'join', rejoinToken: resumed.rejoinToken, mode: 'player', name: 'frame-pressure-player',
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
      expect(measured).toMatchObject({ ok: true, outbound: { connections: 1, pendingFrames: 1 },
        telemetry: { controllerActivity: { player: { freshAssignments: 1, successfulReclaims: 1 } } } });
      const outbound = measured['outbound'] as WsOutboundDiagnostics;
      // A racing observation may fail on the deliberately closed first peer.
      // Preserve the lifetime count and reject any failure on the reclaimed peer.
      const reliableErrors = sendErrors.mock.calls.filter(([label]) => label === '[ws.reliable_send_failed]');
      expect(outbound.reliableFailures).toBe(reliableErrors.length);
      for (const [, details] of reliableErrors) {
        expect(details).toMatchObject({ connId: 1, reason: expect.stringMatching(/not open|closed/iu) });
      }
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
      sendErrors.mockRestore();
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
