import type { RustBackgroundEvent, RustBackgroundIdentity } from '../../src/protocol/rustBackground.ts';
import type { AssignMsg, ReclaimResultMsg, SensorsMsg } from '../protocol.ts';
import { BackgroundFramePool, type BackgroundFrameLease } from './backgroundFrames.ts';
import { BackgroundGenerationRouter } from './backgroundGeneration.ts';
import { BackgroundCommandAdmission } from './commandAdmission.ts';
import { ControllerDeliveryRouter, JoinDeliveryRouter, ReclaimDeliveryRouter } from './controllerDelivery.ts';
import type { ExperimentalServerRuntime } from './experimentalStartup.ts';

/** Transport hooks for the sole background event consumer. */
export interface BackgroundOutputOptions {
  /** Durable, unstarted authority produced by experimental startup. */
  owner: ExperimentalServerRuntime;
  /** Exact socket acceptance; only small Protocol 2 messages cross here. */
  send(connectionId: RustBackgroundIdentity, message: AssignMsg | SensorsMsg | ReclaimResultMsg): boolean;
  /** Observe scalar routing/lifecycle results before delivery of their messages. */
  event(event: RustBackgroundEvent): void;
  /** Transfer one immutable frame lease to the socket hub. */
  frame(lease: BackgroundFrameLease): void;
  /** Avoid frame copies when there is no joined viewer. */
  hasFrameRecipients(): boolean;
  /** Bound connected external controllers independently of native input capacity. */
  maxControllers: number;
}

/** One bounded output consumer shared by generation and ordinary delivery barriers. */
export class BackgroundOutputPump {
  /** The server's incoming control pump must use this same sequence owner. */
  readonly admission: BackgroundCommandAdmission;
  /** Retained ordinary batch receipts. */
  private readonly ordinary: ControllerDeliveryRouter;
  /** Retained same-snake reconnect completion. */
  private readonly reclaim: ReclaimDeliveryRouter;
  /** Retained fresh join completion. */
  private readonly join: JoinDeliveryRouter;
  /** Disk/SQLite/generation delivery orchestration. */
  private readonly generation: BackgroundGenerationRouter;
  /** Two admitted Node send buffers, never a frame backlog. */
  private readonly frames: BackgroundFramePool;
  /** Last frame actually handed to the transport. */
  private frameSequence: RustBackgroundIdentity = '0000000000000000';
  /** Coalesce concurrent wakeups while asynchronous persistence is pending. */
  private active: Promise<boolean> | undefined;

  /** Attach every retained delivery adapter before starting native execution. */
  constructor(private readonly options: BackgroundOutputOptions) {
    const { owner, send, maxControllers } = options;
    this.admission = new BackgroundCommandAdmission(owner.runtime);
    this.ordinary = new ControllerDeliveryRouter(maxControllers, {
      send, nextSequence: () => this.admission.nextSequence(),
      trySubmitReceipt: (sequence, receipt) => this.admission.trySubmitReceipt(sequence, receipt)
    });
    this.reclaim = new ReclaimDeliveryRouter({
      send, nextSequence: () => this.admission.nextSequence(),
      trySubmitReceipt: (sequence, receipt) => this.admission.trySubmitReclaimReceipt(sequence, receipt)
    });
    this.join = new JoinDeliveryRouter({
      send, nextSequence: () => this.admission.nextSequence(),
      trySubmitReceipt: (sequence, receipt) => this.admission.trySubmitJoinReceipt(sequence, receipt)
    });
    this.generation = new BackgroundGenerationRouter({
      native: owner.runtime, admission: this.admission, persistence: owner.persistence,
      managedDirectory: owner.managedDirectory, maxAssignments: maxControllers, send,
      admitCheckpoint: () => owner.admitCheckpoint()
    });
    this.frames = new BackgroundFramePool(owner.runtime, owner.metadata.maximumFrameBytes);
  }

  /** Drain a bounded turn; callers reschedule when true and periodically retry input capacity. */
  drain(): Promise<boolean> {
    this.active ??= Promise.resolve().then(() => this.drainOneTurn()).finally(() => { this.active = undefined; });
    return this.active;
  }

  /** Complete prepared reliable output before attempting any replaceable frame. */
  private async drainOneTurn(): Promise<boolean> {
    const { runtime } = this.options.owner;
    for (let count = 0; count < 64; count++) {
      this.ordinary.flushReceipts();
      this.reclaim.flushReceipts();
      this.join.flushReceipts();
      this.generation.flush();
      const drained = runtime.drainOutputs(1, 1024 * 1024);
      for (const event of drained.events) {
        if (event.kind === 'fault') throw new Error(`${event.faultCode}: ${event.faultDetail}`);
        this.options.event(event);
        if (await this.generation.handle(event)) continue;
        if (event.controllerMessages && !this.ordinary.deliver(event.controllerMessages)) {
          throw new Error('overlapping ordinary controller output');
        }
        if (event.controllerReclaimAssignment && !this.reclaim.deliver(event.controllerReclaimAssignment)) {
          throw new Error('overlapping reclaim output');
        }
        if (event.controllerJoinAssignment && !this.join.deliver(event.controllerJoinAssignment)) {
          throw new Error('overlapping join output');
        }
      }
      if (!drained.moreWork) {
        if (this.options.hasFrameRecipients()) {
          const lease = this.frames.tryAcquireLatest(this.frameSequence);
          if (lease) {
            try { this.options.frame(lease); this.frameSequence = lease.display.sequence; }
            catch (error) { lease.release(); throw error; }
          }
        }
        return this.admission.blocked;
      }
    }
    return true;
  }
}
