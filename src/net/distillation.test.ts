import type { ShadowPublication } from '../protocol/shadows.ts';
import { afterEach, expect, it, vi } from 'vitest';
import { DistillationClient, type DistillationScope } from './distillation.ts';

/** Exact diagnostic words deliberately include values beyond JavaScript integer precision. */
const EPOCH = 'ab'.repeat(16);
/** Immutable complete artifact identity. */
const ARTIFACT = 'cd'.repeat(32);
/** Current process identity, separate from game/RNG identities. */
const PROCESS = 'ef'.repeat(16);
/** Fixed publication with no active candidates. */
const EMPTY: ShadowPublication = { schemaVersion: 1, processEpoch: PROCESS, slots: [null, null, null, null] };
/** Fully bounded manifest; the browser does not decode its sample objects. */
const DATASET = { schemaVersion: 1, runSeed: 42, contract: { captureEpoch: EPOCH, runId: 'run-one',
  configSha256: '11'.repeat(32), sensorSha256: '22'.repeat(32), teacherSha256: '33'.repeat(32),
  teacherRevision: 2, teacherMode: 1, controlIntervalSeconds: 1 / 60, inputSize: 264 },
sensorContractJson: '{}', state: 'active', productionWatermark: 'fffffffffffffffe', storageWatermark: 'fffffffffffffffd',
retentionFloor: '0000000000000001', resolvedThrough: 'fffffffffffffffc', nativeDroppedChunks: '0000000000000000',
nativeDroppedSamples: '0000000000000000', nativeDroppedBytes: '0000000000000000', spoolDroppedChunks: '0000000000000000',
retainedChunks: 1, retainedBytes: '0000000000010000', error: null };

/** Build exact request confirmations without implementing any shadow inference. */
function receipt(body: Record<string, unknown>, slot = 0) {
  return { schemaVersion: 1, processEpoch: body['expectedProcessEpoch'], requestId: body['requestId'],
    commandSequence: '0000000000000002', activationSequence: '0000000000000002',
    previousActivationSequence: body['expectedActivationSequence'], runId: body['expectedRunId'],
    configRevision: body['expectedConfigRevision'], captureEpoch: body['captureEpoch'], artifactSha256: body['artifactSha256'],
    shadowSlot: slot, baselineSlot: body['baselineSlot'], effectiveStep: '0000000000000042', enabled: true, acknowledgedAt: 1234 };
}

/** Restore the browser transport between independent diagnostic tests. */
afterEach(() => vi.unstubAllGlobals());

it('uses only dedicated diagnostic routes and sends applied identities and bounded capture budgets', async () => {
  const fetch = vi.fn(async () => new Response(JSON.stringify(DATASET)));
  vi.stubGlobal('fetch', fetch);
  const client = new DistillationClient(() => ({ base: 'http://server/', runId: 'run-one', configRevision: '0000000000000001' }));
  expect(fetch).not.toHaveBeenCalled();
  await client.start([0, 2, 9]);
  expect(fetch).toHaveBeenCalledTimes(1);
  const [url, init] = fetch.mock.calls[0]! as unknown as [string, RequestInit];
  expect(url).toBe('http://server/api/distillation/v1/capture/start');
  expect(JSON.parse(init.body as string)).toEqual({ expectedRunId: 'run-one', expectedConfigRevision: '0000000000000001',
    baselineSlots: [0, 2, 9], pendingBytes: 16 * 1024 ** 2, queueBytes: 16 * 1024 ** 2, queueChunks: 1024 });
  expect(init.signal).toBeInstanceOf(AbortSignal);
});

it('never replays an uncertain server-assigned capture start and keeps stop pinned to an explicit epoch', async () => {
  const fetch = vi.fn().mockRejectedValueOnce(new Error('lost response')).mockResolvedValue(new Response(JSON.stringify({...DATASET, state: 'stopped'})));
  vi.stubGlobal('fetch', fetch);
  const client = new DistillationClient(() => ({ base: 'http://server', runId: 'run-one', configRevision: '0000000000000001' }));
  await expect(client.start([0])).rejects.toThrow('lost response');
  expect(client.hasUncertainCapture()).toBe(true);
  await expect(client.start([0])).rejects.toThrow('Discover');
  expect(fetch).toHaveBeenCalledTimes(1);
  await client.stop(EPOCH);
  expect(fetch.mock.calls[1]![0]).toBe('http://server/api/distillation/v1/capture/' + EPOCH + '/stop');
  client.dismissUncertainCapture(); expect(client.hasUncertainCapture()).toBe(false);
});

it('rejects invalid teacher selections before allocating a request or sending gameplay traffic', async () => {
  const fetch = vi.fn(); vi.stubGlobal('fetch', fetch);
  const client = new DistillationClient(() => ({ base: 'http://server', runId: 'run-one', configRevision: '0000000000000001' }));
  for (const slots of [[], [0, 0], [2, 1], [-1], [120], [0.5]]) await expect(client.start(slots)).rejects.toThrow('teacher slots');
  expect(fetch).not.toHaveBeenCalled();
});

it('preserves original shadow bodies after an uncertain reply and rejects retargeting to a replacement authority', async () => {
  let scope: DistillationScope = { base: 'http://server', runId: 'run-one', configRevision: '0000000000000001' };
  const fetch = vi.fn().mockRejectedValueOnce(new Error('lost response')).mockImplementation(async (_url: string, init: RequestInit) =>
    new Response(JSON.stringify(receipt(JSON.parse(init.body as string) as Record<string, unknown>))));
  vi.stubGlobal('fetch', fetch);
  const client = new DistillationClient(() => scope);
  await expect(client.activate(0, 9, ARTIFACT, EPOCH, EMPTY)).rejects.toThrow('lost response');
  expect(client.hasPendingShadow()).toBe(true);
  await expect(client.activate(1, 9, ARTIFACT, EPOCH, EMPTY)).rejects.toThrow('pending');
  scope = {...scope, runId: 'replacement'};
  await expect(client.retryShadow()).rejects.toThrow('Authority changed');
  expect(fetch).toHaveBeenCalledTimes(1);
  scope = {...scope, runId: 'run-one'};
  const result = await client.retryShadow();
  expect(fetch.mock.calls[1]![1].body).toBe(fetch.mock.calls[0]![1].body);
  expect(result.artifactSha256).toBe(ARTIFACT); expect(client.hasPendingShadow()).toBe(false);
});

it('does not accept a valid-shaped receipt for another candidate and clears only confirmed rejections', async () => {
  const fetch = vi.fn(async (_url: string, init: RequestInit) => new Response(JSON.stringify({
    ...receipt(JSON.parse(init.body as string) as Record<string, unknown>), artifactSha256: '44'.repeat(32) })));
  vi.stubGlobal('fetch', fetch);
  const client = new DistillationClient(() => ({ base: 'http://server', runId: 'run-one', configRevision: '0000000000000001' }));
  await expect(client.activate(0, 0, ARTIFACT, EPOCH, EMPTY)).rejects.toThrow('differs');
  expect(client.hasPendingShadow()).toBe(true);
  fetch.mockImplementation(async () => new Response(JSON.stringify({error: 'candidate unavailable'}), {status: 410}));
  await expect(client.retryShadow()).rejects.toThrow('candidate unavailable');
  expect(client.hasPendingShadow()).toBe(false);
});

it('validates fixed shadow metadata and exact discovery cursors without narrowing u64 words', async () => {
  const summary = { captureEpoch: EPOCH, runId: 'run-one', state: 'active', productionWatermark: DATASET.productionWatermark,
    storageWatermark: DATASET.storageWatermark, resolvedThrough: DATASET.resolvedThrough };
  const fetch = vi.fn().mockResolvedValueOnce(new Response(JSON.stringify({ datasets: [summary], nextCursor: EPOCH, hasMore: false })))
    .mockResolvedValueOnce(new Response(JSON.stringify(DATASET))).mockResolvedValueOnce(new Response(JSON.stringify(EMPTY)))
    .mockResolvedValueOnce(new Response(JSON.stringify({...EMPTY, slots: [null]})));
  vi.stubGlobal('fetch', fetch);
  const client = new DistillationClient(() => ({ base: 'http://server', runId: 'run-one', configRevision: '0000000000000001' }));
  expect((await client.datasets()).datasets[0]?.productionWatermark).toBe('fffffffffffffffe');
  expect((await client.manifest(EPOCH)).contract.captureEpoch).toBe(EPOCH);
  expect(await client.shadows()).toEqual(EMPTY);
  await expect(client.shadows()).rejects.toThrow('slot count');
  expect(fetch.mock.calls.every(([url]) => String(url).startsWith('http://server/api/distillation/v1/'))).toBe(true);
});

it('rejects malformed pagination and cancels oversized streamed metadata', async () => {
  const cancel = vi.fn();
  const stream = new ReadableStream<Uint8Array>({ start(controller) { controller.enqueue(new Uint8Array(1024 * 1024 + 1)); }, cancel });
  const fetch = vi.fn().mockResolvedValueOnce(new Response(JSON.stringify({datasets: [], nextCursor: EPOCH, hasMore: true})))
    .mockResolvedValueOnce(new Response(stream));
  vi.stubGlobal('fetch', fetch);
  const client = new DistillationClient(() => ({ base: 'http://server', runId: 'run-one', configRevision: '0000000000000001' }));
  await expect(client.datasets()).rejects.toThrow('chronology');
  await expect(client.shadows()).rejects.toThrow('limit'); expect(cancel).toHaveBeenCalledTimes(1);
});

it('does not resolve disconnected operations or accept a capture confirmation from another run', async () => {
  const fetch = vi.fn(async () => new Response(JSON.stringify({...DATASET, contract: {...DATASET.contract, runId: 'old'}})));
  vi.stubGlobal('fetch', fetch);
  const offline = new DistillationClient(() => null);
  await expect(offline.start([0])).rejects.toThrow('Connect'); expect(fetch).not.toHaveBeenCalled();
  const client = new DistillationClient(() => ({ base: 'http://server', runId: 'run-one', configRevision: '0000000000000001' }));
  await expect(client.start([0])).rejects.toThrow('another run');
  expect(client.hasUncertainCapture()).toBe(true);
});

it('pins a shadow stop and its retry to the observed activation rather than a later candidate', async () => {
  const status = {...EMPTY, slots: [{enabled: true, activationSequence: '0000000000000003', captureEpoch: EPOCH,
    baselineSlot: 2, artifactSha256: ARTIFACT}, null, null, null]} as unknown as ShadowPublication;
  const fetch = vi.fn().mockRejectedValueOnce(new Error('lost stop response')).mockImplementation(async (_url: string, init: RequestInit) => {
    const body = JSON.parse(init.body as string) as Record<string, unknown>;
    return new Response(JSON.stringify({schemaVersion: 1, processEpoch: PROCESS, requestId: body['requestId'],
      commandSequence: '0000000000000004', activationSequence: body['activationSequence'],
      previousActivationSequence: body['activationSequence'], runId: 'run-one', configRevision: '0000000000000001',
      captureEpoch: body['captureEpoch'], artifactSha256: ARTIFACT, shadowSlot: 0, baselineSlot: 2,
      effectiveStep: '0000000000000044', enabled: false, acknowledgedAt: 1234}));
  });
  vi.stubGlobal('fetch', fetch);
  const client = new DistillationClient(() => ({base: 'http://server', runId: 'run-one', configRevision: '0000000000000001'}));
  await expect(client.stopShadow(0, status)).rejects.toThrow('lost stop response');
  const body = JSON.parse(fetch.mock.calls[0]![1].body as string);
  expect(body).toEqual({requestId: expect.stringMatching(/^[0-9a-f]{32}$/u), captureEpoch: EPOCH, activationSequence: '0000000000000003'});
  expect((await client.retryShadow()).enabled).toBe(false);
  expect(fetch.mock.calls[1]![1].body).toBe(fetch.mock.calls[0]![1].body);
  expect(client.hasPendingShadow()).toBe(false);
});
