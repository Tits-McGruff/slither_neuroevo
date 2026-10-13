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
