/** Fixed source ordering for ordinary native observation-quality diagnostics. */
export const OBSERVATION_CONSUMERS = ['neural', 'baseline', 'external'] as const;

/** Scalar counters retain exact u64 identity through the test-only coarse bridge. */
export const OBSERVATION_COUNT_KEYS = [
  'observations', 'v4Observations', 'rayIncompleteObservations', 'bodyCappedObservations',
  'pelletCappedObservations', 'querySaturatedObservations', 'rayChargedUnits',
] as const;

/** One actual source class, excluding inspector probes and reused shadow inputs. */
export interface ObservationConsumerCounts {
  /** Stable source class, in native canonical order. */
  consumerClass: typeof OBSERVATION_CONSUMERS[number];
  /** Constructed v3/v4 observations from ordinary published boundaries. */
  observations: string;
  /** The exact quality denominator; legacy observations do not enter it. */
  v4Observations: string;
  /** V4 observations with at least one incomplete capsule ray. */
  rayIncompleteObservations: string;
  /** V4 observations that hit a body detail cap. */
  bodyCappedObservations: string;
  /** V4 observations that hit a pellet detail cap. */
  pelletCappedObservations: string;
  /** Union of ray/body/pellet incomplete observations, counted once per vector. */
  querySaturatedObservations: string;
  /** Charged ray work from these exact v4 vectors. */
  rayChargedUnits: string;
}

/** One coherent cached prefix; it is independent of the separate health read. */
export interface ObservationQualitySnapshot {
  /** First test-diagnostic shape, without changing Protocol 2. */
  schemaVersion: 1;
  /** Run whose latest ordinary publication was counted; null before the first one. */
  runId: string | null;
  /** Ordinary published steps counted once; staged and terminal steps are excluded. */
  ordinaryPublishedSteps: string;
  /** Latest counted authoritative incarnation. */
  worldEpoch: string;
  /** Latest counted generation. */
  generation: string;
  /** Latest counted completed-step receipt. */
  completedStep: string;
  /** Neural, baseline and external counts in canonical order. */
  consumers: ObservationConsumerCounts[];
}
