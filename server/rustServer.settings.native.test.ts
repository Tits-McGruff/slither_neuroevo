import Database from 'better-sqlite3';
import { mkdtemp,readdir,rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { expect,it,vi } from 'vitest';
import { graphKey } from '../src/brains/graph/compiler.ts';
import { buildStackGraphSpec } from '../src/brains/stackBuilder.ts';
import { CFG_DEFAULT } from '../src/config.ts';
import { type WelcomeMsg } from '../src/net/wsClient.ts';
import { parseEvolvedLifecycleTelemetry } from '../src/protocol/evolvedLives.ts';
import { EVOLVED_SETTING_PATHS } from '../src/protocol/settingDefinitions.ts';
import type { SettingsUpdate } from '../src/protocol/settings.ts';
import { DEFAULT_CORE_SETTINGS } from '../src/protocol/settings.ts';
import { applyValuesToSlidersFromCFG,buildSettingsUI,collectSettingsUpdates } from '../src/settings.ts';
import { getByPath } from '../src/utils.ts';
import { CheckpointPersistenceClient } from './rustEngine/checkpointPersistenceClient.ts';
import { startRustServer } from './rustServer.ts';
import { connect,firstFrameSnake,healthUntil,manualFixtureHttp,replacementUntil,selectPhysical,until,type Peer } from './rustServerFixture.ts';
import { fixtureArchiveDownload } from './test/archiveDownload.ts';
import { describeNetworkSuite } from './test/networkSuites.ts';
import { RUST_TEST_CONFIG as DEFAULT_CONFIG } from './test/rustConfig.ts';

describeNetworkSuite('Rust server real sockets', () => {

  it('selects v4 profiles and baseline v2 through reset, New Run and durable resume, then returns to v3', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-v4-selection-'));
    const dbPath = join(root, 'experiment.sqlite');
    let server: Awaited<ReturnType<typeof startRustServer>> | undefined;
    let viewer: Peer | undefined;
    try {
      server = await startRustServer({...DEFAULT_CONFIG, port: 0, resume: 'fresh', seed: 42, rustCalculationWorkers: 1, dbPath});
      viewer = await connect(server.port, 'ui');
      await until(viewer, () => viewer!.packets.some(packet => packet['type'] === 'welcome'));
      const initial = viewer.packets.find(packet => packet['type'] === 'welcome') as unknown as WelcomeMsg;
      expect(initial.sensorSpec).toMatchObject({layoutVersion: 'v4', profile: 'motion', sensorCount: 264});
      expect(initial.settings.updates).toEqual(expect.arrayContaining([
        {path: 'baselineBots.algorithm', value: 2}, {path: 'baselineBots.behaviorMode', value: 1},
        {path: 'baselineBots.controlDt', value: 1 / 60}, {path: 'sense.maxPelletChecks', value: 3000},
        {path: 'sense.maxSegmentChecks', value: 2200}, {path: 'sense.rayMaximumDue', value: 512},
        {path: 'sense.bubbleBins', value: 16}
      ]));
      viewer.socket.send(JSON.stringify({type: 'join', mode: 'spectator'}));
      viewer.packets.length = 0;
      viewer.socket.send(JSON.stringify({type: 'reset', graphSpec: null, updates: [
        {path: 'sense.layoutVersion', value: 3}, {path: 'baselineBots.algorithm', value: 1}]}));
      expect((await replacementUntil(viewer, server.port, 'reset')).sensorSpec?.layoutVersion).toBe('v3');
      viewer.socket.send(JSON.stringify({type: 'join', mode: 'spectator', rejoinToken: viewer.rejoinToken}));
      viewer.packets.length = 0;
      viewer.socket.send(JSON.stringify({type: 'reset', graphSpec: null, updates: [{path: 'sense.layoutVersion', value: 4}]}));
      await until(viewer, () => viewer!.packets.some(packet => packet['type'] === 'error'));
      expect(viewer.packets.findLast(packet => packet['type'] === 'error')!['message']).toContain('explicit sense.profile');
      expect(viewer.packets.some(packet => packet['type'] === 'stateReplaced')).toBe(false);
      for (const [updates, error] of [
        [[{path: 'sense.profile', value: 0}], 'require v4'],
        [[{path: 'baselineBots.behaviorMode', value: 0}], 'require baseline v2'],
        [[{path: 'sense.layoutVersion', value: 4}, {path: 'sense.rayMinimum', value: 1.5}], 'within its declared bounds']
      ] as const) {
        viewer.packets.length = 0;
        viewer.socket.send(JSON.stringify({type: 'reset', graphSpec: null, updates}));
        await until(viewer, () => viewer!.packets.some(packet => packet['type'] === 'error'));
        expect(viewer.packets.findLast(packet => packet['type'] === 'error')!['message']).toContain(error);
        expect(viewer.packets.some(packet => packet['type'] === 'stateReplaced')).toBe(false);
      }
      const updates: SettingsUpdate[] = [
        {path: 'sense.layoutVersion', value: 4}, {path: 'sense.profile', value: 0}, {path: 'sense.bubbleBins', value: 8},
        {path: 'sense.rayMinimum', value: 1}, {path: 'sense.rayCap', value: 256}, {path: 'sense.rayBatch', value: 65536},
        {path: 'sense.rayMaximumDue', value: 200}, {path: 'baselineBots.algorithm', value: 2},
        {path: 'baselineBots.behaviorMode', value: 0}, {path: 'baselineBots.controlDt', value: 0.02},
        {path: 'baselineBots.count', value: 2}
      ];
      viewer.packets.length = 0;
      viewer.socket.send(JSON.stringify({type: 'reset', graphSpec: null, settings: {snakeCount: 3, simSpeed: 0.1},
        updates: updates.map(update => update.path === 'sense.rayMaximumDue' ? {...update, value: 1} : update)}));
      await until(viewer, () => viewer!.packets.some(packet => packet['type'] === 'error'));
      expect(viewer.packets.findLast(packet => packet['type'] === 'error')!['message']).toContain('must cover every configured snake');
      expect(viewer.packets.some(packet => packet['type'] === 'stateReplaced')).toBe(false);
      viewer.packets.length = 0;
      viewer.socket.send(JSON.stringify({type: 'reset', graphSpec: null, settings: {snakeCount: 3, simSpeed: 0.1}, updates}));
      const basic = await replacementUntil(viewer, server.port, 'reset');
      expect(basic.sensorSpec).toMatchObject({layoutVersion: 'v4', profile: 'basic', sensorCount: 72});
      expect(basic.settings.updates).toEqual(expect.arrayContaining(updates));
      expect(basic.worldSeed).toBe(initial.worldSeed);
      expect(basic.capabilities?.teacherCapture).toBe(true);
      // Observation routing must use the committed Basic width, not startup Motion/B16.
      const basicController = await connect(server.port, 'bot');
      try {
        basicController.socket.send(JSON.stringify({type: 'join', mode: 'player', name: 'basic-controller'}));
        await until(basicController, () => basicController.packets.some(packet => packet['type'] === 'sensors'));
        expect(basicController.packets.find(packet => packet['type'] === 'sensors')!['sensors']).toHaveLength(72);
        expect(basicController.packets.filter(packet => packet['type'] === 'error')).toEqual([]);
      } finally { basicController.socket.terminate(); }
      viewer.socket.send(JSON.stringify({type: 'join', mode: 'spectator', rejoinToken: viewer.rejoinToken}));
      viewer.packets.length = 0;
      viewer.socket.send(JSON.stringify({type: 'reset', graphSpec: null, updates: [{path: 'sense.profile', value: 1}]}));
      const motion = await replacementUntil(viewer, server.port, 'reset');
      expect(motion.sensorSpec).toMatchObject({layoutVersion: 'v4', profile: 'motion', sensorCount: 144});
      expect(motion.settings.updates).toEqual(expect.arrayContaining([{path: 'baselineBots.behaviorMode', value: 0}]));
      viewer.socket.send(JSON.stringify({type: 'join', mode: 'spectator', rejoinToken: viewer.rejoinToken}));
      viewer.packets.length = 0;
      viewer.socket.send(JSON.stringify({type: 'newRun', requestId: 'v4-new-run'}));
      const fresh = await replacementUntil(viewer, server.port, 'newRun');
      expect(fresh.settings).toEqual(motion.settings);
      expect(fresh.sensorSpec).toEqual(motion.sensorSpec);
      expect(fresh.runId).not.toBe(motion.runId);
      viewer.socket.terminate(); viewer = undefined; await server.close();
      server = await startRustServer({...DEFAULT_CONFIG, port: 0, resume: 'latest', rustCalculationWorkers: 1, dbPath});
      expect(server.startupFault).toBeUndefined();
      viewer = await connect(server.port, 'ui');
      await until(viewer, () => viewer!.packets.some(packet => packet['type'] === 'welcome'));
      const resumed = viewer.packets.find(packet => packet['type'] === 'welcome') as unknown as WelcomeMsg;
      expect(resumed.settings).toEqual(fresh.settings); expect(resumed.sensorSpec).toEqual(fresh.sensorSpec);
      const exported = await fixtureArchiveDownload(server.port, 'selected v4 settings');
      expect(exported.status).toBe(200);
      const archive = Buffer.from(await exported.arrayBuffer());
      const importedDbPath = join(root, 'imported-v4.sqlite');
      let importedServer: Awaited<ReturnType<typeof startRustServer>> | undefined;
      let importedViewer: Peer | undefined;
      try {
        importedServer = await startRustServer({...DEFAULT_CONFIG, port: 0, seed: 99, resume: 'fresh',
          rustCalculationWorkers: 1, dbPath: importedDbPath});
        const imported = await manualFixtureHttp(`http://127.0.0.1:${importedServer.port}/api/import/archive`, archive);
        expect(imported.status, imported.bytes.toString()).toBe(200);
        expect(JSON.parse(imported.bytes.toString())).toMatchObject({ok: true, runId: fresh.runId});
        await importedServer.close(); importedServer = undefined;
        importedServer = await startRustServer({...DEFAULT_CONFIG, port: 0, resume: 'latest',
          rustCalculationWorkers: 1, dbPath: importedDbPath});
        expect(importedServer.startupFault).toBeUndefined();
        importedViewer = await connect(importedServer.port, 'ui');
        await until(importedViewer, () => importedViewer!.packets.some(packet => packet['type'] === 'welcome'));
        const restored = importedViewer.packets.find(packet => packet['type'] === 'welcome') as unknown as WelcomeMsg;
        expect(restored.runId).toBe(fresh.runId);
        expect(restored.settings).toEqual(fresh.settings);
        expect(restored.sensorSpec).toEqual(fresh.sensorSpec);
        expect(restored.graphSpec).toEqual(fresh.graphSpec);
        expect(restored.capabilities?.teacherCapture).toBe(true);
        await healthUntil(importedServer.port, health => health['lifecycle'] === 'running' &&
          typeof health['completedStep'] === 'string' && BigInt(`0x${health['completedStep']}`) > 0n);
      } finally { importedViewer?.socket.terminate(); await importedServer?.close(); }
      viewer.socket.send(JSON.stringify({type: 'join', mode: 'spectator', rejoinToken: viewer.rejoinToken}));
      viewer.packets.length = 0;
      viewer.socket.send(JSON.stringify({type: 'reset', graphSpec: null, updates: [{path: 'sense.layoutVersion', value: 3}]}));
      await until(viewer, () => viewer!.packets.some(packet => packet['type'] === 'error'));
      expect(viewer.packets.findLast(packet => packet['type'] === 'error')!['message']).toContain('requires v4');
      viewer.packets.length = 0;
      viewer.socket.send(JSON.stringify({type: 'reset', graphSpec: null, updates: [
        {path: 'sense.layoutVersion', value: 3}, {path: 'baselineBots.algorithm', value: 1}]}));
      const frozen = await replacementUntil(viewer, server.port, 'reset');
      expect(frozen.sensorSpec).toMatchObject({layoutVersion: 'v3', sensorCount: 51});
      expect(frozen.capabilities?.teacherCapture).toBe(false);
      expect(frozen.settings.updates).toContainEqual({path: 'baselineBots.algorithm', value: 1});
      expect(frozen.settings.updates.some(update => update.path === 'sense.profile' || update.path.startsWith('sense.ray') ||
        update.path === 'baselineBots.behaviorMode' || update.path === 'baselineBots.controlDt')).toBe(false);
    } finally { viewer?.socket.terminate(); await server?.close(); await rm(root, {recursive: true, force: true}); }
  }, 30_000);

  it('preserves legacy resets and explicitly selects complete life experiments through real respawn and New Run', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-life-contract-'));
    let server: Awaited<ReturnType<typeof startRustServer>> | undefined;
    let viewer: Peer | undefined;
    try {
      server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, dbPath: join(root, 'experiment.sqlite'),
        resume: 'fresh', seed: 42, rustCalculationWorkers: 1 });
      expect(server.startupFault).toBeUndefined(); viewer = await connect(server.port, 'ui');

      await until(viewer, () => viewer!.packets.some(packet => packet['type'] === 'welcome'));
      const fresh = viewer.packets.find(packet => packet['type'] === 'welcome') as unknown as WelcomeMsg;
      expect(fresh.settings.updates).toEqual(expect.arrayContaining([{ path: 'evolvedRespawn.contractVersion', value: 1 },
        { path: 'evolvedRespawn.enabled', value: 0 }, { path: 'world.pelletOverflow', value: 0 }]));
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator' }));

      await until(viewer, () => viewer!.frames > 0);
      /** Rejoin the replacement world before subsequent commands. */
      const rejoin = async (): Promise<void> => {
        const frames = viewer!.frames;
        viewer!.socket.send(JSON.stringify({ type: 'join', mode: 'spectator', rejoinToken: viewer!.rejoinToken }));
        await until(viewer!, () => viewer!.frames > frames);
      };
      const legacy = Buffer.from(JSON.stringify({ generation: 37, archKey: 'legacy-default-graph', worldSeed: 1234567,
        settings: { snakeCount: 2, simSpeed: 0.1 }, genomes: [1, 2].map(fitness => ({ archKey: 'legacy-default-graph',
          brainType: 'mlp', fitness, weights: new Array<number>(13458).fill(0) })) }));

      const imported = await manualFixtureHttp('http://127.0.0.1:' + server.port + '/api/import/archive', legacy);
      expect(imported.status, imported.bytes.toString()).toBe(200);

      await until(viewer, () => viewer!.packets.some(packet => packet['type'] === 'stateReplaced' && packet['reason'] === 'import'));
      const old = viewer.packets.findLast(packet => packet['type'] === 'stateReplaced')!['welcome'] as WelcomeMsg;
      expect(old.settings.updates.some(update => EVOLVED_SETTING_PATHS.has(update.path))).toBe(false);

      await rejoin();
      viewer.packets.length = 0;
      viewer.socket.send(JSON.stringify({ type: 'reset', settings: old.settings.core, updates: [] }));

      await until(viewer, () => viewer!.packets.some(packet => packet['type'] === 'stateReplaced' && packet['reason'] === 'reset'));
      const resetOld = viewer.packets.findLast(packet => packet['type'] === 'stateReplaced')!['welcome'] as WelcomeMsg;
      expect(resetOld.settings.updates.some(update => EVOLVED_SETTING_PATHS.has(update.path))).toBe(false);
      await rejoin();
      viewer.packets.length = 0;
      viewer.socket.send(JSON.stringify({ type: 'reset', updates: [{ path: 'evolvedRespawn.enabled', value: 1 }] }));

      await until(viewer, () => viewer!.packets.some(packet => packet['type'] === 'error'));
      expect(viewer.packets.find(packet => packet['type'] === 'error')!['message']).toContain('complete version-one');
      expect(viewer.packets.some(packet => packet['type'] === 'stateReplaced')).toBe(false);
      viewer.packets.length = 0;
      const updates = [...EVOLVED_SETTING_PATHS].map(path => ({ path, value: Number(getByPath(CFG_DEFAULT, path)) }));
      for (const update of updates) {
        if (update.path === 'evolvedRespawn.enabled') update.value = 1;
        if (update.path === 'evolvedRespawn.maxLives') update.value = 2;
        if (update.path === 'evolvedRespawn.recurrentState' || update.path === 'evolvedRespawn.corpseMode') update.value = 1;
      }
      viewer.socket.send(JSON.stringify({ type: 'reset', settings: { ...resetOld.settings.core, simSpeed: 1 },
        updates: [...updates, { path: 'baselineBots.count', value: 0 }, { path: 'observer.earlyEndMinSeconds', value: 50 }] }));

      await until(viewer, () => viewer!.packets.some(packet => packet['type'] === 'stateReplaced' && packet['reason'] === 'reset'));
      const experiment = viewer.packets.findLast(packet => packet['type'] === 'stateReplaced')!['welcome'] as WelcomeMsg;
      expect(experiment.settings.updates.filter(update => EVOLVED_SETTING_PATHS.has(update.path))).toHaveLength(17);
      expect(experiment.settings.updates).toEqual(expect.arrayContaining(updates));
      await rejoin();
      const selected = firstFrameSnake(viewer.latestFrame)!;
      expect(selected.snakeId).toBe(1);
      const lifeSelection = await selectPhysical(viewer, 'life-select', { snakeId: selected.snakeId });
      viewer.socket.send(JSON.stringify({ type: 'godMode', action: 'kill', requestId: 'life-kill', snakeId: selected.snakeId, selectionToken: lifeSelection.token }));

      await until(viewer, () => viewer!.packets.some(packet => packet['requestId'] === 'life-kill'));
      expect(viewer.packets.find(packet => packet['requestId'] === 'life-kill')).toMatchObject({ applied: true, pelletsDropped: 0 });

      await until(viewer, () => (firstFrameSnake(viewer!.latestFrame)?.snakeId ?? 0) > 2);
      expect(firstFrameSnake(viewer.latestFrame)!.snakeId).toBe(3);
      const nextLife = await selectPhysical(viewer, 'life-follow', { populationSlot: lifeSelection.populationSlot! });
      expect(nextLife).toMatchObject({ snakeId: 3, lifeOrdinal: '0000000000000002' });
      expect(nextLife.token).not.toBe(lifeSelection.token);
      await until(viewer, () => viewer!.packets.some(packet => packet['type'] === 'stats' && parseEvolvedLifecycleTelemetry(packet['evolved'])?.respawns === '0000000000000001'));
      const lifeStats = viewer.packets.findLast(packet => packet['type'] === 'stats' && parseEvolvedLifecycleTelemetry(packet['evolved'])?.respawns === '0000000000000001')!;
      const telemetry = parseEvolvedLifecycleTelemetry(lifeStats['evolved'])!;
      expect(telemetry).toMatchObject({ version: 1, population: 2, alive: 2, lives: '0000000000000003', deaths: '0000000000000001', respawns: '0000000000000001', lifeEndCursor: '0000000000000001', recentEventCount: 1 });
      expect(BigInt('0x' + telemetry.placement.step)).toBe(BigInt(lifeStats['tick'] as number));
      expect(telemetry.placement.candidates).toBeLessThanOrEqual(2048);
      expect(telemetry.placement.geometryChecks).toBeLessThanOrEqual(250000);
      viewer.socket.send(JSON.stringify({ type: 'godMode', action: 'kill', requestId: 'life-stale-kill',
        snakeId: nextLife.snakeId, selectionToken: lifeSelection.token }));
      await until(viewer, () => viewer!.packets.some(packet => packet['requestId'] === 'life-stale-kill'));
      expect(viewer.packets.find(packet => packet['requestId'] === 'life-stale-kill')).toMatchObject({ applied: false });
      viewer.packets.length = 0; viewer.socket.send(JSON.stringify({ type: 'newRun', requestId: 'life-new-run' }));

      await until(viewer, () => viewer!.packets.some(packet => packet['type'] === 'stateReplaced' && packet['reason'] === 'newRun'));
      const newRun = viewer.packets.findLast(packet => packet['type'] === 'stateReplaced')!['welcome'] as WelcomeMsg;
      expect(newRun.runId).not.toBe(experiment.runId);
      expect(newRun.settings.updates).toEqual(experiment.settings.updates);
      await rejoin();
      const reused = firstFrameSnake(viewer.latestFrame)!;
      expect(reused.snakeId).toBe(lifeSelection.snakeId);
      for (const [requestId, token] of [['life-no-token', undefined], ['life-old-world', lifeSelection.token]] as const) {
        viewer.socket.send(JSON.stringify({ type: 'godMode', action: 'kill', requestId,
          snakeId: reused.snakeId, selectionToken: token }));
        await until(viewer, () => viewer!.packets.some(packet => packet['requestId'] === requestId));
        expect(viewer.packets.find(packet => packet['requestId'] === requestId)).toMatchObject({ applied: false });
      }
      const newSelection = await selectPhysical(viewer, 'life-new-world-select', { snakeId: reused.snakeId });
      expect(newSelection.token).not.toBe(lifeSelection.token);
    } finally { viewer?.socket.terminate(); await server?.close(); await rm(root, { recursive: true, force: true }); }
  }, 30_000);

  it('clears custom graphs and preserves truthful stack controls through reset, reconnect and resume', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-stack-reset-'));
    const dbPath = join(root, 'experiment.sqlite');
    let server: Awaited<ReturnType<typeof startRustServer>> | undefined;
    const peers: Peer[] = [];
    const originalRetention = CheckpointPersistenceClient.prototype.inspectRetention;
    const retention = vi.spyOn(CheckpointPersistenceClient.prototype, 'inspectRetention').mockImplementation(async function(
      this: CheckpointPersistenceClient
    ) {
      const result = await originalRetention.call(this);
      // Keep publication visibly ahead of completion, reproducing the loaded-runner ordering.
      await new Promise<void>(done => setTimeout(done, 40));
      return result;
    });
    try {
      server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, resume: 'fresh', seed: 42,
        rustCalculationWorkers: 1, dbPath });
      const viewer = await connect(server.port, 'ui'); peers.push(viewer);
      await until(viewer, () => viewer.packets.some(packet => packet['type'] === 'welcome'));
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
        rejoinToken: viewer.rejoinToken }));
      await until(viewer, () => viewer.frames > 0);
      const custom = buildStackGraphSpec(DEFAULT_CORE_SETTINGS,
        { brain: { ...CFG_DEFAULT.brain, useMlp: false, stack: { gru: 0, lstm: 0, rru: 0 } } });
      custom.nodes[1]!.id = 'custom-output';
      custom.edges[0]!.to = 'custom-output';
      custom.outputs[0]!.nodeId = 'custom-output';
      viewer.socket.send(JSON.stringify({ type: 'reset', graphSpec: custom,
        settings: { snakeCount: 3, simSpeed: 0.1 }, updates: [{ path: 'baselineBots.count', value: 0 }] }));
      await replacementUntil(viewer, server.port, 'reset');
      expect(viewer.packets.findLast(packet => packet['type'] === 'stateReplaced')).toMatchObject({ welcome: { graphSpec: custom } });
      viewer.packets.length = 0;
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
        rejoinToken: viewer.rejoinToken }));
      viewer.socket.send(JSON.stringify({ type: 'reset' }));
      await replacementUntil(viewer, server.port, 'reset');
      expect(viewer.packets.findLast(packet => packet['type'] === 'stateReplaced')).toMatchObject({ welcome: { graphSpec: custom } });

      const core = { ...DEFAULT_CORE_SETTINGS, snakeCount: 3, simSpeed: 0.1, hiddenLayers: 3,
        neurons1: 23, neurons2: 17, neurons3: 11 };
      const brain = { ...CFG_DEFAULT.brain, inSize: 144, gruHidden: 12, lstmHidden: 20, rruHidden: 24,
        stack: { gru: 1, lstm: 1, rru: 1 } };
      const expectedGraph = buildStackGraphSpec(core, { brain });
      const updates = [
        { path: 'sense.bubbleBins', value: 8 }, { path: 'brain.useMlp', value: 1 },
        { path: 'brain.stack.gru', value: 1 }, { path: 'brain.stack.lstm', value: 1 }, { path: 'brain.stack.rru', value: 1 },
        { path: 'brain.gruHidden', value: 12 }, { path: 'brain.lstmHidden', value: 20 }, { path: 'brain.rruHidden', value: 24 }
      ];
      viewer.packets.length = 0;
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
        rejoinToken: viewer.rejoinToken }));
      viewer.socket.send(JSON.stringify({ type: 'reset', graphSpec: null, settings: core, updates }));
      const welcome = await replacementUntil(viewer, server.port, 'reset');
      expect(welcome).toMatchObject({ graphSpec: expectedGraph, settings: {
        core: { hiddenLayers: 3, neurons1: 23, neurons2: 17, neurons3: 11 }, updates: expect.arrayContaining(updates)
      } });
      expect(graphKey(welcome.graphSpec!)).not.toBe(graphKey(custom));

      // Omitted graph plus the browser's unchanged controls remains a valid reset.
      viewer.packets.length = 0;
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
        rejoinToken: viewer.rejoinToken }));
      viewer.socket.send(JSON.stringify({ type: 'reset', settings: welcome.settings.core, updates }));
      await replacementUntil(viewer, server.port, 'reset');
      expect(viewer.packets.findLast(packet => packet['type'] === 'stateReplaced')).toMatchObject({ welcome: { graphSpec: expectedGraph } });

      const reconnect = await connect(server.port, 'ui'); peers.push(reconnect);
      await until(reconnect, () => reconnect.packets.some(packet => packet['type'] === 'welcome'));
      expect(reconnect.packets.find(packet => packet['type'] === 'welcome')).toMatchObject({ graphSpec: expectedGraph,
        settings: welcome.settings });
      for (const peer of peers) peer.socket.terminate();
      peers.length = 0;
      await server.close();
      server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, resume: 'latest', rustCalculationWorkers: 1, dbPath });
      expect(server.startupFault).toBeUndefined();
      const resumed = await connect(server.port, 'ui'); peers.push(resumed);
      await until(resumed, () => resumed.packets.some(packet => packet['type'] === 'welcome'));
      expect(resumed.packets.find(packet => packet['type'] === 'welcome')).toMatchObject({ graphSpec: expectedGraph,
        settings: welcome.settings });
    } finally {
      retention.mockRestore();
      for (const peer of peers) peer.socket.terminate();
      await server?.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 20_000);

  it('rejects an oversized population without replacing the run and accepts a smaller retry', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-reset-size-'));
    let server: Awaited<ReturnType<typeof startRustServer>> | undefined;
    let viewer: Peer | undefined;
    try {
      server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, resume: 'fresh', seed: 42,
        rustCalculationWorkers: 1, dbPath: join(root, 'experiment.sqlite') });
      viewer = await connect(server.port, 'ui');
      await until(viewer, () => viewer!.packets.some(packet => packet['type'] === 'welcome'));
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator', rejoinToken: viewer.rejoinToken }));
      await until(viewer, () => viewer!.frames > 0);
      const before = await healthUntil(server.port, health => health['lifecycle'] === 'running');
      const files = (await readdir(`${join(root, 'experiment.sqlite')}.checkpoints`)).sort();
      const graphSpec = { type: 'graph', nodes: [
        { id: 'input', type: 'Input', outputSize: 83 },
        { id: 'large', type: 'MLP', inputSize: 83, hiddenSizes: [512, 512], outputSize: 512 },
        { id: 'head', type: 'Dense', inputSize: 512, outputSize: 2 }
      ], edges: [{ from: 'input', to: 'large' }, { from: 'large', to: 'head' }],
      outputs: [{ nodeId: 'head' }], outputSize: 2 };
      viewer.packets.length = 0;
      viewer.socket.send(JSON.stringify({ type: 'reset', graphSpec, settings: { snakeCount: 300 },
        updates: [{ path: 'baselineBots.count', value: 0 }, {path: 'sense.layoutVersion', value: 3},
          {path: 'baselineBots.algorithm', value: 1}] }));
      await until(viewer, () => viewer!.packets.some(packet => packet['type'] === 'error'));
      expect(viewer.packets.find(packet => packet['type'] === 'error')).toMatchObject({ message:
        expect.stringMatching(/reset failed:.*569346 parameters each.*512\.00 MiB.*Reduce NPC snakes or brain size/) });
      const after = await healthUntil(server.port, health =>
        BigInt(`0x${health['completedStep'] as string}`) > BigInt(`0x${before['completedStep'] as string}`));
      expect(after).toMatchObject({ ok: true, lifecycle: 'running', runId: before['runId'],
        startupCheckpointId: before['startupCheckpointId'] });
      expect(viewer.packets.some(packet => packet['type'] === 'stateReplaced')).toBe(false);
      expect((await readdir(`${join(root, 'experiment.sqlite')}.checkpoints`)).sort()).toEqual(files);
      viewer.packets.length = 0;
      viewer.socket.send(JSON.stringify({ type: 'reset', settings: { snakeCount: 3, simSpeed: 0.1 },
        updates: [{ path: 'baselineBots.count', value: 0 }] }));
      const replaced = await replacementUntil(viewer, server.port, 'reset');
      expect(replaced.runId).not.toBe(before['runId']);
      expect(replaced.settings.core.snakeCount).toBe(3);
      expect(viewer.packets.some(packet => packet['type'] === 'error')).toBe(false);
    } finally {
      viewer?.socket.terminate();
      await server?.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 20_000);

  it('admits 300 complete long initial bodies through the normal reset boundary', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-long-start-'));
    let server: Awaited<ReturnType<typeof startRustServer>> | undefined;
    let viewer: Peer | undefined;
    try {
      server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, resume: 'fresh',
        seed: 1511506142, dbPath: join(root, 'experiment.sqlite') });
      viewer = await connect(server.port, 'ui');
      await until(viewer, () => viewer!.packets.some(packet => packet['type'] === 'welcome'));
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
        rejoinToken: viewer.rejoinToken }));
      await until(viewer, () => viewer!.frames > 0);
      const graphSpec = { type: 'graph',
        nodes: [{ id: 'input', type: 'Input', outputSize: 264 },
          { id: 'output', type: 'Dense', inputSize: 264, outputSize: 2 }],
        edges: [{ from: 'input', to: 'output' }], outputs: [{ nodeId: 'output' }], outputSize: 2 };
      viewer.socket.send(JSON.stringify({ type: 'reset', graphSpec,
        settings: { snakeCount: 300, simSpeed: 0.1 }, updates: [
          { path: 'worldRadius', value: 10000 },
          { path: 'snakeStartLen', value: 140 },
          { path: 'snakeSpacing', value: 3 },
          // This checks admission geometry; limit movement before a loaded runner observes it.
          { path: 'snakeBaseSpeed', value: 30 },
          { path: 'snakeBoostSpeed', value: 40 },
          { path: 'snakeRadius', value: 3 },
          { path: 'snakeThicknessScale', value: 0 },
          { path: 'collision.skipSegments', value: 30 },
          { path: 'baselineBots.count', value: 0 }
        ] }));
      await until(viewer, () => viewer!.packets.some(packet =>
        packet['type'] === 'stateReplaced' && packet['reason'] === 'reset'));
      delete viewer.latestFrame;
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
        rejoinToken: viewer.rejoinToken }));
      await until(viewer, () => viewer!.latestFrame?.readFloatLE(4) === 300);
      const bytes = viewer.latestFrame!;
      expect(bytes.readFloatLE(0)).toBe(1);
      expect(bytes.readFloatLE(8)).toBe(300);
      expect(bytes.readFloatLE(12)).toBe(10000);
      let offset = 7;
      let bodyPoints = 0;
      for (let snake = 0; snake < 300; snake++) {
        const points = bytes.readFloatLE((offset + 7) * 4);
        expect(Number.isSafeInteger(points)).toBe(true);
        expect(points).toBeGreaterThanOrEqual(140);
        for (let point = 0; point < points; point++) {
          const x = bytes.readFloatLE((offset + 8 + point * 2) * 4);
          const y = bytes.readFloatLE((offset + 9 + point * 2) * 4);
          expect(x * x + y * y).toBeLessThan(10000 ** 2);
        }
        bodyPoints += points;
        offset += 8 + points * 2;
      }
      expect(bodyPoints).toBeGreaterThanOrEqual(42000);
      expect(await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json())
        .toMatchObject({ ok: true, lifecycle: 'running' });
    } finally {
      viewer?.socket.terminate();
      await server?.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 20_000);

  it('durably resets Rust with every authoritative input generated by the browser settings panel', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-panel-reset-'));
    const server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, resume: 'fresh', seed: 42,
      dbPath: join(root, 'experiment.sqlite') });
    let viewer: Peer | undefined;
    try {
      viewer = await connect(server.port, 'ui');
      await until(viewer, () => viewer!.packets.some(packet => packet['type'] === 'welcome'));
      const original = viewer.packets.find(packet => packet['type'] === 'welcome') as unknown as WelcomeMsg;
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator' }));
      /** Capture real generated controls with only the DOM methods used by the panel builder. */
      const inputs: HTMLInputElement[] = [];
      vi.stubGlobal('document', {
        createElement() {
          const options: HTMLElement[] = [];
          const element = { dataset: {}, value: '', checked: false, options,
            addEventListener() {},
            appendChild(child: HTMLElement) { options.push(child); } } as unknown as HTMLInputElement;
          inputs.push(element);
          return element;
        },
        getElementById() { return null; }
      });
      let updates: SettingsUpdate[];
      try {
        const panel = { appendChild() {}, querySelectorAll: () => inputs.filter(input => input.dataset['path']) } as unknown as HTMLElement;
        buildSettingsUI(panel);
        applyValuesToSlidersFromCFG(panel);
        updates = collectSettingsUpdates(panel);
      } finally { vi.unstubAllGlobals(); }
      expect(updates.length).toBeGreaterThan(60);
      const radius = updates.find(update => update.path === 'worldRadius')!;
      radius.value = 4200;
      const graphSpec = buildStackGraphSpec({ ...DEFAULT_CORE_SETTINGS, hiddenLayers: 1, neurons1: 8 }, CFG_DEFAULT);
      viewer.socket.send(JSON.stringify({ type: 'reset',
        settings: { ...DEFAULT_CORE_SETTINGS, snakeCount: 12, hiddenLayers: 1, neurons1: 8 }, updates, graphSpec }));
      const replaced = await replacementUntil(viewer, server.port, 'reset');
      expect(replaced.worldSeed).toBe(original.worldSeed);
      expect(replaced.runId).not.toBe(original.runId);
      expect(replaced.settings).toMatchObject({ core: { snakeCount: 12 },
        updates: expect.arrayContaining([{ path: 'worldRadius', value: 4200 }]) });
      const db = new Database(join(root, 'experiment.sqlite'), { readonly: true });
      try {
        expect(db.prepare('SELECT checkpoint_id FROM rust_checkpoint_v3_current WHERE run_id = ?')
          .get(replaced.runId)).toMatchObject({ checkpoint_id: expect.stringMatching(/^[0-9a-f]{64}$/u) });
      } finally { db.close(); }
      expect(viewer.packets.filter(packet => packet['type'] === 'error')).toEqual([]);
    } finally {
      viewer?.socket.terminate();
      await server.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 15_000);
});
