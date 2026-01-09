import { CFG } from '../config.ts';
import { clamp, TAU, angNorm } from '../utils.ts';
import { createRng, hashSeed, type RandomSource, toUint32 } from '../rng.ts';
import { Snake } from '../snake.ts';
import type { World } from '../world.ts';

/** Life stage threshold: Snakes below this length use the 'Small' survival strategy. */
const STRATEGY_THRESHOLD_MEDIUM = 25;
/** Life stage threshold: Snakes above this length use the 'Large' crowd strategy. */
const STRATEGY_THRESHOLD_LARGE = 80;

/** Bin evaluation penalty for directions with dangerous clearance (hazards/walls). */
const VETO_PENALTY = 1000;
/** Clearance threshold below which a direction is considered for veto. */
const VETO_THRESHOLD = -0.5;
/** Angle wander scale in radians for roaming. */
const WANDER_ANGLE_SCALE = 0.6;
/** Roam-to-Seek food activation threshold. */
const FOOD_TRIGGER_THRESHOLD = 0.1;
/** Minimum clearance to consider the environment 'safe' for random boosting. */
const ENV_SAFE_THRESHOLD = -0.3;
/** Seconds to remain in the 'avoid' state after a hazard trigger. */
const AVOID_DURATION_BASE = 0.35;
/** Chance per frame to trigger a random curiosity boost. */
const BOOST_CHANCE_PER_FRAME = 0.02;

/**
 * Controller states used by baseline bots.
 */
export type BotState = 'roam' | 'seek' | 'avoid' | 'boost';

/**
 * Settings payload for baseline bot behavior.
 */
export interface BaselineBotSettings {
  /** Number of baseline bots to run. */
  count: number;
  /** Base seed used to derive per-bot deterministic RNG streams. */
  seed: number;
  /** If true, generation number influences bot seeds. */
  randomizeSeedPerGen: boolean;
  /**
   * Seconds to wait before respawning a bot.
   * Defaults to 3.0 seconds to avoid large simultaneous respawns.
   */
  respawnDelay?: number;
}

/**
 * Control output for a baseline bot.
 */
export interface BotAction {
  /** Turn control in [-1, 1]. */
  turn: number;
  /** Boost control: 0 or 1. */
  boost: number;
}

/**
 * Normalize settings to safe numeric ranges and apply defaults.
 */
function normalizeSettings(settings: BaselineBotSettings): BaselineBotSettings {
  const count = Number.isFinite(settings.count) ? Math.max(0, Math.floor(settings.count)) : 0;
  const seed = Number.isFinite(settings.seed) ? Math.max(0, Math.floor(settings.seed)) : 0;
  const respawnDelay = Number.isFinite(settings.respawnDelay)
    ? clamp(settings.respawnDelay!, 0.1, 60)
    : 3.0;

  return {
    count,
    seed,
    randomizeSeedPerGen: Boolean(settings.randomizeSeedPerGen),
    respawnDelay
  };
}

/**
 * Convert a bin index into a signed angle relative to heading.
 *
 * @param index - Bin index.
 * @param bins - Total number of bins.
 * @returns Relative angle in radians within [-pi, pi].
 */
function binIndexToAngle(index: number, bins: number): number {
  const ang = (index / bins) * TAU;
  return ang > Math.PI ? ang - TAU : ang;
}

/**
 * Derive a deterministic seed for a baseline bot.
 *
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
 * Manages baseline bot state, seeds, and actions.
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
   *
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
   *
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
   */
  getCount(): number {
    return this.settings.count;
  }

  /**
   * Reset a bot RNG and state machine, returning the RNG for spawning.
   *
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
   *
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
   *
   * This is optional: the manager also schedules respawns automatically when
   * `world.baselineBots[index]` is missing or not alive.
   *
   * @param index - Baseline bot index.
   */
  markDead(index: number): void {
    if (index < 0 || index >= this.settings.count) return;
    const timer = this.respawnTimers[index] ?? -1;
    if (timer < 0) {
      this.respawnTimers[index] = this.settings.respawnDelay!;
    }
  }

  /**
   * Compute baseline bot actions and handle respawns.
   *
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

      // Handle dead or missing snakes via respawn scheduling.
      if (!snake || !snake.alive) {
        const timer = this.respawnTimers[i] ?? -1;
        if (timer < 0) {
          this.respawnTimers[i] = this.settings.respawnDelay!;
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
              this.respawnTimers[i] = this.settings.respawnDelay!;
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
   *
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
   *
   * @param index - Baseline bot index.
   * @returns Bot action or null when index is invalid.
   */
  getActionByIndex(index: number): BotAction | null {
    if (index < 0 || index >= this.settings.count) return null;
    return this.botActions[index] ?? null;
  }

  /**
   * Dispatches the bot's movement logic to a specific behavioral strategy 
   * based on its current length (Life Stage).
   * 
   * Strategy Archetypes:
   * 1. "Coward" (Small, length \< 25): Growth and extreme caution. 
   *    High bias towards fleeing (clearWeight: 1.8). Rarely boosts unless escaping.
   * 2. "Hunter" (Medium, length 25 - 79): Aggressive mass acquisition. 
   *    Targets nearby snake heads for interception and uses attacking boosts.
   * 3. "Bully" (Large, length \>= 80): Strategic dominance. 
   *    Uses mass to circle or crowd opponents, minimizing risk with low food priority.
   * 
   * @param world - Current simulation world.
   * @param snake - Bot's snake instance.
   * @param index - Bot's index in the manager.
   * @param dt - Frame delta time.
   */
  private computeAction(world: World, snake: Snake, index: number, dt: number): void {
    const len = snake.length();

    if (len < STRATEGY_THRESHOLD_MEDIUM) {
      this.computeActionSmall(world, snake, index, dt);
    } else if (len < STRATEGY_THRESHOLD_LARGE) {
      this.computeActionMedium(world, snake, index, dt);
    } else {
      this.computeActionLarge(world, snake, index, dt);
    }
  }

  /**
   * Small Strategy Logic: Extreme Survival & Growth.
   * 
   * Priorities:
   * 1. Avoidance: Uses a high `clearWeight` (1.8) to steer clear of all snakes/walls.
   * 2. Safety: Clamps food rewards to 0.4 to prevent chasing pellets into tight spaces.
   * 3. Conservation: Boost is only allowed if the bot is in an explicit 'avoid' state.
   */
  private computeActionSmall(world: World, snake: Snake, index: number, dt: number): void {
    const sensors = snake.computeSensors(world);
    const bins = Math.floor((sensors.length - 5) / 3);

    const foodOffset = 5;
    const hazardOffset = foodOffset + bins;
    const wallOffset = hazardOffset + bins;

    // Default weights for 'roam' state.
    let foodWeight = 0.5;
    let clearWeight = 1.8;

    const rng = this.botRngs[index];
    this.updateState(index, dt, rng, sensors, bins, foodOffset, hazardOffset, wallOffset, snake);

    const state = this.botStates[index] ?? 'roam';

    if (state === 'seek') {
      foodWeight = 0.5;
      clearWeight = 1.6;
    } else if (state === 'avoid') {
      foodWeight = 0.0;
      clearWeight = 2.5;
    }

    const FOOD_CLAMP_SMALL = 0.4;
    const { targetIdx } = this.evaluateBins(
      sensors,
      bins,
      foodOffset,
      hazardOffset,
      wallOffset,
      foodWeight,
      clearWeight,
      FOOD_CLAMP_SMALL
    );

    this.applyOutput(
      index,
      targetIdx,
      bins,
      dt,
      rng,
      state,
      sensors,
      hazardOffset,
      wallOffset,
      true // small bots only boost for escape
    );
  }

  /**
   * Medium Strategy Logic: Opportunistic Hunting.
   * 
   * Priorities:
   * 1. Interception: Periodically scans for nearby snakes and biases movement 
   *    towards a predicted intercept course.
   * 2. Aggression: Uses slightly higher `foodWeight` (0.8) and allows attacking boosts.
   * 3. Safety: Subject to the same `VETO_THRESHOLD` as all strategies.
   */
  private computeActionMedium(world: World, snake: Snake, index: number, dt: number): void {
    const sensors = snake.computeSensors(world);
    const bins = Math.floor((sensors.length - 5) / 3);

    const foodOffset = 5;
    const hazardOffset = foodOffset + bins;
    const wallOffset = hazardOffset + bins;

    let foodWeight = 0.8;
    let clearWeight = 1.2;

    const rng = this.botRngs[index];
    this.updateState(index, dt, rng, sensors, bins, foodOffset, hazardOffset, wallOffset, snake);

    let state = this.botStates[index] ?? 'roam';
    if (state === 'avoid') {
      foodWeight = 0.0;
      clearWeight = 2.0;
    }

    // Hunt bias toward the closest nearby snake head.
    let huntBiasAngle = 0;
    let huntStrength = 0;

    if (state !== 'avoid') {
      const myHead = snake.head();
      const SENSE_RADIUS_MULTIPLIER = 25;
      const senseR = snake.radius * SENSE_RADIUS_MULTIPLIER;
      let bestTarget: Snake | null = null;
      let minDistSq = Infinity;

      for (const other of world.snakes) {
        if (!other.alive || other === snake) continue;
        const oh = other.head();
        const dx = oh.x - myHead.x;
        const dy = oh.y - myHead.y;
        const d2 = dx * dx + dy * dy;
        if (d2 < senseR * senseR && d2 < minDistSq) {
          minDistSq = d2;
          bestTarget = other;
        }
      }

      if (bestTarget) {
        const oh = bestTarget.head();
        const dx = oh.x - myHead.x;
        const dy = oh.y - myHead.y;
        const angleTo = Math.atan2(dy, dx);

        // Size-based tactics:
        // If we are significantly longer than the target, try to encircle.
        // Otherwise, try to cut them off (aim ahead).
        const ENCIRCLE_RATIO = 2.5;
        const myLen = snake.length();
        const otherLen = bestTarget.length();

        if (myLen > otherLen * ENCIRCLE_RATIO && myLen > 50) {
          // ENCIRCLE: Aim for a point that orbits the target.
          // To "close in", we aim slightly towards the target from the tangent.
          const orbitAngle = angleTo + Math.PI / 2; // tangent
          const inwardBias = 0.4; // Aim 20-30 degrees inward to tighten the circle
          const targetWorldAngle = orbitAngle - inwardBias;
          huntBiasAngle = angNorm(targetWorldAngle - snake.dir);
          huntStrength = 1.0;
        } else {
          // CUTOFF: Aim ahead of the target's current heading.
          const leadFactor = 0.5;
          const leadX = oh.x + Math.cos(bestTarget.dir) * bestTarget.speed * leadFactor;
          const leadY = oh.y + Math.sin(bestTarget.dir) * bestTarget.speed * leadFactor;
          const leadAngle = Math.atan2(leadY - myHead.y, leadX - myHead.x);
          huntBiasAngle = angNorm(leadAngle - snake.dir);
          huntStrength = 0.8;
        }
        state = 'seek';
      }
    }

    const FOOD_CLAMP_MEDIUM = 0.6;
    const { targetIdx } = this.evaluateBins(
      sensors,
      bins,
      foodOffset,
      hazardOffset,
      wallOffset,
      foodWeight,
      clearWeight,
      FOOD_CLAMP_MEDIUM,
      (binAngle) => {
        if (huntStrength <= 0) return 0;
        const diff = Math.abs(angNorm(binAngle - huntBiasAngle));
        // Falloff the hunt bias as the bin angle diverges from the target angle.
        const HUNTER_BIAS_FALLOFF_RAD = 1.2;
        return diff < HUNTER_BIAS_FALLOFF_RAD ? huntStrength * (1.0 - diff / HUNTER_BIAS_FALLOFF_RAD) : 0;
      }
    );

    const canAttackBoost = huntStrength > 0 && state !== 'avoid';

    this.applyOutput(
      index,
      targetIdx,
      bins,
      dt,
      rng,
      state,
      sensors,
      hazardOffset,
      wallOffset,
      false,
      canAttackBoost
    );
  }

  /**
   * Large Strategy Logic: Crowd Pressure & Space Control.
   * 
   * Priorities:
   * 1. Crowd Density: Calculates the centroid of all nearby snakes and biases 
   *    movement towards that center to maximize area coverage and potential blocks.
   * 2. Mass Protection: High `clearWeight` (1.5) and low `foodWeight` (0.4) to 
   *    minimize risk of tail-biting or accidental collisions.
   * 3. Stability: Boost is heavily restricted to prevent erratic maneuvers.
   */
  private computeActionLarge(world: World, snake: Snake, index: number, dt: number): void {
    const sensors = snake.computeSensors(world);
    const bins = Math.floor((sensors.length - 5) / 3);

    const foodOffset = 5;
    const hazardOffset = foodOffset + bins;
    const wallOffset = hazardOffset + bins;

    let foodWeight = 0.4;
    let clearWeight = 1.5;

    const rng = this.botRngs[index];
    this.updateState(index, dt, rng, sensors, bins, foodOffset, hazardOffset, wallOffset, snake);

    const state = this.botStates[index] ?? 'roam';

    if (state === 'avoid') {
      foodWeight = 0.0;
      clearWeight = 2.5;
    }

    let crowdBiasAngle = 0;
    let crowdStrength = 0;

    if (state !== 'avoid') {
      const myHead = snake.head();
      const SENSE_RADIUS_MULTIPLIER = 30;
      const senseR = snake.radius * SENSE_RADIUS_MULTIPLIER;
      let sumX = 0;
      let sumY = 0;
      let count = 0;

      for (const other of world.snakes) {
        if (!other.alive || other === snake) continue;
        const oh = other.head();
        const dx = oh.x - myHead.x;
        const dy = oh.y - myHead.y;
        if (dx * dx + dy * dy < senseR * senseR) {
          sumX += oh.x;
          sumY += oh.y;
          count++;
        }
      }

      if (count > 0) {
        const centerX = sumX / count;
        const centerY = sumY / count;
        const angleTo = Math.atan2(centerY - myHead.y, centerX - myHead.x);
        crowdBiasAngle = angNorm(angleTo - snake.dir);

        const CROWD_PUSH_INTENSITY = 0.6;
        crowdStrength = CROWD_PUSH_INTENSITY;
      }
    }

    const FOOD_CLAMP_LARGE = 0.4;
    const { targetIdx } = this.evaluateBins(
      sensors,
      bins,
      foodOffset,
      hazardOffset,
      wallOffset,
      foodWeight,
      clearWeight,
      FOOD_CLAMP_LARGE,
      (binAngle) => {
        if (crowdStrength <= 0) return 0;
        const diff = Math.abs(angNorm(binAngle - crowdBiasAngle));
        // Large snakes use a regional bias to slowly "herd" others.
        const CROWD_BIAS_FALLOFF_RAD = 1.0;
        return diff < CROWD_BIAS_FALLOFF_RAD ? crowdStrength * (1.0 - diff) : 0;
      }
    );

    this.applyOutput(index, targetIdx, bins, dt, rng, state, sensors, hazardOffset, wallOffset, false, false);
  }

  /**
   * Update bot state (roam/seek/avoid/boost) based on sensor readings and timers.
   *
   * @param index - Bot index.
   * @param dt - Delta time in seconds.
   * @param rng - Bot RNG stream.
   * @param sensors - Sensor array.
   * @param bins - Number of sensor bins.
   * @param foodOffset - Food sensor start offset.
   * @param hazardOffset - Hazard sensor start offset.
   * @param wallOffset - Wall sensor start offset.
   * @param snake - Snake instance (used for boost eligibility).
   */
  private updateState(
    index: number,
    dt: number,
    rng: RandomSource | undefined,
    sensors: Float32Array,
    bins: number,
    foodOffset: number,
    hazardOffset: number,
    wallOffset: number,
    snake: Snake
  ): void {
    let state = this.botStates[index] ?? 'roam';
    let stateTimer = this.botStateTimers[index] ?? 0;

    // Timed states decay back to roam.
    if (state === 'avoid' || state === 'boost') {
      stateTimer -= dt;
      if (stateTimer <= 0) {
        state = 'roam';
        stateTimer = 0;
      }
    }

    // Scan sensors for the worst clearance and best available food.
    let worstClear = Infinity;
    let bestFood = -Infinity;

    for (let i = 0; i < bins; i++) {
      const h = sensors[hazardOffset + i] ?? -1;
      const w = sensors[wallOffset + i] ?? -1;
      const cl = (h + w) * 0.5;
      if (cl < worstClear) worstClear = cl;

      const f = sensors[foodOffset + i] ?? -1;
      if (f > bestFood) bestFood = f;
    }

    const hazardTrigger = -0.25;

    // Enter avoid immediately when boxed-in risk is detected.
    if (state !== 'avoid' && worstClear < hazardTrigger) {
      state = 'avoid';
      stateTimer = AVOID_DURATION_BASE + (rng ? rng() : 0) * AVOID_DURATION_BASE;
    } else if (state !== 'avoid' && state !== 'boost') {
      // Seek food when present, otherwise roam.
      state = bestFood > FOOD_TRIGGER_THRESHOLD ? 'seek' : 'roam';

      // Random short boosts (not used by small bots due to output policy).
      // We require a 10% safety margin over the global minimum to prevent starving the snake.
      const BOOST_SCORE_MARGIN = 1.1;
      const boostOk = snake.pointsScore > CFG.boost.minPointsToBoost * BOOST_SCORE_MARGIN;
      const environmentSafe = worstClear > ENV_SAFE_THRESHOLD;

      if (boostOk && environmentSafe && rng && rng() < BOOST_CHANCE_PER_FRAME) {
        state = 'boost';
        const BOOST_DURATION_BASE = 0.2;
        stateTimer = BOOST_DURATION_BASE + rng() * BOOST_DURATION_BASE;
      }
    }

    this.botStates[index] = state;
    this.botStateTimers[index] = stateTimer;
  }

  /**
   * Evaluate sensor bins and choose the best target index.
   *
   * Clearance is computed as the average of hazard and wall channels.
   * Bins with clearance below a veto threshold are strongly penalized.
   *
   * If all bins are vetoed, the selection falls back to the bin with the best clearance.
   *
   * @param sensors - Sensor array.
   * @param bins - Number of bins.
   * @param foodOffset - Food sensor start offset.
   * @param hazardOffset - Hazard sensor start offset.
   * @param wallOffset - Wall sensor start offset.
   * @param foodWeight - Food weight multiplier.
   * @param clearWeight - Clearance weight multiplier.
   * @param foodClamp - Maximum food contribution per bin.
   * @param biasFn - Optional additive bias function based on bin angle.
   * @returns Target index and best score.
   */
  private evaluateBins(
    sensors: Float32Array,
    bins: number,
    foodOffset: number,
    hazardOffset: number,
    wallOffset: number,
    foodWeight: number,
    clearWeight: number,
    foodClamp: number,
    biasFn?: (binAngle: number) => number
  ): { targetIdx: number; bestScore: number } {
    let bestScore = -Infinity;
    let targetIdx = 0;

    let bestClearVal = -Infinity;
    let bestClearIdx = 0;

    // Track whether any bin is not vetoed.
    let anyNonVeto = false;

    for (let i = 0; i < bins; i++) {
      const angle = binIndexToAngle(i, bins);
      const rawFood = sensors[foodOffset + i] ?? -1;
      const food = Math.min(rawFood, foodClamp);
      const hazard = sensors[hazardOffset + i] ?? -1;
      const wall = sensors[wallOffset + i] ?? -1;
      const clearance = (hazard + wall) * 0.5;

      if (clearance > bestClearVal) {
        bestClearVal = clearance;
        bestClearIdx = i;
      }

      let score = food * foodWeight + clearance * clearWeight;

      if (biasFn) {
        score += biasFn(angle);
      }

      // Turn Feasibility Penalty:
      // If a target (implied by high food or bias) is at a sharp angle but very close,
      // it might be inside our turning circle.
      // We check if the 'clearance' suggests something is close and at an angle
      // we can't easily hit.
      // Note: This is an approximation since we don't know the exact distance of pellets here,
      // but we can penalize extreme side-angles if the clearance is low.
      if (Math.abs(angle) > Math.PI / 4 && clearance < 0.2) {
        const sideRisk = (Math.abs(angle) - Math.PI / 4) / (Math.PI / 2);
        const proximityRisk = clamp(1.0 - clearance, 0, 1);
        // Penalty scale based on how much "outside" our turn radius we'd need to be
        score -= sideRisk * proximityRisk * 50.0;
      }

      // Safety Veto (Safe Harbor Retreat):
      // If the clearance in this direction is dangerously low, we heavily penalize 
      // the score to ensure the bot prioritizes survival (fleeing toward "safe harbor") 
      // even if high-reward food or prey is present in the hazard zone.
      if (clearance < VETO_THRESHOLD) {
        score -= VETO_PENALTY;
      } else {
        anyNonVeto = true;
      }

      if (score > bestScore) {
        bestScore = score;
        targetIdx = i;
      }
    }

    // If every bin is vetoed, choose the best available clearance rather than a score artifact.
    if (!anyNonVeto) {
      targetIdx = bestClearIdx;
    }

    return { targetIdx, bestScore };
  }

  /**
   * Compute final turn/boost output and write to the action buffer.
   *
   * @param index - Bot index.
   * @param targetIdx - Selected bin index.
   * @param bins - Number of bins.
   * @param dt - Delta time in seconds.
   * @param rng - RNG stream for wander updates.
   * @param state - Current bot state.
   * @param sensors - Sensor array.
   * @param hazardOffset - Hazard channel offset.
   * @param wallOffset - Wall channel offset.
   * @param strictBoost - If true, boost is only allowed during avoid.
   * @param attackBoost - If true, boost may be used during seek (subject to safety checks).
   */
  private applyOutput(
    index: number,
    targetIdx: number,
    bins: number,
    dt: number,
    rng: RandomSource | undefined,
    state: BotState,
    sensors: Float32Array,
    hazardOffset: number,
    wallOffset: number,
    strictBoost: boolean,
    attackBoost: boolean = false
  ): void {
    // Wander applies only in roam to prevent rigid motion.
    if (state === 'roam' && rng) {
      let wanderTimer = this.botWanderTimers[index] ?? 0;
      wanderTimer -= dt;
      if (wanderTimer <= 0) {
        this.botWanderAngles[index] = (rng() - 0.5) * WANDER_ANGLE_SCALE;
        const WANDER_DURATION_BASE = 0.6;
        const WANDER_DURATION_VAR = 1.4;
        wanderTimer = WANDER_DURATION_BASE + rng() * WANDER_DURATION_VAR;
      }
      this.botWanderTimers[index] = wanderTimer;
    }

    const wander = state === 'roam' ? this.botWanderAngles[index] ?? 0 : 0;
    const targetAngle = binIndexToAngle(targetIdx, bins) + wander;
    const turn = clamp(targetAngle / (Math.PI / 2), -1, 1);

    let boost = state === 'boost' ? 1 : 0;

    if (state === 'avoid') {
      const tgtHazard = sensors[hazardOffset + targetIdx] ?? -1;
      const tgtWall = sensors[wallOffset + targetIdx] ?? -1;
      const tgtClear = (tgtHazard + tgtWall) * 0.5;
      const ESCAPE_BOOST_CLEARANCE = 0.2;
      boost = tgtClear > ESCAPE_BOOST_CLEARANCE ? 1 : 0;
    }

    // Attack boost: allow some risk when actively hunting, but do not boost into a tight path.
    if (attackBoost && state !== 'avoid') {
      const tgtHazard = sensors[hazardOffset + targetIdx] ?? -1;
      const tgtWall = sensors[wallOffset + targetIdx] ?? -1;
      const tgtClear = (tgtHazard + tgtWall) * 0.5;
      const ATTACK_BOOST_MAX_RISK = -0.1;
      if (tgtClear > ATTACK_BOOST_MAX_RISK) boost = 1;
    }

    // Strict boost policy prevents boost outside avoid (used for small bots).
    if (strictBoost && state !== 'avoid') {
      boost = 0;
    }

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
