/** Helpers to pack world state into transferable buffers. */

import { FRAME_HEADER_FLOATS } from './protocol/frame.ts';

/** Minimal point shape used in serialized snakes. */
interface SerializablePoint {
  x: number;
  y: number;
}

/** Minimal snake shape for serialization. */
interface SerializableSnake {
  id: number;
  radius: number;
  skin?: number;
  color?: string;
  x: number;
  y: number;
  dir: number;
  boost: number;
  alive: boolean;
  points: SerializablePoint[];
}

/** Minimal pellet shape for serialization. */
interface SerializablePellet {
  x: number;
  y: number;
  v: number;
  kind: string;
  colorId?: number;
}

/** Minimal world shape for serialization. */
interface SerializableWorld {
  generation: number;
  worldRadius: number;
  cameraX: number;
  cameraY: number;
  zoom: number;
  snakes: SerializableSnake[];
  pellets: SerializablePellet[];
}

/** Serializer for packing world state into a Float32Array. */
export class WorldSerializer {
  /**
   * Create a serializer with optional sizing hints.
   * Currently, these hints are reserved for future buffer pooling optimizations 
   * to reduce Garbage Collection (GC) pressure in high-throughput simulation loops.
   */
  constructor(maxSnakes = 5000, maxPointsPerSnake = 1000, maxPellets = 50000) {
    void maxSnakes;
    void maxPointsPerSnake;
    void maxPellets;
  }

  /**
   * Packs the complete world state into a high-performance binary Float32Array.
   * 
   * Buffer Layout Contract v7:
   * 1. Global Header (7 floats):
   *    [gen, totalSnakes, aliveCount, worldRadius, cameraX, cameraY, zoom]
   * 2. Snake Block (Variable length):
   *    For each alive snake: 
   *    [id, radius, skin, x, y, dir, boost, ptCount] (8 floats)
   *    followed by [x, y] * ptCount.
   * 3. Pellet Block (Variable length):
   *    [pelletCount] (1 float)
   *    followed by [x, y, value, type, colorId] * pelletCount (5 floats per pellet).
   * 
   * @param world - World snapshot to serialize.
   * @returns Float32Array buffer ready for transfer to main thread or persistence.
   */
  static serialize(world: SerializableWorld): Float32Array {
    const SNAKE_HEADER_SIZE = 8;
    const PELLET_BLOCK_SIZE = 5;

    // Phase 1: Pre-calculate the total buffer size to perform a single allocation.
    let snakeFloats = 0;
    let aliveCount = 0;

    for (const s of world.snakes) {
      if (s.alive) {
        aliveCount++;
        // Header + 2 floats per point (X,Y).
        snakeFloats += SNAKE_HEADER_SIZE + s.points.length * 2;
      }
    }
    const pelletFloats = 1 + world.pellets.length * PELLET_BLOCK_SIZE;

    const totalFloats = FRAME_HEADER_FLOATS + snakeFloats + pelletFloats;
    const buffer = new Float32Array(totalFloats);
    let ptr = 0;

    // Phase 2: Write Global Header (7 floats total)
    // [gen, totalSnakes, aliveCount, worldRadius, cameraX, cameraY, zoom]
    buffer[ptr++] = world.generation;
    buffer[ptr++] = world.snakes.length;
    buffer[ptr++] = aliveCount;
    // worldRadius is serialized to allow the fast-path renderer to draw 
    // arena boundaries without a direct reference to the world state.
    buffer[ptr++] = world.worldRadius;
    buffer[ptr++] = world.cameraX;
    buffer[ptr++] = world.cameraY;
    buffer[ptr++] = world.zoom;

    // Phase 3: Write Snake Data
    for (const s of world.snakes) {
      if (!s.alive) continue;

      // Snake Header (8 floats): [id, radius, skin, x, y, dir, boost, ptCount]
      buffer[ptr++] = s.id;
      buffer[ptr++] = s.radius;

      // Skin Logic:
      // Binary protocol uses a float ID for skin: 0=Default, 1=Gold (Legacy), 2=Robot.
      // We prioritize the modern 'skin' property but maintain backward compatibility 
      // with the legacy hex color check for 'Gold'.
      const skinVal = s.skin !== undefined ? s.skin : (s.color === '#FFD700' ? 1.0 : 0.0);
      buffer[ptr++] = skinVal;

      buffer[ptr++] = s.x;
      buffer[ptr++] = s.y;
      buffer[ptr++] = s.dir;
      buffer[ptr++] = s.boost ? 1.0 : 0.0;

      const pts = s.points;
      buffer[ptr++] = pts.length;
      for (let i = 0; i < pts.length; i++) {
        const pt = pts[i];
        buffer[ptr++] = pt ? pt.x : 0;
        buffer[ptr++] = pt ? pt.y : 0;
      }
    }

    // Phase 4: Write Pellet Data
    buffer[ptr++] = world.pellets.length;
    for (let i = 0; i < world.pellets.length; i++) {
      const p = world.pellets[i];
      if (!p) {
        // Null/undefined entries can occur in the pellet grid due to concurrent 
        // deletions or lazy cell cleanup in the worker thread. 
        // We skip them while maintaining pointer alignment for the reader.
        ptr += PELLET_BLOCK_SIZE;
        continue;
      }
      buffer[ptr++] = p.x;
      buffer[ptr++] = p.y;
      buffer[ptr++] = p.v;

      // Pellet Type Mapping:
      // 0=Ambient, 1=Corpse (Big), 2=Corpse (Small), 3=Boost.
      let typeId = 0;
      if (p.kind === 'corpse_big') typeId = 1;
      else if (p.kind === 'corpse_small') typeId = 2;
      else if (p.kind === 'boost') typeId = 3;

      buffer[ptr++] = typeId;
      buffer[ptr++] = p.colorId || 0;
    }

    return buffer;
  }
}
