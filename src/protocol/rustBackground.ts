/** Exact fixed-width unsigned identity emitted by the Rust background bridge. */
export type RustBackgroundIdentity = string;

/** Cached stats from the same committed boundary as the Rust-packed frame. */
export interface RustBackgroundDisplay {
  /** Monotonic display publication, independent of command sequences. */
  sequence: RustBackgroundIdentity;
  /** Published authority incarnation. */
  worldEpoch: RustBackgroundIdentity;
  /** Completed fixed-step chronology. */
  completedStep: RustBackgroundIdentity;
  /** Published generation. */
  generation: RustBackgroundIdentity;
  /** Elapsed simulation seconds within this generation. */
  generationTime: number;
  /** Alive evolving population members. */
  alivePopulation: number;
  /** Alive built-in baseline bots. */
  baselineBotsAlive: number;
  /** Configured built-in baseline slots. */
  baselineBotsTotal: number;
  /** All snake records, including dead snakes omitted from the frame. */
  totalSnakes: number;
  /** Alive snakes packed into the frame. */
  aliveSnakes: number;
  /** Packed pellet count. */
  pellets: number;
  /** Cached byte length; welcome refresh never serializes the world. */
  frameByteLength: number;
}

/** Every unsuccessful copy leaves both destination and latest cached frame intact. */
export type RustBackgroundFrameCopy =
  | { status: 'busy' | 'unchanged'; display?: never }
  | { status: 'tooSmall' | 'copied'; display: RustBackgroundDisplay };

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

/** Latest steering routed to Rust; receipt time is stamped inside the addon. */
export interface RustBackgroundControllerAction {
  /** Rust assignment epoch. */
  leaseId: RustBackgroundIdentity;
  /** Live socket epoch. */
  connectionId: RustBackgroundIdentity;
  /** Finite normalized steering in [-1, 1]. */
  turn: number;
  /** Latest boost request. */
  boost: boolean;
  /** Exact diagnostic client tick, not an authority clock. */
  clientTick: RustBackgroundIdentity;
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

/** Ordinary reliable observation or death-replacement assignment prepared by Rust. */
export interface RustBackgroundControllerMessage {
  /** Exact retained operation epoch. */
  operationEpoch: RustBackgroundIdentity;
  /** Exact retained event sequence. */
  eventSequence: RustBackgroundIdentity;
  /** Live socket epoch. */
  connectionId: RustBackgroundIdentity;
  /** Controller assignment epoch. */
  leaseId: RustBackgroundIdentity;
  /** Player browser or Protocol 2 bot. */
  controllerKind: 'player' | 'reinforcementLearning';
  /** Internal authority identity, never substituted for the wire snake ID. */
  internalSnakeId: RustBackgroundIdentity;
  /** Exact frame-v1/Protocol 2 snake identity. */
  snakeId: number;
  /** Completed-step boundary at which the observation was sampled. */
  sourceCompletedStep: RustBackgroundIdentity;
  /** Reliable event discriminator. */
  kind: 'observation' | 'replacementAssignment';
  /** Rust-delivered sensor-v3 vector for observations. */
  sensors?: number[];
  /** Pre-movement pose, present on observations. */
  x?: number;
  /** Pre-movement pose, present on observations. */
  y?: number;
  /** Pre-movement heading, present on observations. */
  direction?: number;
  /** Rust-issued reclaim token on a replacement assignment. */
  resumeToken?: string;
}

/** Ordinary-step publication after applying exact local transport results. */
export interface RustControllerReceiptResolution {
  /** Newly accepted messages. */
  matchedAcceptances: RustBackgroundIdentity;
  /** Newly failed local sends. */
  matchedFailures: RustBackgroundIdentity;
  /** Stale, duplicate, or mismatched receipts. */
  ignoredReceipts: RustBackgroundIdentity;
  /** Unresolved messages retained by Rust. */
  remaining: RustBackgroundIdentity;
  /** Newly published step, absent while any delivery remains pending. */
  publishedCompletedStep?: RustBackgroundIdentity;
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
  /** Replaceable basic stats and cached frame metadata, after priority events. */
  display?: RustBackgroundDisplay;
  /** Full reliable ordinary-step batch, delivered before its step is published. */
  controllerMessages?: RustBackgroundControllerMessage[];
  /** Exact ordinary receipt result, separate from generation assignments. */
  controllerReceiptResolution?: RustControllerReceiptResolution;
  /** Lease whose action was applied at a fresh pre-step boundary. */
  controllerActionLeaseId?: RustBackgroundIdentity;
  /** Completed-step boundary before the accepted action can affect physics. */
  controllerActionCompletedStep?: RustBackgroundIdentity;
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
  /** Prepared events in queue priority order, ending with replaceable display stats. */
  events: RustBackgroundEvent[];
  /** Whether Node should schedule another bounded drain before sleeping. */
  moreWork: boolean;
  /** Wake generation used by the native rearm protocol. */
  generation: RustBackgroundIdentity;
}
