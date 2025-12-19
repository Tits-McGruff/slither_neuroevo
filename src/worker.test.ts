import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { CFG, resetCFGToDefaults } from './config.ts';

describe('worker.ts', () => {
  const globalAny = globalThis as any;
  let originalSelf: any;

  beforeEach(() => {
    originalSelf = globalAny.self;
    globalAny.self = {
      onmessage: null,
      postMessage: () => {}
    } as any;
  });

  afterEach(() => {
    globalAny.self = originalSelf;
    resetCFGToDefaults();
  });

  it('applies updateSettings messages to CFG', async () => {
    await import('./worker.ts');

    (globalAny.self.onmessage as any)({
      data: {
        type: 'updateSettings',
        updates: [{ path: 'collision.cellSize', value: 123 }]
      }
    });

    expect(CFG.collision.cellSize).toBe(123);
  });
});
