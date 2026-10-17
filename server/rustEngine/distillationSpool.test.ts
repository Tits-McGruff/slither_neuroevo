import { DistillationClient as BrowserDistillationClient } from '../../src/net/distillation.ts';
import { createHash } from 'node:crypto';
import Database from 'better-sqlite3';
import { createServer } from 'node:http';
import { mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { TeacherCaptureDrain, TeacherChunkTransfer } from '../../src/protocol/distillation.ts';
import { DISTILLATION_SPOOL_DEFAULTS, DISTILLATION_ZERO, distillationSpoolLimitsForGiB, parseStoragePolicy, parseTeacherDrain, type DistillationSpoolLimits } from './distillationProtocol.ts';
import { DistillationSpoolStore, type DistillationSpoolFailpoint } from './distillationSpoolStore.ts';
import { DistillationSpoolClient } from './distillationSpoolClient.ts';
import { DistillationHttpRouter } from './distillationHttp.ts';
import { networkTest } from '../test/networkSuites.ts';
import { DistillationCapture } from './distillationCapture.ts';
import { BackgroundCommandAdmission } from './commandAdmission.ts';
import type { ExperimentalRunningAuthorityNativeHandle } from './backgroundRuntime.ts';
import type { TeacherCaptureRequest } from '../../src/protocol/distillation.ts';
import type { RustBackgroundEvent } from '../../src/protocol/rustBackground.ts';

/** Frozen Rust canonical fixture; this suite verifies opaque storage rather than encoding. */
const sensor = (JSON.parse(readFileSync(resolve(import.meta.dirname, '../../native/fixtures/sensor-contract-shapes.json'), 'utf8')) as
  Array<{ profile: string; bins: number; canonicalJson: string; sha256: string }>).find(entry => entry.profile === 'motion' && entry.bins === 8)!;
/** Test-owned isolated roots only. */
const roots: string[] = [];
/** Direct stores closed before cleaning their exact fixture roots. */
const stores: DistillationSpoolStore[] = [];
/** Actual isolated workers shut down before fixture deletion. */
const clients: DistillationSpoolClient[] = [];

/** Encode a small test counter without compromising production u64 handling. */
function word(value: number): string { return value.toString(16).padStart(16, '0'); }

/** Create synthetic opaque bytes; no JavaScript sample codec is introduced. */
function drain(sequences: number[], options: { production?: number; released?: number; stopped?: boolean; epoch?: string; size?: number } = {}): TeacherCaptureDrain {
  const epoch = options.epoch ?? '12'.repeat(16);
  const contract = { captureEpoch: epoch, runId: 'run', configSha256: '23'.repeat(32),
    sensorSha256: sensor.sha256, teacherSha256: '37'.repeat(32), teacherRevision: 2,
    teacherMode: 1, controlIntervalSeconds: 0.06, inputSize: 144 };
  const chunks: TeacherChunkTransfer[] = sequences.map(sequence => {
    const bytes = new Uint8Array(options.size ?? 32).fill(sequence);
    bytes.set([0, 0, 0, 128]); // Opaque negative-zero bit pattern must survive exactly.
    return { descriptor: { captureEpoch: epoch, sequence: word(sequence), generation: word(1), baselineSlot: 7,
      physicalSnakeId: word(71), firstSample: word(sequence), lastSample: word(sequence), firstStep: word(sequence), lastStep: word(sequence),
      count: 1, inputSize: 144, configSha256: contract.configSha256, sensorSha256: contract.sensorSha256,
      teacherSha256: contract.teacherSha256, startReason: 'continuation', endReason: 'simulatedSecond', byteCount: bytes.byteLength,
      sha256: createHash('sha256').update(bytes).digest('hex') }, bytes };
  });
  const production = options.production ?? sequences.at(-1) ?? 0;
  return { contract, sensorContractJson: sensor.canonicalJson, chunks, productionWatermark: word(production),
    releasedThrough: word(options.released ?? production), queuedChunks: 0, queuedBytes: DISTILLATION_ZERO,
    droppedChunks: word(Math.max(0, production - sequences.length)), droppedSamples: DISTILLATION_ZERO,
    droppedBytes: DISTILLATION_ZERO, stopped: options.stopped ?? false, moreWork: false };
}

/** Create one independently owned dedicated spool, using actual filesystem reserve checks. */
function fixture(overrides: Partial<DistillationSpoolLimits> = {}, failpoint?: DistillationSpoolFailpoint): DistillationSpoolStore {
  const root = mkdtempSync(join(tmpdir(), 'slither-distillation-spool-'));
  roots.push(root);
  const store = new DistillationSpoolStore(root, { ...DISTILLATION_SPOOL_DEFAULTS, ...overrides }, failpoint);
  stores.push(store);
  return store;
}

afterEach(async () => {
  vi.restoreAllMocks();
  await Promise.all(clients.splice(0).map(client => client.close()));
  stores.splice(0).forEach(store => store.close());
  for (const root of roots.splice(0)) {
    if (dirname(resolve(root)) !== resolve(tmpdir()) || !root.includes('slither-distillation-spool-')) throw new Error('unsafe test cleanup');
    rmSync(root, { recursive: true, force: true });
  }
});

describe('durable distillation spool', () => {
  it.each([1, 4, 64])('admits a %i-GiB disk ceiling while writing only actual opaque objects', quota => {
    const limits = distillationSpoolLimitsForGiB(quota);
    const store = fixture(limits);
    const source = drain([1]);
    expect(store.ingest(source, 42).retainedBytes).toBe(word(32));
    expect(store.limits).toEqual({ ...DISTILLATION_SPOOL_DEFAULTS, maxBytes: quota * 1024 ** 3 });
    const lease = store.lease(source.contract!.captureEpoch, [word(1)], 60_000);
    expect([...store.download(lease.captureEpoch, word(1), lease.id).bytes]).toEqual([...source.chunks[0]!.bytes]);
    expect(readdirSync(store.root).filter(file => file.endsWith('.teacher'))).toHaveLength(1);
  });

  it('reports worker-admitted global disk policy without allocating the selected ceiling', async () => {
    const root = mkdtempSync(join(tmpdir(), 'slither-distillation-spool-')); roots.push(root);
    const client = new DistillationSpoolClient(root, distillationSpoolLimitsForGiB(64)); clients.push(client);
    const policy = await client.storagePolicy();
    expect(policy).toEqual({ schemaVersion: 1, scope: 'producer-global', maxBytes: '0000001000000000',
      maxChunks: 65_536, maxDatasets: 256, maxLeases: 64, maxPinnedChunks: 1024,
      maxPinnedBytes: '0000000020000000', maxLeaseMs: 300_000, diskReserveBytes: '0000000040000000' });
    policy.maxBytes = word(1);
    expect((await client.storagePolicy()).maxBytes).toBe('0000001000000000');
    expect(readdirSync(root).filter(file => file.endsWith('.teacher'))).toEqual([]);
  });

  it.each([
    { schemaVersion: true }, { scope: 'job-local' }, { maxBytes: word(0) },
    { maxBytes: '0000001000000001' }, { maxChunks: true }, { maxLeaseMs: 300_001 },
    { maxPinnedBytes: '0000000020000001' }, { diskReserveBytes: '0020000000000000' },
    { unexpected: 1 }
  ])('rejects malformed storage policy metadata: %j', change => {
    const policy = { schemaVersion: 1, scope: 'producer-global', maxBytes: '0000000100000000',
      maxChunks: 65_536, maxDatasets: 256, maxLeases: 64, maxPinnedChunks: 1024,
      maxPinnedBytes: '0000000020000000', maxLeaseMs: 300_000, diskReserveBytes: '0000000040000000' };
    expect(() => parseStoragePolicy({ ...policy, ...change })).toThrow();
  });

  it('refuses to report an admitted policy when retained storage cannot pass startup admission', async () => {
    const store = fixture({ maxBytes: 96 });
    const source = drain([1, 2]); store.ingest(source, 42);
    store.lease(source.contract!.captureEpoch, [word(1), word(2)], 60_000);
    store.close();
    const client = new DistillationSpoolClient(store.root, { ...DISTILLATION_SPOOL_DEFAULTS, maxBytes: 32 });
    clients.push(client);
    await expect(client.storagePolicy()).rejects.toThrow(/active leased objects/u);
  });

  it('applies a smaller restart quota by rotating oldest unpinned objects while preserving leases and watermarks', () => {
    const store = fixture({ maxBytes: 96, maxChunks: 3 });
    const source = drain([1, 2, 3]); store.ingest(source, 42);
    const lease = store.lease(source.contract!.captureEpoch, [word(1)], 60_000);
    store.close();
    const restored = new DistillationSpoolStore(store.root, { ...DISTILLATION_SPOOL_DEFAULTS, maxBytes: 64, maxChunks: 3 });
    stores.push(restored);
    expect(restored.list(lease.captureEpoch).chunks.map(chunk => chunk.sequence)).toEqual([word(1), word(3)]);
    expect(restored.dataset(lease.captureEpoch)).toMatchObject({ retainedBytes: word(64), retainedChunks: 2,
      productionWatermark: word(3), storageWatermark: word(3), resolvedThrough: word(3), retentionFloor: word(1) });
    expect([...restored.download(lease.captureEpoch, word(1), lease.id).bytes]).toEqual([...source.chunks[0]!.bytes]);
    restored.close();
    const exact = new DistillationSpoolStore(store.root, { ...DISTILLATION_SPOOL_DEFAULTS, maxBytes: 64, maxChunks: 2 });
    stores.push(exact);
    expect(exact.dataset(lease.captureEpoch).retainedChunks).toBe(2);
  });

  it('charges overlapping leases once during restart quota admission', () => {
    const store = fixture({ maxBytes: 96 });
    const source = drain([1, 2, 3]); store.ingest(source, 42);
    const first = store.lease(source.contract!.captureEpoch, [word(1)], 60_000);
    const second = store.lease(source.contract!.captureEpoch, [word(1)], 60_000);
    store.close();
    const restored = new DistillationSpoolStore(store.root, { ...DISTILLATION_SPOOL_DEFAULTS, maxBytes: 32 });
    stores.push(restored);
    expect(restored.dataset(first.captureEpoch)).toMatchObject({ retainedBytes: word(32), retainedChunks: 1 });
    expect([...restored.download(first.captureEpoch, word(1), first.id).bytes]).toEqual([...source.chunks[0]!.bytes]);
    restored.releaseLease(first.id);
    expect([...restored.download(second.captureEpoch, word(1), second.id).bytes]).toEqual([...source.chunks[0]!.bytes]);
  });

  it('refuses a restart quota smaller than active pins before evicting any complete object', () => {
    const store = fixture({ maxBytes: 96 });
    const source = drain([1, 2, 3]); store.ingest(source, 42);
    const lease = store.lease(source.contract!.captureEpoch, [word(1), word(2)], 60_000);
    const files = readdirSync(store.root).filter(file => file.endsWith('.teacher')).sort();
    store.close();
    expect(() => new DistillationSpoolStore(store.root, { ...DISTILLATION_SPOOL_DEFAULTS, maxBytes: 32 }))
      .toThrow(/active leased objects/u);
    expect(readdirSync(store.root).filter(file => file.endsWith('.teacher')).sort()).toEqual(files);
    const restored = new DistillationSpoolStore(store.root, { ...DISTILLATION_SPOOL_DEFAULTS, maxBytes: 96 });
    stores.push(restored);
    expect(restored.dataset(lease.captureEpoch).retainedChunks).toBe(3);
    for (const sequence of [1, 2]) expect([...restored.download(lease.captureEpoch, word(sequence), lease.id).bytes])
      .toEqual([...source.chunks[sequence - 1]!.bytes]);
  });

  it('binds the recorded run seed and rejects mismatched immutable provenance', () => {
    const store = fixture();
    const source = drain([1]);
    expect(store.ingest(source, 0xffff_ffff).runSeed).toBe(0xffff_ffff);
    expect(() => store.ingest(source, 42)).toThrow(/run seed mismatch/u);
    expect(() => store.ingest(source, -1)).toThrow(/run seed/u);
    expect(store.dataset(source.contract!.captureEpoch).runSeed).toBe(0xffff_ffff);
  });

  it('migrates old diagnostic metadata without guessing a missing run seed', () => {
    const store = fixture();
    const source = drain([1], { stopped: true });
    store.ingest(source, 42);
    const lease = store.lease(source.contract!.captureEpoch, [word(1)], 1000);
    store.close();
    const old = new Database(join(store.root, 'metadata.sqlite'));
    old.exec('ALTER TABLE datasets DROP COLUMN run_seed');
    old.pragma('user_version = 1');
    old.close();
    const reopened = new DistillationSpoolStore(store.root);
    stores.push(reopened);
    expect(reopened.dataset(lease.captureEpoch).runSeed).toBeNull();
    expect([...reopened.download(lease.captureEpoch, word(1), lease.id).bytes]).toEqual([...source.chunks[0]!.bytes]);
    reopened.ingest(source, 42);
    expect(reopened.dataset(lease.captureEpoch).runSeed).toBeNull();
  });

  it('preserves exact opaque bytes, visible gaps and paginated coherent watermarks', () => {
    const store = fixture();
    const source = drain([1, 3], { production: 4, released: 4 });
    const dataset = store.ingest(source, 42);
    expect(dataset).toMatchObject({ productionWatermark: word(4), storageWatermark: word(3), resolvedThrough: word(4),
      retentionFloor: word(1), nativeDroppedChunks: word(2), retainedChunks: 2 });
    const first = store.list(source.contract!.captureEpoch, DISTILLATION_ZERO, 1);
    expect(first.chunks.map(chunk => chunk.sequence)).toEqual([word(1)]);
    expect(first.hasMore).toBe(true);
    const second = store.list(source.contract!.captureEpoch, first.nextCursor, 1);
    expect(second.chunks.map(chunk => chunk.sequence)).toEqual([word(3)]);
    expect(second.hasMore).toBe(false);
    const lease = store.lease(source.contract!.captureEpoch, [word(1)], 1000);
    expect([...store.download(lease.captureEpoch, word(1), lease.id).bytes]).toEqual([...source.chunks[0]!.bytes]);
    expect(() => store.download(lease.captureEpoch, word(3), lease.id)).toThrow(/unavailable|expired/u);
    store.ingest(source, 42);
    expect(store.dataset(lease.captureEpoch)).toEqual(dataset);
  });

  it('validates the whole handoff before changing any files or metadata', () => {
    const store = fixture();
    const source = drain([1, 2]);
    source.chunks[1]!.bytes[0] = 99;
    expect(() => store.ingest(source, 42)).toThrow(/digest/u);
    expect(readdirSync(store.root).filter(name => name.endsWith('.teacher'))).toEqual([]);
    expect(() => store.dataset(source.contract!.captureEpoch)).toThrow(/not found/u);
    const invalid = drain([1]);
    invalid.sensorContractJson += ' ';
    expect(() => parseTeacherDrain(invalid)).toThrow(/digest/u);
    const unordered = drain([2, 1]);
    expect(() => store.ingest(unordered, 42)).toThrow(/ordering/u);
  });

  it('rotates only oldest unpinned complete objects and never reinstalls settled evictions', () => {
    const store = fixture({ maxChunks: 2, maxBytes: 64 });
    const first = drain([1, 2]);
    store.ingest(first, 42);
    const lease = store.lease(first.contract!.captureEpoch, [word(1)], 1000);
    const third = store.ingest(drain([3]), 42);
    expect(store.list(lease.captureEpoch).chunks.map(chunk => chunk.sequence)).toEqual([word(1), word(3)]);
    expect(third.retentionFloor).toBe(word(1));
    store.ingest(first, 42);
    expect(store.list(lease.captureEpoch).chunks.map(chunk => chunk.sequence)).toEqual([word(1), word(3)]);
    store.releaseLease(lease.id);
    store.ingest(drain([4]), 42);
    expect(store.list(lease.captureEpoch).chunks.map(chunk => chunk.sequence)).toEqual([word(3), word(4)]);
    expect(store.dataset(lease.captureEpoch).retentionFloor).toBe(word(3));
  });

  it('drops new objects under pinned saturation while settling production independently of storage', () => {
    const store = fixture({ maxChunks: 1, maxBytes: 32 });
    const source = drain([1]); store.ingest(source, 42);
    const lease = store.lease(source.contract!.captureEpoch, [word(1)], 1000);
    const dropped = store.ingest(drain([2]), 42);
    expect(dropped).toMatchObject({ productionWatermark: word(2), storageWatermark: word(1), resolvedThrough: word(2),
      spoolDroppedChunks: word(1), retainedChunks: 1 });
    store.ingest(drain([2]), 42);
    expect(store.dataset(lease.captureEpoch).spoolDroppedChunks).toBe(word(1));
    expect([...store.download(lease.captureEpoch, word(1), lease.id).bytes]).toEqual([...source.chunks[0]!.bytes]);
  });

  it.each(['after-write', 'after-rename'] as const)('never advertises a publication interrupted %s', failpoint => {
    const store = fixture({}, failpoint);
    const source = drain([1]);
    expect(() => store.ingest(source, 42)).toThrow(/injected/u);
    expect(store.list(source.contract!.captureEpoch).chunks).toEqual([]);
    expect(readdirSync(store.root).filter(name => name.includes('.teacher'))).toEqual([]);
    expect(store.ingest(source, 42).retainedChunks).toBe(1);
  });

  it('keeps a committed object after a lost reply and deduplicates exact retry', () => {
    const store = fixture({}, 'after-metadata');
    const source = drain([1]);
    expect(() => store.ingest(source, 42)).toThrow(/injected/u);
    expect(store.dataset(source.contract!.captureEpoch)).toMatchObject({ productionWatermark: word(1), storageWatermark: word(1),
      resolvedThrough: DISTILLATION_ZERO, retainedChunks: 1 });
    expect(store.ingest(source, 42)).toMatchObject({ resolvedThrough: word(1), retainedChunks: 1 });
  });

  it('labels interrupted recovery using only the durable watermark and retains offline downloads', () => {
    const store = fixture(); const source = drain([1]); store.ingest(source, 42); store.close();
    const restored = new DistillationSpoolStore(store.root); stores.push(restored);
    expect(restored.dataset(source.contract!.captureEpoch)).toMatchObject({ state: 'interrupted', productionWatermark: word(1), resolvedThrough: word(1) });
    expect(() => restored.ingest(drain([2]), 42)).toThrow(/cannot resume/u);
    const lease = restored.lease(source.contract!.captureEpoch, [word(1)], 1000);
    expect([...restored.download(lease.captureEpoch, word(1), lease.id).bytes]).toEqual([...source.chunks[0]!.bytes]);
    expect(restored.ingest(source, 42).state).toBe('interrupted');
  });

  it('reconciles corrupt retained bytes as unavailable without advertising a partial object', () => {
    const store = fixture(); const source = drain([1]); store.ingest(source, 42); store.close();
    const filename = readdirSync(store.root).find(name => name.endsWith('.teacher'))!;
    writeFileSync(join(store.root, filename), Uint8Array.of(1));
    const restored = new DistillationSpoolStore(store.root); stores.push(restored);
    expect(restored.list(source.contract!.captureEpoch)).toMatchObject({ chunks: [], hasMore: false,
      dataset: { storageWatermark: word(1), resolvedThrough: word(1), retainedChunks: 0, retentionFloor: word(2) } });
  });

  it('bounds leases, renewals and expired downloads independently of game ownership', () => {
    const store = fixture({ maxLeases: 1, maxPinnedChunks: 1, maxPinnedBytes: 32 });
    const source = drain([1, 2]); store.ingest(source, 42);
    expect(() => store.lease(source.contract!.captureEpoch, [word(1), word(2)], 1000)).toThrow(/pin quota/u);
    const now = Date.now();
    const clock = vi.spyOn(Date, 'now').mockReturnValue(now);
    const lease = store.lease(source.contract!.captureEpoch, [word(1)], 10);
    expect(() => store.lease(source.contract!.captureEpoch, [word(2)], 1000)).toThrow(/count quota/u);
    expect(store.renewLease(lease.id, 20)).toBeGreaterThanOrEqual(lease.expiresAt);
    clock.mockReturnValue(now + 30);
    expect(() => store.download(lease.captureEpoch, word(1), lease.id)).toThrow(/expired/u);
    expect(() => store.renewLease(lease.id, 1)).toThrow(/expired/u);
    store.releaseLease(lease.id); store.releaseLease(lease.id);
  });

  it('deduplicates a refused object across a partially published handoff and lost reply', () => {
    const store = fixture({ maxBytes: 32 }, 'after-metadata');
    const source = drain([2, 3]);
    const refused = source.chunks[0]!;
    refused.bytes = new Uint8Array(64).fill(2);
    refused.descriptor.byteCount = 64;
    refused.descriptor.sha256 = createHash('sha256').update(refused.bytes).digest('hex');
    expect(() => store.ingest(source, 42)).toThrow(/injected/u);
    expect(store.dataset(source.contract!.captureEpoch)).toMatchObject({ spoolDroppedChunks: word(1),
      productionWatermark: word(3), storageWatermark: word(3), resolvedThrough: DISTILLATION_ZERO });
    expect(store.ingest(source, 42)).toMatchObject({ spoolDroppedChunks: word(1), resolvedThrough: word(3), retainedChunks: 1 });
  });

  it('preserves the actual filesystem reserve and records an unavailable object without a file', () => {
    const store = fixture({ diskReserveBytes: Number.MAX_SAFE_INTEGER });
    const source = drain([1]);
    expect(store.ingest(source, 42)).toMatchObject({ spoolDroppedChunks: word(1), retainedChunks: 0,
      productionWatermark: word(1), storageWatermark: DISTILLATION_ZERO, resolvedThrough: word(1) });
    expect(readdirSync(store.root).filter(name => name.endsWith('.teacher'))).toEqual([]);
  });

  it('runs opaque ingestion, pins and downloads through the actual isolated worker', async () => {
    const root = mkdtempSync(join(tmpdir(), 'slither-distillation-spool-')); roots.push(root);
    const client = new DistillationSpoolClient(root); clients.push(client); await client.ready;
    const source = drain([1, 3], { production: 4 });
    const original = new Uint8Array(source.chunks[0]!.bytes);
    const active = client.ingest(source, 42);
    await expect(client.ingest(source, 42)).rejects.toThrow(/already active/u);
    await active;
    expect(source.chunks[0]!.bytes).toEqual(original); // Transferring the worker copy never detaches native/caller bytes.
    const page = await client.list(source.contract!.captureEpoch, DISTILLATION_ZERO, 1);
    expect(page).toMatchObject({ hasMore: true, nextCursor: word(1), dataset: { resolvedThrough: word(4) } });
    const lease = await client.lease(source.contract!.captureEpoch, [word(1)], 1000);
    expect((await client.download(lease.captureEpoch, word(1), lease.id)).bytes).toEqual(original);
    await client.releaseLease(lease.id);
    await client.interrupt(lease.captureEpoch);
    expect((await client.dataset(lease.captureEpoch)).state).toBe('interrupted');
    await Promise.all([client.close(), client.close()]);
  });

  networkTest('serves independent metadata/pagination, leased immutable bytes and expiry over real HTTP', async () => {
    const root = mkdtempSync(join(tmpdir(), 'slither-distillation-spool-')); roots.push(root);
    const client = new DistillationSpoolClient(root); clients.push(client); await client.ready;
    const source = drain([1, 3], { production: 4 }); await client.ingest(source, 42);
    const router = new DistillationHttpRouter(client);
    const server = createServer((request, response) => {
      if (!router.handle(request, response, new URL(request.url ?? '/', 'http://localhost'))) { response.writeHead(404); response.end(); }
    });
    await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve));
    const address = server.address(); if (!address || typeof address === 'string') throw new Error('missing test HTTP address');
    const base = `http://127.0.0.1:${address.port}/api/distillation/v1`;
    const epoch = source.contract!.captureEpoch;
    try {
      const policy = await (await fetch(`${base}/storage`)).json() as { schemaVersion: number; scope: string; maxBytes: string };
      expect(policy).toMatchObject({ schemaVersion: 1, scope: 'producer-global', maxBytes: '0000000100000000' });
      expect((await fetch(`${base}/storage`, { method: 'POST' })).status).toBe(404);
      expect(await (await fetch(`${base}/storage`)).json()).toEqual(policy);
      const discovered = await (await fetch(`${base}/datasets`)).json() as { datasets: Array<{ captureEpoch: string }> };
      expect(discovered.datasets.map(dataset => dataset.captureEpoch)).toEqual([epoch]);
      const manifest = await (await fetch(`${base}/datasets/${epoch}/manifest`)).json() as { sensorContractJson: string };
      expect(manifest.sensorContractJson).toBe(sensor.canonicalJson);
      const browser = new BrowserDistillationClient(() => ({base: `http://127.0.0.1:${address.port}`,
        runId: source.contract!.runId, configRevision: '0000000000000001'}));
      expect((await browser.datasets()).datasets.map(dataset => dataset.captureEpoch)).toEqual([epoch]);
      expect((await browser.manifest(epoch)).contract).toEqual(source.contract);
      const first = await (await fetch(`${base}/datasets/${epoch}/chunks?limit=1`)).json() as { nextCursor: string; hasMore: boolean };
      expect(first).toMatchObject({ nextCursor: word(1), hasMore: true });
      const second = await (await fetch(`${base}/datasets/${epoch}/chunks?after=${first.nextCursor}&limit=1`)).json() as { nextCursor: string; hasMore: boolean };
      expect(second).toMatchObject({ nextCursor: word(3), hasMore: false });
      expect((await fetch(`${base}/datasets/${epoch}/chunks?limit=1oops`)).status).toBe(400);
      const leaseResponse = await fetch(`${base}/datasets/${epoch}/leases`, { method: 'POST',
        headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ sequences: [word(1)], durationMs: 30_000 }) });
      expect(leaseResponse.status).toBe(201);
      const lease = await leaseResponse.json() as { id: string };
      const download = await fetch(`${base}/datasets/${epoch}/chunks/${word(1)}?lease=${lease.id}`);
      expect(download.status).toBe(200);
      expect(download.headers.get('x-slither-chunk-sha256')).toBe(source.chunks[0]!.descriptor.sha256);
      expect(new Uint8Array(await download.arrayBuffer())).toEqual(source.chunks[0]!.bytes);
      expect((await fetch(`${base}/leases/${lease.id}/renew`, { method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ durationMs: 30_000 }) })).status).toBe(200);
      expect((await fetch(`${base}/leases/${lease.id}`, { method: 'DELETE' })).status).toBe(200);
      expect((await fetch(`${base}/datasets/${epoch}/chunks/${word(1)}?lease=${lease.id}`)).status).toBe(410);
      expect((await fetch(`${base}/datasets/${'34'.repeat(16)}/manifest`)).status).toBe(404);
      expect((await fetch(`${base}/candidates`, { method: 'POST', body: 'pickle is not a deployed graph' })).status).toBe(404);
    } finally { await new Promise<void>((resolve, reject) => server.close(error => error ? reject(error) : resolve())); }
  });

  it('coalesces capture/spool work and uses only ordered diagnostic arm/stop commands', async () => {
    const root = mkdtempSync(join(tmpdir(), 'slither-distillation-spool-')); roots.push(root);
    const client = new DistillationSpoolClient(root); clients.push(client); await client.ready;
    let capture!: DistillationCapture;
    let source: TeacherCaptureDrain | undefined;
    let commands = 0;
    /** Deliver the exact diagnostic receipt with no actor/controller actions. */
    const receipt = (sequence: string, epoch: string, enabled: boolean): void => capture.event({
      kind: 'teacherCaptureChanged', commandSequence: sequence, teacherCaptureEpoch: epoch, teacherCaptureEnabled: enabled
    } as RustBackgroundEvent);
    const native = {
      submitStartTeacherCapture(sequence: string, request: TeacherCaptureRequest) {
        commands += 1; source = drain([1], { epoch: request.captureEpoch, production: 3 }); receipt(sequence, request.captureEpoch, true);
      },
      submitStopTeacherCapture(sequence: string, epoch: string) {
        commands += 1; source!.stopped = true; receipt(sequence, epoch, false);
      },
      drainTeacherChunks() {
        if (!source) return { ...drain([]), contract: null, sensorContractJson: null };
        const result = source; source = { ...source, chunks: [] }; return result;
      }
    } as unknown as ExperimentalRunningAuthorityNativeHandle;
    const admission = new BackgroundCommandAdmission(native);
    capture = new DistillationCapture(() => native, admission, client, () => ({ runId: 'run', seed: 42 }));
    try {
      const dataset = await capture.start({ expectedRunId: 'run', expectedConfigRevision: word(1), baselineSlots: [7],
        pendingBytes: 1024 * 1024, queueChunks: 4, queueBytes: 16 * 1024 * 1024 });
      expect(dataset).toMatchObject({ state: 'active', storageWatermark: word(1), resolvedThrough: word(3) });
      expect(admission.nextSequence()).toBe(word(2));
      const first = capture.poll(); expect(capture.poll()).toBe(first); await first;
      const stopped = await capture.stop(dataset.contract.captureEpoch);
      expect(stopped.state).toBe('stopped');
      expect(commands).toBe(2);
      expect(admission.nextSequence()).toBe(word(3));
    } finally { await capture.close(); }
  });
});
