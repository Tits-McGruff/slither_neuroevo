/** Exact unsigned 64-bit diagnostic counter encoded as lowercase hexadecimal. */
export type GenerationCounterHex = string;
type U64Hex = GenerationCounterHex;

/** Exact counters; mean = total / samples, or unavailable when the record is null. */
export interface GenerationWorkDistribution {
  samples: U64Hex;
  total: U64Hex;
  maximum: U64Hex;
}

/** Operational measurements, excluded from checkpoint state and exact replay.
 * Phase wall times overlap their nested fields. Worker query sums are not batch wall time.
 * Workload counts sample committed end-of-step geometry; evolved length is pooled alive-snake samples.
 */
export interface GenerationWorkSummary {
  version: 1;
  generation: U64Hex;
  firstStep: U64Hex;
  lastStep: U64Hex;
  steps: U64Hex;
  partial: boolean;
  neuralDeliveries: U64Hex;
  baselineDeliveries: U64Hex;
  rayPreparations: U64Hex;
  timingsNs: {
    authorityBegin: U64Hex | null;
    prefix: U64Hex | null;
    controlTotal: U64Hex | null;
    controlCommit: U64Hex | null;
    worldTotal: U64Hex | null;
    publication: U64Hex | null;

    movement: U64Hex | null;
    food: U64Hex | null;
    collision: U64Hex | null;
    collisionIndex: U64Hex | null;
    collisionHeadHead: U64Hex | null;
    collisionHeadBody: U64Hex | null;
    physicsPelletIndex: U64Hex | null;
    effects: U64Hex | null;
    controlSpatialIndex: U64Hex | null;
    bodyRayPrepare: U64Hex | null;
    baselineSensing: U64Hex | null;
    baselinePolicy: U64Hex | null;
    neuralSensing: U64Hex | null;
    neuralInference: U64Hex | null;
    neuralBatchPreparation: U64Hex | null;
    controlBodyIndex: U64Hex | null;
    controlPelletIndex: U64Hex | null;
    sensorHeadPrepare: U64Hex | null;
    bodyQueryWorker: U64Hex | null;
    bodyRayQueryWorker: U64Hex | null;
  };
  workload: {
    aliveEvolved: GenerationWorkDistribution | null;
    aliveTotal: GenerationWorkDistribution | null;
    bodyPoints: GenerationWorkDistribution | null;
    bodySegments: GenerationWorkDistribution | null;
    evolvedBodyLength: GenerationWorkDistribution | null;
    pellets: GenerationWorkDistribution | null;
    bodyQueryCandidates: GenerationWorkDistribution | null;
    rayQueryCandidates: GenerationWorkDistribution | null;
  };
}
