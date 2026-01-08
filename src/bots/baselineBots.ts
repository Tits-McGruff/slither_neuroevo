import { CFG } from '../config.ts';
import { clamp, TAU, angNorm } from '../utils.ts';
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
  respawnDelay?: number;
}


/** Control output for a baseline bot. */
export interface BotAction {
  turn: number;
  boost: number;
}

function normalizeSettings(settings: BaselineBotSettings): BaselineBotSettings {
  const count = Number.isFinite(settings.count) ? Math.max(0, Math.floor(settings.count)) : 0;
  const seed = Number.isFinite(settings.seed) ? Math.max(0, Math.floor(settings.seed)) : 0;
  // Default 3.0 seconds to prevent horde
  const respawnDelay = Number.isFinite(settings.respawnDelay) ? clamp(settings.respawnDelay!, 0.1, 60) : 3.0; 
  return {
    count,
    seed,
    randomizeSeedPerGen: Boolean(settings.randomizeSeedPerGen),
    respawnDelay
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
      this.respawnTimers[index] = this.settings.respawnDelay!;
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
   * Dispatches to specific strategies based on snake size.
   */
  private computeAction(world: World, snake: Snake, index: number, dt: number): void {
    const len = snake.length();
    // Life Stage Thresholds
    // Small: < 25 (Survival Mode)
    // Medium: 25 - 80 (Hunter Mode)
    // Large: >= 80 (Crowd Control Mode)
    
    if (len < 25) {
      this.computeActionSmall(world, snake, index, dt);
    } else if (len < 80) {
      this.computeActionMedium(world, snake, index, dt);
    } else {
      this.computeActionLarge(world, snake, index, dt);
    }
  }

  /**
   * Small Bot Strategy: "The Coward"
   * Focus: Survival / Growing.
   * - High clearance weight.
   * - Clamped food weight (don't get overwhelmed).
   * - Boost ONLY for escape.
   */
  private computeActionSmall(world: World, snake: Snake, index: number, dt: number): void {
    const sensors = snake.computeSensors(world);
    const bins = Math.floor((sensors.length - 5) / 3);
    const foodOffset = 5;
    const hazardOffset = foodOffset + bins;
    const wallOffset = hazardOffset + bins;

    // Weights: Prioritize Safety
    let foodWeight = 0.5;
    let clearWeight = 1.8; // Very high safety priority for small snakes
    
    const rng = this.botRngs[index];
    this.updateState(index, dt, rng, sensors, bins, hazardOffset, wallOffset, snake);

    let state = this.botStates[index] ?? 'roam';

    if (state === 'seek') {
        foodWeight = 0.5;
        clearWeight = 1.6;
    } else if (state === 'avoid') {
        foodWeight = 0.0;
        clearWeight = 2.5; // Extreme avoidance
    }

    const { targetIdx } = this.evaluateBins(
      sensors, bins, foodOffset, hazardOffset, wallOffset, 
      foodWeight, clearWeight, 0.4 // Max food clamp
    );

    this.applyOutput(index, targetIdx, bins, dt, rng, state, sensors, hazardOffset, wallOffset, true);
  }

  /**
   * Medium Bot Strategy: "The Hunter"
   * Focus: Aggression / Intercept.
   * - Seeks food but looks for targets.
   * - If a vulnerable target is found, biases heading towards intercept.
   * - Boosts allowed for attack.
   */
  private computeActionMedium(world: World, snake: Snake, index: number, dt: number): void {
    const sensors = snake.computeSensors(world);
    const bins = Math.floor((sensors.length - 5) / 3);
    const foodOffset = 5;
    const hazardOffset = foodOffset + bins;
    const wallOffset = hazardOffset + bins;

    // Weights: Balanced but aggressive
    let foodWeight = 0.8; // More interested in food/growth than small bots
    let clearWeight = 1.2; 
    
    const rng = this.botRngs[index];
    this.updateState(index, dt, rng, sensors, bins, hazardOffset, wallOffset, snake);
    let state = this.botStates[index] ?? 'roam';

    if (state === 'avoid') {
        foodWeight = 0.0;
        clearWeight = 2.0;
    }

    // Hunter Logic: Find a target
    let huntBiasAngle = 0;
    let huntStrength = 0;
    
    // Only hunt if not avoiding and not already boosting to escape
    if (state !== 'avoid') {
        const myHead = snake.head();
        const senseR = snake.radius * 25; // Roughly the sensor bubble
        let bestTarget: Snake | null = null;
        let minDistSq = Infinity;
        
        for (const other of world.snakes) {
            if (!other.alive || other === snake) continue;
            // Target smaller snakes or roughly equal? 
            // Actually, mid-size bots often attack larger ones in Slither.io to eat their corpse.
            // But let's stick to "vulnerable" or "close".
            // Let's target ANY snake that is close enough to intercept.
            const oh = other.head();
            const dx = oh.x - myHead.x;
            const dy = oh.y - myHead.y;
            const d2 = dx*dx + dy*dy;
            if (d2 < senseR*senseR && d2 < minDistSq) {
                minDistSq = d2;
                bestTarget = other;
            }
        }

        if (bestTarget) {
            // Simple intercept: Move towards current position (predictive is better but expensive)
            const oh = bestTarget.head();
            const angleTo = Math.atan2(oh.y - myHead.y, oh.x - myHead.x);
            // Relative angle required
            const relAngle = angNorm(angleTo - snake.dir);
            huntBiasAngle = relAngle;
            huntStrength = 0.8; // Strong pull towards target
            state = 'seek'; // Force seek mode if hunting
        }
    }

    // Evaluate bins with Hunt Bias
    // We add a "hunt bonus" to bins aligned with huntBiasAngle
    const { targetIdx } = this.evaluateBins(
      sensors, bins, foodOffset, hazardOffset, wallOffset, 
      foodWeight, clearWeight, 0.6, // Higher food clamp
      (binAngle) => {
         if (huntStrength <= 0) return 0;
         const diff = Math.abs(angNorm(binAngle - huntBiasAngle));
         // Gaussian-ish width
         return diff < 1.0 ? huntStrength * (1.0 - diff) : 0;
      }
    );

    // Allow boost for hunting (if safe)
    // If state is seek and we have a target (huntStrength > 0), allow boost
    const canAttackBoost = huntStrength > 0 && state !== 'avoid';

    this.applyOutput(index, targetIdx, bins, dt, rng, state, sensors, hazardOffset, wallOffset, false, canAttackBoost);
  }

  /**
   * Large Bot Strategy: "The Bully"
   * Focus: Crowd Control / Accidents.
   * - Moves towards DENSITY (groups of snakes).
   * - Blocking behavior (high clearance weight to create walls).
   */
  private computeActionLarge(world: World, snake: Snake, index: number, dt: number): void {
    const sensors = snake.computeSensors(world);
    const bins = Math.floor((sensors.length - 5) / 3);
    const foodOffset = 5;
    const hazardOffset = foodOffset + bins;
    const wallOffset = hazardOffset + bins;

    // Weights: Safety is paramount to maintain bulk, but we want to be near others
    let foodWeight = 0.4; 
    let clearWeight = 1.5;
    
    const rng = this.botRngs[index];
    this.updateState(index, dt, rng, sensors, bins, hazardOffset, wallOffset, snake);
    let state = this.botStates[index] ?? 'roam';

    if (state === 'avoid') {
        foodWeight = 0.0;
        clearWeight = 2.5;
    }

    // Crowd Logic: Find center of mass of nearby snakes
    let crowdBiasAngle = 0;
    let crowdStrength = 0;

    if (state !== 'avoid') {
        const myHead = snake.head();
        const senseR = snake.radius * 30; // Big view
        let sumX = 0, sumY = 0, count = 0;
        
        for (const other of world.snakes) {
            if (!other.alive || other === snake) continue;
            const oh = other.head();
            const dx = oh.x - myHead.x;
            const dy = oh.y - myHead.y;
            if (dx*dx + dy*dy < senseR*senseR) {
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
            crowdStrength = 0.6; // Moderate pull towards crowds
        }
    }

    const { targetIdx } = this.evaluateBins(
      sensors, bins, foodOffset, hazardOffset, wallOffset, 
      foodWeight, clearWeight, 0.4,
      (binAngle) => {
         if (crowdStrength <= 0) return 0;
         const diff = Math.abs(angNorm(binAngle - crowdBiasAngle));
         return diff < 1.0 ? crowdStrength * (1.0 - diff) : 0;
      }
    );

    this.applyOutput(index, targetIdx, bins, dt, rng, state, sensors, hazardOffset, wallOffset, false, false);
  }

  // --- Helpers ---

  /** Updates bot state (roam/seek/avoid/boost) based on immediate hazards and timers. */
  private updateState(
      index: number, dt: number, rng: RandomSource | undefined, 
      sensors: Float32Array, bins: number, hazardOffset: number, wallOffset: number,
      snake: Snake
  ): void {
      let state = this.botStates[index] ?? 'roam';
      let stateTimer = this.botStateTimers[index] ?? 0;
      
      // Update Timers
      if (state === 'avoid' || state === 'boost') {
          stateTimer -= dt;
          if (stateTimer <= 0) {
              state = 'roam';
              stateTimer = 0;
          }
      }

      // Check Hazard
      let worstClear = Infinity;
      let bestFood = -Infinity;
      const foodOffset = 5; // Fixed assumption

      for(let i=0; i<bins; i++) {
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

      // Transitions
      if (state !== 'avoid' && worstClear < hazardTrigger) {
          state = 'avoid';
          stateTimer = 0.35 + (rng ? rng() : 0) * 0.35;
      } else if (state !== 'avoid' && state !== 'boost') {
          // Default logic: Seek if food, else Roam
          state = bestFood > foodTrigger ? 'seek' : 'roam';

          // Random Boost Trigger
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

  /** Evaluates sensor bins and returns best target index. */
  private evaluateBins(
      sensors: Float32Array, bins: number, 
      foodOffset: number, hazardOffset: number, wallOffset: number,
      foodWeight: number, clearWeight: number, foodClamp: number,
      biasFn?: (binAngle: number) => number
  ): { targetIdx: number, bestScore: number } {
      let bestScore = -Infinity;
      let targetIdx = 0;
      const VETO_THRESHOLD = -0.5;

      // Find best bin just for fallback
      let bestClearVal = -Infinity;
      let bestClearIdx = 0;

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
        
        // Add Bias (Hunter/Crowd)
        if (biasFn) {
            const angle = binIndexToAngle(i, bins);
            score += biasFn(angle);
        }

        // Strict Veto
        if (clearance < VETO_THRESHOLD) {
          score -= 1000;
        }

        if (score > bestScore) {
          bestScore = score;
          targetIdx = i;
        }
      }
      
      // Fallback if all vetoed (rare but possible) or -Infinity
      if (bestScore === -Infinity) targetIdx = bestClearIdx;

      return { targetIdx, bestScore };
  }

  /** Computes final turn/boost output and writes to action buffer. */
  private applyOutput(
      index: number, targetIdx: number, bins: number, dt: number, rng: RandomSource | undefined,
      state: BotState, sensors: Float32Array, hazardOffset: number, wallOffset: number,
      strictBoost: boolean, attackBoost: boolean = false
  ): void {
      // Wander logic
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
      
      // Escape Boost Logic (Avoid Mode)
      if (state === 'avoid') {
         const tgtHazard = sensors[hazardOffset + targetIdx] ?? -1;
         const tgtWall = sensors[wallOffset + targetIdx] ?? -1;
         const tgtClear = (tgtHazard + tgtWall) * 0.5;
         if (tgtClear > 0.2) {
             boost = 1;
         } else {
             boost = 0; // Don't boost if escape path is tight
         }
      }

      // Attack Boost Logic (Hunter Mode)
      if (attackBoost && state !== 'avoid') {
          // Check safety of target dir
          const tgtHazard = sensors[hazardOffset + targetIdx] ?? -1;
          const tgtWall = sensors[wallOffset + targetIdx] ?? -1;
          const tgtClear = (tgtHazard + tgtWall) * 0.5;
          if (tgtClear > -0.1) { // Accept slight risk for kill
              boost = 1; 
          }
      }

      // Strict Boost prevention (Small/Coward Mode)
      if (strictBoost && state !== 'avoid') {
          boost = 0; // Never boost unless avoiding
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
