import { afterEach, expect, it, vi } from 'vitest';
import { BrainLibraryClient } from './brainLibrary.ts';

/** Synthetic metadata only; native tests own portable model validation. */
const ID = 'ab'.repeat(32);
/** Fixed bounded wire metadata fixture, with no parameters or observations. */
const RECORD = {schemaVersion: 1, brainId: ID, name: 'saved test', artifact: {artifactSha256: ID,
  inputSize: 72, parameterCount: 146, stateSize: 0, sensorSha256: 'cd'.repeat(32),
  architectureKey: 'graph-key', controlIntervalSeconds: 0.06, lifeStateRule: 'preserve', sourceKind: 'manual'},
  graphManifest: {schemaVersion: 1}, provenance: {schemaVersion: 1, sourceKind: 'manual', details: {seed: 42}}};
/** Current scope/cursor fixture returned before a new explicit request. */
const PAGE = {schemaVersion: 1, processEpoch: '12'.repeat(16), deploymentSequence: '0000000000000000',
  scope: {expectedRunId: 'run', expectedConfigRevision: '0000000000000001', expectedWorldEpoch: '0000000000000009'},
  page: {entries: [RECORD], next: null}};

/** Build actual streamed JSON responses so size bounds and decoding are exercised. */
function response(value: unknown, status = 200): Response { return new Response(JSON.stringify(value), {status, headers: {'Content-Type': 'application/json'}}); }
/** Native-shaped confirmation preserves all immutable request fields. */
function confirmation(body: string): object { return {schemaVersion: 1, ...JSON.parse(body), brainId: ID,
  commandSequence: '0000000000000001', snakeId: 42, effectiveStep: '0000000000000002'}; }
/** Restore transport globals after each browser client test. */
afterEach(() => vi.unstubAllGlobals());

it('reuses the exact request after an uncertain response and verifies the receipt scope', async () => {
  const bodies: string[] = [];
  vi.stubGlobal('fetch', vi.fn(async (_url: string, init?: RequestInit) => {
    if (!init?.body) return response(PAGE);
    bodies.push(String(init.body));
    if (bodies.length === 1) throw new Error('lost reply');
    return response(confirmation(String(init.body)));
  }));
  const client = new BrainLibraryClient(() => 'http://localhost:4500');
  await expect(client.spawn(ID)).rejects.toThrow('lost reply');
  expect(client.hasPendingSpawn()).toBe(true);
  await expect(client.spawn(ID)).rejects.toThrow(/pending/);
  const receipt = await client.retrySpawn();
  expect(bodies).toHaveLength(2); expect(bodies[0]).toBe(bodies[1]);
  expect(client.hasPendingSpawn()).toBe(false);
  expect(await client.isCurrent(receipt)).toBe(true);
  expect(await client.isCurrent({...receipt, expectedWorldEpoch: '000000000000000a'})).toBe(false);
});

it('admits only one new request while the initial status refresh is unresolved', async () => {
  let ready!: (value: Response) => void;
  const gate = new Promise<Response>(done => {ready = done;});
  const fetcher = vi.fn(async (_url: string, init?: RequestInit) => init?.body ? response(confirmation(String(init.body))) : gate);
  vi.stubGlobal('fetch', fetcher);
  const client = new BrainLibraryClient(() => 'http://localhost');
  const first = client.spawn(ID);
  await expect(client.spawn(ID)).rejects.toThrow(/pending/);
  ready(response(PAGE)); await first;
  expect(fetcher).toHaveBeenCalledTimes(2);
});

it('uploads the original File and uses a direct export URL without reading model bytes', async () => {
  const file = new File([new Uint8Array(88)], 'original.slither-brain');
  Object.defineProperty(file, 'arrayBuffer', {value: () => {throw new Error('must not decode portable model');}});
  const fetcher = vi.fn(async () => response(RECORD)); vi.stubGlobal('fetch', fetcher);
  const client = new BrainLibraryClient(() => 'http://localhost:4500/');
  expect((await client.importFile(file, 'original')).brainId).toBe(ID);
  expect(fetcher.mock.calls[0]).toEqual(['http://localhost:4500/api/distillation/v1/library/import?name=original',
    expect.objectContaining({body: file, method: 'POST', headers: {'Content-Type': 'application/octet-stream'}})]);
  expect(client.exportUrl(ID)).toBe(`http://localhost:4500/api/distillation/v1/library/${ID}/export`);
  expect(fetcher).toHaveBeenCalledTimes(1);
});

it('keeps an uncertain request bound to its original server and rejects malformed receipts', async () => {
  let base = 'http://first'; let valid = false;
  const fetcher = vi.fn(async (_url: string, init?: RequestInit) => init?.body
    ? response({...confirmation(String(init.body)), brainId: valid ? ID : 'ef'.repeat(32)}) : response(PAGE));
  vi.stubGlobal('fetch', fetcher);
  const client = new BrainLibraryClient(() => base);
  await expect(client.spawn(ID)).rejects.toThrow(/Invalid spawn confirmation/);
  base = 'http://other';
  await expect(client.retrySpawn()).rejects.toThrow(/original server/);
  expect(fetcher).toHaveBeenCalledTimes(2);
  base = 'http://first'; valid = true; await client.retrySpawn();
  expect(client.hasPendingSpawn()).toBe(false);
});

it('rejects oversized streamed metadata and invalid page ordering without any model parsing', async () => {
  vi.stubGlobal('fetch', vi.fn(async () => response({padding: 'x'.repeat(1200 * 1024)})));
  const client = new BrainLibraryClient(() => 'http://localhost');
  await expect(client.list()).rejects.toThrow(/too large/);
  vi.stubGlobal('fetch', vi.fn(async () => response({...PAGE, page: {entries: [RECORD, RECORD], next: null}})));
  await expect(client.list()).rejects.toThrow(/pagination/);
});

it('clears a confirmed rejection while retaining the exact request after a 503 outcome', async () => {
  let status = 400;
  vi.stubGlobal('fetch', vi.fn(async (_url: string, init?: RequestInit) => init?.body
    ? response({error: status === 400 ? 'saved brain graph does not match' : 'native outcome remains pending'}, status) : response(PAGE)));
  const client = new BrainLibraryClient(() => 'http://localhost');
  await expect(client.spawn(ID)).rejects.toThrow(/graph/);
  expect(client.hasPendingSpawn()).toBe(false);
  status = 503; await expect(client.spawn(ID)).rejects.toThrow(/outcome/);
  expect(client.hasPendingSpawn()).toBe(true);
});


it('builds only explicit bounded sorted archive selections without parsing artifact bytes', () => {
  const client = new BrainLibraryClient(() => 'http://localhost:4500/');
  const second = '01'.repeat(32);
  expect(client.archiveUrl([ID, second])).toBe(`http://localhost:4500/api/export/latest?brainIds=${second},${ID}`);
  for (const ids of [[], [ID, ID], ['../bad'], Array.from({length: 101}, () => ID)]) {
    expect(() => client.archiveUrl(ids)).toThrow();
  }
  expect(() => new BrainLibraryClient(() => null).archiveUrl([ID])).toThrow(/Connect/);
});
