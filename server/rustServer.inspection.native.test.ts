import { mkdtemp,rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { expect,it,vi } from 'vitest';
import WebSocket from 'ws';
import type { ExternalRewardEnvelopeV1 } from '../contracts/types.ts';
import { buildStackGraphSpec } from '../src/brains/stackBuilder.ts';
import { PlayerActionPump } from '../src/net/playerActionPump.ts';
import { createWsClient,type AssignMsg,type SensorsMsg,type WelcomeMsg,type WsClient } from '../src/net/wsClient.ts';
import { parseInspectorSnapshot,type InspectEntityMsg } from '../src/protocol/inspector.ts';
import { startRustServer } from './rustServer.ts';
import { cleanupInspectorFixture,cleanupRewardFixture,connect,directionDelta,frameDirection,replacementUntil,rewardFixturePellet,selectPhysical,until,type Peer } from './rustServerFixture.ts';
import { describeNetworkSuite } from './test/networkSuites.ts';
import { RUST_TEST_CONFIG as DEFAULT_CONFIG } from './test/rustConfig.ts';

describeNetworkSuite('Rust server real sockets', () => {
  it('routes authoritative food and complete reward closures over negotiated binary sensors', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-reward-'));
    const peers: Peer[] = [];
    let server: Awaited<ReturnType<typeof startRustServer>> | undefined;
    try {
      server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, resume: 'fresh', seed: 42,
        dbPath: join(root, 'reward.sqlite'), rustCalculationWorkers: 1 });
      expect(server.startupFault).toBeUndefined();
      const viewer = await connect(server.port, 'ui'); peers.push(viewer);
      await until(viewer, () => viewer.packets.some(packet => packet['type'] === 'welcome'));
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator' }));
      viewer.socket.send(JSON.stringify({ type: 'reset', settings: { snakeCount: 1, simSpeed: 0.1 },
        updates: [{ path: 'baselineBots.count', value: 0 }, { path: 'pelletCountTarget', value: 100 },
          { path: 'growPerFood', value: 1 }, { path: 'generationSeconds', value: 240 }],
        graphSpec: buildStackGraphSpec({ hiddenLayers: 1, neurons1: 2, neurons2: 2,
          neurons3: 2, neurons4: 2, neurons5: 2 },
        { brain: { inSize: 264, outSize: 2, useMlp: false } }) }));
      const welcome = await replacementUntil(viewer, server.port, 'reset');
      expect(welcome).toMatchObject({ rewardVersion: 1, binarySensorsVersion: 2,
        sensorSpec: { layoutVersion: 'v4', profile: 'motion', sensorCount: 264 } });
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator', rejoinToken: viewer.rejoinToken }));
      const bot = await connect(server.port, 'bot', undefined, true); peers.push(bot);
      await until(bot, () => bot.packets.some(packet => packet['type'] === 'welcome'));
      bot.socket.send(JSON.stringify({ type: 'join', mode: 'player', name: 'reward-wire-fixture' }));
      const rewards = (): ExternalRewardEnvelopeV1[] => bot.packets
        .filter(packet => packet['type'] === 'sensors' || packet['type'] === 'transition')
        .map(packet => packet['reward'] as ExternalRewardEnvelopeV1);
      await until(bot, () => rewards().some(reward => reward.transitionKind === 'initial'));
      const initial = rewards()[0]!;
      const actionId = '00000000000000ac';
      bot.socket.send(JSON.stringify({ type: 'action', snakeId: initial.snakeId,
        tick: Number(BigInt(`0x${initial.toStep}`)), turn: 0, boost: 0, actionId }));
      await until(bot, () => rewards().some(reward =>
        reward.control.appliedCommand?.command.clientActionId === actionId));
      const confirmed = rewards().find(reward =>
        reward.control.appliedCommand?.command.clientActionId === actionId)!;
      expect(confirmed.control).toMatchObject({
        acceptedCount: 1, appliedCount: 1, supersededCount: 0,
        acceptedCommand: confirmed.control.appliedCommand!.command,
      });
      expect(confirmed.control.appliedCommand!.firstAppliedStep < confirmed.toStep).toBe(true);
      await until(viewer, () => viewer.latestFrame !== undefined);
      const pellet = rewardFixturePellet(viewer.latestFrame!, initial.snakeId);
      const selected = await selectPhysical(viewer, 'reward-move-selection', { snakeId: initial.snakeId });
      viewer.socket.send(JSON.stringify({ type: 'godMode', requestId: 'reward-food-move', action: 'move',
        snakeId: initial.snakeId, selectionToken: selected.token, ...pellet }));
      await until(viewer, () => viewer.packets.some(packet => packet['requestId'] === 'reward-food-move' && packet['applied'] === true));
      await until(bot, () => rewards().some(reward => reward.leaseId === initial.leaseId && reward.deltas.foodCountDelta > 0));
      const eaten = rewards().filter(reward => reward.leaseId === initial.leaseId);
      expect(eaten.reduce((sum, reward) => sum + reward.deltas.foodValueDelta, 0)).toBeGreaterThan(0);
      expect(eaten.reduce((sum, reward) => sum + reward.deltas.retainedGrowthDelta, 0)).toBeGreaterThan(0);
      expect(eaten.reduce((sum, reward) => sum + reward.deltas.foodPointsDelta, 0)).toBeGreaterThan(0);

      const kill = await selectPhysical(viewer, 'reward-kill-selection', { snakeId: initial.snakeId });
      viewer.socket.send(JSON.stringify({ type: 'godMode', requestId: 'reward-real-death', action: 'kill',
        snakeId: initial.snakeId, selectionToken: kill.token }));
      await until(bot, () => rewards().some(reward => reward.leaseId === initial.leaseId && reward.transitionKind === 'terminal'));
      await until(bot, () => rewards().some(reward => reward.leaseId !== initial.leaseId && reward.transitionKind === 'initial'));
      const next = rewards().find(reward => reward.leaseId !== initial.leaseId && reward.transitionKind === 'initial')!;
      const terminalIndex = bot.packets.findIndex(packet => packet['type'] === 'transition' &&
        (packet['reward'] as ExternalRewardEnvelopeV1).leaseId === initial.leaseId);
      expect(bot.packets[terminalIndex]).toMatchObject({ reward: { transitionKind: 'terminal', reason: 'death' } });
      expect(bot.packets[terminalIndex]?.['finalSensors']).toBeUndefined();
      expect(bot.packets.findIndex((packet, index) => index > terminalIndex && packet['type'] === 'assign')).toBeGreaterThan(terminalIndex);

      const health = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as { generation: string };
      viewer.socket.send(JSON.stringify({ type: 'finishGeneration', requestId: 'reward-manual-cutoff',
        expectedRunId: welcome.runId, expectedGeneration: Number(BigInt(`0x${health.generation}`)) }));
      await until(bot, () => rewards().some(reward => reward.leaseId === next.leaseId && reward.transitionKind === 'truncated'));
      const truncationIndex = bot.packets.findIndex(packet => packet['type'] === 'transition' &&
        (packet['reward'] as ExternalRewardEnvelopeV1).leaseId === next.leaseId);
      const cutoff = bot.packets[truncationIndex]!;
      expect(cutoff).toMatchObject({ reward: { transitionKind: 'truncated', reason: 'manualGeneration' } });
      expect(cutoff['finalSensors']).toHaveLength(264);
      expect((cutoff['finalSensors'] as number[]).every(Number.isFinite)).toBe(true);
      await until(bot, () => bot.packets.some((packet, index) => index > truncationIndex && packet['type'] === 'assign'));

      const previous = new Map<string, ExternalRewardEnvelopeV1>();
      for (const reward of rewards()) {
        expect(reward.runId).toBe(welcome.runId);
        expect(reward.rewardVersion).toBe(1);
        expect(reward.state.startingLength).toBeGreaterThan(0);
        expect(reward.state.bodyLength).toBeGreaterThan(0);
        expect(reward.state.retainedGrowth).toBe(Math.max(
          reward.state.bodyLength - reward.state.startingLength, 0));
        const key = `${reward.worldEpoch}/${reward.leaseId}/${reward.physicalSnakeId}`;
        const predecessor = previous.get(key);
        if (reward.transitionKind === 'initial') {
          expect(predecessor).toBeUndefined();
          expect(reward.previousObservationSequence).toBeNull();
          expect(Object.values(reward.deltas).every(value => value === 0)).toBe(true);
        } else {
          expect(reward.previousObservationSequence).toBe(predecessor?.observationSequence);
          expect(reward.fromStep).toBe(predecessor?.toStep);
          expect(reward.state.startingLength).toBe(predecessor!.state.startingLength);
          expect(reward.state.bodyLength - predecessor!.state.bodyLength).toBe(reward.deltas.lengthDelta);
          expect(reward.state.retainedGrowth - predecessor!.state.retainedGrowth)
            .toBe(reward.deltas.retainedGrowthDelta);
          const delta = reward.deltas;
          expect(delta.pointsDelta).toBeCloseTo(delta.survivalPointsDelta + delta.foodPointsDelta +
            delta.killPointsDelta + delta.otherPointsDelta - delta.boostPointsSpentDelta, 8);
          expect(Number.isFinite(delta.aliveSecondsDelta)).toBe(true);
          expect(delta.aliveSecondsDelta).toBeGreaterThanOrEqual(0);
          expect(Number.isFinite(delta.elapsedSeconds)).toBe(true);
          expect(delta.elapsedSeconds).toBeGreaterThanOrEqual(0);
        }
        previous.set(key, reward);
      }
    } finally {
      for (const peer of peers) peer.socket.terminate();
      await server?.close();
      await cleanupRewardFixture(root);
    }
  }, 30_000);
  it('routes one native decision inspector through real sockets and releases disconnected ownership', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-inspector-'));
    const peers: Peer[] = [];
    let server: Awaited<ReturnType<typeof startRustServer>> | undefined;
    try {
      server = await startRustServer({...DEFAULT_CONFIG, port: 0, resume: 'fresh', seed: 42,
        rustCalculationWorkers: 1, dbPath: join(root, 'game.sqlite')});
      const first = await connect(server.port, 'ui'); peers.push(first);
      const second = await connect(server.port, 'ui'); peers.push(second);
      for (const peer of peers) {
        await until(peer, () => peer.packets.some(packet => packet['type'] === 'welcome'));
        peer.socket.send(JSON.stringify({type: 'join', mode: 'spectator'}));
      }
      const selected = await selectPhysical(first, 'inspector-selection', {populationSlot: 0});
      const welcome = first.packets.find(packet => packet['type'] === 'welcome') as unknown as WelcomeMsg;
      /** Send one actual protocol command and await its correlated native result. */
      const command = async (peer: Peer, requestId: string, enabled = true, pureProbe = false): Promise<Record<string, unknown>> => {
        const message: InspectEntityMsg = {type: 'inspectEntity', requestId,
          expectedSessionId: welcome.sessionId, expectedRunId: welcome.runId,
          ...(enabled ? {selectionToken: selected.token, snakeId: selected.snakeId, ...(pureProbe ? {pureProbe: true} : {})} : {})};
        peer.socket.send(JSON.stringify(message));
        await until(peer, () => peer.packets.some(packet => packet['requestId'] === requestId));
        return peer.packets.findLast(packet => packet['requestId'] === requestId)!;
      };
      const opened = await command(first, 'inspector-open');
      expect(opened).toMatchObject({type: 'inspectorResult', applied: true, enabled: true});
      await until(first, () => first.packets.some(packet => packet['type'] === 'inspector'));
      const row = parseInspectorSnapshot(first.packets.find(packet => packet['type'] === 'inspector')!['snapshot']);
      expect(row.subscriptionSequence).toBe(opened['subscriptionSequence']);
      expect(row.sample).toMatchObject({snakeId: selected.snakeId, profile: 'motion', bins: 16, source: {kind: 'neural'}});
      expect(await command(second, 'inspector-busy')).toMatchObject({applied: false, enabled: false});
      expect(second.packets.some(packet => packet['type'] === 'inspector')).toBe(false);
      first.socket.terminate();
      let attempt = 0;
      await vi.waitFor(async () => {
        expect(await command(second, 'inspector-reclaim-' + ++attempt)).toMatchObject({applied: true, enabled: true});
      }, {timeout: 5000, interval: 50});
      await until(second, () => second.packets.some(packet => packet['type'] === 'inspector'));
      const reclaimed = parseInspectorSnapshot(second.packets.find(packet => packet['type'] === 'inspector')!['snapshot']);
      expect(reclaimed.subscriptionSequence).not.toBe(row.subscriptionSequence);
      const probe = await command(second, 'inspector-probe', true, true);
      expect(probe).toMatchObject({applied: true, enabled: true});
      await until(second, () => second.packets.some(packet => packet['type'] === 'inspector'
        && (packet['snapshot'] as ReturnType<typeof parseInspectorSnapshot>).subscriptionSequence === probe['subscriptionSequence']));
      const probed = parseInspectorSnapshot(second.packets.findLast(packet => packet['type'] === 'inspector'
        && (packet['snapshot'] as ReturnType<typeof parseInspectorSnapshot>).subscriptionSequence === probe['subscriptionSequence'])!['snapshot']);
      expect(probed.sample).toMatchObject({pureProbe: true, snakeId: selected.snakeId, profile: 'motion', neuralOutputs: null,
        baselineV1Action: null, baselineAction: null, shadowOutputs: []});
      expect(await command(second, 'inspector-close', false)).toMatchObject({applied: true, enabled: false});
      expect(second.frames).toBeGreaterThan(0);
    } finally {
      for (const peer of peers) peer.socket.terminate();
      await server?.close();
      await cleanupInspectorFixture(root);
    }
  }, 30_000);

  it('opens independent distillation discovery lazily and rejects v3 capture without controller ownership', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-distillation-'));
    const databasePath = join(root, 'source.sqlite');
    const server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, resume: 'fresh', seed: 42, dbPath: databasePath });
    const base = `http://127.0.0.1:${server.port}`;
    try {
      const viewer = await connect(server.port, 'ui');
      try {
        await until(viewer, () => viewer.packets.some(packet => packet['type'] === 'welcome'));
        viewer.socket.send(JSON.stringify({type: 'join', mode: 'spectator'}));
        viewer.socket.send(JSON.stringify({type: 'reset', graphSpec: null, updates: [
          {path: 'sense.layoutVersion', value: 3}, {path: 'baselineBots.algorithm', value: 1}]}));
        await replacementUntil(viewer, server.port, 'reset');
      } finally { await new Promise<void>(done => { viewer.socket.once('close', () => done()); viewer.socket.close(); }); }
      const initial = await (await fetch(`${base}/api/health`)).json() as { runId: string; configRevision: number; completedStep: string };
      const discovery = await fetch(`${base}/api/distillation/v1/datasets`);
      expect(discovery.status).toBe(200);
      expect(await discovery.json()).toMatchObject({ datasets: [], hasMore: false });
      const capture = await fetch(`${base}/api/distillation/v1/capture/start`, { method: 'POST',
        headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({
          expectedRunId: initial.runId, expectedConfigRevision: initial.configRevision.toString(16).padStart(16, '0'),
          baselineSlots: [0], pendingBytes: 1024 * 1024, queueChunks: 4, queueBytes: 16 * 1024 * 1024
        }) });
      expect(capture.status).toBe(400);
      expect(await capture.text()).toMatch(/baseline-v2|v4/u);
      const later = await (await fetch(`${base}/api/health`)).json() as { runId: string; completedStep: string; outbound: { connections: number } };
      expect(later.runId).toBe(initial.runId);
      expect(BigInt(`0x${later.completedStep}`)).toBeGreaterThanOrEqual(BigInt(`0x${initial.completedStep}`));
      expect(later.outbound.connections).toBe(0);
      expect((await fetch(`${base}/api/distillation/v1/datasets/${'12'.repeat(16)}/manifest`)).status).toBe(404);
    } finally { await server.close(); await rm(root, { recursive: true, force: true }); }
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
});
