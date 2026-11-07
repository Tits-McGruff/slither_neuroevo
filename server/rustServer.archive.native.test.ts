import Database from 'better-sqlite3';
import { createHash } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { mkdtemp,readdir,readFile,rm,stat,writeFile } from 'node:fs/promises';
import { request as httpRequest,Server,type IncomingMessage,type ServerResponse } from 'node:http';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { join,resolve } from 'node:path';
import { expect,it,vi } from 'vitest';
import WebSocket from 'ws';
import { BackgroundOutputPump } from './rustEngine/backgroundOutput.ts';
import type { ExperimentalRunningAuthorityNativeHandle,RustArchiveWorkProgress } from './rustEngine/backgroundRuntime.ts';
import { CheckpointPersistenceClient } from './rustEngine/checkpointPersistenceClient.ts';
import { startRustServer } from './rustServer.ts';
import { archiveFileSha256,connect,frameDirection,healthUntil,selectPhysical,until,type Peer } from './rustServerFixture.ts';
import { describeNetworkSuite } from './test/networkSuites.ts';
import { RUST_TEST_CONFIG as DEFAULT_CONFIG,rustWorkersForTest } from './test/rustConfig.ts';

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

  it.each([false, true])('streams, imports, and atomically activates one exact Rust save (phase trace=%s)', async trace => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-export-server-'));
    const dbPath = join(root, 'experiment.sqlite');
    const managedDirectory = `${dbPath}.checkpoints`;
    const server = await startRustServer({
      ...DEFAULT_CONFIG, port: 0, resume: 'fresh', seed: 41, dbPath
    });
    let target: Awaited<ReturnType<typeof startRustServer>> | undefined;
    const peers: Peer[] = [];
    try {
      vi.stubEnv('SLITHER_TRACE_ARCHIVE_PHASES', trace ? '1' : '0');
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
        archiveWork: RustArchiveWorkProgress;
      };
      expect(afterExport.archiveWork).toMatchObject({ kind: 'export', started: true, finished: true });
      expect(BigInt(`0x${afterExport.archiveWork.completedBytes}`)).toBeGreaterThan(0n);
      if (trace) {
        const timings = afterExport.archiveWork.phaseTrace!;
        expect(timings.truncated).toBe(false);
        expect(timings.intervals.length).toBeLessThanOrEqual(4096);
        expect(timings.rssSamplerStarted).toBe(true);
        expect(BigInt(`0x${timings.requestedRssSampleIntervalMicros}`)).toBe(2000n);
        for (const interval of timings.intervals) {
          expect(BigInt(`0x${interval.startRssBytes}`)).toBeGreaterThan(0n);
          expect(BigInt(`0x${interval.finishRssBytes}`)).toBeGreaterThan(0n);
          expect(BigInt(`0x${interval.rssSamples}`)).toBeGreaterThanOrEqual(2n);
          expect(BigInt(`0x${interval.sampledPeakRssBytes}`)).toBeGreaterThanOrEqual(BigInt(`0x${interval.startRssBytes}`));
          expect(BigInt(`0x${interval.sampledPeakRssBytes}`)).toBeGreaterThanOrEqual(BigInt(`0x${interval.finishRssBytes}`));
        }
        expect(timings.intervals.every(interval => interval.finishedMicros !== undefined)).toBe(true);
        const source = timings.intervals.find(interval => interval.phase === 'export-source-population')!;
        const write = timings.intervals.find(interval => interval.phase === 'temporary-file-write')!;
        const validation = timings.intervals.find(interval => interval.phase === 'validation')!;
        expect(BigInt(`0x${source.finishedMicros}`)).toBeLessThanOrEqual(BigInt(`0x${write.startedMicros}`));
        expect(BigInt(`0x${write.finishedMicros}`)).toBeLessThanOrEqual(BigInt(`0x${validation.startedMicros}`));
      } else expect(afterExport.archiveWork.phaseTrace).toBeUndefined();

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
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
        rejoinToken: viewer.rejoinToken }));
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
        archiveWork: RustArchiveWorkProgress;
      };
      expect(importedHealth.archiveWork).toMatchObject({ kind: 'import', started: true, finished: true });
      expect(BigInt(`0x${importedHealth.archiveWork.completedBytes}`)).toBeGreaterThan(0n);
      if (trace) {
        const timings = importedHealth.archiveWork.phaseTrace!;
        expect(timings.truncated).toBe(false);
        expect(timings.rssSamplerStarted).toBe(true);
        for (const interval of timings.intervals) {
          expect(BigInt(`0x${interval.startRssBytes}`)).toBeGreaterThan(0n);
          expect(BigInt(`0x${interval.finishRssBytes}`)).toBeGreaterThan(0n);
          expect(BigInt(`0x${interval.rssSamples}`)).toBeGreaterThanOrEqual(2n);
        }
        expect(timings.intervals.every(interval => interval.finishedMicros !== undefined)).toBe(true);
        expect(new Set(timings.intervals.map(interval => interval.phase))).toEqual(new Set([
          'import', 'validation', 'temporary-file-write', 'numeric-decode', 'checkpoint-restore',
          'managed-publication', 'candidate-construction'
        ]));
      } else expect(importedHealth.archiveWork.phaseTrace).toBeUndefined();
      await until(viewer, () => viewer.packets.some(packet => packet['type'] === 'stateReplaced'));
      expect(viewer.socket.readyState).toBe(WebSocket.OPEN);
      expect(viewer.packets.find(packet => packet['type'] === 'stateReplaced')).toMatchObject({
        reason: 'import', checkpointId: health.startupCheckpointId,
        welcome: { runId: health.runId, worldSeed: 41 }
      });
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
        rejoinToken: viewer.rejoinToken }));
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
      vi.unstubAllEnvs();
    }
  }, 30_000);

  it.each(['before headers', 'after headers'] as const)(
    'releases a cancelled export %s and admits a fresh direct request', async boundary => {
      const root = await mkdtemp(join(tmpdir(), 'slither-rust-export-cancel-'));
      const dbPath = join(root, 'experiment.sqlite');
      const managedDirectory = `${dbPath}.checkpoints`;
      const server = await startRustServer({ ...DEFAULT_CONFIG, port: 0,
        resume: 'fresh', seed: 41, dbPath });
      /** Hold only the real selected lease so cancellation precedes preparation deterministically. */
      const gate = Promise.withResolvers<void>();
      /** Observe the actual persistence lease rather than guessing from request timing. */
      const selected = Promise.withResolvers<Awaited<ReturnType<CheckpointPersistenceClient['acquireCurrentExportLease']>>>();
      const originalAcquire = CheckpointPersistenceClient.prototype.acquireCurrentExportLease;
      const acquire = vi.spyOn(CheckpointPersistenceClient.prototype, 'acquireCurrentExportLease')
        .mockImplementationOnce(async function(this: CheckpointPersistenceClient) {
          const lease = await originalAcquire.call(this);
          selected.resolve(lease);
          if (boundary === 'before headers') await gate.promise;
          return lease;
        });
      const release = vi.spyOn(CheckpointPersistenceClient.prototype, 'releaseExportLease');
      /** Observe real server termination so an already-complete response cannot masquerade as cancellation. */
      let cancelledResponseClosedBeforeFinish: boolean | undefined;
      let observedExport = false;
      const originalEmit = Server.prototype.emit;
      const dispatch = vi.spyOn(Server.prototype, 'emit').mockImplementation(function(
        this: Server, event: string | symbol, ...args: unknown[]
      ): boolean {
        if (event === 'request') {
          const incoming = args[0] as IncomingMessage;
          const response = args[1] as ServerResponse;
          if (!observedExport && incoming.socket.localPort === server.port &&
              incoming.url === '/api/export/latest') {
            observedExport = true;
            response.once('close', () => {
              cancelledResponseClosedBeforeFinish = !response.writableFinished;
            });
          }
        }
        return Reflect.apply(originalEmit, this, [event, ...args]) as boolean;
      });
      const request = httpRequest(`http://127.0.0.1:${server.port}/api/export/latest`);
      /** Client close is observed independently of server cleanup. */
      const closed = new Promise<void>(done => request.once('close', done));
      let receivedHeaders = false;
      request.on('error', () => { /* Destroying the cancelled client may report ECONNRESET. */ });
      request.on('response', response => {
        receivedHeaders = true;
        expect(response.statusCode).toBe(200);
        response.destroy();
      });
      /** Snapshot every durable metadata row; export must not change the experiment. */
      const metadata = (): unknown => {
        const database = new Database(dbPath, { readonly: true });
        try {
          const tables = database.prepare(`SELECT name FROM sqlite_master
            WHERE type = 'table' AND name LIKE 'rust_%' ORDER BY name`).all() as Array<{ name: string }>;
          return tables.map(({ name }) => ({ name, rows: database.prepare(
            `SELECT * FROM "${name.replaceAll('"', '""')}" ORDER BY rowid`
          ).all() }));
        } finally { database.close(); }
      };
      /** Check exact source file bytes as well as filenames and SQLite references. */
      const managedHashes = async (files: readonly string[]): Promise<string[]> => Promise.all(
        files.map(async name => createHash('sha256').update(await readFile(join(managedDirectory, name))).digest('hex'))
      );
      try {
        const before = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as {
          runId: string; generation: string; startupCheckpointId: string; configHash: string; seed: number;
        };
        const beforeMetadata = metadata();
        const beforeFiles = (await readdir(managedDirectory)).sort();
        const beforeHashes = await managedHashes(beforeFiles);
        request.end();
        const lease = await selected.promise;
        expect(lease.descriptor.logicalRootSha256).toBe(before.startupCheckpointId);
        if (boundary === 'before headers') request.destroy();
        await closed;
        expect(receivedHeaders).toBe(boundary === 'after headers');
        gate.resolve();

        const deadline = performance.now() + 2000;
        let files: string[];
        do {
          files = (await readdir(managedDirectory)).sort();
          if (release.mock.calls.length === 1 && JSON.stringify(files) === JSON.stringify(beforeFiles)) break;
          await new Promise<void>(done => setTimeout(done, 10));
        } while (performance.now() < deadline);
        expect(files!).toEqual(beforeFiles);
        expect(cancelledResponseClosedBeforeFinish).toBe(true);
        expect(release).toHaveBeenCalledExactlyOnceWith(lease.operationId);
        await release.mock.results[0]!.value;
        expect(metadata()).toEqual(beforeMetadata);
        expect(await managedHashes(beforeFiles)).toEqual(beforeHashes);
        expect(await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json()).toMatchObject({
          ok: true, runId: before.runId, generation: before.generation,
          startupCheckpointId: before.startupCheckpointId, configHash: before.configHash, seed: before.seed
        });

        const fresh = await fetch(`http://127.0.0.1:${server.port}/api/export/latest`);
        expect(fresh.status).toBe(200);
        expect(fresh.headers.get('x-slither-checkpoint-id')).toBe(before.startupCheckpointId);
        expect(fresh.headers.get('content-disposition')).toContain('-gen-1-v1.slither-save');
        expect((await fresh.arrayBuffer()).byteLength).toBe(Number(fresh.headers.get('content-length')));
        const retryDeadline = performance.now() + 2000;
        do {
          files = (await readdir(managedDirectory)).sort();
          if (release.mock.calls.length === 2 && JSON.stringify(files) === JSON.stringify(beforeFiles)) break;
          await new Promise<void>(done => setTimeout(done, 10));
        } while (performance.now() < retryDeadline);
        expect(files!).toEqual(beforeFiles);
        expect(acquire).toHaveBeenCalledTimes(2);
        expect(release).toHaveBeenCalledTimes(2);
        await release.mock.results[1]!.value;
        expect(metadata()).toEqual(beforeMetadata);
        expect(await managedHashes(beforeFiles)).toEqual(beforeHashes);
      } finally {
        request.destroy();
        gate.resolve();
        try { await server.close(); }
        finally {
          dispatch.mockRestore();
          acquire.mockRestore();
          release.mockRestore();
          await rm(root, { recursive: true, force: true });
        }
      }
    }, 20_000
  );

  it.runIf(process.env['SLITHER_FULL_DOWNLOAD_TIMEOUT_TEST'] === '1')(
    'releases a connected large download after the full no-progress deadline', async () => {
      const archivePath = process.env['SLITHER_DOWNLOAD_TIMEOUT_ARCHIVE'];
      if (!archivePath) throw new Error('SLITHER_DOWNLOAD_TIMEOUT_ARCHIVE must name a verified save over 50 MiB');
      const archive = await stat(archivePath);
      expect(archive.isFile()).toBe(true);
      expect(archive.size).toBeGreaterThan(50 * 1024 * 1024);
      const originalSha256 = await archiveFileSha256(archivePath);
      const root = await mkdtemp(join(tmpdir(), 'slither-rust-stalled-export-'));
      const dbPath = join(root, 'experiment.sqlite');
      const managedDirectory = `${dbPath}.checkpoints`;
      const server = await startRustServer({ ...DEFAULT_CONFIG, port: 0,
        resume: 'fresh', seed: 41, dbPath, rustCalculationWorkers: rustWorkersForTest(6) });
      /** Observe server-side close even while the client refuses to drain buffered bytes. */
      const terminal = Promise.withResolvers<{ finished: boolean; elapsedSinceProgressMs: number }>();
      let observedExport = false;
      let lastProgressAt = 0;
      const originalEmit = Server.prototype.emit;
      const dispatch = vi.spyOn(Server.prototype, 'emit').mockImplementation(function(
        this: Server, event: string | symbol, ...args: unknown[]
      ): boolean {
        if (event === 'request') {
          const incoming = args[0] as IncomingMessage;
          const response = args[1] as ServerResponse;
          if (!observedExport && incoming.socket.localPort === server.port &&
              incoming.url === '/api/export/latest') {
            observedExport = true;
            const originalWrite = response.write;
            response.write = function(...values: unknown[]): boolean {
              lastProgressAt = performance.now();
              return Reflect.apply(originalWrite, this, values) as boolean;
            };
            response.on('drain', () => { lastProgressAt = performance.now(); });
            response.once('close', () => terminal.resolve({ finished: response.writableFinished,
              elapsedSinceProgressMs: performance.now() - lastProgressAt }));
          }
        }
        return Reflect.apply(originalEmit, this, [event, ...args]) as boolean;
      });
      const release = vi.spyOn(CheckpointPersistenceClient.prototype, 'releaseExportLease');
      const request = httpRequest(`http://127.0.0.1:${server.port}/api/export/latest`);
      request.on('error', () => { /* The timed-out server may reset the blocked client. */ });
      /** Original headers arrive before deliberately stopping all client body reads. */
      const headers = Promise.withResolvers<{ checkpointId: string; contentLength: number }>();
      request.once('response', response => {
        response.pause();
        response.on('error', () => { /* Expected when the server ends an idle download. */ });
        headers.resolve({ checkpointId: String(response.headers['x-slither-checkpoint-id']),
          contentLength: Number(response.headers['content-length']) });
      });
      try {
        const imported = await fetch(`http://127.0.0.1:${server.port}/api/import/archive`, {
          method: 'POST', headers: { 'Content-Type': 'application/vnd.slither-neuroevo.save',
            'Content-Length': String(archive.size) },
          body: createReadStream(archivePath) as unknown as BodyInit, duplex: 'half'
        } as RequestInit & { duplex: 'half' });
        expect(imported.status).toBe(200);
        const receipt = await imported.json() as { ok: boolean; runId: string; checkpointId: string };
        expect(receipt.ok).toBe(true);
        const before = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as {
          runId: string; configHash: string; completedStep: string;
        };
        request.end();
        const advertised = await headers.promise;
        expect(advertised.checkpointId).toBe(receipt.checkpointId);
        expect(advertised.contentLength).toBeGreaterThan(50 * 1024 * 1024);
        const result = await terminal.promise;
        expect(result.finished).toBe(false);
        expect(result.elapsedSinceProgressMs).toBeGreaterThanOrEqual(59_000);
        expect(result.elapsedSinceProgressMs).toBeLessThan(70_000);
        // Keep the non-reading client connected until the actual server timeout is observed.
        request.destroy();
        const cleanupDeadline = performance.now() + 2000;
        let leftovers: string[];
        do {
          leftovers = (await readdir(managedDirectory)).filter(name =>
            name.includes('export-inventory') || name.includes('slither-save') || name.includes('export-hof'));
          if (release.mock.calls.length === 1 && leftovers.length === 0) break;
          await new Promise<void>(done => setTimeout(done, 10));
        } while (performance.now() < cleanupDeadline);
        expect(leftovers!).toEqual([]);
        expect(release).toHaveBeenCalledTimes(1);
        await release.mock.results[0]!.value;
        const after = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as {
          completedStep: string; generation: string;
        };
        expect(after).toMatchObject({ ok: true, runId: before.runId, configHash: before.configHash });
        expect(BigInt(`0x${after.completedStep}`)).toBeGreaterThan(BigInt(`0x${before.completedStep}`));
        expect(await archiveFileSha256(archivePath)).toBe(originalSha256);
        console.log(JSON.stringify({ idleDownload: result, advertisedBytes: advertised.contentLength,
          generationAfterTimeout: after.generation, originalArchiveSha256: originalSha256 }));
      } finally {
        request.destroy();
        try { await server.close(); }
        finally {
          dispatch.mockRestore();
          release.mockRestore();
          await rm(root, { recursive: true, force: true });
        }
      }
    }, 90_000
  );

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

  it('imports evolved Hall-of-Fame objects and reuses them on an exact retry', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-import-evolved-hof-'));
    const sourcePath = join(root, 'source.sqlite');
    const targetPath = join(root, 'target.sqlite');
    const source = await startRustServer({ ...DEFAULT_CONFIG, port: 0,
      resume: 'fresh', seed: 41, dbPath: sourcePath });
    let target: Awaited<ReturnType<typeof startRustServer>> | undefined;
    let viewer: Peer | undefined;
    let targetViewer: Peer | undefined;
    let cleanupSpy: ReturnType<typeof vi.spyOn> | undefined;
    let stageSpy: ReturnType<typeof vi.spyOn> | undefined;
    let releaseSpy: ReturnType<typeof vi.spyOn> | undefined;
    let orphanClient: CheckpointPersistenceClient | undefined;
    let cleanupLease: Awaited<ReturnType<CheckpointPersistenceClient['acquireCurrentExportLease']>> | undefined;
    try {
      let initialExportReleased = false;
      const releaseExportLease = CheckpointPersistenceClient.prototype.releaseExportLease;
      // Download completion precedes lease release; Reset must wait for the real release reply.
      releaseSpy = vi.spyOn(CheckpointPersistenceClient.prototype, 'releaseExportLease')
        .mockImplementationOnce(async function(this: CheckpointPersistenceClient, operationId) {
          await releaseExportLease.call(this, operationId);
          initialExportReleased = true;
        });
      const initialExport = await fetch(`http://127.0.0.1:${source.port}/api/export/latest`);
      expect(initialExport.status).toBe(200);
      const initialCheckpointId = initialExport.headers.get('x-slither-checkpoint-id');
      const initialArchive = Buffer.from(await initialExport.arrayBuffer());
      viewer = await connect(source.port, 'ui');
      await until(viewer, () => viewer!.packets.some(packet => packet['type'] === 'welcome'));
      await until(viewer, () => initialExportReleased);
      releaseSpy.mockRestore();
      releaseSpy = undefined;
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
        rejoinToken: viewer.rejoinToken }));
      viewer.socket.send(JSON.stringify({ type: 'reset', settings: { snakeCount: 12, simSpeed: 12 },
        updates: [{ path: 'generationSeconds', value: 8 }, { path: 'baselineBots.count', value: 0 }] }));
      await until(viewer, () => viewer!.packets.some(packet =>
        packet['type'] === 'stateReplaced' && packet['reason'] === 'reset'));
      await healthUntil(source.port, health => BigInt(`0x${health['generation'] as string}`) >= 2n);
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
        rejoinToken: viewer.rejoinToken }));
      viewer.socket.send(JSON.stringify({ type: 'settings', requestId: 'hold-hof-fixture',
        updates: [{ path: 'simSpeed', value: 0.1 }] }));
      await until(viewer, () => viewer!.packets.some(packet =>
        packet['type'] === 'settingsApplied' && packet['requestId'] === 'hold-hof-fixture'));
      const exported = await fetch(`http://127.0.0.1:${source.port}/api/export/latest`);
      expect(exported.status).toBe(200);
      const checkpointId = exported.headers.get('x-slither-checkpoint-id');
      const archive = Buffer.from(await exported.arrayBuffer());
      const sourceDatabase = new Database(sourcePath, { readonly: true });
      let expectedElites: Array<{ relative_filename: string }>;
      try {
        expectedElites = sourceDatabase.prepare(`SELECT DISTINCT weights.relative_filename
          FROM rust_hall_of_fame_weights_v1 AS weights
          JOIN rust_hall_of_fame_v1 AS hall ON hall.weights_sha256 = weights.logical_sha256`).all() as
          Array<{ relative_filename: string }>;
      } finally { sourceDatabase.close(); }
      expect(expectedElites.length).toBeGreaterThan(0);
      target = await startRustServer({ ...DEFAULT_CONFIG, port: 0,
        resume: 'fresh', seed: 42, dbPath: targetPath });
      const managedDirectory = `${targetPath}.checkpoints`;
      /** Capture every durable metadata table and managed file in this small fixture. */
      const snapshot = async (): Promise<unknown> => {
        const database = new Database(targetPath, { readonly: true });
        try {
          const tables = database.prepare(`SELECT name FROM sqlite_master
            WHERE type = 'table' AND name LIKE 'rust_%' ORDER BY name`).all() as Array<{ name: string }>;
          const names = (await readdir(managedDirectory)).sort();
          return { tables: tables.map(({ name }) => ({ name,
            rows: database.prepare(`SELECT * FROM "${name.replaceAll('"', '""')}" ORDER BY rowid`).all() })),
          files: await Promise.all(names.map(async name => ({ name,
            hash: createHash('sha256').update(await readFile(join(managedDirectory, name))).digest('hex') }))) };
        } finally { database.close(); }
      };
      const failureDatabase = new Database(targetPath);
      try {
        failureDatabase.exec(`CREATE TRIGGER reject_import_activation
          BEFORE INSERT ON rust_active_run_v1 BEGIN
            SELECT RAISE(ABORT, 'injected actual import transaction rejection');
          END`);
      } finally { failureDatabase.close(); }
      const beforeFailure = await snapshot();
      const previousAuthority = await (await fetch(`http://127.0.0.1:${target.port}/api/health`)).json() as {
        runId: string; worldEpoch: string; generation: string;
      };
      const originalCleanup = CheckpointPersistenceClient.prototype.reclaimManagedOrphans;
      /** Observe real permanent files and a held world across the actual reference scan. */
      cleanupSpy = vi.spyOn(CheckpointPersistenceClient.prototype, 'reclaimManagedOrphans')
        .mockImplementationOnce(async function(this: CheckpointPersistenceClient) {
          const files = await readdir(managedDirectory);
          expect(files).toEqual(expect.arrayContaining([`${checkpointId}.checkpoint-v3`,
            ...expectedElites.map(elite => elite.relative_filename)]));
          const held = await (await fetch(`http://127.0.0.1:${target!.port}/api/health`)).json() as {
            completedStep: string; worldEpoch: string;
          };
          const result = await originalCleanup.call(this);
          expect(result).toMatchObject({ completed: true, deletedCheckpointCount: '0000000000000001' });
          expect(BigInt(`0x${result.deletedHallOfFameCount}`)).toBe(BigInt(expectedElites.length));
          expect(await (await fetch(`http://127.0.0.1:${target!.port}/api/health`)).json()).toMatchObject({
            completedStep: held.completedStep, worldEpoch: held.worldEpoch
          });
          return result;
        });
      const transactionRejected = await fetch(`http://127.0.0.1:${target.port}/api/import/archive`, {
        method: 'POST', headers: { 'Content-Type': 'application/vnd.slither-neuroevo.save' }, body: archive
      });
      expect(transactionRejected.status).toBe(400);
      expect(await transactionRejected.json()).toMatchObject({ ok: false,
        message: expect.stringContaining('injected actual import transaction rejection') });
      expect(cleanupSpy).toHaveBeenCalledOnce();
      cleanupSpy.mockRestore();
      expect(await snapshot()).toEqual(beforeFailure);
      expect(await (await fetch(`http://127.0.0.1:${target.port}/api/health`)).json()).toMatchObject({
        ok: true, runId: previousAuthority.runId, worldEpoch: previousAuthority.worldEpoch,
        generation: previousAuthority.generation
      });
      const recoveryDatabase = new Database(targetPath);
      try { recoveryDatabase.exec('DROP TRIGGER reject_import_activation'); }
      finally { recoveryDatabase.close(); }
      for (let attempt = 0; attempt < 2; attempt++) {
        const imported = await fetch(`http://127.0.0.1:${target.port}/api/import/archive`, {
          method: 'POST', headers: { 'Content-Type': 'application/vnd.slither-neuroevo.save' }, body: archive
        });
        expect(imported.status).toBe(200);
        expect(await imported.json()).toMatchObject({ ok: true, checkpointId });
        targetViewer ??= await connect(target.port, 'ui');
        targetViewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
          rejoinToken: targetViewer.rejoinToken }));
        const requestId = `hold-imported-hof-${attempt}`;
        targetViewer.socket.send(JSON.stringify({ type: 'settings', requestId,
          updates: [{ path: 'simSpeed', value: 0.1 }] }));
        await until(targetViewer, () => targetViewer!.packets.some(packet =>
          packet['type'] === 'settingsApplied' && packet['requestId'] === requestId));
        for (const elite of expectedElites) {
          expect(await readFile(join(`${targetPath}.checkpoints`, elite.relative_filename)))
            .toEqual(await readFile(join(`${sourcePath}.checkpoints`, elite.relative_filename)));
        }
        const names = await readdir(`${targetPath}.checkpoints`);
        expect(names.filter(name => name.includes('import-') || name.endsWith('.partial'))).toEqual([]);
      }
      const targetDatabase = new Database(targetPath, { readonly: true });
      try {
        expect(targetDatabase.prepare('SELECT count(*) AS count FROM rust_hall_of_fame_v1').get())
          .toEqual({ count: expectedElites.length });
      } finally { targetDatabase.close(); }
      let retryExportReleased = false;
      // Observe the actual cleanup reply before hashing immutable files; body
      // delivery alone does not join deletion of the export inventory.
      releaseSpy = vi.spyOn(CheckpointPersistenceClient.prototype, 'releaseExportLease')
        .mockImplementationOnce(async function(this: CheckpointPersistenceClient, operationId) {
          await releaseExportLease.call(this, operationId);
          retryExportReleased = true;
        });
      const retryExport = await fetch(`http://127.0.0.1:${target.port}/api/export/latest`);
      expect(retryExport.status).toBe(200);
      expect(retryExport.headers.get('x-slither-checkpoint-id')).toBe(checkpointId);
      expect(Buffer.from(await retryExport.arrayBuffer())).toEqual(archive);
      await until(targetViewer!, () => retryExportReleased);
      releaseSpy.mockRestore();
      releaseSpy = undefined;

      const winnersResponse = await fetch(`http://127.0.0.1:${target.port}/api/hof`);
      expect(winnersResponse.status).toBe(200);
      const winners = await winnersResponse.json() as { hof: Array<{ entryId: string }> };
      expect(winners.hof.length).toBe(expectedElites.length);
      const beforeResurrection = await snapshot();
      const resurrectedResponse = await fetch(`http://127.0.0.1:${target.port}/api/resurrect`, {
        method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ entryId: winners.hof[0]!.entryId })
      });
      expect(resurrectedResponse.status).toBe(200);
      const resurrected = await resurrectedResponse.json() as { ok: boolean; snakeId: number };
      expect(resurrected).toMatchObject({ ok: true, snakeId: expect.any(Number) });
      await until(targetViewer!, () => frameDirection(targetViewer!.latestFrame, resurrected.snakeId) !== undefined);
      const selectedWinner = await selectPhysical(targetViewer!, 'retained-winner-origin', {snakeId: resurrected.snakeId});
      expect(selectedWinner.retainedOrigin).toEqual({kind: 'hall-of-fame', entryId: winners.hof[0]!.entryId,
        weightsSha256: expect.stringMatching(/^[0-9a-f]{64}$/u)});
      expect(await snapshot()).toEqual(beforeResurrection);

      // A same-name corrupt elite must be preserved as evidence and reject the
      // import before any new inventory or immutable object becomes permanent.
      const elitePath = join(managedDirectory, expectedElites[0]!.relative_filename);
      const originalElite = await readFile(elitePath);
      const damaged = Buffer.from(originalElite);
      damaged[0] = damaged[0]! ^ 1;
      await writeFile(elitePath, damaged);
      const before = await snapshot();
      const beforeHealth = await (await fetch(`http://127.0.0.1:${target.port}/api/health`)).json() as {
        runId: string; worldEpoch: string; generation: string;
      };
      cleanupSpy = vi.spyOn(CheckpointPersistenceClient.prototype, 'reclaimManagedOrphans');
      const rejected = await fetch(`http://127.0.0.1:${target.port}/api/import/archive`, {
        method: 'POST', headers: { 'Content-Type': 'application/vnd.slither-neuroevo.save' }, body: archive
      });
      expect(rejected.status).toBe(400);
      expect(await rejected.json()).toMatchObject({ ok: false,
        message: expect.stringContaining('existing immutable object differs') });
      expect(cleanupSpy).toHaveBeenCalledOnce();
      expect(await cleanupSpy.mock.results[0]!.value).toMatchObject({ completed: true });
      orphanClient = cleanupSpy.mock.contexts[0] as CheckpointPersistenceClient;
      cleanupSpy.mockRestore();
      expect(await snapshot()).toEqual(before);
      expect(await (await fetch(`http://127.0.0.1:${target.port}/api/health`)).json()).toMatchObject({
        ok: true, runId: beforeHealth.runId, worldEpoch: beforeHealth.worldEpoch, generation: beforeHealth.generation
      });
      await writeFile(elitePath, originalElite);
      const recovered = await fetch(`http://127.0.0.1:${target.port}/api/export/latest`);
      expect(recovered.status).toBe(200);
      expect(Buffer.from(await recovered.arrayBuffer())).toEqual(archive);
      const exportCleanupDeadline = performance.now() + 2000;
      let exportScratch: string[];
      do {
        exportScratch = (await readdir(managedDirectory)).filter(name =>
          name.endsWith('.slither-save.ready') || name.endsWith('.export-inventory-v1'));
        if (!exportScratch.length) break;
        await new Promise<void>(done => setTimeout(done, 10));
      } while (performance.now() < exportCleanupDeadline);
      expect(exportScratch!).toEqual([]);
      // The held rejection already scanned. Its next durable boundary must not repeat cleanup.
      const beforeSettlement = await (await fetch(`http://127.0.0.1:${target.port}/api/health`)).json() as {
        generation: string;
      };
      cleanupSpy = vi.spyOn(CheckpointPersistenceClient.prototype, 'reclaimManagedOrphans');
      targetViewer!.socket.send(JSON.stringify({ type: 'settings', requestId: 'settle-corruption-cleanup',
        updates: [{ path: 'simSpeed', value: 12 }] }));
      await healthUntil(target.port, health =>
        BigInt(`0x${health['generation'] as string}`) > BigInt(`0x${beforeSettlement.generation}`));
      expect(cleanupSpy).not.toHaveBeenCalled();
      cleanupSpy.mockRestore();
      targetViewer!.socket.send(JSON.stringify({ type: 'settings', requestId: 'hold-deferred-cleanup-fixture',
        updates: [{ path: 'simSpeed', value: 0.1 }] }));
      await until(targetViewer!, () => targetViewer!.packets.some(packet =>
        packet['type'] === 'settingsApplied' && packet['requestId'] === 'hold-deferred-cleanup-fixture'));

      // A real export lease defers reclamation after files were published but before import commit.
      const binding = createRequire(import.meta.url)(resolve('native/index.js')) as {
        ExperimentalRunningAuthority: { prototype: ExperimentalRunningAuthorityNativeHandle };
      };
      const originalPrepare = binding.ExperimentalRunningAuthority.prototype.prepareImportArchive;
      cleanupSpy = vi.spyOn(CheckpointPersistenceClient.prototype, 'reclaimManagedOrphans');
      stageSpy = vi.spyOn(binding.ExperimentalRunningAuthority.prototype, 'prepareImportArchive')
        .mockImplementationOnce(async function(this: ExperimentalRunningAuthorityNativeHandle, ...args) {
          await originalPrepare.apply(this, args);
          cleanupLease = await orphanClient!.acquireCurrentExportLease();
          throw new Error('injected post-preparation rejection');
        });
      const unstaged = await fetch(`http://127.0.0.1:${target.port}/api/import/archive`, {
        method: 'POST', headers: { 'Content-Type': 'application/vnd.slither-neuroevo.save' }, body: initialArchive
      });
      expect(unstaged.status).toBe(400);
      expect(await unstaged.json()).toMatchObject({ ok: false, message: 'injected post-preparation rejection' });
      stageSpy.mockRestore();
      expect(cleanupSpy).toHaveBeenCalledOnce();
      expect(await cleanupSpy.mock.results[0]!.value).toMatchObject({ completed: false });
      cleanupSpy.mockRestore();
      const deferredFile = join(managedDirectory, `${initialCheckpointId}.checkpoint-v3`);
      expect((await stat(deferredFile)).isFile()).toBe(true);
      await orphanClient!.releaseExportLease(cleanupLease!.operationId);
      cleanupLease = undefined;
      const unchanged = await (await fetch(`http://127.0.0.1:${target.port}/api/health`)).json() as {
        runId: string; generation: string;
      };
      expect(unchanged).toMatchObject({ ok: true, runId: beforeHealth.runId });
      cleanupSpy = vi.spyOn(CheckpointPersistenceClient.prototype, 'reclaimManagedOrphans');
      targetViewer!.socket.send(JSON.stringify({ type: 'settings', requestId: 'reach-orphan-cleanup-boundary',
        updates: [{ path: 'simSpeed', value: 12 }] }));
      await healthUntil(target.port, health =>
        BigInt(`0x${health['generation'] as string}`) > BigInt(`0x${unchanged.generation}`));
      expect(cleanupSpy).toHaveBeenCalledOnce();
      expect(await cleanupSpy.mock.results[0]!.value).toMatchObject({ completed: true });
      expect(await readdir(managedDirectory)).not.toContain(`${initialCheckpointId}.checkpoint-v3`);
      cleanupSpy.mockRestore();
      // The cleanup boundary has completed. Hold its successor before archive
      // preparation so an accelerated next boundary cannot race the retry.
      targetViewer!.socket.send(JSON.stringify({ type: 'settings', requestId: 'hold-valid-import-retry',
        updates: [{ path: 'simSpeed', value: 0.1 }] }));
      await until(targetViewer!, () => targetViewer!.packets.some(packet =>
        packet['type'] === 'settingsApplied' && packet['requestId'] === 'hold-valid-import-retry'));
      const validRetry = await fetch(`http://127.0.0.1:${target.port}/api/import/archive`, {
        method: 'POST', headers: { 'Content-Type': 'application/vnd.slither-neuroevo.save' }, body: initialArchive
      });
      const retryResult = await validRetry.json();
      expect(validRetry.status, JSON.stringify(retryResult)).toBe(200);
      expect(retryResult).toMatchObject({ ok: true, checkpointId: initialCheckpointId });
    } finally {
      if (cleanupLease) await orphanClient!.releaseExportLease(cleanupLease.operationId).catch(() => {});
      releaseSpy?.mockRestore();
      stageSpy?.mockRestore();
      cleanupSpy?.mockRestore();
      targetViewer?.socket.close();
      viewer?.socket.close();
      await target?.close();
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
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
        rejoinToken: viewer.rejoinToken }));
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
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
        rejoinToken: viewer.rejoinToken }));
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
});
