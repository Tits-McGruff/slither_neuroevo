import { createServer, request as httpRequest, type Server, type IncomingHttpHeaders } from 'node:http';
import { SavedBrainLibrary } from './savedBrainLibrary.ts';
import { DistillationSpoolClient } from './distillationSpoolClient.ts';
import { DistillationHttpRouter } from './distillationHttp.ts';
import { BackgroundCommandAdmission } from './commandAdmission.ts';
import type { SavedBrainSpawnReceiptV1 } from '../protocol.ts';
import { createHash, randomBytes } from 'node:crypto';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { expect, it, vi } from 'vitest';
import type { RustBackgroundEvent } from '../../src/protocol/rustBackground.ts';
import { loadPanicBinding } from '../test/panicRuntime.ts';
import type { ExperimentalFreshRunNativeHandle } from './experimentalFreshRunSession.ts';
import type { ExperimentalRunningAuthorityNativeHandle } from './backgroundRuntime.ts';
import { CheckpointPersistenceClient } from './checkpointPersistenceClient.ts';
import { parseManagedCheckpointDescriptor } from './checkpointPersistenceProtocol.ts';
import type { ExperimentalEngineInit } from './experimentalNativeBridge.ts';

/** Explicit native-only v4 admission, absent from the production constructor. */
interface QualifiedSession extends ExperimentalFreshRunNativeHandle {
  /** Build the native qualification fixture's Dense Basic B8 graph. */
  initializeBaselineV2ForTest(request: {profile: string; bins: number; mode: number;
    cadence: number; rayMinimum: number; rayCap: number; rayBatch: number}): Promise<unknown>;
}

/** Production-sized bounded queues; test code never executes a neural graph. */
const INIT: ExperimentalEngineInit = {contractVersion: 1, maxInboundBatches: 16, maxInboundCommands: 16,
  maxInboundOwnedBytes: 2 * 1024 * 1024, maxBatchCommands: 1, maxBatchOwnedBytes: 1024 * 1024,
  maxOutputReliable: 32, maxOutputReliableOwnedBytes: 16 * 1024 * 1024, maxOutputDiscrete: 4,
  maxOutputDiscreteOwnedBytes: 1024 * 1024, maxOutputTotalOwnedBytes: 32 * 1024 * 1024,
  maxOutputEventOwnedBytes: 1024 * 1024, maxOutputFrameConnections: 4};

/** Remove only this test's verified direct child of the temporary directory. */
function cleanup(root: string): void {
  if (dirname(resolve(root)) !== resolve(tmpdir()) || !root.includes('slither-library-spawn-')) throw new Error('unsafe fixture cleanup');
  rmSync(root, {recursive: true, force: true});
}

/** Real loopback requests with bounded opaque/metadata responses and no unsafe-port exclusions. */
async function exchange(port: number, path: string, method = 'GET', body?: Uint8Array | object): Promise<{
  status: number; value: unknown; bytes: Buffer; headers: IncomingHttpHeaders;
}> {
  const binary = body instanceof Uint8Array;
  const bytes = body === undefined ? undefined : binary ? body : Buffer.from(JSON.stringify(body));
  return new Promise((resolveReply, reject) => {
    const request = httpRequest({hostname: '127.0.0.1', port, path, method, signal: AbortSignal.timeout(10_000),
      headers: bytes ? {'Content-Length': bytes.byteLength, 'Content-Type': binary ? 'application/octet-stream' : 'application/json'} : {}}, response => {
      const chunks: Buffer[] = []; let length = 0;
      response.on('data', (chunk: Buffer) => {
        length += chunk.byteLength;
        if (length > 64 * 1024 * 1024) response.destroy(new Error('fixture response admission'));
        else chunks.push(chunk);
      });
      response.once('error', reject);
      response.once('end', () => {
        try {
          const data = Buffer.concat(chunks);
          resolveReply({status: response.statusCode ?? 0, headers: response.headers, bytes: data,
            value: response.headers['content-type']?.includes('application/json') ? JSON.parse(data.toString('utf8')) : undefined});
        } catch (error) { reject(error); }
      });
    });
    request.once('error', reject); request.end(bytes);
  });
}

for (const workers of [1, 3]) {
  it(`deploys, exports and deletes saved brains through native commands and HTTP (workers=${workers})`, async () => {
    const root = mkdtempSync(join(tmpdir(), 'slither-library-spawn-'));
    const managed = join(root, 'managed'); mkdirSync(managed);
    const persistence = new CheckpointPersistenceClient({databasePath: join(root, 'game.sqlite'), managedRootPath: managed});
    const Constructor = loadPanicBinding().ExperimentalStage6aFreshRunSession as new (
      run: string, seed: string, memory: string, workers: number) => QualifiedSession;
    const runId = 'saved-spawn-' + randomBytes(8).toString('hex');
    const session = new Constructor(runId, '0000002a', '0000000100000000', workers);
    let runtime: ExperimentalRunningAuthorityNativeHandle | undefined;
    let library: SavedBrainLibrary | undefined;
    let spool: DistillationSpoolClient | undefined;
    let server: Server | undefined;
    let pump: NodeJS.Timeout | undefined;
    let pumpError: Error | undefined;
    try {
      await session.initializeBaselineV2ForTest({profile: 'basic', bins: 8, mode: 0, cadence: 0.06,
        rayMinimum: 1, rayCap: 256, rayBatch: 65536});
      const checkpoint = parseManagedCheckpointDescriptor(await session.publishRunStartCheckpoint({
        managedDirectory: managed, operationId: randomBytes(16).toString('hex')}));
      await persistence.commit(checkpoint); session.acknowledgeRunStartPersistence(checkpoint);
      await session.activateRunningAuthority();
      runtime = await session.createBackgroundRuntime(INIT, () => {}) as ExperimentalRunningAuthorityNativeHandle;
      const native = runtime;
      const saved = await persistence.savedBrainCatalog({action: 'save', path: resolve('native/fixtures/portable-dense-basic8-v1.slither-brain'), name: 'explicit native deployment'});
      if (saved.action !== 'save') throw new Error('save response mismatch');
      const leaseId = randomBytes(16).toString('hex');
      await persistence.savedBrainCatalog({action: 'acquire', brainId: saved.record.brainId, leaseId});
      native.start();
      /** Drain reliable receipts while the actual native fixed-step authority runs. */
      const receipt = async (sequence: string): Promise<RustBackgroundEvent> => {
        const deadline = performance.now() + 5000;
        while (performance.now() < deadline) {
          for (const event of native.drainOutputs(32, INIT.maxOutputEventOwnedBytes).events) {
            if (event.kind === 'fault') throw new Error(event.faultDetail);
            if (event.commandSequence === sequence) return event;
          }
          await new Promise<void>(done => setImmediate(done));
        }
        throw new Error('native saved deployment receipt deadline');
      };
      native.submitLiveSettings('0000000000000001', [{path: 'brain.controlDt', value: 0.06}]);
      expect((await receipt('0000000000000001')).kind).toBe('liveSettingsApplied');
      const request = {brainId: saved.record.brainId, expectedRunId: runId,
        expectedConfigRevision: '0000000000000002', expectedWorldEpoch: native.health().worldEpoch,
        weights: saved.record.weights};
      expect(() => native.submitSavedBrainSpawn('0000000000000002', managed, {...request, expectedWorldEpoch: '0'.repeat(16)})).toThrow();
      native.submitSavedBrainSpawn('0000000000000002', managed, {...request, expectedRunId: 'stale-run'});
      expect((await receipt('0000000000000002')).kind).toBe('commandRejected');
      native.submitSavedBrainSpawn('0000000000000003', managed, {...request, weights: {
        ...request.weights, weightCount: '0000000040000000', decodedByteCount: '0000000100000000'}});
      expect(await receipt('0000000000000003')).toMatchObject({kind: 'commandRejected',
        rejectionDetail: expect.stringContaining('shared weight descriptor')});
      native.submitSavedBrainSpawn('0000000000000004', managed, request);
      const spawned = await receipt('0000000000000004');
      expect(spawned).toMatchObject({kind: 'savedBrainSpawned', savedBrainSpawn: {
        snakeId: expect.any(Number), effectiveStep: expect.stringMatching(/^[0-9a-f]{16}$/u)}});
      expect(spawned.hallOfFameResurrection).toBeUndefined();
      const step = BigInt('0x' + spawned.savedBrainSpawn!.effectiveStep);
      await persistence.savedBrainCatalog({action: 'delete', brainId: saved.record.brainId});
      await persistence.savedBrainCatalog({action: 'release', leaseId});
      expect(existsSync(join(managed, saved.record.artifactRelativeFilename))).toBe(false);
      const deadline = performance.now() + 5000;
      while (BigInt('0x' + native.health().completedStep) <= step && performance.now() < deadline) {
        for (const event of native.drainOutputs(32, INIT.maxOutputEventOwnedBytes).events) {
          if (event.kind === 'fault') throw new Error(event.faultDetail);
        }
        await new Promise<void>(done => setImmediate(done));
      }
      expect(BigInt('0x' + native.health().completedStep)).toBeGreaterThan(step);
      spool = new DistillationSpoolClient(join(root, 'diagnostics'));
      library = new SavedBrainLibrary(persistence, spool, managed, () => native, new BackgroundCommandAdmission(native, 5n),
        () => ({expectedRunId: runId, expectedConfigRevision: '0000000000000002', expectedWorldEpoch: native.health().worldEpoch}),
        () => true, () => {});
      const controls = library;
      const router = new DistillationHttpRouter(spool, undefined, controls);
      server = createServer((request, response) => router.handle(request, response, new URL(request.url!, 'http://localhost')));
      pump = setInterval(() => {
        try {
          for (const event of native.drainOutputs(32, INIT.maxOutputEventOwnedBytes).events) {
            if (event.kind === 'fault') pumpError = new Error(event.faultDetail);
            controls.event(event);
          }
        } catch (error) { pumpError = error as Error; }
      }, 5);
      await new Promise<void>(done => server!.listen(0, '127.0.0.1', done));
      const address = server.address();
      if (!address || typeof address === 'string') throw new Error('fixture port missing');
      const base = '/api/distillation/v1';
      const bytes = readFileSync(resolve('native/fixtures/portable-dense-basic8-v1.slither-brain'));
      const brainId = createHash('sha256').update(bytes).digest('hex');
      expect((await exchange(address.port, `${base}/candidates/${brainId}`, 'POST', bytes)).status).toBe(201);
      expect((await exchange(address.port, `${base}/library`)).value).toMatchObject({page: {entries: []}});
      expect((await exchange(address.port, `${base}/library/${brainId}/save`, 'POST', {name: 'api saved brain', path: 'not-accepted'})).status).toBe(400);
      const retained = await exchange(address.port, `${base}/library/${brainId}/save`, 'POST', {name: 'api saved brain'});
      expect(retained.status, JSON.stringify(retained.value)).toBe(201);
      expect(retained.value).toMatchObject({brainId, artifact: {parameterCount: 146}, provenance: {sourceKind: 'manual'}});
      expect((await exchange(address.port, `${base}/library/${brainId}/save`, 'POST', {name: 'exact retry'})).value).toEqual(retained.value);
      expect((await exchange(address.port, `${base}/library/${brainId}`)).value).toEqual(retained.value);
      expect((await exchange(address.port, `${base}/library?limit=26`)).status).toBe(400);
      expect((await exchange(address.port, `${base}/candidates/${brainId}/artifact`)).status).toBe(404);
      const exported = await exchange(address.port, `${base}/library/${brainId}/export`);
      expect(exported.status, exported.bytes.toString('utf8')).toBe(200);
      expect(exported.bytes).toEqual(bytes);
      expect(exported.headers['content-disposition']).toBe(`attachment; filename="${brainId}.slither-brain"`);
      const status = controls.status();
      const spawnBody = {...status.scope, requestId: randomBytes(16).toString('hex'),
        expectedProcessEpoch: status.processEpoch, expectedDeploymentSequence: status.deploymentSequence};
      const deployed = await exchange(address.port, `${base}/library/${brainId}/spawn`, 'POST', spawnBody);
      expect(deployed.status, JSON.stringify(deployed.value)).toBe(200);
      const deployedReceipt = deployed.value as SavedBrainSpawnReceiptV1;
      // Confirmation precedes the metadata worker's lease-release acknowledgement.
      // Check stale-cursor admission only after that real cold-operation boundary.
      await vi.waitFor(() => expect(controls.busy).toBe(false), {timeout: 5000, interval: 10});
      expect(deployedReceipt).toMatchObject({brainId, schemaVersion: 1, snakeId: expect.any(Number), commandSequence: '0000000000000005'});
      expect((await exchange(address.port, `${base}/library/${brainId}/spawn`, 'POST', spawnBody)).value).toEqual(deployedReceipt);
      expect((await exchange(address.port, `${base}/library/${brainId}/spawn`, 'POST', {...spawnBody, requestId: randomBytes(16).toString('hex')})).status).toBe(400);
      // Same-size corruption must fail native cold admission before any model export or retry succeeds.
      const original = join(managed, `${brainId}.slither-brain`);
      const damaged = Buffer.from(bytes); damaged[damaged.length - 1] = damaged[damaged.length - 1]! ^ 1;
      writeFileSync(original, damaged);
      expect((await exchange(address.port, `${base}/library/${brainId}/export`)).status).toBe(400);
      expect((await exchange(address.port, `${base}/library/${brainId}/save`, 'POST', {name: 'corrupt retry'})).status).toBe(400);
      expect((await exchange(address.port, `${base}/library/${brainId}`, 'DELETE')).value).toEqual({brainId, deleted: true});
      expect((await exchange(address.port, `${base}/library/${brainId}`)).status).toBe(404);
      expect(existsSync(original)).toBe(false);
      const imported = await exchange(address.port, `${base}/library/import?name=explicit%20file%20import`, 'POST', bytes);
      expect(imported.status, JSON.stringify(imported.value)).toBe(201);
      expect(imported.value).toMatchObject({brainId, name: 'explicit file import', provenance: {sourceKind: 'manual'}});
      expect((await exchange(address.port, `${base}/library/${brainId}/export`)).bytes).toEqual(bytes);

      const after = BigInt('0x' + deployedReceipt.effectiveStep);
      const liveDeadline = performance.now() + 5000;
      while (BigInt('0x' + native.health().completedStep) <= after && performance.now() < liveDeadline) await new Promise<void>(done => setImmediate(done));
      expect(BigInt('0x' + native.health().completedStep)).toBeGreaterThan(after);
      expect(pumpError).toBeUndefined();

    } finally {
      clearInterval(pump);
      if (server) { server.closeAllConnections(); await new Promise<void>(done => server!.close(() => done())); }
      await library?.close();
      await spool?.close();
      if (runtime) { runtime.requestStop(); await runtime.join(); }
      await persistence.close();
      cleanup(root);
    }
  }, 15000);
}
