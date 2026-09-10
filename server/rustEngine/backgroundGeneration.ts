import { randomBytes } from 'node:crypto';
import type { RustBackgroundEvent, RustBackgroundGenerationAssignment, RustBackgroundIdentity } from '../../src/protocol/rustBackground.ts';
import type { ExperimentalRunningAuthorityNativeHandle } from './backgroundRuntime.ts';
import { BackgroundCommandAdmission } from './commandAdmission.ts';
import { GenerationDeliveryRouter, type ControllerDeliveryPorts } from './controllerDelivery.ts';
import { managedCheckpointCommitResultMatchesDescriptor } from './checkpointPersistenceClient.ts';
import { parseManagedCheckpointDescriptor, parseManagedGenerationCommit } from './checkpointPersistenceProtocol.ts';
import type { GenerationCheckpointCommitter } from './generationPersistenceHandoff.ts';

/** Thin ports for a retained generation; all game values originate in Rust. */
export interface BackgroundGenerationOptions {
  /** Sole authority and bounded command queue. */
  native: ExperimentalRunningAuthorityNativeHandle;
  /** Shared sequence owner used by every controller and lifecycle command. */
  admission: BackgroundCommandAdmission;
  /** Dedicated SQLite metadata worker. */
  persistence: GenerationCheckpointCommitter;
  /** Controlled destination for immutable files. */
  managedDirectory: string;
  /** Maximum connected assignments admitted by the runtime. */
  maxAssignments: number;
  /** Exact local socket acceptance, without inspecting game state. */
  send: ControllerDeliveryPorts['send'];
  /** Free-disk admission before publishing this generation. */
  admitCheckpoint(): Promise<void>;
  /** Observe the complete transition-to-running durability barrier. */
  observeBarrier?(durationMs: number): void;
}

/**
 * Route one native generation barrier through disk, SQLite and local delivery.
 * The enclosing event pump awaits handle and retries flush at bounded wakeups;
 * it must stop on a thrown fault instead of publishing a successor itself.
 */
export class BackgroundGenerationRouter {
  /** Node orchestration phase; never a second simulation state. */
  private phase: 'idle' | 'disk' | 'publication' | 'commit' | 'ack' | 'prepare' | 'delivery' | 'resume' = 'idle';
  /** One unadmitted lifecycle command, retaining its exact scalar payload. */
  private pending: ((sequence: RustBackgroundIdentity) => void) | undefined;
  /** Correlate the next lifecycle reply with its admitted command. */
  private expectedSequence: RustBackgroundIdentity | undefined;
  /** One operation token survives publication and durability. */
  private operationId: string | undefined;
  /** Monotonic start of the active generation durability barrier. */
  private barrierStartedAt: number | undefined;
  /** Bounded send-once assignments and exact generation receipts. */
  private readonly delivery: GenerationDeliveryRouter;

  /** Bind transport to the shared generation receipt phase. */
  constructor(private readonly options: BackgroundGenerationOptions) {
    this.delivery = new GenerationDeliveryRouter(options.maxAssignments, {
      send: options.send,
      nextSequence: () => options.admission.nextSequence(),
      trySubmitReceipt: (sequence, receipt) => options.admission.trySubmitGenerationReceipt(sequence, receipt)
    });
  }

  /** Whether a generation is still retained by this interface. */
  get active(): boolean { return this.phase !== 'idle'; }

  /** Retry only bounded queue admission; never repeat a file commit or socket send. */
  flush(): boolean {
    if (!this.delivery.flushReceipts()) return false;
    const pending = this.pending;
    if (!pending) return true;
    if (!this.options.admission.trySubmitControl(sequence => {
      pending(sequence);
      this.expectedSequence = sequence;
    })) return false;
    this.pending = undefined;
    return true;
  }

  /** Retain one lifecycle command until capacity returns. */
  private queue(command: (sequence: RustBackgroundIdentity) => void): void {
    if (this.pending) throw new Error('generation command already pending');
    this.pending = command;
    this.flush();
  }

  /** Require the response to the exact lifecycle command issued in this phase. */
  private expect(event: RustBackgroundEvent, phase: typeof this.phase): void {
    if (this.phase !== phase || !this.expectedSequence || event.commandSequence !== this.expectedSequence) {
      throw new Error('unexpected background generation reply');
    }
    this.expectedSequence = undefined;
  }

  /** Consume generation events only; ordinary controller/display routing remains with the caller. */
  async handle(event: RustBackgroundEvent): Promise<boolean> {
    const { native, persistence, managedDirectory } = this.options;
    switch (event.kind) {
      case 'generationTransitionPending': {
        if (this.active) throw new Error('overlapping background generation transition');
        this.barrierStartedAt = performance.now();
        this.phase = 'disk';
        await this.options.admitCheckpoint();
        const operationId = randomBytes(16).toString('hex');
        this.operationId = operationId;
        this.phase = 'publication';
        this.queue(sequence => native.submitGenerationCheckpoint(sequence, { operationId, managedDirectory }));
        return true;
      }
      case 'generationCheckpointPublished': {
        this.expect(event, 'publication');
        const publication = event.checkpoint as {
          descriptor?: unknown;
          generationCommit?: unknown;
          hallOfFameWeights?: unknown;
        } | undefined;
        const descriptor = parseManagedCheckpointDescriptor(publication?.descriptor);
        if (descriptor.operationId !== this.operationId) throw new Error('generation publication operation mismatch');
        const rawRecord = publication?.generationCommit;
        const record = parseManagedGenerationCommit(rawRecord && typeof rawRecord === 'object' && !Array.isArray(rawRecord)
          ? { ...rawRecord, hallOfFameWeights: publication?.hallOfFameWeights }
          : rawRecord, descriptor);
        if (!record) throw new Error('generation publication omitted history');
        this.phase = 'commit';
        const committed = await persistence.commit(descriptor, record);
        if (!managedCheckpointCommitResultMatchesDescriptor(committed, descriptor)) {
          throw new Error('generation persistence acknowledgement mismatch');
        }
        this.phase = 'ack';
        this.queue(sequence => native.submitGenerationPersistenceAcknowledgement(sequence, descriptor));
        return true;
      }
      case 'generationPersistenceAcknowledged':
        this.expect(event, 'ack');
        if (event.acknowledgedOperationId !== this.operationId) throw new Error('generation acknowledgement operation mismatch');
        this.phase = 'prepare';
        this.queue(sequence => native.submitPrepareGenerationReassignments(sequence));
        return true;
      case 'generationReassignmentsPrepared': {
        this.expect(event, 'prepare');
        const result = event.reassignments as { ready?: unknown; assignments?: RustBackgroundGenerationAssignment[] } | undefined;
        if (!result || typeof result.ready !== 'boolean' || !Array.isArray(result.assignments)) {
          throw new TypeError('invalid generation reassignments');
        }
        this.phase = 'delivery';
        if (result.ready) {
          if (result.assignments.length !== 0) throw new Error('ready generation still has assignments');
          this.resume();
        } else if (!this.delivery.deliverAssignments(result.assignments)) {
          throw new Error('generation delivery already pending');
        }
        return true;
      }
      case 'generationAssignmentReceiptsApplied': {
        if (this.phase !== 'delivery') throw new Error('generation receipt outside delivery');
        const result = event.receiptResolution as { state?: unknown } | undefined;
        if (result?.state === 'ready') {
          if (this.delivery.blocked) throw new Error('generation ready before all local receipts admitted');
          this.resume();
        } else if (result?.state !== 'pending') throw new TypeError('invalid generation receipt resolution');
        return true;
      }
      case 'generationStartPublished':
        this.expect(event, 'resume');
        if (this.barrierStartedAt !== undefined) {
          this.options.observeBarrier?.(performance.now() - this.barrierStartedAt);
        }
        this.phase = 'idle';
        this.operationId = undefined;
        this.barrierStartedAt = undefined;
        return true;
      case 'commandRejected':
        if (event.commandSequence === this.expectedSequence && this.expectedSequence) {
          throw new Error(`generation command rejected: ${event.rejectionCode}: ${event.rejectionDetail}`);
        }
        return false;
      default: return false;
    }
  }

  /** Ask Rust for the sole authority swap only after its durability/delivery gates. */
  private resume(): void {
    this.phase = 'resume';
    this.queue(sequence => this.options.native.submitPublishGenerationStart(sequence));
  }
}
