import { afterEach, expect, it, vi } from 'vitest';
import { DistillationCapture } from './distillationCapture.ts';
import { BackgroundCommandAdmission } from './commandAdmission.ts';
import type { DistillationSpoolClient } from './distillationSpoolClient.ts';
import type { ExperimentalRunningAuthorityNativeHandle } from './backgroundRuntime.ts';
import type { TeacherCaptureRequest, TeacherCaptureDrain, DistillationDataset } from '../../src/protocol/distillation.ts';

afterEach(() => vi.useRealTimers());

/** Hold one preparation boundary without sleeping or accessing owner storage. */
function gate() {
  let release!: () => void;
  const promise = new Promise<void>(resolve => { release = resolve; });
  return { promise, release };
}

/** Control only worker/native receipt timing; no simulation or codec behavior is substituted. */
function coordinator(ready: Promise<void> = Promise.resolve(), acknowledge = true) {
  let capture!: DistillationCapture;
  let stopped = false;
  const calls: { sequence: string; request: TeacherCaptureRequest }[] = [];
  const native = {
    submitStartTeacherCapture(sequence: string, request: TeacherCaptureRequest) {
      calls.push({ sequence, request });
      stopped = false;
      if (acknowledge) capture.event({ kind: 'teacherCaptureChanged', commandSequence: sequence,
        teacherCaptureEpoch: request.captureEpoch, teacherCaptureEnabled: true });
    },
    submitStopTeacherCapture: vi.fn((sequence: string, epoch: string) => {
      stopped = true;
      capture.event({ kind: 'teacherCaptureChanged', commandSequence: sequence,
        teacherCaptureEpoch: epoch, teacherCaptureEnabled: false });
    }),
    drainTeacherChunks: vi.fn(() => ({ contract: calls.length ? { captureEpoch: calls.at(-1)!.request.captureEpoch, runId: 'run' } : null,
      stopped, moreWork: false, releasedThrough: '0000000000000001', productionWatermark: '0000000000000001', chunks: [] } as unknown as TeacherCaptureDrain))
  } as unknown as ExperimentalRunningAuthorityNativeHandle;
  const spool = { ready, storagePolicy: vi.fn(async () => ({ maxDatasets: 256 })),
    datasets: vi.fn(async () => ({ datasets: [] as unknown[], hasMore: false, nextCursor: '0'.repeat(32) })),
    dataset: vi.fn(async (epoch: string) => ({ contract: { captureEpoch: epoch }, state: stopped ? 'stopped' : 'active' } as DistillationDataset)),
    ingest: vi.fn(async (drain: TeacherCaptureDrain) => ({ contract: drain.contract, state: drain.stopped ? 'stopped' : 'active' } as DistillationDataset)),
    interrupt: vi.fn(async (): Promise<void> => undefined), close: vi.fn(async () => undefined) };
  const admission = new BackgroundCommandAdmission(native);
  capture = new DistillationCapture(() => native, admission, spool as unknown as DistillationSpoolClient,
    () => ({ runId: 'run', seed: 42 }));
  const body = { expectedRunId: 'run', expectedConfigRevision: '0000000000000001', baselineSlots: [7],
    pendingBytes: 1024, queueChunks: 4, queueBytes: 4096 };
  return { capture, native, spool, calls, admission, body };
}

it.each(['ready', 'health'] as const)('owns capture start while awaiting spool %s', async phase => {
  const held = gate();
  const fixture = coordinator(phase === 'ready' ? held.promise : Promise.resolve());
  if (phase === 'health') fixture.spool.datasets.mockImplementationOnce(async () => { await held.promise; return { datasets: [], hasMore: false, nextCursor: '0'.repeat(32) }; });
  const started = fixture.capture.start(fixture.body);
  try {
    if (phase === 'health') await vi.waitFor(() => expect(fixture.spool.datasets).toHaveBeenCalled());
    await expect(fixture.capture.start(fixture.body)).rejects.toThrow(/already active/u);
    held.release();
    expect((await started).contract.captureEpoch).toBe(fixture.calls[0]!.request.captureEpoch);
    expect(fixture.calls).toHaveLength(1);
    expect(fixture.admission.nextSequence()).toBe('0000000000000002');
  } finally { held.release(); await fixture.capture.close(); }
});

it('releases failed preparation and does not submit a prepared start after shutdown', async () => {
  const fixture = coordinator();
  fixture.spool.datasets.mockRejectedValueOnce(new Error('worker unavailable'));
  try {
    await expect(fixture.capture.start(fixture.body)).rejects.toThrow('worker unavailable');
    await fixture.capture.start(fixture.body);
    expect(fixture.calls).toHaveLength(1);
  } finally { await fixture.capture.close(); }
  const held = gate(); const closing = coordinator(held.promise);
  const started = closing.capture.start(closing.body);
  await closing.capture.close(); held.release();
  await expect(started).rejects.toThrow(/closing/u);
  expect(closing.calls).toHaveLength(0);
});

it.each([1, 256])('rejects a full %i-manifest store before native admission', async count => {
  const fixture = coordinator();
  fixture.spool.storagePolicy.mockResolvedValue({ maxDatasets: count });
  fixture.spool.datasets.mockResolvedValue({ datasets: Array(count).fill({}), hasMore: false, nextCursor: '1'.repeat(32) });
  try {
    await expect(fixture.capture.start(fixture.body)).rejects.toThrow(/manifest quota exhausted/u);
    expect(fixture.calls).toHaveLength(0);
    expect(fixture.admission.nextSequence()).toBe('0000000000000001');
  } finally { await fixture.capture.close(); }
});

it('does not admit an old-epoch stop while a new start owns preparation', async () => {
  const fixture = coordinator(); const held = gate();
  let next: Promise<DistillationDataset> | undefined;
  try {
    const first = await fixture.capture.start(fixture.body);
    await fixture.capture.stop(first.contract.captureEpoch);
    fixture.spool.datasets.mockImplementationOnce(async () => { await held.promise; return { datasets: [], hasMore: false, nextCursor: '0'.repeat(32) }; });
    next = fixture.capture.start(fixture.body);
    await vi.waitFor(() => expect(fixture.spool.datasets).toHaveBeenCalledTimes(2));
    await expect(fixture.capture.stop(first.contract.captureEpoch)).rejects.toThrow();
    expect(fixture.native.submitStopTeacherCapture).toHaveBeenCalledTimes(1);
    held.release(); await next;
    expect(fixture.calls).toHaveLength(2);
    expect(fixture.admission.nextSequence()).toBe('0000000000000004');
  } finally { held.release(); await next?.catch(() => undefined); await fixture.capture.close(); }
});

it.each(['mailbox tail', 'unreleased watermark'] as const)('retains capture ownership through the stopped %s until final storage completes', async reason => {
  vi.useFakeTimers();
  const fixture = coordinator(); const held = gate();
  let stopping: Promise<DistillationDataset> | undefined;
  let settled = false;
  try {
    const first = await fixture.capture.start(fixture.body);
    const base = fixture.native.drainTeacherChunks(64, 1024 * 1024);
    vi.spyOn(fixture.native, 'drainTeacherChunks')
      .mockReturnValueOnce({ ...base, stopped: true, moreWork: reason === 'mailbox tail',
        productionWatermark: '0000000000000002', releasedThrough: '0000000000000001' })
      .mockReturnValueOnce({ ...base, stopped: true, moreWork: false,
        productionWatermark: '0000000000000002', releasedThrough: '0000000000000002' });
    fixture.spool.ingest.mockImplementationOnce(async drain => ({ contract: drain.contract, state: 'stopped' } as DistillationDataset))
      .mockImplementationOnce(async drain => { await held.promise; return { contract: drain.contract, state: 'stopped' } as DistillationDataset; });
    stopping = fixture.capture.stop(first.contract.captureEpoch).then(dataset => { settled = true; return dataset; });
    await vi.waitFor(() => expect(fixture.spool.ingest).toHaveBeenCalledTimes(3));
    expect(settled).toBe(false);
    await expect(fixture.capture.start(fixture.body)).rejects.toThrow(/draining/u);
    expect(fixture.calls).toHaveLength(1);
    expect(fixture.spool.ingest.mock.calls.slice(1).map(([drain]) => drain.contract!.captureEpoch))
      .toEqual([first.contract.captureEpoch, first.contract.captureEpoch]);
    held.release(); expect((await stopping).state).toBe('stopped');
    await fixture.capture.start(fixture.body);
    expect(fixture.calls).toHaveLength(2);
  } finally { held.release(); await stopping?.catch(() => undefined); await fixture.capture.close(); }
});

it.each(['active', 'stopped tail'] as const)('joins an in-flight %s poll before finalizing replacement and admitting another capture', async state => {
  const fixture = coordinator(); const held = gate(); const interrupted = gate();
  let poll: Promise<void> | undefined; let replacement: Promise<void> | undefined;
  try {
    const first = await fixture.capture.start(fixture.body);
    const drain = fixture.native.drainTeacherChunks(64, 1024 * 1024);
    vi.spyOn(fixture.native, 'drainTeacherChunks').mockReturnValueOnce({ ...drain, stopped: state !== 'active', moreWork: true });
    fixture.spool.ingest.mockImplementationOnce(async value => {
      await held.promise; return { contract: value.contract, state: state === 'active' ? 'active' : 'stopped' } as DistillationDataset;
    });
    fixture.spool.interrupt.mockImplementationOnce(() => interrupted.promise);
    poll = fixture.capture.poll();
    await vi.waitFor(() => expect(fixture.spool.ingest).toHaveBeenCalledTimes(2));
    replacement = fixture.capture.runReplaced();
    expect(fixture.capture.runReplaced()).toBe(replacement);
    await expect(fixture.capture.start(fixture.body)).rejects.toThrow(/already active/u);
    held.release(); await poll;
    await vi.waitFor(() => expect(fixture.spool.interrupt).toHaveBeenCalledWith(first.contract.captureEpoch));
    await expect(fixture.capture.start(fixture.body)).rejects.toThrow(/already active/u);
    interrupted.release(); await replacement;
    const next = await fixture.capture.start(fixture.body);
    expect(next.contract.captureEpoch).not.toBe(first.contract.captureEpoch);
    expect(fixture.calls).toHaveLength(2);
  } finally { held.release(); interrupted.release(); await poll; await replacement; await fixture.capture.close(); }
});

it.each(['success', 'retry interruption', 'missing manifest'] as const)('settles a retired capture after failed ingestion: %s', async outcome => {
  const fixture = coordinator(); const held = gate();
  try {
    const first = await fixture.capture.start(fixture.body);
    fixture.spool.ingest.mockImplementationOnce(async () => { await held.promise; throw new Error('file publication failed'); });
    if (outcome !== 'success') fixture.spool.interrupt.mockRejectedValueOnce(new Error(outcome === 'missing manifest' ? 'dataset not found' : 'metadata publication failed'));
    const poll = fixture.capture.poll(); const rejectedPoll = expect(poll).rejects.toThrow(/file publication/u);
    await vi.waitFor(() => expect(fixture.spool.ingest).toHaveBeenCalledTimes(2));
    const replacement = fixture.capture.runReplaced();
    const settled = outcome === 'retry interruption' ? expect(replacement).rejects.toThrow(/metadata publication/u) : replacement;
    held.release(); await rejectedPoll; await settled;
    expect(fixture.spool.interrupt).toHaveBeenCalledWith(first.contract.captureEpoch);
    if (outcome === 'retry interruption') {
      await expect(fixture.capture.stop(first.contract.captureEpoch)).rejects.toThrow();
      expect(fixture.native.submitStopTeacherCapture).not.toHaveBeenCalled();
    }
    const next = await fixture.capture.start(fixture.body);
    expect(next.contract.captureEpoch).not.toBe(first.contract.captureEpoch);
    expect(fixture.spool.interrupt).toHaveBeenCalledTimes(outcome === 'retry interruption' ? 2 : 1);
    expect(fixture.calls).toHaveLength(2);
  } finally { held.release(); await fixture.capture.close(); }
});

it('invalidates held capture preparation even when a replacement retains the same run ID', async () => {
  const held = gate(); const fixture = coordinator(held.promise);
  const started = fixture.capture.start(fixture.body);
  try {
    await fixture.capture.runReplaced(); held.release();
    await expect(started).rejects.toThrow(/run replaced/u);
    expect(fixture.calls).toHaveLength(0);
    await fixture.capture.start(fixture.body);
    expect(fixture.calls).toHaveLength(1);
  } finally { held.release(); await fixture.capture.close(); }
});

it('retires a pending native control during replacement and ignores its late receipt', async () => {
  const fixture = coordinator(Promise.resolve(), false);
  const started = fixture.capture.start(fixture.body);
  const rejected = expect(started).rejects.toThrow(/run replaced/u);
  try {
    await vi.waitFor(() => expect(fixture.calls).toHaveLength(1));
    await fixture.capture.runReplaced(); await rejected;
    const old = fixture.calls[0]!;
    fixture.capture.event({ kind: 'teacherCaptureChanged', commandSequence: old.sequence,
      teacherCaptureEpoch: old.request.captureEpoch, teacherCaptureEnabled: true });
    const next = fixture.capture.start(fixture.body);
    await vi.waitFor(() => expect(fixture.calls).toHaveLength(2));
    const current = fixture.calls[1]!;
    fixture.capture.event({ kind: 'teacherCaptureChanged', commandSequence: current.sequence,
      teacherCaptureEpoch: current.request.captureEpoch, teacherCaptureEnabled: true });
    expect((await next).contract.captureEpoch).toBe(current.request.captureEpoch);
  } finally { await fixture.capture.close(); }
});

it('polls a confirmed late capture even after its original API wait timed out', async () => {
  vi.useFakeTimers(); const fixture = coordinator(Promise.resolve(), false);
  try {
    const started = fixture.capture.start(fixture.body);
    const timedOut = expect(started).rejects.toThrow(/timed out/u);
    await vi.advanceTimersByTimeAsync(30_001); await timedOut;
    const command = fixture.calls[0]!;
    fixture.capture.event({ kind: 'teacherCaptureChanged', commandSequence: command.sequence,
      teacherCaptureEpoch: command.request.captureEpoch, teacherCaptureEnabled: true });
    await vi.advanceTimersByTimeAsync(21);
    expect(fixture.native.drainTeacherChunks).toHaveBeenCalled();
    await expect(fixture.capture.start(fixture.body)).rejects.toThrow(/already active/u);
  } finally { await fixture.capture.close(); }
});
