/**
 * Unified Simulation Core.
 *
 * This class acts as the platform-agnostic engine for the Slither Neuroevolution simulation.
 * It owns the World instance, manages the fixed-timestep physics loop, and generates standardized
 * statistics. It is designed to be wrapped by both the Server (Node.js) and the Client (Browser/Worker).
 */

import {
  World,
  type BatchInferenceRunner,
  type ControllerRegistryLike,
  type GenerationBoundaryHook
} from '../world.ts';
import { WorldSerializer } from '../serializer.ts';
import { CFG } from '../config.ts';
import { normalizeSeed } from '../rng.ts';
import type { CoreSettings } from '../protocol/settings.ts';
import type { Snake } from '../snake.ts';
import type { InferenceBackend } from '../brains/types.ts';
import type {
  FitnessData,
  FitnessHistoryEntry,
  HallOfFameEntry,
  VizData
} from '../protocol/messages.ts';

/** Default hard limit for complete fixed steps executed by one scheduler pump. */
const DEFAULT_MAX_STEPS_PER_PUMP = 120;
/** Monotonic process-local identity source that never consumes simulation RNG. */
let nextLocalRunOrdinal = 1;

/** Public identity for one in-memory evolutionary lineage. */
export interface SimulationRunIdentity {
  /** Normalized active run seed. */
  seed: number;
  /** Opaque lineage identifier independent from simulation RNG. */
  runId: string;
}

/** Optional identity overrides accepted by a same-seed Reset. */
export interface SimCoreResetOptions {
  /** Fresh lineage id; a process-local id is created when omitted. */
  runId?: string;
}

/** Required identity inputs accepted by an explicit New Run. */
export interface SimCoreNewRunOptions {
  /** New root seed supplied by a system-entropy owner. */
  seed: number;
  /** Fresh lineage id; a process-local id is created when omitted. */
  runId?: string;
}

/**
 * Create a process-local run id without reading an authoritative random stream.
 * @param seed - Normalized lineage seed included only for diagnostics.
 * @returns Fresh process-local run id.
 */
function createLocalRunId(seed: number): string {
  const ordinal = nextLocalRunOrdinal++;
  return `local-${seed.toString(16).padStart(8, '0')}-${ordinal.toString(36)}`;
}

/**
 * Normalize a supplied run id or create an independent process-local fallback.
 * @param runId - Optional externally generated id.
 * @param seed - Normalized lineage seed used by the fallback.
 * @returns Non-empty run id.
 */
function normalizeRunId(runId: string | undefined, seed: number): string {
  const normalized = runId?.trim();
  return normalized || createLocalRunId(seed);
}

/** Operational scheduler measurements that never feed authoritative World state. */
export interface SchedulerDiagnostics {
  /** Current requested simulation-time multiplier. */
  requestedMultiplier: number;
  /** Cumulative completed simulation seconds divided by observed wall seconds. */
  achievedMultiplier: number;
  /** Total successfully completed fixed steps. */
  completedSteps: number;
  /** Successfully completed fixed steps in the latest pump. */
  completedStepsThisPump: number;
  /** Cumulative finite non-negative wall time supplied to the scheduler. */
  wallSeconds: number;
  /** Cumulative authoritative simulation time completed by fixed steps. */
  simulatedSeconds: number;
  /** Cumulative simulation-time debt discarded by the pump cap. */
  droppedSimulationSeconds: number;
  /** Simulation-time debt discarded in the latest pump. */
  droppedSimulationSecondsThisPump: number;
  /** Fractional scheduled simulation time retained for a future whole step. */
  pendingSimulationSeconds: number;
  /** Configured upper bound on complete steps per pump. */
  maxStepsPerPump: number;
}

/** Standardized experiment statistics produced by SimCore. */
export interface CoreStats {
  tick: number;
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

/** Configuration options for SimCore. */
export interface SimCoreOptions {
  /** Initial world settings. */
  settings?: Partial<CoreSettings>;
  /** Optional hash of the current config (for server/persistence). */
  cfgHash?: string;
  /** Optional world seed. */
  worldSeed?: number;
  /** Optional lineage id generated independently from simulation randomness. */
  runId?: string;
  /** Immutable neural math backend prepared before World construction. */
  inferenceBackend?: InferenceBackend;
  /** Optional exact pre-spawn generation-boundary observer. */
  onGenerationBoundary?: GenerationBoundaryHook;
  /** Optional brain pool for batch inference. */
  brainPool?: BatchInferenceRunner | null;
  /** Tick rate in Hz (simulation updates per second). */
  tickRateHz?: number;
  /** Maximum complete fixed steps allowed in one scheduler pump. */
  maxStepsPerPump?: number;
}

/**
 * The Core Simulation Engine.
 */
export class SimCore {
  /** The physics world. */
  world: World;

  /** Normalized active run seed. */
  worldSeed: number;

  /** Opaque evolutionary-lineage identifier. */
  runId: string;

  /** Immutable neural math backend preserved across run reconstruction. */
  readonly inferenceBackend: InferenceBackend;

  /** Boundary observer preserved across Reset and New Run reconstruction. */
  private generationBoundaryHook: GenerationBoundaryHook | null;

  /** Current tick ID. */
  tickId: number = 0;

  /** Accumulator for fixed-timestep logic. */
  accumulator: number = 0;

  /** Fixed delta time step (default 60Hz). */
  fixedDt: number = 1 / 60;

  /** Hard upper bound on complete fixed steps in one scheduler pump. */
  maxStepsPerPump: number = DEFAULT_MAX_STEPS_PER_PUMP;

  /** Cumulative finite non-negative wall time supplied to the scheduler. */
  private totalWallSeconds = 0;

  /** Cumulative authoritative simulation time completed by fixed steps. */
  private totalSimulatedSeconds = 0;

  /** Cumulative simulation-time debt discarded by the pump cap. */
  private totalDroppedSimulationSeconds = 0;

  /** Complete fixed steps committed by the latest scheduler pump. */
  private completedStepsThisPump = 0;

  /** Simulation-time debt discarded by the latest scheduler pump. */
  private droppedSimulationSecondsThisPump = 0;

  /** Last calculated FPS. */
  fps: number = 0;

  /** Current view dimensions (for serialization culling). */
  viewW: number = 0;
  viewH: number = 0;

  /** Optional Brain Pool for parallel inference. */
  brainPool: BatchInferenceRunner | null = null;

  /** Last known fitness history length (for incremental stats updates). */
  lastHistoryLen: number = 0;

  /** Last generation (for HoF/Snapshot tracking). */
  lastGeneration: number = 0;

  /** Callbacks for visualization viz data picking (optional). */
  onVizSnakePick: (() => Snake | null) | null = null;

  constructor(options: SimCoreOptions = {}) {
    // 1. Initialize Settings
    // The world is initialized with the provided settings.

    // 2. Create World
    this.worldSeed = normalizeSeed(options.worldSeed ?? 0);
    this.runId = normalizeRunId(options.runId, this.worldSeed);
    this.inferenceBackend = options.inferenceBackend ?? 'js';
    this.generationBoundaryHook = options.onGenerationBoundary ?? null;
    this.world = new World(options.settings || {}, {
      seed: this.worldSeed,
      inferenceBackend: this.inferenceBackend,
      ...(this.generationBoundaryHook
        ? { onGenerationBoundary: this.generationBoundaryHook }
        : {})
    });
    this.brainPool = options.brainPool || null;
    const tickRateHz = options.tickRateHz;
    if (typeof tickRateHz === 'number' && Number.isFinite(tickRateHz) && tickRateHz > 0) {
      this.fixedDt = 1 / tickRateHz;
    }
    const maxStepsPerPump = options.maxStepsPerPump;
    if (
      typeof maxStepsPerPump === 'number' &&
      Number.isFinite(maxStepsPerPump) &&
      maxStepsPerPump > 0
    ) {
      this.maxStepsPerPump = Math.max(1, Math.floor(maxStepsPerPump));
    }

    this.viewW = CFG.worldRadius * 2;
    this.viewH = CFG.worldRadius * 2; // Default to full world view
    this.lastGeneration = this.world.generation;
    this.lastHistoryLen = this.world.fitnessHistory.length;
  }

  /**
   * Convert elapsed wall time into zero or more complete fixed World steps.
   * The measured delta controls scheduling only; every authoritative step
   * receives exactly `fixedDt` regardless of speed or pump grouping.
   *
   * @param wallDt - Measured wall time since the previous pump, in seconds.
   * @param controllerProvider - Optional provider for snake controllers.
   * @returns Number of complete fixed steps committed by this pump.
   */
  async update(
    wallDt: number,
    controllerProvider?: ControllerRegistryLike
  ): Promise<number> {
    const elapsed = Number.isFinite(wallDt) && wallDt > 0 ? wallDt : 0;
    const requestedMultiplier = Number.isFinite(this.world.simSpeed)
      ? Math.max(0, this.world.simSpeed)
      : 0;
    this.fps = elapsed > 0 ? 1 / elapsed : 0;
    this.totalWallSeconds += elapsed;
    this.accumulator += elapsed * requestedMultiplier;
    this.completedStepsThisPump = 0;
    this.droppedSimulationSecondsThisPump = 0;

    const roundingAllowance = this.fixedDt * 1e-9;
    const dueSteps = Math.max(
      0,
      Math.floor((this.accumulator + roundingAllowance) / this.fixedDt)
    );
    const droppedSteps = Math.max(0, dueSteps - this.maxStepsPerPump);
    if (droppedSteps > 0) {
      const droppedSeconds = droppedSteps * this.fixedDt;
      this.accumulator = Math.max(0, this.accumulator - droppedSeconds);
      this.droppedSimulationSecondsThisPump = droppedSeconds;
      this.totalDroppedSimulationSeconds += droppedSeconds;
    }

    const stepsToRun = Math.min(dueSteps, this.maxStepsPerPump);
    try {
      for (let stepIndex = 0; stepIndex < stepsToRun; stepIndex++) {
        const nextTickId = this.tickId + 1;
        await this.world.step(
          this.fixedDt,
          this.viewW,
          this.viewH,
          controllerProvider,
          nextTickId,
          this.brainPool ?? undefined
        );
        this.tickId = nextTickId;
        this.accumulator = Math.max(0, this.accumulator - this.fixedDt);
        this.totalSimulatedSeconds += this.fixedDt;
        this.completedStepsThisPump += 1;
      }
    } finally {
      if (this.accumulator < roundingAllowance) this.accumulator = 0;
    }

    if (this.world.generation !== this.lastGeneration) {
      this.lastGeneration = this.world.generation;
    }
    return this.completedStepsThisPump;
  }

  /**
   * Return current operational scheduler diagnostics.
   * @returns Copy of requested/achieved speed and dropped-debt measurements.
   */
  getSchedulerDiagnostics(): SchedulerDiagnostics {
    const requestedMultiplier = Number.isFinite(this.world.simSpeed)
      ? Math.max(0, this.world.simSpeed)
      : 0;
    return {
      requestedMultiplier,
      achievedMultiplier: this.totalWallSeconds > 0
        ? this.totalSimulatedSeconds / this.totalWallSeconds
        : 0,
      completedSteps: this.tickId,
      completedStepsThisPump: this.completedStepsThisPump,
      wallSeconds: this.totalWallSeconds,
      simulatedSeconds: this.totalSimulatedSeconds,
      droppedSimulationSeconds: this.totalDroppedSimulationSeconds,
      droppedSimulationSecondsThisPump: this.droppedSimulationSecondsThisPump,
      pendingSimulationSeconds: this.accumulator,
      maxStepsPerPump: this.maxStepsPerPump
    };
  }

  /**
   * Serialize the current world state to a binary buffer.
   */
  serialize(): Float32Array {
    return WorldSerializer.serialize(this.world);
  }

  /**
   * Return the visible identity of the active in-memory lineage.
   * @returns Seed and independent run id.
   */
  getRunIdentity(): SimulationRunIdentity {
    return { seed: this.worldSeed, runId: this.runId };
  }

  /**
   * Build a statistics object for the current frame.
   * This is the single source of truth for server-side stats.
   *
   * @param includeViz - Whether to include brain visualization data.
   */
  buildStats(includeViz: boolean = false): CoreStats {
    const populationCount = this.world.population.length;
    const baselineBotsTotal = this.world.baselineBots.length;
    let alivePopulation = 0;
    let aliveTotal = 0;
    let baselineBotsAlive = 0;
    let maxFit = 0;
    let minFit = Infinity;
    let sumFit = 0;

    // Scan snakes
    for (let i = 0; i < populationCount; i++) {
      const s = this.world.snakes[i];
      if (!s || !s.alive) continue;
      alivePopulation++;
      const fit = s.pointsScore || 0;
      maxFit = Math.max(maxFit, fit);
      minFit = Math.min(minFit, fit);
      sumFit += fit;
    }

    for (const s of this.world.snakes) {
      if (s.alive) aliveTotal++;
    }

    for (const bot of this.world.baselineBots) {
      if (bot && bot.alive) baselineBotsAlive++;
    }

    if (minFit === Infinity) minFit = 0;
    const avgFit = alivePopulation > 0 ? sumFit / alivePopulation : 0;

    const stats: CoreStats = {
      tick: this.tickId,
      gen: this.world.generation,
      generationTime: this.world.generationTime,
      generationSeconds: CFG.generationSeconds,
      alive: alivePopulation,
      aliveTotal,
      baselineBotsAlive,
      baselineBotsTotal,
      fps: this.fps,
      fitnessData: {
        gen: this.world.generation,
        avgFitness: avgFit,
        maxFitness: maxFit,
        minFitness: minFit
      }
    };

    // Incremental History
    if (this.world.fitnessHistory.length !== this.lastHistoryLen) {
      stats.fitnessHistory = this.world.fitnessHistory.slice();
      this.lastHistoryLen = this.world.fitnessHistory.length;
    }

    // Visualization
    if (includeViz) {
      let vizSnake: Snake | null = null;
      if (this.onVizSnakePick) {
        vizSnake = this.onVizSnakePick();
      } else if (this.world.focusSnake && this.world.focusSnake.alive) {
        vizSnake = this.world.focusSnake;
      }

      if (vizSnake && vizSnake.brain && typeof vizSnake.brain.getVizData === 'function') {
        const viz = vizSnake.brain.getVizData();
        if (viz) stats.viz = viz;
      }
    }

    // HoF Entry
    if (this.world._lastHoFEntry) {
      stats.hofEntry = this.world._lastHoFEntry;
      this.world._lastHoFEntry = null; // Consume it
    }

    return stats;
  }

  /**
   * Rebuild all run-scoped state from a selected seed and fresh lineage id.
   * @param settings - Authoritative settings for the rebuilt generation one.
   * @param seed - Root seed for the rebuilt World.
   * @param runId - New lineage id independent from simulation randomness.
   * @returns Visible identity of the rebuilt run.
   */
  private restart(
    settings: Partial<CoreSettings>,
    seed: number,
    runId: string
  ): SimulationRunIdentity {
    const nextSeed = normalizeSeed(seed);
    const nextRunId = normalizeRunId(runId, nextSeed);
    const nextWorld = new World(settings, {
      seed: nextSeed,
      inferenceBackend: this.inferenceBackend,
      ...(this.generationBoundaryHook
        ? { onGenerationBoundary: this.generationBoundaryHook }
        : {})
    });
    this.worldSeed = nextSeed;
    this.runId = nextRunId;
    this.world = nextWorld;
    this.tickId = 0;
    this.accumulator = 0;
    this.totalWallSeconds = 0;
    this.totalSimulatedSeconds = 0;
    this.totalDroppedSimulationSeconds = 0;
    this.completedStepsThisPump = 0;
    this.droppedSimulationSecondsThisPump = 0;
    this.lastGeneration = this.world.generation;
    this.lastHistoryLen = this.world.fitnessHistory.length;
    return this.getRunIdentity();
  }

  /**
   * Apply/Reset to generation one using the same seed and a new run id.
   * @param settings - Authoritative settings for the rebuilt generation one.
   * @param options - Optional independently generated lineage id.
   * @returns Visible identity of the rebuilt run.
   */
  reset(
    settings: Partial<CoreSettings>,
    options: SimCoreResetOptions = {}
  ): SimulationRunIdentity {
    return this.restart(
      settings,
      this.worldSeed,
      normalizeRunId(options.runId, this.worldSeed)
    );
  }

  /**
   * Start generation one with a new externally selected seed and run id.
   * @param settings - Authoritative settings for the rebuilt generation one.
   * @param options - New seed and optional independently generated lineage id.
   * @returns Visible identity of the rebuilt run.
   */
  newRun(
    settings: Partial<CoreSettings>,
    options: SimCoreNewRunOptions
  ): SimulationRunIdentity {
    const seed = normalizeSeed(options.seed);
    return this.restart(settings, seed, normalizeRunId(options.runId, seed));
  }
}
