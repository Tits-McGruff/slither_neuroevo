// snake.js
// Definitions of Pellet, Snake and segment grid classes.  These objects
// encapsulate the state and behaviour of the snakes and provide
// geometry helpers for collision detection.

import { CFG } from './config.js';
import { clamp, hashColor, rand, lerp, angNorm, hypot, TAU } from './utils.js';
import { buildSensors } from './sensors.js';

/**
 * Simple data class representing a pellet at (x,y) with value v.
 * @class
 */
export class Pellet {
  constructor(x, y, v, color = null, kind = "ambient") {
    this.x = x;
    this.y = y;
    this.v = v;
    this.color = color;
    this.kind = kind;
  }
}

/**
 * Computes a snake’s radius as a function of its length using a
 * logarithmic growth curve.  The radius increases slowly with length
 * until clamped at snakeRadiusMax.
 * @param {number} len
 * @returns {number}
 */
function computeSnakeRadiusByLen(len) {
  const grow = Math.max(0, len - CFG.snakeStartLen);
  const div = Math.max(1e-6, CFG.snakeThicknessLogDiv);
  const r = CFG.snakeRadius + CFG.snakeThicknessScale * Math.log1p(grow / div);
  return clamp(r, CFG.snakeRadius, CFG.snakeRadiusMax);
}

/**
 * Represents an individual snake in the simulation.  Each snake has
 * position, direction, body segments, a brain and a genome from which
 * the brain is constructed.  Snakes manage their own growth, boosting
 * logic, food collection and state updates.
 */
export class Snake {
  constructor(id, genome, arch) {
    this.id = id;
    this.color = hashColor(id * 17 + 3);
    // Spawn at a random position and orientation within a fraction of the arena.
    const a = Math.random() * TAU;
    const r = Math.sqrt(Math.random()) * (CFG.worldRadius * 0.60);
    this.x = Math.cos(a) * r;
    this.y = Math.sin(a) * r;
    this.dir = Math.random() * TAU;
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
    this.brain = genome.buildBrain(arch);
    this.turnInput = 0;
    this.boostInput = 0;
    this.updateRadiusFromLen();
  }
  /**
   * Builds the initial body by laying out points behind the head.
   */
  _initBody() {
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
  head() {
    return this.points[0];
  }
  /**
   * Current number of segments in the body.
   */
  length() {
    return this.points.length;
  }
  /**
   * Normalised size fraction relative to the start and maximum length.
   */
  sizeNorm() {
    const denom = Math.max(1, CFG.snakeMaxLen - CFG.snakeStartLen);
    return clamp((this.length() - CFG.snakeStartLen) / denom, 0, 1);
  }
  /**
   * Updates the radius field to reflect the current length.
   */
  updateRadiusFromLen() {
    this.radius = computeSnakeRadiusByLen(this.length());
  }
  /**
   * Kills the snake and drops pellets behind it.  Only applies once.
   * @param {World} world
   */
  die(world) {
    if (!this.alive) return;
    this.alive = false;
    world.particles.spawnBurst(this.x, this.y, this.color, 25, 3.0);
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
      const v = bigVBase * (0.85 + Math.random() * 0.30);
      world.addPellet(new Pellet(p.x + rand(jitter, -jitter), p.y + rand(jitter, -jitter), v, corpseColor, "corpse_big"));
    } else {
      for (let k = 0; k < bigCount; k++) {
        const idx = Math.floor((k * (len - 1)) / (bigCount - 1));
        const p = this.points[idx];
        const v = bigVBase * (0.85 + Math.random() * 0.30);
        world.addPellet(new Pellet(p.x + rand(jitter, -jitter), p.y + rand(jitter, -jitter), v, corpseColor, "corpse_big"));
      }
    }

    // Small filler orbs: placed along the body and lightly clustered.
    if (smallCount > 0) {
      for (let k = 0; k < smallCount; k++) {
        const idx = Math.floor((k * (len - 1)) / Math.max(1, smallCount));
        const p = this.points[idx];
        const v = smallVBase * (0.80 + Math.random() * 0.40);
        world.addPellet(
          new Pellet(
            p.x + rand(clusterJitter, -clusterJitter),
            p.y + rand(clusterJitter, -clusterJitter),
            v,
            corpseColor,
            "corpse_small"
          )
        );
      }
    }
  }
  /**
   * Computes the fitness score according to the configured reward weights.
   * @param {number} pointsNorm Normalised points score in [0,1].
   * @param {number} topPointsBonus Bonus applied to top performers.
   */
  computeFitness(pointsNorm, topPointsBonus) {
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
   * Burns points to enable boosting and shrinks the snake accordingly.
   * @private
   * @param {World} world
   * @param {number} dt
   * @returns {number} Points spent this frame
   */
  _applyBoostMassBurn(world, dt) {
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
      const tail = this.points.pop();
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
          "boost"
        )
      );
    }
    return spend;
  }
  /**
   * Main update routine.  Invoked once per substep by the World.  Handles
   * sensor evaluation, neural network inference, movement, food collection
   * and growth/shrink logic.
   * @param {World} world
   * @param {number} dt
   */
  update(world, dt) {
    if (!this.alive) return;
    this.age += dt;
    this.pointsScore += dt * CFG.reward.pointsPerSecondAlive;
    if (!this._sensorBuf || this._sensorBuf.length !== CFG.brain.inSize) this._sensorBuf = new Float32Array(CFG.brain.inSize);
    if (this._ctrlAcc == null) this._ctrlAcc = 0;
    const ctrlDt = Math.max(0.001, (CFG.brain && CFG.brain.controlDt) ? CFG.brain.controlDt : 1 / 60);
    this._ctrlAcc += dt;
    // Only evaluate the brain on a fixed controller step. Movement and
    // collision substepping can change dt, so this keeps "memory" stable.
    if (!this._hasAct || this._ctrlAcc >= ctrlDt) {
      this._ctrlAcc = this._ctrlAcc % ctrlDt;
      const sensors = buildSensors(world, this, this._sensorBuf);
      this.lastSensors = Array.from(sensors);
      const out = this.brain.forward(sensors);
      this.lastOutputs = Array.from(out);
      this.turnInput = clamp(out[0], -1, 1);
      this.boostInput = clamp(out[1], -1, 1);
      this._hasAct = 1;
    }
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
    this.dir = angNorm(this.dir + this.turnInput * CFG.snakeTurnRate * dt);
    this.x += Math.cos(this.dir) * this.speed * dt;
    this.y += Math.sin(this.dir) * this.speed * dt;
    const d = hypot(this.x, this.y);
    if (d > CFG.worldRadius) {
      const angToCenter = Math.atan2(-this.y, -this.x);
      const delta = angNorm(angToCenter - this.dir);
      this.dir = angNorm(this.dir + clamp(delta, -1.0, 1.0) * dt * 4.0);
      this.x = (this.x / d) * CFG.worldRadius;
      this.y = (this.y / d) * CFG.worldRadius;
    }
    this.points[0].x = this.x;
    this.points[0].y = this.y;
    for (let i = 1; i < this.points.length; i++) {
      const prev = this.points[i - 1];
      const cur = this.points[i];
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
const candidates = [];
if (world.pelletGrid) {
  world.pelletGrid.forEachInRadius(this.x, this.y, eatR, p => candidates.push(p));
} else {
  for (let i = 0; i < world.pellets.length; i++) candidates.push(world.pellets[i]);
}
for (let k = 0; k < candidates.length; k++) {
  const p = candidates[k];
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
}

/**
 * Returns the squared distance from point (px,py) to segment (ax,ay)-(bx,by).
 * Used by collision detection.
 */
export function pointSegmentDist2(px, py, ax, ay, bx, by) {
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
  constructor() {
    this.cellSize = CFG.collision.cellSize;
    this.map = new Map();
  }
  /**
   * Updates the cell size from the current CFG and clears the map.
   */
  resetForCFG() {
    this.cellSize = Math.max(1, CFG.collision.cellSize);
    this.map.clear();
  }
  /**
   * Builds a unique key for a cell coordinate.
   */
  _key(cx, cy) {
    return cx + "," + cy;
  }
  /**
   * Adds a segment of a snake into the appropriate cell.  The segment
   * index must be >=1 so that it references a valid segment between
   * points[idx-1] and points[idx].
   */
  addSegment(snake, idx) {
    const p0 = snake.points[idx - 1];
    const p1 = snake.points[idx];
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
   * Populates the grid with segments from all alive snakes.
   * @param {Array<Snake>} snakes
   */
  build(snakes) {
    this.resetForCFG();
    const skip = Math.max(0, Math.floor(CFG.collision.skipSegments));
    for (const s of snakes) {
      if (!s.alive) continue;
      const pts = s.points;
      for (let i = Math.max(1, skip); i < pts.length; i++) this.addSegment(s, i);
    }
  }
  /**
   * Looks up all segments in the cell (cx,cy).
   * @param {number} cx
   * @param {number} cy
   * @returns {Array<{s: Snake, i: number}>|null}
   */
  query(cx, cy) {
    return this.map.get(this._key(cx, cy)) || null;
  }
}