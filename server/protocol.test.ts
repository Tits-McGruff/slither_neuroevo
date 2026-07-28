import { describe, it, expect } from 'vitest';
import { getProtocolVersionError, parseClientMessage } from './protocol.ts';
import type { StatsMsg } from './protocol.ts';

/** Test suite label for server protocol validation. */
const SUITE = 'server protocol';

describe(SUITE, () => {
  it('accepts a valid hello message', () => {
    const msg = parseClientMessage({ type: 'hello', clientType: 'ui', version: 2 });
    expect(msg?.type).toBe('hello');
  });

  it('describes Protocol 1 as explicitly incompatible', () => {
    const hello = { type: 'hello', clientType: 'ui', version: 1 };
    expect(parseClientMessage(hello)).toBeNull();
    expect(getProtocolVersionError(hello)).toContain('server requires 2');
  });

  it('rejects hello with NaN version', () => {
    const msg = parseClientMessage({
      type: 'hello',
      clientType: 'ui',
      version: Number.NaN
    });
    expect(msg).toBeNull();
  });

  it('accepts a valid action message', () => {
    const msg = parseClientMessage({
      type: 'action',
      tick: 12,
      snakeId: 2,
      turn: -0.4,
      boost: 1
    });
    expect(msg?.type).toBe('action');
  });

  it('accepts a valid view message', () => {
    const msg = parseClientMessage({
      type: 'view',
      viewW: 1280,
      viewH: 720,
      mode: 'overview'
    });
    expect(msg?.type).toBe('view');
  });

  it('accepts a valid viz message', () => {
    const msg = parseClientMessage({
      type: 'viz',
      enabled: true
    });
    expect(msg?.type).toBe('viz');
  });

  it('accepts a valid reset message', () => {
    const msg = parseClientMessage({
      type: 'reset',
      settings: {
        snakeCount: 80,
        simSpeed: 1.25,
        hiddenLayers: 2,
        neurons1: 64,
        neurons2: 64,
        neurons3: 64,
        neurons4: 48,
        neurons5: 32
      },
      updates: [{ path: 'worldRadius', value: 3200 }]
    });
    expect(msg?.type).toBe('reset');
  });

  it('accepts sensor settings updates in reset payloads', () => {
    const msg = parseClientMessage({
      type: 'reset',
      settings: {
        snakeCount: 80,
        simSpeed: 1.25,
        hiddenLayers: 2,
        neurons1: 64,
        neurons2: 64,
        neurons3: 64,
        neurons4: 48,
        neurons5: 32
      },
      updates: [{ path: 'sense.bubbleBins', value: 12 }]
    });
    expect(msg?.type).toBe('reset');
  });

  it('rejects reset updates with unknown settings paths', () => {
    const msg = parseClientMessage({
      type: 'reset',
      settings: {
        snakeCount: 80,
        simSpeed: 1.25,
        hiddenLayers: 2,
        neurons1: 64,
        neurons2: 64,
        neurons3: 64,
        neurons4: 48,
        neurons5: 32
      },
      updates: [{ path: 'sense.unknownField', value: 12 }]
    });
    expect(msg).toBeNull();
  });

  it('accepts strict Protocol 2 settings, God Mode, and New Run messages', () => {
    expect(parseClientMessage({
      type: 'settings',
      requestId: 'settings-1',
      updates: [{ path: 'simSpeed', value: 2.5 }]
    })?.type).toBe('settings');
    expect(parseClientMessage({
      type: 'godMode',
      requestId: 'god-1',
      action: 'move',
      snakeId: 7,
      x: 12,
      y: -4
    })?.type).toBe('godMode');
    expect(parseClientMessage({ type: 'newRun', requestId: 'run-1' })?.type).toBe('newRun');
  });

  it('rejects unknown fields and non-finite command coordinates', () => {
    expect(parseClientMessage({
      type: 'settings',
      requestId: 'settings-1',
      updates: [{ path: 'simSpeed', value: 2 }],
      unexpected: true
    })).toBeNull();
    expect(parseClientMessage({
      type: 'godMode',
      requestId: 'god-1',
      action: 'move',
      snakeId: 7,
      x: Number.NaN,
      y: 0
    })).toBeNull();
  });

  it('stats requires total fields', () => {
    const stats: StatsMsg = {
      type: 'stats',
      tick: 1,
      gen: 1,
      generationTime: 12,
      generationSeconds: 240,
      alive: 2,
      aliveTotal: 3,
      baselineBotsAlive: 1,
      baselineBotsTotal: 1,
      fps: 60,
      collisionGrid: {
        currentEntries: 10,
        peakEntries: 12,
        capacity: 100,
        maxCapacity: 1000,
        estimatedCapacityBytes: 1600,
        rebuilds: 5,
        growths: 0,
        outOfBoundsEntries: 0,
        admissionReason: 'test fixture',
        faultReason: null
      }
    };
    expect(stats.aliveTotal).toBe(3);
    // @ts-expect-error stats requires total fields
    const _missingTotals: StatsMsg = {
      type: 'stats',
      tick: 1,
      gen: 1,
      generationTime: 12,
      generationSeconds: 240,
      alive: 2,
      fps: 60
    };
    expect(stats.type).toBe('stats');
  });
});
