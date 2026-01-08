/** Helpers to pack world state into transferable buffers. */

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
   * @param maxSnakes - Maximum snakes expected.
   * @param maxPointsPerSnake - Maximum points per snake expected.
   * @param maxPellets - Maximum pellets expected.
   */
  constructor(maxSnakes = 5000, maxPointsPerSnake = 1000, maxPellets = 50000) {
    // Estimate buffer size
    void maxSnakes;
    void maxPointsPerSnake;
    void maxPellets;
  }
  
  /**
   * Packs the world state for rendering.
   * @param world - World snapshot to serialize.
   * @returns Float32Array buffer containing the serialized frame.
   */
  static serialize(world: SerializableWorld): Float32Array {
    // 1. Calculate size
    let snakeFloats = 0;
    let aliveCount = 0;
    
    for (const s of world.snakes) {
      if (s.alive) {
        aliveCount++;
        snakeFloats += 8; // ID, Rad, Skin, X, Y, Ang, Boost, PtCount
        snakeFloats += s.points.length * 2; // px, py
      }
    }
    const pelletFloats = 1 + world.pellets.length * 5; // count + x, y, val, type, colorId
    
    // Total Bytes = (Headers + Snakes + Pellets) * 4
    // Headers: Gen(1), TotalSnakes(1), AliveCount(1), CamX(1), CamY(1), CamZoom(1) = 6 floats
    const totalBytes = (6 + snakeFloats + pelletFloats) * 4; 
    const buffer = new Float32Array(totalBytes / 4);
    let ptr = 0;
    
    // Global Header
    buffer[ptr++] = world.generation;
    buffer[ptr++] = world.snakes.length;
    buffer[ptr++] = aliveCount;
    buffer[ptr++] = world.cameraX;
    buffer[ptr++] = world.cameraY;
    buffer[ptr++] = world.zoom;
    
    // Snakes
    for (const s of world.snakes) {
      if (!s.alive) continue;
      
      // Header: 8 floats
      buffer[ptr++] = s.id;
      buffer[ptr++] = s.radius;
      // Skin flag: 0=default, 1=gold, 2=robot
      // Prefer explicit skin property, fallback to legacy color check for gold.
      const skinVal = s.skin !== undefined ? s.skin : (s.color === '#FFD700' ? 1.0 : 0.0);
      buffer[ptr++] = skinVal; 
      buffer[ptr++] = s.x;
      buffer[ptr++] = s.y;
      buffer[ptr++] = s.dir;
      buffer[ptr++] = s.boost ? 1.0 : 0.0;
      
      const pts = s.points;
      buffer[ptr++] = pts.length; // Point Count
      for (let i = 0; i < pts.length; i++) {
        const pt = pts[i];
        buffer[ptr++] = pt ? pt.x : 0;
        buffer[ptr++] = pt ? pt.y : 0;
      }
    }
    
    // Pellets
    buffer[ptr++] = world.pellets.length;
    for (let i = 0; i < world.pellets.length; i++) {
      const p = world.pellets[i];
      if (!p) {
        buffer[ptr++] = 0;
        buffer[ptr++] = 0;
        buffer[ptr++] = 0;
        buffer[ptr++] = 0;
        buffer[ptr++] = 0;
        continue;
      }
      buffer[ptr++] = p.x;
      buffer[ptr++] = p.y;
      buffer[ptr++] = p.v;
      // Type mapping
      let t = 0;
      if (p.kind === 'corpse_big') t = 1;
      else if (p.kind === 'corpse_small') t = 2;
      else if (p.kind === 'boost') t = 3;
      buffer[ptr++] = t;
      buffer[ptr++] = p.colorId || 0;
    }
    
    return buffer;
  }
}
