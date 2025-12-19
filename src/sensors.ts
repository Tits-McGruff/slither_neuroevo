// sensors.ts
// Functions for constructing the input vector fed into each snake’s neural
// network.  These functions gather information about the environment
// (food, threats, obstacles) and encode it into a fixed length array.

import { CFG } from './config.ts';
import { clamp, angNorm, TAU } from './utils.ts';

interface PelletLike {
  x: number;
  y: number;
  v: number;
}

interface SnakePoint {
  x: number;
  y: number;
}

interface SnakeLike {
  x: number;
  y: number;
  dir: number;
  pointsScore: number;
  points: SnakePoint[];
  radius: number;
  alive: boolean;
  length: () => number;
  sizeNorm: () => number;
}

interface SegmentRef {
  s: SnakeLike;
  i: number;
}

interface WorldLike {
  pellets: PelletLike[];
  bestPointsThisGen: number;
  pelletGrid?: { map?: Map<string, PelletLike[]>; cellSize?: number };
  _collGrid?: { map?: Map<string, unknown>; cellSize?: number; query?: (cx: number, cy: number) => SegmentRef[] | null };
}

function _closestPointOnSegment(px: number, py: number, ax: number, ay: number, bx: number, by: number) {
  const abx = bx - ax;
  const aby = by - ay;
  const apx = px - ax;
  const apy = py - ay;
  const ab2 = abx * abx + aby * aby;
  let t = 0;
  if (ab2 > 1e-9) t = (apx * abx + apy * aby) / ab2;
  t = clamp(t, 0, 1);
  const qx = ax + abx * t;
  const qy = ay + aby * t;
  const dx = px - qx;
  const dy = py - qy;
  const d2 = dx * dx + dy * dy;
  return { qx, qy, d2 };
}

/**
 * Computes forward distance along a ray from (x,y) to the arena wall.
 * Assumes the point is inside or on the wall.
 */
function _distToWallAlongRay(x: number, y: number, theta: number, R: number): number {
  const ux = Math.cos(theta);
  const uy = Math.sin(theta);
  const b = x * ux + y * uy;
  const c = x * x + y * y - R * R;
  const disc = b * b - c;
  if (disc <= 0) return 0;
  // Positive root gives distance in the forward direction.
  return -b + Math.sqrt(disc);
}

/**
 * Iterates segments from the world collision grid in cells intersecting a radius
 * around (x,y). The callback receives {s, i} where i is the segment end index
 * in s.points (segment is i-1 -> i).
 */
function _forEachNearbySegment(
  world: WorldLike,
  x: number,
  y: number,
  r: number,
  fn: (ref: SegmentRef) => void
): void {
  const grid = world._collGrid;
  if (!grid || !grid.map || !grid.query) return;
  const cs = Math.max(1, grid.cellSize || CFG.collision.cellSize);
  // Segment midpoints are hashed; pad by ~1.5 cells to reduce misses.
  const pad = cs * 1.5;
  const rr = r + pad;
  const minCx = Math.floor((x - rr) / cs);
  const maxCx = Math.floor((x + rr) / cs);
  const minCy = Math.floor((y - rr) / cs);
  const maxCy = Math.floor((y + rr) / cs);
  let checks = 0;
  const maxChecks = Math.max(200, CFG.sense?.maxSegmentChecks ?? 1600);
  for (let cy = minCy; cy <= maxCy; cy++) {
    for (let cx = minCx; cx <= maxCx; cx++) {
      const arr = grid.query(cx, cy);
      if (!arr) continue;
      for (let j = 0; j < arr.length; j++) {
        fn(arr[j]);
        checks++;
        if (checks >= maxChecks) return;
      }
    }
  }
}

/**
 * Iterates pellets from the world pellet grid in a radius around (x,y).
 * The callback receives the pellet object.
 *
 * This version supports early exit and bounds the amount of work via
 * CFG.sense.maxPelletChecks.
 */
function _forEachNearbyPellet(
  world: WorldLike,
  x: number,
  y: number,
  r: number,
  fn: (p: PelletLike) => boolean | void
): void {
  const grid = world.pelletGrid;
  if (!grid || !grid.map) {
    // Fallback: iterate the raw array with a cap.
    const maxChecks = Math.max(120, CFG.sense?.maxPelletChecks ?? 900);
    const step = Math.max(1, Math.ceil(world.pellets.length / maxChecks));
    for (let i = 0; i < world.pellets.length; i += step) {
      const p = world.pellets[i];
      const dx = p.x - x;
      const dy = p.y - y;
      if (dx * dx + dy * dy > r * r) continue;
      if (fn(p) === false) return;
    }
    return;
  }

  const cs = Math.max(10, grid.cellSize || (CFG.pelletGrid?.cellSize ?? 120));
  const minCx = Math.floor((x - r) / cs);
  const maxCx = Math.floor((x + r) / cs);
  const minCy = Math.floor((y - r) / cs);
  const maxCy = Math.floor((y + r) / cs);
  const r2 = r * r;
  const maxChecks = Math.max(120, CFG.sense?.maxPelletChecks ?? 900);
  let checks = 0;

  for (let cy = minCy; cy <= maxCy; cy++) {
    for (let cx = minCx; cx <= maxCx; cx++) {
      const arr = grid.map.get(cx + "," + cy);
      if (!arr) continue;
      for (let i = 0; i < arr.length; i++) {
        const p = arr[i];
        const dx = p.x - x;
        const dy = p.y - y;
        if (dx * dx + dy * dy > r2) continue;
        if (fn(p) === false) return;
        checks++;
        if (checks >= maxChecks) return;
      }
    }
  }
}

/**
 * Converts snake length into a sensing radius that scales like the follow
 * camera zoom: larger snakes get a larger "view bubble".
 */
function _bubbleRadiusForSnake(snake: SnakeLike): number {
  const len = snake.length();
  const maxLen = Math.max(1, CFG.snakeMaxLen);
  const zoom = clamp(1.15 - (len / maxLen) * 0.55, 0.45, 1.12);
  const base = Math.max(80, CFG.sense?.bubbleRadiusBase ?? 760);
  const minR = Math.max(80, CFG.sense?.bubbleRadiusMin ?? 420);
  const maxR = Math.max(minR, CFG.sense?.bubbleRadiusMax ?? 1700);
  const r = base / Math.max(0.02, zoom);
  return clamp(r, minR, maxR);
}

function _angleToBin(relAngle: number, bins: number): number {
  // relAngle is in [-pi, pi]. Map so 0 is forward.
  let a = relAngle;
  if (a < 0) a += TAU;
  const idx = Math.floor((a / TAU) * bins);
  return clamp(idx, 0, bins - 1);
}

/**
 * Computes a 360° food density histogram around the head.
 * Returns values in [-1,1] where -1 means "no food" and +1 means "very dense".
 */
let _scratchFood = new Float32Array(0);
let _scratchHaz = new Float32Array(0);

function _ensureScratch(bins: number): void {
  if (_scratchFood.length !== bins) _scratchFood = new Float32Array(bins);
  if (_scratchHaz.length !== bins) _scratchHaz = new Float32Array(bins);
}

function _fillFoodBubbleBins(
  world: WorldLike,
  snake: SnakeLike,
  bins: number,
  r: number,
  ins: Float32Array,
  insOffset: number
): void {
  // Accumulate weighted food value by direction bin.
  _ensureScratch(bins);
  for (let i = 0; i < bins; i++) _scratchFood[i] = 0;

  const sx = snake.x;
  const sy = snake.y;
  const baseV = Math.max(1e-6, CFG.foodValue);

  _forEachNearbyPellet(world, sx, sy, r, p => {
    const dx = p.x - sx;
    const dy = p.y - sy;
    const d = Math.hypot(dx, dy);
    if (d <= 1e-6 || d > r) return;
    const ang = Math.atan2(dy, dx);
    const rel = angNorm(ang - snake.dir);
    const b = _angleToBin(rel, bins);
    const wDist = 1 - d / r;
    const wVal = clamp(p.v / baseV, 0, 6.0);
    _scratchFood[b] += wDist * wVal;
  });

  const K0 = Math.max(0.1, CFG.sense?.bubbleFoodK ?? 4.0);
  const scale = r / Math.max(1e-6, CFG.sense?.bubbleRadiusMin ?? 420);
  const K = K0 * Math.max(0.75, scale);

  for (let i = 0; i < bins; i++) {
    const s = _scratchFood[i];
    const frac = s / (s + K);
    ins[insOffset + i] = clamp(frac * 2 - 1, -1, 1);
  }
}

/**
 * Computes a 360° hazard clearance histogram around the head.
 * Returns values in [-1,1] where +1 means clear and -1 means blocked.
 */
function _fillHazardBubbleBins(
  world: WorldLike,
  snake: SnakeLike,
  bins: number,
  r: number,
  ins: Float32Array,
  insOffset: number
): void {
  _ensureScratch(bins);
  for (let i = 0; i < bins; i++) _scratchHaz[i] = r;
  const sx = snake.x;
  const sy = snake.y;

  _forEachNearbySegment(world, sx, sy, r, ref => {
    const other = ref.s;
    if (!other || !other.alive || other === snake) return;
    const i = ref.i;
    const pts = other.points;
    if (!pts || i <= 0 || i >= pts.length) return;
    const a = pts[i - 1];
    const b = pts[i];
    const c = _closestPointOnSegment(sx, sy, a.x, a.y, b.x, b.y);
    const d = Math.sqrt(c.d2);
    if (d > r + (other.radius || CFG.snakeRadius) + 8) return;
    const free = Math.max(0, d - (other.radius || CFG.snakeRadius));
    const ang = Math.atan2(c.qy - sy, c.qx - sx);
    const rel = angNorm(ang - snake.dir);
    const bi = _angleToBin(rel, bins);
    if (free < _scratchHaz[bi]) _scratchHaz[bi] = free;
  });

  for (let i = 0; i < bins; i++) {
    const ratio = clamp(_scratchHaz[i] / r, 0, 1);
    ins[insOffset + i] = ratio * 2 - 1;
  }
}

/**
 * Computes a 360° wall distance histogram around the head.
 * Returns values in [-1,1] where +1 means wall is beyond the bubble radius.
 */
function _fillWallBubbleBins(
  snake: SnakeLike,
  bins: number,
  r: number,
  ins: Float32Array,
  insOffset: number
): void {
  const sx = snake.x;
  const sy = snake.y;
  const R = CFG.worldRadius;
  for (let i = 0; i < bins; i++) {
    const theta = snake.dir + (i / bins) * TAU;
    const t = _distToWallAlongRay(sx, sy, theta, R);
    const ratio = clamp(t / r, 0, 1);
    ins[insOffset + i] = ratio * 2 - 1;
  }
}

/**
 * Builds the full sensor vector. If out is provided, fills it
 * in-place to avoid per‑tick allocations.
 * @param {World} world
 * @param {Snake} snake
 * @param {Float32Array|null} out
 */
export function buildSensors(
  world: WorldLike,
  snake: SnakeLike,
  out: Float32Array | null = null
): Float32Array {
  const bins = Math.max(8, Math.floor(CFG.sense?.bubbleBins ?? 12));
  const expected = 5 + 3 * bins;
  const ins = out && out.length === expected ? out : new Float32Array(expected);

  const r = _bubbleRadiusForSnake(snake);

  // 0-1: heading sin/cos
  ins[0] = Math.sin(snake.dir);
  ins[1] = Math.cos(snake.dir);

  // 2: size fraction [-1,1]
  ins[2] = clamp(snake.sizeNorm() * 2 - 1, -1, 1);

  // 3: boost margin relative to minimum points to boost
  const minBoostPts = CFG.boost.minPointsToBoost;
  const margin = snake.pointsScore - minBoostPts;
  ins[3] = clamp(margin / Math.max(1e-6, minBoostPts), -1, 1);

  // 4: log-scaled percentile of points relative to best this generation
  const bestPts = Math.max(0.001, world.bestPointsThisGen);
  const logFrac = Math.log(1 + snake.pointsScore) / Math.log(1 + bestPts);
  ins[4] = clamp(logFrac * 2 - 1, -1, 1);

  // 5..: 360° food / hazard / wall bubbles (relative to heading)
  const foodOff = 5;
  const hazOff = foodOff + bins;
  const wallOff = hazOff + bins;

  _fillFoodBubbleBins(world, snake, bins, r, ins, foodOff);
  _fillHazardBubbleBins(world, snake, bins, r, ins, hazOff);
  _fillWallBubbleBins(snake, bins, r, ins, wallOff);

  return ins;
}
