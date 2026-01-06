import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { CFG, resetCFGToDefaults } from './config.ts';

describe('worker.ts', () => {
  /** Minimal worker scope stub for message handling tests. */
  type WorkerScope = {
    onmessage: ((event: MessageEvent) => void) | null;
    postMessage: (message: unknown) => void;
  };
  /** Global shim for worker self assignment. */
  const globalAny = globalThis as Record<string, unknown> & { self?: unknown };
  let originalSelf: unknown;

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
        updates: [{ path: 'collision.cellSize', value: 123 }]
      }
    } as MessageEvent);

    expect(CFG.collision.cellSize).toBe(123);
  });
});
