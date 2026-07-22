import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createAuthoritativeControls } from './authoritativeControls.ts';
import type { LiveSettingsUpdate } from '../protocol/settings.ts';

/** Captured settings transport call. */
interface SettingsCall {
  /** Generated request id. */
  requestId: string;
  /** Coalesced settings batch. */
  updates: LiveSettingsUpdate[];
}

/** Captured God Mode move transport call. */
interface MoveCall {
  /** Generated request id. */
  requestId: string;
  /** Target snake id. */
  snakeId: number;
  /** Requested X. */
  x: number;
  /** Requested Y. */
  y: number;
}

describe('authoritative control client', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(0);
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('coalesces only unsent values for the same path into one atomic request', () => {
    const calls: SettingsCall[] = [];
    const controls = createAuthoritativeControls({
      sendSettings: (requestId, updates) => calls.push({ requestId, updates }),
      sendGodModeKill: () => {},
      sendGodModeMove: () => {},
      sendNewRun: () => {}
    }, {
      requestIdPrefix: 'client-a',
      settingsDebounceMs: 80,
      clock: {
        now: () => Date.now(),
        schedule: (callback, delayMs) => setTimeout(callback, delayMs),
        cancel: (timer) => clearTimeout(timer)
      }
    });

    controls.queueSetting('simSpeed', 2);
    controls.queueSetting('simSpeed', 3);
    controls.queueSetting('sense.debug', 1);
    vi.advanceTimersByTime(79);
    expect(calls).toHaveLength(0);
    vi.advanceTimersByTime(1);

    expect(calls).toEqual([{
      requestId: 'client-a-1',
      updates: [
        { path: 'simSpeed', value: 3 },
        { path: 'sense.debug', value: 1 }
      ]
    }]);
  });

  it('throttles intermediate drag positions and always sends final mouse-up state', () => {
    const moves: MoveCall[] = [];
    const controls = createAuthoritativeControls({
      sendSettings: () => {},
      sendGodModeKill: () => {},
      sendGodModeMove: (requestId, snakeId, x, y) =>
        moves.push({ requestId, snakeId, x, y }),
      sendNewRun: () => {}
    }, {
      requestIdPrefix: 'client-b',
      dragThrottleMs: 50,
      clock: {
        now: () => Date.now(),
        schedule: (callback, delayMs) => setTimeout(callback, delayMs),
        cancel: (timer) => clearTimeout(timer)
      }
    });

    controls.moveSnake(9, 1, 2);
    vi.advanceTimersByTime(10);
    controls.moveSnake(9, 3, 4);
    vi.advanceTimersByTime(10);
    controls.finishMove(9, 8, 10);
    vi.advanceTimersByTime(100);

    expect(moves).toEqual([
      { requestId: 'client-b-1', snakeId: 9, x: 1, y: 2 },
      { requestId: 'client-b-2', snakeId: 9, x: 8, y: 10 }
    ]);
  });

  it('exposes immediate kill and New Run transport methods', () => {
    const types: string[] = [];
    const controls = createAuthoritativeControls({
      sendSettings: () => {},
      sendGodModeKill: () => types.push('kill'),
      sendGodModeMove: () => {},
      sendNewRun: () => types.push('newRun')
    }, { requestIdPrefix: 'client-c' });

    expect(controls.killSnake(4)).toBe('client-c-1');
    expect(controls.requestNewRun()).toBe('client-c-2');
    expect(types).toEqual(['kill', 'newRun']);
  });
});
