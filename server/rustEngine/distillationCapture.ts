import { randomBytes } from 'node:crypto';
import type { TeacherCaptureRequest } from '../../src/protocol/distillation.ts';
import type { RustBackgroundEvent } from '../../src/protocol/rustBackground.ts';
import type { ExperimentalRunningAuthorityNativeHandle } from './backgroundRuntime.ts';
import type { BackgroundCommandAdmission } from './commandAdmission.ts';
import { distillationHex, distillationInteger, type DistillationDataset } from './distillationProtocol.ts';
import type { DistillationSpoolClient } from './distillationSpoolClient.ts';

/** One explicit ordered arm/stop waiting for its exact native diagnostic receipt. */
interface PendingControl {
  /** Sequence assigned by the shared authority command admission. */
  sequence: string;
  /** Expected immutable capture identity. */
  epoch: string;
  /** Expected arm/stop state. */
  enabled: boolean;
  /** Settle only the matching native receipt. */
  resolve(): void;
  /** Reject admission/receipt failure independently of game authority. */
  reject(error: Error): void;
  /** A retained gameplay barrier can delay control; the API wait is bounded. */
  timer: NodeJS.Timeout;
}

/** Periodic opaque drain/spool coordinator with no actor or simulation interface. */
export class DistillationCapture {
  /** At most one bounded in-flight native handoff and worker ingestion. */
  private activePoll: Promise<void> | undefined;
  /** Periodic asynchronous diagnostic work. */
  private timer: NodeJS.Timeout | undefined;
  /** Exact current diagnostic epoch after a successful native arm. */
  private epoch: string | undefined;
  /** Last persisted capture state, for explicit arm/stop gating. */
  private stopped = true;
  /** Idle/fully settled epochs need no repeated native wake or SQLite transaction. */
  private complete = true;
  /** One ordered capture control awaiting its native result. */
  private pending: PendingControl | undefined;
  /** Bounded diagnostic failure; it never faults the native game. */
  private failure: Error | undefined;
  /** Prevent future polling after server shutdown. */
  private closed = false;
  /** Concurrent shutdown callers share one complete final-drain/worker-close witness. */
  private closePromise: Promise<void> | undefined;

  /** Keep capture work on its own timer and route controls through shared admission. */
  constructor(
    private readonly native: () => ExperimentalRunningAuthorityNativeHandle,
    private readonly admission: BackgroundCommandAdmission,
    private readonly spool: DistillationSpoolClient,
    private readonly runIdentity: () => { runId: string; seed: number }
  ) {}

  /** Admit one explicit capture with a new storage epoch; production defaults remain unchanged. */
  async start(value: unknown): Promise<DistillationDataset> {
    if (this.closed || this.pending || !this.stopped) throw new Error('capture already active or closing');
    if (!value || typeof value !== 'object' || Array.isArray(value)) throw new TypeError('invalid capture request');
    const raw = value as Record<string, unknown>;
    const keys = ['expectedRunId', 'expectedConfigRevision', 'baselineSlots', 'pendingBytes', 'queueChunks', 'queueBytes'];
    if (Object.keys(raw).length !== keys.length || keys.some(key => !Object.hasOwn(raw, key))) throw new TypeError('invalid capture request fields');
    if (typeof raw['expectedRunId'] !== 'string' || !raw['expectedRunId'] || Buffer.byteLength(raw['expectedRunId']) > 128 || raw['expectedRunId'].includes('\0')) {
      throw new TypeError('invalid capture run identity');
    }
    const revision = distillationHex(raw['expectedConfigRevision'], 16, 'config revision', true);
    const slots = raw['baselineSlots'];
    if (!Array.isArray(slots) || slots.length < 1 || slots.length > 120) throw new TypeError('invalid capture slots');
    slots.forEach((slot, index) => {
      distillationInteger(slot, 0, 119, 'baseline slot');
      if (index > 0 && (slots[index - 1] as number) >= slot) throw new TypeError('capture slots must increase strictly');
    });
    const request: TeacherCaptureRequest = { captureEpoch: randomBytes(16).toString('hex'),
      expectedRunId: raw['expectedRunId'], expectedConfigRevision: revision, baselineSlots: [...slots] as number[],
      pendingBytes: distillationInteger(raw['pendingBytes'], 1, 16 * 1024 ** 2, 'pending bytes'),
      queueChunks: distillationInteger(raw['queueChunks'], 1, 65_536, 'queue chunks'),
      queueBytes: distillationInteger(raw['queueBytes'], 1, 16 * 1024 ** 2, 'queue bytes') };
    await this.spool.ready;
    await this.spool.datasets('0'.repeat(32), 1); // Validate current worker health before arming native capture.
    this.failure = undefined;
    await this.command(request.captureEpoch, true, sequence => this.native().submitStartTeacherCapture(sequence, request));
    this.epoch = request.captureEpoch;
    this.stopped = false;
    this.complete = false;
    this.startPolling();
    const deadline = performance.now() + 10_000;
    for (;;) {
      await this.poll();
      try { return await this.spool.dataset(request.captureEpoch); }
      catch (error) {
        if (!(error instanceof Error) || !/not found/u.test(error.message) || performance.now() >= deadline) throw error;
      }
      await new Promise(resolve => setTimeout(resolve, 10));
    }
  }

  /** Stop an exact epoch; trainer consumption never gates this ordered command. */
  async stop(epoch: string): Promise<DistillationDataset> {
    distillationHex(epoch, 32, 'capture epoch', true);
    if (epoch !== this.epoch || this.pending || this.closed) throw new Error('capture epoch mismatch or closing');
    await this.command(epoch, false, sequence => this.native().submitStopTeacherCapture(sequence, epoch));
    await this.poll();
    const deadline = performance.now() + 10_000;
    while (!this.stopped && performance.now() < deadline) {
      await this.poll();
      if (!this.stopped) await new Promise(resolve => setTimeout(resolve, 10));
    }
    if (!this.stopped) throw new Error('capture stop diagnostic timed out');
    return this.spool.dataset(epoch);
  }

  /** Observe only matching diagnostic receipts; never manufacture a controller acceptance. */
  event(event: RustBackgroundEvent): void {
    if (!this.pending || event.commandSequence !== this.pending.sequence) return;
    if (event.kind === 'commandRejected') {
      const pending = this.pending; this.pending = undefined; clearTimeout(pending.timer);
      pending.reject(new Error(event.rejectionDetail ?? 'native rejected capture')); return;
    }
    if (event.kind !== 'teacherCaptureChanged') return;
    const pending = this.pending; this.pending = undefined; clearTimeout(pending.timer);
    if (event.teacherCaptureEpoch !== pending.epoch || event.teacherCaptureEnabled !== pending.enabled) {
      pending.reject(new Error('native capture receipt mismatch')); return;
    }
    this.epoch = pending.epoch;
    this.stopped = !pending.enabled;
    this.complete = false;
    pending.resolve();
  }

  /** Coalesce drain requests while a worker write is in progress; native overflow remains independent. */
  poll(): Promise<void> {
    if (this.closed || this.failure) return this.failure ? Promise.reject(this.failure) : Promise.resolve();
    if (this.complete) return Promise.resolve();
    this.activePoll ??= this.pollOnce().finally(() => { this.activePoll = undefined; });
    return this.activePoll;
  }

  /** Mark replaced runs interrupted without claiming an unobserved diagnostic tail. */
  async runReplaced(): Promise<void> {
    if (this.epoch && !this.stopped) {
      const epoch = this.epoch;
      this.stopped = true;
      this.complete = true;
      clearInterval(this.timer); this.timer = undefined;
      await this.activePoll;
      await this.spool.interrupt(epoch);
    }
  }

  /** Drain final stopped-runtime ownership before closing only the diagnostic worker. */
  async close(): Promise<void> {
    this.closePromise ??= this.closeInternal();
    return this.closePromise;
  }

  /** Drain the full admitted native count/byte envelope without allowing new requests. */
  private async closeInternal(): Promise<void> {
    if (this.closed) return;
    this.closed = true;
    clearInterval(this.timer);
    try {
      await this.activePoll;
      if (!this.failure) {
        // At most 65,536 queued objects plus the 128-object mailbox; 2,048
        // bounded drains cover both alternating mailbox refills and byte limits.
        for (let count = 0; count < 2048; count += 1) {
          const drain = this.native().drainTeacherChunks(64, 1024 * 1024);
          if (!drain.contract) break;
          const identity = this.runIdentity();
          if (identity.runId !== drain.contract.runId) continue;
          await this.spool.ingest(drain, identity.seed);
          if (!drain.moreWork) break;
        }
      }
      if (this.epoch && !this.stopped) await this.spool.interrupt(this.epoch);
    } finally {
      if (this.pending) { clearTimeout(this.pending.timer); this.pending.reject(new Error('capture coordinator closing')); this.pending = undefined; }
      await this.spool.close();
    }
  }

  /** One at-most-one-MiB opaque handoff; no loop waits for the trainer/storage ACK. */
  private async pollOnce(): Promise<void> {
    await this.spool.ready;
    const drain = this.native().drainTeacherChunks(64, 1024 * 1024);
    if (!drain.contract) return;
    if (this.epoch && drain.contract.captureEpoch !== this.epoch) return;
    const identity = this.runIdentity();
    if (identity.runId !== drain.contract.runId) return;
    const dataset = await this.spool.ingest(drain, identity.seed);
    if (this.epoch && dataset.contract.captureEpoch !== this.epoch) return;
    this.epoch = dataset.contract.captureEpoch;
    this.stopped = dataset.state !== 'active';
    this.complete = this.stopped && !drain.moreWork && drain.releasedThrough === drain.productionWatermark;
    if (this.complete) { clearInterval(this.timer); this.timer = undefined; }
    if (drain.error) throw new Error(drain.error);
  }

  /** Reserve only ordered arm/stop metadata, sharing the existing global command sequence. */
  private command(epoch: string, enabled: boolean, submit: (sequence: string) => void): Promise<void> {
    return new Promise((resolve, reject) => {
      const sequence = this.admission.nextSequence();
      const timer = setTimeout(() => {
        reject(new Error('native capture control timed out'));
      }, 30_000);
      timer.unref();
      this.pending = { sequence, epoch, enabled, resolve, reject, timer };
      try {
        if (!this.admission.trySubmitControl(submit)) throw new Error('native capture admission exhausted');
      } catch (error) {
        this.pending = undefined; clearTimeout(timer); reject(error instanceof Error ? error : new Error(String(error)));
      }
    });
  }

  /** Poll only an admitted active epoch; discovery and stopped epochs stay idle. */
  private startPolling(): void {
    clearInterval(this.timer);
    this.timer = setInterval(() => { void this.poll().catch(error => this.fail(error)); }, 20);
    this.timer.unref();
  }

  /** Capture/storage failure is isolated; the game keeps its existing authority. */
  private fail(error: unknown): void {
    this.failure ??= error instanceof Error ? error : new Error(String(error));
    clearInterval(this.timer);
  }
}
