// serializer.js
// Helpers to pack World state into Transferable ArrayBuffers for the Worker.

export class WorldSerializer {
  constructor(maxSnakes = 5000, maxPointsPerSnake = 1000, maxPellets = 50000) {
    // Estimate buffer size
  }
  
  /**
   * Packs the world state for rendering.
   * @param {World} world 
   * @returns {Float32Array}
   */
  static serialize(world) {
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
    const pelletFloats = 1 + world.pellets.length * 4; // count + x, y, val, type
    
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
      // Color
      buffer[ptr++] = s.color === '#FFD700' ? 1.0 : 0.0; 
      buffer[ptr++] = s.x;
      buffer[ptr++] = s.y;
      buffer[ptr++] = s.dir;
      buffer[ptr++] = s.boost ? 1.0 : 0.0;
      
      const pts = s.points;
      buffer[ptr++] = pts.length; // Point Count
      for (let i = 0; i < pts.length; i++) {
        buffer[ptr++] = pts[i].x;
        buffer[ptr++] = pts[i].y;
      }
    }
    
    // Pellets
    buffer[ptr++] = world.pellets.length;
    for (let i = 0; i < world.pellets.length; i++) {
      const p = world.pellets[i];
      buffer[ptr++] = p.x;
      buffer[ptr++] = p.y;
      buffer[ptr++] = p.v;
      // Type mapping
      let t = 0;
      if (p.kind === 'corpse_big') t = 1;
      else if (p.kind === 'corpse_small') t = 2;
      else if (p.kind === 'boost') t = 3;
      buffer[ptr++] = t;
    }
    
    return buffer;
  }
}
