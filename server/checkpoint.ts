import type { World, GenerationBoundaryState } from '../src/world.ts';
import { buildAuthoritativeConfigHash } from './configIdentity.ts';
import { buildCoreSettingsSnapshot, buildSettingsUpdatesSnapshot } from './settingsSnapshot.ts';
import {
  SNAPSHOT_BOUNDARY_VERSION,
  SNAPSHOT_FORMAT_VERSION,
  type PopulationCheckpoint,
  type PopulationCheckpointMetadata,
  type SnapshotBoundaryKind,
  type TypedGenomeSnapshot
} from './snapshotTypes.ts';

/** Runtime identity supplied by the server around a checkpoint operation. */
export interface CheckpointIdentity {
  /** Evolutionary lineage represented by the checkpoint. */
  runId: string;
  /** Monotonic authoritative configuration revision. */
  configRevision: number;
}

/** Inputs for a manual, non-resumable population export. */
export interface PopulationExportCheckpointOptions extends CheckpointIdentity {
  /** Last committed authoritative step represented by the current World. */
  simulationStep: number;
}

/**
 * Iterate the active population as typed durable slots without JSON conversion.
 * @param world - Authoritative World whose genomes remain owned by the caller.
 * @returns Lazy dense genome iterable.
 */
function* iterateTypedGenomes(world: World): Iterable<TypedGenomeSnapshot> {
  for (let slot = 0; slot < world.population.length; slot++) {
    const genome = world.population[slot];
    if (!genome) throw new Error(`population slot ${slot} is missing`);
    yield {
      slot,
      archKey: genome.archKey,
      brainType: genome.brainType,
      fitness: genome.fitness,
      weights: genome.weights
    };
  }
}

/**
 * Create a reusable lazy iterable so a failed transaction may be retried safely.
 * @param world - World whose current dense population is read on iteration.
 * @returns Iterable that creates a fresh one-genome-at-a-time iterator per pass.
 */
function createTypedGenomeIterable(world: World): Iterable<TypedGenomeSnapshot> {
  return {
    [Symbol.iterator]: () => iterateTypedGenomes(world)[Symbol.iterator]()
  };
}

/**
 * Assemble one complete current-format metadata record.
 * @param world - World at either an exact boundary or a manual export point.
 * @param boundaryKind - Stored boundary semantics.
 * @param simulationStep - Last committed step represented by the population.
 * @param identity - Run and config revision identity.
 * @param boundary - Exact boundary state when the row is resumable.
 * @returns Strict metadata stored once in the parent snapshot row.
 */
function buildMetadata(
  world: World,
  boundaryKind: SnapshotBoundaryKind,
  simulationStep: number,
  identity: CheckpointIdentity,
  boundary?: GenerationBoundaryState
): PopulationCheckpointMetadata {
  const resumable = boundaryKind !== 'population-export';
  if (resumable && !boundary) {
    throw new Error(`resumable ${boundaryKind} checkpoint requires exact boundary state`);
  }
  const rng = boundary?.rng ?? world.exportRngState();
  const allocators = boundary?.allocators ?? world.exportAllocatorState();
  return {
    formatVersion: SNAPSHOT_FORMAT_VERSION,
    boundaryVersion: SNAPSHOT_BOUNDARY_VERSION,
    boundaryKind,
    resumable,
    generation: world.generation,
    simulationStep,
    runId: identity.runId,
    worldSeed: world.seed,
    configHash: buildAuthoritativeConfigHash(world),
    configRevision: identity.configRevision,
    archKey: world.archKey,
    graphSpec: world.arch.spec,
    populationCount: world.population.length,
    settings: buildCoreSettingsSnapshot(world),
    updates: buildSettingsUpdatesSnapshot(),
    rng,
    allocators,
    bestFitnessEver: world.bestFitnessEver,
    fitnessHistory: world.fitnessHistory.map((entry) => ({ ...entry })),
    lastHofEntry: world._lastHoFEntry
  };
}

/**
 * Build a resumable checkpoint from the exact pre-spawn World hook.
 * @param world - World whose new population is assigned but not yet spawned.
 * @param boundary - Exact RNG/allocator boundary emitted by World.
 * @param configRevision - Revision applying to the new generation.
 * @returns Lazy typed checkpoint consumed synchronously by persistence.
 */
export function buildGenerationCheckpoint(
  world: World,
  boundary: GenerationBoundaryState,
  configRevision: number
): PopulationCheckpoint {
  if (boundary.generation !== world.generation || boundary.seed !== world.seed) {
    throw new Error('generation boundary identity does not match World state');
  }
  if (boundary.runId !== world.runId) {
    throw new Error('generation boundary run id does not match World state');
  }
  return {
    metadata: buildMetadata(
      world,
      boundary.kind,
      boundary.simulationStep,
      { runId: boundary.runId, configRevision },
      boundary
    ),
    genomes: createTypedGenomeIterable(world)
  };
}

/**
 * Build a non-resumable JSON-export checkpoint from live population state.
 * @param world - Active World at an arbitrary committed step.
 * @param options - Current run, configuration revision, and committed step.
 * @returns Lazy typed population-export checkpoint.
 */
export function buildPopulationExportCheckpoint(
  world: World,
  options: PopulationExportCheckpointOptions
): PopulationCheckpoint {
  return {
    metadata: buildMetadata(
      world,
      'population-export',
      options.simulationStep,
      options
    ),
    genomes: createTypedGenomeIterable(world)
  };
}
