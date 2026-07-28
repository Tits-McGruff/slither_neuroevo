import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { CFG, resetCFGToDefaults } from './config.ts';
import type { Snake } from './snake.ts';
import { WorldSerializer } from './serializer.ts';
import { World } from './world.ts';

/**
 * Return the sorted ids killed by the current collision pass after placing two
 * heads and bodies on the same stable read state.
 * @param reverse - Whether to reverse the authoritative snake container first.
 * @returns Sorted ids marked dead by the pass.
 */
function runContestedCollision(reverse: boolean): number[] {
  const world = new World({ snakeCount: 2 }, { seed: 0xc0111de });
  for (const snake of world.snakes) {
    snake.x = 0;
    snake.y = 0;
    snake.dir = 0;
    snake.points = [
      { x: 0, y: 0 },
      { x: 0, y: 0 }
    ];
  }
  if (reverse) world.snakes.reverse();
  world._collGrid.build(world.snakes, CFG.collision.skipSegments);
  world._resolveCollisionsGrid();
  return world.snakes
    .filter(snake => !snake.alive)
    .map(snake => snake.id)
    .sort((left, right) => left - right);
}

/**
 * Check whether any two live snakes begin with overlapping body points.
 * This deliberately checks complete initial bodies rather than heads alone.
 * @param snakes - Spawned snake set.
 * @returns True when any cross-snake body points overlap their combined radii.
 */
function hasCompleteBodyOverlap(snakes: Snake[]): boolean {
  for (let leftIndex = 0; leftIndex < snakes.length; leftIndex++) {
    const left = snakes[leftIndex];
    if (!left?.alive) continue;
    for (let rightIndex = leftIndex + 1; rightIndex < snakes.length; rightIndex++) {
      const right = snakes[rightIndex];
      if (!right?.alive) continue;
      const thresholdSquared = (left.radius + right.radius) ** 2;
      for (const leftPoint of left.points) {
        for (const rightPoint of right.points) {
          const dx = leftPoint.x - rightPoint.x;
          const dy = leftPoint.y - rightPoint.y;
          if (dx * dx + dy * dy <= thresholdSquared) return true;
        }
      }
    }
  }
  return false;
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

describe('Stage 1 known-defect correction fixtures', () => {
  it.fails('COLL-002 requires simultaneous head collisions to ignore snake-array order', () => {
    const forward = runContestedCollision(false);
    const reversed = runContestedCollision(true);

    expect(forward).toEqual(reversed);
    expect(forward).toEqual([1, 2]);
  });

  it.fails('SPAWN-001 requires complete-body-safe population admission or a clear rejection', () => {
    const world = new World({ snakeCount: 64 }, { seed: 0x5afe });
    world.worldRng.asSource = () => () => 0;
    let rejected = false;
    try {
      world._spawnAll();
    } catch {
      rejected = true;
    }

    expect(rejected || !hasCompleteBodyOverlap(world.snakes)).toBe(true);
  });

  it.fails('RNG-001 requires external joins not to consume world or evolution RNG', () => {
    const world = new World({ snakeCount: 2 }, { seed: 0x51de });
    const before = {
      world: world.worldRng.exportState(),
      evolution: world.evolutionRng.exportState()
    };

    world.spawnExternalSnake();

    expect({
      world: world.worldRng.exportState(),
      evolution: world.evolutionRng.exportState()
    }).toEqual(before);
  });

  it.fails('FRAME-002 requires frame-v1 ids to remain distinct or reject an unsafe range', () => {
    let rejected = false;
    let firstId: number | undefined;
    let secondId: number | undefined;
    try {
      const frame = WorldSerializer.serialize({
        generation: 1,
        worldRadius: 2200,
        cameraX: 0,
        cameraY: 0,
        zoom: 1,
        snakes: [
          {
            id: 16_777_216,
            radius: 10,
            x: 0,
            y: 0,
            dir: 0,
            boost: 0,
            alive: true,
            points: []
          },
          {
            id: 16_777_217,
            radius: 10,
            x: 1,
            y: 1,
            dir: 0,
            boost: 0,
            alive: true,
            points: []
          }
        ],
        pellets: []
      });
      firstId = frame[7];
      secondId = frame[15];
    } catch {
      rejected = true;
    }

    expect(rejected || firstId !== secondId).toBe(true);
  });
});
