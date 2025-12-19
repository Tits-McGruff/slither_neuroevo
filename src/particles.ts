// particles.ts
// A simple, high-performance particle system for visual effects like
// boost exhaust, deaths, and impacts.

import { rand, randInt, clamp, TAU } from './utils.ts';

class Particle {
  x: number;
  y: number;
  vx: number;
  vy: number;
  life: number;
  maxLife: number;
  size: number;
  color: string;
  decay: number;
  shrink: number;
  active: boolean;

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

export class ParticleSystem {
  pool: Particle[];
  count: number;

  constructor(capacity = 2000) {
    this.pool = [];
    for (let i = 0; i < capacity; i++) this.pool.push(new Particle());
    this.count = 0;
  }

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
      if (!p.active) {
        const a = baseAngle + rand(spread, -spread);
        const s = rand(speedMax, speedMin);
        p.spawn(x, y, a, s, life, size, color);
        spawned++;
        if (spawned >= count) break;
      }
    }
  }

  spawnBurst(x: number, y: number, color: string, count = 10, speed = 2.0): void {
    this.spawn(x, y, 0, Math.PI, speed * 0.5, speed * 1.5, 0.6, rand(3, 1), color, count);
  }
  
  spawnBoost(x: number, y: number, angle: number, color: string): void {
    // Exhaust opposite to angle
    this.spawn(x, y, angle + Math.PI, 0.4, 0.5, 2.5, 0.4, rand(3, 1.5), color, 1);
  }

  update(dt: number): void {
    for (const p of this.pool) p.update(dt);
  }

  render(ctx: CanvasRenderingContext2D, cameraX: number, cameraY: number, zoom: number): void {
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
