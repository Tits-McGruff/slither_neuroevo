import { describe, it, expect, beforeAll } from 'vitest';
import { performance } from 'node:perf_hooks';
import { World } from '../src/world.ts';
import { WorldSerializer } from '../src/serializer.ts';
import { CFG, resetCFGToDefaults, syncBrainInputSize } from '../src/config.ts';
import { loadSimdKernels } from '../src/brains/wasmBridge.ts';

/** Test suite label for server performance checks. */
const SUITE = 'performance: world tick + serialize';

describe(SUITE, () => {
  beforeAll(async () => {
    await loadSimdKernels();
  });

  it('ticks 60 frames under a reasonable budget', () => {
    resetCFGToDefaults();
    const originalBaselineBots = CFG.baselineBots.count;
    CFG.baselineBots.count = 0;
    syncBrainInputSize();
    try {
      const world = new World({ snakeCount: 20, simSpeed: 1 });
      const frames = 60;
      const start = performance.now();
      for (let i = 0; i < frames; i++) {
        world.update(1 / 60, 800, 600);
        WorldSerializer.serialize(world);
      }
      const elapsed = performance.now() - start;
      const msPerFrame = elapsed / frames;
      // Generous budget to avoid CI flakiness.
      expect(msPerFrame).toBeLessThan(40);
    } finally {
      CFG.baselineBots.count = originalBaselineBots;
      resetCFGToDefaults();
    }
  });
});
