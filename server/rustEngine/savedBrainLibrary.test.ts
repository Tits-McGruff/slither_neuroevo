import { afterEach, expect, it, vi } from 'vitest';
import type { SavedBrainCatalogOperation, SavedBrainCatalogResult, SavedBrainRecordV1 } from '../protocol.ts';
import type { ExperimentalRunningAuthorityNativeHandle } from './backgroundRuntime.ts';
import { BackgroundCommandAdmission } from './commandAdmission.ts';
import { SavedBrainLibrary } from './savedBrainLibrary.ts';

/** Synthetic small metadata only; actual codecs and inference are covered by native integration. */
const ID = 'ab'.repeat(32);
/** Complete exact observed scope, without any simulator implementation in the test. */
const SCOPE = {expectedRunId: 'library-unit', expectedConfigRevision: '0000000000000001', expectedWorldEpoch: '0000000000000009'};

/** Deferred metadata/native completion lets tests prove lease lifetime and ordering. */
function deferred<T>(): {promise: Promise<T>; resolve(value: T): void} {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(done => { resolve = done; });
  return {promise, resolve};
}

/** A small command/lease double makes no claim about neural validation or execution. */
function fixture() {
  const events: string[] = [];
  const calls: string[] = [];
  let scope = {...SCOPE};
  let acquiring: Promise<void> = Promise.resolve();
  let joining: Promise<void> = Promise.resolve();
  const native = {
    submitSavedBrainSpawn(sequence: string) { calls.push(sequence); },
    requestStop() { events.push('stop'); },
    async join() { events.push('join'); await joining; }
  } as unknown as ExperimentalRunningAuthorityNativeHandle;
  const record = {brainId: ID, weights: {version: 1, logicalSha256: 'cd'.repeat(32)}} as SavedBrainRecordV1;
  const persistence = {async savedBrainCatalog(request: SavedBrainCatalogOperation): Promise<SavedBrainCatalogResult> {
    events.push(request.action);
    if (request.action === 'acquire') { await acquiring; return {action: 'acquire', record}; }
    if (request.action === 'release') return {action: 'release', leaseId: request.leaseId};
    throw new Error('unexpected unit metadata command');
  }};
  const library = new SavedBrainLibrary(persistence, {downloadCandidate: async () => {throw new Error('no candidate IO in command unit test');}},
    'unused-metadata-only-root', () => native, new BackgroundCommandAdmission(native), () => scope, () => true, () => {});
  return {library, events, calls, setScope(value: typeof SCOPE) {scope = value;},
    setAcquire(value: Promise<void>) {acquiring = value;}, setJoin(value: Promise<void>) {joining = value;}};
}

/** Produce a fresh request from the coordinator's current compare-and-swap cursor. */
function request(library: SavedBrainLibrary, id = '11'.repeat(16)) {
  const status = library.status();
  return {...status.scope, requestId: id, expectedProcessEpoch: status.processEpoch,
    expectedDeploymentSequence: status.deploymentSequence};
}

/** Let one async acquire reach its admitted native wait without advancing wall-clock timers. */
async function admitted(): Promise<void> { await Promise.resolve(); await Promise.resolve(); await Promise.resolve(); }

/** Fake clocks are scoped to command timeout tests and always restored. */
afterEach(() => vi.useRealTimers());

it('rejects changed scope after lease preparation and releases before any native submission', async () => {
  const f = fixture();
  const gate = deferred<void>(); f.setAcquire(gate.promise);
  const promise = f.library.spawn(ID, request(f.library));
  f.setScope({...SCOPE, expectedWorldEpoch: '000000000000000a'});
  gate.resolve();
  await expect(promise).rejects.toThrow(/scope changed/);
  expect(f.calls).toEqual([]);
  expect(f.events).toEqual(['acquire', 'release']);
  await f.library.close();
});

it('retains timed-out command files until the real native outcome and then resolves exact retries', async () => {
  vi.useFakeTimers();
  const f = fixture();
  const body = request(f.library);
  const promise = f.library.spawn(ID, body);
  const rejected = expect(promise).rejects.toThrow(/outcome remains pending/);
  await admitted();
  expect(f.calls).toEqual(['0000000000000001']);
  expect(f.library.spawn(ID, body)).toBe(promise);
  await vi.advanceTimersByTimeAsync(30_000); await rejected;
  expect(f.library.busy).toBe(true);
  expect(f.events).toEqual(['acquire']);
  f.library.event({kind: 'savedBrainSpawned', commandSequence: f.calls[0]!,
    savedBrainSpawn: {snakeId: 41, effectiveStep: '0000000000000002'}});
  await admitted();
  expect(await f.library.spawn(ID, body)).toMatchObject({brainId: ID, snakeId: 41, commandSequence: f.calls[0]});
  expect(f.calls).toHaveLength(1);
  expect(f.events).toEqual(['acquire', 'release']);
  expect(() => f.library.spawn(ID, {...body, expectedRunId: 'other'})).toThrow(/immutable retry/);
  await f.library.close();
});

it('uses the deployment cursor to reject replay after old receipts leave the bounded cache', async () => {
  const f = fixture();
  const original = request(f.library);
  for (let index = 1; index <= 65; index++) {
    const body = index === 1 ? original : request(f.library, index.toString(16).padStart(32, '0'));
    const promise = f.library.spawn(ID, body); await admitted();
    f.library.event({kind: 'savedBrainSpawned', commandSequence: f.calls.at(-1)!,
      savedBrainSpawn: {snakeId: index, effectiveStep: '0000000000000002'}});
    await promise; await admitted();
  }
  expect(() => f.library.spawn(ID, original)).toThrow(/sequence changed/);
  expect(f.calls).toHaveLength(65);
  expect(() => f.library.spawn(ID, {...request(f.library), expectedProcessEpoch: '22'.repeat(16)})).toThrow(/process changed/);
  await f.library.close();
});

it('joins the stopped authority before releasing files for an unknown admitted outcome', async () => {
  const f = fixture();
  const gate = deferred<void>(); f.setJoin(gate.promise);
  const promise = f.library.spawn(ID, request(f.library));
  const rejected = expect(promise).rejects.toThrow(/closing/);
  await admitted();
  const closed = f.library.close(); await admitted();
  expect(f.events).toEqual(['acquire', 'stop', 'join']);
  gate.resolve(); await closed; await rejected;
  expect(f.events).toEqual(['acquire', 'stop', 'join', 'release']);
});

it('waits for an in-flight export acquire and releases it once when shutdown cancels preparation', async () => {
  const f = fixture();
  const gate = deferred<void>(); f.setAcquire(gate.promise);
  const exporting = f.library.export(ID);
  const rejected = expect(exporting).rejects.toThrow(/closing/);
  const closed = f.library.close(); await admitted();
  expect(f.events).toEqual(['acquire']);
  gate.resolve(); await closed; await rejected;
  expect(f.events).toEqual(['acquire', 'release']);
});

it('retains a known native rejection for exact retries without a second command or lease', async () => {
  const f = fixture(); const body = request(f.library);
  const pending = f.library.spawn(ID, body);
  const rejected = expect(pending).rejects.toThrow(/structural sensors/);
  await admitted();
  f.library.event({kind: 'commandRejected', commandSequence: f.calls[0]!, rejectionDetail: 'structural sensors changed'});
  await rejected; await admitted();
  await expect(f.library.spawn(ID, body)).rejects.toThrow(/structural sensors/);
  expect(f.calls).toHaveLength(1); expect(f.events).toEqual(['acquire', 'release']);
  await f.library.close();
});

it('treats a malformed native publication as an uncertain worker failure rather than a known rejection', async () => {
  const f = fixture(); const pending = f.library.spawn(ID, request(f.library));
  const rejected = expect(pending).rejects.toThrow(/worker.*uncertain/);
  await admitted();
  f.library.event({kind: 'savedBrainSpawned', commandSequence: f.calls[0]!});
  await rejected; await admitted();
  expect(() => f.library.status()).toThrow(/uncertain/);
  expect(f.events).toEqual(['acquire', 'release']);
  await f.library.close();
});
