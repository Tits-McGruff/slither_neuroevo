import type {
  RustBackgroundEvent,
  RustBackgroundIdentity,
  RustBackgroundVisualization
} from '../../src/protocol/rustBackground.ts';
import type { AssignMsg, ReclaimResultMsg, SensorsMsg } from '../protocol.ts';
import { BackgroundFramePool, type BackgroundFrameLease } from './backgroundFrames.ts';
import { BackgroundGenerationRouter } from './backgroundGeneration.ts';
import { BackgroundCommandAdmission } from './commandAdmission.ts';
import { ControllerDeliveryRouter, JoinDeliveryRouter, ReclaimDeliveryRouter } from './controllerDelivery.ts';
import type { ExperimentalServerRuntime } from './experimentalStartup.ts';
import type { RustNativeVisualization } from './backgroundRuntime.ts';
import type { ManagedCheckpointDescriptor, U64Hex } from './checkpointPersistenceProtocol.ts';

/** Scalar result of the one complete imported-authority swap. */
export interface RustImportPublication {
  /** New process-local world identity. */
  worldEpoch: U64Hex;
  /** Imported generation boundary now running. */
  generation: U64Hex;
  /** Imported completed-step chronology. */
  completedStep: U64Hex;
  /** Imported population identity. */
  populationEpoch: U64Hex;
}

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
  /** Whether any UI currently requests focused neural data. */
  wantsVisualization?(): boolean;
  /** Accept one newer complete focused neural snapshot. */
  visualization?(snapshot: RustBackgroundVisualization): void;
  /** Bound connected external controllers independently of native input capacity. */
  maxControllers: number;
  /** Observe one complete generation durability barrier. */
  observeCheckpointBarrier?(durationMs: number): void;
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
  /** Last focused snapshot copied from the replaceable Rust cache. */
  private visualizationSequence: RustBackgroundIdentity = '0000000000000000';
  /** Coalesce concurrent wakeups while asynchronous persistence is pending. */
  private active: Promise<boolean> | undefined;
  /** One import lifecycle command awaiting queue admission or its exact reply. */
  private importCommand: {
    kind: 'stage' | 'publish' | 'cancel';
    submit(sequence: RustBackgroundIdentity): void;
    expectedSequence?: RustBackgroundIdentity;
    resolve(value: RustImportPublication | undefined): void;
    reject(error: Error): void;
  } | undefined;

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
      admitCheckpoint: () => owner.admitCheckpoint(),
      observeBarrier: durationMs => options.observeCheckpointBarrier?.(durationMs)
    });
    this.frames = new BackgroundFramePool(owner.runtime, owner.metadata.maximumFrameBytes);
  }

  /** Drain a bounded turn; callers reschedule when true and periodically retry input capacity. */
  drain(): Promise<boolean> {
    this.active ??= Promise.resolve().then(() => this.drainOneTurn()).catch(error => {
      const command = this.importCommand;
      if (command) {
        this.importCommand = undefined;
        command.reject(error instanceof Error ? error : new Error(String(error)));
      }
      throw error;
    }).finally(() => { this.active = undefined; });
    return this.active;
  }

  /** Pause stepping after Rust has prepared a complete private import. */
  stagePreparedImport(): Promise<void> {
    if (this.generation.active) return Promise.reject(new Error('generation persistence is busy'));
    return this.issueImportCommand('stage', sequence =>
      this.options.owner.runtime.submitStagePreparedImport(sequence)
    ).then(() => undefined);
  }

  /** Swap only the exact descriptor returned by the committed SQLite import. */
  publishPreparedImport(
    descriptor: ManagedCheckpointDescriptor,
    branchRunId?: string
  ): Promise<RustImportPublication> {
    return this.issueImportCommand('publish', sequence =>
      this.options.owner.runtime.submitImportPersistenceAcknowledgement(sequence, descriptor, branchRunId)
    ).then(value => {
      if (!value) throw new Error('import publication omitted its result');
      return value;
    });
  }

  /** Resume the old game after a failure before the import transaction commits. */
  cancelPreparedImport(): Promise<void> {
    return this.issueImportCommand('cancel', sequence =>
      this.options.owner.runtime.submitCancelPreparedImport(sequence)
    ).then(() => undefined);
  }

  /** Retain one import control until bounded queue capacity is available. */
  private issueImportCommand(
    kind: 'stage' | 'publish' | 'cancel',
    submit: (sequence: RustBackgroundIdentity) => void
  ): Promise<RustImportPublication | undefined> {
    if (this.importCommand) return Promise.reject(new Error('another import command is pending'));
    return new Promise((resolve, reject) => {
      this.importCommand = { kind, submit, resolve, reject };
      this.flushImportCommand();
    });
  }

  /** Retry only admission; the exact command is never regenerated after acceptance. */
  private flushImportCommand(): void {
    const command = this.importCommand;
    if (!command || command.expectedSequence) return;
    this.admission.trySubmitControl(sequence => {
      command.submit(sequence);
      command.expectedSequence = sequence;
    });
  }

  /** Resolve only the exact import reply; unrelated events remain normally routed. */
  private handleImportEvent(event: RustBackgroundEvent): boolean {
    const command = this.importCommand;
    if (!command?.expectedSequence || event.commandSequence !== command.expectedSequence) return false;
    const expectedKind = command.kind === 'stage' ? 'importStaged' :
      command.kind === 'publish' ? 'importPublished' : 'importCancelled';
    if (event.kind === 'commandRejected') {
      this.importCommand = undefined;
      command.reject(new Error(`import command rejected: ${event.rejectionCode}: ${event.rejectionDetail}`));
      return true;
    }
    if (event.kind !== expectedKind) return false;
    this.importCommand = undefined;
    if (command.kind === 'publish') {
      const value = event.importPublication as Partial<RustImportPublication> | undefined;
      if (!value || !/^[0-9a-f]{16}$/u.test(value.worldEpoch ?? '') ||
          !/^[0-9a-f]{16}$/u.test(value.generation ?? '') ||
          !/^[0-9a-f]{16}$/u.test(value.completedStep ?? '') ||
          !/^[0-9a-f]{16}$/u.test(value.populationEpoch ?? '')) {
        command.reject(new TypeError('invalid imported-authority publication'));
      } else {
        command.resolve(value as RustImportPublication);
      }
    } else {
      command.resolve(undefined);
    }
    return true;
  }

  /** Complete prepared reliable output before attempting any replaceable frame. */
  private async drainOneTurn(): Promise<boolean> {
    const { runtime } = this.options.owner;
    for (let count = 0; count < 64; count++) {
      this.ordinary.flushReceipts();
      this.reclaim.flushReceipts();
      this.join.flushReceipts();
      this.generation.flush();
      this.flushImportCommand();
      const drained = runtime.drainOutputs(1, 1024 * 1024);
      for (const event of drained.events) {
        if (event.kind === 'fault') throw new Error(`${event.faultCode}: ${event.faultDetail}`);
        this.options.event(event);
        if (this.handleImportEvent(event)) continue;
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
        if (this.options.wantsVisualization?.() === true) {
          const nativeVisualization = runtime.latestVisualization(this.visualizationSequence);
          if (nativeVisualization) {
            const visualization = normalizeVisualization(nativeVisualization);
            this.options.visualization?.(visualization);
            this.visualizationSequence = nativeVisualization.sequence;
          }
        }
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

/** Reject malformed native snapshots before they reach the shared browser protocol. */
function normalizeVisualization(value: RustNativeVisualization): RustBackgroundVisualization {
  if (!/^[0-9a-f]{16}$/u.test(value.sequence) ||
      !/^[0-9a-f]{16}$/u.test(value.worldEpoch) ||
      !/^[0-9a-f]{16}$/u.test(value.completedStep) ||
      value.kind !== 'graph' || !Number.isSafeInteger(value.snakeId) || value.snakeId <= 0 ||
      !Array.isArray(value.layers) || value.layers.length > 256) {
    throw new TypeError('invalid Rust visualization identity');
  }
  let values = 0;
  for (const layer of value.layers) {
    if (!Number.isSafeInteger(layer.count) || layer.count < 0 || layer.count > 1_000_000) {
      throw new TypeError('invalid Rust visualization layer width');
    }
    if (typeof layer.hasActivations !== 'boolean' || !Array.isArray(layer.activations) ||
        layer.activations.length !== (layer.hasActivations ? layer.count : 0)) {
      throw new TypeError('invalid Rust visualization activation shape');
    }
    if (layer.hasActivations) {
      for (const activation of layer.activations) {
        if (typeof activation !== 'number' || !Number.isFinite(activation)) {
          throw new TypeError('invalid Rust visualization activation');
        }
      }
      values += layer.activations.length;
      if (values > 1_000_000) throw new RangeError('Rust visualization exceeds its value limit');
    }
    if (layer.isRecurrent !== undefined && layer.isRecurrent !== true) {
      throw new TypeError('invalid Rust visualization recurrent marker');
    }
  }
  return {
    sequence: value.sequence,
    worldEpoch: value.worldEpoch,
    completedStep: value.completedStep,
    snakeId: value.snakeId,
    kind: value.kind,
    layers: value.layers.map(layer => ({
      count: layer.count,
      activations: layer.hasActivations ? layer.activations : null,
      ...(layer.isRecurrent ? { isRecurrent: true } : {})
    }))
  };
}
