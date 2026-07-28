import { readFileSync } from 'node:fs';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { CFG, resetCFGToDefaults } from '../src/config.ts';
import {
  PlayerActionPump,
  type PlayerActionPumpClock
} from '../src/net/playerActionPump.ts';
import { World } from '../src/world.ts';
import { ControllerRegistry } from './controllerRegistry.ts';
import { parseClientMessage } from './protocol.ts';

/** Minimal controllable clock for one trailing browser send. */
class ControlClock implements PlayerActionPumpClock {
  /** Current monotonic wall time. */
  now = 0;
  /** Next timer identity. */
  nextId = 1;
  /** Pending timeout/interval callbacks. */
  timers = new Map<number, { callback: () => void; due: number; interval: number | null }>();

  /** @returns Current wall time. */
  nowMs = (): number => this.now;
  /** Schedule a timeout. */
  setTimeout = (callback: () => void, delay: number): ReturnType<typeof setTimeout> => {
    const id = this.nextId++;
    this.timers.set(id, { callback, due: this.now + delay, interval: null });
    return id as unknown as ReturnType<typeof setTimeout>;
  };
  /** Cancel a timeout. */
  clearTimeout = (timer: ReturnType<typeof setTimeout>): void => {
    this.timers.delete(timer as unknown as number);
  };
  /** Schedule an interval. */
  setInterval = (callback: () => void, delay: number): ReturnType<typeof setInterval> => {
    const id = this.nextId++;
    this.timers.set(id, { callback, due: this.now + delay, interval: delay });
    return id as unknown as ReturnType<typeof setInterval>;
  };
  /** Cancel an interval. */
  clearInterval = (timer: ReturnType<typeof setInterval>): void => {
    this.timers.delete(timer as unknown as number);
  };

  /**
   * Advance wall time and execute callbacks that became due.
   * @param elapsed - Milliseconds to advance.
   */
  advance(elapsed: number): void {
    const target = this.now + elapsed;
    while (true) {
      const due = Array.from(this.timers.entries())
        .filter(([, timer]) => timer.due <= target)
        .sort((left, right) => left[1].due - right[1].due)[0];
      if (!due) break;
      const [id, timer] = due;
      this.now = timer.due;
      if (timer.interval === null) this.timers.delete(id);
      else timer.due += timer.interval;
      timer.callback();
    }
    this.now = target;
  }
}

beforeEach(() => {
  resetCFGToDefaults();
  CFG.baselineBots.count = 0;
  CFG.pelletCountTarget = 0;
  CFG.pelletSpawnPerSecond = 0;
  CFG.generationSeconds = 10_000;
  CFG.observer.earlyEndAliveThreshold = -1;
});

afterEach(() => {
  resetCFGToDefaults();
});

describe('Stage 1 browser-player transmission correction', () => {
  it('delivers steering and boost release while sensors are suppressed and display is stalled', async () => {
    const clock = new ControlClock();
    const world = new World({ snakeCount: 1 }, { seed: 9201 });
    world.snakes[0]!.alive = false;
    const external = world.spawnExternalSnake();
    let suppressedSensors = 0;
    const registry = new ControllerRegistry(
      {
        maxActionsPerTick: 4,
        maxActionsPerSecond: 120,
        inputHoldMs: 500,
        disconnectGraceMs: 30_000
      },
      {
        getSnakes: () =>
          world.snakes.map(snake => ({
            id: snake.id,
            alive: snake.alive,
            controllable: snake.baselineBotIndex === null
          })),
        send: (_connId, message) => {
          if (message.type === 'sensors') suppressedSensors++;
        },
        nowMs: clock.nowMs,
        createResumeToken: () => 'browser-control-token'
      }
    );
    registry.setTickId(1);
    registry.assignSnake(1, 'player', external.id, 'player:browser');

    let desiredTurn = 0.75;
    let desiredBoost = 1;
    const transmitted: string[] = [];
    let newestUnconsumedDisplayFrame = Uint8Array.of(1);
    const pump = new PlayerActionPump({
      cadenceHz: 60,
      isActive: () => true,
      buildLatestAction: () => ({
        tick: 1,
        snakeId: external.id,
        turn: desiredTurn,
        boost: desiredBoost
      }),
      sendAction: action => {
        const payload = JSON.stringify({ type: 'action', ...action });
        transmitted.push(payload);
        const parsed = parseClientMessage(JSON.parse(payload) as unknown);
        if (parsed?.type === 'action') registry.handleAction(1, parsed);
      },
      clock
    });
    pump.start();
    newestUnconsumedDisplayFrame = Uint8Array.of(2);
    newestUnconsumedDisplayFrame = Uint8Array.of(3);
    await world.step(1 / 60, 800, 600, registry, 1);
    expect(external.turnInput).toBe(0.75);
    expect(external.boostInput).toBe(1);
    expect(suppressedSensors).toBeGreaterThan(0);

    registry.setTickId(2);
    desiredTurn = -0.6;
    desiredBoost = 0;
    pump.requestImmediate();
    newestUnconsumedDisplayFrame = Uint8Array.of(4);
    clock.advance(17);
    await world.step(1 / 60, 800, 600, registry, 2);

    expect(transmitted.length).toBeGreaterThanOrEqual(2);
    expect(JSON.parse(transmitted.at(-1)!)).toMatchObject({
      turn: -0.6,
      boost: 0
    });
    expect(newestUnconsumedDisplayFrame).toEqual(Uint8Array.of(4));
    expect(external.turnInput).toBeCloseTo(-0.6, 6);
    expect(external.boostInput).toBe(0);
    pump.stop();
  });

  it('keeps the normal onSensors callback free of player-action transmission', () => {
    const source = readFileSync(new URL('../src/main.ts', import.meta.url), 'utf8');
    const callback = source.match(/onSensors:\s*\(msg\)\s*=>\s*\{([\s\S]*?)\n\s*\},\n\s*onSettingsApplied:/);
    expect(callback?.[1]).not.toMatch(/sendAction|requestImmediate|sendPlayerAction/);
  });
});
