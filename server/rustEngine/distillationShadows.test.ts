import { afterEach, expect, it, vi } from 'vitest';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import Database from 'better-sqlite3';
import { DistillationShadows } from './distillationShadows.ts';
import { BackgroundCommandAdmission } from './commandAdmission.ts';
import { BrainCandidateStore } from './brainCandidateStore.ts';
import { parseShadowMetrics, parseShadowReceipt } from './shadowProtocol.ts';
import type { ExperimentalRunningAuthorityNativeHandle } from './backgroundRuntime.ts';
import type { DistillationSpoolClient } from './distillationSpoolClient.ts';
import type { ShadowReceipt, ShadowActivationRequest } from '../../src/protocol/shadows.ts';

/** Exact test words preserve counters above JavaScript's safe integer range. */
const word = (value: number | bigint): string => BigInt(value).toString(16).padStart(16, '0');
/** Synthetic immutable receipt metadata; no controlling/native proof is claimed here. */
function receipt(sequence = 1): ShadowReceipt {
  return { schemaVersion: 1, processEpoch: '13'.repeat(16), requestId: word(sequence).padStart(32, '0'),
    commandSequence: word(sequence), activationSequence: word(sequence), previousActivationSequence: word(0),
    runId: 'run', configRevision: word(1), captureEpoch: '17'.repeat(16), artifactSha256: '23'.repeat(32),
    shadowSlot: 0, baselineSlot: 7, effectiveStep: word(12), enabled: true, acknowledgedAt: 1000 };
}
/** Empty native metrics are honest before explicit admission. */
const emptyMetrics = JSON.stringify({ schemaVersion: 1, slots: [null, null, null, null] });
/** Controlled roots only, closed before recursive removal. */
const roots: string[] = [];
/** Test-owned SQLite connections; never owner databases. */
const stores: BrainCandidateStore[] = [];
afterEach(() => {
  vi.useRealTimers(); stores.splice(0).forEach(store => store.close());
  for (const root of roots.splice(0)) {
    if (dirname(resolve(root)) !== resolve(tmpdir()) || !root.includes('slither-shadow-control-')) throw new Error('unsafe shadow cleanup');
    rmSync(root, { recursive: true, force: true });
  }
});

/** Fake only transport timing; storage/native end-to-end evidence is supplied by the native suite. */
function coordinator(autoReceipt = true) {
  const saved = new Map<string, ShadowReceipt>();
  const calls: { sequence: string; request: ShadowActivationRequest }[] = [];
  let service!: DistillationShadows;
  const spool = { prepareShadow: vi.fn(async () => ({})),
    shadowReceipt: vi.fn(async (_epoch: string, id: string) => saved.get(id) ?? null),
    recordShadow: vi.fn(async (value: ShadowReceipt) => { saved.set(value.requestId, value); return value; }) };
  const native = { getShadowMetrics: () => emptyMetrics,
    submitActivateShadow(sequence: string, request: ShadowActivationRequest) {
      calls.push({ sequence, request });
      if (autoReceipt) service.event({ kind: 'shadowChanged', commandSequence: sequence,
        shadowChange: { shadowSlot: request.shadowSlot, artifactSha256: request.artifactSha256, enabled: true, effectiveStep: word(12) } });
    }, submitStopShadow(sequence: string, request: { shadowSlot: number }) {
      service.event({ kind: 'shadowChanged', commandSequence: sequence, shadowChange: {
        shadowSlot: request.shadowSlot, artifactSha256: '23'.repeat(32), enabled: false, effectiveStep: word(13) } });
    } } as unknown as ExperimentalRunningAuthorityNativeHandle;
  const admission = new BackgroundCommandAdmission(native);
  service = new DistillationShadows(() => native, admission, spool as unknown as DistillationSpoolClient, () => 'run');
  const body = { requestId: '19'.repeat(16), expectedProcessEpoch: service.processEpoch, expectedActivationSequence: word(0), expectedRunId: 'run',
    expectedConfigRevision: word(1), captureEpoch: '17'.repeat(16), baselineSlot: 7, artifactSha256: '23'.repeat(32) };
  return { service, admission, calls, spool, body, native };
}

it('settles explicit controls only after matching native receipt and durable storage, with immutable lost-reply retries', async () => {
  const { service, admission, calls, spool, body } = coordinator();
  try {
    expect(service.metrics()).toMatchObject({ schemaVersion: 1, slots: [null, null, null, null] });
    const admitted = await service.activate(0, body);
    expect(admitted.activationSequence).toBe(word(1)); expect(calls).toHaveLength(1);
    expect(await service.activate(0, body)).toEqual(admitted); expect(calls).toHaveLength(1);
    await expect(service.activate(0, { ...body, baselineSlot: 8 })).rejects.toThrow(/immutable retry/u);
    await expect(service.activate(0, { ...body, requestId: '20'.repeat(16) })).rejects.toThrow(/previous activation/u);
    const stopped = await service.stop(0, { requestId: '21'.repeat(16), captureEpoch: body.captureEpoch, activationSequence: admitted.activationSequence });
    expect(stopped).toMatchObject({ enabled: false, commandSequence: word(2), activationSequence: word(1) });
    expect(admission.nextSequence()).toBe(word(3)); expect(spool.recordShadow).toHaveBeenCalledTimes(2);
  } finally { await service.close(); }
});

it('a timed-out control retains its admitted identity and records its late outcome without admitting replacements', async () => {
  vi.useFakeTimers();
  const { service, calls, body, spool } = coordinator(false);
  const pending = service.activate(0, body); const rejected = expect(pending).rejects.toThrow(/outcome remains pending/u);
  await vi.advanceTimersByTimeAsync(30_001); await rejected;
  await expect(service.activate(1, { ...body, requestId: '20'.repeat(16) })).rejects.toThrow(/already active/u);
  service.event({ kind: 'shadowChanged', commandSequence: calls[0]!.sequence, shadowChange: {
    shadowSlot: 0, artifactSha256: body.artifactSha256, enabled: true, effectiveStep: word(20) } });
  await vi.advanceTimersByTimeAsync(0);
  expect(spool.recordShadow).toHaveBeenCalledTimes(1);
  expect(await service.activate(0, body)).toMatchObject({ effectiveStep: word(20) });
  expect(calls).toHaveLength(1); await service.close();
});

it('cold preparation cannot cross run replacement and queue pressure never consumes a command sequence', async () => {
  const first = coordinator();
  let finish!: () => void;
  first.spool.prepareShadow.mockImplementationOnce(() => new Promise(resolve => { finish = () => resolve({}); }));
  const pending = first.service.activate(0, first.body);
  await vi.waitFor(() => expect(first.spool.prepareShadow).toHaveBeenCalled());
  first.service.runReplaced(); finish(); await expect(pending).rejects.toThrow(/run replaced/u);
  expect(first.calls).toHaveLength(0); await first.service.close();
  const second = coordinator();
  second.native.submitActivateShadow = () => { throw new Error('QueueByteLimit: full'); };
  await expect(second.service.activate(0, second.body)).rejects.toThrow(/admission exhausted/u);
  expect(second.admission.nextSequence()).toBe(word(1)); await second.service.close();
});

it('a receipt persistence failure preserves confirmed activation for exact diagnostic retry', async () => {
  const { service, body, spool, calls } = coordinator();
  spool.recordShadow.mockRejectedValueOnce(new Error('disk reserve exhausted'));
  await expect(service.activate(0, body)).rejects.toThrow(/applied; receipt persistence failed/u);
  expect(await service.activate(0, body)).toMatchObject({ activationSequence: word(1) });
  expect(calls).toHaveLength(1); await service.close();
});

it('native metrics reject expanded slots, nonfinite outputs, bad strata and lossy long counters', () => {
  expect(parseShadowMetrics(emptyMetrics).slots).toHaveLength(4);
  expect(() => parseShadowMetrics('{"schemaVersion":1,"slots":[]}')).toThrow(/slot count/u);
  expect(() => parseShadowMetrics(' '.repeat(65537))).toThrow(/byte admission/u);
  const totals = { samples: word(9007199254740993n), turnAbsoluteError: 0, turnSquaredError: 0,
    boostDisagreements: word(0), turnMae: 0, turnMse: 0, boostDisagreementRate: 0 };
  const slot = { shadowSlot: 0, artifactSha256: '23'.repeat(32), captureEpoch: '17'.repeat(16), baselineSlot: 7,
    activationSequence: word(1), activationStep: word(0), completedStep: word(9007199254740994n), lastSampleStep: word(9007199254740993n),
    generation: word(1), physicalSnakeId: word(2), enabled: true, error: null, rawOutputs: [0, 0], totals,
    strategies: Array(4).fill(totals), lifeStages: Array(3).fill(totals), strata: Array(6).fill(totals), cruising: totals, preDeath: totals, deaths: word(0) };
  const value = { schemaVersion: 1, slots: [slot, null, null, null] };
  expect(parseShadowMetrics(JSON.stringify(value)).slots[0]!.lastSampleStep).toBe(word(9007199254740993n));
  slot.rawOutputs = [NaN, 0]; expect(() => parseShadowMetrics(JSON.stringify(value))).toThrow(/outputs/u);
  slot.rawOutputs = [0, 0]; slot.strata = [totals]; expect(() => parseShadowMetrics(JSON.stringify(value))).toThrow(/strata/u);
  slot.strata = Array(6).fill(totals); (slot.totals.samples as unknown) = Number(9007199254740993n);
  expect(() => parseShadowMetrics(JSON.stringify(value))).toThrow(/samples/u);
});

it('durable receipt chronology migrates v1 metadata, rotates at 1024 and preserves exact large SQLite cursors', () => {
  const root = mkdtempSync(join(tmpdir(), 'slither-shadow-control-')); roots.push(root);
  const dbPath = join(root, 'metadata.sqlite');
  const legacy = new Database(dbPath);
  legacy.exec('CREATE TABLE candidates (ordinal INTEGER PRIMARY KEY AUTOINCREMENT,id TEXT UNIQUE NOT NULL,descriptor TEXT NOT NULL,bytes INTEGER NOT NULL)');
  legacy.pragma('application_id=1397506881'); legacy.pragma('user_version=1'); legacy.close();
  const validate = (): never => { throw new Error('no artifact admission in metadata-only fixture'); };
  const store = new BrainCandidateStore(root, validate, 0); stores.push(store);
  const db = new Database(dbPath);
  const insert = db.prepare('INSERT INTO shadow_events(process_epoch,command_sequence,request_id,payload) VALUES(?,?,?,?)');
  db.transaction(() => { for (let i = 1; i <= 1024; i++) { const item = receipt(i); insert.run(item.processEpoch, item.commandSequence, item.requestId, JSON.stringify(item)); } })();
  db.prepare("UPDATE sqlite_sequence SET seq=? WHERE name='shadow_events'").run(9007199254740993n); db.close();
  const item = receipt(1025); expect(store.recordShadow(item)).toEqual(item);
  expect(store.recordShadow(item)).toEqual(item);
  expect(() => store.recordShadow({ ...item, effectiveStep: word(13) })).toThrow(/immutable retry/u);
  expect(store.shadowReceipt(item.processEpoch, item.requestId)).toEqual(item);
  expect(store.shadowEvents(word(0), 1).events[0]!.ordinal).toBe(word(2));
  const page = store.shadowEvents(word(9007199254740993n), 1);
  expect(page.events[0]!.ordinal).toBe(word(9007199254740994n)); expect(page.hasMore).toBe(false);
  store.close(); stores.pop(); const reopened = new BrainCandidateStore(root, validate, 0); stores.push(reopened);
  expect(reopened.shadowReceipt(item.processEpoch, item.requestId)).toEqual(item);
  expect(() => parseShadowReceipt({ ...item, commandSequence: word(1) })).toThrow(/scope/u);
});
