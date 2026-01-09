/**
 * Float offsets for the serialized frame header. 
 * These indices reference the starting positions in a Float32Array.
 */
export const FRAME_HEADER_OFFSETS = {
  /** Current simulation generation index. */
  generation: 0,
  /** Historical total count of snakes since simulation start. */
  totalSnakes: 1,
  /** Number of currently active (alive) snakes in the world. */
  aliveCount: 2,
  /** Absolute world radius, used by the renderer for arena boundary drawing. */
  worldRadius: 3,
  /** Normalized or camera-centered X coordinate. */
  cameraX: 4,
  /** Normalized or camera-centered Y coordinate. */
  cameraY: 5,
  /** Current viewport zoom scale (1.0 = 1:1). */
  zoom: 6
} as const;

/** 
 * Total number of float entries reserved for the frame header. 
 * Serializers and parsers must skip this many elements to reach snake data.
 */
export const FRAME_HEADER_FLOATS = 7;

/** 
 * Parsed header values for a serialized frame buffer. 
 * This structure represents the global state of a single simulation frame.
 */
export interface FrameHeader {
  /** Current simulation generation. */
  generation: number;
  /** Total snakes across history. */
  totalSnakes: number;
  /** Count of snakes active in this frame. */
  aliveCount: number;
  /** Radius of the circular world arena. */
  worldRadius: number;
  /** Camera focus X position. */
  cameraX: number;
  /** Camera focus Y position. */
  cameraY: number;
  /** Camera zoom level. */
  zoom: number;
}

/**
 * Read a frame header from a Float32Array buffer.
 * @param buffer - Serialized frame buffer.
 * @param offset - Starting float offset.
 * @returns Parsed frame header.
 */
export function readFrameHeader(buffer: Float32Array, offset = 0): FrameHeader {
  return {
    generation: buffer[offset + FRAME_HEADER_OFFSETS.generation] ?? 0,
    totalSnakes: buffer[offset + FRAME_HEADER_OFFSETS.totalSnakes] ?? 0,
    aliveCount: buffer[offset + FRAME_HEADER_OFFSETS.aliveCount] ?? 0,
    worldRadius: buffer[offset + FRAME_HEADER_OFFSETS.worldRadius] ?? 0,
    cameraX: buffer[offset + FRAME_HEADER_OFFSETS.cameraX] ?? 0,
    cameraY: buffer[offset + FRAME_HEADER_OFFSETS.cameraY] ?? 0,
    zoom: buffer[offset + FRAME_HEADER_OFFSETS.zoom] ?? 0
  };
}
