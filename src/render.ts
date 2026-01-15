/** Helper functions for drawing the world onto the canvas. */

import { TAU, clamp, hashColor } from './utils.ts';
import { THEME, getPelletColor, getPelletGlow } from './theme.ts';
import { CFG } from './config.ts';
import { FRAME_HEADER_FLOATS } from './protocol/frame.ts';

/** Camera state required by background drawing. */
interface CameraState {
  zoom: number;
  cameraX: number;
  cameraY: number;
}

/** Boost particle state for fast-path rendering. */
interface BoostParticle {
  x: number;
  y: number;
  vx: number;
  vy: number;
  life: number;
  maxLife: number;
  size: number;
  color: string;
}

/** Cached snake render state for speed smoothing. */
interface SnakeRenderCacheEntry {
  x: number;
  y: number;
  speed: number;
  seen: number;
}

/** Serialized snake structure used by the fast renderer. */
interface SnakeStruct {
  id: number;
  radius: number;
  skin: number;
  x: number;
  y: number;
  ang: number;
  boost: number;
  pts: ArrayLike<number>;
  speed?: number;
  color?: string | null;
}

/** Metadata for deferred snake rendering. */
interface SnakeMeta {
  basePtr: number;
  ptCount: number;
  id: number;
  rad: number;
  skin: number;
  x: number;
  y: number;
  ang: number;
  boost: number;
  speed: number;
}

/** Pellet shape for legacy renderer. */
interface RenderPellet {
  x: number;
  y: number;
  v: number;
  kind?: string;
  colorId?: number;
  [key: string]: unknown;
}

/** Snake shape for legacy renderer. */
interface RenderSnake {
  id: number;
  alive: boolean;
  radius: number;
  x: number;
  y: number;
  dir: number;
  turnInput?: number;
  boostInput?: number;
  lastSensors?: Float32Array | number[];
  lastOutputs?: Float32Array | number[];
}

/** World shape for legacy renderer. */
interface RenderWorld {
  zoom: number;
  cameraX: number;
  cameraY: number;
  pellets: RenderPellet[];
  snakes: RenderSnake[];
  particles: {
    render: (ctx: CanvasRenderingContext2D, cameraX: number, cameraY: number, zoom: number) => void;
  };
  focusSnake?: RenderSnake | null;
  viewMode?: string;
}

/** Legacy drawSnake helper provided elsewhere in the renderer. */
declare function drawSnake(ctx: CanvasRenderingContext2D, s: RenderSnake, zoom: number): void;

/** Cached render info for snakes by id. */
const snakeRenderCache = new Map<number, SnakeRenderCacheEntry>();
/** Active boost particles for fast-path rendering. */
const boostParticles: BoostParticle[] = [];
/** Upper bound on boost particle count. */
const MAX_BOOST_PARTICLES = 1400;
/** Boost particle lifetime in seconds. */
const BOOST_PARTICLE_LIFE = 0.38;
/** Global render tick counter for cache eviction. */
let renderTick = 0;
/** Last render timestamp for dt calculation. */
let lastRenderTime = 0;

/**
 * Generate a random number in [min,max].
 * @param min - Minimum value.
 * @param max - Maximum value.
 * @returns Random value in range.
 */
function randRange(min: number, max: number): number {
  return min + Math.random() * (max - min);
}

/**
 * Compute render delta time clamped to avoid large spikes.
 * @returns Delta time in seconds.
 */
function getRenderDt(): number {
  if (typeof performance === 'undefined' || !performance.now) return 1 / 60;
  const now = performance.now();
  const dt = lastRenderTime ? Math.min(0.05, (now - lastRenderTime) / 1000) : 1 / 60;
  lastRenderTime = now;
  return dt || 1 / 60;
}

/**
 * Resolve a snake color from id and skin flag.
 * @param id - Snake id.
 * @param skin - Skin flag (1 indicates gold).
 * @returns CSS color string.
 */
function getSnakeColor(id: number, skin: number): string {
  if (skin === 1.0) return '#FFD700';
  if (skin === 2.0) return THEME.snakeRobotBody;
  return hashColor(id * 17 + 3);
}

/**
 * Spawn a boost particle in the fast renderer.
 * @param x - Spawn x position.
 * @param y - Spawn y position.
 * @param ang - Snake heading angle in radians.
 * @param color - Particle color.
 * @param strength - Boost strength in [0,1].
 */
function spawnBoostParticle(x: number, y: number, ang: number, color: string, strength: number): void {
  if (boostParticles.length >= MAX_BOOST_PARTICLES) boostParticles.shift();
  const dir = ang + Math.PI + randRange(-0.35, 0.35);
  const speed = randRange(60, 140) * (0.65 + strength);
  boostParticles.push({
    x: x + Math.cos(dir) * randRange(4, 10),
    y: y + Math.sin(dir) * randRange(4, 10),
    vx: Math.cos(dir) * speed,
    vy: Math.sin(dir) * speed,
    life: BOOST_PARTICLE_LIFE,
    maxLife: BOOST_PARTICLE_LIFE,
    size: randRange(1.6, 3.4) * (0.7 + strength),
    color
  });
}

/**
 * Render and advance boost particles.
 * @param ctx - Canvas 2D context to draw into.
 * @param dt - Delta time in seconds.
 */
function renderBoostParticles(ctx: CanvasRenderingContext2D, dt: number): void {
  if (!boostParticles.length) return;
  ctx.save();
  ctx.globalCompositeOperation = 'lighter';
  for (let i = boostParticles.length - 1; i >= 0; i--) {
    const p = boostParticles[i];
    if (!p) continue;
    p.life -= dt;
    if (p.life <= 0) {
      boostParticles.splice(i, 1);
      continue;
    }
    p.x += p.vx * dt;
    p.y += p.vy * dt;
    p.vx *= 0.94;
    p.vy *= 0.94;
    const alpha = clamp(p.life / p.maxLife, 0, 1);
    ctx.globalAlpha = alpha;
    ctx.shadowBlur = p.size * 4;
    ctx.shadowColor = p.color;
    ctx.fillStyle = p.color;
    ctx.beginPath();
    ctx.arc(p.x, p.y, p.size, 0, TAU);
    ctx.fill();
  }
  ctx.restore();
}

/**
 * Hash two integer coordinates into a deterministic 32-bit value.
 * @param x - Cell x coordinate.
 * @param y - Cell y coordinate.
 * @returns Unsigned 32-bit hash.
 */
function hash2(x: number, y: number): number {
  let h = (x * 374761393 + y * 668265263) | 0;
  h ^= h >>> 13;
  h = Math.imul(h, 1274126177);
  return h >>> 0;
}

/**
 * Advance the hash to the next pseudo-random value.
 * @param h - Current hash value.
 * @returns Next hash value.
 */
function nextRand(h: number): number {
  h ^= h << 13;
  h ^= h >>> 17;
  h ^= h << 5;
  return h >>> 0;
}

/**
 * Draw a starfield background aligned to the camera.
 * @param ctx - Canvas 2D context to draw into.
 * @param world - Camera state for positioning.
 * @param viewW - Viewport width in pixels.
 * @param viewH - Viewport height in pixels.
 */
export function drawStarfield(
  ctx: CanvasRenderingContext2D,
  world: CameraState,
  viewW: number,
  viewH: number
): void {
  const cell = 240;
  const halfWWorld = viewW / (2 * world.zoom);
  const halfHWorld = viewH / (2 * world.zoom);
  const left = world.cameraX - halfWWorld;
  const right = world.cameraX + halfWWorld;
  const top = world.cameraY - halfHWorld;
  const bottom = world.cameraY + halfHWorld;

  const minCx = Math.floor(left / cell);
  const maxCx = Math.floor(right / cell);
  const minCy = Math.floor(top / cell);
  const maxCy = Math.floor(bottom / cell);

  ctx.save();
  ctx.fillStyle = 'rgba(255,255,255,0.9)';
  for (let cy = minCy; cy <= maxCy; cy++) {
    for (let cx = minCx; cx <= maxCx; cx++) {
      let h = hash2(cx, cy);
      if ((h & 1023) > 120) continue;
      h = nextRand(h);
      const ox = (h & 0xffff) / 0xffff;
      h = nextRand(h);
      const oy = (h & 0xffff) / 0xffff;
      h = nextRand(h);
      const size = 0.6 + ((h & 255) / 255) * 1.8;
      h = nextRand(h);
      const alpha = 0.35 + ((h & 255) / 255) * 0.45;

      const px = (cx + ox) * cell;
      const py = (cy + oy) * cell;
      ctx.globalAlpha = alpha;
      ctx.beginPath();
      ctx.arc(px, py, size, 0, TAU);
      ctx.fill();
    }
  }
  ctx.restore();
}

/**
 * Draw a grid centered on the origin using a cached pattern.
 * @param ctx - Canvas 2D context to draw into.
 * @param world - Camera state for positioning.
 * @param viewW - Viewport width in pixels.
 * @param viewH - Viewport height in pixels.
 */
export function drawGrid(
  ctx: CanvasRenderingContext2D,
  world: CameraState,
  viewW: number,
  viewH: number
): void {
  const step = 160;

  // Lazy init the pattern once
  if (!bgPattern) {
    // Check for OffscreenCanvas support, fall back to simple canvas
    if (typeof OffscreenCanvas !== 'undefined') {
      bgCanvas = new OffscreenCanvas(step, step);
    } else {
      bgCanvas = document.createElement('canvas');
      bgCanvas.width = step;
      bgCanvas.height = step;
    }
    const bx = bgCanvas.getContext('2d') as CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D;

    // Fill background (optional, but keep transparent for now)
    // Draw lines
    bx.strokeStyle = THEME.gridLine; // e.g. rgba(255,255,255,0.08)
    bx.lineWidth = 1;
    bx.beginPath();
    // Vertical line at 0 (left edge)
    bx.moveTo(0, 0);
    bx.lineTo(0, step);
    // Horizontal line at 0 (top edge)
    bx.moveTo(0, 0);
    bx.lineTo(step, 0);
    bx.stroke();

    // Create pattern from this tile
    bgPattern = ctx.createPattern(bgCanvas, 'repeat');
  }

  if (!bgPattern) return; // Should not happen

  ctx.save();
  // We need to offset the fill so it aligns with the world origin (0,0)
  // The pattern repeats from the storage origin.
  // ctx is already transformed by the camera (translated and scaled).
  // Ideally, we just fill the visible rect.
  // Use setTransform to draw in screen space? 
  // No, easier to draw in world space but large rect.

  const halfWWorld = viewW / (2 * world.zoom);
  const halfHWorld = viewH / (2 * world.zoom);
  const left = world.cameraX - halfWWorld;
  const top = world.cameraY - halfHWorld;
  const w = halfWWorld * 2;
  const h = halfHWorld * 2;

  ctx.fillStyle = bgPattern;
  // Offset pattern to match world origin?
  // pattern starts at 0,0. Our world origin is 0,0. 
  // So a fillRect of everything should align perfectly!
  // Just massive fill rect covering the view.
  // Add a bit of margin
  ctx.fillRect(left - step, top - step, w + step * 2, h + step * 2);

  ctx.restore();
}

/** Cached grid pattern for background rendering. */
let bgPattern: CanvasPattern | null = null;
/** Cached canvas for generating the grid pattern. */
let bgCanvas: OffscreenCanvas | HTMLCanvasElement | null = null;

/**
 * Renders a single snake entity using calculated local metadata.
 * 
 * Visual Features:
 * 1. Adaptive Glow: Increases shadowBlur based on the snake's radius and current speed.
 * 2. Speed-Dependent Width: Slightly thickens the snake body during high-speed motion.
 * 3. Skin Variations: 
 *    - Default/Gold: Uses secondary stroke colors.
 *    - Robot (ID 2): Adds a specific cyan glow and red eyes.
 * 4. Coordinate Stability: Uses worldMin to prevent lines from disappearing at extreme zoom levels.
 * 
 * @param ctx - Canvas 2D rendering context.
 * @param s - Processed snake metadata (position, speed, body points).
 * @param zoom - Current global viewport scale.
 */
export function drawSnakeStruct(ctx: CanvasRenderingContext2D, s: SnakeStruct, zoom: number): void {
  ctx.lineCap = "round";
  ctx.lineJoin = "round";

  const color = s.color || getSnakeColor(s.id, s.skin);

  // Lighting & Glow Calculations:
  // We amplify the shadow blur based on speed (speedGlow) and active boosting.
  const speed = Math.max(0, s.speed || 0);
  const BOOST_GLOW_MULTIPLIER = 1.35;
  const boostGlow = s.boost > 0.5 ? BOOST_GLOW_MULTIPLIER : 1;

  // Speed glow is normalized against a radius-dependent threshold.
  const SPEED_GLOW_THRESHOLD = s.radius * 1.6;
  const speedGlow = clamp(speed / Math.max(6, SPEED_GLOW_THRESHOLD), 0, 1);

  const BASE_GLOW_SCALE = 1.0;
  const MAX_SPEED_GLOW_BONUS = 0.9;
  const glowScale = BASE_GLOW_SCALE + speedGlow * MAX_SPEED_GLOW_BONUS;

  ctx.shadowBlur = s.radius * 1.6 * glowScale * boostGlow;
  ctx.shadowColor = color;
  ctx.strokeStyle = color;

  // Scaling Safety:
  // Ensure the snake remains visible as it zooms out by capping the minimum screen-pixel width.
  const MIN_DISPLAY_PX = 2.1;
  const worldMin = MIN_DISPLAY_PX / Math.max(0.0001, zoom);
  const SPEED_WIDTH_BONUS = 0.25;
  ctx.lineWidth = Math.max(s.radius * 2 * (1 + speedGlow * SPEED_WIDTH_BONUS), worldMin);

  // Body Path Construction
  ctx.beginPath();
  const pts = s.pts;
  if (pts.length >= 2) {
    const startX = pts[0] ?? s.x;
    const startY = pts[1] ?? s.y;
    ctx.moveTo(startX, startY);
    for (let i = 2; i < pts.length; i += 2) {
      const px = pts[i];
      const py = pts[i + 1];
      if (px === undefined || py === undefined) continue;
      ctx.lineTo(px, py);
    }
  }
  ctx.stroke();

  // Head and Eye Rendering:
  // The head is a circle slightly larger than the body radius.
  const HEAD_SCALE = 1.05;
  const hx = pts.length >= 2 ? (pts[0] ?? s.x) : s.x;
  const hy = pts.length >= 2 ? (pts[1] ?? s.y) : s.y;
  ctx.fillStyle = color;
  ctx.beginPath();
  ctx.arc(hx, hy, s.radius * HEAD_SCALE, 0, TAU);
  ctx.fill();
  ctx.shadowBlur = 0;

  // Eye Mapping:
  // We position eyes relative to the head heading (s.ang).
  const EYE_OFFSET_PCT = 0.45;
  const EYE_FORWARD_PCT = 0.35;
  const eyeOffset = s.radius * EYE_OFFSET_PCT;
  const eyeForward = s.radius * EYE_FORWARD_PCT;

  const sin = Math.sin(s.ang);
  const cos = Math.cos(s.ang);
  const px = -sin;
  const py = cos;
  const ex = hx + cos * eyeForward;
  const ey = hy + sin * eyeForward;

  // Robot eyes are slightly larger/more intense.
  const EYE_RAD_DEFAULT = 0.18;
  const EYE_RAD_ROBOT = 0.22;
  const eyeR = Math.max(1.2, s.radius * (s.skin === 2 ? EYE_RAD_ROBOT : EYE_RAD_DEFAULT));

  // Skin ID 2 (Robot) uses distinct eye colors and adds a reactive glow.
  if (s.skin === 2) {
    const ROBOT_GLOW_PCT = 0.6;
    ctx.fillStyle = THEME.snakeRobotEye;
    ctx.shadowColor = THEME.snakeRobotGlow;
    ctx.shadowBlur = s.radius * ROBOT_GLOW_PCT;
  } else {
    ctx.fillStyle = THEME.snakeSelfEye;
  }

  ctx.beginPath();
  ctx.arc(ex + px * eyeOffset, ey + py * eyeOffset, eyeR, 0, TAU);
  ctx.arc(ex - px * eyeOffset, ey - py * eyeOffset, eyeR, 0, TAU);
  ctx.fill();

  if (s.skin === 2) {
    ctx.shadowBlur = 0;
  }
}

/**
 * Render the world state from a binary buffer.
 * @param ctx - Canvas 2D context to draw into.
 * Fast-path renderer that consumes a serialized binary buffer to draw the entire world.
 * 
 * Performance Strategy:
 * 1. Minimize allocations: Uses a Float32Array and linear pointer scanning (`ptr`).
 * 2. Coordinate Mapping: 
 *    - Applies Device Pixel Ratio (DPR) to ensure crisp rendering on high-DPI screens.
 *    - Translates the origin to the screen center (viewW/2, viewH/2).
 *    - Scales by `zoom` and translates by `-cameraX, -cameraY` to move the world into view.
 * 3. Z-Ordering (Painter's Algorithm):
 *    - Background: Starfield and Grid.
 *    - Arena: World boundary circle.
 *    - Infrastructure: Pellets.
 *    - Entities: Snakes (on top of food).
 * 
 * @param ctx - Canvas 2D rendering context.
 * @param buffer - Flat Float32Array containing serialized frame data.
 * @param viewW - Viewport width in CSS pixels.
 * @param viewH - Viewport height in CSS pixels.
 * @param zoomOverride - Optional manual zoom for debugging or UI overlays.
 * @param camXOverride - Optional manual camera X.
 * @param camYOverride - Optional manual camera Y.
 */
export function renderWorldStruct(
  ctx: CanvasRenderingContext2D,
  buffer: Float32Array,
  viewW: number,
  viewH: number,
  zoomOverride?: number,
  camXOverride?: number,
  camYOverride?: number
): void {
  const read = (p: number) => buffer[p] ?? 0;

  // Buffer layout contract: [gen, totalSnakes, aliveCount, r, camX, camY, zoom] 
  // then for each alive snake: [id, radius, skinFlag, x, y, ang, boost, ptCount, ...pts], 
  // then pellets: [pelletCount, x, y, value, type, colorId] * pelletCount.

  // Linear parsing state for the binary contract.
  const dt = getRenderDt();
  renderTick += 1;

  // Handle High-DPI scaling.
  const dpr = ctx.getTransform().a || 1;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, viewW, viewH);

  // View Space Transformation:
  // Move origin to center, then apply simulation scaling and world translation.
  ctx.save();
  ctx.translate(viewW / 2, viewH / 2);

  let ptr = 0;
  const gen = read(ptr++);
  const totalCount = read(ptr++);
  const aliveCount = read(ptr++) | 0;
  const worldRadius = read(ptr++);
  const camX = read(ptr++);
  const camY = read(ptr++);
  const camZoom = read(ptr++);
  void gen;
  void totalCount;

  // Resolve final camera parameters.
  const zoom = zoomOverride || camZoom || 1;
  const cX = camXOverride ?? camX ?? 0;
  const cY = camYOverride ?? camY ?? 0;

  ctx.scale(zoom, zoom);
  ctx.translate(-cX, -cY);

  // Skip the fixed-size global header (7 floats) to reach entity data.
  ptr = FRAME_HEADER_FLOATS;

  // Layer 1: Environment
  drawStarfield(ctx, { zoom, cameraX: cX, cameraY: cY }, viewW, viewH);
  drawGrid(ctx, { zoom, cameraX: cX, cameraY: cY }, viewW, viewH);

  // Layer 2: World Boundaries
  const ARENA_BORDER_WIDTH = 3;
  ctx.strokeStyle = THEME.worldBorder;
  ctx.lineWidth = ARENA_BORDER_WIDTH;
  ctx.beginPath();
  ctx.arc(0, 0, worldRadius, 0, TAU);
  ctx.stroke();

  // Layer 3: Entities (Pellets and Snakes)
  // The buffer is ordered: [Header] -> [Snake 1] -> [Snake 2] ... -> [Snake N] -> [Pellet Count] -> [Pellets...]
  //
  // Z-Order Constraint (Painter's Algorithm): 
  // We want pellets to be drawn *underneath* snakes for visual clarity.
  // Because the buffer uses variable-length snake blocks (due to ptCount), we must perform 
  // a metadata pre-pass to find the starting pointer for the pellet section.
  const snakeMeta: SnakeMeta[] = [];

  for (let i = 0; i < aliveCount; i++) {
    const basePtr = ptr;
    const id = read(ptr++);
    const rad = read(ptr++);
    const skin = read(ptr++);
    const x = read(ptr++);
    const y = read(ptr++);
    const ang = read(ptr++);
    const boost = read(ptr++);

    // Body point count used for skipping and body drawing. 
    const ptCount = read(ptr++) | 0;
    const pointsEnd = ptr + ptCount * 2;
    if (pointsEnd > buffer.length) break;

    // Speed Calculation (Client-Side Interp):
    // We compute an instantaneous speed from the displacement since the last frame 
    // and smooth it to prevent jittery glow/particles.
    const prev = snakeRenderCache.get(id);
    let speed = 0;
    if (prev) {
      const dx = x - prev.x;
      const dy = y - prev.y;
      const inst = Math.hypot(dx, dy);
      const SMOOTHING_FACTOR = 0.65;
      const INST_FACTOR = 0.35;
      speed = prev.speed * SMOOTHING_FACTOR + inst * INST_FACTOR;
    }
    snakeRenderCache.set(id, { x, y, speed, seen: renderTick });

    snakeMeta.push({ basePtr, ptCount, id, rad, skin, x, y, ang, boost, speed });
    ptr = pointsEnd;
  };

  // Cleanup the transient cache for snakes that haven't been seen in several seconds.
  const CACHE_EXPIRY_TICKS = 120;
  for (const [id, data] of snakeRenderCache) {
    if (renderTick - data.seen > CACHE_EXPIRY_TICKS) snakeRenderCache.delete(id);
  }

  // Draw Pellets
  const pelletCount = read(ptr++) | 0;
  for (let i = 0; i < pelletCount; i++) {
    if (ptr + 4 >= buffer.length) break;
    const px = read(ptr++);
    const py = read(ptr++);
    const pv = read(ptr++);
    const type = read(ptr++);
    const colorId = read(ptr++);

    // Draw pellet
    // Can't use Pellet object.
    // Inline drawing
    let color: string | null = null;
    let glow: string | null = null;
    if (colorId > 0) {
      color = hashColor(colorId * 17 + 3);
      glow = color;
    }

    // Map Type to Color
    // 0=Ambient, 1=CorpseBig, 2=CorpseSmall, 3=Boost
    if (type === 0) {
      if (!color) {
        color = getPelletColor({ v: pv, kind: 'ambient' });
        glow = getPelletGlow({ kind: 'ambient' });
      }
    } else if (type === 1 || type === 2) {
      if (!color) {
        color = THEME.pelletCorpse;
        glow = THEME.glowCorpse;
      }
    } else {
      if (!color) {
        color = THEME.pelletBoost;
        glow = THEME.glowBoost;
      }
    }

    // Pellet size scaling: base size plus sqrt(value) to prevent huge food from 
    // obscuring the screen while still indicating high energy density.
    const BASE_PELLET_RAD = 2;
    const PELLET_VALUE_SCALE = 2;
    const r = BASE_PELLET_RAD + Math.sqrt(pv) * PELLET_VALUE_SCALE;

    ctx.fillStyle = color!;
    ctx.shadowBlur = r * 1.5;
    ctx.shadowColor = glow!;
    ctx.beginPath();
    ctx.arc(px, py, r, 0, TAU);
    ctx.fill();
    ctx.shadowBlur = 0;
  }

  // Boost effects: Spawn particles behind snakes that are currently boosting.
  for (const meta of snakeMeta) {
    if (meta.boost > 0.5) {
      // Scale particle density and velocity based on normalized speed.
      const BOOST_SPEED_THRESHOLD = Math.max(12, meta.rad * 2);
      const strength = clamp(meta.speed / BOOST_SPEED_THRESHOLD, 0, 1);
      const color = getSnakeColor(meta.id, meta.skin);

      const BASE_PARTICLE_COUNT = 1;
      const MAX_PARTICLE_BONUS = 2;
      const count = BASE_PARTICLE_COUNT + Math.floor(strength * MAX_PARTICLE_BONUS);

      for (let k = 0; k < count; k++) {
        spawnBoostParticle(meta.x, meta.y, meta.ang, color, strength);
      }
    }
  }
  renderBoostParticles(ctx, dt);

  // Draw Snakes
  for (const meta of snakeMeta) {
    let p = meta.basePtr;
    const id = read(p++);
    const rad = read(p++);
    const skin = read(p++);
    const x = read(p++);
    const y = read(p++);
    const ang = read(p++);
    const boost = read(p++);
    const ptCount = read(p++) | 0;

    // Reconstruct the points array from the shared buffer.
    // We use a subarray to avoid copying the body data while keeping drawSnakeStruct fast.
    const pts = buffer.subarray(p, p + ptCount * 2);

    const s: SnakeStruct = {
      id,
      radius: rad,
      skin,
      x,
      y,
      ang,
      boost,
      pts,
      speed: meta.speed
    };

    // Draw the calculated snake block.
    drawSnakeStruct(ctx, s, zoom);
  }
  ctx.restore();
}

/**
 * Render the entire world to the canvas.
 * @param ctx - Canvas 2D context to draw into.
 * @param world - World snapshot to render.
 * @param viewW - Viewport width in pixels.
 * @param viewH - Viewport height in pixels.
 * @param dpr - Device pixel ratio for scaling.
 */
export function renderWorld(
  ctx: CanvasRenderingContext2D,
  world: RenderWorld,
  viewW: number,
  viewH: number,
  dpr: number
): void {
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, viewW, viewH);
  ctx.save();
  ctx.translate(viewW / 2, viewH / 2);
  ctx.scale(world.zoom, world.zoom);
  ctx.translate(-world.cameraX, -world.cameraY);

  drawStarfield(ctx, world, viewW, viewH);
  drawGrid(ctx, world, viewW, viewH);

  // Draw particles (before snakes/pellets or after? After usually looks better for additive, or before for transparency)
  // Let's draw before pellets so pellets are on top, or maybe particles on top?
  // Boost particles should be below snakes probably.
  world.particles.render(ctx, world.cameraX, world.cameraY, world.zoom);

  // Draw pellets
  for (const p of world.pellets) {
    const kind = p.kind || 'ambient';
    let pr = 1.8 + Math.sqrt(Math.max(0, p.v)) * 0.9;
    if (kind === 'boost') pr *= 0.85;
    else if (kind === 'corpse_small') pr *= 1.10;
    else if (kind === 'corpse_big') pr *= 1.35;
    pr = clamp(pr, 1.1, 7.5);

    // Glow for pellets
    ctx.shadowBlur = pr * 2.0;
    ctx.shadowColor = getPelletGlow(p);

    ctx.fillStyle = getPelletColor(p);
    ctx.beginPath();
    ctx.arc(p.x, p.y, pr, 0, TAU);
    ctx.fill();

    ctx.shadowBlur = 0; // Reset
  }

  // Arena boundary
  ctx.strokeStyle = THEME.worldBorder;
  ctx.lineWidth = 3;
  ctx.beginPath();
  ctx.arc(0, 0, CFG.worldRadius, 0, TAU);
  ctx.stroke();

  // Draw snakes
  for (const s of world.snakes) if (s.alive) drawSnake(ctx, s, world.zoom);

  // Focused snake overlays
  if (world.focusSnake && world.focusSnake.alive && world.viewMode === 'follow') {
    const s = world.focusSnake;
    // Halo
    ctx.strokeStyle = THEME.snakeSnakeSelfHalo;
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.arc(s.x, s.y, s.radius * 2.0, 0, TAU);
    ctx.stroke();
    // Steering arrow
    const steerAngle = s.dir + (s.turnInput || 0) * (Math.PI / 2);
    const arrowLen = s.radius * 4;
    const tipX = s.x + Math.cos(steerAngle) * arrowLen;
    const tipY = s.y + Math.sin(steerAngle) * arrowLen;
    ctx.strokeStyle = THEME.snakeSteerArrow;
    ctx.lineWidth = 1.5;
    ctx.beginPath();
    ctx.moveTo(s.x, s.y);
    ctx.lineTo(tipX, tipY);
    ctx.stroke();
    const headLen = arrowLen * 0.25;
    for (const sign of [-1, 1]) {
      const ang = steerAngle + sign * 0.4;
      const hx = tipX - Math.cos(ang) * headLen;
      const hy = tipY - Math.sin(ang) * headLen;
      ctx.beginPath();
      ctx.moveTo(tipX, tipY);
      ctx.lineTo(hx, hy);
      ctx.stroke();
    }
    // Boost intent ring
    if ((s.boostInput ?? 0) > 0.35) {
      ctx.strokeStyle = THEME.snakeBoostActive;
      ctx.lineWidth = 2;
      ctx.beginPath();
      ctx.arc(s.x, s.y, s.radius * 2.4, 0, TAU);
      ctx.stroke();
    }
    // Wall distance gauge
    const distToWall = CFG.worldRadius - Math.hypot(s.x, s.y);
    const wallRatio = clamp(distToWall / CFG.worldRadius, 0, 1);
    ctx.strokeStyle = THEME.snakeWallGauge;
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.arc(s.x, s.y, s.radius * (0.8 + wallRatio * 1.8), 0, TAU);
    ctx.stroke();
    // Boost margin bar
    const margin = s.lastSensors && s.lastSensors.length > 3 ? (s.lastSensors[3] ?? -1) : -1;
    const barLen = s.radius * 3;
    const barDir = s.dir;
    const barX = s.x + Math.cos(barDir) * s.radius * 2.6;
    const barY = s.y + Math.sin(barDir) * s.radius * 2.6;
    ctx.strokeStyle = THEME.snakeBoostRefill;
    ctx.lineWidth = 3;
    ctx.beginPath();
    ctx.moveTo(barX, barY);
    ctx.lineTo(
      barX + Math.cos(barDir) * barLen * clamp((margin + 1) / 2, 0, 1),
      barY + Math.sin(barDir) * barLen * clamp((margin + 1) / 2, 0, 1)
    );
    ctx.stroke();
  }
  ctx.restore();
}
