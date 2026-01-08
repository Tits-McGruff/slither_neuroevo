import { CFG } from '../config.ts';
import { clamp, TAU, angNorm } from '../utils.ts';
import { createRng, hashSeed, type RandomSource, toUint32 } from '../rng.ts';
import type { Snake } from '../snake.ts';
import type { World } from '../world.ts';

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
   * Compute a baseline bot action based on sensors and state.
   *
   * Strategy is selected based on snake length:
   * - Small: survival/growth focus.
   * - Medium: opportunistic hunting.
   * - Large: crowd pressure and position control.
   */
  private computeAction(world: World, snake: Snake, index: number, dt: number): void {
    const len = snake.length();

    if (len < 25) {
      this.computeActionSmall(world, snake, index, dt);
    } else if (len < 80) {
      this.computeActionMedium(world, snake, index, dt);
    } else {
      this.computeActionLarge(world, snake, index, dt);
    }
  }

  /**
   * Small strategy: survival / growth.
   *
   * - Strong preference for clearance.
   * - Food contribution is clamped to reduce risky chasing.
   * - Boost is only used for escape.
   */
  private computeActionSmall(world: World, snake: Snake, index: number, dt: number): void {
    const sensors = snake.computeSensors(world);
    const bins = Math.floor((sensors.length - 5) / 3);

    const foodOffset = 5;
    const hazardOffset = foodOffset + bins;
    const wallOffset = hazardOffset + bins;

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

    const { targetIdx } = this.evaluateBins(
      sensors,
      bins,
      foodOffset,
      hazardOffset,
      wallOffset,
      foodWeight,
      clearWeight,
      0.4
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
      true
    );
  }

  /**
   * Medium strategy: opportunistic hunting.
   *
   * - Balances food and clearance.
   * - Adds an intercept bias toward nearby snakes.
   * - Boost may be used when attacking, subject to safety checks.
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
      const senseR = snake.radius * 25;
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
        const angleTo = Math.atan2(oh.y - myHead.y, oh.x - myHead.x);
        huntBiasAngle = angNorm(angleTo - snake.dir);
        huntStrength = 0.8;
        state = 'seek';
      }
    }

    const { targetIdx } = this.evaluateBins(
      sensors,
      bins,
      foodOffset,
      hazardOffset,
      wallOffset,
      foodWeight,
      clearWeight,
      0.6,
      (binAngle) => {
        if (huntStrength <= 0) return 0;
        const diff = Math.abs(angNorm(binAngle - huntBiasAngle));
        return diff < 1.0 ? huntStrength * (1.0 - diff) : 0;
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
   * Large strategy: crowd pressure.
   *
   * - Keeps high clearance preference to protect mass.
   * - Adds a mild bias toward the center of nearby snake density.
   * - Avoid mode overrides with maximum safety preference.
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
      const senseR = snake.radius * 30;
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
        crowdStrength = 0.6;
      }
    }

    const { targetIdx } = this.evaluateBins(
      sensors,
      bins,
      foodOffset,
      hazardOffset,
      wallOffset,
      foodWeight,
      clearWeight,
      0.4,
      (binAngle) => {
        if (crowdStrength <= 0) return 0;
        const diff = Math.abs(angNorm(binAngle - crowdBiasAngle));
        return diff < 1.0 ? crowdStrength * (1.0 - diff) : 0;
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
    const foodTrigger = 0.1;
    const boostChance = 0.02;

    // Enter avoid immediately when boxed-in risk is detected.
    if (state !== 'avoid' && worstClear < hazardTrigger) {
      state = 'avoid';
      stateTimer = 0.35 + (rng ? rng() : 0) * 0.35;
    } else if (state !== 'avoid' && state !== 'boost') {
      // Seek food when present, otherwise roam.
      state = bestFood > foodTrigger ? 'seek' : 'roam';

      // Random short boosts (not used by small bots due to output policy).
      const boostOk = snake.pointsScore > CFG.boost.minPointsToBoost * 1.1;
      const environmentSafe = worstClear > -0.3;

      if (boostOk && environmentSafe && rng && rng() < boostChance) {
        state = 'boost';
        stateTimer = 0.2 + rng() * 0.2;
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

    const VETO_THRESHOLD = -0.5;

    // Track best clearance for fallback behavior.
    let bestClearVal = -Infinity;
    let bestClearIdx = 0;

    // Track whether any bin is not vetoed.
    let anyNonVeto = false;

    for (let i = 0; i < bins; i++) {
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
        const angle = binIndexToAngle(i, bins);
        score += biasFn(angle);
      }

      if (clearance < VETO_THRESHOLD) {
        score -= 1000;
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
        this.botWanderAngles[index] = (rng() - 0.5) * 0.6;
        wanderTimer = 0.6 + rng() * 1.4;
      }
      this.botWanderTimers[index] = wanderTimer;
    }

    const wander = state === 'roam' ? this.botWanderAngles[index] ?? 0 : 0;
    const targetAngle = binIndexToAngle(targetIdx, bins) + wander;
    const turn = clamp(targetAngle / (Math.PI / 2), -1, 1);

    let boost = state === 'boost' ? 1 : 0;

    // Escape boost: only boost when the chosen direction has enough clearance.
    if (state === 'avoid') {
      const tgtHazard = sensors[hazardOffset + targetIdx] ?? -1;
      const tgtWall = sensors[wallOffset + targetIdx] ?? -1;
      const tgtClear = (tgtHazard + tgtWall) * 0.5;
      boost = tgtClear > 0.2 ? 1 : 0;
    }

    // Attack boost: allow some risk when actively hunting, but do not boost into a tight path.
    if (attackBoost && state !== 'avoid') {
      const tgtHazard = sensors[hazardOffset + targetIdx] ?? -1;
      const tgtWall = sensors[wallOffset + targetIdx] ?? -1;
      const tgtClear = (tgtHazard + tgtWall) * 0.5;
      if (tgtClear > -0.1) boost = 1;
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
