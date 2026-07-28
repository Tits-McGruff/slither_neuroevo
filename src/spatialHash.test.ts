import { describe, it, expect } from 'vitest';
import { FlatSpatialHash } from './spatialHash.ts';

describe('spatialHash.ts', () => {
  it('adds and queries entries in the same cell', () => {
    type SnakeLike = { id?: number | string; alive: boolean; points: Array<{ x: number; y: number }> };
    const grid = new FlatSpatialHash<SnakeLike>(100, 100, 10, 4);
    grid.reset();
    const snake: SnakeLike = { id: 1, alive: true, points: [] };
    grid.add(0, 0, snake, 2);

    const hits: Array<[SnakeLike, number]> = [];
    grid.query(0, 0, (obj, idx) => hits.push([obj, idx]));

    expect(hits).toEqual([[snake, 2]]);
  });

  it('queryCell uses raw cell coordinates', () => {
    type SnakeLike = { id?: number | string; alive: boolean; points: Array<{ x: number; y: number }> };
    const grid = new FlatSpatialHash<SnakeLike>(100, 100, 10, 4);
    grid.reset();
    const snake: SnakeLike = { id: 2, alive: true, points: [] };
    grid.add(15, 15, snake, 1); // cell (1,1)

    const hits: Array<[SnakeLike, number]> = [];
    grid.queryCell(1, 1, (obj, idx) => hits.push([obj, idx]));

    expect(hits).toEqual([[snake, 1]]);
  });

  it('ignores out-of-bounds adds and queries', () => {
    type SnakeLike = { id?: number | string; alive: boolean; points: Array<{ x: number; y: number }> };
    const grid = new FlatSpatialHash<SnakeLike>(20, 20, 10, 2);
    grid.reset();

    grid.add(1000, 0, { id: 3, alive: true, points: [] }, 0);
    expect(grid.count).toBe(0);

    let hit = false;
    grid.query(1000, 0, () => {
      hit = true;
    });
    expect(hit).toBe(false);
  });

  it('reset clears entries', () => {
    type SnakeLike = { id?: number | string; alive: boolean; points: Array<{ x: number; y: number }> };
    const grid = new FlatSpatialHash<SnakeLike>(100, 100, 10, 4);
    grid.reset();
    const snake: SnakeLike = { id: 4, alive: true, points: [] };
    grid.add(0, 0, snake, 3);

    grid.reset();
    const hits: Array<[SnakeLike, number]> = [];
    grid.query(0, 0, (obj, idx) => hits.push([obj, idx]));

    expect(hits).toEqual([]);
  });

  it('rejects an insertion that was not admitted instead of silently dropping it', () => {
    type SnakeLike = { id?: number | string; alive: boolean; points: Array<{ x: number; y: number }> };
    const grid = new FlatSpatialHash<SnakeLike>(100, 100, 10, 1);
    grid.reset();
    const snakeA: SnakeLike = { id: 'a', alive: true, points: [] };
    const snakeB: SnakeLike = { id: 'b', alive: true, points: [] };
    grid.add(0, 0, snakeA, 1);
    expect(() => grid.add(0, 0, snakeB, 2)).toThrow(/exceeded admitted capacity/);

    const hits: Array<[SnakeLike, number]> = [];
    grid.query(0, 0, (obj, idx) => hits.push([obj, idx]));

    expect(grid.count).toBe(1);
    expect(hits).toEqual([[snakeA, 1]]);
    expect(grid.getDiagnostics().faultReason).toMatch(/exceeded admitted capacity/);
  });

  it('grows before a complete build so entries beyond the old capacity remain queryable', () => {
    type SnakeLike = { alive: boolean; points: Array<{ x: number; y: number }> };
    const grid = new FlatSpatialHash<SnakeLike>(200, 200, 10, 1, 8);
    const snake: SnakeLike = {
      alive: true,
      points: [
        { x: 0, y: 0 },
        { x: 10, y: 0 },
        { x: 20, y: 0 },
        { x: 30, y: 0 }
      ]
    };

    grid.build([snake]);

    const hits: number[] = [];
    for (const x of [5, 15, 25]) grid.query(x, 0, (_object, index) => hits.push(index));
    expect(hits.sort((left, right) => left - right)).toEqual([1, 2, 3]);
    expect(grid.getDiagnostics()).toMatchObject({
      currentEntries: 3,
      peakEntries: 3,
      growths: 1,
      rebuilds: 1,
      faultReason: null
    });
  });

  it('keeps every segment when a rebuild exceeds the former 200,000-entry ceiling', () => {
    type SnakeLike = { alive: boolean; points: Array<{ x: number; y: number }> };
    const oldCapacity = 200_000;
    const point = { x: 0, y: 0 };
    const snake: SnakeLike = {
      alive: true,
      points: new Array<{ x: number; y: number }>(oldCapacity + 2).fill(point)
    };
    const grid = new FlatSpatialHash<SnakeLike>(100, 100, 10, oldCapacity, oldCapacity * 2);

    grid.build([snake]);

    let visited = 0;
    grid.query(0, 0, () => {
      visited++;
    });
    expect(grid.count).toBe(oldCapacity + 1);
    expect(visited).toBe(oldCapacity + 1);
    expect(grid.getDiagnostics()).toMatchObject({
      currentEntries: oldCapacity + 1,
      growths: 1,
      faultReason: null
    });
  });

  it('fails a rebuild before clearing the prior grid when its maximum is unsafe', () => {
    type SnakeLike = { alive: boolean; points: Array<{ x: number; y: number }> };
    const grid = new FlatSpatialHash<SnakeLike>(100, 100, 10, 1, 2);
    const prior: SnakeLike = {
      alive: true,
      points: [{ x: 0, y: 0 }, { x: 10, y: 0 }]
    };
    grid.build([prior]);
    const oversized: SnakeLike = {
      alive: true,
      points: [{ x: 0, y: 0 }, { x: 10, y: 0 }, { x: 20, y: 0 }, { x: 30, y: 0 }]
    };

    expect(() => grid.build([oversized])).toThrow(/requires 3 entries.*maximum 2/);
    const hits: number[] = [];
    grid.query(5, 0, (_object, index) => hits.push(index));
    expect(hits).toEqual([1]);
  });

  it('build populates segments for alive snakes', () => {
    type SnakeLike = { alive: boolean; points: Array<{ x: number; y: number }> };
    const grid = new FlatSpatialHash<SnakeLike>(100, 100, 10, 10);
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

    const hits: Array<[SnakeLike, number]> = [];
    grid.query(5, 0, (obj, idx) => hits.push([obj, idx]));
    expect(hits).toEqual([[alive, 1]]);
  });
});
