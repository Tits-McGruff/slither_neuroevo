// render.js
// Helper functions for drawing the world onto the canvas.  These
// functions rely on the global 2D rendering context `ctx` which is
// passed in to the render function.

import { TAU, clamp, hashColor } from './utils.js';
import { THEME, getPelletColor, getPelletGlow } from './theme.js';
import { CFG } from './config.js';


function hash2(x, y) {
  let h = (x * 374761393 + y * 668265263) | 0;
  h ^= h >>> 13;
  h = Math.imul(h, 1274126177);
  return h >>> 0;
}

function nextRand(h) {
  h ^= h << 13;
  h ^= h >>> 17;
  h ^= h << 5;
  return h >>> 0;
}

export function drawStarfield(ctx, world, viewW, viewH) {
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
 * Draws a grid centred on the origin using a cached pattern.
 */
let bgPattern = null;
let bgCanvas = null;

export function drawGrid(ctx, world, viewW, viewH) {
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
    const bx = bgCanvas.getContext('2d');
    
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
  ctx.fillRect(left - step, top - step, w + step*2, h + step*2);
  
  ctx.restore();
}

/**
 * Renders a single snake.
 */
/**
 * Renders a single snake from serialized data.
 * struct: { id, radius, skin, x, y, ang, boost, pts: [x,y,x,y...] }
 */
export function drawSnakeStruct(ctx, s, zoom) {
  ctx.lineCap = "round";
  ctx.lineJoin = "round";
  
  // Resolve Color
  // 0.0 = Default, 1.0 = Gold.
  let color = THEME.snakeDefault;
  if (s.skin === 1.0) color = '#FFD700'; // Gold
  else {
      // Re-hash ID for color
      // Need hashColor from utils. But utils import is available.
      // We can duplicate hashColor logic here for speed or import.
      // Ideally import.
      // For now, let's assuming importing 'hashColor' works.
      // import { hashColor } from './utils.js';
      // Wait, we need to import it in this file.
      // It is imported at top of file.
      color = hashColor(s.id * 17 + 3);
  }

  // Shadow/Glow
  ctx.shadowBlur = s.radius * 1.5;
  ctx.shadowColor = color;
  ctx.strokeStyle = color;
  
  const minPx = 2.1;
  const worldMin = minPx / Math.max(0.0001, zoom);
  ctx.lineWidth = Math.max(s.radius * 2, worldMin);
  
  ctx.beginPath();
  const pts = s.pts;
  if (pts.length >= 2) {
      ctx.moveTo(pts[0], pts[1]);
      for (let i = 2; i < pts.length; i+=2) {
          ctx.lineTo(pts[i], pts[i+1]);
      }
  }
  ctx.stroke();
  
  ctx.shadowBlur = 0; // Reset
}

/**
 * Renders the world state from a binary buffer.
 * @param {Float32Array} flt 
 */
export function renderWorldStruct(ctx, flt, viewW, viewH, zoomOverride, camXOverride, camYOverride) {
  let ptr = 0;
  // Header: Gen, TotalCount, AliveCount, CamX, CamY, Zoom
  const gen = flt[ptr++];
  const totalCount = flt[ptr++];
  const aliveCount = flt[ptr++];
  const camX = flt[ptr++];
  const camY = flt[ptr++];
  const camZoom = flt[ptr++];
  
  const zoom = zoomOverride || camZoom;
  const cX = camXOverride || camX;
  const cY = camYOverride || camY;

  ptr = 6; // Skip header (3 original + 3 new)
  
  // Render starfield and grid (using efficient offscreen pattern)
  drawStarfield(ctx, { zoom, cameraX: cX, cameraY: cY }, viewW, viewH);
  drawGrid(ctx, { zoom, cameraX: cX, cameraY: cY }, viewW, viewH);
  
  // Render Pellets FIRST (so snakes are on top)
  // But we need to skip snakes to find pellets?
  // We don't know snake byte size because point count varies.
  // So we must parse linear.
  // Logic: 
  // We have a list of snakes. We can store them for drawing after pellets?
  // Or just draw snakes first? Snakes on top is better.
  // But efficient parsing is linear.
  // Two passes?
  // Pass 1: Parse snakes, store in temp array, find Pellets start.
  // Pass 2: Draw Pellets.
  // Pass 3: Draw Snakes.
  
  // Optimization: Render Pellets? Pellets are simple circles.
  // If we draw snakes first, pellets are hidden under them. Correct z-order is Pellets -> Snakes.
  
  // We MUST scan snakes to find pellets.
  // Let's just scan and store pointers?
  const snakePtrs = [];
  const count = aliveCount; // Wait, loop until we parse 'aliveCount' snakes?
  // Loop 'total' times? No, Serializer loops 'world.snakes'.
  // BUT Serializer writes 'AliveCount'.
  // And loops 'world.snakes' but `if (!s.alive) continue;`.
  // So buffer ONLY contains alive snakes.
  // So we read 'AliveCount' blocks.
  
  for (let i = 0; i < aliveCount; i++) {
      snakePtrs.push(ptr);
      // Skip this snake
      // ID, Rad, Skin, X, Y, Ang, Boost, PtCount
      // 8 floats.
      const ptCount = flt[ptr + 7];
      ptr += 8 + ptCount * 2;
  };
  
  // Now ptr is at Pellets Count
  const pelletCount = flt[ptr++];
  
  // Draw Pellets
  for (let i = 0; i < pelletCount; i++) {
      const px = flt[ptr++];
      const py = flt[ptr++];
      const pv = flt[ptr++];
      const type = flt[ptr++];
      
      // Draw pellet
      // Can't use Pellet object.
      // Inline drawing
      let color = '#fff';
      let glow = '#fff';
      
      // Map Type to Color
      // 0=Ambient, 1=CorpseBig, 2=CorpseSmall, 3=Boost
      if (type === 0) {
           // Ambient gradient? getPelletColor(pv)
           color = getPelletColor(pv);
           glow = getPelletGlow(pv);
      } else if (type === 1 || type === 2) {
           color = '#ff9999'; // Corpse 
           glow = '#ff0000';
      } else {
           color = '#00ff00'; // Boost
           glow = '#00ff00';
      }
      
      const r = 2 + Math.sqrt(pv) * 2; // Approx radius logic
      
      ctx.fillStyle = color;
      ctx.shadowBlur = r * 1.5;
      ctx.shadowColor = glow;
      ctx.beginPath();
      ctx.arc(px, py, r, 0, TAU);
      ctx.fill();
      ctx.shadowBlur = 0;
  }
  
  // Draw Snakes
  for (let idx of snakePtrs) {
      let p = idx;
      const id = flt[p++];
      const rad = flt[p++];
      const skin = flt[p++];
      const x = flt[p++];
      const y = flt[p++];
      const ang = flt[p++];
      const boost = flt[p++];
      const ptCount = flt[p++];
      
      // Reconstruct points array wrapper
      // We can't use subarray as points because it's [x,y,x,y].
      // drawSnakeStruct expects [x,y,x,y] in 'pts' prop.
      // We can pass the typed array subarray?
      const pts = flt.subarray(p, p + ptCount * 2);
      
      const s = {
          id, radius: rad, skin, x, y, ang, boost, pts
      };
      
      drawSnakeStruct(ctx, s, zoom);
  }
}

/**
 * Renders the entire world to the canvas.
 * @param {CanvasRenderingContext2D} ctx
 * @param {World} world
 * @param {number} viewW
 * @param {number} viewH
 * @param {number} dpr
 */
export function renderWorld(ctx, world, viewW, viewH, dpr) {
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
  for (let i = 0; i < world.pellets.length; i++) {
    const p = world.pellets[i];
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
    if (s.boostInput > 0.35) {
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
    const margin = s.lastSensors && s.lastSensors.length > 3 ? s.lastSensors[3] : -1;
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
