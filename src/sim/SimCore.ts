/**
 * Unified Simulation Core.
 *
 * This class acts as the platform-agnostic engine for the Slither Neuroevolution simulation.
 * It owns the World instance, manages the fixed-timestep physics loop, and generates standardized
 * statistics. It is designed to be wrapped by both the Server (Node.js) and the Client (Browser/Worker).
 */

import { World, type BatchInferenceRunner, type ControllerRegistryLike } from '../world.ts';
import { WorldSerializer } from '../serializer.ts';
import { CFG } from '../config.ts';
import type { CoreSettings } from '../protocol/settings.ts';
import type { Snake } from '../snake.ts';
import type {
  FitnessData,
  FitnessHistoryEntry,
  HallOfFameEntry,
  VizData
} from '../protocol/messages.ts';

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
  /** Optional brain pool for batch inference. */
  brainPool?: BatchInferenceRunner | null;
  /** Tick rate in Hz (simulation updates per second). */
  tickRateHz?: number;
}

/**
 * The Core Simulation Engine.
 */
export class SimCore {
  /** The physics world. */
  world: World;

  /** Current tick ID. */
  tickId: number = 0;

  /** Accumulator for fixed-timestep logic. */
  accumulator: number = 0;

  /** Fixed delta time step (default 60Hz). */
  fixedDt: number = 1 / 60;

  /** Last calculated FPS. */
  fps: number = 0;

  /** Current view dimensions (for serialization culling). */
  viewW: number = 0;
  viewH: number = 0;

  /** Optional Brain Pool for parallel inference. */
  brainPool: BatchInferenceRunner | null = null;
  /** Whether batch inference is active/enabled. */
  batchEnabled: boolean = true;

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
    this.world = new World(options.settings || {});
    this.brainPool = options.brainPool || null;
    if (options.tickRateHz) {
      this.fixedDt = 1 / options.tickRateHz;
    }

    this.viewW = CFG.worldRadius * 2;
    this.viewH = CFG.worldRadius * 2; // Default to full world view
    this.lastGeneration = this.world.generation;
    this.lastHistoryLen = this.world.fitnessHistory.length;
  }

  /**
   * Run the simulation loop for a given real-world delta time.
   * Handles the fixed-timestep accumulation and interpolation (conceptually).
   * 
   * @param dt - Real time elapsed since last call (in seconds).
   * @param controllerProvider - Optional provider for snake controllers.
   */
  async update(dt: number, controllerProvider?: ControllerRegistryLike): Promise<void> {
    // Cap dt to prevent spiral of death
    if (dt > 0.2) dt = 0.2;

    this.fps = dt > 0 ? 1 / dt : 0;
    this.accumulator += dt;

    // Fixed Update Loop
    while (this.accumulator >= this.fixedDt) {
      this.tickId++;

      // Execute one physics step
      if (this.brainPool && this.batchEnabled) {
        // Use parallel batch inference if available.
        await this.world.updateAsync(
          this.fixedDt,
          this.viewW,
          this.viewH,
          controllerProvider,
          this.tickId,
          this.brainPool
        );
      } else {
        // Serial fallback
        this.world.update(
          this.fixedDt,
          this.viewW,
          this.viewH,
          controllerProvider,
          this.tickId
        );
      }

      this.accumulator -= this.fixedDt;
    }

    // Post-update logic (generation tracking)
    if (this.world.generation !== this.lastGeneration) {
      this.lastGeneration = this.world.generation;
      // Hook for persistence could go here
    }
  }

  /**
   * Serialize the current world state to a binary buffer.
   */
  serialize(): Float32Array {
    return WorldSerializer.serialize(this.world);
  }

  /**
   * Build a statistics object for the current frame.
   * This unifies the logic from server/simServer.ts and src/worker.ts
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
   * Reset the simulation with new settings.
   */
  reset(settings: Partial<CoreSettings>): void {
    this.world = new World(settings);
    this.tickId = 0;
    this.accumulator = 0;
    this.lastGeneration = this.world.generation;
    this.lastHistoryLen = this.world.fitnessHistory.length;
  }
}
