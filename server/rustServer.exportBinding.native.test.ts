/** Real downloads retain their selected boundary while later generations and cleanup proceed. */
import { createRequire } from 'node:module';
import { mkdtemp, readdir, readFile, rm, stat } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import Database from 'better-sqlite3';
import { expect, it, onTestFinished, vi } from 'vitest';
import WebSocket from 'ws';
import { DEFAULT_CONFIG } from './config.ts';
import { startRustServer, type RustServer } from './rustServer.ts';
import { CheckpointPersistenceClient } from './rustEngine/checkpointPersistenceClient.ts';
import { parseManagedCheckpointDescriptor, type ManagedCheckpointDescriptor } from './rustEngine/checkpointPersistenceProtocol.ts';
import type { ExperimentalRunningAuthorityNativeHandle } from './rustEngine/backgroundRuntime.ts';
import type { GraphSpec } from '../src/brains/graph/schema.ts';
import { describeNetworkSuite } from './test/networkSuites.ts';
import { ARCHIVE_PREPARATION_TIMEOUT_MS, fixtureArchiveDownload } from './test/archiveDownload.ts';

/** Actual addon prototype used only to alter returned scalar facts after real archive creation. */
const native = createRequire(import.meta.url)(resolve('native/index.js')) as {
  ExperimentalRunningAuthority: { prototype: ExperimentalRunningAuthorityNativeHandle }
};

/** Real, small inference graph keeps this storage contract independent of runner CPU throughput. */
const BINDING_GRAPH: GraphSpec = {
  type: 'graph', nodes: [{ id: 'input', type: 'Input', outputSize: 83 },
    { id: 'head', type: 'Dense', inputSize: 83, outputSize: 2 }],
  edges: [{ from: 'input', to: 'head' }], outputs: [{ nodeId: 'head' }], outputSize: 2
};

/** Startup/shutdown plus two genuine archive preparations, each with its own ten-second limit. */
const ARCHIVE_RETRY_FIXTURE_TIMEOUT_MS = 10_000 + 2 * ARCHIVE_PREPARATION_TIMEOUT_MS;

/** Wait for a specific completed operation, never substituting a different fixture. */
async function observed<T>(read: () => T | undefined): Promise<T> {
  const deadline = performance.now() + 5000;
  while (performance.now() < deadline) {
    const result = read();
    if (result !== undefined) return result;
    await new Promise<void>(done => setTimeout(done, 10));
  }
  throw new Error('export binding observation timed out');
}

/** Inspect only the bounded manifest in the small genuine archive fixture. */
function manifest(bytes: Buffer): Record<string, unknown> {
  expect(bytes.byteLength).toBeLessThan(4 * 1024 * 1024);
  for (let offset = 0; offset + 512 <= bytes.length;) {
    const name = bytes.toString('ascii', offset, offset + 100).replace(/\0.*$/u, '');
    const size = Number.parseInt(bytes.toString('ascii', offset + 124, offset + 136), 8);
    if (!Number.isSafeInteger(size) || size < 0 || offset + 512 + size > bytes.length) break;
    if (name === 'manifest.json') {
      expect(size).toBeLessThanOrEqual(1024 * 1024);
      return JSON.parse(bytes.toString('utf8', offset + 512, offset + 512 + size)) as Record<string, unknown>;
    }
    offset += 512 + Math.ceil(size / 512) * 512;
  }
  throw new Error('real export has no bounded manifest');
}

/** Verify every operation file and its lease inventory has been removed. */
async function cleanTransfer(directory: string): Promise<void> {
  let names: string[] = [];
  const deadline = performance.now() + 5000;
  do {
    names = (await readdir(directory)).filter(name => name.includes('slither-save') || name.includes('export-'));
    if (names.length === 0) return;
    await new Promise<void>(done => setTimeout(done, 10));
  } while (performance.now() < deadline);
  expect(names).toEqual([]);
}

describeNetworkSuite('Rust exact download binding', () => {
  it('keeps generation two byte-identical while nine later checkpoints and real pruning complete', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-export-binding-'));
    const databasePath = join(root, 'source.sqlite');
    const directory = `${databasePath}.checkpoints`;
    const servers: RustServer[] = [];
    const boundaries = new Map<number, ManagedCheckpointDescriptor>();
    const generationTwo = Promise.withResolvers<void>();
    const generationEleven = Promise.withResolvers<void>();
    const preparation = Promise.withResolvers<void>();
    let viewer: WebSocket | undefined;
    let selectedLease: Awaited<ReturnType<CheckpointPersistenceClient['acquireCurrentExportLease']>> | undefined;
    const leaseClients = new Set<CheckpointPersistenceClient>();
    let holdingLease = false;
    let pendingDownload: Promise<Response> | undefined;
    const cancellation = new AbortController();
    const originalCommit = CheckpointPersistenceClient.prototype.commit;
    const originalAcquire = CheckpointPersistenceClient.prototype.acquireCurrentExportLease;
    const originalRelease = CheckpointPersistenceClient.prototype.releaseExportLease;
    let baselineReleased = false;
    const releasedLeases = new Set<string>();
    const releases = vi.spyOn(CheckpointPersistenceClient.prototype, 'releaseExportLease').mockImplementation(async function(
      this: CheckpointPersistenceClient, ...args: Parameters<CheckpointPersistenceClient['releaseExportLease']>
    ) {
      const result = await originalRelease.apply(this, args);
      baselineReleased = true;
      releasedLeases.add(args[0]);
      return result;
    });
    const commits = vi.spyOn(CheckpointPersistenceClient.prototype, 'commit').mockImplementation(async function(
      this: CheckpointPersistenceClient, ...args: Parameters<CheckpointPersistenceClient['commit']>
    ) {
      const result = await originalCommit.apply(this, args);
      const descriptor = parseManagedCheckpointDescriptor(args[0]);
      if (descriptor.boundaryKind === 'generation') {
        const generation = Number(BigInt(`0x${descriptor.generation}`));
        boundaries.set(generation, descriptor);
        if (generation === 2) await generationTwo.promise;
        if (generation === 11) await generationEleven.promise;
      }
      return result;
    });
    const acquiring = vi.spyOn(CheckpointPersistenceClient.prototype, 'acquireCurrentExportLease').mockImplementation(async function(
      this: CheckpointPersistenceClient
    ) {
      const lease = await originalAcquire.call(this);
      if (holdingLease) {
        selectedLease = lease; leaseClients.add(this);
        await preparation.promise;
      }
      return lease;
    });
    try {
      const source = await startRustServer({ ...DEFAULT_CONFIG, port: 0, dbPath: databasePath, resume: 'fresh', seed: 42,
        rustCalculationWorkers: 1 });
      servers.push(source);
      expect(source.startupFault).toBeUndefined();
      const messages: Array<Record<string, unknown>> = [];
      viewer = new WebSocket(`ws://127.0.0.1:${source.port}`);
      viewer.on('message', (bytes, binary) => { if (!binary) messages.push(JSON.parse(bytes.toString()) as Record<string, unknown>); });
      await new Promise<void>((done, reject) => { viewer!.once('open', done); viewer!.once('error', reject); });
      viewer.send(JSON.stringify({ type: 'hello', version: 2, clientType: 'ui' }));
      await observed(() => messages.find(message => message['type'] === 'welcome'));
      viewer.send(JSON.stringify({ type: 'join', mode: 'spectator' }));
      viewer.send(JSON.stringify({ type: 'reset', graphSpec: BINDING_GRAPH, settings: { snakeCount: 12, simSpeed: 12 },
        updates: [{ path: 'generationSeconds', value: 8 }, { path: 'baselineBots.count', value: 2 },
          { path: 'pelletCountTarget', value: 100 }] }));
      await observed(() => messages.find(message => message['type'] === 'stateReplaced'));
      const selected = await observed(() => boundaries.get(2));
      const originalBytes = await readFile(join(directory, selected.relativeFilename));
      const baseline = await fetch(`http://127.0.0.1:${source.port}/api/export/latest`);
      expect(baseline.status).toBe(200);
      const baselineBytes = Buffer.from(await baseline.arrayBuffer());
      await cleanTransfer(directory);
      // Ready-file cleanup precedes the lease ACK and clearing the server's single-export slot.
      await observed(() => baselineReleased ? true : undefined);
      await new Promise<void>(done => setImmediate(done));
      holdingLease = true;
      let headersDelivered = false;
      pendingDownload = fetch(`http://127.0.0.1:${source.port}/api/export/latest`, { signal: cancellation.signal })
        .then(response => { headersDelivered = true; return response; });
      await observed(() => selectedLease);
      expect(selectedLease!.descriptor).toEqual(selected);
      const sourceClient = [...leaseClients][0]!;
      const second = await fetch(`http://127.0.0.1:${source.port}/api/export/latest`);
      expect(second.status).toBe(409);
      expect(await second.json()).toMatchObject({ ok: false, message: expect.stringContaining('in progress') });
      generationTwo.resolve();
      for (let generation = 3; generation <= 11; generation++) await observed(() => boundaries.get(generation));
      expect(headersDelivered).toBe(false);
      expect(await readFile(join(directory, selected.relativeFilename))).toEqual(originalBytes);
      const inventory = await sourceClient.inspectRetention();
      expect(inventory.plannedPrune.checkpointCount).toBeGreaterThan(0);
      const current = await sourceClient.selectCurrent();
      expect(current).toEqual(boundaries.get(11));
      preparation.resolve();
      const response = await pendingDownload;
      expect(response.status).toBe(200);
      expect(response.headers.get('x-slither-checkpoint-id')).toBe(selected.logicalRootSha256);
      expect(response.headers.get('content-disposition')).toBe(
        `attachment; filename="slither-neuroevo-${selected.logicalRootSha256.slice(0, 12)}-gen-2-v1.slither-save"`);
      const bytes = Buffer.from(await response.arrayBuffer());
      expect(bytes.byteLength).toBe(Number(response.headers.get('content-length')));
      expect(bytes).toEqual(baselineBytes);
      expect(manifest(bytes)).toMatchObject({ checkpointLogicalRootSha256: selected.logicalRootSha256,
        runId: selected.runId, generationHex: selected.generation, historyCountHex: '0000000000000001',
        logicalRootSha256: response.headers.get('x-slither-save-root') });
      await cleanTransfer(directory);
      await observed(() => releasedLeases.has(selectedLease!.operationId) ? true : undefined);
      const cleanup = await sourceClient.applyRetention();
      expect(cleanup.deletedCheckpointCount).toBeGreaterThan(0);
      await expect(stat(join(directory, selected.relativeFilename))).rejects.toMatchObject({ code: 'ENOENT' });
      const database = new Database(databasePath, { readonly: true });
      try {
        expect(database.prepare('SELECT count(*) AS count FROM rust_generation_history_v1 WHERE run_id = ?')
          .get(selected.runId)).toEqual({ count: 10 });
      } finally { database.close(); }
      holdingLease = false;
      const target = await startRustServer({ ...DEFAULT_CONFIG, port: 0, dbPath: join(root, 'target.sqlite'), resume: 'fresh', seed: 99,
        rustCalculationWorkers: 1 });
      servers.push(target);
      const imported = await fetch(`http://127.0.0.1:${target.port}/api/import/archive`, {
        method: 'POST', body: new Uint8Array(bytes) });
      expect(imported.status).toBe(200);
      expect(await imported.json()).toMatchObject({ ok: true, runId: selected.runId,
        generation: selected.generation, checkpointId: selected.logicalRootSha256 });
    } finally {
      holdingLease = false;
      generationTwo.resolve(); generationEleven.resolve(); preparation.resolve();
      cancellation.abort();
      await pendingDownload?.then(response => {
        if (response.body && !response.body.locked) return response.body.cancel();
        return undefined;
      }, () => undefined).catch(() => undefined);
      commits.mockRestore(); acquiring.mockRestore(); releases.mockRestore(); viewer?.terminate();
      for (const server of servers.reverse()) await server.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);

  it.each(['checkpoint', 'generation'] as const)('rejects a safe filename identifying the wrong %s before HTTP success', async mismatch => {
    const root = await mkdtemp(join(tmpdir(), 'slither-export-name-'));
    const databasePath = join(root, 'source.sqlite');
    let server: RustServer | undefined;
    const prototype = native.ExperimentalRunningAuthority.prototype;
    const originalPrepare = prototype.prepareExportArchive;
    const changed = vi.spyOn(prototype, 'prepareExportArchive').mockImplementation(async function(
      this: ExperimentalRunningAuthorityNativeHandle, ...args: Parameters<ExperimentalRunningAuthorityNativeHandle['prepareExportArchive']>
    ) {
      const result = await originalPrepare.apply(this, args);
      const checkpoint = args[2];
      const prefix = mismatch === 'checkpoint' ?
        `${checkpoint.logicalRootSha256[0] === '0' ? '1' : '0'}${checkpoint.logicalRootSha256.slice(1, 12)}` :
        checkpoint.logicalRootSha256.slice(0, 12);
      const generation = BigInt(`0x${checkpoint.generation}`) + (mismatch === 'generation' ? 1n : 0n);
      return { ...result, downloadFilename: `slither-neuroevo-${prefix}-gen-${generation}-v1.slither-save` };
    });
    let restored = false;
    /** Restore at timeout too; a delayed finally must not undo a newer test's mock. */
    const restore = (): void => {
      if (restored) return;
      restored = true;
      changed.mockRestore();
    };
    onTestFinished(restore);
    try {
      server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, dbPath: databasePath, resume: 'fresh', seed: 42,
        rustCalculationWorkers: 1 });
      expect(server.startupFault).toBeUndefined();
      const health = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as { startupCheckpointId: string };
      const response = await fixtureArchiveDownload(server.port, 'invalid filename');
      expect(response.status).toBe(500);
      expect(response.headers.get('content-disposition')).toBeNull();
      expect(await response.json()).toMatchObject({ ok: false, message: 'Rust returned an invalid export archive descriptor' });
      await cleanTransfer(`${databasePath}.checkpoints`);
      expect(await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json()).toMatchObject({
        ok: true, startupCheckpointId: health.startupCheckpointId });
      restore();
      const retry = await fixtureArchiveDownload(server.port, 'valid filename retry');
      expect(retry.status).toBe(200);
      expect(retry.headers.get('content-disposition')).toBe(
        `attachment; filename="slither-neuroevo-${health.startupCheckpointId.slice(0, 12)}-gen-1-v1.slither-save"`);
      expect(manifest(Buffer.from(await retry.arrayBuffer()))).toMatchObject({ checkpointLogicalRootSha256: health.startupCheckpointId });
      await cleanTransfer(`${databasePath}.checkpoints`);
    } finally {
      restore(); await server?.close();
      await rm(root, { recursive: true, force: true });
    }
  }, ARCHIVE_RETRY_FIXTURE_TIMEOUT_MS);
});
