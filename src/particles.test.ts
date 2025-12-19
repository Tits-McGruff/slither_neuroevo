import { describe, it, expect } from 'vitest';
import { ParticleSystem } from './particles.ts';

describe('particles.ts', () => {
  it('spawns particles and expires them after updates', () => {
    const system = new ParticleSystem(2);
    system.spawn(0, 0, 0, 0, 1, 1, 0.05, 2, '#fff', 1);

    const activeAfterSpawn = system.pool.filter(p => p.active).length;
    expect(activeAfterSpawn).toBe(1);

    system.update(1);
    const activeAfterUpdate = system.pool.filter(p => p.active).length;
    expect(activeAfterUpdate).toBe(0);
  });

  it('spawnBurst activates multiple particles', () => {
    const system = new ParticleSystem(5);
    system.spawnBurst(0, 0, '#0f0', 3, 2.5);

    const activeCount = system.pool.filter(p => p.active).length;
    expect(activeCount).toBeGreaterThanOrEqual(1);
  });
});
