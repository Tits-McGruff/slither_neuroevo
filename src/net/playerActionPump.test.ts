import { describe, expect, it } from 'vitest';
import {
  PlayerActionPump,
  normalizePlayerActionRate,
  type PlayerActionPumpClock
} from './playerActionPump.ts';

/** Deterministic clock with manually advanced timeout/interval callbacks. */
class FakePumpClock implements PlayerActionPumpClock {
  /** Current monotonic test time. */
  now = 0;
  /** Next timer identity. */
  nextId = 1;
  /** Scheduled timers keyed by identity. */
  timers = new Map<number, { callback: () => void; due: number; interval: number | null }>();

  /** @returns Current fake monotonic time. */
  nowMs = (): number => this.now;

  /** Schedule one timeout. */
  setTimeout = (callback: () => void, delayMs: number): ReturnType<typeof setTimeout> => {
    const id = this.nextId++;
    this.timers.set(id, { callback, due: this.now + delayMs, interval: null });
    return id as unknown as ReturnType<typeof setTimeout>;
  };

  /** Cancel one timeout. */
  clearTimeout = (timer: ReturnType<typeof setTimeout>): void => {
    this.timers.delete(timer as unknown as number);
  };

  /** Schedule one interval. */
  setInterval = (callback: () => void, delayMs: number): ReturnType<typeof setInterval> => {
    const id = this.nextId++;
    this.timers.set(id, { callback, due: this.now + delayMs, interval: delayMs });
    return id as unknown as ReturnType<typeof setInterval>;
  };

  /** Cancel one interval. */
  clearInterval = (timer: ReturnType<typeof setInterval>): void => {
    this.timers.delete(timer as unknown as number);
  };

  /**
   * Advance fake time and run every due callback in chronological order.
   * @param elapsedMs - Milliseconds to advance.
   */
  advance(elapsedMs: number): void {
    const target = this.now + elapsedMs;
    while (true) {
      let nextId: number | null = null;
      let nextDue = Number.POSITIVE_INFINITY;
      for (const [id, timer] of this.timers) {
        if (timer.due < nextDue) {
          nextId = id;
          nextDue = timer.due;
        }
      }
      if (nextId === null || nextDue > target) break;
      this.now = nextDue;
      const timer = this.timers.get(nextId);
      if (!timer) continue;
      if (timer.interval === null) this.timers.delete(nextId);
      else timer.due += timer.interval;
      timer.callback();
    }
    this.now = target;
  }
}

describe('PlayerActionPump', () => {
  it('accepts only the two Stage 2 measurement candidates', () => {
    expect(normalizePlayerActionRate(30)).toBe(30);
    expect(normalizePlayerActionRate('60')).toBe(60);
    expect(normalizePlayerActionRate(45)).toBe(60);
  });

  it('sends latest pointer and boost changes without any sensor callback', () => {
    const clock = new FakePumpClock();
    let active = true;
    let turn = 0;
    let boost = 1;
    const sent: Array<{ turn: number; boost: number }> = [];
    const pump = new PlayerActionPump({
      cadenceHz: 60,
      isActive: () => active,
      buildLatestAction: () => ({ tick: 4, snakeId: 9, turn, boost }),
      sendAction: action => sent.push({ turn: action.turn, boost: action.boost }),
      clock
    });
    pump.start();
    expect(sent).toEqual([{ turn: 0, boost: 1 }]);

    turn = 0.75;
    boost = 0;
    pump.requestImmediate();
    turn = -0.5;
    pump.requestImmediate();
    clock.advance(17);

    expect(sent.at(-1)).toEqual({ turn: -0.5, boost: 0 });
    active = false;
    clock.advance(100);
    expect(sent).toHaveLength(2);
  });

  it('periodically resends the newest command while ownership is active', () => {
    const clock = new FakePumpClock();
    const sent: number[] = [];
    let turn = 0.1;
    const pump = new PlayerActionPump({
      cadenceHz: 30,
      isActive: () => true,
      buildLatestAction: () => ({ tick: 1, snakeId: 2, turn, boost: 0 }),
      sendAction: action => sent.push(action.turn),
      clock
    });
    pump.start();
    turn = 0.9;
    clock.advance(67);

    expect(sent).toEqual([0.1, 0.9, 0.9]);
    pump.stop();
  });
});
