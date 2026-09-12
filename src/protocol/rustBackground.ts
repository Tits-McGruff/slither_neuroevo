/** Exact fixed-width unsigned identity emitted by the Rust background bridge. */
export type RustBackgroundIdentity = string;

/** Immutable Rust-owned facts used to construct the initial server welcome. */
export interface RustStartupMetadata {
  /** Exact admitted lineage. */
  runId: string;
  /** Normalized Uint32 seed. */
  seed: number;
  /** Exact accepted configuration revision. */
  configRevision: RustBackgroundIdentity;
  /** Native normalized configuration identity. */
  configHash: string;
  /** Fixed simulation delta, independent of display cadence. */
  fixedStepSeconds: number;
  /** Admitted maximum frame allocation for the send pool. */
  maximumFrameBytes: number;
  /** Native canonical graph identity. */
  graphKey: string;
  /** Parameters in each evolved genome. */
  parameterCount: number;
  /** Actual authoritative inference implementation. */
  mathBackend: string;
  /** Browser binary-frame version. */
  serializerVersion: number;
  /** External observation version. */
  sensorVersion: number;
  /** Complete scalar settings from the admitted authority, without game arrays. */
  settings: Array<{
    /** Canonical setting path. */
    path: string;
    /** Native normalized value. */
    value: boolean | number | string;
  }>;
}

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
  /** Successful authoritative step computations represented by timing data. */
  stepTimingSamples: RustBackgroundIdentity;
  /** Saturating sum of sampled step computation in microseconds. */
  stepTimingTotalMicros: RustBackgroundIdentity;
  /** Largest sampled step computation in microseconds. */
  stepTimingMaxMicros: RustBackgroundIdentity;
  /** Conservative histogram ceiling containing the 95th percentile. */
  stepTimingP95Micros: RustBackgroundIdentity;
  /** Conservative histogram ceiling containing the 99th percentile. */
  stepTimingP99Micros: RustBackgroundIdentity;
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

/** Socket close routed with exact assignment identity and a Rust-owned receipt clock. */
export interface RustBackgroundControllerDisconnect {
  /** Rust assignment epoch. */
  leaseId: RustBackgroundIdentity;
  /** Live transport epoch being closed. */
  connectionId: RustBackgroundIdentity;
}

/** Fresh controller request after the reconnect path found no reservation. */
export interface RustBackgroundJoinRequest extends Omit<RustBackgroundReclaimRequest, 'resumeToken' | 'identityKey'> {
  /** Bounded run-scoped legacy identity retained for later reconnect. */
  identityKey: string;
}

/** Token reconnect to the same live Rust-owned snake. */
export interface RustBackgroundReclaimRequest {
  /** New live socket identity. */
  connectionId: RustBackgroundIdentity;
  /** Kind must match the retained lease. */
  controllerKind: 'player' | 'reinforcementLearning';
  /** Previous server-issued ownership token. */
  resumeToken?: string;
  /** Legacy fallback used only when no explicit token was supplied. */
  identityKey?: string;
}

/** Retained same-snake assignment prepared before any ownership mutation. */
export interface RustBackgroundReclaimAssignment extends RustBackgroundReclaimRequest {
  /** Newly staged token, required on every successful assignment. */
  resumeToken: string;
  /** Exact command that created this assignment. */
  requestSequence: RustBackgroundIdentity;
  /** Existing controller lease identity. */
  leaseId: RustBackgroundIdentity;
  /** Existing exact frame-v1 identity. */
  snakeId: number;
  /** Unchanged source step while delivery is pending. */
  completedStep: RustBackgroundIdentity;
}

/** Exact local-send result, distinct from ordinary and generation receipts. */
export interface RustBackgroundReclaimReceipt {
  /** Command that prepared the retained reclaim. */
  requestSequence: RustBackgroundIdentity;
  /** Exact destination socket epoch. */
  connectionId: RustBackgroundIdentity;
  /** Existing lease being reclaimed. */
  leaseId: RustBackgroundIdentity;
  /** Both reliable Protocol 2 messages were accepted locally. */
  accepted: boolean;
}

/** Exact local-send result for a Rust-issued generation assignment. */
export interface RustBackgroundGenerationAssignment extends Omit<RustGenerationAssignmentReceipt, 'accepted'> {
  /** Rust controller category. */
  controllerKind: 'player' | 'reinforcementLearning';
  /** Internal snake identity, distinct from its public frame identity. */
  snakeId: RustBackgroundIdentity;
  /** Exact public frame-v1 integer encoded without narrowing. */
  frameV1Id: RustBackgroundIdentity;
  /** Newly issued ownership token. */
  resumeToken: string;
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
  /** Retained fresh assignment awaiting exact local-send completion. */
  controllerJoinAssignment?: RustBackgroundReclaimAssignment;
  /** Fresh receipts cannot resolve another controller barrier. */
  controllerJoinResolution?: {
    /** Original fresh-join command. */
    requestSequence: RustBackgroundIdentity;
    /** Receipt matched the pending fresh assignment. */
    matched: boolean;
    /** Matching delivery succeeded and the snake became current. */
    accepted: boolean;
  };
  /** Same-snake reconnect awaiting exact local delivery. */
  controllerReclaimAssignment?: RustBackgroundReclaimAssignment;
  /** Receipt correlation outcome; unmatched receipts change nothing. */
  controllerReclaimResolution?: {
    /** Original reconnect command. */
    requestSequence: RustBackgroundIdentity;
    /** Receipt matched the retained assignment. */
    matched: boolean;
    /** Matched delivery succeeded and ownership committed. */
    accepted: boolean;
  };
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
  /** Scalar proof that a durably committed import became authoritative. */
  importPublication?: unknown;
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
  /** Config revision installed by one atomic live-settings command. */
  settingsConfigRevision?: RustBackgroundIdentity;
  /** Canonical native config hash installed with the revision. */
  settingsConfigHash?: string;
  /** First step that observes the newly active settings. */
  settingsEffectiveStep?: RustBackgroundIdentity;
  /** Successful in-bounds God Mode translation. */
  godModeMove?: {
    /** Exact browser/frame snake identity. */
    snakeId: number;
    /** Clamped authoritative head X. */
    x: number;
    /** Clamped authoritative head Y. */
    y: number;
    /** First fixed step that observes this translation. */
    effectiveStep: RustBackgroundIdentity;
  };
  /** Exact close result; false means stale or already disconnected. */
  controllerDisconnect?: {
    /** Requested assignment epoch. */
    leaseId: RustBackgroundIdentity;
    /** Source boundary at close application. */
    completedStep: RustBackgroundIdentity;
    /** Whether this result changed the connected lease. */
    applied: boolean;
  };
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

/** Compact recovery notice shared by health and Protocol 2 welcome messages. */
export interface RustRecoveryNotice {
  /** Failed effective lineage, preserved with its original suffix. */
  failedRunId: string;
  /** New durable active lineage. */
  branchRunId: string;
  /** Original failed current pointer. */
  failedCheckpointId: string;
  /** Immutable root selected for recovery. */
  recoveredCheckpointId: string;
  /** Pre-spawn generation resumed, as exact lowercase u64 hex. */
  recoveredGeneration: string;
  /** Completed generations lost relative to the newest retained boundary. */
  lostCompletedGenerations: { from: string; through: string } | null;
}

/** Compact provenance for an owner-selected older-checkpoint import branch. */
export interface RustImportBranchNotice {
  /** Original archive lineage whose later local history remains preserved. */
  sourceRunId: string;
  /** Fresh active lineage continuing from the imported boundary. */
  branchRunId: string;
  /** Exact imported checkpoint generation. */
  sourceGeneration: RustBackgroundIdentity;
  /** Exact imported checkpoint root. */
  sourceCheckpointId: string;
}
