/** Float offsets for the serialized frame header. */
export const FRAME_HEADER_OFFSETS = {
  generation: 0,
  totalSnakes: 1,
  aliveCount: 2,
  cameraX: 3,
  cameraY: 4,
  zoom: 5
} as const;

/** Number of float entries in the frame header. */
export const FRAME_HEADER_FLOATS = 6;

/** Parsed header values for a serialized frame buffer. */
export interface FrameHeader {
  generation: number;
  totalSnakes: number;
  aliveCount: number;
  cameraX: number;
  cameraY: number;
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
    cameraX: buffer[offset + FRAME_HEADER_OFFSETS.cameraX] ?? 0,
    cameraY: buffer[offset + FRAME_HEADER_OFFSETS.cameraY] ?? 0,
    zoom: buffer[offset + FRAME_HEADER_OFFSETS.zoom] ?? 0
  };
}
