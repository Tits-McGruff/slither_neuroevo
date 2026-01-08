import { CFG } from '../config.ts';
import { clamp, TAU } from '../utils.ts';
import { createRng, hashSeed, type RandomSource, toUint32 } from '../rng.ts';
import type { Snake } from '../snake.ts';
import type { World } from '../world.ts';

/** Bot controller states for baseline bots. */
export type BotState = 'roam' | 'seek' | 'avoid' | 'boost';

/** Settings payload for baseline bot behavior. */
export interface BaselineBotSettings {
  count: number;
  seed: number;
  randomizeSeedPerGen: boolean;
}

/** Control output for a baseline bot. */
export interface BotAction {
  turn: number;
  boost: number;
}

/** Fixed respawn delay in seconds for baseline bots. */
const BASELINE_BOT_RESPAWN_DELAY = 0.5;

/**
 * Normalize baseline bot settings to safe defaults.
 * @param settings - Raw settings payload.
 * @returns Normalized settings.
 */
function normalizeSettings(settings: BaselineBotSettings): BaselineBotSettings {
  const count = Number.isFinite(settings.count) ? Math.max(0, Math.floor(settings.count)) : 0;
  const seed = Number.isFinite(settings.seed) ? Math.max(0, Math.floor(settings.seed)) : 0;
  return {
    count,
    seed,
    randomizeSeedPerGen: Boolean(settings.randomizeSeedPerGen)
  };
}

/**
 * Convert a bin index into a signed angle relative to heading.
 * @param index - Bin index.
 * @param bins - Total bins.
 * @returns Relative angle in radians within [-pi, pi].
 */
function binIndexToAngle(index: number, bins: number): number {
  const ang = (index / bins) * TAU;
  return ang > Math.PI ? ang - TAU : ang;
}

/**
 * Derive a deterministic seed for a baseline bot.
 * @param baseSeed - Base seed from settings.
 * @param generation - Current generation index.
 * @param baselineBotIndex - Stable bot index within the baseline bot group.
 * @param randomizeSeedPerGen - Whether generation should influence the base seed.
 * @returns Unsigned 32-bit seed.
 */
export function deriveBotSeed(
  baseSeed: number,
  generation: number,
  baselineBotIndex: number,
  randomizeSeedPerGen: boolean
): number {
  const safeBase = toUint32(baseSeed);
  const safeGen = toUint32(generation);
  const safeIndex = toUint32(baselineBotIndex);
  const genSeed = randomizeSeedPerGen ? hashSeed(safeBase, safeGen) : safeBase;
  return hashSeed(genSeed, safeIndex);
}

/**
 * Manage baseline bot state, seeds, and actions.
 */
export class BaselineBotManager {
  /** Normalized baseline bot settings. */
  private settings: BaselineBotSettings;
  /** Per-bot deterministic seeds. */
  private botSeeds: number[];
  /** Per-bot RNG streams. */
  private botRngs: RandomSource[];
  /** Per-bot current state. */
  private botStates: BotState[];
  /** Per-bot state timers in seconds. */
  private botStateTimers: number[];
  /** Per-bot wander offsets in radians. */
  private botWanderAngles: number[];
  /** Per-bot wander timers in seconds. */
  private botWanderTimers: number[];
  /** Per-bot action buffers. */
  private botActions: BotAction[];
  /** Per-bot snake id mapping. */
  private botSnakeIds: number[];
  /** Map of snake id to bot index. */
  private snakeIdToIndex: Map<number, number>;
  /** Per-bot respawn timers in seconds. */
  private respawnTimers: number[];
  /** Whether bot updates are disabled for the current generation. */
  private controllerDisabled: boolean;

  /**
   * Create a baseline bot manager.
   * @param settings - Baseline bot settings payload.
   */
  constructor(settings: BaselineBotSettings) {
    this.settings = normalizeSettings(settings);
    this.botSeeds = [];
    this.botRngs = [];
    this.botStates = [];
    this.botStateTimers = [];
    this.botWanderAngles = [];
    this.botWanderTimers = [];
    this.botActions = [];
    this.botSnakeIds = [];
    this.snakeIdToIndex = new Map();
    this.respawnTimers = [];
    this.controllerDisabled = false;
    this.resetForGeneration(1);
  }

  /**
   * Reset bot seeds and state for a new generation.
   * @param generation - New generation index.
   */
  resetForGeneration(generation: number): void {
    const count = this.settings.count;
    this.botSeeds.length = count;
    this.botRngs.length = count;
    this.botStates.length = count;
    this.botStateTimers.length = count;
    this.botWanderAngles.length = count;
    this.botWanderTimers.length = count;
    this.botActions.length = count;
    this.botSnakeIds.length = count;
    this.respawnTimers.length = count;
    this.snakeIdToIndex.clear();
    this.controllerDisabled = false;
    for (let i = 0; i < count; i++) {
      const seed = deriveBotSeed(
        this.settings.seed,
        generation,
        i,
        this.settings.randomizeSeedPerGen
      );
      this.botSeeds[i] = seed;
      this.botRngs[i] = createRng(seed);
      this.botStates[i] = 'roam';
      this.botStateTimers[i] = 0;
      this.botWanderAngles[i] = 0;
      this.botWanderTimers[i] = 0;
      this.botActions[i] = { turn: 0, boost: 0 };
      this.botSnakeIds[i] = -1;
      this.respawnTimers[i] = -1;
    }
  }

  /**
   * Return the configured baseline bot count.
   * @returns Bot count.
   */
  getCount(): number {
    return this.settings.count;
  }

  /**
   * Reset a bot RNG and state machine, returning the RNG for spawning.
   * @param index - Baseline bot index.
   * @returns RNG for spawn usage.
   */
  prepareBotSpawn(index: number): RandomSource {
    const seed = this.botSeeds[index] ?? 0;
    const rng = createRng(seed);
    this.botRngs[index] = rng;
    this.botStates[index] = 'roam';
    this.botStateTimers[index] = 0;
    this.botWanderAngles[index] = 0;
    this.botWanderTimers[index] = 0;
    return rng;
  }

  /**
   * Register a baseline bot snake id mapping.
   * @param index - Baseline bot index.
   * @param snakeId - Snake id assigned to the bot.
   */
  registerBot(index: number, snakeId: number): void {
    if (index < 0 || index >= this.settings.count) return;
    const priorId = this.botSnakeIds[index];
    if (priorId != null && priorId >= 0) {
      this.snakeIdToIndex.delete(priorId);
    }
    this.botSnakeIds[index] = snakeId;
    this.snakeIdToIndex.set(snakeId, index);
  }

  /**
   * Mark a baseline bot as dead and schedule a respawn.
   * @param index - Baseline bot index.
   */
  markDead(index: number): void {
    if (index < 0 || index >= this.settings.count) return;
    const timer = this.respawnTimers[index] ?? -1;
    if (timer < 0) {
      this.respawnTimers[index] = BASELINE_BOT_RESPAWN_DELAY;
    }
  }

  /**
   * Compute baseline bot actions and handle respawns.
   * @param world - World instance to read sensors and spawn bots.
   * @param dt - Delta time in seconds for timers.
   * @param respawn - Callback that respawns a bot for a given index.
   */
  update(world: World, dt: number, respawn: (index: number, rng: RandomSource) => Snake | null): void {
    const count = this.settings.count;
    if (count <= 0) return;
    if (this.controllerDisabled) {
      this.zeroActions();
      return;
    }
    for (let i = 0; i < count; i++) {
      const snake = world.baselineBots[i];
      if (!snake || !snake.alive) {
        const timer = this.respawnTimers[i] ?? -1;
        if (timer < 0) {
          this.respawnTimers[i] = BASELINE_BOT_RESPAWN_DELAY;
        } else {
          const nextTimer = timer - dt;
          this.respawnTimers[i] = nextTimer;
          if (nextTimer <= 0) {
            const rng = this.prepareBotSpawn(i);
            const respawned = respawn(i, rng);
            if (respawned) {
              this.registerBot(i, respawned.id);
              this.respawnTimers[i] = -1;
            } else {
              this.respawnTimers[i] = BASELINE_BOT_RESPAWN_DELAY;
            }
          }
        }
        const action = this.botActions[i];
        if (action) {
          action.turn = 0;
          action.boost = 0;
        }
        continue;
      }
      try {
        this.computeAction(world, snake, i, dt);
      } catch (err) {
        const snakeId = snake.id;
        console.warn('[baselineBots] bot.controller.error', {
          snakeId,
          state: this.botStates[i],
          error: err instanceof Error ? err.message : String(err)
        });
        this.controllerDisabled = true;
        this.zeroActions();
        return;
      }
    }
  }

  /**
   * Get the action buffer for a specific snake id.
   * @param snakeId - Snake id to query.
   * @returns Bot action or null when not a baseline bot.
   */
  getActionForSnake(snakeId: number): BotAction | null {
    const index = this.snakeIdToIndex.get(snakeId);
    if (index == null) return null;
    return this.botActions[index] ?? null;
  }

  /**
   * Get the action buffer for a baseline bot index.
   * @param index - Baseline bot index.
   * @returns Bot action or null when index is invalid.
   */
  getActionByIndex(index: number): BotAction | null {
    if (index < 0 || index >= this.settings.count) return null;
    return this.botActions[index] ?? null;
  }

  /**
   * Compute a baseline bot action based on sensors and state.
   * @param world - World instance for sensors.
   * @param snake - Baseline bot snake instance.
   * @param index - Baseline bot index.
   * @param dt - Delta time in seconds for timers.
   */
  private computeAction(world: World, snake: Snake, index: number, dt: number): void {
    const sensors = snake.computeSensors(world);
    const bins = Math.floor((sensors.length - 5) / 3);
    const foodOffset = 5;
    const hazardOffset = foodOffset + bins;
    const wallOffset = hazardOffset + bins;

    let bestFood = -Infinity;
    let bestClear = -Infinity;
    let bestClearIdx = 0;
    let worstClear = Infinity;
    for (let i = 0; i < bins; i++) {
      const food = sensors[foodOffset + i] ?? -1;
      const hazard = sensors[hazardOffset + i] ?? -1;
      const wall = sensors[wallOffset + i] ?? -1;
      const clearance = (hazard + wall) * 0.5;
      if (food > bestFood) {
        bestFood = food;
      }
      if (clearance > bestClear) {
        bestClear = clearance;
        bestClearIdx = i;
      }
      if (clearance < worstClear) {
        worstClear = clearance;
      }
    }

    const rng = this.botRngs[index];
    const foodTrigger = 0.1;
    const hazardTrigger = -0.25;
    const boostChance = 0.02;

    let state = this.botStates[index] ?? 'roam';
    let stateTimer = this.botStateTimers[index] ?? 0;

    if (state === 'avoid' || state === 'boost') {
      stateTimer -= dt;
      if (stateTimer <= 0) {
        state = 'roam';
        stateTimer = 0;
      }
    }

    if (state !== 'avoid' && worstClear < hazardTrigger) {
      state = 'avoid';
      stateTimer = 0.35 + (rng ? rng() : 0) * 0.35;
    } else if (state !== 'avoid' && state !== 'boost') {
      state = bestFood > foodTrigger ? 'seek' : 'roam';
      const boostOk = snake.pointsScore > CFG.boost.minPointsToBoost * 1.1;
      if (boostOk && rng && rng() < boostChance) {
        state = 'boost';
        stateTimer = 0.2 + rng() * 0.2;
      }
    }

    this.botStates[index] = state;
    this.botStateTimers[index] = stateTimer;

    let foodWeight = 0.5;
    let clearWeight = 0.7;
    if (state === 'seek') {
      foodWeight = 1.4;
      clearWeight = 0.6;
    } else if (state === 'avoid') {
      foodWeight = 0.2;
      clearWeight = 1.6;
    }

    let bestScore = -Infinity;
    let targetIdx = bestClearIdx;
    for (let i = 0; i < bins; i++) {
      const food = sensors[foodOffset + i] ?? -1;
      const hazard = sensors[hazardOffset + i] ?? -1;
      const wall = sensors[wallOffset + i] ?? -1;
      const clearance = (hazard + wall) * 0.5;
      const score = food * foodWeight + clearance * clearWeight;
      if (score > bestScore) {
        bestScore = score;
        targetIdx = i;
      }
    }

    if (state === 'roam' && rng) {
      let wanderTimer = this.botWanderTimers[index] ?? 0;
      wanderTimer -= dt;
      if (wanderTimer <= 0) {
        this.botWanderAngles[index] = (rng() - 0.5) * 0.6;
        wanderTimer = 0.6 + rng() * 1.4;
      }
      this.botWanderTimers[index] = wanderTimer;
    }

    const wander = state === 'roam' ? this.botWanderAngles[index] ?? 0 : 0;
    const targetAngle = binIndexToAngle(targetIdx, bins) + wander;
    const turn = clamp(targetAngle / (Math.PI / 2), -1, 1);
    const boost = state === 'boost' ? 1 : 0;
    const action = this.botActions[index];
    if (action) {
      action.turn = turn;
      action.boost = boost;
    }
  }

  /**
   * Clear all bot actions to neutral inputs.
   */
  private zeroActions(): void {
    for (const action of this.botActions) {
      if (!action) continue;
      action.turn = 0;
      action.boost = 0;
    }
  }
}
