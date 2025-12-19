import { describe, it, expect } from 'vitest';
import { WorldSerializer } from './serializer.js';

describe('serializer.js', () => {
  it('serializes header, snakes, and pellets into a flat buffer', () => {
    const world = {
      generation: 3,
      cameraX: 12,
      cameraY: -8,
      zoom: 0.75,
      snakes: [
        {
          id: 1,
          radius: 5,
          color: '#33aaff',
          x: 10,
          y: 20,
          dir: 0.5,
          boost: 1,
          alive: true,
          points: [{ x: 10, y: 20 }, { x: 5, y: 20 }]
        },
        {
          id: 2,
          radius: 7,
          color: '#FFD700',
          x: 30,
          y: 40,
          dir: -1,
          boost: 0,
          alive: false,
          points: [{ x: 30, y: 40 }]
        },
        {
          id: 3,
          radius: 6,
          color: '#FFD700',
          x: -5,
          y: 15,
          dir: 1.2,
          boost: 0,
          alive: true,
          points: [{ x: -5, y: 15 }]
        }
      ],
      pellets: [
        { x: 1, y: 2, v: 1.5, kind: 'ambient' },
        { x: -3, y: 4, v: 2.0, kind: 'boost' }
      ]
    };

    const buf = WorldSerializer.serialize(world);

    expect(buf[0]).toBe(3);
    expect(buf[1]).toBe(3);
    expect(buf[2]).toBe(2);
    expect(buf[3]).toBe(12);
    expect(buf[4]).toBe(-8);
    expect(buf[5]).toBe(0.75);

    let ptr = 6;
    const aliveSnake1 = world.snakes[0];
    expect(buf[ptr++]).toBe(aliveSnake1.id);
    expect(buf[ptr++]).toBe(aliveSnake1.radius);
    expect(buf[ptr++]).toBe(0);
    expect(buf[ptr++]).toBe(aliveSnake1.x);
    expect(buf[ptr++]).toBe(aliveSnake1.y);
    expect(buf[ptr++]).toBe(aliveSnake1.dir);
    expect(buf[ptr++]).toBe(1);
    expect(buf[ptr++]).toBe(aliveSnake1.points.length);
    expect(buf[ptr++]).toBe(aliveSnake1.points[0].x);
    expect(buf[ptr++]).toBe(aliveSnake1.points[0].y);
    expect(buf[ptr++]).toBe(aliveSnake1.points[1].x);
    expect(buf[ptr++]).toBe(aliveSnake1.points[1].y);

    const aliveSnake2 = world.snakes[2];
    expect(buf[ptr++]).toBe(aliveSnake2.id);
    expect(buf[ptr++]).toBe(aliveSnake2.radius);
    expect(buf[ptr++]).toBe(1);
    expect(buf[ptr++]).toBe(aliveSnake2.x);
    expect(buf[ptr++]).toBe(aliveSnake2.y);
    expect(buf[ptr++]).toBeCloseTo(aliveSnake2.dir, 5);
    expect(buf[ptr++]).toBe(0);
    expect(buf[ptr++]).toBe(aliveSnake2.points.length);
    expect(buf[ptr++]).toBe(aliveSnake2.points[0].x);
    expect(buf[ptr++]).toBe(aliveSnake2.points[0].y);

    const pelletCount = buf[ptr++];
    expect(pelletCount).toBe(2);
    expect(buf[ptr++]).toBe(1);
    expect(buf[ptr++]).toBe(2);
    expect(buf[ptr++]).toBe(1.5);
    expect(buf[ptr++]).toBe(0);
    expect(buf[ptr++]).toBe(0);
    expect(buf[ptr++]).toBe(-3);
    expect(buf[ptr++]).toBe(4);
    expect(buf[ptr++]).toBe(2);
    expect(buf[ptr++]).toBe(3);
    expect(buf[ptr++]).toBe(0);
  });

  it('serializes pellet colorId when provided', () => {
    const world = {
      generation: 1,
      cameraX: 0,
      cameraY: 0,
      zoom: 1,
      snakes: [],
      pellets: [
        { x: 5, y: 6, v: 2, kind: 'corpse_big', colorId: 42 }
      ]
    };

    const buf = WorldSerializer.serialize(world);
    let ptr = 6;
    expect(buf[ptr++]).toBe(1); // pellet count
    expect(buf[ptr++]).toBe(5);
    expect(buf[ptr++]).toBe(6);
    expect(buf[ptr++]).toBe(2);
    expect(buf[ptr++]).toBe(1);
    expect(buf[ptr++]).toBe(42);
  });
});
