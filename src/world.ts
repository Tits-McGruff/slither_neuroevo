/** Simulation world state, evolution loop, and rendering helpers. */

import { CFG } from './config.ts';
import { buildArch, archKey, Genome, crossover, mutate, enrichArchInfo } from './mlp.ts';
import { ParticleSystem } from './particles.ts';
import { Snake, Pellet, pointSegmentDist2 } from './snake.ts';
import type { ControlInput } from './snake.ts';
import { clamp, lerp, TAU } from './utils.ts';
import { hof } from './hallOfFame.ts';
import { FlatSpatialHash, type SpatialHashDiagnostics } from './spatialHash.ts';
import { BaselineBotManager, type BaselineBotRngState } from './bots/baselineBots.ts';
import { NullBrain } from './brains/nullBrain.ts';
import type { InferenceBackend } from './brains/types.ts';
import type { SimProfiler } from './profiling.ts';
import type { ArchDefinition } from './mlp.ts';
import type { GenomeJSON, HallOfFameEntry, PopulationImportData, PopulationExport } from './protocol/messages.ts';
import { DEFAULT_CORE_SETTINGS } from './protocol/settings.ts';
import {
  StatefulRng,
  deriveSeed,
  normalizeSeed,
  type RandomGenerator,
  type RandomSource,
  type SerializedRngState
} from './rng.ts';
import { THEME } from './theme.ts';
import { getSensorLayout } from './protocol/sensors.ts';

/** Starting id reserved for externally controlled snakes. */
const EXTERNAL_SNAKE_ID_START = 100000;
/** Starting id reserved for baseline bot snakes. */
const BASELINE_BOT_ID_START = 200000;
/** Starting id reserved for deterministic Hall-of-Fame resurrection spawns. */
const RESURRECTED_SNAKE_ID_START = 1000000000;
/** Hard safety bound for lower-level collision integration within one fixed step. */
const MAX_COLLISION_SUBSTEPS = 64;
/** Version of the exported authoritative RNG bundle. */
const WORLD_RNG_STATE_VERSION = 1 as const;
/** Version of the exported deterministic id-allocator bundle. */
const WORLD_ALLOCATOR_STATE_VERSION = 1 as const;

/** Authoritative result of a God Mode world mutation. */
export interface GodModeWorldResult {
  /** Whether the requested mutation was applied. */
  applied: boolean;
  /** Target snake id. */
  snakeId: number;
  /** Stable rejection reason when the mutation was not applied. */
  reason?: string;
  /** Actual authoritative head X after an accepted move. */
  x?: number;
  /** Actual authoritative head Y after an accepted move. */
  y?: number;
  /** Number of normal death pellets added by an accepted kill. */
  pelletsDropped?: number;
}

/**
 * Find the largest fraction of one translation that keeps every point inside
 * a circle centered at the origin.
 * @param points - Snake body points in world coordinates.
 * @param dx - Requested X translation.
 * @param dy - Requested Y translation.
 * @param limit - Maximum radial distance for each body point.
 * @returns Translation scale in [0, 1], or -1 when current state is invalid.
 */
function maxTranslationScaleInsideCircle(
  points: readonly { x: number; y: number }[],
  dx: number,
  dy: number,
  limit: number
): number {
  const deltaSquared = dx * dx + dy * dy;
  if (!Number.isFinite(deltaSquared)) return -1;
  const limitSquared = limit * limit;
  let scale = 1;
  for (const point of points) {
    if (!Number.isFinite(point.x) || !Number.isFinite(point.y)) return -1;
    const currentError = point.x * point.x + point.y * point.y - limitSquared;
    if (currentError > 1e-6) return -1;
    if (deltaSquared <= Number.EPSILON) continue;
    const linear = 2 * (point.x * dx + point.y * dy);
    const discriminant = linear * linear - 4 * deltaSquared * currentError;
    if (!Number.isFinite(linear) || !Number.isFinite(discriminant) || discriminant < 0) return -1;
    const exitScale = (-linear + Math.sqrt(discriminant)) / (2 * deltaSquared);
    if (Number.isFinite(exitScale)) scale = Math.min(scale, exitScale);
  }
  return clamp(scale, 0, 1);
}

/** Optional settings overrides accepted by the World constructor. */
interface WorldSettingsInput {
  snakeCount?: number;
  simSpeed?: number;
  hiddenLayers?: number;
  neurons1?: number;
  neurons2?: number;
  neurons3?: number;
  neurons4?: number;
  neurons5?: number;
  worldRadius?: number;
  observer?: Partial<typeof CFG.observer>;
  collision?: Partial<typeof CFG.collision>;
}

/** Normalized settings stored by the World instance. */
interface WorldSettings {
  snakeCount: number;
  simSpeed: number;
  hiddenLayers: number;
  neurons1: number;
  neurons2: number;
  neurons3: number;
  neurons4: number;
  neurons5: number;
  worldRadius: number;
  observer: typeof CFG.observer;
  collision: typeof CFG.collision;
}

/** Exported continuation state for every authoritative random stream. */
export interface WorldRngState {
  /** Bundle schema version. */
  version: typeof WORLD_RNG_STATE_VERSION;
  /** Normalized active run seed. */
  seed: number;
  /** Gameplay/world-construction stream. */
  world: SerializedRngState;
  /** Genome initialization and evolution stream. */
  evolution: SerializedRngState;
  /** Observer-only selection stream. */
  observer: SerializedRngState;
  /** Per-slot baseline-bot streams. */
  baselines: BaselineBotRngState[];
}

/** Exported continuation state for deterministic generated identifiers. */
export interface WorldAllocatorState {
  /** Bundle schema version. */
  version: typeof WORLD_ALLOCATOR_STATE_VERSION;
  /** Next external-controller snake id candidate. */
  nextExternalSnakeId: number;
  /** Next baseline-bot snake id candidate. */
  nextBaselineBotId: number;
  /** Next Hall-of-Fame resurrection id candidate. */
  nextResurrectedSnakeId: number;
}

/** Exact pre-spawn checkpoint boundary exposed to later persistence work. */
export interface GenerationBoundaryState {
  /** Boundary schema version. */
  version: 1;
  /** Reason this population boundary was created. */
  kind: 'run-start' | 'generation';
  /** Generation whose population has been assigned. */
  generation: number;
  /** Fixed step committed once generation construction completes. */
  simulationStep: number;
  /** Normalized active run seed. */
  seed: number;
  /** Evolutionary-lineage identifier independent from simulation randomness. */
  runId: string;
  /** Every authoritative RNG continuation before construction draws. */
  rng: WorldRngState;
  /** Every deterministic generated-id continuation. */
  allocators: WorldAllocatorState;
}

/** Callback invoked at an exact population-assigned, pre-spawn boundary. */
export type GenerationBoundaryHook = (
  boundary: GenerationBoundaryState,
  world: World
) => void;

/** One typed population member accepted by exact-boundary reconstruction. */
export interface WorldPopulationGenomeState {
  /** Stable architecture key. */
  archKey: string;
  /** Runtime brain-family metadata. */
  brainType: string;
  /** Fitness retained at the stored boundary. */
  fitness: number;
  /** Float32 parameter buffer. */
  weights: Float32Array;
}

/** Fitness-history record restored with an exact generation checkpoint. */
export interface WorldFitnessHistoryEntry {
  /** Generation summarized by the record. */
  gen: number;
  /** Best fitness in the generation. */
  best: number;
  /** Average fitness in the generation. */
  avg: number;
  /** Minimum fitness in the generation. */
  min: number;
  /** Number of detected species. */
  speciesCount: number;
  /** Size of the largest detected species. */
  topSpeciesSize: number;
  /** Average network weight. */
  avgWeight: number;
  /** Network-weight variance. */
  weightVariance: number;
}

/** Population-assigned state used to reconstruct a generation without random initialization. */
export interface WorldResumeState {
  /** Generation whose initial construction must be replayed. */
  generation: number;
  /** Last authoritative fixed step represented by the checkpoint. */
  simulationStep: number;
  /** Dense typed population in durable slot order. */
  population: readonly WorldPopulationGenomeState[];
  /** Exact random continuation, absent only for read-only legacy compatibility. */
  rng?: WorldRngState;
  /** Exact generated-id continuation, absent only for legacy compatibility. */
  allocators?: WorldAllocatorState;
  /** Best fitness observed before the stored boundary. */
  bestFitnessEver: number;
  /** Bounded evolution history retained across restart. */
  fitnessHistory: readonly WorldFitnessHistoryEntry[];
  /** Pending Hall-of-Fame event retained at the boundary. */
  lastHofEntry: HallOfFameEntry | null;
  /** Whether RNG and allocator continuations provide exact reconstruction. */
  exact: boolean;
}

/** Optional deterministic construction controls for a World. */
export interface WorldConstructionOptions {
  /** Root seed from which every named stream is derived directly. */
  seed?: number;
  /** Evolutionary-lineage identifier exposed at checkpoint boundaries. */
  runId?: string;
  /** Immutable neural math backend prepared before World construction. */
  inferenceBackend?: InferenceBackend;
  /** Optional observer for exact generation checkpoint boundaries. */
  onGenerationBoundary?: GenerationBoundaryHook;
  /** Optional generation-boundary state restored before any construction draw. */
  resume?: WorldResumeState;
}

/** Fitness history record stored by the world for charts. */
export interface FitnessHistoryEntry {
  gen: number;
  best: number;
  avg: number;
  min: number;
  speciesCount: number;
  topSpeciesSize: number;
  avgWeight: number;
  weightVariance: number;
}

/** Batched control buffers for neural inference. */
interface ControlBatch {
  /** Durable population slots requiring pooled inference in this batch. */
  indices: Uint32Array;
  /** Current snake-array index corresponding to each batch entry. */
  snakeIndices: Uint32Array;
  /** Number of active entries in the batch. */
  count: number;
  /** Allocated capacity for the batch. */
  capacity: number;
  /** Input stride for each batch entry. */
  inputStride: number;
  /** Output stride for each batch entry. */
  outputStride: number;
  /** Packed input buffer for batched inference. */
  inputs: Float32Array;
  /** Packed output buffer for batched inference. */
  outputs: Float32Array;
}

/** Runner interface for batched inference in async update paths. */
export interface BatchInferenceRunner {
  /**
   * Run batched inference into the provided output buffer.
   * @param inputs - Packed input buffer for the batch.
   * @param outputs - Packed output buffer for the batch.
   * @param indices - Snake indices for each batch entry.
   * @param count - Number of batch entries to process.
   * @param inputStride - Stride between batch inputs.
   * @param outputStride - Stride between batch outputs.
   */
  runBatch: (
    inputs: Float32Array,
    outputs: Float32Array,
    indices: Uint32Array,
    count: number,
    inputStride: number,
    outputStride: number
  ) => Promise<void>;
}

/** Minimal controller registry interface used by the World. */
export interface ControllerRegistryLike {
  isControlled: (snakeId: number) => boolean;
  getAction: (snakeId: number, tickId: number) => ControlInput | null;
  publishSensors: (
    snakeId: number,
    tickId: number,
    sensors: Float32Array,
    meta: { x: number; y: number; dir: number }
  ) => boolean;
}

/** Main simulation world containing population state, pellets, and snakes. */
export class World {
  /** Normalized active seed for this simulation lineage. */
  seed: number;
  /** Evolutionary-lineage identifier used by exact-boundary persistence. */
  readonly runId: string;
  /** Immutable math backend attached to every neural brain in this World. */
  readonly inferenceBackend: InferenceBackend;
  /** Gameplay and world-construction random stream. */
  worldRng: StatefulRng;
  /** Genome initialization and evolution random stream. */
  evolutionRng: StatefulRng;
  /** Observer-only random stream. */
  observerRng: StatefulRng;
  /** Normalized settings for the world instance. */
  settings: WorldSettings;
  /** Neural network architecture definition for the population. */
  arch: ArchDefinition;
  /** Stable architecture key used for persistence. */
  archKey: string;
  /** Active pellet instances in the world. */
  pellets: Pellet[];
  /** Spatial grid for pellet lookup. */
  pelletGrid: PelletGrid;
  /** Pellet spawn accumulator in seconds. */
  _pelletSpawnAcc: number;
  /** Active snake instances in the world. */
  snakes: Snake[];
  /** Baseline bot snakes appended after the population. */
  baselineBots: Snake[];
  /** Manager for baseline bot state and actions. */
  botManager: BaselineBotManager;
  /** Particle system used by the legacy renderer. */
  particles: ParticleSystem;
  /** Current generation index. */
  generation: number;
  /** Elapsed time in the current generation. */
  generationTime: number;
  /** Current population of genomes. */
  population: Genome[];
  /** Best fitness recorded across all generations. */
  bestFitnessEver: number;
  /** Rolling fitness history for charts. */
  fitnessHistory: FitnessHistoryEntry[];
  /** Best points achieved in the current generation. */
  bestPointsThisGen: number;
  /** Snake id that currently holds best points. */
  bestPointsSnakeId: number;
  /** Last Hall of Fame entry emitted by the world. */
  _lastHoFEntry: HallOfFameEntry | null;
  /** Current simulation tick id. */
  tickId: number = 0;
  /** Simulation speed multiplier. */
  simSpeed: number;
  /** Camera X coordinate for rendering. */
  cameraX: number;
  /** Camera Y coordinate for rendering. */
  cameraY: number;
  /** Camera zoom factor for rendering. */
  zoom: number;
  /** Snake currently focused by the observer. */
  focusSnake: Snake | null;
  /** Cooldown timer for focus switching. */
  _focusCooldown: number;
  /** Observer view mode. */
  viewMode: string;
  /** Collision grid for snake segments. */
  _collGrid: FlatSpatialHash<Snake>;
  /** Batched control buffers for neural inference. */
  _controlBatch: ControlBatch;
  /** Pending control source marker per snake. */
  _pendingControlSource: Uint8Array;
  /** Pending turn input per snake. */
  _pendingControlTurn: Float32Array;
  /** Pending boost input per snake. */
  _pendingControlBoost: Float32Array;
  /** Snake-array indices whose neural inference runs on the serial path. */
  _serialControlIndices: Uint32Array;
  /** Number of valid serial-control entries for the current fixed step. */
  _serialControlCount: number;
  /** Whether a sensor layout mismatch warning has been logged. */
  _didWarnSensorLayout: boolean;
  /** Next id to assign to externally controlled snakes. */
  _nextExternalSnakeId: number;
  /** Next id to assign to baseline bot spawns. */
  _nextBaselineBotId: number;
  /** Next deterministic id candidate for Hall-of-Fame resurrection spawns. */
  _nextResurrectedSnakeId: number;
  /** Optional exact-boundary observer used by later persistence work. */
  private generationBoundaryHook: GenerationBoundaryHook | null;
  /** Optional profiler for timing breakdowns. */
  profiler?: SimProfiler;

  /** Access the world radius from current settings. */
  get worldRadius(): number {
    return this.settings.worldRadius;
  }

  /**
   * Return operational collision-index measurements without exposing its storage.
   * @returns Capacity, load, rebuild, admission, and fault diagnostics.
   */
  getCollisionGridDiagnostics(): SpatialHashDiagnostics {
    return this._collGrid.getDiagnostics();
  }

  /**
   * Create a new World instance with optional settings overrides.
   * @param settings - World settings overrides from UI or worker.
   * @param options - Seed and generation-boundary controls.
   */
  constructor(settings: WorldSettingsInput = {}, options: WorldConstructionOptions = {}) {
    this.seed = normalizeSeed(options.seed ?? 0);
    this.runId = options.runId?.trim() || `world-${this.seed.toString(16).padStart(8, '0')}`;
    this.inferenceBackend = options.inferenceBackend ?? 'js';
    this.worldRng = new StatefulRng(deriveSeed(this.seed, 'world'));
    this.evolutionRng = new StatefulRng(deriveSeed(this.seed, 'evolution'));
    this.observerRng = new StatefulRng(deriveSeed(this.seed, 'observer'));
    this.generationBoundaryHook = options.onGenerationBoundary ?? null;
    // Store a shallow copy of the UI settings to decouple from external
    // mutations.  The settings include snakeCount, simSpeed and hidden layer
    // sizes.
    const observerSettings = { ...CFG.observer, ...(settings.observer ?? {}) };
    const collisionSettings = { ...CFG.collision, ...(settings.collision ?? {}) };
    const simSpeed = Number.isFinite(settings.simSpeed)
      ? clamp(settings.simSpeed as number, 0.01, 500)
      : 1;
    this.settings = {
      ...settings,
      snakeCount: settings.snakeCount ?? DEFAULT_CORE_SETTINGS.snakeCount,
      hiddenLayers: settings.hiddenLayers ?? DEFAULT_CORE_SETTINGS.hiddenLayers,
      neurons1: settings.neurons1 ?? DEFAULT_CORE_SETTINGS.neurons1,
      neurons2: settings.neurons2 ?? DEFAULT_CORE_SETTINGS.neurons2,
      neurons3: settings.neurons3 ?? DEFAULT_CORE_SETTINGS.neurons3,
      neurons4: settings.neurons4 ?? DEFAULT_CORE_SETTINGS.neurons4,
      neurons5: settings.neurons5 ?? DEFAULT_CORE_SETTINGS.neurons5,
      simSpeed,
      worldRadius: settings.worldRadius ?? CFG.worldRadius,
      observer: observerSettings,
      collision: collisionSettings
    };
    // Construct the neural architecture based on current settings.
    this.arch = buildArch(this.settings);
    this.archKey = this.arch.key || archKey(this.arch);
    this.pellets = [];
    this.pelletGrid = new PelletGrid();
    this._pelletSpawnAcc = 0;
    this.snakes = [];
    this.baselineBots = [];
    this.botManager = new BaselineBotManager(CFG.baselineBots, this.seed);
    this.particles = new ParticleSystem(); // Initialize particle system
    this.generation = 1;
    this.generationTime = 0;
    this.population = [];
    this.bestFitnessEver = 0;
    // Must start finite or sensor percentiles will produce NaNs on the first tick.
    this.fitnessHistory = []; // Track fitness over generations
    this.bestPointsThisGen = 0;
    this.bestPointsSnakeId = 0;
    this._lastHoFEntry = null;
    // Simulation speed is consumed only by SimCore's wall-time scheduler.
    this.simSpeed = this.settings.simSpeed;
    // Camera state for panning and zooming.
    this.cameraX = 0;
    this.cameraY = 0;
    this.zoom = 1.0;
    this.focusSnake = null;
    this._focusCooldown = 0;
    this.viewMode = this.settings.observer.defaultViewMode || "overview";
    this.zoom = 1.0;

    // Init physics
    // Estimate capacity: 50 snakes * 100 len = 5000 segments. 5000 * 500 len = 2.5m.
    // Let's allocate big. 200,000 capacity safe for now?
    // worldRadius * 2 = width.
    const w = this.settings.worldRadius * 2.5;
    this._collGrid = new FlatSpatialHash(w, w, this.settings.collision.cellSize, 200000);
    this._controlBatch = {
      indices: new Uint32Array(0),
      snakeIndices: new Uint32Array(0),
      count: 0,
      capacity: 0,
      inputStride: 0,
      outputStride: 0,
      inputs: new Float32Array(0),
      outputs: new Float32Array(0)
    };
    this._pendingControlSource = new Uint8Array(0);
    this._pendingControlTurn = new Float32Array(0);
    this._pendingControlBoost = new Float32Array(0);
    this._serialControlIndices = new Uint32Array(0);
    this._serialControlCount = 0;
    this._didWarnSensorLayout = false;
    this._nextExternalSnakeId = EXTERNAL_SNAKE_ID_START;
    this._nextBaselineBotId = BASELINE_BOT_ID_START;
    this._nextResurrectedSnakeId = RESURRECTED_SNAKE_ID_START;
    if (options.resume) {
      this._restoreGenerationBoundary(options.resume);
    } else {
      this._initPopulation();
      this._resetBaselineBotsForGen();
      this._emitGenerationBoundary('run-start', 0);
    }
    this._spawnAll();
    this._collGrid.build(this.snakes, CFG.collision.skipSegments);
    this._initPellets();
    this._chooseInitialFocus();
  }

  /**
   * Export every authoritative RNG stream for a future exact-boundary resume.
   * @returns Lossless versioned RNG bundle.
   */
  exportRngState(): WorldRngState {
    return {
      version: WORLD_RNG_STATE_VERSION,
      seed: this.seed,
      world: this.worldRng.exportState(),
      evolution: this.evolutionRng.exportState(),
      observer: this.observerRng.exportState(),
      baselines: this.botManager.exportRngStates()
    };
  }

  /**
   * Restore every authoritative RNG stream after strict seed/version validation.
   * @param state - Bundle previously returned by `exportRngState`.
   */
  restoreRngState(state: WorldRngState): void {
    if (state.version !== WORLD_RNG_STATE_VERSION || state.seed !== this.seed) {
      throw new TypeError(
        `World RNG state ${state.version}/${state.seed} does not match ${WORLD_RNG_STATE_VERSION}/${this.seed}`
      );
    }
    const world = StatefulRng.fromState(state.world);
    const evolution = StatefulRng.fromState(state.evolution);
    const observer = StatefulRng.fromState(state.observer);
    this.botManager.restoreRngStates(state.baselines);
    this.worldRng.restoreState(world.exportState());
    this.evolutionRng.restoreState(evolution.exportState());
    this.observerRng.restoreState(observer.exportState());
  }

  /**
   * Export every deterministic generated-id continuation.
   * @returns Versioned allocator state.
   */
  exportAllocatorState(): WorldAllocatorState {
    return {
      version: WORLD_ALLOCATOR_STATE_VERSION,
      nextExternalSnakeId: this._nextExternalSnakeId,
      nextBaselineBotId: this._nextBaselineBotId,
      nextResurrectedSnakeId: this._nextResurrectedSnakeId
    };
  }

  /**
   * Restore deterministic generated-id continuations after validation.
   * @param state - Allocator state previously returned by `exportAllocatorState`.
   */
  restoreAllocatorState(state: WorldAllocatorState): void {
    if (state.version !== WORLD_ALLOCATOR_STATE_VERSION) {
      throw new TypeError(`Unsupported World allocator state version ${state.version}`);
    }
    if (
      !Number.isSafeInteger(state.nextExternalSnakeId) ||
      state.nextExternalSnakeId < EXTERNAL_SNAKE_ID_START ||
      !Number.isSafeInteger(state.nextBaselineBotId) ||
      state.nextBaselineBotId < BASELINE_BOT_ID_START ||
      !Number.isSafeInteger(state.nextResurrectedSnakeId) ||
      state.nextResurrectedSnakeId < RESURRECTED_SNAKE_ID_START
    ) {
      throw new TypeError('World allocator state contains an invalid id candidate');
    }
    this._nextExternalSnakeId = state.nextExternalSnakeId;
    this._nextBaselineBotId = state.nextBaselineBotId;
    this._nextResurrectedSnakeId = state.nextResurrectedSnakeId;
  }

  /**
   * Publish an exact population-assigned boundary before construction draws.
   * @param kind - Run-start or evolved-generation boundary kind.
   */
  private _emitGenerationBoundary(
    kind: GenerationBoundaryState['kind'],
    simulationStep: number
  ): void {
    if (!this.generationBoundaryHook) return;
    this.generationBoundaryHook({
      version: 1,
      kind,
      generation: this.generation,
      simulationStep,
      seed: this.seed,
      runId: this.runId,
      rng: this.exportRngState(),
      allocators: this.exportAllocatorState()
    }, this);
  }

  /**
   * Restore a population-assigned generation boundary before any spawn draw.
   * @param resume - Strict current checkpoint or bounded legacy compatibility state.
   */
  private _restoreGenerationBoundary(resume: WorldResumeState): void {
    if (!Number.isSafeInteger(resume.generation) || resume.generation < 1) {
      throw new TypeError('World resume generation is invalid');
    }
    if (!Number.isSafeInteger(resume.simulationStep) || resume.simulationStep < 0) {
      throw new TypeError('World resume simulation step is invalid');
    }
    if (resume.population.length !== this.settings.snakeCount) {
      throw new TypeError(
        `World resume population ${resume.population.length} does not match snakeCount ${this.settings.snakeCount}`
      );
    }
    const expectedWeights = enrichArchInfo(this.arch).totalCount;
    const population: Genome[] = new Array(resume.population.length);
    for (let slot = 0; slot < resume.population.length; slot++) {
      const source = resume.population[slot];
      if (!source) throw new TypeError(`World resume population slot ${slot} is missing`);
      if (source.archKey !== this.archKey) {
        throw new TypeError(
          `World resume genome ${slot} architecture ${source.archKey} does not match ${this.archKey}`
        );
      }
      if (source.brainType !== this.arch.spec.type) {
        throw new TypeError(
          `World resume genome ${slot} brain type ${source.brainType} does not match ${this.arch.spec.type}`
        );
      }
      if (!(source.weights instanceof Float32Array) || source.weights.length !== expectedWeights) {
        throw new TypeError(
          `World resume genome ${slot} has ${source.weights?.length ?? 0} weights; expected ${expectedWeights}`
        );
      }
      for (let index = 0; index < source.weights.length; index++) {
        if (!Number.isFinite(source.weights[index])) {
          throw new TypeError(`World resume genome ${slot} weight ${index} is not finite`);
        }
      }
      if (!Number.isFinite(source.fitness)) {
        throw new TypeError(`World resume genome ${slot} fitness is invalid`);
      }
      const genome = new Genome(source.archKey, source.weights, source.brainType);
      genome.fitness = source.fitness;
      population[slot] = genome;
    }
    if (!Number.isFinite(resume.bestFitnessEver)) {
      throw new TypeError('World resume best fitness is invalid');
    }
    if (resume.exact && (!resume.rng || !resume.allocators)) {
      throw new TypeError('Exact World resume requires RNG and allocator state');
    }
    this.population = population;
    this.generation = resume.generation;
    this.tickId = resume.simulationStep;
    this.bestFitnessEver = resume.bestFitnessEver;
    this.fitnessHistory = resume.fitnessHistory.map((entry) => ({ ...entry }));
    this._lastHoFEntry = resume.lastHofEntry;
    this._resetBaselineBotsForGen();
    if (resume.rng) this.restoreRngState(resume.rng);
    if (resume.allocators) this.restoreAllocatorState(resume.allocators);
  }

  /**
   * Remove prior-generation transient objects before exposing a new boundary.
   * Population and RNG/allocator continuations remain intact; no random draw
   * occurs here, and new snakes/pellets/focus are created only after the hook.
   */
  private _clearTransientGenerationState(): void {
    this.snakes.length = 0;
    this.baselineBots.length = 0;
    this.pellets.length = 0;
    this.pelletGrid.resetForCFG();
    this._pelletSpawnAcc = 0;
    this.focusSnake = null;
    this._collGrid.reset(this.settings.collision.cellSize);
  }

  /**
   * Immediately adjusts the simulation speed.  Also stores the new
   * value back into the settings object.
   * @param x - New simulation speed multiplier.
   */
  applyLiveSimSpeed(x: number): void {
    this.simSpeed = clamp(x, 0.01, 500.0);
    this.settings.simSpeed = this.simSpeed;
  }

  /**
   * Apply the cached baseline-bot respawn delay alongside the global CFG value.
   * @param seconds - Authoritative normalized delay in seconds.
   * @returns Delay retained by the bot manager.
   */
  applyLiveBaselineRespawnDelay(seconds: number): number {
    return this.botManager.updateRespawnDelay(seconds);
  }

  /**
   * Kill an alive snake through its normal death path and refresh collisions.
   * @param snakeId - Target snake id.
   * @returns Applied or rejected authoritative result.
   */
  applyGodModeKill(snakeId: number): GodModeWorldResult {
    const snake = this.snakes.find((candidate) => candidate.id === snakeId);
    if (!snake || !snake.alive) {
      return { applied: false, snakeId, reason: 'snake is missing or already dead' };
    }
    const pelletCountBefore = this.pellets.length;
    snake.die(this);
    this._rebuildCollisionGrid();
    return {
      applied: true,
      snakeId,
      pelletsDropped: Math.max(0, this.pellets.length - pelletCountBefore)
    };
  }

  /**
   * Translate an alive snake head and every body point by one clamped delta.
   * @param snakeId - Target snake id.
   * @param targetX - Requested head X coordinate.
   * @param targetY - Requested head Y coordinate.
   * @returns Applied or rejected authoritative result including actual position.
   */
  applyGodModeMove(snakeId: number, targetX: number, targetY: number): GodModeWorldResult {
    if (!Number.isFinite(targetX) || !Number.isFinite(targetY)) {
      return { applied: false, snakeId, reason: 'move coordinates must be finite' };
    }
    const snake = this.snakes.find((candidate) => candidate.id === snakeId);
    if (!snake || !snake.alive) {
      return { applied: false, snakeId, reason: 'snake is missing or already dead' };
    }
    const dx = targetX - snake.x;
    const dy = targetY - snake.y;
    const radialLimit = Math.max(0, this.worldRadius - snake.radius);
    const scale = maxTranslationScaleInsideCircle(snake.points, dx, dy, radialLimit);
    if (scale < 0) {
      return { applied: false, snakeId, reason: 'snake body is outside valid world bounds' };
    }
    if (scale <= Number.EPSILON && (Math.abs(dx) > 1e-9 || Math.abs(dy) > 1e-9)) {
      return { applied: false, snakeId, reason: 'translation cannot keep the body in bounds' };
    }
    const appliedDx = dx * scale;
    const appliedDy = dy * scale;
    snake.x += appliedDx;
    snake.y += appliedDy;
    for (const point of snake.points) {
      point.x += appliedDx;
      point.y += appliedDy;
    }
    this._rebuildCollisionGrid();
    return { applied: true, snakeId, x: snake.x, y: snake.y };
  }
  /**
   * Toggles between overview and follow camera modes.  Ensures that a
   * valid focus snake is selected when switching to follow mode.
   */
  toggleViewMode(): void {
    this.viewMode = this.viewMode === "overview" ? "follow" : "overview";
    if (!this.focusSnake || !this.focusSnake.alive) this.focusSnake = this._pickAnyAlive();
    if (this.viewMode === "overview") {
      this.cameraX = 0;
      this.cameraY = 0;
    } else if (this.focusSnake && this.focusSnake.alive) {
      const h = this.focusSnake.head();
      this.cameraX = h.x;
      this.cameraY = h.y;
    }
  }

  /**
   * Notify the world when a baseline bot dies.
   * @param snake - Snake that died.
   */
  baselineBotDied(snake: Snake): void {
    const idx = snake.baselineBotIndex;
    if (idx == null) return;
    this.botManager.markDead(idx);
  }
  /**
   * Chooses an alive snake at random.  Returns null if none.
   */
  _pickAnyAlive(): Snake | null {
    const alive = this.snakes.filter(s => s.alive);
    if (!alive.length) return null;
    const idx = this.observerRng.int(alive.length);
    return alive[idx] ?? null;
  }
  /**
   * Initialises the population with random genomes according to the
   * current architecture.
   */
  _initPopulation(): void {
    this.population.length = 0;
    const rng = this.evolutionRng.asSource();
    for (let i = 0; i < this.settings.snakeCount; i++) {
      this.population.push(Genome.random(this.arch, rng));
    }
  }

  /**
   * Serializes the current population for export.
   * @returns Population export payload.
   */
  exportPopulation(): PopulationExport {
    return {
      generation: this.generation,
      archKey: this.archKey,
      genomes: this.population.map(g => g.toJSON())
    };
  }

  /**
   * Replaces the current population from imported JSON data.
   * The caller is responsible for validating the data before calling.
   * @param data - Import payload containing genomes and optional generation.
   * @returns Import result summary.
   */
  importPopulation(data: PopulationImportData): { ok: boolean; reason?: string; used?: number; total?: number } {
    if (!data || !Array.isArray(data.genomes)) {
      return { ok: false, reason: 'missing genomes' };
    }
    const info = enrichArchInfo(this.arch);
    const expectedLen = info.totalCount;
    const expectedKey = this.archKey;
    const parsed = [];
    for (const raw of data.genomes) {
      try {
        const g = Genome.fromJSON(raw);
        if (g.archKey !== expectedKey) continue;
        if (!g.weights || g.weights.length !== expectedLen) continue;
        g.fitness = 0;
        parsed.push(g);
      } catch {
        // Skip malformed entries.
      }
    }
    if (!parsed.length) {
      return { ok: false, reason: 'no compatible genomes' };
    }
    const targetCount = Math.max(1, Math.floor(this.settings.snakeCount || parsed.length));
    const nextPop = [];
    const rng = this.evolutionRng.asSource();
    for (let i = 0; i < targetCount; i++) {
      const candidate = parsed[i];
      if (candidate) nextPop.push(candidate.clone());
      else nextPop.push(Genome.random(this.arch, rng));
    }
    this.population = nextPop;
    this.generation = Number.isFinite(data.generation)
      ? Math.max(1, Math.floor(data.generation!))
      : 1;
    this.generationTime = 0;
    this.bestPointsThisGen = 0;
    this.bestPointsSnakeId = 0;
    this.bestFitnessEver = 0;
    this.fitnessHistory = [];
    this.particles = new ParticleSystem();
    this._initPellets();
    this._resetBaselineBotsForGen();
    this._spawnAll();
    this._collGrid.build(this.snakes, CFG.collision.skipSegments);
    this._chooseInitialFocus();
    return { ok: true, used: parsed.length, total: targetCount };
  }
  /**
   * Spawns snakes from the current population genomes.
   */
  _spawnAll(): void {
    this.snakes.length = 0;
    const rng = this.worldRng.asSource();
    for (let i = 0; i < this.population.length; i++) {
      const g = this.population[i];
      if (!g) continue;
      this.snakes.push(new Snake(i + 1, g.clone(), this.arch, {
        populationSlot: i,
        inferenceBackend: this.inferenceBackend,
        rng
      }));
    }
    this._spawnBaselineBots();
  }

  /**
   * Reset baseline bot manager state for the current generation.
   */
  _resetBaselineBotsForGen(): void {
    this.botManager.resetForGeneration(this.generation);
    this._nextBaselineBotId = BASELINE_BOT_ID_START;
  }

  /**
   * Spawn baseline bots after the population snakes.
   */
  _spawnBaselineBots(): void {
    this.baselineBots.length = 0;
    const count = this.botManager.getCount();
    if (count <= 0) return;
    for (let i = 0; i < count; i++) {
      const rng = this.botManager.prepareBotSpawn(i);
      const snake = this._createBaselineSnake(i, rng);
      if (!snake) {
        console.warn('[baselineBots] bot.respawn.failed', {
          baselineBotIndex: i,
          reason: 'invalid id range'
        });
        continue;
      }
      this.baselineBots.push(snake);
      this.snakes.push(snake);
      this.botManager.registerBot(i, snake.id);
    }
  }

  /**
   * Build a baseline bot genome with zeroed weights.
   * @returns Baseline genome instance.
   */
  _createBaselineGenome(): Genome {
    const info = enrichArchInfo(this.arch);
    const weights = new Float32Array(info.totalCount);
    return new Genome(this.archKey, weights, this.arch.spec.type);
  }

  /**
   * Create a baseline bot snake instance.
   * @param index - Baseline bot index.
   * @param rng - RNG for spawn position and heading.
   * @returns Spawned snake or null when the id allocator is exhausted.
   */
  _createBaselineSnake(index: number, rng: RandomSource): Snake | null {
    const nextId = this._nextBaselineBotId;
    if (!Number.isSafeInteger(nextId) || nextId >= Number.MAX_SAFE_INTEGER) return null;
    this._nextBaselineBotId = nextId + 1;
    const snake = new Snake(nextId, this._createBaselineGenome(), this.arch, {
      rng,
      brain: new NullBrain(),
      controlMode: 'external-only',
      baselineBotIndex: index,
      populationSlot: null,
      skin: 2,
    });
    snake.color = THEME.snakeRobotBody;
    return snake;
  }

  /**
   * Respawn a baseline bot and reinsert it into the snake list.
   * @param index - Baseline bot index.
   * @param rng - RNG for spawn position and heading.
   * @returns Spawned snake or null when respawn fails.
   */
  _respawnBaselineBot(index: number, rng: RandomSource): Snake | null {
    const snake = this._createBaselineSnake(index, rng);
    if (!snake) return null;
    const slot = this.population.length + index;
    if (slot < 0 || slot > this.snakes.length) return null;
    if (slot === this.snakes.length) {
      this.snakes.push(snake);
    } else {
      this.snakes[slot] = snake;
    }
    this.baselineBots[index] = snake;
    return snake;
  }
  /**
   * Fills the world with pellets up to the configured target count.
   */
  _initPellets(): void {
    this.pellets.length = 0;
    this.pelletGrid.resetForCFG();
    this._pelletSpawnAcc = 0;
    while (this.pellets.length < CFG.pelletCountTarget) this.addPellet(this._spawnAmbientPellet());
  }

  /**
   * Adds a pellet to the world and to the pellet spatial hash.
   * @param p - Pellet to add.
   */
  addPellet(p: Pellet): void {
    p._idx = this.pellets.length;
    this.pellets.push(p);
    this.pelletGrid.add(p);
  }

  /**
   * Removes a pellet from the world and from the pellet spatial hash.
   * @param p - Pellet to remove.
   */
  removePellet(p: Pellet): void {
    if (!p) return;
    this.pelletGrid.remove(p);
    const idx = p._idx;
    if (idx == null || idx < 0 || idx >= this.pellets.length) return;
    const last = this.pellets.pop()!;
    if (last !== p) {
      this.pellets[idx] = last;
      last._idx = idx;
    }
    p._idx = -1;
  }

  /**
   * Execute one complete authoritative fixed simulation step.
   *
   * Ordering is intentionally shared by serial and pooled inference: advance
   * time/accounting, sample the stable pre-movement world, collect every due
   * control, await inference when needed, commit controls, integrate movement
   * and collisions, then complete observer/statistics/generation work.
   *
   * @param baseDt - Positive fixed simulation delta in seconds.
   * @param viewW - Viewport width used only for observer camera state.
   * @param viewH - Viewport height used only for observer camera state.
   * @param controllers - Optional external controller registry.
   * @param tickId - Authoritative step id assigned by the caller.
   * @param batchRunner - Optional population inference runner.
   */
  async step(
    baseDt: number,
    viewW: number,
    viewH: number,
    controllers?: ControllerRegistryLike,
    tickId = 0,
    batchRunner?: BatchInferenceRunner
  ): Promise<void> {
    if (!Number.isFinite(baseDt) || baseDt <= 0) {
      throw new RangeError(`World.step requires a positive finite baseDt; received ${baseDt}`);
    }

    const profiler = this.profiler;
    profiler?.beginTick();
    try {
      const controllerTick = Number.isSafeInteger(tickId) && tickId >= 0 ? tickId : 0;
      this.generationTime += baseDt;
      this.particles.update(baseDt);
      if (!Number.isFinite(this.bestPointsThisGen)) {
        console.warn('[world] bestPointsThisGen.invalid', { value: this.bestPointsThisGen });
        this.bestPointsThisGen = 0;
      }
      this._warnOnSensorLayoutMismatch();

      for (const snake of this.snakes) {
        if (snake.alive) snake.prepareForStep(baseDt);
      }
      this._spawnAmbientForFixedStep(baseDt);
      if (controllers) this._publishControllerSensors(controllers, controllerTick);
      if (this.botManager.getCount() > 0) {
        this.botManager.update(this, baseDt, (index, rng) => {
          const respawned = this._respawnBaselineBot(index, rng);
          respawned?.prepareForStep(baseDt);
          return respawned;
        });
      }

      await this._collectFixedStepControls(
        baseDt,
        controllers,
        controllerTick,
        batchRunner
      );
      this._applyFixedStepControls();
      this._advanceFixedStepPhysics(baseDt);
      this._finishFixedStep(baseDt, viewW, viewH, controllerTick);
      this.tickId = controllerTick;
    } finally {
      profiler?.endTick();
    }
  }

  /**
   * Spawn ambient pellets due during one fixed step before observations.
   * @param baseDt - Fixed simulation delta in seconds.
   */
  private _spawnAmbientForFixedStep(baseDt: number): void {
    const deficit = Math.max(0, CFG.pelletCountTarget - this.pellets.length);
    this._pelletSpawnAcc += CFG.pelletSpawnPerSecond * baseDt;
    const spawnCount = Math.min(deficit, Math.floor(this._pelletSpawnAcc));
    this._pelletSpawnAcc -= spawnCount;
    for (let index = 0; index < spawnCount; index++) {
      this.addPellet(this._spawnAmbientPellet());
    }
  }

  /**
   * Collect all due controls from one stable pre-movement snapshot.
   * @param baseDt - Fixed simulation delta in seconds.
   * @param controllers - Optional external controller registry.
   * @param tickId - Authoritative step id for controller actions.
   * @param batchRunner - Optional population inference runner.
   */
  private async _collectFixedStepControls(
    baseDt: number,
    controllers: ControllerRegistryLike | undefined,
    tickId: number,
    batchRunner: BatchInferenceRunner | undefined
  ): Promise<void> {
    this._ensureControlScratchCapacity(this.snakes.length);
    const batch = this._buildControlBatch();
    const pendingSource = this._pendingControlSource;
    const pendingTurn = this._pendingControlTurn;
    const pendingBoost = this._pendingControlBoost;
    pendingSource.fill(0, 0, this.snakes.length);
    this._serialControlCount = 0;

    for (let snakeIndex = 0; snakeIndex < this.snakes.length; snakeIndex++) {
      const snake = this.snakes[snakeIndex];
      if (!snake || !snake.alive) continue;

      const botAction = this.botManager.getActionForSnake(snake.id);
      if (botAction) {
        pendingSource[snakeIndex] = 1;
        pendingTurn[snakeIndex] = botAction.turn ?? 0;
        pendingBoost[snakeIndex] = botAction.boost ?? 0;
        continue;
      }
      if (controllers && controllers.isControlled(snake.id)) {
        const control = controllers.getAction(snake.id, tickId);
        if (control) {
          pendingSource[snakeIndex] = 1;
          pendingTurn[snakeIndex] = control.turn ?? 0;
          pendingBoost[snakeIndex] = control.boost ?? 0;
          continue;
        }
      }
      if (snake.controlMode === 'external-only') {
        pendingSource[snakeIndex] = 1;
        pendingTurn[snakeIndex] = 0;
        pendingBoost[snakeIndex] = 0;
        continue;
      }
      if (!snake.needsControlUpdate(baseDt)) continue;

      const sensors = this._sampleControlSensors(snake);
      snake.lastSensors = sensors;
      const populationSlot = snake.populationSlot;
      if (populationSlot !== null && (
        !Number.isSafeInteger(populationSlot) ||
        populationSlot < 0 ||
        populationSlot >= this.population.length
      )) {
        throw new Error(`Invalid population slot ${populationSlot} for snake ${snake.id}`);
      }

      if (batchRunner && populationSlot !== null) {
        const batchIndex = batch.count++;
        batch.indices[batchIndex] = populationSlot;
        batch.snakeIndices[batchIndex] = snakeIndex;
        batch.inputs.set(sensors, batchIndex * batch.inputStride);
      } else {
        this._serialControlIndices[this._serialControlCount++] = snakeIndex;
      }
    }

    for (let serialIndex = 0; serialIndex < this._serialControlCount; serialIndex++) {
      const snakeIndex = this._serialControlIndices[serialIndex] ?? 0;
      const snake = this.snakes[snakeIndex];
      if (!snake || !snake.alive || !snake.lastSensors) continue;
      const output = this._runSerialInference(snake, snake.lastSensors);
      pendingSource[snakeIndex] = 2;
      pendingTurn[snakeIndex] = output[0] ?? 0;
      pendingBoost[snakeIndex] = output[1] ?? 0;
    }

    if (!batchRunner || batch.count <= 0) return;
    const start = this.profiler?.now();
    await batchRunner.runBatch(
      batch.inputs,
      batch.outputs,
      batch.indices,
      batch.count,
      batch.inputStride,
      batch.outputStride
    );
    if (this.profiler && start != null) {
      this.profiler.recordBrain(this.profiler.now() - start);
    }
    for (let batchIndex = 0; batchIndex < batch.count; batchIndex++) {
      const snakeIndex = batch.snakeIndices[batchIndex] ?? 0;
      const outputBase = batchIndex * batch.outputStride;
      pendingSource[snakeIndex] = 2;
      pendingTurn[snakeIndex] = batch.outputs[outputBase] ?? 0;
      pendingBoost[snakeIndex] = batch.outputStride > 1
        ? (batch.outputs[outputBase + 1] ?? 0)
        : 0;
    }
  }

  /**
   * Compute one neural observation with optional profiling.
   * @param snake - Snake whose observation is due.
   * @returns Sensor vector owned by the snake scratch buffer.
   */
  private _sampleControlSensors(snake: Snake): Float32Array {
    const profiler = this.profiler;
    if (!profiler) return snake.sampleSensors(this);
    const start = profiler.now();
    const sensors = snake.sampleSensors(this);
    profiler.recordSensors(profiler.now() - start);
    return sensors;
  }

  /**
   * Run one serial neural inference after control collection is complete.
   * @param snake - Snake owning the serial brain.
   * @param sensors - Stable observation sampled for this fixed step.
   * @returns Raw neural outputs.
   */
  private _runSerialInference(snake: Snake, sensors: Float32Array): Float32Array {
    const profiler = this.profiler;
    if (!profiler) return snake.brain.forward(sensors);
    const start = profiler.now();
    const output = snake.brain.forward(sensors);
    profiler.recordBrain(profiler.now() - start);
    return output;
  }

  /** Commit collected controls without moving any snake. */
  private _applyFixedStepControls(): void {
    for (let snakeIndex = 0; snakeIndex < this.snakes.length; snakeIndex++) {
      const snake = this.snakes[snakeIndex];
      if (!snake || !snake.alive) continue;
      const source = this._pendingControlSource[snakeIndex] ?? 0;
      if (source === 1) {
        snake.applyExternalControl({
          turn: this._pendingControlTurn[snakeIndex] ?? 0,
          boost: this._pendingControlBoost[snakeIndex] ?? 0
        });
      } else if (source === 2) {
        snake.applyBrainOutput(
          this._pendingControlTurn[snakeIndex] ?? 0,
          this._pendingControlBoost[snakeIndex] ?? 0
        );
      }
    }
  }

  /**
   * Integrate movement and collisions using controls held for the full step.
   * Lower-level subdivision depends only on fixed-step collision safety, never
   * on the requested simulation-speed multiplier.
   * @param baseDt - Fixed simulation delta in seconds.
   */
  private _advanceFixedStepPhysics(baseDt: number): void {
    const maxSubstep = clamp(CFG.collision.substepMaxDt, 0.001, baseDt);
    const substepCount = clamp(
      Math.ceil(baseDt / maxSubstep),
      1,
      MAX_COLLISION_SUBSTEPS
    );
    const substepDt = baseDt / substepCount;
    for (let substep = 0; substep < substepCount; substep++) {
      for (const snake of this.snakes) {
        if (snake.alive) snake.advance(this, substepDt);
      }
      this._rebuildCollisionGrid();
      this._resolveCollisionsGrid();
    }
  }

  /** Rebuild the segment collision grid from the current alive snake bodies. */
  private _rebuildCollisionGrid(): void {
    this._collGrid.build(
      this.snakes,
      CFG.collision.skipSegments,
      CFG.collision.cellSize
    );
  }

  /**
   * Complete observer, score-summary, and generation-boundary work.
   * @param baseDt - Fixed simulation delta in seconds.
   * @param viewW - Viewport width used for observer camera state.
   * @param viewH - Viewport height used for observer camera state.
   */
  private _finishFixedStep(
    baseDt: number,
    viewW: number,
    viewH: number,
    controllerTick: number
  ): void {
    this._updateFocus(baseDt);
    this._updateCamera(viewW, viewH);
    let bestPoints = -Infinity;
    let bestId = 0;
    let aliveCount = 0;
    for (let populationSlot = 0; populationSlot < this.population.length; populationSlot++) {
      const snake = this.snakes[populationSlot];
      if (!snake || !snake.alive) continue;
      aliveCount += 1;
      if (snake.pointsScore > bestPoints) {
        bestPoints = snake.pointsScore;
        bestId = snake.id;
      }
    }
    const previousBest = Number.isFinite(this.bestPointsThisGen)
      ? this.bestPointsThisGen
      : 0;
    this.bestPointsThisGen = Math.max(
      previousBest,
      bestPoints > -Infinity ? bestPoints : 0
    );
    if (bestId) this.bestPointsSnakeId = bestId;
    const early = (
      aliveCount <= CFG.observer.earlyEndAliveThreshold &&
      this.generationTime >= CFG.observer.earlyEndMinSeconds
    );
    if (this.generationTime >= CFG.generationSeconds || early) {
      this._endGeneration(controllerTick);
    }
  }

  /**
   * Warn once when the sensor layout size does not match CFG.brain.inSize.
   */
  _warnOnSensorLayoutMismatch(): void {
    if (this._didWarnSensorLayout) return;
    const sense = CFG.sense ?? {};
    const layout = getSensorLayout(sense.bubbleBins ?? 16, sense.layoutVersion ?? 'v2');
    if (layout.inputSize === CFG.brain.inSize) return;
    console.warn('[world] sensor_layout.mismatch', {
      expected: layout.inputSize,
      actual: CFG.brain.inSize,
      layoutVersion: layout.layoutVersion,
      bins: layout.bins
    });
    this._didWarnSensorLayout = true;
  }
  /**
   * Ensure the control batch buffers are sized for the current strides.
   * @param required - Required number of batch entries.
   */
  _ensureControlBatchCapacity(required: number): void {
    const batch = this._controlBatch;
    const inputStride = Math.max(0, Math.floor(CFG.brain.inSize));
    const outputStride = Math.max(0, Math.floor(CFG.brain.outSize));
    const capacity = Math.max(0, Math.floor(required));
    if (
      batch.capacity >= capacity &&
      batch.inputStride === inputStride &&
      batch.outputStride === outputStride
    ) {
      return;
    }
    batch.capacity = capacity;
    batch.inputStride = inputStride;
    batch.outputStride = outputStride;
    batch.indices = new Uint32Array(capacity);
    batch.snakeIndices = new Uint32Array(capacity);
    batch.inputs = new Float32Array(capacity * inputStride);
    batch.outputs = new Float32Array(capacity * outputStride);
  }
  /**
   * Ensure pending control scratch buffers are sized for the current population.
   * @param required - Required number of snake slots.
   */
  _ensureControlScratchCapacity(required: number): void {
    const capacity = Math.max(0, Math.floor(required));
    if (
      this._pendingControlSource.length >= capacity &&
      this._serialControlIndices.length >= capacity
    ) {
      return;
    }
    this._pendingControlSource = new Uint8Array(capacity);
    this._pendingControlTurn = new Float32Array(capacity);
    this._pendingControlBoost = new Float32Array(capacity);
    this._serialControlIndices = new Uint32Array(capacity);
  }
  /**
   * Build the control batch buffers for this substep.
   * @returns Batched control data for this substep.
   */
  _buildControlBatch(): ControlBatch {
    this._ensureControlBatchCapacity(this.snakes.length);
    const batch = this._controlBatch;
    batch.count = 0;
    return batch;
  }
  /**
   * Publishes sensor vectors for externally controlled snakes at the start
   * of each tick so clients see a consistent snapshot.
   */
  _publishControllerSensors(controllers: ControllerRegistryLike, tickId: number): void {
    const profiler = this.profiler;
    for (const sn of this.snakes) {
      if (!sn.alive) continue;
      if (!controllers.isControlled(sn.id)) continue;
      const sensorStart = profiler?.now();
      const deliver = (sensors: Float32Array): boolean => {
        if (profiler && sensorStart !== undefined) {
          profiler.recordSensors(profiler.now() - sensorStart);
        }
        return controllers.publishSensors(sn.id, tickId, sensors, {
          x: sn.x,
          y: sn.y,
          dir: sn.dir
        });
      };
      sn.sampleSensors(this, undefined, deliver);
    }
  }
  /**
   * Selects an initial focus snake when a generation starts or when
   * switching view modes.
   */
  _chooseInitialFocus(): void {
    const alive = this.snakes.filter(s => s.alive);
    if (alive.length) {
      const idx = this.observerRng.int(alive.length);
      this.focusSnake = alive[idx] ?? null;
    } else {
      this.focusSnake = null;
    }
    if (this.viewMode === "follow" && this.focusSnake) {
      const h = this.focusSnake.head();
      this.cameraX = h.x;
      this.cameraY = h.y;
    } else {
      this.cameraX = 0;
      this.cameraY = 0;
    }
    this._focusCooldown = CFG.observer.focusRecheckSeconds;
  }
  /**
   * Computes a heuristic leader score to determine which snake should be
   * followed.  Combines points, length, kills and age.
   */
  _leaderScore(s: Snake): number {
    return s.pointsScore * 3.0 + s.length() * 1.5 + s.killScore * 35.0 + s.age * 0.15;
  }
  /**
   * Periodically reevaluates which snake should be the focus.  Uses
   * hysteresis to avoid rapid switching.
   * @param dt - Delta time in seconds.
   */
  _updateFocus(dt: number): void {
    this._focusCooldown -= dt;
    if (!this.focusSnake || !this.focusSnake.alive) {
      this.focusSnake = null;
      this._focusCooldown = 0;
    }
    if (this._focusCooldown > 0) return;
    const alive = this.snakes.filter(s => s.alive);
    if (!alive.length) {
      this.focusSnake = null;
      this._focusCooldown = CFG.observer.focusRecheckSeconds;
      return;
    }
    let best = alive[0]!;
    let bestScore = this._leaderScore(best);
    for (let i = 1; i < alive.length; i++) {
      const s = alive[i];
      if (!s) continue;
      const sc = this._leaderScore(s);
      if (sc > bestScore) {
        best = s;
        bestScore = sc;
      }
    }
    if (!this.focusSnake) this.focusSnake = best;
    else {
      const cur = this.focusSnake;
      const curScore = this._leaderScore(cur);
      if (best !== cur && bestScore > curScore * CFG.observer.focusSwitchMargin) this.focusSnake = best;
    }
    this._focusCooldown = CFG.observer.focusRecheckSeconds;
  }
  /**
   * Updates camera position and zoom based on view mode and focused snake.
   * 
   * This method manages three distinct UI states:
   * 1. "Overview": Centers the world and scales zoom to fit the arena boundary (+ margin) within the viewport.
   * 2. "Follow": Focuses on the head of a specific snake and adjusts zoom dynamically based on its length (zoom out as it grows).
   * 3. "Idle/Fallback": Slowly drifts and centers when no valid focus is found.
   * 
   * @param viewW - Viewport width in pixels.
   * @param viewH - Viewport height in pixels.
   */
  _updateCamera(viewW: number, viewH: number): void {
    // Basic normalization for invalid viewport dimensions (e.g., initial load or worker state lag).
    // We default to a square that comfortably fits the entire world.
    if (!Number.isFinite(viewW) || !Number.isFinite(viewH) || viewW <= 0 || viewH <= 0) {
      const DEFAULT_NORMALIZED_DIM = CFG.worldRadius * 2;
      viewW = DEFAULT_NORMALIZED_DIM;
      viewH = DEFAULT_NORMALIZED_DIM;
    }

    if (this.viewMode === "overview") {
      this.cameraX = 0;
      this.cameraY = 0;

      // The Overview fitting calculates a 'fit' scale factor that ensures the arena (CFG.worldRadius)
      // plus an extra safety margin (overviewExtraWorldMargin) is fully visible.
      // We standardize to the smallest dimension (min(viewW, viewH)) to ensure the arena 
      // fits regardless of the window's aspect ratio.
      // We also apply internal padding (overviewPadding) to prevent objects from touching edges.
      const effectiveR = CFG.worldRadius + CFG.observer.overviewExtraWorldMargin;
      const fit = Math.min(viewW, viewH) / (2 * effectiveR * CFG.observer.overviewPadding);
      const targetZoom = clamp(fit, 0.01, 2.0);

      // Snap zoom to target if it is at default (1.0) to avoid an unnecessary "zoom glide" 
      // when the simulation first loads or when explicitly enabled via config.
      if (this.zoom === 1.0 || (CFG.observer.snapZoomOutInOverview && this.zoom > targetZoom)) {
        this.zoom = targetZoom;
      } else {
        // Smoothly interpolate towards the target zoom to provide visual continuity.
        this.zoom = lerp(this.zoom, targetZoom, CFG.observer.zoomLerpOverview);
      }
      return;
    }

    if (this.focusSnake && this.focusSnake.alive) {
      const h = this.focusSnake.head();
      this.cameraX = h.x;
      this.cameraY = h.y;

      // Follow Zoom Logic:
      // Larger snakes require a wider FOV (lower zoom) to keep their perspective manageable.
      // We map the snake's length to a zoom range [0.45, 1.12]. This "comfort corridor" 
      // ensures that even at maximum length, the snake head and its immediate surroundings 
      // remain clearly visible without the world feeling too small.
      const focusLen = this.focusSnake.length();
      const MAX_ZOOM = 1.15;
      const MIN_ZOOM = 0.45;
      const ZOOM_RANGE = 0.55;
      const targetZoom = clamp(MAX_ZOOM - (focusLen / Math.max(1, CFG.snakeMaxLen)) * ZOOM_RANGE, MIN_ZOOM, 1.12);

      this.zoom = lerp(this.zoom, targetZoom, CFG.observer.zoomLerpFollow);
    } else {
      // Fallback: Return to center with a default zoom level when focus is lost.
      this.cameraX = 0;
      this.cameraY = 0;
      const FALLBACK_ZOOM = 0.95;
      const FALLBACK_LERP = 0.05;
      this.zoom = lerp(this.zoom, FALLBACK_ZOOM, FALLBACK_LERP);
    }
  }
  /**
   * Resolves collisions by querying the segment grid around each head and
   * killing snakes that intersect another snake’s body.  Awards kill
   * points to the aggressor.
   */
  _resolveCollisionsGrid(): void {
    const cellSize = Math.max(1, CFG.collision.cellSize);
    const hitScale = CFG.collision.hitScale;
    for (const s of this.snakes) {
      if (!s.alive) continue;

      // Head point
      const hx = s.x;
      const hy = s.y;

      let collision = false;
      let killedBy: Snake | null = null;

      const checkNeighbor = (otherS: Snake, idx: number) => {
        if (collision) return; // Already found a collision for this snake

        if (otherS === s) return;
        if (!otherS || !otherS.alive) return; // Guard against empty grid entries

        const p = otherS.points;
        if (idx >= p.length || idx <= 0) return; // Ensure valid segment indices

        const p0 = p[idx - 1];
        const p1 = p[idx];
        if (!p0 || !p1) return;
        const dist2 = pointSegmentDist2(hx, hy, p0.x, p0.y, p1.x, p1.y);
        // Effective radius
        const thr = (s.radius + otherS.radius) * hitScale;
        if (dist2 <= thr * thr) {
          collision = true;
          killedBy = otherS;
        }
      };

      // Query local and neighbor cells
      const cx = Math.floor(hx / cellSize);
      const cy = Math.floor(hy / cellSize);
      // Query current cell and 8 neighbors
      for (let oy = -1; oy <= 1; oy++) {
        for (let ox = -1; ox <= 1; ox++) {
          this._collGrid.queryCell(cx + ox, cy + oy, checkNeighbor);
          if (collision) break; // Stop querying if collision found
        }
        if (collision) break;
      }

      if (collision) {
        s.die(this);
        if (killedBy && killedBy !== s) { // Award points only if killed by another snake
          const killer = killedBy as Snake;
          killer.killScore += 1;
          killer.pointsScore += CFG.reward.pointsPerKill;
        }
      }
    }
  }
  /**
   * Ends the current generation: computes fitness scores, selects elites
   * and breeds new genomes via tournament selection, crossover and
   * mutation.  Resets state for the new generation.
   */
  _endGeneration(simulationStep = this.tickId): void {
    if (!this.population.length) return;
    const populationSnakes = this.snakes.slice(0, this.population.length);
    let maxPts = 0;
    for (const s of populationSnakes) if (s) maxPts = Math.max(maxPts, s.pointsScore);
    if (maxPts <= 0) maxPts = 1;
    const logDen = Math.log(1 + maxPts);
    const topIds = new Set();
    for (const s of populationSnakes) if (s && Math.abs(s.pointsScore - maxPts) <= 1e-6) topIds.add(s.id);
    for (let i = 0; i < this.population.length; i++) {
      const s = populationSnakes[i];
      const pop = this.population[i];
      if (!s || !pop) continue;
      const pointsNorm = clamp(Math.log(1 + s.pointsScore) / logDen, 0, 1);
      const topBonus = topIds.has(s.id) ? CFG.reward.fitnessTopPointsBonus : 0;
      const fit = s.computeFitness(pointsNorm, topBonus);
      pop.fitness = fit;
      s.fitness = fit; // Store on snake for HoF retrieval
      if (fit > this.bestFitnessEver) this.bestFitnessEver = fit;
    }
    this.population.sort((a, b) => b.fitness - a.fitness);

    // Record history
    const avgFit = this.population.reduce((sum, g) => sum + g.fitness, 0) / this.population.length;
    const minFit = this.population[this.population.length - 1]?.fitness ?? 0;
    const diversity = computeSpeciesStats(this.population);
    const complexity = computeNetworkStats(this.population);
    const bestGenome = this.population[0];
    if (!bestGenome) return;
    this.fitnessHistory.push({
      gen: this.generation,
      best: bestGenome.fitness,
      avg: avgFit,
      min: minFit,
      speciesCount: diversity.speciesCount,
      topSpeciesSize: diversity.topSpeciesSize,
      avgWeight: complexity.avgWeight,
      weightVariance: complexity.weightVariance
    });
    if (this.fitnessHistory.length > 100) this.fitnessHistory.shift();

    // Hall of Fame: Record the best snake of this generation
    const bestG = bestGenome;
    // Find the actual snake object to get its length/kill stats, as genome doesn't have them
    // The population is sorted by fitness, so population[0] is the best genome.
    // However, the snakes array might not match population order unless we track IDs.
    // Easier: find the snake with the best fitness.
    let bestS: Snake | null = null;
    let maxFit = -1;
    for (const s of populationSnakes) {
      const fit = s.fitness ?? -Infinity;
      if (fit > maxFit) {
        maxFit = fit;
        bestS = s;
      }
    }
    // Fallback if fitness wasn't stored on snake yet (it is computed in this function)
    if (!bestS && this.snakes.length > 0) bestS = this.snakes[0] ?? null; // Should rarely happen

    if (bestS) {
      const hofEntry = {
        gen: this.generation,
        seed: bestS.id, // Using ID as a proxy for 'seed' or unique identifier
        fitness: bestS.fitness ?? 0, // Ensure fitness is set
        points: bestS.pointsScore,
        length: bestS.length(),
        genome: bestG.toJSON() // Persist the genome data
      };
      void hof.add(hofEntry);
      this._lastHoFEntry = hofEntry;
    }

    const eliteN = Math.max(1, Math.floor(CFG.eliteFrac * this.population.length));
    const elites = this.population.slice(0, eliteN).map(g => g.clone());
    const newPop = [];
    for (let i = 0; i < eliteN; i++) {
      const elite = elites[i];
      if (elite) newPop.push(elite.clone());
    }
    while (newPop.length < this.population.length) {
      const parentA = tournamentPick(this.population, 5, this.evolutionRng);
      const parentB = tournamentPick(this.population, 5, this.evolutionRng);
      const child = crossover(parentA, parentB, this.arch, this.evolutionRng.asSource());
      mutate(child, this.arch, this.evolutionRng);
      child.fitness = 0;
      newPop.push(child);
    }
    this.population = newPop;
    this.generation += 1;
    this.generationTime = 0;
    this.bestPointsThisGen = 0;
    this.bestPointsSnakeId = 0;
    this.particles = new ParticleSystem(); // Reset particles
    this._clearTransientGenerationState();
    this._resetBaselineBotsForGen();
    this._emitGenerationBoundary('generation', simulationStep);
    this._spawnAll();
    this._initPellets();
    this._collGrid.build(this.snakes, CFG.collision.skipSegments);
    this._chooseInitialFocus();
  }

  /**
   * Allocate a deterministic collision-safe Hall-of-Fame snake identifier.
   * @returns Unique safe integer id.
   */
  private _allocateResurrectedSnakeId(): number {
    let candidate = this._nextResurrectedSnakeId;
    while (this.snakes.some(snake => snake.id === candidate)) {
      candidate += 1;
      if (!Number.isSafeInteger(candidate)) throw new RangeError('Hall-of-Fame snake id allocator exhausted');
    }
    if (candidate >= Number.MAX_SAFE_INTEGER) {
      throw new RangeError('Hall-of-Fame snake id allocator exhausted');
    }
    this._nextResurrectedSnakeId = candidate + 1;
    return candidate;
  }

  /**
   * Spawns a snake from a saved genome immediately into the world.
   * @param genomeJSON - Serialized genome to resurrect.
   * @returns Newly spawned snake id.
   */
  resurrect(genomeJSON: GenomeJSON): number {
    const genome = Genome.fromJSON(genomeJSON);
    const id = this._allocateResurrectedSnakeId();
    const snake = new Snake(id, genome, this.arch, {
      skin: 1,
      populationSlot: null,
      inferenceBackend: this.inferenceBackend,
      rng: this.worldRng.asSource()
    });

    // Give it a distinct look (e.g. golden glow) if possible, or just standard
    snake.color = '#FFD700'; // Gold color to signify HoF status

    this.snakes.push(snake);
    this.focusSnake = snake; // Auto-focus the resurrected snake
    this.viewMode = 'follow';
    this.zoom = 1.0;
    return id;
  }

  /**
   * Spawns a new externally controlled snake with a fresh genome.
   * Reuses dead external slots to avoid unbounded growth.
   */
  spawnExternalSnake(): Snake {
    const genome = Genome.random(this.arch, this.evolutionRng.asSource());
    const reusableIndex = this.snakes.findIndex(
      (snake) => !snake.alive && snake.id >= EXTERNAL_SNAKE_ID_START && snake.baselineBotIndex == null
    );
    if (reusableIndex >= 0) {
      const existingId = this.snakes[reusableIndex]!.id;
      const snake = new Snake(existingId, genome, this.arch, {
        populationSlot: null,
        inferenceBackend: this.inferenceBackend,
        rng: this.worldRng.asSource()
      });
      this.snakes[reusableIndex] = snake;
      return snake;
    }
    const id = this._nextExternalSnakeId++;
    const snake = new Snake(id, genome, this.arch, {
      populationSlot: null,
      inferenceBackend: this.inferenceBackend,
      rng: this.worldRng.asSource()
    });
    this.snakes.push(snake);
    return snake;
  }

  /**
   * Generates a new ambient pellet using a fractal noise rejection algorithm.
   * 
   * Performance & Distribution:
   * This algorithm creates "Fractal Food" patterns where pellets form filaments and 
   * clusters rather than a uniform distribution. This encourages organic movement 
   * and strategic clustering behavior in the snakes.
   * 
   * Approach:
   * 1. We use "Interference Noise" by summing overlapping sinusoidal waves of varying 
   *    frequencies and phases. This creates complex patterns of "peaks" (filaments) 
   *    and "valleys" (voids) without the overhead of Perlin noise.
   * 2. Rejection Sampling: We pick a random spot and check the local noise density. 
   *    Higher density spots are more likely to spawn pellets, concentrating food 
   *    into strategic clusters that encourage movement and conflict.
   * 3. Fallback: If several attempts fail to meet the density criteria, we spawn 
   *    uniformly to ensure food density doesn't drop too low in "void" regions.
   * 
   * @returns A new Pellet instance.
   */
  _spawnAmbientPellet(): Pellet {
    const r = CFG.worldRadius;
    const TIME_DRIFT_SCALE = 0.04;
    const t = this.generationTime * TIME_DRIFT_SCALE;

    const foodCfg = CFG.foodSpawn ?? {};
    // Domain warp to keep filaments from aligning to a rigid grid.
    const WARP_FREQ = Math.max(0, foodCfg.warpFreq ?? 0.0013);
    const WARP_SCALE = Math.max(0, (foodCfg.warpScale ?? 0.08) * r);

    // Filament noise frequencies (feature scales).
    const FREQ_LARGE = Math.max(0, foodCfg.freqLarge ?? 0.0026);
    const FREQ_MEDIUM = Math.max(0, foodCfg.freqMedium ?? 0.0042);
    const FREQ_SMALL = Math.max(0, foodCfg.freqSmall ?? 0.0068);

    // Rejection Sampling Loop:
    // We attempt to find a location that satisfies our noise-based density requirements.
    const REJECTION_RETRIES = 8;
    const FILAMENT_POWER = Math.max(0.1, foodCfg.filamentPower ?? 4.2);
    const DUST_STRENGTH = clamp(foodCfg.dustStrength ?? 0.35, 0, 1);
    const EDGE_FADE_START = clamp(foodCfg.edgeFadeStart ?? 0.35, 0, 0.95);
    const EDGE_FADE_POWER = Math.max(0.1, foodCfg.edgeFadePower ?? 2.6);
    const edgeFalloffEnabled = foodCfg.edgeFalloffEnabled ?? true;
    let bestProb = -1;
    let bestX = 0;
    let bestY = 0;

    for (let i = 0; i < REJECTION_RETRIES; i++) {
      // Phase 1: Pick a random candidate point within the circular world.
      const a = this.worldRng.next() * TAU;
      const d = Math.sqrt(this.worldRng.next()) * r;
      const x = Math.cos(a) * d;
      const y = Math.sin(a) * d;

      // Phase 2: Domain warp + ridged interference for filament bands.
      const warpX = Math.sin(y * WARP_FREQ + t * 0.7) * WARP_SCALE
        + Math.cos(x * WARP_FREQ * 1.25 - t * 0.4) * (WARP_SCALE * 0.6);
      const warpY = Math.cos(x * WARP_FREQ - t * 0.5) * WARP_SCALE
        + Math.sin(y * WARP_FREQ * 1.1 + t * 0.8) * (WARP_SCALE * 0.6);
      const xw = x + warpX;
      const yw = y + warpY;

      const n1 = Math.sin((xw + yw) * FREQ_LARGE + t)
        + Math.cos((xw - yw) * FREQ_LARGE - t * 0.7);
      const n2 = Math.sin(xw * FREQ_MEDIUM - t * 1.1)
        + Math.cos(yw * FREQ_MEDIUM + t * 0.9);
      const n3 = Math.sin(xw * FREQ_SMALL + t * 1.6)
        * Math.cos(yw * FREQ_SMALL - t * 1.3);

      const ridgeA = clamp(1 - Math.abs(n1) / 2, 0, 1);
      const ridgeB = clamp(1 - Math.abs(n2) / 2, 0, 1);
      const ridgeC = clamp(1 - Math.abs(n3), 0, 1);

      const webA = Math.pow(ridgeA, FILAMENT_POWER);
      const webB = Math.pow(ridgeB, FILAMENT_POWER * 0.95);
      const dust = Math.pow(ridgeC, 2.2) * DUST_STRENGTH;
      let prob = clamp(Math.max(webA, webB) + dust, 0, 1);

      if (edgeFalloffEnabled) {
        const edgeT = d / r;
        const edgeRamp = clamp((edgeT - EDGE_FADE_START) / (1 - EDGE_FADE_START), 0, 1);
        const edgeSmooth = edgeRamp * edgeRamp * (3 - 2 * edgeRamp);
        const edgeFalloff = clamp(1 - Math.pow(edgeSmooth, EDGE_FADE_POWER), 0, 1);
        prob *= edgeFalloff;
      }

      if (prob > bestProb) {
        bestProb = prob;
        bestX = x;
        bestY = y;
      }

      if (this.worldRng.next() < prob) {
        return new Pellet(x, y, CFG.foodValue, null, "ambient", 0);
      }
    }

    // Phase 4: Fallback Distribution
    // If rejection sampling fails after all attempts, pick the best candidate
    // to keep the voids expansive while still guaranteeing a pellet.
    return new Pellet(bestX, bestY, CFG.foodValue, null, "ambient", 0);
  }
}

/**
 * Selects a genome by k‑tournament selection: chooses k random candidates
 * from the population and returns the fittest among them.  Used for
 * breeding new individuals.
 * @param pop - Candidate population.
 * @param k - Tournament size.
 * @param rng - Evolution random stream.
 */
function tournamentPick(pop: Genome[], k: number, rng: RandomGenerator): Genome {
  let best: Genome | null = null;
  for (let i = 0; i < k; i++) {
    const g = pop[rng.int(pop.length)] ?? pop[0]!;
    if (!best || g.fitness > best.fitness) best = g;
  }
  return best!;
}

/** Spatial hash for pellets to support fast local queries for sensing and eating. */
class PelletGrid {
  /** Grid cell size in world units. */
  cellSize: number;
  /** Map of cell keys to pellets in that cell. */
  map: Map<string, Pellet[]>;

  constructor() {
    this.cellSize = Math.max(10, CFG.pelletGrid?.cellSize ?? 120);
    this.map = new Map();
  }
  /** Reset the grid sizing based on the current CFG. */
  resetForCFG(): void {
    this.cellSize = Math.max(10, CFG.pelletGrid?.cellSize ?? 120);
    this.map.clear();
  }
  /**
   * Build the key for a cell coordinate.
   * @param cx - Cell x coordinate.
   * @param cy - Cell y coordinate.
   * @returns Map key string.
   */
  _key(cx: number, cy: number): string {
    return cx + "," + cy;
  }
  /**
   * Convert world coordinates to cell coordinates.
   * @param x - World x position.
   * @param y - World y position.
   * @returns Cell coordinate object.
   */
  _coords(x: number, y: number): { cx: number; cy: number } {
    return { cx: Math.floor(x / this.cellSize), cy: Math.floor(y / this.cellSize) };
  }
  /**
   * Add a pellet to the spatial hash.
   * @param p - Pellet to add.
   */
  add(p: Pellet): void {
    const { cx, cy } = this._coords(p.x, p.y);
    const k = this._key(cx, cy);
    let arr = this.map.get(k);
    if (!arr) {
      arr = [];
      this.map.set(k, arr);
    }
    p._pcx = cx;
    p._pcy = cy;
    p._pkey = k;
    p._cellArr = arr;
    p._cellIndex = arr.length;
    arr.push(p);
  }
  /**
   * Remove a pellet from the spatial hash.
   * @param p - Pellet to remove.
   */
  remove(p: Pellet): void {
    const arr = p._cellArr;
    if (!arr) return;
    const idx = p._cellIndex!;
    const last = arr.pop()!;
    if (last !== p) {
      arr[idx] = last;
      last._cellIndex = idx;
      last._cellArr = arr;
    }
    p._cellArr = null;
    p._cellIndex = -1;
    if (arr.length === 0 && p._pkey) {
      // Safe even if already deleted.
      this.map.delete(p._pkey);
    }
  }
  /**
   * Iterate pellets in cells intersecting a radius around (x,y).
   * @param x - World x position.
   * @param y - World y position.
   * @param r - Query radius.
   * @param fn - Callback invoked for each pellet.
   */
  forEachInRadius(x: number, y: number, r: number, fn: (p: Pellet) => void): void {
    const cs = this.cellSize;
    const minCx = Math.floor((x - r) / cs);
    const maxCx = Math.floor((x + r) / cs);
    const minCy = Math.floor((y - r) / cs);
    const maxCy = Math.floor((y + r) / cs);
    for (let cy = minCy; cy <= maxCy; cy++) {
      for (let cx = minCx; cx <= maxCx; cx++) {
        const arr = this.map.get(this._key(cx, cy));
        if (!arr) continue;
        for (let i = 0; i < arr.length; i++) {
          const pellet = arr[i];
          if (pellet) fn(pellet);
        }
      }
    }
  }
}

/** Distance threshold for species bucketing. */
const SPECIES_DISTANCE_THRESHOLD = 0.35;

/**
 * Compute RMS distance between two genomes.
 * @param a - Genome A.
 * @param b - Genome B.
 * @returns RMS distance or Infinity when incompatible.
 */
function genomeDistanceRms(a: Genome, b: Genome): number {
  const wa = a.weights;
  const wb = b.weights;
  if (!wa || !wb || wa.length !== wb.length) return Infinity;
  let sumSq = 0;
  for (let i = 0; i < wa.length; i++) {
    const d = (wa[i] ?? 0) - (wb[i] ?? 0);
    sumSq += d * d;
  }
  return Math.sqrt(sumSq / wa.length);
}

/**
 * Compute species count and top species size for a population.
 * @param population - Genomes to analyze.
 * @returns Species statistics summary.
 */
function computeSpeciesStats(population: Genome[]): { speciesCount: number; topSpeciesSize: number } {
  if (!population || population.length === 0) {
    return { speciesCount: 0, topSpeciesSize: 0 };
  }
  const species: Array<{ rep: Genome; size: number }> = [];
  for (const genome of population) {
    let assigned = false;
    for (const bucket of species) {
      if (genomeDistanceRms(genome, bucket.rep) <= SPECIES_DISTANCE_THRESHOLD) {
        bucket.size += 1;
        assigned = true;
        break;
      }
    }
    if (!assigned) {
      species.push({ rep: genome, size: 1 });
    }
  }
  let topSize = 0;
  for (const bucket of species) topSize = Math.max(topSize, bucket.size);
  return { speciesCount: species.length, topSpeciesSize: topSize };
}

/**
 * Compute weight statistics across a population.
 * @param population - Genomes to analyze.
 * @returns Network weight summary.
 */
function computeNetworkStats(population: Genome[]): { avgWeight: number; weightVariance: number } {
  if (!population || population.length === 0) {
    return { avgWeight: 0, weightVariance: 0 };
  }
  let sumAbs = 0;
  let sumAbsSq = 0;
  let count = 0;
  for (const genome of population) {
    const w = genome.weights;
    if (!w) continue;
    for (let i = 0; i < w.length; i++) {
      const aw = Math.abs(w[i] ?? 0);
      sumAbs += aw;
      sumAbsSq += aw * aw;
      count += 1;
    }
  }
  if (!count) return { avgWeight: 0, weightVariance: 0 };
  const avgWeight = sumAbs / count;
  const weightVariance = Math.max(0, sumAbsSq / count - avgWeight * avgWeight);
  return { avgWeight, weightVariance };
}
