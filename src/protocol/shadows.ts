/** Cold activation over one existing exact teacher stream; Rust owns inference and recurrence. */
export interface ShadowActivationRequest {
  /** Run observed when the command was requested. */
  expectedRunId: string;
  /** Exact hexadecimal configuration revision. */
  expectedConfigRevision: string;
  /** Exact active diagnostic epoch. */
  captureEpoch: string;
  /** Independent diagnostic slot from zero through three. */
  shadowSlot: number;
  /** Selected baseline teacher slot from zero through 119. */
  baselineSlot: number;
  /** Immutable complete candidate digest, previously validated by Rust. */
  artifactSha256: string;
}

/** A delayed stop cannot disable a later replacement in the same diagnostic slot. */
export interface ShadowStopRequest {
  /** Independent diagnostic slot from zero through three. */
  shadowSlot: number;
  /** Capture identity recorded by the activation receipt. */
  captureEpoch: string;
  /** Exact hexadecimal command sequence recorded by the activation receipt. */
  activationSequence: string;
}

/** Unweighted action-error statistics computed only by Rust over committed teacher rows. */
export interface ShadowMetricTotals {
  /** Exact hexadecimal delivered-row count. */
  samples: string;
  /** Sum of absolute applied-turn differences. */
  turnAbsoluteError: number;
  /** Sum of squared applied-turn differences. */
  turnSquaredError: number;
  /** Exact hexadecimal requested-boost disagreement count. */
  boostDisagreements: string;
  /** Mean absolute applied-turn error, zero before any sample. */
  turnMae: number;
  /** Mean squared applied-turn error, zero before any sample. */
  turnMse: number;
  /** Requested-boost disagreement fraction, zero before any sample. */
  boostDisagreementRate: number;
}

/** Fixed Rust diagnostic segment; no weights, observations or world state are copied. */
export interface ShadowSnapshot {
  /** Independent diagnostic slot from zero through three. */
  shadowSlot: number;
  /** Complete immutable candidate digest. */
  artifactSha256: string;
  /** Exact admitted teacher capture identity. */
  captureEpoch: string;
  /** Baseline teacher ownership, distinct from physical snake identity. */
  baselineSlot: number;
  /** Exact command sequence that started this segment. */
  activationSequence: string;
  /** Completed boundary at explicit activation. */
  activationStep: string;
  /** Latest complete publication observed, including held-control steps. */
  completedStep: string;
  /** Actual delivery step of rawOutputs, zero before the first delivery. */
  lastSampleStep: string;
  /** Generation of the latest delivered sample. */
  generation: string;
  /** Full physical identity of the latest delivered teacher sample. */
  physicalSnakeId: string;
  /** Whether the slot currently evaluates delivered rows. */
  enabled: boolean;
  /** Bounded isolated diagnostic error or null. */
  error: string | null;
  /** Last successful raw turn/boost outputs. */
  rawOutputs: number[];
  /** All delivered rows in this activation segment. */
  totals: ShadowMetricTotals;
  /** Roam, Seek, Avoid and Boost, in that order. */
  strategies: ShadowMetricTotals[];
  /** Small, Medium and Large, in that order. */
  lifeStages: ShadowMetricTotals[];
  /** Hazard, pursuit/escape, boost, transition, saturation and respawn. */
  strata: ShadowMetricTotals[];
  /** Rows belonging to no rare stratum. */
  cruising: ShadowMetricTotals;
  /** Latest at-most-32 deliveries per sealed physical death. */
  preDeath: ShadowMetricTotals;
  /** Exact hexadecimal deduplicated physical death count. */
  deaths: string;
}

/** Bounded on-demand native publication with four fixed diagnostic slots. */
export interface ShadowMetrics {
  /** Supported fixed-summary schema. */
  schemaVersion: 1;
  /** Exactly four entries; unused slots are null. */
  slots: (ShadowSnapshot | null)[];
}

/** Durable explicit command result; sequences are unique within the server process epoch. */
export interface ShadowReceipt {
  /** Receipt schema independent of gameplay Protocol 2. */
  schemaVersion: 1;
  /** Server diagnostic process identity, independent of authoritative RNG. */
  processEpoch: string;
  /** Client idempotency key, distinct from native command sequence. */
  requestId: string;
  /** Admitted command producing this receipt. */
  commandSequence: string;
  /** Activation targeted by the command; equal to commandSequence for activation. */
  activationSequence: string;
  /** Slot activation observed by the caller before activation, or the target for a stop. */
  previousActivationSequence: string;
  /** Admitted run identity. */
  runId: string;
  /** Exact configuration revision captured on admission. */
  configRevision: string;
  /** Exact teacher capture identity. */
  captureEpoch: string;
  /** Immutable complete candidate digest. */
  artifactSha256: string;
  /** Independent diagnostic slot. */
  shadowSlot: number;
  /** Selected baseline teacher slot. */
  baselineSlot: number;
  /** Complete boundary at native application. */
  effectiveStep: string;
  /** Requested diagnostic state confirmed by Rust. */
  enabled: boolean;
  /** Server receipt wall time, never used for game scheduling. */
  acknowledgedAt: number;
}

/** One retained native confirmation and its exact durable chronology ordinal. */
export interface ShadowEvent {
  /** Exact hexadecimal SQLite chronology, independent of command/source identity. */
  ordinal: string;
  /** Complete immutable confirmed activation or stop. */
  receipt: ShadowReceipt;
}

/** Bounded durable command chronology; retention does not alter active native shadows. */
export interface ShadowEventPage {
  /** At most the requested page size. */
  events: ShadowEvent[];
  /** Exact hexadecimal retained chronology cursor. */
  nextCursor: string;
  /** Another retained event follows this page. */
  hasMore: boolean;
}

/** Dedicated HTTP publication with the process identity required by explicit shadow commands. */
export interface ShadowPublication extends ShadowMetrics {
  /** Diagnostic process identity; independent of authoritative RNG and run identity. */
  processEpoch: string;
}
