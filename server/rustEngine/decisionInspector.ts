import { parseInspectorSnapshot, type InspectEntityMsg, type InspectorMsg, type InspectorResultMsg } from '../../src/protocol/inspector.ts';
import type { RustBackgroundEvent } from '../../src/protocol/rustBackground.ts';
import type { ExperimentalRunningAuthorityNativeHandle } from './backgroundRuntime.ts';
import type { BackgroundCommandAdmission } from './commandAdmission.ts';

/** Only metadata and coarse native operations enter this diagnostic router. */
export interface DecisionInspectorOptions {
  /** Current authority after any durable replacement. */ runtime(): Pick<ExperimentalRunningAuthorityNativeHandle, 'submitInspector' | 'latestInspector'>;
  /** Shared gameplay/receipt sequence owner. */ admission: Pick<BackgroundCommandAdmission, 'trySubmitControl'>;
  /** Current observed process/run and cold-operation availability. */ scope(): {sessionId: string; runId: string; available: boolean};
  /** Priority admission results. */ reliable(connection: number, result: InspectorResultMsg): boolean;
  /** One newest unsent snapshot. */ replaceable(connection: number, snapshot: InspectorMsg): boolean;
  /** Retire any unsent row before acknowledging a subscription change. */ clear(connection: number): void;
  /** Wake the ordinary output pump after command admission. */ schedule(): void;
}
/** Bounded pending command; disconnect never requires retaining a closed socket. */
interface PendingInspector {
  /** Null identifies an internal orphan cleanup. */ connection: number | null;
  /** Client request, or empty for cleanup. */ requestId: string;
  /** Global native command identity. */ sequence: string;
  /** Requested capture state. */ enabled: boolean;
  /** Exact displayed target. */ snakeId?: number;
  /** Whether the original peer still exists. */ connected: boolean;
}
/** One global selected capture, independent of gameplay controllers and reliable output. */
export class DecisionInspectorRouting {
  /** At most one client owns native capture. */ private owner: number | undefined;
  /** Exact acknowledged command prevents old mailbox rows from crossing target changes. */ private subscription: string | undefined;
  /** Current reliable correlation for asynchronous diagnostic errors. */ private requestId = '';
  /** Native frame identity paired with the acknowledged token. */ private snakeId: number | undefined;
  /** One bounded command awaiting a native result. */ private pending: PendingInspector | undefined;
  /** A closed owner must be disabled before another owner can be admitted. */ private orphaned = false;
  /** Latest native mailbox read, including filtered rows. */ private afterSequence = '0000000000000000';

  /** Attach to the existing global command and socket owners. */
  constructor(private readonly options: DecisionInspectorOptions) {}

  /** Admit one joined UI peer's exact current-scope intent. */
  request(connection: number, message: InspectEntityMsg): void {
    const scope = this.options.scope();
    const reject = (reason: string): void => { this.options.reliable(connection, {type: 'inspectorResult',
      requestId: message.requestId, applied: false, enabled: this.owner === connection, reason}); };
    if (!scope.available || message.expectedSessionId !== scope.sessionId || message.expectedRunId !== scope.runId) {
      reject('inspector requires the current available run'); return;
    }
    if (this.pending || this.orphaned || (this.owner !== undefined && this.owner !== connection)) {
      reject('the selected inspector is busy'); return;
    }
    const enabled = message.selectionToken !== undefined;
    try {
      let sequence: string | undefined;
      const admitted = this.options.admission.trySubmitControl(value => {
        this.options.runtime().submitInspector(value, scope.runId, message.selectionToken, message.snakeId, message.pureProbe);
        sequence = value;
      });
      if (!admitted || !sequence) { reject('authoritative command queue is busy'); return; }
      this.pending = {connection, requestId: message.requestId, sequence, enabled,
        ...(message.snakeId === undefined ? {} : {snakeId: message.snakeId}), connected: true};
      this.options.schedule();
    } catch (error) { reject(error instanceof Error ? error.message.slice(0, 512) : 'inspector admission failed'); }
  }

  /** Resolve only this router's exact pending command. */
  event(event: RustBackgroundEvent): void {
    const pending = this.pending;
    if (!pending || event.commandSequence !== pending.sequence
      || (event.kind !== 'inspectorChanged' && event.kind !== 'commandRejected')) return;
    this.pending = undefined;
    const applied = event.kind === 'inspectorChanged' && event.inspectorEnabled === pending.enabled;
    if (applied) {
      if (pending.connection !== null) this.options.clear(pending.connection);
      this.owner = pending.enabled && pending.connected ? pending.connection ?? undefined : undefined;
      this.subscription = pending.enabled ? pending.sequence : undefined;
      this.snakeId = pending.snakeId;
      this.requestId = pending.requestId;
      this.orphaned = pending.enabled && !pending.connected;
    }
    if (pending.connected && pending.connection !== null) {
      const accepted = this.options.reliable(pending.connection, {type: 'inspectorResult', requestId: pending.requestId,
        applied, enabled: this.owner === pending.connection,
        ...(applied ? {subscriptionSequence: pending.sequence} : {reason: (event.rejectionDetail ?? 'Rust rejected inspector').slice(0, 512)})});
      if (!accepted && this.owner === pending.connection) this.disconnect(pending.connection);
    }
  }

  /** Release an owner, including a subscription whose enable command is still in flight. */
  disconnect(connection: number): void {
    if (this.pending?.connection === connection) this.pending.connected = false;
    if (this.owner === connection) { this.owner = undefined; this.orphaned = true; }
    this.options.schedule();
  }

  /** Invalidate old intent only after a new authority or settings cache is published. */
  reset(reason = 'inspector scope changed'): void {
    if (this.owner !== undefined) {
      this.options.clear(this.owner);
      this.options.reliable(this.owner, {type: 'inspectorResult', requestId: this.requestId,
        applied: false, enabled: false, reason});
    }
    if (this.pending?.connected && this.pending.connection !== null) this.options.reliable(this.pending.connection,
      {type: 'inspectorResult', requestId: this.pending.requestId, applied: false, enabled: false, reason});
    this.orphaned = this.owner !== undefined || this.pending !== undefined;
    if (this.pending) this.pending.connected = false;
    this.owner = undefined; this.subscription = undefined;
    this.snakeId = undefined; this.afterSequence = '0000000000000000';
  }

  /** Called only after priority output drains; no metadata is copied while closed. */
  service(): void {
    const scope = this.options.scope();
    if (!scope.available) return;
    if (this.orphaned && !this.pending) {
      let sequence: string | undefined;
      try {
        if (this.options.admission.trySubmitControl(value => {
          this.options.runtime().submitInspector(value, scope.runId); sequence = value;
        }) && sequence) {
          this.pending = {connection: null, requestId: '', sequence, enabled: false, connected: false};
          this.options.schedule();
        }
      } catch { /* Authority shutdown or replacement can temporarily reject diagnostic cleanup. */ }
      return;
    }
    if (this.owner === undefined || !this.subscription) return;
    try {
      const json = this.options.runtime().latestInspector(this.afterSequence);
      if (json === null) return;
      if (Buffer.byteLength(json, 'utf8') > 64 * 1024) throw new Error('inspector projection exceeds byte admission');
      const snapshot = parseInspectorSnapshot(JSON.parse(json));
      if (BigInt('0x' + snapshot.sequence) <= BigInt('0x' + this.afterSequence)) throw new Error('inspector mailbox sequence did not advance');
      this.afterSequence = snapshot.sequence;
      if (snapshot.subscriptionSequence !== this.subscription) return;
      if (snapshot.sample && snapshot.sample.snakeId !== this.snakeId) throw new Error('inspector target does not match its subscription');
      if (!this.options.replaceable(this.owner, {type: 'inspector', snapshot})) this.disconnect(this.owner);
    } catch (error) {
      const owner = this.owner;
      this.options.reliable(owner, {type: 'inspectorResult', requestId: this.requestId, applied: false, enabled: false,
        reason: error instanceof Error ? error.message.slice(0, 512) : 'inspector projection failed'});
      this.disconnect(owner);
    }
  }
}
