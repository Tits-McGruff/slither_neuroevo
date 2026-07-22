import type { GraphSpec } from '../src/brains/graph/schema.ts';
import type {
  GenomeJSON,
  HallOfFameEntry,
  PopulationExport
} from '../src/protocol/messages.ts';
import type { CoreSettings, SettingsUpdate } from '../src/protocol/settings.ts';
import type { WorldAllocatorState, WorldRngState } from '../src/world.ts';

/** Current bounded-memory SQLite snapshot format. */
export const SNAPSHOT_FORMAT_VERSION = 2 as const;
/** Current exact pre-spawn checkpoint boundary schema. */
export const SNAPSHOT_BOUNDARY_VERSION = 1 as const;

/** Boundary classes stored by the current snapshot format. */
export type SnapshotBoundaryKind = 'run-start' | 'generation' | 'population-export';

/** Fitness-history entry retained across generation-boundary resume. */
export interface SnapshotFitnessHistoryEntry {
  /** Generation represented by this summary. */
  gen: number;
  /** Best fitness in the generation. */
  best: number;
  /** Average fitness in the generation. */
  avg: number;
  /** Minimum fitness in the generation. */
  min: number;
  /** Count of detected species. */
  speciesCount: number;
  /** Size of the largest detected species. */
  topSpeciesSize: number;
  /** Average network weight. */
  avgWeight: number;
  /** Network-weight variance. */
  weightVariance: number;
}

/** One typed genome consumed or returned without population-wide JSON conversion. */
export interface TypedGenomeSnapshot {
  /** Dense population slot for this checkpoint. */
  slot: number;
  /** Stable architecture content key. */
  archKey: string;
  /** Runtime brain family metadata. */
  brainType: string;
  /** Fitness retained with the genome. */
  fitness: number;
  /** Float32 parameter buffer. */
  weights: Float32Array;
}

/** Metadata stored once for a current-format snapshot. */
export interface PopulationCheckpointMetadata {
  /** Current snapshot format discriminator. */
  formatVersion: typeof SNAPSHOT_FORMAT_VERSION;
  /** Current boundary schema discriminator. */
  boundaryVersion: typeof SNAPSHOT_BOUNDARY_VERSION;
  /** Exact boundary or non-resumable population-export marker. */
  boundaryKind: SnapshotBoundaryKind;
  /** Whether normal/explicit startup may select this row. */
  resumable: boolean;
  /** Generation whose population is stored. */
  generation: number;
  /** Authoritative fixed step committed at the boundary. */
  simulationStep: number;
  /** Evolutionary lineage identifier. */
  runId: string;
  /** Normalized active lineage seed. */
  worldSeed: number;
  /** Canonical experiment-configuration content hash. */
  configHash: string;
  /** Monotonic runtime configuration revision. */
  configRevision: number;
  /** Stable architecture key shared by every current population genome. */
  archKey: string;
  /** Graph definition required to reconstruct the architecture. */
  graphSpec: GraphSpec;
  /** Number of dense population slots expected in child rows. */
  populationCount: number;
  /** Authoritative core settings. */
  settings: CoreSettings;
  /** Authoritative CFG path/value snapshot. */
  updates: SettingsUpdate[];
  /** Exact authoritative random-stream continuation. */
  rng: WorldRngState;
  /** Exact deterministic generated-id continuation. */
  allocators: WorldAllocatorState;
  /** Best fitness observed before this boundary. */
  bestFitnessEver: number;
  /** Bounded chart/evolution history retained across resume. */
  fitnessHistory: SnapshotFitnessHistoryEntry[];
  /** Pending Hall-of-Fame entry created by the preceding generation, if any. */
  lastHofEntry: HallOfFameEntry | null;
}

/** Current-format checkpoint input with a lazy typed genome source. */
export interface PopulationCheckpoint {
  /** One-row metadata payload. */
  metadata: PopulationCheckpointMetadata;
  /** Dense, slot-ordered genomes read one at a time by persistence. */
  genomes: Iterable<TypedGenomeSnapshot>;
}

/** Strictly loaded current-format checkpoint. */
export interface LoadedPopulationCheckpoint {
  /** SQLite snapshot row id. */
  id: number;
  /** Snapshot creation time in milliseconds since epoch. */
  createdAt: number;
  /** Current bounded format marker. */
  compatibility: 'current';
  /** Validated current-format metadata. */
  metadata: PopulationCheckpointMetadata;
  /** Dense typed genomes ordered by population slot. */
  genomes: TypedGenomeSnapshot[];
}

/** Read-only compatibility representation for one legacy blob snapshot. */
export interface LoadedLegacyCheckpoint {
  /** SQLite snapshot row id. */
  id: number;
  /** Snapshot creation time in milliseconds since epoch. */
  createdAt: number;
  /** Legacy compatibility marker. */
  compatibility: 'legacy';
  /** Legacy JSON transport payload. */
  payload: PopulationSnapshotPayload;
  /** Typed genomes decoded from the legacy combined blob. */
  genomes: TypedGenomeSnapshot[];
}

/** Snapshot selected for startup resume. */
export type LoadedResumeSnapshot = LoadedPopulationCheckpoint | LoadedLegacyCheckpoint;

/** JSON-compatible population-transfer shape retained for import/export. */
export interface PopulationSnapshotPayload extends PopulationExport {
  /** Canonical configuration hash recorded with the payload. */
  cfgHash: string;
  /** Active lineage seed recorded with the payload. */
  worldSeed: number;
  /** Optional core settings retained by legacy and current exports. */
  settings?: CoreSettings;
  /** Optional CFG updates retained by legacy and current exports. */
  updates?: SettingsUpdate[];
  /** Current format version when exported from child rows. */
  formatVersion?: number;
  /** Lineage id when available. */
  runId?: string;
  /** Runtime config revision when available. */
  configRevision?: number;
  /** Graph definition when available. */
  graphSpec?: GraphSpec;
  /** Boundary metadata when available. */
  boundary?: {
    /** Boundary schema version. */
    version: number;
    /** Stored boundary kind. */
    kind: SnapshotBoundaryKind | 'legacy';
    /** Committed authoritative simulation step. */
    simulationStep: number;
    /** Whether startup may resume this snapshot exactly. */
    resumable: boolean;
  };
}

/** Snapshot metadata returned by list endpoints and diagnostics. */
export interface SnapshotMeta {
  /** SQLite row id. */
  id: number;
  /** Snapshot creation time in milliseconds since epoch. */
  createdAt: number;
  /** Stored generation number. */
  gen: number;
  /** Stored format version, with zero representing legacy. */
  formatVersion: number;
  /** Boundary kind when known. */
  boundaryKind: SnapshotBoundaryKind | 'legacy';
  /** Whether the row is a startup resume candidate. */
  resumable: boolean;
}

/** Convert one typed genome to the established JSON transport shape. */
export function typedGenomeToJson(genome: TypedGenomeSnapshot): GenomeJSON {
  return {
    archKey: genome.archKey,
    brainType: genome.brainType,
    weights: Array.from(genome.weights),
    fitness: genome.fitness
  };
}
