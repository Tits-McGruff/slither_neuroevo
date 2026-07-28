import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { CFG, resetCFGToDefaults } from './config.ts';
import type { Snake } from './snake.ts';
import { World } from './world.ts';

/**
 * Place one snake's head and two-point body.
 * @param snake - Snake to position.
 * @param headX - Head X coordinate.
 * @param tailX - Tail X coordinate.
 */
function placeLine(snake: Snake, headX: number, tailX: number): void {
  snake.x = headX;
  snake.y = 0;
  snake.points = [
    { x: headX, y: 0 },
    { x: tailX, y: 0 }
  ];
  snake.killScore = 0;
  snake.pointsScore = 0;
}

/**
 * Rebuild and resolve the current mutable TypeScript collision pass.
 * @param world - Fixture world.
 */
function resolve(world: World): void {
  world._collGrid.build(world.snakes, CFG.collision.skipSegments);
  world._resolveCollisionsGrid();
}

beforeEach(() => {
  resetCFGToDefaults();
  CFG.baselineBots.count = 0;
  CFG.pelletCountTarget = 0;
  CFG.pelletSpawnPerSecond = 0;
});

afterEach(() => {
  resetCFGToDefaults();
});

describe('Stage 2 current kill-credit characterization', () => {
  it('currently credits the live body owner for an unambiguous head-to-body death', () => {
    const world = new World({ snakeCount: 2 }, { seed: 0xc4ed17 });
    const victim = world.snakes[0]!;
    const owner = world.snakes[1]!;
    placeLine(victim, 0, -5);
    placeLine(owner, 100, 0);

    resolve(world);

    expect(victim.alive).toBe(false);
    expect(owner.alive).toBe(true);
    expect(owner.killScore).toBe(1);
    expect(owner.pointsScore).toBe(CFG.reward.pointsPerKill);
  });

  it('currently resolves simultaneous head overlap by array order and credits the survivor', () => {
    const run = (reverse: boolean) => {
      const world = new World({ snakeCount: 2 }, { seed: 0xc4ed17 });
      for (const snake of world.snakes) placeLine(snake, 0, 0);
      if (reverse) world.snakes.reverse();
      resolve(world);
      return world.snakes
        .map(snake => ({
          id: snake.id,
          alive: snake.alive,
          killScore: snake.killScore
        }))
        .sort((left, right) => left.id - right.id);
    };

    expect(run(false)).toEqual([
      { id: 1, alive: false, killScore: 0 },
      { id: 2, alive: true, killScore: 1 }
    ]);
    expect(run(true)).toEqual([
      { id: 1, alive: true, killScore: 1 },
      { id: 2, alive: false, killScore: 0 }
    ]);
  });

  it('currently ignores an already-dead body owner and awards no kill', () => {
    const world = new World({ snakeCount: 2 }, { seed: 0xc4ed17 });
    const candidate = world.snakes[0]!;
    const deadOwner = world.snakes[1]!;
    placeLine(candidate, 0, -5);
    placeLine(deadOwner, 100, 0);
    deadOwner.alive = false;

    resolve(world);

    expect(candidate.alive).toBe(true);
    expect(deadOwner.killScore).toBe(0);
  });

  it('currently gives an exact multi-body tie to grid insertion order', () => {
    const run = (swapOwners: boolean) => {
      const world = new World({ snakeCount: 3 }, { seed: 0xc4ed17 });
      const victim = world.snakes[0]!;
      const firstOwner = world.snakes[1]!;
      const secondOwner = world.snakes[2]!;
      placeLine(victim, 0, -5);
      placeLine(firstOwner, 100, 110);
      firstOwner.points = [{ x: 0, y: -1 }, { x: 0, y: 1 }];
      placeLine(secondOwner, -100, -110);
      secondOwner.points = [{ x: 0, y: -1 }, { x: 0, y: 1 }];
      if (swapOwners) world.snakes.splice(1, 2, secondOwner, firstOwner);
      resolve(world);
      return [firstOwner, secondOwner]
        .map(snake => ({ id: snake.id, killScore: snake.killScore }))
        .sort((left, right) => left.id - right.id);
    };

    expect(run(false)).toEqual([
      { id: 2, killScore: 0 },
      { id: 3, killScore: 1 }
    ]);
    expect(run(true)).toEqual([
      { id: 2, killScore: 1 },
      { id: 3, killScore: 0 }
    ]);
  });
});
