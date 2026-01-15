import { describe, it, expect, beforeEach, afterEach, beforeAll } from 'vitest';
import { CFG, resetCFGToDefaults } from './config.ts';
import { World } from './world.ts';
import { WorldSerializer } from './serializer.ts';
import { loadSimdKernels } from './brains/wasmBridge.ts';

describe('worker.ts', () => {
  /** Minimal worker scope stub for message handling tests. */
  type WorkerScope = {
    onmessage: ((event: MessageEvent) => void) | null;
    postMessage: (message: unknown) => void;
  };
  /** Global shim for worker self assignment. */
  const globalAny = globalThis as Record<string, unknown> & { self?: unknown };
  let originalSelf: unknown;

  beforeAll(async () => {
    await loadSimdKernels();
  });

  beforeEach(() => {
    originalSelf = globalAny.self;
    globalAny.self = {
      onmessage: null,
      postMessage: () => {}
    } as WorkerScope;
  });

  afterEach(() => {
    globalAny.self = originalSelf;
    resetCFGToDefaults();
  });

  it('applies updateSettings messages to CFG', async () => {
    await import('./worker.ts');

    const handler = (globalAny.self as WorkerScope | undefined)?.onmessage;
    if (!handler) throw new Error('Expected worker onmessage handler');
    handler({
      data: {
        type: 'updateSettings',
        updates: [
          { path: 'collision.cellSize', value: 123 },
          { path: 'sense.maxPelletChecks', value: 750 },
          { path: 'sense.layoutVersion', value: 1 }
        ]
      }
    } as MessageEvent);

    expect(CFG.collision.cellSize).toBe(123);
    expect(CFG.sense.maxPelletChecks).toBe(750);
    expect(CFG.sense.layoutVersion).toBe('v2');
  });

  it('stats exclude baseline bots and include totals', async () => {
    resetCFGToDefaults();
    CFG.baselineBots.count = 1;
    try {
      const { buildWorkerStats } = await import('./worker.ts');
      const world = new World({ snakeCount: 1 });
      const result = buildWorkerStats(world, 1 / 60, 0, false, 0);
      expect(result.stats.alive).toBe(1);
      expect(result.stats.aliveTotal).toBe(2);
      expect(result.stats.baselineBotsAlive).toBe(1);
      expect(result.stats.baselineBotsTotal).toBe(1);
    } finally {
      resetCFGToDefaults();
    }
  });

  it('frame header includes bots while stats do not', async () => {
    resetCFGToDefaults();
    CFG.baselineBots.count = 1;
    try {
      const { buildWorkerStats } = await import('./worker.ts');
      const world = new World({ snakeCount: 1 });
      const statsResult = buildWorkerStats(world, 1 / 60, 0, false, 0);
      const buffer = WorldSerializer.serialize(world);
      const aliveCount = buffer[2];
      expect(aliveCount).toBe(2);
      expect(statsResult.stats.alive).toBe(1);
    } finally {
      resetCFGToDefaults();
    }
  });
});
