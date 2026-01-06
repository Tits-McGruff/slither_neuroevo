/** High-performance particle system for boost exhaust, deaths, and impacts. */

import { rand, clamp, TAU } from './utils.ts';

/** Internal particle instance reused via pooling. */
class Particle {
  /** Current X position. */
  x: number;
  /** Current Y position. */
  y: number;
  /** Velocity in X. */
  vx: number;
  /** Velocity in Y. */
  vy: number;
  /** Remaining lifetime in seconds. */
  life: number;
  /** Initial lifetime in seconds. */
  maxLife: number;
  /** Current render size. */
  size: number;
  /** Fill color string. */
  color: string;
  /** Velocity decay factor per tick. */
  decay: number;
  /** Size shrink rate per tick. */
  shrink: number;
  /** Whether this particle is active. */
  active: boolean;

  /** Create a particle with default inactive state. */
  constructor() {
    this.x = 0;
    this.y = 0;
    this.vx = 0;
    this.vy = 0;
    this.life = 0;
    this.maxLife = 1;
    this.size = 1;
    this.color = '#fff';
    this.decay = 0.95; // Velocity decay
    this.shrink = 0;  // Size shrink per frame
    this.active = false;
  }
  
  /**
   * Initialize particle state for a new spawn.
   * @param x - Spawn x position.
   * @param y - Spawn y position.
   * @param angle - Spawn direction in radians.
   * @param speed - Spawn speed.
   * @param life - Lifetime in seconds.
   * @param size - Starting size.
   * @param color - Fill color.
   */
  spawn(
    x: number,
    y: number,
    angle: number,
    speed: number,
    life: number,
    size: number,
    color: string
  ): void {
    this.x = x;
    this.y = y;
    this.vx = Math.cos(angle) * speed;
    this.vy = Math.sin(angle) * speed;
    this.life = life;
    this.maxLife = life;
    this.size = size;
    this.color = color;
    this.active = true;
    // Randomize decay slightly
    this.decay = rand(0.96, 0.90);
    this.shrink = size / (life * 60); // Approx shrink to 0
  }

  /**
   * Advance the particle simulation by dt seconds.
   * @param dt - Delta time in seconds.
   */
  update(dt: number): void {
    if (!this.active) return;
    this.life -= dt;
    if (this.life <= 0) {
      this.active = false;
      return;
    }
    this.x += this.vx * dt * 60; // Normalize speed to 60fps scale
    this.y += this.vy * dt * 60;
    this.vx *= this.decay;
    this.vy *= this.decay;
    this.size = Math.max(0, this.size - this.shrink * dt * 60);
  }
}

/** Particle system with fixed-size pool for real-time effects. */
export class ParticleSystem {
  /** Pool of reusable particle instances. */
  pool: Particle[];
  /** Number of active particles. */
  count: number;

  /**
   * Create a particle system with a fixed pool size.
   * @param capacity - Number of particles to allocate in the pool.
   */
  constructor(capacity = 2000) {
    this.pool = [];
    for (let i = 0; i < capacity; i++) this.pool.push(new Particle());
    this.count = 0;
  }

  /**
   * Spawn one or more particles using a spread and speed range.
   * @param x - Spawn x position.
   * @param y - Spawn y position.
   * @param baseAngle - Center direction in radians.
   * @param spread - Angular spread in radians.
   * @param speedMin - Minimum spawn speed.
   * @param speedMax - Maximum spawn speed.
   * @param life - Lifetime in seconds.
   * @param size - Starting size.
   * @param color - Fill color.
   * @param count - Number of particles to spawn.
   */
  spawn(
    x: number,
    y: number,
    baseAngle: number,
    spread: number,
    speedMin: number,
    speedMax: number,
    life: number,
    size: number,
    color: string,
    count = 1
  ): void {
    let spawned = 0;
    for (let i = 0; i < this.pool.length; i++) {
      const p = this.pool[i];
      if (!p) continue;
      if (!p.active) {
        const a = baseAngle + rand(spread, -spread);
        const s = rand(speedMax, speedMin);
        p.spawn(x, y, a, s, life, size, color);
        spawned++;
        if (spawned >= count) break;
      }
    }
  }

  /**
   * Spawn a burst of particles centered on a point.
   * @param x - Spawn x position.
   * @param y - Spawn y position.
   * @param color - Fill color.
   * @param count - Number of particles to spawn.
   * @param speed - Base speed for the burst.
   */
  spawnBurst(x: number, y: number, color: string, count = 10, speed = 2.0): void {
    this.spawn(x, y, 0, Math.PI, speed * 0.5, speed * 1.5, 0.6, rand(3, 1), color, count);
  }
  
  /**
   * Spawn a single boost exhaust particle.
   * @param x - Spawn x position.
   * @param y - Spawn y position.
   * @param angle - Snake heading angle in radians.
   * @param color - Fill color.
   */
  spawnBoost(x: number, y: number, angle: number, color: string): void {
    // Exhaust opposite to angle
    this.spawn(x, y, angle + Math.PI, 0.4, 0.5, 2.5, 0.4, rand(3, 1.5), color, 1);
  }

  /**
   * Advance all active particles by dt seconds.
   * @param dt - Delta time in seconds.
   */
  update(dt: number): void {
    for (const p of this.pool) p.update(dt);
  }

  /**
   * Render all active particles to the canvas.
   * @param ctx - Canvas 2D context to draw into.
   * @param cameraX - Camera x position (unused).
   * @param cameraY - Camera y position (unused).
   * @param zoom - Camera zoom (unused).
   */
  render(ctx: CanvasRenderingContext2D, cameraX: number, cameraY: number, zoom: number): void {
    void cameraX;
    void cameraY;
    void zoom;
    ctx.save();
    // Batch drawing could be optimized but standard path stroking/filling is fine for <2000 particles
    // We can group by color if needed, but simple iteration is okay for now.
    
    // Global composition for additive blending looks nice for particles
    ctx.globalCompositeOperation = 'lighter';
    
    for (const p of this.pool) {
      if (!p.active) continue;
      // Fade out alpha
      const alpha = clamp(p.life / p.maxLife, 0, 1);
      ctx.globalAlpha = alpha;
      ctx.fillStyle = p.color;
      ctx.beginPath();
      ctx.arc(p.x, p.y, p.size, 0, TAU);
      ctx.fill();
    }
    
    ctx.restore();
  }
}
