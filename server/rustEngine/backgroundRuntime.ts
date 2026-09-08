import type {
  RustBackgroundDrain,
  RustBackgroundDisplay,
  RustBackgroundFrameCopy,
  RustBackgroundHealth,
  RustBackgroundControllerAction,
  RustBackgroundControllerDisconnect,
  RustGenerationAssignmentReceipt
} from '../../src/protocol/rustBackground.ts';
import type { ManagedCheckpointDescriptor, U64Hex } from './checkpointPersistenceProtocol.ts';
import type { RustRunStartCheckpointPublishOptions } from './runStartPersistenceHandoff.ts';

/** Coarse production-addon handle created by transferring the durable fresh run. */
export interface ExperimentalRunningAuthorityNativeHandle {
  /** Start only after attaching the Node output router. */
  start(): void;
  /** Queue steering for the next eligible step without altering a pending step. */
  submitControllerAction(sequence: U64Hex, action: RustBackgroundControllerAction): void;
  /** Queue a close without invalidating an already prepared step. */
  submitControllerDisconnect(sequence: U64Hex, close: RustBackgroundControllerDisconnect): void;
  /** Publish or exactly retry the retained generation's immutable managed file. */
  submitGenerationCheckpoint(sequence: U64Hex, options: RustRunStartCheckpointPublishOptions): void;
  /** Return the complete descriptor committed by the dedicated SQLite worker. */
  submitGenerationPersistenceAcknowledgement(sequence: U64Hex, descriptor: ManagedCheckpointDescriptor): void;
  /** Prepare connected-controller assignments after durability. */
  submitPrepareGenerationReassignments(sequence: U64Hex): void;
  /** Return the exact local transport result for a Rust-issued assignment. */
  submitGenerationAssignmentReceipt(sequence: U64Hex, receipt: RustGenerationAssignmentReceipt): void;
  /** Return an ordinary observation/replacement send result to its retained step. */
  submitControllerDeliveryReceipt(sequence: U64Hex, receipt: RustGenerationAssignmentReceipt): void;
  /** Commit the successor only after both retained barriers resolve. */
  submitPublishGenerationStart(sequence: U64Hex): void;
  /** Drain prepared output without inspecting or reconstructing the world. */
  drainOutputs(maxEvents: number, maxOwnedBytes: number): RustBackgroundDrain;
  /** Read only bounded atomic health scalars. */
  health(): RustBackgroundHealth;
  /** Read cached metadata without serializing or waiting on the live world. */
  latestDisplay(): RustBackgroundDisplay | null;
  /** Copy a newer complete frame into a non-shared caller-owned Uint8Array. */
  copyLatestFrame(destination: Uint8Array, afterSequence: U64Hex): RustBackgroundFrameCopy;
  /** Signal shutdown without waiting for authoritative work. */
  requestStop(): void;
  /** Join on a native worker, leaving the Node event loop responsive. */
  join(): Promise<void>;
}

/** Required coarse operations on the source-identified native runtime. */
const REQUIRED_METHODS: readonly (keyof ExperimentalRunningAuthorityNativeHandle)[] = [
  'start', 'submitControllerAction', 'submitControllerDisconnect', 'submitGenerationCheckpoint', 'submitGenerationPersistenceAcknowledgement',
  'submitPrepareGenerationReassignments', 'submitGenerationAssignmentReceipt',
  'submitControllerDeliveryReceipt',
  'submitPublishGenerationStart', 'drainOutputs', 'health', 'latestDisplay',
  'copyLatestFrame', 'requestStop', 'join'
];

/** Validate the handoff result before a Node router can use it. */
export function validateBackgroundRuntime(value: unknown): ExperimentalRunningAuthorityNativeHandle {
  if (value === null || typeof value !== 'object') {
    throw new TypeError('background authority transfer returned no native handle');
  }
  const handle = value as Partial<ExperimentalRunningAuthorityNativeHandle>;
  for (const method of REQUIRED_METHODS) {
    if (typeof handle[method] !== 'function') {
      throw new TypeError(`background authority handle is missing ${method}`);
    }
  }
  return handle as ExperimentalRunningAuthorityNativeHandle;
}
