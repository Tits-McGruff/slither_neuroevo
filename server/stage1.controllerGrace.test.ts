import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { CFG, resetCFGToDefaults } from '../src/config.ts';
import { World } from '../src/world.ts';
import { ControllerRegistry, type ControllerType } from './controllerRegistry.ts';

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

describe('Stage 1 wall-time controller grace', () => {
  for (const controllerType of ['player', 'bot'] as const satisfies readonly ControllerType[]) {
    it(`keeps a disconnected ${controllerType} neural-free through grace then takes over once`, async () => {
      let now = 100;
      let tokenCounter = 0;
      const world = new World({ snakeCount: 1 }, { seed: 9100 + controllerType.length });
      world.snakes[0]!.alive = false;
      const external = world.spawnExternalSnake();
      external.x = 0;
      external.y = 0;
      external.points.forEach((point, index) => {
        point.x = -index * CFG.snakeSpacing;
        point.y = 0;
      });
      const brain = vi.fn(() => Float32Array.of(0.5, 1));
      external.brain.forward = brain;
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
          send: () => undefined,
          nowMs: () => now,
          createResumeToken: () => `grace-${++tokenCounter}`,
          getLeaseScope: () => 'stage1:test'
        }
      );
      registry.setTickId(1);
      registry.assignSnake(1, controllerType, external.id, `${controllerType}:owner`);
      registry.handleAction(1, {
        type: 'action',
        tick: 1,
        snakeId: external.id,
        turn: -0.8,
        boost: 1
      });
      registry.disconnectConnection(1);

      await world.step(1 / 60, 800, 600, registry, 1);
      expect(brain).not.toHaveBeenCalled();
      expect(external.turnInput).toBe(0);
      expect(external.boostInput).toBe(0);

      now += 29_999;
      registry.setTickId(2);
      registry.refresh();
      await world.step(1 / 60, 800, 600, registry, 2);
      expect(brain).not.toHaveBeenCalled();

      now += 1;
      registry.setTickId(3);
      registry.refresh();
      await world.step(1 / 60, 800, 600, registry, 3);
      expect(brain).toHaveBeenCalledTimes(1);
      expect(external.turnInput).toBe(0.5);
      expect(registry.isControlled(external.id)).toBe(false);

      registry.setTickId(4);
      await world.step(1 / 60, 800, 600, registry, 4);
      expect(registry.reclaimSnake(2, controllerType, 'grace-1').reason).toBe('expired');
    });
  }
});
