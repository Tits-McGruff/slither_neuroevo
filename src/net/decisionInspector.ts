import { parseInspectorSnapshot, type InspectEntityMsg, type InspectorResultMsg, type InspectorSnapshot } from '../protocol/inspector.ts';
import type { SelectedEntity } from '../protocol/entitySelection.ts';

/** Observed presentation scope; tokens remain opaque. */
export interface InspectorClientScope {
  /** Current server process. */ sessionId: string;
  /** Current authoritative run. */ runId: string;
  /** Explicit current server support. */ supported: boolean;
}
/** Browser metadata callbacks; no world or model execution enters the client. */
export interface InspectorClientOptions {
  /** Queue one exact current-scope command. */ send(message: InspectEntityMsg): void;
  /** Replace or clear the visible native sample. */ changed(snapshot: InspectorSnapshot | null): void;
  /** Display honest admission and waiting state. */ status(message: string): void;
}
/** One target and one command, with one bounded retry timer for queue contention. */
export class DecisionInspectorClient {
  /** Current welcome scope, absent while disconnected. */ private scope: InspectorClientScope | null = null;
  /** Explicit local opt-in; construction is always off. */ private enabled = false;
  /** Current exact selected native identity. */ private selected: SelectedEntity | null = null;
  /** Client-owned correlation counter. */ private ordinal = 0;
  /** One in-flight command; latest user intent remains in enabled/selected. */ private pending: {id: string; target: SelectedEntity | null; pureProbe: boolean; revision: number} | null = null;
  /** Last acknowledged subscription and its exact target. */ private active: {id: string; sequence: string; target: SelectedEntity; pureProbe: boolean; revision: number} | null = null;
  /** Do not repeatedly retry a rejected physical token. */ private rejectedToken: string | null = null;
  /** At most one contention retry. */ private retry: ReturnType<typeof setTimeout> | undefined;
  /** Invalid admission metadata still requires an explicit native close before retry. */ private closeRequired = false;
  /** Explicit view mode; probes never create continuous query work. */ private pureProbe = false;
  /** Only an explicitly requested physical life may be probed; automatic follow never requests another. */ private probeToken: string | null = null;
  /** Repeated one-shot requests remain distinguishable while an older admission is pending. */ private revision = 0;
  /** Last displayed mailbox sequence, scoped to the acknowledged subscription. */ private afterSequence = 0n;

  /** Create an idle presentation client. */
  constructor(private readonly options: InspectorClientOptions) {}

  /** Replace process/run context and discard all former-scope intent. */
  context(scope: InspectorClientScope | null): void {
    if (this.scope?.sessionId === scope?.sessionId && this.scope?.runId === scope?.runId
        && this.scope?.supported === scope?.supported) return;
    this.cancelRetry(); this.scope = scope; this.pending = null; this.active = null;
    this.selected = null; this.rejectedToken = null; this.closeRequired = false; this.afterSequence = 0n; this.pureProbe = false; this.probeToken = null; this.revision = 0;
    this.options.changed(null);
    this.options.status(scope?.supported ? 'Select a snake, then enable the decision inspector.' : 'Decision inspector unavailable.');
  }

  /** Opt in or close; changing intent during an in-flight command coalesces safely. */
  setEnabled(enabled: boolean): void {
    this.enabled = enabled; this.rejectedToken = null; this.cancelRetry();
    this.options.changed(null); this.pump();
  }

  /** Explicitly capture one sensor-only sample, coalescing repeated pending clicks. */
  probeOnce(): void { this.pureProbe = true; this.probeToken = this.selected?.token ?? null; this.revision++; this.setEnabled(true); }

  /** Resume capturing actual decisions after a one-shot probe. */
  showDelivered(): void { this.pureProbe = false; this.revision++; this.setEnabled(true); }

  /** Follow native selection, including pending/dead individual lives. */
  selection(selected: SelectedEntity | null): void {
    if (this.selected?.token === selected?.token) return;
    this.selected = selected; this.rejectedToken = null; this.cancelRetry();
    this.options.changed(null); this.pump();
  }

  /** Admit only the current correlation and exact queue sequence. */
  result(message: InspectorResultMsg): void {
    if (typeof message.applied !== 'boolean' || typeof message.enabled !== 'boolean') return;
    const pending = this.pending;
    if (!pending || pending.id !== message.requestId) {
      if (this.active?.id === message.requestId && !message.enabled) {
        this.active = null; this.options.changed(null); this.later();
      }
      return;
    }
    this.pending = null;
    if (message.applied) {
      if (typeof message.subscriptionSequence !== 'string' || !/^[0-9a-f]{16}$/u.test(message.subscriptionSequence)
        || message.subscriptionSequence === '0000000000000000' || message.enabled !== (pending.target !== null)) {
        this.closeRequired = true; this.rejectedToken = pending.target?.token ?? null;
        this.options.changed(null); this.options.status('Invalid inspector admission result; closing capture.'); this.later(); return;
      }
      this.active = pending.target ? {id: pending.id, sequence: message.subscriptionSequence, target: pending.target, pureProbe: pending.pureProbe, revision: pending.revision} : null;
      this.closeRequired = false; this.afterSequence = 0n;
      if (this.enabled && this.rejectedToken === this.selected?.token) {
        this.options.status('Inspector closed after invalid admission. Reselect the individual to retry.'); return;
      }
      this.pump(); return;
    }
    if (!message.enabled) this.active = null;
    this.options.status(message.reason ?? 'Inspector request rejected.');
    if (!this.enabled || /busy|current available run/u.test(message.reason ?? '')) this.later();
    else this.rejectedToken = pending.target?.token ?? null;
  }

  /** Accept only a complete current-target row or its explicit retirement. */
  snapshot(value: unknown): void {
    if (!this.enabled || this.pending || !this.active || this.active.target.token !== this.selected?.token) return;
    try {
      const snapshot = parseInspectorSnapshot(value);
      if (snapshot.subscriptionSequence !== this.active.sequence) return;
      const sequence = BigInt('0x' + snapshot.sequence);
      if (sequence <= this.afterSequence) return;
      if (snapshot.sample && (snapshot.sample.snakeId !== this.active.target.snakeId
        || (snapshot.sample.pureProbe === true) !== this.active.pureProbe)) return;
      this.afterSequence = sequence;
      this.options.changed(snapshot);
      this.options.status(snapshot.sample ? snapshot.sample.pureProbe ? 'Showing an explicit pure sensor probe; no controller consumed it.' : 'Showing the actual last delivered decision.'
        : this.active.pureProbe ? 'Waiting for the explicit sensor probe.' : 'Waiting for a delivered sample from the selected life.');
    } catch { this.options.status('Invalid inspector snapshot discarded.'); }
  }

  /** Retire timers and former-scope state on disconnect or page teardown. */
  dispose(): void { this.context(null); this.cancelRetry(); }

  /** Cancel only this client's single contention timer. */
  private cancelRetry(): void { if (this.retry !== undefined) clearTimeout(this.retry); this.retry = undefined; }
  /** Retry bounded admission without adding a second pending request. */
  private later(): void {
    if (!this.scope?.supported || this.retry !== undefined) return;
    this.retry = setTimeout(() => { this.retry = undefined; this.pump(); }, 250);
  }
  /** Reconcile acknowledged state with the newest explicit user/selection intent. */
  private pump(): void {
    if (!this.scope?.supported || this.pending) return;
    const target = this.closeRequired ? null : this.enabled && (!this.pureProbe || this.probeToken === this.selected?.token) ? this.selected : null;
    if (!this.closeRequired && target?.token === this.active?.target.token
      && (!target || (this.active?.pureProbe === this.pureProbe && this.active.revision === this.revision))) {
      this.options.status(target ? this.pureProbe ? 'Pure probe view; use Probe once to capture again or Show delivered to resume decisions.' : 'Waiting for the actual last delivered decision.' : this.enabled ? this.pureProbe ? 'Select a snake and choose Probe once for that physical life.' : 'Select a snake to inspect.' : 'Decision inspector is off.');
      return;
    }
    if (target && target.token === this.rejectedToken) return;
    const id = 'inspector-' + ++this.ordinal;
    this.pending = {id, target, pureProbe: this.pureProbe, revision: this.revision};
    this.options.status(target ? 'Admitting the selected inspector…' : 'Closing the decision inspector…');
    this.options.send({type: 'inspectEntity', requestId: id, expectedSessionId: this.scope.sessionId,
      expectedRunId: this.scope.runId, ...(target ? {selectionToken: target.token, snakeId: target.snakeId, ...(this.pureProbe ? {pureProbe: true} : {})} : {})});
  }
}
