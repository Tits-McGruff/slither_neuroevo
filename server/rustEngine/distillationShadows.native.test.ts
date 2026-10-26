import { readObservationQuality, summarizeObservationQuality, type ObservationQualityNative } from '../../scripts/qualification/observationQuality.ts';
import { createHash, randomBytes } from 'node:crypto';
import { createServer, request as httpRequest } from 'node:http';
import { mkdirSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { expect } from 'vitest';
import type { ExperimentalRunningAuthorityNativeHandle } from './backgroundRuntime.ts';
import { loadPanicBinding } from '../test/panicRuntime.ts';
import { networkTest } from '../test/networkSuites.ts';
import { CheckpointPersistenceClient } from './checkpointPersistenceClient.ts';
import { parseManagedCheckpointDescriptor } from './checkpointPersistenceProtocol.ts';
import { DistillationSpoolClient } from './distillationSpoolClient.ts';
import { DistillationCapture } from './distillationCapture.ts';
import { DistillationHttpRouter } from './distillationHttp.ts';
import { BackgroundCommandAdmission } from './commandAdmission.ts';
import { parseShadowMetrics } from './shadowProtocol.ts';
import type { ShadowReceipt, ShadowMetrics } from '../../src/protocol/shadows.ts';
import type { ExperimentalFreshRunNativeHandle } from './experimentalFreshRunSession.ts';
import type { ExperimentalEngineInit } from './experimentalNativeBridge.ts';

/** Same bounded coarse queues as production's native integration fixtures. */
const INIT: ExperimentalEngineInit = { contractVersion: 1, maxInboundBatches: 16, maxInboundCommands: 16,
  maxInboundOwnedBytes: 2 * 1024 * 1024, maxBatchCommands: 1, maxBatchOwnedBytes: 1024 * 1024,
  maxOutputReliable: 32, maxOutputReliableOwnedBytes: 16 * 1024 * 1024, maxOutputDiscrete: 4,
  maxOutputDiscreteOwnedBytes: 1024 * 1024, maxOutputTotalOwnedBytes: 32 * 1024 * 1024,
  maxOutputEventOwnedBytes: 1024 * 1024, maxOutputFrameConnections: 4 };
/** Fixed synthetic portable graph/weights, independently validated by both source-identified addons. */
const artifactPath = resolve('native/fixtures/portable-brain-v1.slither-brain');
/** Exclusive test upload bytes, never owner models. */
const artifact = readFileSync(artifactPath);
/** Complete immutable candidate identity. */
const artifactId = createHash('sha256').update(artifact).digest('hex');
/** A qualification constructor exists only on the isolated test-hooks build. */
interface QualifiedSession extends ExperimentalFreshRunNativeHandle {
  /** Delegate to the existing explicit native v4/v2 qualification constructor. */
  initializeBaselineV2ForTest(request: { profile: string; bins: number; mode: number; cadence: number;
    rayMinimum: number; rayCap: number; rayBatch: number }): Promise<unknown>;
}
/** Read real loopback transport without Fetch's browser unsafe-port restrictions. */
async function exchange(port: number, path: string, body?: Uint8Array | Record<string, unknown>): Promise<{ status: number; data: unknown }> {
  const binary = body instanceof Uint8Array;
  const bytes = body === undefined ? undefined : binary ? body : Buffer.from(JSON.stringify(body));
  return await new Promise((resolveReply, reject) => {
    const request = httpRequest({ hostname: '127.0.0.1', port, path, method: bytes ? 'POST' : 'GET',
      headers: bytes ? { 'Content-Length': bytes.byteLength, 'Content-Type': binary ? 'application/octet-stream' : 'application/json' } : {},
      signal: AbortSignal.timeout(10_000) }, response => {
      const chunks: Buffer[] = []; let length = 0;
      response.on('data', (chunk: Buffer) => { length += chunk.byteLength; if (length > 1024 * 1024) response.destroy(new Error('fixture metadata byte admission')); else chunks.push(chunk); });
      response.once('error', reject);
      response.once('end', () => { try { resolveReply({ status: response.statusCode ?? 0, data: JSON.parse(Buffer.concat(chunks).toString()) }); } catch (error) { reject(error); } });
    });
    request.once('error', reject); request.end(bytes);
  });
}

/** Verify the resolved test-owned root before recursive cleanup. */
function removeFixtureRoot(root: string): void {
  if (dirname(resolve(root)) !== resolve(tmpdir()) || !root.includes('slither-shadow-api-')) throw new Error('unsafe fixture cleanup');
  rmSync(root, { recursive: true, force: true });
}

for (const workers of [1, 3]) {
networkTest('activates four immutable shadows over actual Rust teacher rows through HTTP (workers=' + workers + ')', async () => {
  const root = mkdtempSync(join(tmpdir(), 'slither-shadow-api-'));
  const managed = join(root, 'managed'); mkdirSync(managed);
  const binding = loadPanicBinding() as ReturnType<typeof loadPanicBinding> & { validateBrainArtifactFile(path: string, registerShadow?: boolean): string };
  const Constructor = binding.ExperimentalStage6aFreshRunSession as new (run: string, seed: string, memory: string, workers: number) => QualifiedSession;
  const runId = 'shadow-api-' + randomBytes(8).toString('hex');
  const session = new Constructor(runId, '0000002a', '0000000100000000', workers);
  const persistence = new CheckpointPersistenceClient({ databasePath: join(root, 'game.sqlite'), managedRootPath: managed });
  const spool = new DistillationSpoolClient(join(root, 'diagnostics'));
  let runtime: ExperimentalRunningAuthorityNativeHandle | undefined;
  let capture: DistillationCapture | undefined;
  let timer: NodeJS.Timeout | undefined;
  let pumpError: Error | undefined;
  const server = createServer((request, response) => { if (!router) { response.writeHead(503); response.end(); } else router.handle(request, response, new URL(request.url!, 'http://localhost')); });
  let router: DistillationHttpRouter | undefined;
  try {
    await session.initializeBaselineV2ForTest({ profile: 'basic', bins: 8, mode: 0, cadence: 0.06, rayMinimum: 1, rayCap: 256, rayBatch: 65536 });
    const descriptor = parseManagedCheckpointDescriptor(await session.publishRunStartCheckpoint({ managedDirectory: managed, operationId: randomBytes(16).toString('hex') }));
    await persistence.commit(descriptor); session.acknowledgeRunStartPersistence(descriptor);
    await session.activateRunningAuthority(); runtime = await session.createBackgroundRuntime(INIT, () => {}) as ExperimentalRunningAuthorityNativeHandle;
    const native = runtime;
    const qualityBefore = readObservationQuality(native as typeof native & ObservationQualityNative);
    expect(qualityBefore.snapshot.runId).toBeNull();
    expect(qualityBefore.snapshot.ordinaryPublishedSteps).toBe('0'.repeat(16));
    const admission = new BackgroundCommandAdmission(native);
    capture = new DistillationCapture(() => native, admission, spool, () => ({ runId, seed: 42 }));
    router = new DistillationHttpRouter(spool, capture);
    // The hook fixture and production diagnostic worker are different DLL images.
    // Admit the same immutable file into the isolated fixture too; both validators and all inference remain real Rust.
    const prepare = spool.prepareShadow.bind(spool);
    spool.prepareShadow = async id => {
      const record = await prepare(id);
      const fixtureDescriptor = JSON.parse(binding.validateBrainArtifactFile(artifactPath, true));
      expect(fixtureDescriptor).toEqual(record.descriptor); return record;
    };
    timer = setInterval(() => {
      try {
        for (const event of native.drainOutputs(32, INIT.maxOutputEventOwnedBytes).events) {
          if (event.kind === 'fault') pumpError = new Error(event.faultDetail ?? 'native fault');
          capture!.event(event);
        }
      } catch (error) { pumpError = error as Error; }
    }, 5);
    native.start();
    await new Promise<void>(done => server.listen(0, '127.0.0.1', done));
    const address = server.address(); if (!address || typeof address === 'string') throw new Error('fixture address missing');
    const base = '/api/distillation/v1';
    expect((await exchange(address.port, `${base}/candidates/${artifactId}`, artifact)).status).toBe(201);
    const started = await exchange(address.port, `${base}/capture/start`, { expectedRunId: runId, expectedConfigRevision: '0000000000000001',
      baselineSlots: Array.from({ length: 10 }, (_, slot) => slot), pendingBytes: 1024 * 1024, queueChunks: 32, queueBytes: 4 * 1024 * 1024 });
    expect(started.status, JSON.stringify(started.data)).toBe(201);
    const epoch = (started.data as { contract: { captureEpoch: string } }).contract.captureEpoch;
    const teacherDeadline = performance.now() + 5000;
    let teacher: number | undefined;
    do {
      const page = await spool.list(epoch, '0'.repeat(16), 32);
      teacher = page.chunks.find(chunk => chunk.endReason === 'simulatedSecond' && chunk.count > 0)?.baselineSlot;
      if (teacher !== undefined) break;
      if (pumpError) throw pumpError;
      await new Promise(done => setTimeout(done, 10));
    } while (performance.now() < teacherDeadline);
    expect(teacher, 'capture must produce an actual continuing teacher stream').toBeDefined();
    const receipts: ShadowReceipt[] = [];
    for (let slot = 0; slot < 4; slot++) {
      const body = { requestId: randomBytes(16).toString('hex'), expectedProcessEpoch: capture.shadows.processEpoch, expectedActivationSequence: '0000000000000000', expectedRunId: runId,
        expectedConfigRevision: '0000000000000001', captureEpoch: epoch, baselineSlot: teacher!, artifactSha256: artifactId };
      const activated = await exchange(address.port, `${base}/shadows/${slot}/activate`, body);
      expect(activated.status, JSON.stringify(activated.data)).toBe(200);
      receipts.push(activated.data as ShadowReceipt);
      expect((await exchange(address.port, `${base}/shadows/${slot}/activate`, body)).data).toEqual(activated.data);
    }
    const deadline = performance.now() + 5000;
    let metrics!: ShadowMetrics;
    do {
      if (pumpError) throw pumpError;
      const result = await exchange(address.port, `${base}/shadows`); expect(result.status).toBe(200);
      const { processEpoch: _process, ...publication } = result.data as ShadowMetrics & { processEpoch: string }; void _process;
      metrics = parseShadowMetrics(JSON.stringify(publication));
      if (metrics.slots.every(slot => slot && BigInt('0x' + slot.totals.samples) > 0n)) break;
      await new Promise(done => setTimeout(done, 10));
    } while (performance.now() < deadline);
    expect(metrics.slots.every(slot => slot?.enabled && BigInt('0x' + slot.totals.samples) > 0n), JSON.stringify({ slots: metrics.slots.map(slot => slot && ({ enabled: slot.enabled, error: slot.error, samples: slot.totals.samples })), step: native.health().completedStep })).toBe(true);
    expect(metrics.slots.every(slot => slot!.lastSampleStep <= slot!.completedStep)).toBe(true);
    const beforeStop = native.health().completedStep;
    const stopped = await exchange(address.port, `${base}/shadows/0/stop`, { requestId: randomBytes(16).toString('hex'),
      captureEpoch: epoch, activationSequence: receipts[0]!.activationSequence });
    expect(stopped.status, JSON.stringify(stopped.data)).toBe(200);
    expect(stopped.data).toMatchObject({ enabled: false, activationSequence: receipts[0]!.activationSequence });
    const history = await exchange(address.port, `${base}/shadows/events?limit=10`);
    expect((history.data as { events: unknown[] }).events).toHaveLength(5);
    expect(native.health().completedStep >= beforeStop).toBe(true);
    expect(pumpError).toBeUndefined();
    const qualityAfter = readObservationQuality(native as typeof native & ObservationQualityNative);
    const quality = summarizeObservationQuality(qualityBefore, qualityAfter);
    expect(qualityAfter.snapshot.runId).toBe(runId);
    const steps = BigInt(quality.ordinaryPublishedSteps);
    expect(steps).toBeGreaterThan(0n);
    for (const [index, maximum] of [55n, 10n].entries()) {
      const row = quality.consumers[index]!;
      expect(BigInt(row.counts['v4Observations']!)).toBeGreaterThan(0n);
      expect(BigInt(row.counts['v4Observations']!)).toBeLessThanOrEqual(steps * maximum);
      expect(row.counts['observations']).toBe(row.counts['v4Observations']);
    }
    expect(quality.consumers[2]!.counts['observations']).toBe('0');
  } finally {
    clearInterval(timer);
    server.closeAllConnections(); await new Promise<void>(done => server.close(() => done()));
    if (runtime) { runtime.requestStop(); await runtime.join(); }
    await capture?.close(); if (!capture) await spool.close(); await persistence.close();
    removeFixtureRoot(root);
  }
}, 15_000);

}
