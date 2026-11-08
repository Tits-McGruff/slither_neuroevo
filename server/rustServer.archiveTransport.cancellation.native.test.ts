/** Real HTTP archive framing and wire-limit acceptance with an unchanged prior experiment. */
import Database from 'better-sqlite3';
import { createHash } from 'node:crypto';
import { readdir,readFile,writeFile } from 'node:fs/promises';
import { request,Server,type IncomingMessage,type ServerResponse } from 'node:http';
import { createRequire } from 'node:module';
import { dirname,join,resolve } from 'node:path';
import { expect,it,vi } from 'vitest';
import WebSocket from 'ws';
import * as archiveUpload from './rustEngine/archiveUpload.ts';
import { BackgroundOutputPump } from './rustEngine/backgroundOutput.ts';
import type { ExperimentalRunningAuthorityNativeHandle } from './rustEngine/backgroundRuntime.ts';
import { CheckpointPersistenceClient } from './rustEngine/checkpointPersistenceClient.ts';
import * as diskAdmission from './rustEngine/diskAdmission.ts';
import type { ExperimentalRuntimeTelemetrySnapshot } from './rustEngine/runtimeTelemetry.ts';
import { startRustServer } from './rustServer.ts';
import { advancing,ARCHIVE_FIXTURE_TIMEOUT_MS,bounded,controller,evolvedArchiveFixture,experiment,exportAfterDisconnect,files,headingChange,health,heldInput,legacyPopulation,noTransferScratch,observeDisconnect,originalHttpEmit,outcome,preserved,raw,testCleanup } from './rustServerArchiveFixture.ts';
import { ARCHIVE_PREPARATION_TIMEOUT_MS } from './test/archiveDownload.ts';
import { describeNetworkSuite } from './test/networkSuites.ts';
import { RUST_TEST_CONFIG as DEFAULT_CONFIG } from './test/rustConfig.ts';

describeNetworkSuite('Rust archive HTTP framing', () => {

  it.each(['after source selection', 'after disk admission'] as const)(
    'skips native export work when the client disconnects %s', async boundary => {
      await experiment(async fixture => {
        await evolvedArchiveFixture(fixture);
        const sourcePath = join(dirname(fixture.databasePath), 'user-original.save');
        await writeFile(sourcePath, fixture.archive, { flag: 'wx' });
        const peers = await Promise.all([controller(fixture, 'ui'), controller(fixture, 'bot')]);
        const before = await health(fixture.server);
        const activity = (before['telemetry'] as ExperimentalRuntimeTelemetrySnapshot).controllerActivity;
        const baselines = peers.map(peer => ({ assignment: { ...peer.assignment! }, sample: peer.sample! }));
        const binding = createRequire(import.meta.url)(resolve('native/index.js')) as {
          ExperimentalRunningAuthority: { prototype: ExperimentalRunningAuthorityNativeHandle };
        };
        expect(binding.ExperimentalRunningAuthority).toBeTypeOf('function');
        const prepare = vi.spyOn(binding.ExperimentalRunningAuthority.prototype, 'prepareExportArchive');
        const reached = Promise.withResolvers<void>();
        const gate = Promise.withResolvers<void>();
        const originalAcquire = CheckpointPersistenceClient.prototype.acquireCurrentExportLease;
        const acquire = vi.spyOn(CheckpointPersistenceClient.prototype, 'acquireCurrentExportLease')
          .mockImplementationOnce(async function(this: CheckpointPersistenceClient) {
            const lease = await originalAcquire.call(this);
            if (boundary === 'after source selection') { reached.resolve(); await gate.promise; }
            return lease;
          });
        const originalAdmit = diskAdmission.admitDiskOperation;
        const admission = vi.spyOn(diskAdmission, 'admitDiskOperation').mockImplementationOnce(async (...args) => {
          const decision = await originalAdmit(...args);
          if (boundary === 'after disk admission') { reached.resolve(); await gate.promise; }
          return decision;
        });
        const release = vi.spyOn(CheckpointPersistenceClient.prototype, 'releaseExportLease');
        const observation = observeDisconnect(fixture.server, '/api/export/latest');
        const client = request(`http://127.0.0.1:${fixture.server.port}/api/export/latest`);
        fixture.requests.add(client);
        let receivedHeaders = false;
        client.on('error', () => {});
        client.on('response', response => { receivedHeaders = true; response.resume(); });
        const cleanup = testCleanup(() => {
          gate.resolve(); client.destroy(); observation.restore();
          prepare.mockRestore(); acquire.mockRestore(); admission.mockRestore(); release.mockRestore();
        });
        try {
          client.end();
          await bounded(reached.promise, 'export did not reach its actual preparation boundary');
          await heldInput(peers, false);
          client.destroy();
          expect(await bounded(observation.closed, 'server did not observe early export cancellation')).toBe(true);
          gate.resolve();
          await outcome(() => release.mock.calls.length === 1, 'early cancellation did not release its source');
          await release.mock.results[0]!.value;
          expect(receivedHeaders).toBe(false);
          expect(prepare).not.toHaveBeenCalled();
          if (boundary === 'after source selection') expect(admission).not.toHaveBeenCalled();
          else expect(admission).toHaveBeenCalledOnce();
          await preserved(fixture);
          await outcome(() => peers.every((peer, index) => peer.sample!.tick > baselines[index]!.sample.tick &&
            headingChange(baselines[index]!.sample, peer.sample!) < -0.01), 'steering during early cancellation was lost');
          for (const [index, peer] of peers.entries()) {
            expect(peer.socket.readyState).toBe(WebSocket.OPEN);
            expect(peer.assignment).toEqual(baselines[index]!.assignment);
            expect(peer.packets.filter(packet => packet['type'] === 'assign')).toHaveLength(1);
            expect(peer.packets.filter(packet => ['error', 'stateReplaced'].includes(String(packet['type'])))).toEqual([]);
          }
          const after = await health(fixture.server);
          const applied = (after['telemetry'] as ExperimentalRuntimeTelemetrySnapshot).controllerActivity;
          expect(applied.player.appliedActions).toBe(activity.player.appliedActions + 1);
          expect(applied.trainer.appliedActions).toBe(activity.trainer.appliedActions + 1);
          expect(BigInt(`0x${after['completedStep'] as string}`)).toBeGreaterThan(BigInt(`0x${before['completedStep'] as string}`));
          expect((await readFile(sourcePath)).equals(fixture.archive)).toBe(true);
          await exportAfterDisconnect(fixture);
          await outcome(() => release.mock.calls.length === 2, 'retry did not release its source');
          await release.mock.results[1]!.value;
          expect(prepare).toHaveBeenCalledOnce();
          await preserved(fixture);
          expect((await readFile(sourcePath)).equals(fixture.archive)).toBe(true);
        } finally { cleanup(); }
      });
    }, 15_000
  );

  it.each(['during body delivery', 'while waiting for drain'] as const)(
    'cleans an evolved export cancelled %s and preserves live controllers', async boundary => {
      await experiment(async fixture => {
        await evolvedArchiveFixture(fixture, 'streaming');
        const sourcePath = join(dirname(fixture.databasePath), 'user-original.save');
        await writeFile(sourcePath, fixture.archive, { flag: 'wx' });
        const peers = await Promise.all([controller(fixture, 'ui'), controller(fixture, 'bot')]);
        const before = await health(fixture.server);
        const activity = (before['telemetry'] as ExperimentalRuntimeTelemetrySnapshot).controllerActivity;
        const baselines = peers.map(peer => ({ assignment: { ...peer.assignment! }, sample: peer.sample! }));
        let actualResponse: ServerResponse | undefined;
        let closedBeforeFinish: boolean | undefined;
        let clientResponse: IncomingMessage | undefined;
        let receivedBytes = 0;
        let reached!: () => void;
        const bodyReceived = new Promise<void>(done => { reached = done; });
        let serverClosed!: () => void;
        const disconnected = new Promise<void>(done => { serverClosed = done; });
        const dispatch = function(this: Server, event: string | symbol, ...args: unknown[]): boolean {
          if (event === 'request') {
            const incoming = args[0] as IncomingMessage;
            const response = args[1] as ServerResponse;
            if (incoming.socket.localPort === fixture.server.port && incoming.url === '/api/export/latest') {
              actualResponse = response;
              response.once('close', () => {
                closedBeforeFinish = !response.writableFinished;
                serverClosed();
              });
            }
          }
          return Reflect.apply(originalHttpEmit, this, [event, ...args]) as boolean;
        };
        Server.prototype.emit = dispatch;
        const release = vi.spyOn(CheckpointPersistenceClient.prototype, 'releaseExportLease');
        const client = request(`http://127.0.0.1:${fixture.server.port}/api/export/latest`);
        fixture.requests.add(client);
        client.on('error', () => {});
        client.on('response', response => {
          clientResponse = response;
          response.on('error', () => {});
          response.on('data', (bytes: Buffer) => {
            receivedBytes += bytes.byteLength;
            response.pause();
            reached();
          });
        });
        const cleanup = testCleanup(() => {
          client.destroy();
          clientResponse?.destroy();
          release.mockRestore();
          if (Server.prototype.emit === dispatch) Server.prototype.emit = originalHttpEmit;
        });
        try {
          client.end();
          // The first byte follows complete validation, encoding and publication
          // of this >8 MiB archive. Its preparation is not cancellation liveness;
          // keep the later disconnect/lease/controller deadlines at five seconds.
          await bounded(bodyReceived, 'download did not deliver body bytes', ARCHIVE_PREPARATION_TIMEOUT_MS);
          expect(clientResponse!.statusCode).toBe(200);
          expect(receivedBytes).toBeGreaterThan(0);
          expect(receivedBytes).toBeLessThan(fixture.archive.byteLength);
          expect(actualResponse!.writableFinished).toBe(false);
          await heldInput(peers, false);
          await outcome(() => peers.every((peer, index) => peer.sample!.tick > baselines[index]!.sample.tick &&
            headingChange(baselines[index]!.sample, peer.sample!) < -0.01),
          'simulation did not resume before archive body delivery completed');
          // Observe and cancel in one synchronous predicate after the controller
          // awaits; a socket drain observed before an await is not a stable state.
          if (boundary === 'while waiting for drain') {
            await outcome(() => {
              if (!actualResponse!.writableNeedDrain || actualResponse!.writableFinished) return false;
              expect(actualResponse!.writableFinished).toBe(false);
              expect(actualResponse!.writableNeedDrain).toBe(true);
              expect(actualResponse!.writableLength).toBeGreaterThan(0);
              clientResponse!.destroy();
              client.destroy();
              return true;
            }, 'real download did not wait for socket drain');
          } else {
            expect(actualResponse!.writableFinished).toBe(false);
            clientResponse!.destroy();
            client.destroy();
          }
          await bounded(disconnected, 'server did not observe mid-body cancellation');
          expect(closedBeforeFinish).toBe(true);
          await noTransferScratch(fixture.managedDirectory);
          await outcome(() => release.mock.calls.length === 1, 'cancelled download did not release its lease');
          await release.mock.results[0]!.value;
          await preserved(fixture);
          await outcome(() => peers.every((peer, index) => peer.sample!.tick > baselines[index]!.sample.tick &&
            headingChange(baselines[index]!.sample, peer.sample!) < -0.01), 'steering during cancelled export was lost');
          for (const [index, peer] of peers.entries()) {
            expect(peer.socket.readyState).toBe(WebSocket.OPEN);
            expect(peer.assignment).toEqual(baselines[index]!.assignment);
            expect(peer.packets.filter(packet => packet['type'] === 'assign')).toHaveLength(1);
            expect(peer.packets.filter(packet => ['error', 'stateReplaced'].includes(String(packet['type'])))).toEqual([]);
          }
          const after = await health(fixture.server);
          const applied = (after['telemetry'] as ExperimentalRuntimeTelemetrySnapshot).controllerActivity;
          expect(applied.player.appliedActions).toBe(activity.player.appliedActions + 1);
          expect(applied.trainer.appliedActions).toBe(activity.trainer.appliedActions + 1);
          expect(BigInt(`0x${after['completedStep'] as string}`)).toBeGreaterThan(BigInt(`0x${before['completedStep'] as string}`));
          expect((await readFile(sourcePath)).equals(fixture.archive)).toBe(true);
          await exportAfterDisconnect(fixture);
          await outcome(() => release.mock.calls.length === 2, 'retry download did not release its lease');
          expect(release).toHaveBeenCalledTimes(2);
          await release.mock.results[1]!.value;
          await preserved(fixture);
          expect((await readFile(sourcePath)).equals(fixture.archive)).toBe(true);
        } finally { cleanup(); }
      });
    }, 90_000
  );

  it.each(['afterSpool', 'afterPreparation', 'afterStage', 'beforeCommit'] as const)(
    'preserves the experiment when the import client disconnects %s', async boundary => {
      await experiment(async fixture => {
        let reached!: () => void;
        let release!: () => void;
        const paused = new Promise<void>(done => { reached = done; });
        const released = new Promise<void>(done => { release = done; });
        const hold = async (): Promise<void> => { reached(); await released; };
        const originalSpool = archiveUpload.spoolArchiveUpload;
        const originalStage = BackgroundOutputPump.prototype.stagePreparedImport;
        const originalSelect = CheckpointPersistenceClient.prototype.selectStartup;
        const binding = createRequire(import.meta.url)(resolve('native/index.js')) as {
          ExperimentalRunningAuthority: { prototype: ExperimentalRunningAuthorityNativeHandle };
        };
        const originalPrepare = binding.ExperimentalRunningAuthority.prototype.prepareImportArchive;
        const boundarySpy = boundary === 'afterSpool'
          ? vi.spyOn(archiveUpload, 'spoolArchiveUpload').mockImplementationOnce(async options => {
            const result = await originalSpool(options);
            await hold();
            return result;
          })
          : boundary === 'afterPreparation'
            ? vi.spyOn(binding.ExperimentalRunningAuthority.prototype, 'prepareImportArchive')
              .mockImplementationOnce(async function(this: ExperimentalRunningAuthorityNativeHandle, ...args) {
                const result = await originalPrepare.apply(this, args);
                await hold();
                return result;
              })
          : boundary === 'beforeCommit'
            ? vi.spyOn(CheckpointPersistenceClient.prototype, 'selectStartup')
              .mockImplementationOnce(async function(this: CheckpointPersistenceClient, ...args) {
                const result = await originalSelect.apply(this, args);
                await hold();
                return result;
              })
            : vi.spyOn(BackgroundOutputPump.prototype, 'stagePreparedImport')
            .mockImplementationOnce(async function(this: BackgroundOutputPump) {
              await originalStage.call(this);
              if (boundary === 'afterStage') await hold();
            });
        const observation = observeDisconnect(fixture.server);
        const cleanup = testCleanup(() => {
          release();
          boundarySpy.mockRestore();
          observation.restore();
        });
        const body = legacyPopulation();
        const sourceHash = createHash('sha256').update(body).digest('hex');
        try {
          const client = request(`http://127.0.0.1:${fixture.server.port}/api/import/archive`, {
            method: 'POST', headers: { 'Content-Length': body.byteLength }
          });
          fixture.requests.add(client);
          client.on('error', () => {});
          client.end(body);
          await bounded(paused, `import never reached ${boundary}`);
          if (boundary === 'afterPreparation' || boundary === 'beforeCommit') {
            const checkpoints = (await readdir(fixture.managedDirectory)).filter(name => name.endsWith('.checkpoint-v3'));
            expect(checkpoints.length).toBeGreaterThan(
              fixture.files.filter(file => file.filename.endsWith('.checkpoint-v3')).length);
          }
          client.destroy();
          expect(await bounded(observation.closed, 'server never observed disconnect')).toBe(true);
          release();
          await preserved(fixture);
          expect(createHash('sha256').update(body).digest('hex')).toBe(sourceHash);
          await advancing(fixture);
        } finally {
          cleanup();
        }
      });
    }
  );

  it.each(['afterCommit', 'afterSwap'] as const)(
    'finishes an import disconnected %s and resumes that same checkpoint on restart', async boundary => {
    await experiment(async fixture => {
      let reached!: () => void;
      let release!: () => void;
      const committed = new Promise<void>(done => { reached = done; });
      const released = new Promise<void>(done => { release = done; });
      const originalCommit = CheckpointPersistenceClient.prototype.commitImport;
      const originalPublish = BackgroundOutputPump.prototype.publishPreparedImport;
      let durable: Awaited<ReturnType<typeof originalCommit>> | undefined;
      const commit = vi.spyOn(CheckpointPersistenceClient.prototype, 'commitImport')
        .mockImplementationOnce(async function(this: CheckpointPersistenceClient, ...args) {
          durable = await originalCommit.apply(this, args);
          if (boundary === 'afterCommit') { reached(); await released; }
          return durable;
        });
      const publish = boundary === 'afterSwap'
        ? vi.spyOn(BackgroundOutputPump.prototype, 'publishPreparedImport')
          .mockImplementationOnce(async function(this: BackgroundOutputPump, ...args) {
            const result = await originalPublish.apply(this, args);
            reached();
            await released;
            return result;
          })
        : undefined;
      const observation = observeDisconnect(fixture.server);
      const cleanup = testCleanup(() => {
        release();
        commit.mockRestore();
        publish?.mockRestore();
        observation.restore();
      });
      const body = legacyPopulation();
      const sourceHash = createHash('sha256').update(body).digest('hex');
      try {
        const client = request(`http://127.0.0.1:${fixture.server.port}/api/import/archive`, {
          method: 'POST', headers: { 'Content-Length': body.byteLength, 'Content-Type': 'application/json' }
        });
        fixture.requests.add(client);
        client.on('error', () => {});
        client.end(body);
        await bounded(committed, `import never reached ${boundary}`);
        expect(durable!.descriptor.runId).not.toBe(fixture.identity['runId']);
        client.destroy();
        expect(await bounded(observation.closed, 'server never observed disconnect')).toBe(true);
        release();
        await noTransferScratch(fixture.managedDirectory);
        const published = await health(fixture.server);
        expect(published).toMatchObject({ ok: true, runId: durable!.descriptor.runId,
          startupCheckpointId: durable!.checkpointId });
        expect(BigInt(`0x${published['worldEpoch'] as string}`))
          .toBeGreaterThan(BigInt(`0x${fixture.identity['worldEpoch'] as string}`));
        const database = new Database(fixture.databasePath, { readonly: true });
        try {
          expect(database.prepare(`SELECT active.run_id, current.checkpoint_id FROM rust_active_run_v1 AS active
            JOIN rust_checkpoint_v3_current AS current ON current.run_id = active.run_id`).get())
            .toEqual({ run_id: durable!.descriptor.runId, checkpoint_id: durable!.checkpointId });
        } finally { database.close(); }
        expect(await files(fixture.managedDirectory)).toEqual(expect.arrayContaining(fixture.files));
        expect(createHash('sha256').update(body).digest('hex')).toBe(sourceHash);
        await advancing(fixture);
        await fixture.server.close();
        fixture.server = await startRustServer({ ...DEFAULT_CONFIG, host: '127.0.0.1', port: 0,
          resume: 'latest', dbPath: fixture.databasePath });
        expect(fixture.server.startupFault).toBeUndefined();
        expect(await health(fixture.server)).toMatchObject({ ok: true, runId: durable!.descriptor.runId,
          startupCheckpointId: durable!.checkpointId });
        const exported = await fetch(`http://127.0.0.1:${fixture.server.port}/api/export/latest`);
        expect(exported.status).toBe(200);
        expect(exported.headers.get('x-slither-checkpoint-id')).toBe(durable!.checkpointId);
        expect((await exported.arrayBuffer()).byteLength).toBeGreaterThan(1024);
        await noTransferScratch(fixture.managedDirectory);
      } finally {
        cleanup();
      }
    });
  });

  it('cancels an upload disconnected during disk admission before opening its spool', async () => {
    await experiment(async fixture => {
      const entered = Promise.withResolvers<void>();
      const resume = Promise.withResolvers<void>();
      const originalAdmit = diskAdmission.admitDiskOperation;
      const admission = vi.spyOn(diskAdmission, 'admitDiskOperation').mockImplementation(async (directory, operation) => {
        const result = await originalAdmit(directory, operation);
        if (operation.operation === 'import') {
          entered.resolve();
          await resume.promise;
        }
        return result;
      });
      const spool = vi.spyOn(archiveUpload, 'spoolArchiveUpload');
      const preparation = vi.spyOn(BackgroundOutputPump.prototype, 'stagePreparedImport');
      const commit = vi.spyOn(CheckpointPersistenceClient.prototype, 'commitImport');
      const disconnected = raw(fixture, ['Content-Length: 100'], Buffer.from('incomplete'), true);
      try {
        await bounded(entered.promise, 'upload did not reach disk admission');
        expect(await disconnected).not.toContain('200 OK');
        const busy = await fetch(`http://127.0.0.1:${fixture.server.port}/api/export/latest`);
        expect(busy.status).toBe(409);
        await busy.arrayBuffer();
        resume.resolve();
        await exportAfterDisconnect(fixture);
        expect(spool).not.toHaveBeenCalled();
        // The successful export retry owns one reservation; the disconnected import owned none.
        expect(preparation).toHaveBeenCalledOnce();
        expect(commit).not.toHaveBeenCalled();
        await preserved(fixture);
        await advancing(fixture);
      } finally {
        resume.resolve();
        try { await disconnected; }
        finally { admission.mockRestore(); spool.mockRestore(); preparation.mockRestore(); commit.mockRestore(); }
      }
    });
  }, 20_000);
}, ARCHIVE_FIXTURE_TIMEOUT_MS);
