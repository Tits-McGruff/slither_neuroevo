import type { RustBackgroundIdentity, RustGenerationAssignmentReceipt } from '../../src/protocol/rustBackground.ts';
import type { ExperimentalRunningAuthorityNativeHandle } from './backgroundRuntime.ts';

/** Maximum command identity admitted by the Rust queue. */
const MAX_SEQUENCE = (1n << 64n) - 1n;

/** Recognize only the native bridge's explicit bounded-admission rejections. */
function isCapacityRejection(error: unknown): boolean {
  return error instanceof Error && /^(QueueCountLimit|QueueByteLimit): /u.test(error.message);
}

/** Compare the complete retained transport result, including its success/failure bit. */
function sameReceipt(a: RustGenerationAssignmentReceipt, b: RustGenerationAssignmentReceipt): boolean {
  return a.operationEpoch === b.operationEpoch && a.eventSequence === b.eventSequence &&
    a.connectionId === b.connectionId && a.leaseId === b.leaseId && a.accepted === b.accepted;
}

/**
 * Assign global command sequences only at native admission. Rejected controls
 * retain no sequence reservation, allowing a delivery receipt to unblock Rust.
 * A receipt already attempted by the delivery router pins its exact sequence;
 * later controls wait until that receipt is admitted. This owns at most one
 * small receipt; callers retain bounded ordered controls/latest player input.
 */
export class BackgroundCommandAdmission {
  /** Next identity, consumed only when Rust accepts a command. */
  private next: bigint;
  /** One failed receipt admission whose identity must survive exact retry. */
  private pendingReceipt: RustGenerationAssignmentReceipt | undefined;
  /** Guard synchronous native callbacks against reentrant submissions. */
  private busy = false;

  /** Start at one, or immediately after commands already submitted during startup. */
  constructor(
    private readonly native: Pick<ExperimentalRunningAuthorityNativeHandle, 'submitControllerDeliveryReceipt'>,
    firstSequence = 1n
  ) {
    if (typeof firstSequence !== 'bigint' || firstSequence <= 0n || firstSequence > MAX_SEQUENCE) {
      throw new RangeError('invalid first command sequence');
    }
    this.next = firstSequence;
  }

  /** Read the current sequence for a receipt; allocation does not consume it. */
  nextSequence(): RustBackgroundIdentity {
    if (this.next > MAX_SEQUENCE) throw new RangeError('background command sequence exhausted');
    return this.next.toString(16).padStart(16, '0');
  }

  /** True while a retained receipt must precede any later ordinary command. */
  get blocked(): boolean { return this.busy || this.pendingReceipt !== undefined; }

  /** Try an ordered control or newest unsent action; capacity rejection never pins a sequence. */
  trySubmitControl(submit: (sequence: RustBackgroundIdentity) => void): boolean {
    if (this.blocked) return false;
    return this.attempt(submit);
  }

  /** Adapt directly to ControllerDeliveryRouter's retained receipt port. */
  trySubmitReceipt(sequence: RustBackgroundIdentity, receipt: RustGenerationAssignmentReceipt): boolean {
    if (this.busy) return false;
    if (sequence !== this.nextSequence()) throw new Error('receipt command sequence is stale or out of order');
    if (this.pendingReceipt && !sameReceipt(this.pendingReceipt, receipt)) {
      throw new Error('another receipt owns the pending command sequence');
    }
    this.pendingReceipt ??= { ...receipt };
    const accepted = this.attempt(current => this.native.submitControllerDeliveryReceipt(current, receipt));
    if (accepted) this.pendingReceipt = undefined;
    return accepted;
  }

  /** Submit synchronously, advancing the shared sequence only on successful queue admission. */
  private attempt(submit: (sequence: RustBackgroundIdentity) => void): boolean {
    const sequence = this.nextSequence();
    this.busy = true;
    try {
      submit(sequence);
      this.next++;
      return true;
    } catch (error) {
      if (isCapacityRejection(error)) return false;
      throw error;
    } finally { this.busy = false; }
  }
}
