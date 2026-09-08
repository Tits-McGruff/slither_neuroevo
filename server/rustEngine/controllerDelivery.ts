import type { AssignMsg, SensorsMsg } from '../protocol.ts';
import type {
  RustBackgroundControllerMessage,
  RustBackgroundIdentity,
  RustGenerationAssignmentReceipt
} from '../../src/protocol/rustBackground.ts';

/** Transport and bounded command admission supplied by the background event router. */
export interface ControllerDeliveryPorts {
  /** Route to the exact socket epoch; false means the local send was not accepted. */
  send(connectionId: RustBackgroundIdentity, message: AssignMsg | SensorsMsg): boolean;
  /** Allocate from the shared ordered command pump, which must hold later commands while blocked. */
  nextSequence(): RustBackgroundIdentity;
  /** Return false only for queue capacity; other native failures must propagate. */
  trySubmitReceipt(sequence: RustBackgroundIdentity, receipt: RustGenerationAssignmentReceipt): boolean;
}

/** One send and its retained exact completion, allocated before transport starts. */
interface PendingDelivery {
  /** Rust-issued correlation fields, with the actual transport outcome filled once. */
  receipt: RustGenerationAssignmentReceipt;
  /** Existing Protocol 2 envelope containing Rust-produced values. */
  message: AssignMsg | SensorsMsg;
  /** Internal command sequence assigned when the receipt first reaches the queue. */
  sequence?: RustBackgroundIdentity;
}

/** Exact positive Rust identity accepted at this internal routing boundary. */
const POSITIVE_ID = /^(?!0{16}$)[0-9a-f]{16}$/u;
/** Largest integer that Protocol 2 can represent without narrowing. */
const MAX_WIRE_INTEGER = BigInt(Number.MAX_SAFE_INTEGER);

/** Reject malformed internal identity fields before a batch can reach a socket. */
function identity(value: string): RustBackgroundIdentity {
  if (!POSITIVE_ID.test(value)) throw new TypeError('invalid Rust controller identity');
  return value;
}

/** Translate a complete Rust envelope without calculating sensors or looking up game state. */
function prepare(message: RustBackgroundControllerMessage): PendingDelivery {
  const receipt: RustGenerationAssignmentReceipt = {
    operationEpoch: identity(message.operationEpoch),
    eventSequence: identity(message.eventSequence),
    connectionId: identity(message.connectionId),
    leaseId: identity(message.leaseId),
    accepted: false
  };
  if (!Number.isInteger(message.snakeId) || message.snakeId <= 0 || message.snakeId > 16_777_216) {
    throw new RangeError('controller snake ID exceeds the frame-v1 identity range');
  }
  if (message.controllerKind !== 'player' && message.controllerKind !== 'reinforcementLearning') {
    throw new TypeError('invalid Rust controller kind');
  }
  if (message.kind === 'replacementAssignment') {
    if (typeof message.resumeToken !== 'string' || message.resumeToken.length === 0) {
      throw new TypeError('controller assignment omits its resume token');
    }
    return { receipt, message: {
      type: 'assign', snakeId: message.snakeId,
      controller: message.controllerKind === 'player' ? 'player' : 'bot',
      resumeToken: message.resumeToken
    } };
  }
  if (message.kind !== 'observation' || !/^[0-9a-f]{16}$/u.test(message.sourceCompletedStep)) {
    throw new TypeError('invalid Rust controller observation');
  }
  const tick = BigInt(`0x${message.sourceCompletedStep}`);
  if (tick > MAX_WIRE_INTEGER) throw new RangeError('observation tick exceeds Protocol 2 exact integers');
  const { sensors, x, y, direction } = message;
  if (!Array.isArray(sensors) || sensors.length < 51 || sensors.length > 147 ||
      (sensors.length - 19) % 4 !== 0 || !sensors.every(Number.isFinite) ||
      typeof x !== 'number' || !Number.isFinite(x) || typeof y !== 'number' || !Number.isFinite(y) ||
      typeof direction !== 'number' || !Number.isFinite(direction)) {
    throw new TypeError('controller observation omits valid sensor-v3 values or pose');
  }
  return { receipt, message: {
    type: 'sensors', tick: Number(tick), snakeId: message.snakeId,
    sensors, meta: { x, y, dir: direction }
  } };
}

/**
 * Bounded adapter for one reliable ordinary-step batch. The event pump must stop
 * draining controller batches while this adapter retains send completions.
 * Queue backpressure retries only receipts; socket sends happen exactly once.
 */
export class ControllerDeliveryRouter {
  /** Retained completions for the one admitted batch. */
  private pending: PendingDelivery[] = [];
  /** First completion not yet admitted to Rust's bounded input queue. */
  private cursor = 0;
  /** Guard transport callbacks against recursively delivering or flushing a batch. */
  private busy = false;

  /** Bound the adapter by the maximum controller messages admitted for this runtime. */
  constructor(private readonly maxMessages: number, private readonly ports: ControllerDeliveryPorts) {
    if (!Number.isSafeInteger(maxMessages) || maxMessages <= 0) {
      throw new RangeError('controller batch capacity must be a positive safe integer');
    }
  }

  /** Whether the event pump must retain its next batch until receipt capacity returns. */
  get blocked(): boolean { return this.busy || this.cursor < this.pending.length; }

  /** Validate and reserve the complete batch before the first externally visible send. */
  deliver(messages: readonly RustBackgroundControllerMessage[]): boolean {
    if (this.blocked) return false;
    if (messages.length === 0 || messages.length > this.maxMessages) {
      throw new RangeError('controller batch exceeds admitted message capacity');
    }
    const prepared = messages.map(prepare);
    this.pending = prepared;
    this.cursor = 0;
    this.busy = true;
    try {
      for (const item of prepared) {
        // A thrown send is a local delivery failure, just like a closed socket.
        try { item.receipt.accepted = this.ports.send(item.receipt.connectionId, item.message); }
        catch { item.receipt.accepted = false; }
      }
    } finally { this.busy = false; }
    this.flushReceipts();
    return true;
  }

  /** Retry retained completions after input capacity returns, without repeating sends. */
  flushReceipts(): boolean {
    if (this.busy) return false;
    this.busy = true;
    try {
      while (this.cursor < this.pending.length) {
        const item = this.pending[this.cursor]!;
        item.sequence ??= identity(this.ports.nextSequence());
        if (!this.ports.trySubmitReceipt(item.sequence, item.receipt)) return false;
        this.cursor++;
      }
      this.pending = [];
      this.cursor = 0;
      return true;
    } finally { this.busy = false; }
  }
}
