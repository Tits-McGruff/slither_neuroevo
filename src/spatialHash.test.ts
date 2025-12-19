import { describe, it, expect } from 'vitest';
import { FlatSpatialHash } from './spatialHash.ts';

describe('spatialHash.ts', () => {
  it('adds and queries entries in the same cell', () => {
    const grid = new FlatSpatialHash(100, 100, 10, 4);
    grid.reset();
    const snake = { id: 1 };
    grid.add(0, 0, snake, 2);

    const hits: Array<[unknown, number]> = [];
    grid.query(0, 0, (obj, idx) => hits.push([obj, idx]));

    expect(hits).toEqual([[snake, 2]]);
  });

  it('queryCell uses raw cell coordinates', () => {
    const grid = new FlatSpatialHash(100, 100, 10, 4);
    grid.reset();
    const snake = { id: 2 };
    grid.add(15, 15, snake, 1); // cell (1,1)

    const hits: Array<[unknown, number]> = [];
    grid.queryCell(1, 1, (obj, idx) => hits.push([obj, idx]));

    expect(hits).toEqual([[snake, 1]]);
  });

  it('ignores out-of-bounds adds and queries', () => {
    const grid = new FlatSpatialHash(20, 20, 10, 2);
    grid.reset();

    grid.add(1000, 0, { id: 3 }, 0);
    expect(grid.count).toBe(0);

    let hit = false;
    grid.query(1000, 0, () => {
      hit = true;
    });
    expect(hit).toBe(false);
  });

  it('reset clears entries', () => {
    const grid = new FlatSpatialHash(100, 100, 10, 4);
    grid.reset();
    const snake = { id: 4 };
    grid.add(0, 0, snake, 3);

    grid.reset();
    const hits: Array<[unknown, number]> = [];
    grid.query(0, 0, (obj, idx) => hits.push([obj, idx]));

    expect(hits).toEqual([]);
  });

  it('caps inserts at capacity', () => {
    const grid = new FlatSpatialHash(100, 100, 10, 1);
    grid.reset();
    const snakeA = { id: 'a' };
    const snakeB = { id: 'b' };
    grid.add(0, 0, snakeA, 1);
    grid.add(0, 0, snakeB, 2);

    const hits: Array<[unknown, number]> = [];
    grid.query(0, 0, (obj, idx) => hits.push([obj, idx]));

    expect(grid.count).toBe(1);
    expect(hits).toEqual([[snakeA, 1]]);
  });

  it('build populates segments for alive snakes', () => {
    const grid = new FlatSpatialHash(100, 100, 10, 10);
    const alive = {
      alive: true,
      points: [
        { x: 0, y: 0 },
        { x: 10, y: 0 },
        { x: 20, y: 0 }
      ]
    };
    const dead = {
      alive: false,
      points: [
        { x: 0, y: 0 },
        { x: 0, y: 10 }
      ]
    };

    grid.build([alive, dead], 0);

    const hits: Array<[unknown, number]> = [];
    grid.query(5, 0, (obj, idx) => hits.push([obj, idx]));
    expect(hits).toEqual([[alive, 1]]);
  });
});
