import { isSupportedSensorCount } from '../../src/protocol/sensors.ts';
import type { AssignMsg, SensorsMsg, ReclaimResultMsg, RewardTransitionMsg } from '../protocol.ts';
import { EXTERNAL_REWARD_MAX_ENVELOPE_BYTES, type ExternalRewardEnvelopeV1 } from '../../contracts/types.ts';
import type {
  RustBackgroundControllerMessage,
  RustBackgroundIdentity,
  RustGenerationAssignmentReceipt,
  RustBackgroundReclaimAssignment,
  RustBackgroundReclaimReceipt
} from '../../src/protocol/rustBackground.ts';
import type { RustBackgroundGenerationAssignment } from '../../src/protocol/rustBackground.ts';

/** Transport and bounded command admission supplied by the background event router. */
export interface ControllerDeliveryPorts {
  /** Current Rust-admitted width; historical fixture adapters may omit this v3-only hint. */
  sensorCount?(): number;
  /** Route to the exact socket epoch; false means the local send was not accepted. */
  send(connectionId: RustBackgroundIdentity, message: AssignMsg | SensorsMsg | RewardTransitionMsg): boolean;
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
  /** Old-life closure must be accepted before the replacement assignment. */
  closure: RewardTransitionMsg | undefined;
  /** Internal command sequence assigned when the receipt first reaches the queue. */
  sequence?: RustBackgroundIdentity;
}

/** Why an externally controlled snake was replaced without dying. */
type ReplacementReason = 'generation';
/** Internal delivery message carrying transport-only replacement context. */
type ReplacementControllerMessage = RustBackgroundControllerMessage & {
  replacementReason?: ReplacementReason;
};
/** Assignment packet with optional additive replacement context for newer clients. */
type ReplacementAssignMsg = AssignMsg & { replacementReason?: ReplacementReason };

/** Exact positive Rust identity accepted at this internal routing boundary. */
const POSITIVE_ID = /^(?!0{16}$)[0-9a-f]{16}$/u;
/** Largest integer that Protocol 2 can represent without narrowing. */
const MAX_WIRE_INTEGER = BigInt(Number.MAX_SAFE_INTEGER);

/** Reject malformed internal identity fields before a batch can reach a socket. */
function identity(value: string): RustBackgroundIdentity {
  if (!POSITIVE_ID.test(value)) throw new TypeError('invalid Rust controller identity');
  return value;
}

/** Validate bounded native routing facts without reconstructing game accounting. */
function nativeReward(message: RustBackgroundControllerMessage): ExternalRewardEnvelopeV1 | undefined {
  if (message.rewardJson === undefined) return undefined;
  if (typeof message.rewardJson !== 'string' || Buffer.byteLength(message.rewardJson, 'utf8') > EXTERNAL_REWARD_MAX_ENVELOPE_BYTES) {
    throw new TypeError('native reward exceeds its wire bound');
  }
  const reward = JSON.parse(message.rewardJson) as ExternalRewardEnvelopeV1;
  if (!reward || reward.rewardVersion !== 1 || reward.observationSequence !== message.eventSequence ||
      typeof reward.runId !== 'string' || reward.runId.length === 0 ||
      !/^[0-9a-f]{16}$/u.test(reward.toStep) || !reward.deltas ||
      !Object.values(reward.deltas).every(value => typeof value === 'number' && Number.isFinite(value))) {
    throw new TypeError('invalid native reward envelope');
  }
  if (!reward.state || !Number.isSafeInteger(reward.state.startingLength) ||
      !Number.isSafeInteger(reward.state.bodyLength) || !Number.isSafeInteger(reward.state.retainedGrowth) ||
      reward.state.startingLength <= 0 || reward.state.bodyLength <= 0 || reward.state.retainedGrowth < 0) {
    throw new TypeError('invalid native reward boundary state');
  }
  identity(reward.worldEpoch);
  identity(reward.leaseId);
  identity(reward.physicalSnakeId);
  if (!Number.isInteger(reward.snakeId) || reward.snakeId <= 0 || reward.snakeId > 16_777_216) {
    throw new TypeError('invalid native reward snake identity');
  }
  if (message.kind === 'observation' &&
      (reward.leaseId !== message.leaseId || reward.physicalSnakeId !== message.internalSnakeId ||
       reward.snakeId !== message.snakeId || reward.toStep !== message.sourceCompletedStep ||
       !['initial', 'live'].includes(reward.transitionKind))) {
    throw new TypeError('native observation reward identity mismatch');
  }
  return reward;
}

/** A closure travels on the successor transaction but names only the old life. */
function prepareClosure(message: RustBackgroundControllerMessage, expectedSensorCount?: number): RewardTransitionMsg | undefined {
  if (message.kind !== 'replacementAssignment') return undefined;
  const reward = nativeReward(message);
  const finalSensors = message.finalSensors;
  if (!reward) {
    if (finalSensors !== undefined) throw new TypeError('cutoff sensors omit their reward identity');
    return undefined;
  }
  if (reward.transitionKind !== 'terminal' && reward.transitionKind !== 'truncated') {
    throw new TypeError('assignment reward must close the old life');
  }
  if (reward.transitionKind === 'terminal') {
    if (reward.reason !== 'death' || finalSensors !== undefined) throw new TypeError('invalid terminal closure');
    return { type: 'transition', reward };
  }
  if (!reward.reason || reward.reason === 'death' || !Array.isArray(finalSensors) ||
      !isSupportedSensorCount(finalSensors.length) ||
      (expectedSensorCount !== undefined && finalSensors.length !== expectedSensorCount) ||
      !finalSensors.every(Number.isFinite)) {
    throw new TypeError('truncation requires final old-life sensors');
  }
  return { type: 'transition', reward, finalSensors };
}

/** Translate one Rust envelope without calculating sensors or looking up game state. */
function prepareMessage(
  message: RustBackgroundControllerMessage,
  expectedSensorCount?: number
): AssignMsg | SensorsMsg {
  identity(message.operationEpoch);
  identity(message.eventSequence);
  identity(message.connectionId);
  identity(message.leaseId);
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
    const replacementReason = (message as ReplacementControllerMessage).replacementReason;
    if (replacementReason !== undefined && replacementReason !== 'generation') {
      throw new TypeError('invalid controller replacement reason');
    }
    const assignment: ReplacementAssignMsg = {
      type: 'assign', snakeId: message.snakeId,
      controller: message.controllerKind === 'player' ? 'player' : 'bot',
      resumeToken: message.resumeToken
    };
    if (replacementReason !== undefined) assignment.replacementReason = replacementReason;
    return assignment;
  }
  if (message.kind !== 'observation' || !/^[0-9a-f]{16}$/u.test(message.sourceCompletedStep)) {
    throw new TypeError('invalid Rust controller observation');
  }
  const tick = BigInt(`0x${message.sourceCompletedStep}`);
  if (tick > MAX_WIRE_INTEGER) throw new RangeError('observation tick exceeds Protocol 2 exact integers');
  const { sensors, x, y, direction } = message;
  if (!Array.isArray(sensors) || !isSupportedSensorCount(sensors.length) ||
      (expectedSensorCount === undefined ? (sensors.length<51 || sensors.length>147 || (sensors.length-19)%4!==0) :
        (!isSupportedSensorCount(expectedSensorCount) || sensors.length!==expectedSensorCount)) || !sensors.every(Number.isFinite) ||
      typeof x !== 'number' || !Number.isFinite(x) || typeof y !== 'number' || !Number.isFinite(y) ||
      typeof direction !== 'number' || !Number.isFinite(direction)) {
    throw new TypeError('controller observation omits the admitted sensor values or pose');
  }
  const reward = nativeReward(message);
  return {
    type: 'sensors', tick: Number(tick), snakeId: message.snakeId,
    sensors, meta: { x, y, dir: direction }, ...(reward ? { reward } : {})
  };
}

/** Add a retained receipt only for delivery paths whose success changes authority state. */
function prepare(message: RustBackgroundControllerMessage, expectedSensorCount?: number): PendingDelivery {
  return {
    receipt: {
      operationEpoch: identity(message.operationEpoch),
      eventSequence: identity(message.eventSequence),
      connectionId: identity(message.connectionId),
      leaseId: identity(message.leaseId),
      accepted: false
    },
    message: prepareMessage(message, expectedSensorCount),
    closure: prepareClosure(message, expectedSensorCount)
  };
}

/** Transport for ordered observations admitted to the native output stream. */
export interface ControllerObservationPorts {
  /** Current Rust-admitted sensor width. */
  sensorCount?(): number;
  /** Route to the exact socket epoch; false means local transport rejected it. */
  send(connectionId: RustBackgroundIdentity, message: SensorsMsg): boolean;
  /** Report only an exact failed lease/connection so Rust can disconnect asynchronously. */
  failed(connectionId: RustBackgroundIdentity, leaseId: RustBackgroundIdentity): void;
}

/** Send admitted observation batches without manufacturing native receipts. */
export class ControllerObservationRouter {
  constructor(private readonly maxMessages: number, private readonly ports: ControllerObservationPorts) {
    if (!Number.isSafeInteger(maxMessages) || maxMessages <= 0) {
      throw new RangeError('controller observation capacity must be a positive safe integer');
    }
  }

  /** Validate the complete batch before the first externally visible send. */
  deliver(messages: readonly RustBackgroundControllerMessage[]): void {
    if (messages.length === 0 || messages.length > this.maxMessages) {
      throw new RangeError('controller observation batch exceeds admitted message capacity');
    }
    if (messages.some(message => message.kind !== 'observation')) {
      throw new TypeError('asynchronous controller output may contain observations only');
    }
    const sensorCount = this.ports.sensorCount?.();
    const prepared = messages.map(message => prepareMessage(message, sensorCount));
    for (const item of prepared) {
      if (item.type !== 'sensors') {
        throw new TypeError('asynchronous controller output prepared a non-observation');
      }
    }
    for (let index = 0; index < prepared.length; index++) {
      const item = prepared[index]! as SensorsMsg;
      const source = messages[index]!;
      let accepted = false;
      try { accepted = this.ports.send(source.connectionId, item); }
      catch { accepted = false; }
      if (!accepted) this.ports.failed(source.connectionId, source.leaseId);
    }
  }
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
  constructor(protected readonly maxMessages: number, private readonly ports: ControllerDeliveryPorts) {
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
    const sensorCount = this.ports.sensorCount?.();
    const prepared = messages.map(message=>prepare(message,sensorCount));
    this.pending = prepared;
    this.cursor = 0;
    this.busy = true;
    try {
      for (const item of prepared) {
        // A thrown send is a local delivery failure, just like a closed socket.
        try {
          item.receipt.accepted = (!item.closure || this.ports.send(item.receipt.connectionId, item.closure)) &&
            this.ports.send(item.receipt.connectionId, item.message);
        }
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

/** Thin transport ports for one retained Rust reclaim assignment. */
export interface ReclaimDeliveryPorts {
  /** Exact socket send, preserving the existing Protocol 2 result/assign order. */
  send(connectionId: RustBackgroundIdentity, message: AssignMsg | ReclaimResultMsg): boolean;
  /** Allocate from the shared command admission stream. */
  nextSequence(): RustBackgroundIdentity;
  /** Retry only the exact local-send result when the bounded queue is full. */
  trySubmitReceipt(sequence: RustBackgroundIdentity, receipt: RustBackgroundReclaimReceipt): boolean;
}

/** Deliver a reconnect once, retaining its completion until Rust admits it. */
export class ReclaimDeliveryRouter {
  /** One bounded completion; no control or world state is duplicated here. */
  private pending: { receipt: RustBackgroundReclaimReceipt; sequence?: RustBackgroundIdentity } | undefined;
  /** Protect the retained operation from synchronous send callbacks. */
  private busy = false;

  /** Bind the existing socket transport and shared queue admission adapter. */
  constructor(private readonly ports: ReclaimDeliveryPorts, private readonly reclaimed = true) {}

  /** The event pump must retain its next delivery while this is true. */
  get blocked(): boolean { return this.busy || this.pending !== undefined; }

  /** Validate the entire result/assignment pair before the first socket send. */
  deliver(assignment: RustBackgroundReclaimAssignment): boolean {
    if (this.blocked) return false;
    const receipt: RustBackgroundReclaimReceipt = {
      requestSequence: identity(assignment.requestSequence), connectionId: identity(assignment.connectionId),
      leaseId: identity(assignment.leaseId), accepted: false
    };
    if (!Number.isInteger(assignment.snakeId) || assignment.snakeId <= 0 || assignment.snakeId > 16_777_216 ||
      !/^[0-9a-f]{16}$/u.test(assignment.completedStep) ||
      !/^[A-Za-z0-9_-]{32}$/u.test(assignment.resumeToken) ||
      (assignment.controllerKind !== 'player' && assignment.controllerKind !== 'reinforcementLearning')) {
      throw new TypeError('invalid Rust reclaim assignment');
    }
    const result: ReclaimResultMsg = { type: 'reclaimResult', reclaimed: true, reason: 'reclaimed', snakeId: assignment.snakeId };
    const assign: AssignMsg = { type: 'assign', snakeId: assignment.snakeId,
      controller: assignment.controllerKind === 'player' ? 'player' : 'bot', resumeToken: assignment.resumeToken, reclaimed: this.reclaimed };
    this.pending = { receipt };
    this.busy = true;
    try {
      receipt.accepted = (!this.reclaimed || this.ports.send(receipt.connectionId, result)) && this.ports.send(receipt.connectionId, assign);
    } catch { receipt.accepted = false; }
    finally { this.busy = false; }
    this.flushReceipts();
    return true;
  }

  /** Retry queue admission without repeating either Protocol 2 message. */
  flushReceipts(): boolean {
    if (this.busy) return false;
    const pending = this.pending;
    if (!pending) return true;
    this.busy = true;
    try {
      pending.sequence ??= identity(this.ports.nextSequence());
      if (!this.ports.trySubmitReceipt(pending.sequence, pending.receipt)) return false;
      this.pending = undefined;
      return true;
    } finally { this.busy = false; }
  }
}

/** Deliver one fresh assignment without a reconnect-result packet. */
export class JoinDeliveryRouter extends ReclaimDeliveryRouter {
  /** Bind the separate fresh-receipt port to the shared transport behavior. */
  constructor(ports: ReclaimDeliveryPorts) { super(ports, false); }
}

/** Route successor assignments through the same bounded, send-once batch owner. */
export class GenerationDeliveryRouter extends ControllerDeliveryRouter {
  /** Validate every public identity before the complete batch can reach sockets. */
  deliverAssignments(assignments: readonly RustBackgroundGenerationAssignment[]): boolean {
    if (this.blocked) return false;
    if (assignments.length === 0 || assignments.length > this.maxMessages) {
      throw new RangeError('generation batch exceeds admitted message capacity');
    }
    const messages = assignments.map(assignment => {
      const frameId = BigInt(`0x${identity(assignment.frameV1Id)}`);
      if (frameId > 16_777_216n || !/^[A-Za-z0-9_-]{32}$/u.test(assignment.resumeToken)) {
        throw new TypeError('invalid Rust generation assignment');
      }
      return {
        ...assignment, internalSnakeId: identity(assignment.snakeId), snakeId: Number(frameId),
        sourceCompletedStep: '0000000000000000', kind: 'replacementAssignment' as const,
        replacementReason: 'generation' as const
      };
    });
    return this.deliver(messages);
  }
}
