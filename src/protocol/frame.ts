export const FRAME_HEADER_OFFSETS = {
  generation: 0,
  totalSnakes: 1,
  aliveCount: 2,
  cameraX: 3,
  cameraY: 4,
  zoom: 5
} as const;

export const FRAME_HEADER_FLOATS = 6;

export interface FrameHeader {
  generation: number;
  totalSnakes: number;
  aliveCount: number;
  cameraX: number;
  cameraY: number;
  zoom: number;
}

export function readFrameHeader(buffer: Float32Array, offset = 0): FrameHeader {
  return {
    generation: buffer[offset + FRAME_HEADER_OFFSETS.generation],
    totalSnakes: buffer[offset + FRAME_HEADER_OFFSETS.totalSnakes],
    aliveCount: buffer[offset + FRAME_HEADER_OFFSETS.aliveCount],
    cameraX: buffer[offset + FRAME_HEADER_OFFSETS.cameraX],
    cameraY: buffer[offset + FRAME_HEADER_OFFSETS.cameraY],
    zoom: buffer[offset + FRAME_HEADER_OFFSETS.zoom]
  };
}
