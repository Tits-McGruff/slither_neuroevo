import { randomBytes } from 'node:crypto';
import type { ShadowActivationRequest, ShadowPublication, ShadowReceipt, ShadowStopRequest } from '../../src/protocol/shadows.ts';
import type { RustBackgroundEvent } from '../../src/protocol/rustBackground.ts';
import type { ExperimentalRunningAuthorityNativeHandle } from './backgroundRuntime.ts';
import type { BackgroundCommandAdmission } from './commandAdmission.ts';
import type { DistillationSpoolClient } from './distillationSpoolClient.ts';
import { distillationHex, distillationInteger } from './distillationProtocol.ts';
import { parseShadowMetrics, parseShadowReceipt } from './shadowProtocol.ts';

/** Single bounded command/result owner; timeout retains identity until its native outcome is known. */
interface Pending {
  /** Immutable confirmed-result fields except the applied boundary/time. */
  expected: Omit<ShadowReceipt, 'effectiveStep' | 'acknowledgedAt'>;
  /** Settle only after native confirmation and durable chronology commitment. */
  resolve(receipt: ShadowReceipt): void;
  /** Diagnostic failures never fault controlling authority. */
  reject(error: Error): void;
  /** API waiting bound independent of retained generation/controller barriers. */
  timer: NodeJS.Timeout;
  /** Suppress duplicate native receipt processing while SQLite commits. */
  received: boolean;
}

/** Ordered shadow controls and cold fixed summaries; no neural execution or gameplay ACKs in Node. */
export class DistillationShadows {
  /** New diagnostic process identity never advances authoritative RNG streams. */
  readonly processEpoch = randomBytes(16).toString('hex');
  /** At most one cold admission/control operation across all four slots. */
  private busy = false;
  /** Prevent new work during server shutdown. */
  private closed = false;
  /** Invalidate preparation that straddles an authoritative run replacement. */
  private incarnation = 0;
  /** One admitted control whose outcome is not yet durably settled. */
  private pending: Pending | undefined;
  /** Four last confirmed activations, retained even if their chronology write failed. */
  private readonly activations: (ShadowReceipt | undefined)[] = Array(4).fill(undefined);
  /** One asynchronous event persistence operation, independent of the authority pump. */
  private persistence: Promise<void> | undefined;

  /** All model/file admission belongs to the diagnostic worker; controls share global sequence ownership. */
  constructor(
    private readonly native: () => ExperimentalRunningAuthorityNativeHandle,
    private readonly admission: BackgroundCommandAdmission,
    private readonly spool: DistillationSpoolClient,
    private readonly runId: () => string
  ) {}

  /** Read only a bounded published native summary, carrying its diagnostic process identity. */
  metrics(): ShadowPublication {
    if (this.closed) throw new Error('shadow coordinator closing');
    return { ...parseShadowMetrics(this.native().getShadowMetrics()), processEpoch: this.processEpoch };
  }

  /** Explicit candidate activation has an idempotency key and exact run/config/capture binding. */
  async activate(slot: number, value: unknown): Promise<ShadowReceipt> {
    const raw = this.body(value, ['requestId', 'expectedProcessEpoch', 'expectedActivationSequence', 'expectedRunId', 'expectedConfigRevision', 'captureEpoch', 'baselineSlot', 'artifactSha256']);
    const requestId = distillationHex(raw['requestId'], 32, 'shadow request', true);
    if (distillationHex(raw['expectedProcessEpoch'], 32, 'shadow process', true) !== this.processEpoch) throw new Error('shadow process changed; begin an explicit new segment');
    const previousActivationSequence = distillationHex(raw['expectedActivationSequence'], 16, 'shadow previous activation');
    const request: ShadowActivationRequest = { shadowSlot: distillationInteger(slot, 0, 3, 'shadow slot'),
      expectedRunId: this.expectedRun(raw['expectedRunId']),
      expectedConfigRevision: distillationHex(raw['expectedConfigRevision'], 16, 'shadow config revision', true),
      captureEpoch: distillationHex(raw['captureEpoch'], 32, 'shadow capture', true),
      baselineSlot: distillationInteger(raw['baselineSlot'], 0, 119, 'shadow teacher'),
      artifactSha256: distillationHex(raw['artifactSha256'], 64, 'shadow candidate') };
    return this.exclusive(async incarnation => {
      const retry = await this.spool.shadowReceipt(this.processEpoch, requestId);
      if (retry) return this.retry(retry, request, previousActivationSequence);
      const previous = this.activations[slot];
      if (previous?.requestId === requestId) {
        this.retry(previous, request, previousActivationSequence);
        return this.persist(previous);
      }
      if ((previous?.activationSequence ?? '0'.repeat(16)) !== previousActivationSequence) throw new Error('shadow previous activation mismatch; refresh metrics');
      await this.spool.prepareShadow(request.artifactSha256);
      this.requireIncarnation(incarnation);
      const expected = { schemaVersion: 1 as const, processEpoch: this.processEpoch, requestId,
        runId: request.expectedRunId, configRevision: request.expectedConfigRevision, previousActivationSequence,
        captureEpoch: request.captureEpoch, artifactSha256: request.artifactSha256,
        shadowSlot: slot, baselineSlot: request.baselineSlot, enabled: true };
      return this.command(sequence => ({ ...expected, commandSequence: sequence, activationSequence: sequence }),
        sequence => this.native().submitActivateShadow(sequence, request));
    });
  }

  /** Delayed stops bind the exact activation, and cannot disable a replacement student. */
  async stop(slot: number, value: unknown): Promise<ShadowReceipt> {
    const raw = this.body(value, ['requestId', 'captureEpoch', 'activationSequence']);
    distillationInteger(slot, 0, 3, 'shadow slot');
    const requestId = distillationHex(raw['requestId'], 32, 'shadow request', true);
    const request: ShadowStopRequest = { shadowSlot: slot,
      captureEpoch: distillationHex(raw['captureEpoch'], 32, 'shadow capture', true),
      activationSequence: distillationHex(raw['activationSequence'], 16, 'shadow activation', true) };
    return this.exclusive(async incarnation => {
      const durable = await this.spool.shadowReceipt(this.processEpoch, requestId);
      const activation = this.activations[slot];
      const retry = durable ?? (activation?.requestId === requestId ? activation : undefined);
      if (retry) {
        if (retry.enabled || retry.shadowSlot !== slot || retry.captureEpoch !== request.captureEpoch || retry.activationSequence !== request.activationSequence) throw new Error('shadow request immutable retry mismatch');
        return durable ?? this.persist(retry);
      }
      if (!activation || activation.captureEpoch !== request.captureEpoch || activation.activationSequence !== request.activationSequence || activation.runId !== this.runId()) throw new Error('shadow stop activation mismatch');
      this.requireIncarnation(incarnation);
      const { acknowledgedAt: _time, effectiveStep: _step, commandSequence: _command, ...expected } = activation;
      void _time; void _step; void _command;
      return this.command(sequence => ({ ...expected, requestId, previousActivationSequence: request.activationSequence, enabled: false, commandSequence: sequence }),
        sequence => this.native().submitStopShadow(sequence, request));
    });
  }

  /** A confirmed native event never blocks the gameplay output pump on diagnostic storage. */
  event(event: RustBackgroundEvent): void {
    const pending = this.pending;
    if (!pending || pending.received || event.commandSequence !== pending.expected.commandSequence) return;
    if (event.kind === 'commandRejected') {
      this.pending = undefined; clearTimeout(pending.timer);
      pending.reject(new Error(event.rejectionDetail ?? 'native shadow command rejected')); return;
    }
    if (event.kind !== 'shadowChanged') return;
    pending.received = true; clearTimeout(pending.timer);
    const change = event.shadowChange;
    if (!change || change.shadowSlot !== pending.expected.shadowSlot || change.artifactSha256 !== pending.expected.artifactSha256 || change.enabled !== pending.expected.enabled) {
      this.pending = undefined; pending.reject(new Error('native shadow receipt mismatch')); return;
    }
    let receipt: ShadowReceipt;
    try { receipt = parseShadowReceipt({ ...pending.expected, effectiveStep: change.effectiveStep, acknowledgedAt: Date.now() }); }
    catch (error) { this.pending = undefined; pending.reject(error as Error); return; }
    this.activations[receipt.shadowSlot] = receipt;
    this.persistence = this.persist(receipt).then(saved => pending.resolve(saved), error => pending.reject(error))
      .finally(() => { if (this.pending === pending) this.pending = undefined; this.persistence = undefined; });
  }

  /** Replaced authority invalidates preparation/context without revising already observed receipts. */
  runReplaced(): void {
    this.incarnation += 1; this.activations.fill(undefined);
    if (this.pending) { clearTimeout(this.pending.timer); this.pending.reject(new Error('shadow run replaced')); this.pending = undefined; }
  }

  /** Settle diagnostic ownership before its worker closes; the native owner is stopped separately. */
  async close(): Promise<void> {
    this.closed = true; this.runReplaced(); await this.persistence;
  }

  /** Cold preparation and receipt waits have one bounded owner, including ambiguous timed-out commands. */
  private async exclusive<T>(operation: (incarnation: number) => Promise<T>): Promise<T> {
    if (this.closed) throw new Error('shadow coordinator closing');
    if (this.busy || this.pending) throw new Error('shadow control outcome remains pending; retry original request');
    this.busy = true;
    try { return await operation(this.incarnation); } finally { this.busy = false; }
  }
  /** Preparation cannot cross a reset/import/New Run boundary. */
  private requireIncarnation(value: number): void {
    if (this.closed || this.incarnation !== value) throw new Error('shadow run replaced or closing');
  }
  /** Validate the explicit expected run without reading or copying game state. */
  private expectedRun(value: unknown): string {
    if (typeof value !== 'string' || !value || Buffer.byteLength(value) > 128 || value.includes('\0') || value !== this.runId()) throw new TypeError('shadow run mismatch');
    return value;
  }
  /** Exact small request envelopes cannot smuggle neural or gameplay fields. */
  private body(value: unknown, keys: string[]): Record<string, unknown> {
    if (!value || typeof value !== 'object' || Array.isArray(value) || Object.keys(value).length !== keys.length || keys.some(key => !Object.hasOwn(value, key))) throw new TypeError('invalid shadow request fields');
    return value as Record<string, unknown>;
  }
  /** Lost replies are idempotent; altered requests cannot repurpose their identifier. */
  private retry(receipt: ShadowReceipt, request: ShadowActivationRequest, previousActivationSequence: string): ShadowReceipt {
    if (!receipt.enabled || receipt.previousActivationSequence !== previousActivationSequence || receipt.runId !== request.expectedRunId || receipt.configRevision !== request.expectedConfigRevision || receipt.captureEpoch !== request.captureEpoch || receipt.artifactSha256 !== request.artifactSha256 || receipt.shadowSlot !== request.shadowSlot || receipt.baselineSlot !== request.baselineSlot) throw new Error('shadow request immutable retry mismatch');
    return receipt;
  }
  /** Every failed write of an applied receipt retains the client's immutable recovery request. */
  private async persist(receipt: ShadowReceipt): Promise<ShadowReceipt> {
    try { return await this.spool.recordShadow(receipt); }
    catch (error) { throw new Error('native shadow applied; receipt persistence failed: ' + (error instanceof Error ? error.message : String(error))); }
  }
  /** Native admission consumes only the shared successful sequence; queue pressure does not pin a control. */
  private command(expected: (sequence: string) => Pending['expected'], submit: (sequence: string) => void): Promise<ShadowReceipt> {
    return new Promise((resolve, reject) => {
      const sequence = this.admission.nextSequence();
      const timer = setTimeout(() => reject(new Error('native shadow control timed out; outcome remains pending')), 30_000);
      timer.unref();
      this.pending = { expected: expected(sequence), resolve, reject, timer, received: false };
      try { if (!this.admission.trySubmitControl(submit)) throw new Error('native shadow admission exhausted'); }
      catch (error) { this.pending = undefined; clearTimeout(timer); reject(error instanceof Error ? error : new Error(String(error))); }
    });
  }
}
