/** Fitness summary for a single generation. */
export interface FitnessData {
  gen: number;
  avgFitness: number;
  maxFitness: number;
  minFitness: number;
}

/** Historical fitness metrics used by charts and UI. */
export interface FitnessHistoryEntry {
  gen: number;
  best: number;
  avg: number;
  min: number;
  speciesCount?: number;
  topSpeciesSize?: number;
  avgWeight?: number;
  weightVariance?: number;
}

/** Brain visualizer layer payload. */
export interface VizLayer {
  count: number;
  activations: ArrayLike<number> | null;
  isRecurrent?: boolean;
}

/** Brain visualizer payload for the UI. */
export interface VizData {
  /** Runtime family that produced the layer visualization. */
  kind: string;
  /** Ordered layer activation snapshots. */
  layers: VizLayer[];
  /** Population slot whose brain produced the snapshot, when pooled. */
  populationSlot?: number;
  /** Last committed authoritative step associated with the snapshot. */
  simulationStep?: number;
  /** Reserved legacy Protocol 2 visualization pool epoch. */
  poolEpoch?: number;
  /** Reserved legacy Protocol 2 visualization weight epoch. */
  weightEpoch?: number;
}

/** Hall of Fame entry for resurrecting elite snakes. */
export interface HallOfFameEntry {
  gen: number;
  seed: number;
  fitness: number;
  points: number;
  length: number;
  /** Opaque Rust run-scoped selector used without transferring genome weights. */
  entryId?: string;
  /** Whether this historical winner is explicitly retained by the owner. */
  pinned?: boolean;
}

/** Stats emitted alongside frame buffers. */
export interface FrameStats {
  gen: number;
  generationTime: number;
  generationSeconds: number;
  alive: number;
  aliveTotal: number;
  baselineBotsAlive: number;
  baselineBotsTotal: number;
  fps: number;
  fitnessData?: FitnessData;
  fitnessHistory?: FitnessHistoryEntry[];
  viz?: VizData;
  hofEntry?: HallOfFameEntry;
}
