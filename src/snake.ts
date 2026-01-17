// snake.ts
// Definitions of Pellet, Snake and segment grid classes.  These objects
// encapsulate the state and behaviour of the snakes and provide
// geometry helpers for collision detection.

import { CFG } from './config.ts';
import { clamp, hashColor, rand, lerp, angNorm, hypot, TAU } from './utils.ts';
import { buildSensors } from './sensors.ts';
import type { ArchDefinition, Genome } from './mlp.ts';
import type { Brain } from './brains/types.ts';
import type { SimProfiler } from './profiling.ts';
import type { RandomSource } from './rng.ts';

/**
 * Simple data class representing a pellet at (x,y) with value v.
 */
export class Pellet {
  /** World X position. */
  x: number;
  /** World Y position. */
  y: number;
  /** Energy value provided when eaten. */
  v: number;
  /** Optional explicit color override. */
  color: string | null;
  /** Pellet kind identifier. */
  kind: string;
  /** Optional palette id for fast renderer. */
  colorId: number;
  /** Internal index hint for bookkeeping. */
  _idx?: number;
  /** Cached grid cell X coordinate. */
  _pcx?: number;
  /** Cached grid cell Y coordinate. */
  _pcy?: number;
  /** Cached grid cell key string. */
  _pkey?: string;
  /** Cached reference to the cell array. */
  _cellArr: Pellet[] | null = null;
  /** Index within the cached cell array. */
  _cellIndex?: number;

  /**
   * Create a pellet with position, value, and optional styling metadata.
   * @param x - World X position.
   * @param y - World Y position.
   * @param v - Pellet energy value.
   * @param color - Optional explicit color.
   * @param kind - Pellet kind identifier.
   * @param colorId - Optional palette id for rendering.
   */
  constructor(x: number, y: number, v: number, color: string | null = null, kind = "ambient", colorId = 0) {
    this.x = x;
    this.y = y;
    this.v = v;
    this.color = color;
    this.kind = kind;
    this.colorId = colorId;
  }
}

/** Simple point structure for snake segments. */
interface Point {
  x: number;
  y: number;
}

/** Minimal pellet shape used for grid interfaces. */
interface PelletLike {
  x: number;
  y: number;
  v: number;
}

/** Pellet grid interface used by the snake for local queries. */
interface PelletGridLike {
  map?: Map<string, PelletLike[]>;
  cellSize?: number;
  forEachInRadius?: (x: number, y: number, r: number, fn: (p: Pellet) => void) => void;
}

/** Particle system interface used for snake effects. */
interface ParticleSystemLike {
  spawnBurst: (x: number, y: number, color: string, count: number, strength: number) => void;
  spawnBoost: (x: number, y: number, ang: number, color: string) => void;
}

/** World interface required by the snake for updates and pellet ops. */
interface WorldLike {
  pellets: Pellet[];
  pelletGrid?: PelletGridLike;
  particles: ParticleSystemLike;
  addPellet: (p: Pellet) => void;
  removePellet: (p: Pellet) => void;
  bestPointsThisGen: number;
  baselineBotDied?: (snake: Snake) => void;
  /** Optional profiler for timing breakdowns. */
  profiler?: SimProfiler;
}

/** External control input for turn and boost values. */
export type ControlInput = { turn: number; boost: number };

/** Control modes supported by the snake update loop. */
export type ControlMode = 'neural' | 'external-only';

/** Optional parameters for spawning a snake. */
export interface SnakeSpawnOptions {
  /** RNG used for spawn position and heading. */
  rng?: RandomSource;
  /** Control mode for the snake. */
  controlMode?: ControlMode;
  /** Optional brain override for the snake. */
  brain?: Brain;
  /** Optional baseline bot index identifier. */
  baselineBotIndex?: number | null;
  /** Optional skin flag for rendering. */
  skin?: number;
}

/** Optional overrides for death side effects. */
export interface SnakeDeathOptions {
  /** Whether to drop corpse pellets after death. */
  dropPellets?: boolean;
}

/**
 * Computes a snake’s radius as a function of its length using a
 * logarithmic growth curve.  The radius increases slowly with length
 * until clamped at snakeRadiusMax.
 */
function computeSnakeRadiusByLen(len: number): number {
  const grow = Math.max(0, len - CFG.snakeStartLen);
  const div = Math.max(1e-6, CFG.snakeThicknessLogDiv);
  const r = CFG.snakeRadius + CFG.snakeThicknessScale * Math.log1p(grow / div);
  return clamp(r, CFG.snakeRadius, CFG.snakeRadiusMax);
}

/**
 * Computes a snake's turn rate (angular velocity) as a function of its length.
 * Larger snakes turn slower, matching Slither.io physics where the turning
 * circle expands more rapidly than girth.
 */
export function computeSnakeTurnRateByLen(len: number): number {
  const denom = Math.max(1, CFG.snakeMaxLen - CFG.snakeStartLen);
  const sn = clamp((len - CFG.snakeStartLen) / denom, 0, 1);
  // Turn rate scales down by a factor of (1 + penalty * sn).
  // At max length (sn=1), turn rate is base / (1 + snakeTurnPenalty).
  const penalty = 1.0 + (CFG.snakeTurnPenalty ?? 1.5) * sn;
  return CFG.snakeTurnRate / penalty;
}

/**
 * Represents an individual snake in the simulation.  Each snake has
 * position, direction, body segments, a brain and a genome from which
 * the brain is constructed.  Snakes manage their own growth, boosting
 * logic, food collection and state updates.
 */
export class Snake {
  /** Unique snake identifier. */
  id: number;
  /** Render color for the snake. */
  color: string;
  /** World X position for the head. */
  x: number;
  /** World Y position for the head. */
  y: number;
  /** Heading angle in radians. */
  dir: number;
  /** Current radius computed from length. */
  radius: number;
  /** Current speed in world units per second. */
  speed: number;
  /** Boost state flag as numeric value. */
  boost: number;
  /** Whether the snake is alive. */
  alive: boolean;
  /** Total number of pellets eaten. */
  foodEaten: number;
  /** Age in seconds since spawn. */
  age: number;
  /** Accumulated kill score. */
  killScore: number;
  /** Accumulated points score. */
  pointsScore: number;
  /** Target length for growth and shrink updates. */
  targetLen: number;
  /** Body segment points from head to tail. */
  points: Point[];
  /** Genome used to build the brain. */
  genome: Genome;
  /** Brain instance used for control decisions. */
  brain: Brain;
  /** Latest turn input applied. */
  turnInput: number;
  /** Latest boost input applied. */
  boostInput: number;
  /** Scratch buffer for sensor values. */
  _sensorBuf?: Float32Array;
  /** Scratch buffer for last output values. */
  _outputBuf?: Float32Array;
  /** Accumulator for control update timing. */
  _ctrlAcc?: number;
  /** Flag indicating control action availability. */
  _hasAct?: number;
  /** Whether the last control input was external. */
  _lastControlExternal?: boolean;
  /** Cached last sensor vector for debug UI. */
  lastSensors?: Float32Array;
  /** Cached last output vector for debug UI. */
  lastOutputs?: Float32Array;
  /** Cached fitness value for reporting. */
  fitness?: number;
  /** Control mode for this snake. */
  controlMode: ControlMode;
  /** Baseline bot identity index or null. */
  baselineBotIndex: number | null;
  /** Skin flag used for rendering. */
  skin: number;

  /**
   * Create a new snake instance with a generated brain.
   * @param id - Unique snake id.
   * @param genome - Genome used to build the brain.
   * @param arch - Architecture definition for the brain.
   * @param options - Optional spawn overrides.
   */
  constructor(id: number, genome: Genome, arch: ArchDefinition, options: SnakeSpawnOptions = {}) {
    this.id = id;
    this.color = hashColor(id * 17 + 3);
    // Spawn at a random position and orientation within a fraction of the arena.
    const rng = options.rng ?? Math.random;
    const a = rng() * TAU;
    const r = Math.sqrt(rng()) * (CFG.worldRadius * 0.60);
    this.x = Math.cos(a) * r;
    this.y = Math.sin(a) * r;
    this.dir = rng() * TAU;
    this.radius = CFG.snakeRadius;
    this.speed = CFG.snakeBaseSpeed;
    this.boost = 0;
    this.alive = true;
    this.foodEaten = 0;
    this.age = 0;
    this.killScore = 0;
    this.pointsScore = 0;
    this.targetLen = CFG.snakeStartLen;
    this.points = [];
    this._initBody();
    this.genome = genome;
    this.brain = options.brain ?? genome.buildBrain(arch);
    this.turnInput = 0;
    this.boostInput = 0;
    this.controlMode = options.controlMode ?? 'neural';
    this.baselineBotIndex = options.baselineBotIndex ?? null;
    this.skin = options.skin ?? 0;
    this.updateRadiusFromLen();
  }
  /**
   * Builds the initial body by laying out points behind the head.
   */
  _initBody(): void {
    this.points.length = 0;
    let px = this.x,
      py = this.y;
    this.points.push({ x: px, y: py });
    for (let i = 1; i < CFG.snakeStartLen; i++) {
      px -= Math.cos(this.dir) * CFG.snakeSpacing;
      py -= Math.sin(this.dir) * CFG.snakeSpacing;
      this.points.push({ x: px, y: py });
    }
  }
  /**
   * Returns the head segment.
   */
  head(): Point {
    return this.points[0]!;
  }
  /**
   * Current number of segments in the body.
   */
  length(): number {
    return this.points.length;
  }
  /**
   * Normalised size fraction relative to the start and maximum length.
   */
  sizeNorm(): number {
    const denom = Math.max(1, CFG.snakeMaxLen - CFG.snakeStartLen);
    return clamp((this.length() - CFG.snakeStartLen) / denom, 0, 1);
  }
  /**
   * Updates the radius field to reflect the current length.
   */
  updateRadiusFromLen(): void {
    this.radius = computeSnakeRadiusByLen(this.length());
  }
  /**
   * Kill the snake and optionally drop pellets behind it. Only applies once.
   * @param world - World context for pellet spawning.
   * @param options - Optional overrides for death side effects.
   */
  die(world: WorldLike, options?: SnakeDeathOptions): void {
    if (!this.alive) return;
    this.alive = false;
    world.particles.spawnBurst(this.x, this.y, this.color, 25, 3.0);
    const dropPellets = options?.dropPellets !== false;
    if (!dropPellets) {
      if (world.baselineBotDied) {
        world.baselineBotDied(this);
      }
      return;
    }
    // Approximate slither.io style remains: larger, brighter pellets along the
    // corpse, with lossy mass recycling for very large snakes.
    const len = this.length();
    if (len <= 0) return;

    const dcfg = CFG.death || {};
    const sn = this.sizeNorm();
    const fracSmall = dcfg.dropFracSmall ?? 0.95;
    const fracLarge = dcfg.dropFracLarge ?? 0.33;
    const pow = dcfg.dropFracPow ?? 1.6;
    const frac = clamp(fracSmall - (fracSmall - fracLarge) * Math.pow(sn, pow), 0, 1);

    // Treat segment count as "mass". Convert a fraction of it into edible value.
    const dropLen = Math.max(0, len * frac);
    const totalValue = dropLen / Math.max(1e-6, CFG.growPerFood);

    const bigVBase = Math.max(0.05, CFG.foodValue * (dcfg.bigPelletValueFactor ?? 3.0));
    const smallVBase = Math.max(0.02, CFG.foodValue * (dcfg.smallPelletValueFactor ?? 1.0));
    const bigShare = clamp(dcfg.bigShare ?? 0.78, 0, 1);

    let bigBudget = totalValue * bigShare;
    let smallBudget = Math.max(0, totalValue - bigBudget);

    let bigCount = Math.max(1, Math.floor(bigBudget / bigVBase));
    let smallCount = Math.max(0, Math.floor(smallBudget / smallVBase));

    const maxPellets = Math.max(20, Math.floor(dcfg.maxPellets ?? 420));
    const totalCount = bigCount + smallCount;
    if (totalCount > maxPellets) {
      const scale = maxPellets / totalCount;
      bigCount = Math.max(1, Math.floor(bigCount * scale));
      smallCount = Math.max(0, Math.floor(smallCount * scale));
    }

    const jitter = dcfg.jitter ?? 8;
    const clusterJitter = dcfg.clusterJitter ?? 14;
    const useSnakeColor = dcfg.useSnakeColor !== false;
    const corpseColor = useSnakeColor && typeof this.color === "string" && this.color.startsWith("rgb(")
      ? this.color.replace("rgb(", "rgba(").replace(")", ",0.90)")
      : null;

    // Big orbs: placed evenly along the body, value varies slightly.
    if (bigCount <= 1) {
      const p = this.points[0];
      if (!p) return;
      const v = bigVBase * (0.85 + Math.random() * 0.30);
      world.addPellet(new Pellet(p.x + rand(jitter, -jitter), p.y + rand(jitter, -jitter), v, corpseColor, "corpse_big", this.id));
    } else {
      for (let k = 0; k < bigCount; k++) {
        const idx = Math.floor((k * (len - 1)) / (bigCount - 1));
        const p = this.points[idx];
        if (!p) continue;
        const v = bigVBase * (0.85 + Math.random() * 0.30);
        world.addPellet(new Pellet(p.x + rand(jitter, -jitter), p.y + rand(jitter, -jitter), v, corpseColor, "corpse_big", this.id));
      }
    }

    // Small filler orbs: placed along the body and lightly clustered.
    if (smallCount > 0) {
      for (let k = 0; k < smallCount; k++) {
        const idx = Math.floor((k * (len - 1)) / Math.max(1, smallCount));
        const p = this.points[idx];
        if (!p) continue;
        const v = smallVBase * (0.80 + Math.random() * 0.40);
        world.addPellet(
          new Pellet(
            p.x + rand(clusterJitter, -clusterJitter),
            p.y + rand(clusterJitter, -clusterJitter),
            v,
            corpseColor,
            "corpse_small",
            this.id
          )
        );
      }
    }
    if (world.baselineBotDied) {
      world.baselineBotDied(this);
    }
  }

  /**
   * Compute the fitness score according to the configured reward weights.
   * @param pointsNorm - Normalized points score in [0,1].
   * @param topPointsBonus - Bonus applied to top performers.
   */
  computeFitness(pointsNorm: number, topPointsBonus: number): number {
    const len = this.length();
    const survive = this.age;
    const eat = this.foodEaten;
    const grow = Math.max(0, len - CFG.snakeStartLen);
    const kill = this.killScore;
    return (
      survive * CFG.reward.fitnessSurvivalPerSecond +
      eat * CFG.reward.fitnessFood +
      grow * CFG.reward.fitnessLengthPerSegment +
      kill * CFG.reward.fitnessKill +
      pointsNorm * CFG.reward.fitnessPointsNorm +
      topPointsBonus
    );
  }
  /**
   * Burn points to enable boosting and shrink the snake accordingly.
   * @internal
   * @param world - World context for pellet spawning.
   * @param dt - Delta time in seconds.
   * @returns Points spent this frame.
   */
  _applyBoostMassBurn(world: WorldLike, dt: number): number {
    const lenNow = this.length();
    if (lenNow <= CFG.snakeMinLen + 1) return 0;
    if (this.pointsScore < CFG.boost.minPointsToBoost) return 0;
    const sn = this.sizeNorm();
    const costRate = CFG.boost.pointsCostPerSecond * (1 + CFG.boost.pointsCostSizeFactor * sn);
    const spend = Math.min(this.pointsScore, costRate * dt);
    if (spend <= 0) return 0;
    this.pointsScore -= spend;
    const loss = spend * CFG.boost.lenLossPerPoint;
    this.targetLen = clamp(this.targetLen - loss, CFG.snakeMinLen, CFG.snakeMaxLen);
    const desired = Math.floor(this.targetLen);
    const dropV = Math.max(0.02, CFG.foodValue * CFG.boost.pelletValueFactor);
    const jitter = CFG.boost.pelletJitter;
    const boostColor = typeof this.color === "string" && this.color.startsWith("rgb(")
      ? this.color.replace("rgb(", "rgba(").replace(")", ",0.70)")
      : null;
    while (this.points.length > desired) {
      const tail = this.points.pop()!;
      const back = this.points[this.points.length - 1] || tail;
      const dx = tail.x - back.x;
      const dy = tail.y - back.y;
      const dist = Math.hypot(dx, dy) || 1e-6;
      const ux = dx / dist;
      const uy = dy / dist;
      world.addPellet(
        new Pellet(
          tail.x + ux * 8 + rand(jitter, -jitter),
          tail.y + uy * 8 + rand(jitter, -jitter),
          dropV,
          boostColor,
          "boost",
          this.id
        )
      );
    }
    return spend;
  }
  /**
   * Prepare per-step bookkeeping before control evaluation.
   * @param dt - Delta time in seconds.
   */
  prepareForStep(dt: number): void {
    if (!this.points.length) this.points.push({ x: this.x, y: this.y });
    this.age += dt;
    this.pointsScore += dt * CFG.reward.pointsPerSecondAlive;
  }
  /**
   * Computes sensors into the provided buffer (or the internal buffer)
   * without mutating snake state beyond the sensor scratch space.
   */
  computeSensors(world: WorldLike, out?: Float32Array): Float32Array {
    const expected = CFG.brain.inSize;
    if (out && out.length === expected) return buildSensors(world, this, out);
    if (!this._sensorBuf || this._sensorBuf.length !== expected) {
      this._sensorBuf = new Float32Array(expected);
    }
    return buildSensors(world, this, this._sensorBuf);
  }
  /**
   * Sync control state when switching between external and neural inputs.
   * @param usingExternal - Whether external control is active.
   */
  private _syncControlSource(usingExternal: boolean): void {
    if (usingExternal !== (this._lastControlExternal ?? false)) {
      this.brain.reset();
      this._ctrlAcc = 0;
      this._hasAct = 0;
    }
    this._lastControlExternal = usingExternal;
  }
  /**
   * Apply external control inputs (player or bot) to the snake.
   * @param control - External control input.
   */
  applyExternalControl(control?: ControlInput): void {
    this._syncControlSource(true);
    const turn = control?.turn ?? 0;
    const boost = control?.boost ?? 0;
    this.turnInput = clamp(turn, -1, 1);
    this.boostInput = clamp(boost, 0, 1);
  }
  /**
   * Update the control accumulator and report whether inference is needed.
   * @param dt - Delta time in seconds.
   * @returns True when a new brain output should be computed.
   */
  needsControlUpdate(dt: number): boolean {
    this._syncControlSource(false);
    if (this._ctrlAcc == null) this._ctrlAcc = 0;
    const ctrlDt = Math.max(0.001, (CFG.brain && CFG.brain.controlDt) ? CFG.brain.controlDt : 1 / 60);
    this._ctrlAcc += dt;
    if (!this._hasAct || this._ctrlAcc >= ctrlDt) {
      this._ctrlAcc = this._ctrlAcc % ctrlDt;
      return true;
    }
    return false;
  }
  /**
   * Apply raw brain outputs to control inputs.
   * @param turn - Raw turn output.
   * @param boost - Raw boost output.
   */
  applyBrainOutput(turn: number, boost: number): void {
    const outSize = Math.max(1, Math.floor(CFG.brain.outSize));
    if (!this._outputBuf || this._outputBuf.length !== outSize) {
      this._outputBuf = new Float32Array(outSize);
    }
    this._outputBuf[0] = turn;
    if (outSize > 1) this._outputBuf[1] = boost;
    this.lastOutputs = this._outputBuf;
    this.turnInput = clamp(turn, -1, 1);
    this.boostInput = clamp(boost, -1, 1);
    this._hasAct = 1;
  }
  /**
   * Advance movement, boosting, feeding, and growth using current inputs.
   * @param world - World context for collisions and pellets.
   * @param dt - Delta time in seconds.
   */
  advance(world: WorldLike, dt: number): void {
    if (!this.alive) return;
    const boostWanted = this.boostInput > 0.35;
    let boostingNow = 0;
    if (boostWanted) {
      const spent = this._applyBoostMassBurn(world, dt);
      boostingNow = spent > 0 ? 1 : 0;
    }
    this.boost = boostingNow;
    if (this.boost) {
      // Emit boost particles
      world.particles.spawnBoost(this.x, this.y, this.dir, this.color);
    }
    const sn = this.sizeNorm();
    const baseNow = CFG.snakeBaseSpeed * (1 - CFG.snakeSizeSpeedPenalty * sn);
    const ratio = CFG.snakeBoostSpeed / Math.max(1e-6, CFG.snakeBaseSpeed);
    const boostMultBase = Math.max(0, ratio - 1);
    const boostMultEff = boostMultBase * (1 - CFG.snakeBoostSizePenalty * sn);
    const boostNow = baseNow * (1 + Math.max(0, boostMultEff));
    const targetSpeed = this.boost ? boostNow : baseNow;
    this.speed = lerp(this.speed, targetSpeed, 1 - Math.exp(-dt * 6.5));
    const turnRate = computeSnakeTurnRateByLen(this.length());
    this.dir = angNorm(this.dir + this.turnInput * turnRate * dt);
    this.x += Math.cos(this.dir) * this.speed * dt;
    this.y += Math.sin(this.dir) * this.speed * dt;
    const d = hypot(this.x, this.y);
    if (d + this.radius >= CFG.worldRadius) {
      this.die(world, { dropPellets: false });
      return;
    }
    const head = this.points[0];
    if (head) {
      head.x = this.x;
      head.y = this.y;
    }
    for (let i = 1; i < this.points.length; i++) {
      const prev = this.points[i - 1];
      const cur = this.points[i];
      if (!prev || !cur) continue;
      const dx = cur.x - prev.x;
      const dy = cur.y - prev.y;
      const dist = Math.hypot(dx, dy) || 1e-6;
      const desired = CFG.snakeSpacing;
      const t = (dist - desired) / dist;
      cur.x -= dx * t;
      cur.y -= dy * t;
    }
    // Eat food
    const eatR = this.radius + 6;
    const eatR2 = eatR * eatR;
    // Fast local pellet collection using the pellet grid.
    const candidates: Pellet[] = [];
    if (world.pelletGrid?.forEachInRadius) {
      world.pelletGrid.forEachInRadius(this.x, this.y, eatR, p => candidates.push(p));
    } else {
      for (let i = 0; i < world.pellets.length; i++) {
        const pellet = world.pellets[i];
        if (pellet) candidates.push(pellet);
      }
    }
    for (let k = 0; k < candidates.length; k++) {
      const p = candidates[k];
      if (!p) continue;
      const dx = p.x - this.x;
      const dy = p.y - this.y;
      if (dx * dx + dy * dy <= eatR2) {
        world.removePellet(p);
        this.foodEaten += p.v;
        this.pointsScore += p.v * CFG.reward.pointsPerFood;
        this.targetLen = clamp(
          this.targetLen + CFG.growPerFood * p.v,
          CFG.snakeMinLen,
          CFG.snakeMaxLen
        );
      }
    }
    // Grow or shrink to target length

    while (this.points.length < Math.floor(this.targetLen)) {
      const tail = this.points[this.points.length - 1];
      if (!tail) {
        this.points.push({ x: this.x, y: this.y });
        continue;
      }
      const before = this.points[this.points.length - 2] || tail;
      const dx = tail.x - before.x;
      const dy = tail.y - before.y;
      const dist = Math.hypot(dx, dy) || 1e-6;
      const ux = dx / dist;
      const uy = dy / dist;
      this.points.push({ x: tail.x + ux * CFG.snakeSpacing, y: tail.y + uy * CFG.snakeSpacing });
    }
    const desired = Math.floor(clamp(this.targetLen, CFG.snakeMinLen, CFG.snakeMaxLen));
    while (this.points.length > desired) this.points.pop();
    this.updateRadiusFromLen();
  }
  /**
   * Main update routine invoked once per substep by the World.
   * Handles sensor evaluation, neural network inference, movement, food
   * collection, and growth/shrink logic.
   * @param world - World context for collisions and pellets.
   * @param dt - Delta time in seconds.
   * @param control - Optional external control input.
   */
  update(world: WorldLike, dt: number, control?: ControlInput): void {
    if (!this.alive) return;
    this.prepareForStep(dt);
    const externalOnly = this.controlMode === 'external-only';
    const usingExternal = externalOnly || !!control;
    if (usingExternal) {
      this.applyExternalControl(control);
    } else if (this.needsControlUpdate(dt)) {
      const profiler = world.profiler;
      let sensors: Float32Array;
      if (profiler) {
        const start = profiler.now();
        sensors = this.computeSensors(world, this._sensorBuf);
        profiler.recordSensors(profiler.now() - start);
      } else {
        sensors = this.computeSensors(world, this._sensorBuf);
      }
      this.lastSensors = sensors;
      let out: Float32Array;
      if (profiler) {
        const start = profiler.now();
        out = this.brain.forward(sensors);
        profiler.recordBrain(profiler.now() - start);
      } else {
        out = this.brain.forward(sensors);
      }
      this.applyBrainOutput(out[0] ?? 0, out[1] ?? 0);
    }
    this.advance(world, dt);
  }
}

/**
 * Returns the squared distance from point (px,py) to segment (ax,ay)-(bx,by).
 * Used by collision detection.
 */
export function pointSegmentDist2(
  px: number,
  py: number,
  ax: number,
  ay: number,
  bx: number,
  by: number
): number {
  const abx = bx - ax;
  const aby = by - ay;
  const apx = px - ax;
  const apy = py - ay;
  const denom = abx * abx + aby * aby;
  if (denom <= 1e-9) {
    const dx = px - ax;
    const dy = py - ay;
    return dx * dx + dy * dy;
  }
  let t = (apx * abx + apy * aby) / denom;
  t = clamp(t, 0, 1);
  const cx = ax + abx * t;
  const cy = ay + aby * t;
  const dx = px - cx;
  const dy = py - cy;
  return dx * dx + dy * dy;
}

/**
 * Spatial hash grid for speeding up collision checks.  Each segment of
 * every snake is placed into a cell based on its midpoint.  Neighboring
 * cells can then be queried during collision resolution.
 */
export class SegmentGrid {
  /** Spatial hash cell size in world units. */
  cellSize: number;
  /** Map of cell keys to snake segment references. */
  map: Map<string, Array<{ s: Snake; i: number }>>;

  constructor() {
    this.cellSize = CFG.collision.cellSize;
    this.map = new Map();
  }
  /**
   * Updates the cell size from the current CFG and clears the map.
   */
  resetForCFG(): void {
    this.cellSize = Math.max(1, CFG.collision.cellSize);
    this.map.clear();
  }
  /**
   * Builds a unique key for a cell coordinate.
   */
  _key(cx: number, cy: number): string {
    return cx + "," + cy;
  }
  /**
   * Add a segment of a snake into the appropriate cell.
   * The segment index must be at least 1 so that it references a valid segment
   * between points[idx-1] and points[idx].
   */
  addSegment(snake: Snake, idx: number): void {
    const p0 = snake.points[idx - 1];
    const p1 = snake.points[idx];
    if (!p0 || !p1) return;
    const mx = (p0.x + p1.x) * 0.5;
    const my = (p0.y + p1.y) * 0.5;
    const cx = Math.floor(mx / this.cellSize);
    const cy = Math.floor(my / this.cellSize);
    const k = this._key(cx, cy);
    let arr = this.map.get(k);
    if (!arr) {
      arr = [];
      this.map.set(k, arr);
    }
    arr.push({ s: snake, i: idx });
  }
  /**
   * Populate the grid with segments from all alive snakes.
   * @param snakes - Snakes to insert into the spatial grid.
   */
  build(snakes: Snake[]): void {
    this.resetForCFG();
    const skip = Math.max(0, Math.floor(CFG.collision.skipSegments));
    for (const s of snakes) {
      if (!s.alive) continue;
      const pts = s.points;
      for (let i = Math.max(1, skip); i < pts.length; i++) this.addSegment(s, i);
    }
  }
  /**
   * Look up all segments in the cell (cx,cy).
   * @param cx - Cell x coordinate.
   * @param cy - Cell y coordinate.
   * @returns Segment list or null when empty.
   */
  query(cx: number, cy: number): Array<{ s: Snake; i: number }> | null {
    return this.map.get(this._key(cx, cy)) || null;
  }
}
