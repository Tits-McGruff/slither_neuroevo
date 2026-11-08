/** Real HTTP archive framing and wire-limit acceptance with an unchanged prior experiment. */
import Database from 'better-sqlite3';
import { readFile,writeFile } from 'node:fs/promises';
import { request,type ClientRequest } from 'node:http';
import { createRequire } from 'node:module';
import { dirname,join,resolve } from 'node:path';
import { expect,it,onTestFinished,vi } from 'vitest';
import WebSocket from 'ws';
import { BackgroundOutputPump } from './rustEngine/backgroundOutput.ts';
import type { ExperimentalRunningAuthorityNativeHandle } from './rustEngine/backgroundRuntime.ts';
import { CheckpointPersistenceClient } from './rustEngine/checkpointPersistenceClient.ts';
import { ExternalControllerRouting } from './rustEngine/externalRouting.ts';
import type { ExperimentalRuntimeTelemetrySnapshot } from './rustEngine/runtimeTelemetry.ts';
import { advancing,ARCHIVE_FIXTURE_TIMEOUT_MS,ARCHIVE_RETRY_FIXTURE_TIMEOUT_MS,archiveEntries,bounded,controller,evolvedArchiveFixture,experiment,exportAfterDisconnect,files,headingChange,health,heldInput,legacyPopulation,loopbackResponse,metadata,noTransferScratch,observeDisconnect,outcome,preserved,testCleanup } from './rustServerArchiveFixture.ts';
import { describeNetworkSuite } from './test/networkSuites.ts';

describeNetworkSuite('Rust archive HTTP framing', () => {
  it('exports only explicitly selected saved brains and atomically restores heterogeneous library membership', async () => {
    await experiment(async fixture => {
      const base = `http://127.0.0.1:${fixture.server.port}`;
      const saved: Array<{brainId: string; name: string; createdAtMs: number; bytes: Buffer}> = [];
      for (const name of ['portable-brain-v1.slither-brain', 'portable-dense-basic8-v1.slither-brain']) {
        const bytes = await readFile(resolve('native/fixtures', name));
        const response = await loopbackResponse(`${base}/api/distillation/v1/library/import?name=${encodeURIComponent(name)}`, {
          method: 'POST', headers: {'Content-Type': 'application/octet-stream'}, body: bytes
        });
        expect(response.status).toBe(201);
        const result = await response.json() as {brainId: string; name: string; createdAtMs: number};
        saved.push({...result, bytes});
      }
      saved.sort((a, b) => a.brainId.localeCompare(b.brainId));
      for (const query of ['?brainIds=', `?brainIds=${saved[0]!.brainId},${saved[0]!.brainId}`, '?unexpected=true']) {
        const invalid = await loopbackResponse(`${base}/api/export/latest${query}`);
        expect(invalid.status).toBe(400); await invalid.arrayBuffer();
      }
      const released = vi.spyOn(CheckpointPersistenceClient.prototype, 'releaseExportLease');
      onTestFinished(() => released.mockRestore());
      /** Wait for the worker acknowledgement after ready-file removal, before another cold operation. */
      const releaseCompleted = async (count: number): Promise<void> => {
        await vi.waitFor(() => expect(released).toHaveBeenCalledTimes(count), {timeout: 5000, interval: 10});
        await released.mock.results[count - 1]!.value;
        await new Promise<void>(done => setImmediate(done));
      };
      const missing = await loopbackResponse(`${base}/api/export/latest?brainIds=${'0'.repeat(64)}`);
      expect(missing.status).not.toBe(200); await missing.arrayBuffer();
      await releaseCompleted(1);
      await noTransferScratch(fixture.managedDirectory);
      const response = await loopbackResponse(`${base}/api/export/latest?brainIds=${saved.map(entry => entry.brainId).join(',')}`);
      expect(response.status).toBe(200);
      const selectedArchive = Buffer.from(await response.arrayBuffer());
      await releaseCompleted(2);
      await noTransferScratch(fixture.managedDirectory);
      const entries = archiveEntries(selectedArchive);
      const last = entries.at(-1)!;
      const manifest = JSON.parse(selectedArchive.subarray(last.data, last.data + last.size).toString()) as {
        archiveVersion: number; recordsVersion: number; savedBrains: Array<Record<string, unknown>>;
      };
      expect(manifest.archiveVersion).toBe(4); expect(manifest.recordsVersion).toBeGreaterThanOrEqual(1);
      expect(manifest.savedBrains).toEqual(saved.map(({brainId, name, createdAtMs, bytes}) => ({brainId, name, createdAtMs,
        artifactByteCount: bytes.byteLength.toString(16).padStart(16, '0')})));
      for (const savedBrain of saved) {
        const entry = entries.find(entry => selectedArchive.subarray(entry.header, entry.header + 100).toString().split('\0')[0] === `brains/${savedBrain.brainId}.slither-brain`)!;
        expect(entry).toBeDefined(); expect(selectedArchive.subarray(entry.data, entry.data + entry.size)).toEqual(savedBrain.bytes);
        const deletion = await loopbackResponse(`${base}/api/distillation/v1/library/${savedBrain.brainId}`, {method: 'DELETE'});
        expect(deletion.status).toBe(200); await deletion.arrayBuffer();
      }
      const beforeImport = await loopbackResponse(`${base}/api/distillation/v1/library`);
      expect((await beforeImport.json() as {page: {entries: unknown[]}}).page.entries).toEqual([]);
      const beforeFailure = await health(fixture.server);
      const injected = new Database(fixture.databasePath);
      try {
        injected.exec(`CREATE TRIGGER reject_second_archive_brain BEFORE INSERT ON rust_saved_brains_v1
          WHEN NEW.brain_id = '${saved[1]!.brainId}' BEGIN SELECT RAISE(ABORT, 'injected archive membership failure'); END`);
        const rejected = await loopbackResponse(`${base}/api/import/archive`, {method: 'POST', headers: {'Content-Type': 'application/vnd.slither-neuroevo.save'}, body: selectedArchive});
        expect(rejected.status).not.toBe(200); expect(await rejected.text()).toContain('injected archive membership failure');
        await noTransferScratch(fixture.managedDirectory);
        const unchanged = await loopbackResponse(`${base}/api/distillation/v1/library`);
        expect((await unchanged.json() as {page: {entries: unknown[]}}).page.entries).toEqual([]);
        expect(await health(fixture.server)).toMatchObject({ok: true, runId: beforeFailure['runId'],
          worldEpoch: beforeFailure['worldEpoch'], startupCheckpointId: beforeFailure['startupCheckpointId']});
      } finally { injected.exec('DROP TRIGGER IF EXISTS reject_second_archive_brain'); injected.close(); }
      const imported = await loopbackResponse(`${base}/api/import/archive`, {method: 'POST', headers: {'Content-Type': 'application/vnd.slither-neuroevo.save'}, body: selectedArchive});
      expect(imported.status).toBe(200); await imported.arrayBuffer();
      await noTransferScratch(fixture.managedDirectory);
      const restored = await loopbackResponse(`${base}/api/distillation/v1/library`);
      const page = (await restored.json() as {page: {entries: Array<Record<string, unknown>>}}).page;
      expect(page.entries).toHaveLength(2);
      for (const entry of saved) expect(page.entries.find(row => row['brainId'] === entry.brainId)).toMatchObject({name: entry.name, createdAtMs: entry.createdAtMs});
      const ordinary = await loopbackResponse(`${base}/api/export/latest`); expect(ordinary.status).toBe(200);
      const bytes = Buffer.from(await ordinary.arrayBuffer());
      await releaseCompleted(3);
      const final = archiveEntries(bytes).at(-1)!;
      const ordinaryManifest = JSON.parse(bytes.subarray(final.data, final.data + final.size).toString()) as Record<string, unknown>;
      expect(ordinaryManifest['archiveVersion']).toBeLessThan(4);
      expect(ordinaryManifest).not.toHaveProperty('savedBrains');
      await noTransferScratch(fixture.managedDirectory);
    });
  }, ARCHIVE_RETRY_FIXTURE_TIMEOUT_MS);

  it.each(['import', 'reset', 'newRun'] as const)(
    'preserves player and trainer leases and held input after a failed staged %s', async kind => {
    await experiment(async fixture => {
      const peers = await Promise.all([controller(fixture, 'ui'), controller(fixture, 'bot')]);
      let reached!: () => void;
      let release!: () => void;
      const staged = new Promise<void>(done => { reached = done; });
      const released = new Promise<void>(done => { release = done; });
      const originalStage = BackgroundOutputPump.prototype.stagePreparedImport;
      const stage = vi.spyOn(BackgroundOutputPump.prototype, 'stagePreparedImport')
        .mockImplementationOnce(async function(this: BackgroundOutputPump) {
          await originalStage.call(this);
          reached();
          await released;
        });
      const observation = observeDisconnect(fixture.server);
      const cleanup = testCleanup(() => {
        release();
        stage.mockRestore();
        observation.restore();
      });
      try {
        let client: ClientRequest | undefined;
        if (kind === 'import') {
          const body = legacyPopulation();
          client = request(`http://127.0.0.1:${fixture.server.port}/api/import/archive`, {
            method: 'POST', headers: { 'Content-Length': body.byteLength }
          });
          fixture.requests.add(client);
          client.on('error', () => {});
          client.end(body);
        } else {
          const database = new Database(fixture.databasePath);
          try {
            database.exec(`CREATE TRIGGER reject_replacement_activation
              BEFORE INSERT ON rust_active_run_v1 BEGIN
                SELECT RAISE(ABORT, 'injected controller preservation rejection');
              END`);
          } finally { database.close(); }
          peers[0]!.socket.send(JSON.stringify(kind === 'reset' ? { type: 'reset' } :
            { type: 'newRun', requestId: 'held-controller-rejection' }));
        }
        await bounded(staged, 'import never held the old world');
        const before = await health(fixture.server);
        const activity = (before['telemetry'] as ExperimentalRuntimeTelemetrySnapshot).controllerActivity;
        const baselines = peers.map(peer => ({ assignment: { ...peer.assignment! }, sample: peer.sample! }));
        await heldInput(peers);
        if (client) {
          client.destroy();
          expect(await bounded(observation.closed, 'server never observed disconnect')).toBe(true);
        }
        release();
        if (kind !== 'import') {
          await outcome(() => peers[0]!.packets.some(packet => kind === 'reset' ?
            packet['type'] === 'error' && String(packet['message']).includes('injected controller preservation rejection') :
            packet['type'] === 'newRunResult' && packet['applied'] === false &&
              String(packet['reason']).includes('injected controller preservation rejection')), 'replacement did not reject its actual transaction');
        }
        await preserved(fixture);
        await outcome(() => peers.every((peer, index) => peer.sample!.tick > baselines[index]!.sample.tick &&
          headingChange(baselines[index]!.sample, peer.sample!) < -0.01), 'held steering was not applied after cancellation');
        for (const [index, peer] of peers.entries()) {
          expect(peer.socket.readyState).toBe(WebSocket.OPEN);
          expect(peer.assignment).toEqual(baselines[index]!.assignment);
          expect(peer.packets.filter(packet => packet['type'] === 'stateReplaced')).toEqual([]);
          expect(peer.packets.filter(packet => packet['type'] === 'error')).toHaveLength(kind === 'reset' && index === 0 ? 1 : 0);
        }
        const after = await health(fixture.server);
        const applied = (after['telemetry'] as ExperimentalRuntimeTelemetrySnapshot).controllerActivity;
        expect(applied.player.appliedActions).toBe(activity.player.appliedActions + 1);
        expect(applied.trainer.appliedActions).toBe(activity.trainer.appliedActions + 1);
      } finally {
        cleanup();
      }
    });
  }, 15_000);

  it.each(['import', 'reset', 'newRun'] as const)(
    'discards held player/trainer input and old tokens after committed %s until explicit rejoin', async kind => {
    await experiment(async fixture => {
      const peers = await Promise.all([controller(fixture, 'ui'), controller(fixture, 'bot')]);
      const baselines = peers.map(peer => ({ assignment: { ...peer.assignment! }, sample: peer.sample! }));
      const activity = ((await health(fixture.server))['telemetry'] as ExperimentalRuntimeTelemetrySnapshot).controllerActivity;
      let reached!: () => void;
      let release!: () => void;
      const staged = new Promise<void>(done => { reached = done; });
      const released = new Promise<void>(done => { release = done; });
      const originalStage = BackgroundOutputPump.prototype.stagePreparedImport;
      const stage = vi.spyOn(BackgroundOutputPump.prototype, 'stagePreparedImport')
        .mockImplementationOnce(async function(this: BackgroundOutputPump) {
          await originalStage.call(this);
          reached();
          await released;
        });
      let response: Promise<Response> | undefined;
      try {
        if (kind === 'import') response = fetch(`http://127.0.0.1:${fixture.server.port}/api/import/archive`, {
          method: 'POST', body: new Uint8Array(legacyPopulation())
        });
        else peers[0]!.socket.send(JSON.stringify(kind === 'reset' ? { type: 'reset' } :
          { type: 'newRun', requestId: 'held-controller-publication' }));
        await bounded(staged, 'replacement never held the old world');
        await heldInput(peers);
        release();
        if (response) {
          const imported = await bounded(response, 'import never responded');
          expect(imported.status).toBe(200);
          expect(await imported.json()).toMatchObject({ ok: true });
        }
        await outcome(() => peers.every(peer => peer.packets.some(packet =>
          packet['type'] === 'stateReplaced' && packet['reason'] === kind)), 'replacement notice was not delivered');
        expect((await health(fixture.server))['worldEpoch']).not.toBe(fixture.identity['worldEpoch']);
        expect(await files(fixture.managedDirectory)).toEqual(expect.arrayContaining(fixture.files));
        for (const [index, peer] of peers.entries()) {
          expect(peer.socket.readyState).toBe(WebSocket.OPEN);
          expect(peer.assignment).toBeUndefined();
          expect(peer.sample).toBeUndefined();
          expect(peer.packets.filter(packet => packet['type'] === 'stateReplaced')).toHaveLength(1);
          peer.socket.send(JSON.stringify({ type: 'join', rejoinToken: peer.packets.findLast(packet => packet['type'] === 'stateReplaced')?.['rejoinToken'], mode: 'player', name: `Import-${index === 0 ? 'ui' : 'bot'}`,
            resumeToken: baselines[index]!.assignment.resumeToken }));
          peer.socket.send(JSON.stringify({ type: 'action', snakeId: baselines[index]!.assignment.snakeId,
            tick: baselines[index]!.sample.tick, turn: 1, boost: 1 }));
        }
        await outcome(() => peers.every(peer => peer.packets.some(packet =>
          packet['type'] === 'reclaimResult' && packet['reclaimed'] === false)), 'old tokens were not rejected');
        await advancing(fixture);
        const unassigned = ((await health(fixture.server))['telemetry'] as ExperimentalRuntimeTelemetrySnapshot).controllerActivity;
        expect(unassigned.player.appliedActions).toBe(activity.player.appliedActions);
        expect(unassigned.trainer.appliedActions).toBe(activity.trainer.appliedActions);
        for (const [index, peer] of peers.entries()) {
          expect(peer.assignment).toBeUndefined();
          peer.socket.send(JSON.stringify({ type: 'join', rejoinToken: peer.packets.findLast(packet => packet['type'] === 'stateReplaced')?.['rejoinToken'], mode: 'player', name: `Import-${index === 0 ? 'ui' : 'bot'}` }));
        }
        await outcome(() => peers.every(peer => !!peer.assignment && !!peer.sample), 'explicit rejoin did not assign new leases')
          .catch((error: Error) => { throw new Error(error.message + ': ' + JSON.stringify(peers.map(peer => ({
            assigned: !!peer.assignment, sampled: !!peer.sample, packets: peer.packets.map(packet => ({
              type: packet['type'], reason: packet['reason'], message: packet['message'], reclaimed: packet['reclaimed']
            }))
          })))); });
        const samples = peers.map(peer => peer.sample!);
        for (const [index, peer] of peers.entries()) {
          expect(peer.assignment!.resumeToken).not.toBe(baselines[index]!.assignment.resumeToken);
          expect(peer.packets.filter(packet => packet['type'] === 'assign')).toHaveLength(2);
          expect(peer.packets.filter(packet => packet['type'] === 'error')).toEqual([]);
        }
        await heldInput(peers);
        await outcome(() => peers.every((peer, index) => {
          const sample = peer.sample;
          return sample !== undefined && sample.tick > samples[index]!.tick &&
            headingChange(samples[index]!, sample) < -0.01;
        }), 'rejoined controllers could not steer');
        const rejoined = ((await health(fixture.server))['telemetry'] as ExperimentalRuntimeTelemetrySnapshot).controllerActivity;
        expect(rejoined.player.appliedActions).toBeGreaterThan(activity.player.appliedActions);
        expect(rejoined.trainer.appliedActions).toBe(activity.trainer.appliedActions + 1);
      } finally {
        release();
        stage.mockRestore();
        await response?.catch(() => {});
      }
    });
  }, 15_000);

  it('replaces evolved controller leases, drops held input and repeats an identical exact import', async () => {
    await experiment(async fixture => {
      await evolvedArchiveFixture(fixture);
      const sourcePath = join(dirname(fixture.databasePath), 'user-original.save');
      await writeFile(sourcePath, fixture.archive, { flag: 'wx' });
      const peers = await Promise.all([controller(fixture, 'ui'), controller(fixture, 'bot')]);
      const nativeRequire = createRequire(import.meta.url);
      const addon = nativeRequire(resolve('native/index.js')) as {
        ExperimentalRunningAuthority: { prototype: ExperimentalRunningAuthorityNativeHandle };
      };
      const submit = vi.spyOn(addon.ExperimentalRunningAuthority.prototype, 'submitControllerAction');
      const input = vi.spyOn(ExternalControllerRouting.prototype, 'action');
      const originalStage = BackgroundOutputPump.prototype.stagePreparedImport;
      const viewer = new WebSocket(`ws://127.0.0.1:${fixture.server.port}`);
      fixture.peers.add(viewer);
      const replies: Array<Record<string, unknown>> = [];
      let replacement = 0;
      viewer.on('message', (bytes, binary) => {
        if (binary) return;
        const packet = JSON.parse(bytes.toString()) as Record<string, unknown>;
        if (replies.length < 64) replies.push(packet);
        if (packet['type'] === 'welcome' || packet['type'] === 'stateReplaced') {
          viewer.send(JSON.stringify({ type: 'join', mode: 'spectator', rejoinToken: packet['rejoinToken'] }));
          if (packet['type'] === 'stateReplaced') viewer.send(JSON.stringify({ type: 'settings',
            requestId: `replacement-speed-${++replacement}`, updates: [{ path: 'simSpeed', value: 0.1 }] }));
        }
      });
      viewer.on('open', () => viewer.send(JSON.stringify({ type: 'hello', version: 2, clientType: 'ui' })));
      const restore = testCleanup(() => { submit.mockRestore(); input.mockRestore(); });
      try {
        await outcome(() => replies.some(packet => packet['type'] === 'welcome'), 'replacement viewer did not connect');
        for (const round of [1, 2]) {
          const probe = await controller(fixture, 'ui', `Import-probe-${round}`);
          const probeBefore = { assignment: { ...probe.assignment! }, sample: probe.sample! };
          const before = await health(fixture.server);
          const baselines = peers.map(peer => ({ assignment: { ...peer.assignment! }, packetCount: peer.packets.length }));
          const staged = Promise.withResolvers<void>();
          const release = Promise.withResolvers<void>();
          const stage = vi.spyOn(BackgroundOutputPump.prototype, 'stagePreparedImport')
            .mockImplementationOnce(async function(this: BackgroundOutputPump) {
              await originalStage.call(this);
              staged.resolve();
              await release.promise;
            });
          const cleanup = testCleanup(() => { release.resolve(); stage.mockRestore(); });
          let response: Promise<Response> | undefined;
          try {
            const submittedBefore = submit.mock.calls.length;
            const inputBefore = input.mock.calls.length;
            response = fetch(`http://127.0.0.1:${fixture.server.port}/api/import/archive`, {
              method: 'POST', body: new Uint8Array(fixture.archive), signal: AbortSignal.timeout(10_000) });
            await bounded(staged.promise, 'exact import did not stage its actual replacement');
            await heldInput(peers, false);
            expect(input.mock.calls.length).toBe(inputBefore + 2);
            expect(submit.mock.calls.length).toBe(submittedBefore);
            release.resolve();
            const imported = await bounded(response, 'exact import did not finish');
            expect(imported.status).toBe(200);
            expect(await imported.json()).toMatchObject({ ok: true, branched: false,
              runId: fixture.identity['runId'], generation: fixture.identity['generation'],
              checkpointId: fixture.identity['startupCheckpointId'] });
            await outcome(() => peers.every((peer, index) => peer.packets.slice(baselines[index]!.packetCount)
              .some(packet => packet['type'] === 'stateReplaced')), 'live controllers missed replacement');
            await outcome(() => replies.some(packet => packet['type'] === 'settingsApplied' &&
              packet['requestId'] === `replacement-speed-${round}` && packet['applied'] === true),
            'replacement world did not accept its live slowdown');
            expect(submit.mock.calls.length).toBe(submittedBefore);
            const replaced = await health(fixture.server);
            expect(BigInt(`0x${replaced['worldEpoch'] as string}`)).toBeGreaterThan(BigInt(`0x${before['worldEpoch'] as string}`));
            for (const [index, peer] of peers.entries()) {
              expect(peer.socket.readyState).toBe(WebSocket.OPEN);
              expect(peer.assignment).toBeUndefined();
              expect(peer.sample).toBeUndefined();
              const packets = peer.packets.slice(baselines[index]!.packetCount);
              expect(packets.filter(packet => packet['type'] === 'stateReplaced')).toHaveLength(1);
              expect(packets.filter(packet => ['assign', 'error'].includes(String(packet['type'])))).toEqual([]);
            }
            const processed = new Promise<Buffer>(done => probe.socket.once('pong', done));
            probe.socket.send(JSON.stringify({ type: 'action', snakeId: probeBefore.assignment.snakeId,
              tick: probeBefore.sample.tick, turn: 1, boost: 0 }));
            probe.socket.send(JSON.stringify({ type: 'view', viewW: 800, viewH: 600 }));
            probe.socket.ping(`stale-input-${round}`);
            expect((await bounded(processed, 'stale-input transport barrier did not complete')).toString())
              .toBe(`stale-input-${round}`);
            expect(probe.socket.readyState).toBe(WebSocket.OPEN);
            expect(probe.packets.filter(packet => packet['type'] === 'error')).toEqual([]);
            expect(input.mock.calls.length).toBe(inputBefore + 2);
            expect(submit.mock.calls.length).toBe(submittedBefore);
            for (const [index, peer] of peers.entries()) peer.socket.send(JSON.stringify({ type: 'join', rejoinToken: peer.packets.findLast(packet => packet['type'] === 'stateReplaced')?.['rejoinToken'],
              mode: 'player', name: `Import-${index === 0 ? 'ui' : 'bot'}`,
              resumeToken: baselines[index]!.assignment.resumeToken }));
            await outcome(() => peers.every((peer, index) => peer.packets.slice(baselines[index]!.packetCount)
              .some(packet => packet['type'] === 'reclaimResult')), 'old replacement tokens were not resolved');
            for (const [index, peer] of peers.entries()) {
              expect(peer.assignment).toBeUndefined();
              expect(peer.sample).toBeUndefined();
              expect(peer.packets.slice(baselines[index]!.packetCount).filter(packet => packet['type'] === 'reclaimResult'))
                .toEqual([{ type: 'reclaimResult', reclaimed: false, reason: 'invalid' }]);
              peer.socket.send(JSON.stringify({ type: 'join', rejoinToken: peer.packets.findLast(packet => packet['type'] === 'stateReplaced')?.['rejoinToken'], mode: 'player', name: `Import-${index === 0 ? 'ui' : 'bot'}` }));
            }
            await outcome(() => peers.every(peer => !!peer.assignment && !!peer.sample), 'fresh replacement assignments were lost');
            for (const [index, peer] of peers.entries()) {
              const packets = peer.packets.slice(baselines[index]!.packetCount);
              expect(packets.filter(packet => packet['type'] === 'reclaimResult'))
                .toEqual([expect.objectContaining({ reclaimed: false })]);
              expect(packets.filter(packet => packet['type'] === 'assign')).toHaveLength(1);
              expect(peer.assignment!.reclaimed).not.toBe(true);
              expect(peer.assignment!.resumeToken).not.toBe(baselines[index]!.assignment.resumeToken);
              expect(peer.socket.readyState).toBe(WebSocket.OPEN);
            }
            const fresh = peers.map(peer => peer.sample!);
            const active = (await health(fixture.server))['telemetry'] as ExperimentalRuntimeTelemetrySnapshot;
            await heldInput(peers, false);
            await outcome(() => peers.every((peer, index) => peer.sample!.tick > fresh[index]!.tick &&
              headingChange(fresh[index]!, peer.sample!) < -0.01), 'fresh steering was lost after exact replacement');
            expect(submit.mock.calls.length).toBe(submittedBefore + 2);
            const applied = ((await health(fixture.server))['telemetry'] as ExperimentalRuntimeTelemetrySnapshot).controllerActivity;
            expect(applied.player.appliedActions).toBe(active.controllerActivity.player.appliedActions + 1);
            expect(applied.trainer.appliedActions).toBe(active.controllerActivity.trainer.appliedActions + 1);
            await exportAfterDisconnect(fixture);
            await noTransferScratch(fixture.managedDirectory);
            expect(metadata(fixture.databasePath)).toEqual(fixture.metadata);
            expect(await files(fixture.managedDirectory)).toEqual(fixture.files);
            expect((await readFile(sourcePath)).equals(fixture.archive)).toBe(true);
          } finally { cleanup(); await response?.catch(() => {}); }
        }
      } finally { restore(); }
    });
  }, 30_000);
}, ARCHIVE_FIXTURE_TIMEOUT_MS);
