/** Whole portable artifact admission; no neural bytes are parsed in TypeScript. */
export const MAX_BRAIN_ARTIFACT_BYTES = 64 * 1024 * 1024;

/** Rust-validated portable neural metadata; no observation or weight parsing in TypeScript. */
export interface BrainArtifactDescriptor {
  /** Portable descriptor schema. */
  schemaVersion: 1;
  /** Exact complete immutable artifact digest. */
  artifactSha256: string;
  /** Exact decoded canonical Float32 digest. */
  weightsSha256: string;
  /** Native compiler compatibility key. */
  architectureKey: string;
  /** Native compiled numeric-layout digest. */
  layoutSha256: string;
  /** Native structural sensor digest. */
  sensorSha256: string;
  /** Complete observation width. */
  inputSize: number;
  /** Canonical packed parameter count. */
  parameterCount: number;
  /** Packed recurrent state width. */
  stateSize: number;
  /** Actual requested deployment cadence. */
  controlIntervalSeconds: number;
  /** Physical-life recurrent handling. */
  lifeStateRule: 'reset' | 'preserve';
  /** Metadata provenance; runtime snake kind remains unchanged. */
  sourceKind: 'distilled' | 'imported' | 'manual';
}

/** A candidate is admitted, not automatically activated or saved into the library. */
export interface BrainCandidateRecord {
  /** Independently Rust-validated descriptor. */
  descriptor: BrainArtifactDescriptor;
  /** Complete portable file length. */
  byteCount: number;
}
