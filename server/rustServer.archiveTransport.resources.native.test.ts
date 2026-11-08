/** Real HTTP archive framing and wire-limit acceptance with an unchanged prior experiment. */
import { createHash } from 'node:crypto';
import { readdir,readFile,statfs,unlink,writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { expect,it,vi } from 'vitest';
import WebSocket from 'ws';
import { buildLargeBrainGraph } from '../scripts/fixtures/largeBrainGraph.ts';
import * as archiveUpload from './rustEngine/archiveUpload.ts';
import { BackgroundOutputPump } from './rustEngine/backgroundOutput.ts';
import { CheckpointPersistenceClient } from './rustEngine/checkpointPersistenceClient.ts';
import * as diskAdmission from './rustEngine/diskAdmission.ts';
import * as freshRunSessions from './rustEngine/experimentalFreshRunSession.ts';
import type { ExperimentalRuntimeTelemetrySnapshot } from './rustEngine/runtimeTelemetry.ts';
import { advancing,ARCHIVE_FIXTURE_TIMEOUT_MS,ARCHIVE_RETRY_FIXTURE_TIMEOUT_MS,controller,experiment,files,headingChange,health,heldInput,largeBrainArchive,metadata,noTransferScratch,outcome,preserved,quota,raw,testCleanup } from './rustServerArchiveFixture.ts';
import { ARCHIVE_PREPARATION_TIMEOUT_MS,fixtureArchiveDownload } from './test/archiveDownload.ts';
import { describeNetworkSuite } from './test/networkSuites.ts';

describeNetworkSuite('Rust archive HTTP framing', () => {

  it('delivers the whole resource rejection and closes a connected unfinished upload without spooling', async () => {
    await experiment(async fixture => {
      const admission = vi.spyOn(diskAdmission, 'admitDiskOperation')
        .mockImplementation(async (_directory, request) => diskAdmission.evaluateDiskAdmission(request,
          diskAdmission.ARCHIVE_TEMP_QUOTA_BYTES, 32n * 1024n ** 3n));
      const spool = vi.spyOn(archiveUpload, 'spoolArchiveUpload');
      const cleanup = testCleanup(() => { admission.mockRestore(); spool.mockRestore(); });
      try {
        const started = performance.now();
        const response = await raw(fixture, [`Content-Length: ${fixture.archive.byteLength}`],
          fixture.archive.subarray(0, 64 * 1024));
        expect(response).toMatch(/^HTTP\/1\.1 400 /u);
        expect(performance.now() - started).toBeLessThan(2500);
        const [headers, body] = response.split('\r\n\r\n');
        const length = Number(headers!.match(/Content-Length: (\d+)/iu)?.[1]);
        expect(length).toBeGreaterThan(0);
        expect(Buffer.byteLength(body!)).toBe(length);
        expect(JSON.parse(body!)).toMatchObject({ ok: false, message: expect.stringContaining('temporary bytes, above') });
        expect(admission).toHaveBeenCalledOnce();
        expect(spool).not.toHaveBeenCalled();
        await preserved(fixture);
        await advancing(fixture);
      } finally { cleanup(); }
    });
  }, 10_000);

  it.each(([
    ['import upload', 1], ['import preparation', 2], ['export', 1]
  ] as const).flatMap(([boundary, attempt]) =>
    (['temporary quota', 'SQLite/WAL allowance', 'operating reserve'] as const)
      .map(resource => ({ boundary, attempt, resource }))))(
    'preserves the game when $resource rejects $boundary', async ({ boundary, attempt, resource }) => {
      await experiment(async fixture => {
        const originalAdmit = diskAdmission.admitDiskOperation;
        const sourceHash = createHash('sha256').update(fixture.archive).digest('hex');
        const operation = boundary === 'export' ? 'export' : 'import';
        let seen = 0;
        let refusal: diskAdmission.DiskAdmissionError | undefined;
        const spool = vi.spyOn(archiveUpload, 'spoolArchiveUpload');
        const stage = vi.spyOn(BackgroundOutputPump.prototype, 'stagePreparedImport');
        const commit = vi.spyOn(CheckpointPersistenceClient.prototype, 'commitImport');
        const release = vi.spyOn(CheckpointPersistenceClient.prototype, 'releaseExportLease');
        /** Inject only scarce-resource readings; run the production admission arithmetic and transfer path. */
        const admission = vi.spyOn(diskAdmission, 'admitDiskOperation')
          .mockImplementation(async (directory, request) => {
            if (request.operation !== operation || ++seen !== attempt) return originalAdmit(directory, request);
            expect(directory).toBe(fixture.managedDirectory);
            const actual = await diskAdmission.inspectManagedDisk(directory);
            if (boundary === 'import preparation') {
              expect(actual.tempByteCount).toBe(BigInt(fixture.archive.byteLength));
              const upload = (await readdir(directory)).filter(name => name.endsWith('.upload.ready'));
              expect(upload).toHaveLength(1);
              const retained = await files(directory);
              expect(retained.find(file => file.filename === upload[0])?.sha256).toBe(sourceHash);
              expect(request.sourceSpoolBytes).toBe(0n);
              expect(request.candidateSpoolBytes + request.finalManagedBytes).toBeGreaterThan(0n);
            } else if (boundary === 'import upload') {
              expect(actual.tempByteCount).toBe(0n);
              expect(request.sourceSpoolBytes).toBe(BigInt(fixture.archive.byteLength));
            } else {
              expect(actual.tempByteCount).toBeGreaterThan(0n);
              expect(request.candidateSpoolBytes).toBeGreaterThan(0n);
            }
            const plannedTemp = request.sourceSpoolBytes + request.candidateSpoolBytes;
            const existingTemp = resource === 'temporary quota'
              ? diskAdmission.ARCHIVE_TEMP_QUOTA_BYTES - plannedTemp + 1n : actual.tempByteCount;
            const payloadBytes = existingTemp + plannedTemp + request.finalManagedBytes;
            const freeBytes = resource === 'temporary quota' ? 32n * 1024n ** 3n :
              payloadBytes + (resource === 'SQLite/WAL allowance'
                ? diskAdmission.OPERATING_DISK_RESERVE_BYTES : diskAdmission.SQLITE_WAL_ALLOWANCE_BYTES);
            try { return diskAdmission.evaluateDiskAdmission(request, existingTemp, freeBytes); }
            catch (error) {
              expect(error).toBeInstanceOf(diskAdmission.DiskAdmissionError);
              refusal = error as diskAdmission.DiskAdmissionError;
              throw error;
            }
          });
        try {
          for (let repetition = 0; repetition < (boundary === 'import upload' ? 12 : 1); repetition++) {
            seen = 0;
            const response = await fetch(`http://127.0.0.1:${fixture.server.port}/api/${operation === 'export' ? 'export/latest' : 'import/archive'}`,
              operation === 'export' ? { signal: AbortSignal.timeout(5000) } :
                { method: 'POST', body: new Uint8Array(fixture.archive), signal: AbortSignal.timeout(5000) })
              .catch(error => { throw new Error(`resource rejection response failed (${repetition})`, { cause: error }); });
            expect(response.status).toBe(operation === 'export' ? 500 : 400);
            expect(refusal).toBeDefined();
            expect(refusal?.code).toBe(resource === 'temporary quota' ? 'TEMP_QUOTA' : 'FREE_DISK');
            expect(await response.json()).toMatchObject({ ok: false, message: refusal!.message });
            await preserved(fixture);
          }
          expect(seen).toBe(attempt);
          expect(spool).toHaveBeenCalledTimes(boundary === 'import preparation' ? 1 : 0);
          expect(stage).not.toHaveBeenCalled();
          expect(commit).not.toHaveBeenCalled();
          await preserved(fixture);
          await advancing(fixture);
          expect(release).toHaveBeenCalledTimes(operation === 'export' ? 1 : 0);
          expect(createHash('sha256').update(fixture.archive).digest('hex')).toBe(sourceHash);
          admission.mockRestore();
          const retry = await fixtureArchiveDownload(fixture.server.port, 'resource rejection retry');
          expect(retry.status).toBe(200);
          expect(Buffer.from(await retry.arrayBuffer())).toEqual(fixture.archive);
          await preserved(fixture);
        } finally {
          admission.mockRestore(); spool.mockRestore(); stage.mockRestore(); commit.mockRestore(); release.mockRestore();
        }
      });
    }, ARCHIVE_RETRY_FIXTURE_TIMEOUT_MS
  );

  it.runIf(process.platform === 'linux' && process.env['SLITHER_PRIVATE_QUOTA_TEST'] === '1').each([
    ['import upload', 'import', 1], ['import preparation', 'import', 2], ['export encoding', 'export', 1]
  ] as const)('preserves the game after real private-filesystem exhaustion during %s',
    async (_boundary, operation, attempt) => {
      await experiment(async fixture => {
        const peers = await Promise.all([controller(fixture, 'ui'), controller(fixture, 'bot')]);
        const baselines = peers.map(peer => ({ assignment: { ...peer.assignment! }, sample: peer.sample! }));
        const activity = ((await health(fixture.server))['telemetry'] as ExperimentalRuntimeTelemetrySnapshot).controllerActivity;
        const originalAdmit = diskAdmission.admitDiskOperation;
        let admitted = 0;
        let constrained = false;
        let beforeFree: bigint | undefined;
        const stage = vi.spyOn(BackgroundOutputPump.prototype, 'stagePreparedImport');
        const commit = vi.spyOn(CheckpointPersistenceClient.prototype, 'commitImport');
        const admission = vi.spyOn(diskAdmission, 'admitDiskOperation')
          .mockImplementation(async (directory, request) => {
            const decision = await originalAdmit(directory, request);
            if (request.operation === operation && ++admitted === attempt) {
              expect(directory).toBe(fixture.managedDirectory);
              const capacity = await statfs(directory, { bigint: true });
              beforeFree = capacity.bavail * capacity.bsize;
              expect(beforeFree).toBeGreaterThan(decision.requiredFreeBytes);
              await quota(directory, (capacity.blocks - capacity.bfree) * capacity.bsize);
              constrained = true;
              expect((await statfs(directory, { bigint: true })).bavail).toBe(0n);
              await heldInput(peers);
            }
            return decision;
          });
        try {
          const response = await fetch(`http://127.0.0.1:${fixture.server.port}/api/${operation === 'export'
            ? 'export/latest' : 'import/archive'}`, operation === 'export'
            ? { signal: AbortSignal.timeout(5000) }
            : { method: 'POST', body: new Uint8Array(fixture.archive), signal: AbortSignal.timeout(5000) });
          expect(response.status).toBe(operation === 'export' ? 500 : 400);
          const rejected = await response.json() as { ok: boolean; message: string };
          expect(rejected.ok).toBe(false);
          expect(rejected.message).toMatch(/ENOSPC|no space left on device|os error 28/iu);
          expect(constrained).toBe(true);
          expect(admitted).toBe(attempt);
          expect(stage).not.toHaveBeenCalled();
          expect(commit).not.toHaveBeenCalled();
          await quota(fixture.managedDirectory, 3n * 1024n ** 3n);
          await preserved(fixture);
          await advancing(fixture);
          await outcome(() => peers.every((peer, index) => peer.sample!.tick > baselines[index]!.sample.tick &&
            headingChange(baselines[index]!.sample, peer.sample!) < -0.01),
          'steering delivered during disk exhaustion did not reach the preserved world');
          for (const [index, peer] of peers.entries()) {
            expect(peer.socket.readyState).toBe(WebSocket.OPEN);
            expect(peer.assignment).toEqual(baselines[index]!.assignment);
            expect(peer.packets.filter(packet => ['stateReplaced', 'error'].includes(String(packet['type'])))).toEqual([]);
          }
          const applied = ((await health(fixture.server))['telemetry'] as ExperimentalRuntimeTelemetrySnapshot).controllerActivity;
          expect(applied.player.appliedActions).toBeGreaterThanOrEqual(activity.player.appliedActions + 1);
          expect(applied.trainer.appliedActions).toBe(activity.trainer.appliedActions + 1);
          admission.mockRestore();
          const retry = await fetch(`http://127.0.0.1:${fixture.server.port}/api/${operation === 'export'
            ? 'export/latest' : 'import/archive'}`, operation === 'export'
            ? { signal: AbortSignal.timeout(5000) }
            : { method: 'POST', body: new Uint8Array(fixture.archive), signal: AbortSignal.timeout(5000) });
          expect(retry.status).toBe(200);
          if (operation === 'export') {
            expect(Buffer.from(await retry.arrayBuffer())).toEqual(fixture.archive);
            await preserved(fixture);
          } else {
            expect(await retry.json()).toMatchObject({ ok: true, runId: fixture.identity['runId'] });
            await noTransferScratch(fixture.managedDirectory);
            expect(metadata(fixture.databasePath)).toEqual(
              (fixture.metadata as Array<{ name: string; rows: unknown[] }>).map(table =>
                table.name === 'rust_active_run_v1' ? { ...table,
                  rows: [{ singleton: 1, run_id: fixture.identity['runId'] }] } : table));
            expect(await files(fixture.managedDirectory)).toEqual(fixture.files);
            const replaced = await health(fixture.server);
            expect(replaced).toMatchObject({ ok: true, runId: fixture.identity['runId'],
              seed: fixture.identity['seed'], generation: fixture.identity['generation'] });
            expect(BigInt(`0x${replaced['worldEpoch'] as string}`))
              .toBeGreaterThan(BigInt(`0x${fixture.identity['worldEpoch'] as string}`));
          }
          await advancing(fixture);
        } finally {
          admission.mockRestore(); stage.mockRestore(); commit.mockRestore();
          if (constrained) await quota(fixture.managedDirectory, 3n * 1024n ** 3n);
        }
      }, true);
    }, 20_000);

  it('rejects independently valid import and reset exceeding native state memory before replacement', async () => {
    // Admit the default source plus its baseline export, but reject the larger graph beside it.
    const ceiling = 2720n * 1024n ** 2n;
    const inspect = CheckpointPersistenceClient.prototype.inspectRetention;
    const retention = vi.spyOn(CheckpointPersistenceClient.prototype, 'inspectRetention')
      .mockImplementation(async function (this: CheckpointPersistenceClient) {
        const inventory = await inspect.call(this);
        await new Promise<void>(done => setTimeout(done, 40));
        return inventory;
      });
    const originalCreate = freshRunSessions.createExperimentalFreshRunSession;
    const construction = vi.spyOn(freshRunSessions, 'createExperimentalFreshRunSession')
      .mockImplementation(options => originalCreate({ ...options,
        memoryCeilingBytes: options.seed === 42 ? ceiling : options.memoryCeilingBytes }));
    try {
      await experiment(async fixture => {
        const candidate = await largeBrainArchive(fixture);
        const candidateHash = createHash('sha256').update(candidate).digest('hex');
        const stage = vi.spyOn(BackgroundOutputPump.prototype, 'stagePreparedImport');
        const commit = vi.spyOn(CheckpointPersistenceClient.prototype, 'commitImport');
        const cancel = vi.spyOn(BackgroundOutputPump.prototype, 'cancelPreparedImport');
        try {
          const response = await fetch(`http://127.0.0.1:${fixture.server.port}/api/import/archive`,
            { method: 'POST', body: new Uint8Array(candidate), signal: AbortSignal.timeout(5000) });
          expect(response.status).toBe(400);
          const rejected = await response.json() as { ok: boolean; message: string };
          expect(rejected).toMatchObject({ ok: false });
          expect(rejected.message).toMatch(/checkpoint state admission failed: state requires an estimated \d+ bytes/iu);
          const remaining = /exceeding the (\d+)-byte ceiling/u.exec(rejected.message)?.[1];
          expect(remaining).toBeDefined();
          expect(BigInt(remaining!)).toBeGreaterThan(0n);
          expect(BigInt(remaining!)).toBeLessThan(ceiling);
          expect(stage).toHaveBeenCalledTimes(1);
          expect(cancel).toHaveBeenCalledTimes(1);
          expect(commit).not.toHaveBeenCalled();
          await preserved(fixture);
          await advancing(fixture);
          expect(createHash('sha256').update(candidate).digest('hex')).toBe(candidateHash);
          const peer = await controller(fixture, 'ui');
          peer.socket.send(JSON.stringify({ type: 'reset', graphSpec: buildLargeBrainGraph(147),
            settings: { snakeCount: 2, simSpeed: 0.1 }, updates: [
              { path: 'sense.bubbleBins', value: 32 }, { path: 'baselineBots.count', value: 0 },
        {path: 'sense.layoutVersion', value: 3}, {path: 'baselineBots.algorithm', value: 1}
            ] }));
          await outcome(() => peer.packets.some(packet => packet['type'] === 'error' &&
            /exceeding the \d+-byte ceiling/u.test(String(packet['message']))),
          'oversized reset did not reject the retained native ceiling');
          const resetError = peer.packets.find(packet => packet['type'] === 'error' &&
            /exceeding the \d+-byte ceiling/u.test(String(packet['message'])));
          const resetRemaining = /exceeding the (\d+)-byte ceiling/u.exec(String(resetError?.['message']))?.[1];
          expect(BigInt(resetRemaining!)).toBeGreaterThan(0n);
          expect(BigInt(resetRemaining!)).toBeLessThan(ceiling);
          expect(peer.packets.filter(packet => packet['type'] === 'stateReplaced')).toEqual([]);
          expect(stage).toHaveBeenCalledTimes(2);
          expect(cancel).toHaveBeenCalledTimes(2);
          expect(commit).not.toHaveBeenCalled();
          await preserved(fixture);
          await advancing(fixture);
          const retry = await fetch(`http://127.0.0.1:${fixture.server.port}/api/import/archive`,
            { method: 'POST', body: new Uint8Array(fixture.archive), signal: AbortSignal.timeout(5000) });
          expect(retry.status).toBe(200);
          expect(await retry.json()).toMatchObject({ ok: true, runId: fixture.identity['runId'] });
          await noTransferScratch(fixture.managedDirectory);
          expect(metadata(fixture.databasePath)).toEqual(
            (fixture.metadata as Array<{ name: string; rows: unknown[] }>).map(table =>
              table.name === 'rust_active_run_v1' ? { ...table,
                rows: [{ singleton: 1, run_id: fixture.identity['runId'] }] } : table));
          expect(await files(fixture.managedDirectory)).toEqual(fixture.files);
          const replaced = await health(fixture.server);
          expect(replaced).toMatchObject({ ok: true, runId: fixture.identity['runId'],
            seed: fixture.identity['seed'], generation: fixture.identity['generation'] });
          expect(BigInt(`0x${replaced['worldEpoch'] as string}`))
            .toBeGreaterThan(BigInt(`0x${fixture.identity['worldEpoch'] as string}`));
          await advancing(fixture);
        } finally { stage.mockRestore(); commit.mockRestore(); cancel.mockRestore(); }
      });
    } finally { construction.mockRestore(); retention.mockRestore(); }
  }, 30_000);

  it.each(['export-hof-weights.partial', 'slither-save.partial', 'slither-save.ready'] as const)(
    'preserves an existing %s when actual export creation or publication fails', async suffix => {
      await experiment(async fixture => {
        const originalAcquire = CheckpointPersistenceClient.prototype.acquireCurrentExportLease;
        const originalRelease = CheckpointPersistenceClient.prototype.releaseExportLease;
        const earlierBytes = Buffer.from('pre-existing task evidence: preserve these exact bytes');
        let earlierPath: string | undefined;
        let released = false;
        const acquiring = vi.spyOn(CheckpointPersistenceClient.prototype, 'acquireCurrentExportLease')
          .mockImplementationOnce(async function(this: CheckpointPersistenceClient, ...args) {
            const lease = await originalAcquire.apply(this, args);
            earlierPath = join(fixture.managedDirectory, `.${lease.operationId}.${suffix}`);
            await writeFile(earlierPath, earlierBytes, { flag: 'wx' });
            return lease;
          });
        const release = vi.spyOn(CheckpointPersistenceClient.prototype, 'releaseExportLease')
          .mockImplementation(async function(this: CheckpointPersistenceClient, ...args) {
            await originalRelease.apply(this, args);
            released = true;
          });
        try {
          const responseDeadlineMs = suffix === 'slither-save.ready' ? ARCHIVE_PREPARATION_TIMEOUT_MS : 5000;
          const response = await fetch(`http://127.0.0.1:${fixture.server.port}/api/export/latest`,
            { signal: AbortSignal.timeout(responseDeadlineMs) });
          expect(response.status).toBe(500);
          expect(response.headers.get('content-disposition')).toBeNull();
          expect(await response.json()).toMatchObject({ ok: false, message: expect.any(String) });
          const deadline = performance.now() + 5000;
          while (!released && performance.now() < deadline) await new Promise<void>(done => setTimeout(done, 10));
          expect(released).toBe(true);
          expect(release).toHaveBeenCalledOnce();
          expect(await readFile(earlierPath!)).toEqual(earlierBytes);
          await unlink(earlierPath!);
          earlierPath = undefined;
          await preserved(fixture);
          await advancing(fixture);
          acquiring.mockRestore();
          const retry = await fixtureArchiveDownload(fixture.server.port, 'export collision retry');
          expect(retry.status).toBe(200);
          expect(Buffer.from(await retry.arrayBuffer())).toEqual(fixture.archive);
          await preserved(fixture);
        } finally {
          acquiring.mockRestore(); release.mockRestore();
          if (earlierPath) await unlink(earlierPath).catch(error => {
            if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error;
          });
        }
      });
    // The ready-file collision allows a 30-second preparation response and then
    // proves successful retry; its whole fixture must not expire after 20 seconds.
    }, ARCHIVE_RETRY_FIXTURE_TIMEOUT_MS
  );
}, ARCHIVE_FIXTURE_TIMEOUT_MS);
