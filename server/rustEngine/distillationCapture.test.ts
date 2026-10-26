import { afterEach, expect, it, vi } from 'vitest';
import { DistillationCapture } from './distillationCapture.ts';
import { BackgroundCommandAdmission } from './commandAdmission.ts';
import type { DistillationSpoolClient } from './distillationSpoolClient.ts';
import type { ExperimentalRunningAuthorityNativeHandle } from './backgroundRuntime.ts';
import type { TeacherCaptureRequest, DistillationDataset } from '../../src/protocol/distillation.ts';

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
  const calls: { sequence: string; request: TeacherCaptureRequest }[] = [];
  const native = {
    submitStartTeacherCapture(sequence: string, request: TeacherCaptureRequest) {
      calls.push({ sequence, request });
      if (acknowledge) capture.event({ kind: 'teacherCaptureChanged', commandSequence: sequence,
        teacherCaptureEpoch: request.captureEpoch, teacherCaptureEnabled: true });
    },
    drainTeacherChunks: vi.fn(() => ({ contract: null }))
  } as unknown as ExperimentalRunningAuthorityNativeHandle;
  const spool = { ready, datasets: vi.fn(async (): Promise<void> => undefined),
    dataset: vi.fn(async (epoch: string) => ({ contract: { captureEpoch: epoch }, state: 'active' } as DistillationDataset)),
    interrupt: vi.fn(async () => undefined), close: vi.fn(async () => undefined) };
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
  if (phase === 'health') fixture.spool.datasets.mockImplementationOnce(() => held.promise);
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
