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
