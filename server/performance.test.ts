import { describe, it, expect } from 'vitest';
import { performance } from 'node:perf_hooks';
import { World } from '../src/world.ts';
import { WorldSerializer } from '../src/serializer.ts';

/** Test suite label for server performance checks. */
const SUITE = 'performance: world tick + serialize';

describe(SUITE, () => {
  it('ticks 120 frames under a reasonable budget', () => {
    const world = new World({ snakeCount: 40, simSpeed: 1 });
    const frames = 120;
    const start = performance.now();
    for (let i = 0; i < frames; i++) {
      world.update(1 / 60, 800, 600);
      WorldSerializer.serialize(world);
    }
    const elapsed = performance.now() - start;
    const msPerFrame = elapsed / frames;
    // Generous budget to avoid CI flakiness.
    expect(msPerFrame).toBeLessThan(16);
  });
});
