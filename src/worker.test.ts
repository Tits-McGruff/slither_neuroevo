import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { CFG, resetCFGToDefaults } from './config.ts';

describe('worker.ts', () => {
  type WorkerScope = {
    onmessage: ((event: MessageEvent) => void) | null;
    postMessage: (message: unknown) => void;
  };
  const globalAny = globalThis as typeof globalThis & { self?: WorkerScope };
  let originalSelf: WorkerScope | undefined;

  beforeEach(() => {
    originalSelf = globalAny.self;
    globalAny.self = {
      onmessage: null,
      postMessage: () => {}
    };
  });

  afterEach(() => {
    globalAny.self = originalSelf;
    resetCFGToDefaults();
  });

  it('applies updateSettings messages to CFG', async () => {
    await import('./worker.ts');

    const handler = globalAny.self?.onmessage;
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
