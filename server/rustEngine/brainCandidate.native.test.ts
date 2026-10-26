import { createRequire } from 'node:module';
import { mkdirSync } from 'node:fs';
import { CheckpointPersistenceClient } from './checkpointPersistenceClient.ts';
import { parseManagedCheckpointDescriptor } from './checkpointPersistenceProtocol.ts';
import type { ExperimentalFreshRunNativeBinding } from './experimentalFreshRunSession.ts';
import type { ExperimentalRunningAuthorityNativeHandle } from './backgroundRuntime.ts';
import { createHash } from 'node:crypto';
import { createServer } from 'node:http';
import { mkdtempSync, readFileSync, readdirSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { afterEach, expect, it } from 'vitest';
import { DistillationSpoolClient } from './distillationSpoolClient.ts';
import { DistillationHttpRouter } from './distillationHttp.ts';
import { networkTest } from '../test/networkSuites.ts';

/** Frozen synthetic graph/weights, produced by the PyRL exporter from a Rust reference graph. */
const artifact = readFileSync(resolve(import.meta.dirname, '../../native/fixtures/portable-brain-v1.slither-brain'));
/** Exact complete portable-byte identity. */
const identity = createHash('sha256').update(artifact).digest('hex');
/** Test-owned controlled metadata roots only. */
const roots: string[] = [];
/** Real diagnostic workers stopped before removing their test-owned directories. */
const workers: DistillationSpoolClient[] = [];

/** Start a real isolated worker; native admission cannot be replaced with a fake. */
function fixture(): DistillationSpoolClient {
  const root = mkdtempSync(join(tmpdir(), 'slither-native-candidate-'));
  roots.push(root);
  const client = new DistillationSpoolClient(root);
  workers.push(client);
  return client;
}

afterEach(async () => {
  await Promise.all(workers.splice(0).map(worker => worker.close()));
  for (const root of roots.splice(0)) {
    if (dirname(resolve(root)) !== resolve(tmpdir()) || !root.includes('slither-native-candidate-')) throw new Error('unsafe candidate test cleanup');
    rmSync(root, { recursive: true, force: true });
  }
});

it('admits portable candidates through the source-identified actual Rust worker and recovers immutable retries', async () => {
  const client = fixture();
  const result = await client.ingestCandidate(identity, artifact);
  expect(result.descriptor).toMatchObject({ schemaVersion: 1, artifactSha256: identity, inputSize: 72,
    parameterCount: 1962, stateSize: 8, controlIntervalSeconds: 0.06, lifeStateRule: 'preserve', sourceKind: 'manual' });
  expect(result.byteCount).toBe(artifact.byteLength);
  expect(await client.ingestCandidate(identity, artifact)).toEqual(result);
  expect([...((await client.downloadCandidate(identity)).bytes)]).toEqual([...artifact]);
  await client.close();
  const reopened = new DistillationSpoolClient(roots[0]!);
  workers.push(reopened);
  expect(await reopened.candidate(identity)).toEqual(result);
  expect(readdirSync(join(roots[0]!, 'candidates')).filter(name => name.endsWith('.slither-brain'))).toEqual([`${identity}.slither-brain`]);
});

it('rejects repaired outer identity with damaged inner weights before advertising any candidate', async () => {
  const client = fixture();
  const damaged = new Uint8Array(artifact);
  damaged[damaged.length - 1] = damaged[damaged.length - 1]! ^ 255;
  const id = createHash('sha256').update(damaged).digest('hex');
  await expect(client.ingestCandidate(id, damaged)).rejects.toThrow(/digest/u);
  await expect(client.candidate(id)).rejects.toThrow(/not found/u);
  expect(readdirSync(join(roots[0]!, 'candidates')).filter(name => name.endsWith('.slither-brain') || name.endsWith('.tmp'))).toEqual([]);
});

networkTest('routes bounded inbound candidate bytes to Rust and returns descriptor-only idempotent receipts', async () => {
  const client = fixture();
  const router = new DistillationHttpRouter(client);
  const server = createServer((request, response) => router.handle(request, response, new URL(request.url!, 'http://localhost')));
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve));
  const address = server.address();
  if (!address || typeof address === 'string') throw new Error('candidate test server address missing');
  const base = `http://127.0.0.1:${address.port}/api/distillation/v1/candidates/${identity}`;
  try {
    for (let repeat = 0; repeat < 2; repeat += 1) {
      const response = await fetch(base, { method: 'POST', headers: { 'Content-Type': 'application/octet-stream' }, body: artifact });
      expect(response.status).toBe(201);
      expect(await response.json()).toMatchObject({ descriptor: { artifactSha256: identity, parameterCount: 1962 }, byteCount: artifact.length });
    }
    expect((await fetch(base)).status).toBe(200);
    expect((await fetch(`${base}/artifact`)).status).toBe(404);
    const wrong = await fetch(base, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: '{}' });
    expect(wrong.status).toBe(400);
    expect(await wrong.text()).toMatch(/binary content length/u);
    const broken = new Uint8Array(artifact);
    broken[0] = 0;
    const rejected = await fetch(base, { method: 'POST', headers: { 'Content-Type': 'application/octet-stream' }, body: broken });
    expect(rejected.status).toBe(400);
  } finally {
    server.closeAllConnections();
    await new Promise<void>(resolve => server.close(() => resolve()));
  }
});


it('registers a worker-prepared candidate in the actual production runtime without granting controlling ownership', async () => {
  const client = fixture(); await client.ingestCandidate(identity, artifact); await client.prepareShadow(identity);
  const require = createRequire(import.meta.url);
  const binding = require(resolve('native/index.js')) as ExperimentalFreshRunNativeBinding;
  const root = roots.at(-1)!; const managed = join(root, 'game-managed'); mkdirSync(managed);
  const persistence = new CheckpointPersistenceClient({ databasePath: join(root, 'game.sqlite'), managedRootPath: managed });
  const session = new binding.ExperimentalStage6aFreshRunSession('candidate-cache-handoff', '0000002a', '0000000100000000', 1);
  let runtime: ExperimentalRunningAuthorityNativeHandle | undefined;
  try {
    await session.initialize();
    const descriptor = parseManagedCheckpointDescriptor(await session.publishRunStartCheckpoint({ managedDirectory: managed, operationId: '37'.repeat(16) }));
    await persistence.commit(descriptor); session.acknowledgeRunStartPersistence(descriptor); await session.activateRunningAuthority();
    runtime = await session.createBackgroundRuntime({ contractVersion: 1, maxInboundBatches: 16, maxInboundCommands: 16,
      maxInboundOwnedBytes: 2 * 1024 * 1024, maxBatchCommands: 1, maxBatchOwnedBytes: 1024 * 1024,
      maxOutputReliable: 32, maxOutputReliableOwnedBytes: 16 * 1024 * 1024, maxOutputDiscrete: 4,
      maxOutputDiscreteOwnedBytes: 1024 * 1024, maxOutputTotalOwnedBytes: 32 * 1024 * 1024,
      maxOutputEventOwnedBytes: 1024 * 1024, maxOutputFrameConnections: 4 }, () => {}) as ExperimentalRunningAuthorityNativeHandle;
    runtime.start();
    expect(() => runtime!.submitActivateShadow('0000000000000001', { expectedRunId: 'candidate-cache-handoff',
      expectedConfigRevision: '0000000000000001', captureEpoch: '17'.repeat(16), shadowSlot: 0, baselineSlot: 0, artifactSha256: identity })).not.toThrow();
    const deadline = performance.now() + 5000; let rejection: string | undefined;
    while (rejection === undefined && performance.now() < deadline) {
      for (const event of runtime.drainOutputs(32, 1024 * 1024).events) if (event.kind === 'commandRejected' && event.commandSequence === '0000000000000001') rejection = event.rejectionDetail;
      await new Promise(done => setTimeout(done, 5));
    }
    expect(rejection).toMatch(/requires active teacher capture/u);
    expect(JSON.parse(runtime.getShadowMetrics()).slots).toEqual([null, null, null, null]);
  } finally {
    if (runtime) { runtime.requestStop(); await runtime.join(); }
    await persistence.close();
  }
});
