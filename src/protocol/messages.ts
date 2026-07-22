/** Serialized genome representation for export/import. */
export interface GenomeJSON {
  archKey: string;
  brainType?: string;
  weights: number[];
  fitness?: number;
}

/** Population export payload. */
export interface PopulationExport {
  generation: number;
  archKey: string;
  genomes: GenomeJSON[];
}

/** Population import payload, optionally including HoF. */
export interface PopulationImportData {
  generation?: number;
  archKey?: string;
  genomes?: GenomeJSON[];
  hof?: HallOfFameEntry[];
}

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
  /** Worker-pool lifecycle epoch associated with the snapshot. */
  poolEpoch?: number;
  /** Population-weight epoch associated with the snapshot. */
  weightEpoch?: number;
}

/** Hall of Fame entry for resurrecting elite snakes. */
export interface HallOfFameEntry {
  gen: number;
  seed: number;
  fitness: number;
  points: number;
  length: number;
  genome: GenomeJSON;
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
