// sensors.ts
// Functions for constructing the input vector fed into each snake’s neural
// network.  These functions gather information about the environment
// (food, threats, obstacles) and encode it into a fixed length array.

import { CFG } from './config.ts';
import {
  getSensorLayout,
  type SensorLayout,
  type SensorLayoutVersion
} from './protocol/sensors.ts';
import { clamp, angNorm, TAU } from './utils.ts';

/** Minimal pellet shape used by sensor sampling. */
interface PelletLike {
  x: number;
  y: number;
  v: number;
}

/** Point structure for snake segments. */
interface SnakePoint {
  x: number;
  y: number;
}

/** Snake interface required for sensor calculations. */
interface SnakeLike {
  /** Unique snake identifier. */
  id: number;
  x: number;
  y: number;
  dir: number;
  /** Current speed in world units per second. */
  speed: number;
  /** Boost state flag as numeric value. */
  boost: number;
  pointsScore: number;
  points: SnakePoint[];
  radius: number;
  alive: boolean;
  length: () => number;
  sizeNorm: () => number;
}

/** Segment reference returned by collision grid queries. */
interface SegmentRef {
  s: SnakeLike;
  i: number;
}

/** World interface required for sensor calculations. */
interface WorldLike {
  pellets: PelletLike[];
  bestPointsThisGen: number;
  snakes?: SnakeLike[];
  pelletGrid?: { map?: Map<string, PelletLike[]>; cellSize?: number };
  _collGrid?: { map?: Map<string, unknown>; cellSize?: number; query?: (cx: number, cy: number) => SegmentRef[] | null };
}

/**
 * Compute the closest point on a segment to a point.
 * @param px - Point x coordinate.
 * @param py - Point y coordinate.
 * @param ax - Segment start x.
 * @param ay - Segment start y.
 * @param bx - Segment end x.
 * @param by - Segment end y.
 * @returns Closest point coordinates and squared distance.
 */
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
 * Iterate segments from the collision grid in cells intersecting a radius
 * around (x,y). The callback receives an object with fields s and i, where i is
 * the segment end index in s.points (segment is i-1 to i).
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
        const ref = arr[j];
        if (!ref) continue;
        fn(ref);
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
      if (!p) continue;
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
        if (!p) continue;
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

/**
 * Map a relative angle to a histogram bin index.
 * @param relAngle - Relative angle in radians.
 * @param bins - Total number of bins.
 * @returns Bin index.
 */
function _angleToBin(relAngle: number, bins: number): number {
  // relAngle is in [-pi, pi]. Map so 0 is forward.
  let a = relAngle;
  if (a < 0) a += TAU;
  const idx = Math.floor((a / TAU) * bins);
  return clamp(idx, 0, bins - 1);
}

/** Default v2 near radius base in world units. */
const DEFAULT_R_NEAR_BASE = 520;
/** Default v2 near radius scale in world units. */
const DEFAULT_R_NEAR_SCALE = 260;
/** Default v2 near radius minimum in world units. */
const DEFAULT_R_NEAR_MIN = 420;
/** Default v2 near radius maximum in world units. */
const DEFAULT_R_NEAR_MAX = 1100;
/** Default v2 far radius base in world units. */
const DEFAULT_R_FAR_BASE = 1200;
/** Default v2 far radius scale in world units. */
const DEFAULT_R_FAR_SCALE = 520;
/** Default v2 far radius minimum in world units. */
const DEFAULT_R_FAR_MIN = 900;
/** Default v2 far radius maximum in world units. */
const DEFAULT_R_FAR_MAX = 2400;
/** Default v2 food saturation constant. */
const DEFAULT_FOOD_K = 4.0;

/**
 * Normalize a v2 sense parameter to a finite number with fallback.
 * @param value - Candidate value to validate.
 * @param fallback - Fallback used when value is not finite.
 * @param key - Optional key name for debug output.
 * @returns Validated numeric value.
 */
function readSenseNumber(value: unknown, fallback: number, key?: string): number {
  if (Number.isFinite(value)) return value as number;
  if (value !== undefined && key) {
    console.warn('[sensors.v2.invalid_config]', { key, value });
  }
  return fallback;
}

/**
 * Map a relative angle to a centered histogram bin index.
 * @param relAngle - Relative angle in radians.
 * @param bins - Total number of bins.
 * @returns Bin index.
 */
export function angleToCenteredBin(relAngle: number, bins: number): number {
  let u = (relAngle + Math.PI) / TAU;
  u = (u + 0.5 / bins) % 1;
  const idx = Math.floor(u * bins);
  return clamp(idx, 0, bins - 1);
}

/**
 * Convert a centered bin index to its representative relative angle.
 * @param index - Bin index.
 * @param bins - Total number of bins.
 * @returns Relative angle in radians.
 */
function centeredBinToAngle(index: number, bins: number): number {
  return -Math.PI + (index / bins) * TAU;
}

/**
 * Compute the near/far sensing radii for v2 sensors.
 * @param sizeNorm - Snake size normalization in [0, 1].
 * @returns Near and far sensing radii in world units.
 */
export function computeSensorRadii(sizeNorm: number): { rNear: number; rFar: number } {
  const safeSize = clamp(Number.isFinite(sizeNorm) ? sizeNorm : 0, 0, 1);
  const sense = CFG.sense ?? {};
  const rNearBase = readSenseNumber(sense.rNearBase, DEFAULT_R_NEAR_BASE, 'sense.rNearBase');
  const rNearScale = readSenseNumber(sense.rNearScale, DEFAULT_R_NEAR_SCALE, 'sense.rNearScale');
  const rNearMin = Math.max(1, readSenseNumber(sense.rNearMin, DEFAULT_R_NEAR_MIN, 'sense.rNearMin'));
  const rNearMax = Math.max(rNearMin, readSenseNumber(sense.rNearMax, DEFAULT_R_NEAR_MAX, 'sense.rNearMax'));
  const rFarBase = readSenseNumber(sense.rFarBase, DEFAULT_R_FAR_BASE, 'sense.rFarBase');
  const rFarScale = readSenseNumber(sense.rFarScale, DEFAULT_R_FAR_SCALE, 'sense.rFarScale');
  const rFarMin = Math.max(1, readSenseNumber(sense.rFarMin, DEFAULT_R_FAR_MIN, 'sense.rFarMin'));
  const rFarMax = Math.max(rFarMin, readSenseNumber(sense.rFarMax, DEFAULT_R_FAR_MAX, 'sense.rFarMax'));

  const rNear = clamp(rNearBase + rNearScale * safeSize, rNearMin, rNearMax);
  let rFar = clamp(rFarBase + rFarScale * safeSize, rFarMin, rFarMax);
  if (rFar < rNear + 1) rFar = rNear + 1;
  return { rNear, rFar };
}

/**
 * Normalize a clearance or distance ratio into [-1, 1].
 * @param ratio - Ratio in [0, 1].
 * @returns Normalized value in [-1, 1].
 */
function ratioToBipolar(ratio: number): number {
  return clamp(ratio, 0, 1) * 2 - 1;
}

/**
 * Computes a 360° food density histogram around the head.
 * Returns values in [-1,1] where -1 means "no food" and +1 means "very dense".
 */
/** Scratch buffer for food bin accumulation. */
let _scratchFood = new Float32Array(0);
/** Scratch buffer for hazard bin accumulation. */
let _scratchHaz = new Float32Array(0);
/** Scratch buffer for head pressure bin accumulation. */
let _scratchHead = new Float32Array(0);
/** Cached sensor layout metadata for the active layout. */
let _cachedLayout: SensorLayout | null = null;
/** Cached bin count for the active layout metadata. */
let _cachedLayoutBins = -1;
/** Cached layout version for the active layout metadata. */
let _cachedLayoutVersion: SensorLayoutVersion | null = null;

/**
 * Resolve the cached sensor layout for the current config.
 * @returns Cached layout metadata.
 */
function getActiveLayout(): SensorLayout {
  const bins = Math.max(8, Math.floor(CFG.sense?.bubbleBins ?? 16));
  const layoutVersion = (CFG.sense?.layoutVersion ?? 'v2') as SensorLayoutVersion;
  if (!_cachedLayout || _cachedLayoutBins !== bins || _cachedLayoutVersion !== layoutVersion) {
    _cachedLayout = getSensorLayout(bins, layoutVersion);
    _cachedLayoutBins = _cachedLayout.bins;
    _cachedLayoutVersion = _cachedLayout.layoutVersion;
  }
  return _cachedLayout;
}

/**
 * Ensure scratch buffers are sized for the given bin count.
 * @param bins - Number of bins to allocate.
 */
function _ensureScratch(bins: number): void {
  if (_scratchFood.length !== bins) _scratchFood = new Float32Array(bins);
  if (_scratchHaz.length !== bins) _scratchHaz = new Float32Array(bins);
  if (_scratchHead.length !== bins) _scratchHead = new Float32Array(bins);
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
    const prev = _scratchFood[b] ?? 0;
    _scratchFood[b] = prev + wDist * wVal;
  });

  const K0 = Math.max(0.1, CFG.sense?.bubbleFoodK ?? 4.0);
  const scale = r / Math.max(1e-6, CFG.sense?.bubbleRadiusMin ?? 420);
  const K = K0 * Math.max(0.75, scale);

  for (let i = 0; i < bins; i++) {
    const s = _scratchFood[i] ?? 0;
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
    if (!a || !b) return;
    const c = _closestPointOnSegment(sx, sy, a.x, a.y, b.x, b.y);
    const d = Math.sqrt(c.d2);
    if (d > r + (other.radius || CFG.snakeRadius) + 8) return;
    const free = Math.max(0, d - (other.radius || CFG.snakeRadius));
    const ang = Math.atan2(c.qy - sy, c.qx - sx);
    const rel = angNorm(ang - snake.dir);
    const bi = _angleToBin(rel, bins);
    const prev = _scratchHaz[bi] ?? r;
    if (free < prev) _scratchHaz[bi] = free;
  });

  for (let i = 0; i < bins; i++) {
    const haz = _scratchHaz[i] ?? r;
    const ratio = clamp(haz / r, 0, 1);
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
 * Compute a v2 food density histogram using centered binning.
 * @param world - World state providing pellets.
 * @param snake - Snake to compute sensors for.
 * @param bins - Number of bins.
 * @param rFar - Far sensing radius.
 * @param ins - Sensor output buffer.
 * @param insOffset - Offset into the output buffer.
 */
function _fillFoodBinsV2(
  world: WorldLike,
  snake: SnakeLike,
  bins: number,
  rFar: number,
  ins: Float32Array,
  insOffset: number
): void {
  _ensureScratch(bins);
  for (let i = 0; i < bins; i++) _scratchFood[i] = 0;

  const sx = snake.x;
  const sy = snake.y;
  const baseV = Math.max(1e-6, CFG.foodValue);
  const rFar2 = rFar * rFar;

  _forEachNearbyPellet(world, sx, sy, rFar, p => {
    const dx = p.x - sx;
    const dy = p.y - sy;
    const d2 = dx * dx + dy * dy;
    if (d2 <= 1e-6 || d2 > rFar2) return;
    const d = Math.sqrt(d2);
    const ang = Math.atan2(dy, dx);
    const rel = angNorm(ang - snake.dir);
    const b = angleToCenteredBin(rel, bins);
    const wDist = 1 - d / rFar;
    const wVal = clamp(p.v / baseV, 0, 6.0);
    const prev = _scratchFood[b] ?? 0;
    _scratchFood[b] = prev + wDist * wVal;
  });

  const K = Math.max(0.1, readSenseNumber(CFG.sense?.foodKBase, DEFAULT_FOOD_K, 'sense.foodKBase'));
  for (let i = 0; i < bins; i++) {
    const s = _scratchFood[i] ?? 0;
    const frac = s / (s + K);
    ins[insOffset + i] = ratioToBipolar(frac);
  }
}

/**
 * Compute a v2 hazard clearance histogram using centered binning.
 * @param world - World state providing collision segments.
 * @param snake - Snake to compute sensors for.
 * @param bins - Number of bins.
 * @param rNear - Near sensing radius.
 * @param ins - Sensor output buffer.
 * @param insOffset - Offset into the output buffer.
 */
function _fillHazardBinsV2(
  world: WorldLike,
  snake: SnakeLike,
  bins: number,
  rNear: number,
  ins: Float32Array,
  insOffset: number
): void {
  _ensureScratch(bins);
  for (let i = 0; i < bins; i++) _scratchHaz[i] = rNear;

  const sx = snake.x;
  const sy = snake.y;

  _forEachNearbySegment(world, sx, sy, rNear, ref => {
    const other = ref.s;
    if (!other || !other.alive || other === snake) return;
    const i = ref.i;
    const pts = other.points;
    if (!pts || i <= 0 || i >= pts.length) return;
    const a = pts[i - 1];
    const b = pts[i];
    if (!a || !b) return;
    const c = _closestPointOnSegment(sx, sy, a.x, a.y, b.x, b.y);
    const d2 = c.d2;
    const thr = (snake.radius + other.radius) * CFG.collision.hitScale;
    const maxDist = rNear + thr;
    if (d2 > maxDist * maxDist) return;
    const dist = Math.sqrt(d2);
    const clear = Math.max(0, dist - thr);
    if (clear > rNear) return;
    const ang = Math.atan2(c.qy - sy, c.qx - sx);
    const rel = angNorm(ang - snake.dir);
    const bi = angleToCenteredBin(rel, bins);
    const current = _scratchHaz[bi] ?? rNear;
    if (clear < current) _scratchHaz[bi] = clear;
  });

  for (let i = 0; i < bins; i++) {
    const haz = _scratchHaz[i] ?? rNear;
    const ratio = haz / rNear;
    ins[insOffset + i] = ratioToBipolar(ratio);
  }
}

/**
 * Compute a v2 wall clearance histogram using centered binning.
 * @param snake - Snake to compute sensors for.
 * @param bins - Number of bins.
 * @param rNear - Near sensing radius.
 * @param ins - Sensor output buffer.
 * @param insOffset - Offset into the output buffer.
 */
function _fillWallBinsV2(
  snake: SnakeLike,
  bins: number,
  rNear: number,
  ins: Float32Array,
  insOffset: number
): void {
  const sx = snake.x;
  const sy = snake.y;
  const R = CFG.worldRadius;
  const distToCenter = Math.hypot(sx, sy);
  if (distToCenter > R) {
    console.warn('[sensors.v2.out_of_bounds]', { x: sx, y: sy, worldRadius: R });
  }

  for (let i = 0; i < bins; i++) {
    const theta = snake.dir + centeredBinToAngle(i, bins);
    let t = _distToWallAlongRay(sx, sy, theta, R);
    if (!Number.isFinite(t) || t <= 0) t = 0;
    const clear = clamp(t - snake.radius, 0, rNear);
    ins[insOffset + i] = ratioToBipolar(clear / rNear);
  }
}

/**
 * Compute a v2 head pressure histogram using centered binning.
 * @param world - World state providing snakes.
 * @param snake - Snake to compute sensors for.
 * @param bins - Number of bins.
 * @param rNear - Near sensing radius.
 * @param ins - Sensor output buffer.
 * @param insOffset - Offset into the output buffer.
 */
function _fillHeadBinsV2(
  world: WorldLike,
  snake: SnakeLike,
  bins: number,
  rNear: number,
  ins: Float32Array,
  insOffset: number
): void {
  _ensureScratch(bins);
  for (let i = 0; i < bins; i++) _scratchHead[i] = rNear;

  const sx = snake.x;
  const sy = snake.y;
  const snakes = world.snakes ?? [];

  for (const other of snakes) {
    if (!other || !other.alive || other === snake || other.id === snake.id) continue;
    const head = other.points[0];
    if (!head) continue;
    const dx = head.x - sx;
    const dy = head.y - sy;
    const d2 = dx * dx + dy * dy;
    if (d2 > rNear * rNear) continue;
    const dist = Math.sqrt(d2);
    const thr = (snake.radius + other.radius) * CFG.collision.hitScale;
    const clear = Math.max(0, dist - thr);
    const ang = Math.atan2(dy, dx);
    const rel = angNorm(ang - snake.dir);
    const bi = angleToCenteredBin(rel, bins);
    const current = _scratchHead[bi] ?? rNear;
    if (clear < current) _scratchHead[bi] = clear;
  }

  for (let i = 0; i < bins; i++) {
    const headClear = _scratchHead[i] ?? rNear;
    ins[insOffset + i] = ratioToBipolar(headClear / rNear);
  }
}

/**
 * Build the full sensor vector. If out is provided, fills it in-place to avoid
 * per-tick allocations.
 * @param world - World state providing pellets and collision data.
 * @param snake - Snake to compute sensors for.
 * @param out - Optional output buffer to reuse.
 */
export function buildSensors(
  world: WorldLike,
  snake: SnakeLike,
  out: Float32Array | null = null
): Float32Array {
  const layout = getActiveLayout();
  const bins = layout.bins;
  const ins = out && out.length === layout.inputSize ? out : new Float32Array(layout.inputSize);

  // 0-1: heading sin/cos
  ins[0] = Math.sin(snake.dir);
  ins[1] = Math.cos(snake.dir);

  // 2: size fraction [-1,1]
  const sizeNorm = snake.sizeNorm();
  ins[2] = clamp(sizeNorm * 2 - 1, -1, 1);

  // 3: boost margin relative to minimum points to boost
  const minBoostPts = CFG.boost.minPointsToBoost;
  const margin = snake.pointsScore - minBoostPts;
  ins[3] = clamp(margin / Math.max(1e-6, minBoostPts), -1, 1);

  // 4: log-scaled percentile of points relative to best this generation
  const bestPts = Math.max(0.001, world.bestPointsThisGen);
  const logFrac = Math.log(1 + snake.pointsScore) / Math.log(1 + bestPts);
  ins[4] = clamp(logFrac * 2 - 1, -1, 1);

  const foodOff = layout.offsets.food;
  const hazOff = layout.offsets.hazard;
  const wallOff = layout.offsets.wall;

  if (layout.layoutVersion === 'v2') {
    const speedRatio = Number.isFinite(snake.speed)
      ? snake.speed / Math.max(1e-6, CFG.snakeBoostSpeed)
      : 0;
    ins[5] = ratioToBipolar(speedRatio);
    const boostRatio = Number.isFinite(snake.boost) ? snake.boost : 0;
    ins[6] = ratioToBipolar(clamp(boostRatio, 0, 1));

    const { rNear, rFar } = computeSensorRadii(sizeNorm);
    _fillFoodBinsV2(world, snake, bins, rFar, ins, foodOff);
    _fillHazardBinsV2(world, snake, bins, rNear, ins, hazOff);
    _fillWallBinsV2(snake, bins, rNear, ins, wallOff);

    if (layout.offsets.head != null) {
      _fillHeadBinsV2(world, snake, bins, rNear, ins, layout.offsets.head);
    }
  } else {
    const r = _bubbleRadiusForSnake(snake);
    _fillFoodBubbleBins(world, snake, bins, r, ins, foodOff);
    _fillHazardBubbleBins(world, snake, bins, r, ins, hazOff);
    _fillWallBubbleBins(snake, bins, r, ins, wallOff);
  }

  return ins;
}
