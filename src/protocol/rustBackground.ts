/** Exact fixed-width unsigned identity emitted by the Rust background bridge. */
export type RustBackgroundIdentity = string;

/** Small operational snapshot; no authoritative game arrays cross this boundary. */
export interface RustBackgroundHealth {
  /** Coordinator lifecycle, including orderly and faulted shutdown. */
  lifecycle: string;
  /** Scheduler or retained delivery/generation barrier state. */
  loopState: string;
  /** Current Rust authority incarnation. */
  worldEpoch: RustBackgroundIdentity;
  /** Published generation. */
  generation: RustBackgroundIdentity;
  /** Published completed-step chronology. */
  completedStep: RustBackgroundIdentity;
  /** Whether the retained generation has published its immutable file. */
  generationCheckpointPublished: boolean;
  /** Whether the worker's exact descriptor has been acknowledged. */
  generationPersistenceAcknowledged: boolean;
  /** Outstanding local controller sends. */
  pendingExternalDeliveries: RustBackgroundIdentity;
  /** Retired scheduler tickets. */
  schedulerCompletedSteps: RustBackgroundIdentity;
  /** Commands whose full replies have entered the output queue. */
  processedCommands: RustBackgroundIdentity;
  /** First terminal fault category. */
  faultCode?: string;
  /** First bounded terminal fault detail. */
  faultDetail?: string;
}

/** Exact local-send result for a Rust-issued generation assignment. */
export interface RustGenerationAssignmentReceipt {
  /** Rust operation epoch. */
  operationEpoch: RustBackgroundIdentity;
  /** Rust event sequence. */
  eventSequence: RustBackgroundIdentity;
  /** Live transport connection epoch. */
  connectionId: RustBackgroundIdentity;
  /** Rust controller lease epoch. */
  leaseId: RustBackgroundIdentity;
  /** Acceptance by the local transport; not remote acknowledgement. */
  accepted: boolean;
}

/** Coarse event envelope; consumers validate the payload for the selected kind. */
export interface RustBackgroundEvent {
  /** Stable Rust event discriminant. */
  kind: string;
  /** Inbound command correlation, absent on unsolicited lifecycle events. */
  commandSequence?: RustBackgroundIdentity;
  /** Rust checkpoint descriptor plus compact generation commit. */
  checkpoint?: unknown;
  /** Retained transition announcement. */
  transition?: unknown;
  /** Exact acknowledged checkpoint operation token. */
  acknowledgedOperationId?: string;
  /** Rust-prepared controller assignments. */
  reassignments?: unknown;
  /** Result of applying exact local delivery receipts. */
  receiptResolution?: unknown;
  /** Published successor and unavailable controller reservations. */
  generationStart?: unknown;
  /** Recoverable command rejection category. */
  rejectionCode?: string;
  /** Bounded rejection detail. */
  rejectionDetail?: string;
  /** Terminal fault category. */
  faultCode?: string;
  /** Bounded terminal fault detail. */
  faultDetail?: string;
}

/** One bounded native output drain and its coalesced wake metadata. */
export interface RustBackgroundDrain {
  /** Prepared reliable events in queue priority order. */
  events: RustBackgroundEvent[];
  /** Whether Node should schedule another bounded drain before sleeping. */
  moreWork: boolean;
  /** Wake generation used by the native rearm protocol. */
  generation: RustBackgroundIdentity;
}
