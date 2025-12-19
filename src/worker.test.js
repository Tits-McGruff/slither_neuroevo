import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { CFG, resetCFGToDefaults } from './config.js';

describe('worker.ts', () => {
  let originalSelf;

  beforeEach(() => {
    originalSelf = globalThis.self;
    globalThis.self = {
      onmessage: null,
      postMessage: () => {}
    };
  });

  afterEach(() => {
    globalThis.self = originalSelf;
    resetCFGToDefaults();
  });

  it('applies updateSettings messages to CFG', async () => {
    await import('./worker.ts');

    globalThis.self.onmessage({
      data: {
        type: 'updateSettings',
        updates: [{ path: 'collision.cellSize', value: 123 }]
      }
    });

    expect(CFG.collision.cellSize).toBe(123);
  });
});
