/** Pure player-action and packed-frame presentation projections. */
import type { VizData } from './protocol/messages.ts';
import { FRAME_HEADER_FLOATS, FRAME_HEADER_OFFSETS } from './protocol/frame.ts';


/** Selected snake snapshot used by God Mode interactions. */
export interface SelectedSnake {
  /** Native physical identity, absent on an unresolved frame snapshot. */
  token?: string;
  id: number;
  x: number;
  y: number;
  radius: number;
  skin: number;
}


/**
 * Normalize viz payloads that arrive from JSON by reconstructing activation arrays.
 * @param viz - Incoming visualization payload.
 * @returns Normalized visualization payload or null.
 */
export function normalizeVizData(viz: VizData | null | undefined): VizData | null {
  if (!viz) return null;
  const layers = viz.layers.map(layer => {
    const activations = layer.activations;
    if (!activations) return layer;
    if (Array.isArray(activations) || ArrayBuffer.isView(activations)) return layer;
    const count = Math.max(0, layer.count);
    const next = new Array<number>(count);
    const source = activations as unknown as Record<string, number>;
    for (let i = 0; i < count; i += 1) {
      const raw = source[i] ?? 0;
      next[i] = Number.isFinite(raw) ? raw : 0;
    }
    return { ...layer, activations: next };
  });
  return { ...viz, layers };
}


/**
 * Compute a normalized turn input toward a target position.
 * @param meta - Current snake position and direction.
 * @param target - Target world position.
 * @returns Turn input in [-1,1].
 */
export function computeTurnInput(
  meta: { x: number; y: number; dir: number },
  target: { x: number; y: number }
): number {
  const dx = target.x - meta.x;
  const dy = target.y - meta.y;
  if (dx === 0 && dy === 0) return 0;
  const desired = Math.atan2(dy, dx);
  const delta = Math.atan2(Math.sin(desired - meta.dir), Math.cos(desired - meta.dir));
  const scaled = delta / (Math.PI / 2);
  return Math.max(-1, Math.min(1, scaled));
}


/** Minimal snapshot of a snake parsed from the frame buffer. */
export type FrameSnakeSnapshot = { id: number; x: number; y: number; ptCount: number };


/**
 * Find a snake in the frame buffer by id or return the first alive snake.
 * @param buffer - Frame buffer to scan.
 * @param targetId - Optional target snake id.
 * @returns Snapshot of the snake or null when none found.
 */
export function findSnakeInFrame(buffer: Float32Array, targetId: number | null): FrameSnakeSnapshot | null {
  if (buffer.length < FRAME_HEADER_FLOATS) return null;
  const aliveCount = (buffer[FRAME_HEADER_OFFSETS.aliveCount] ?? 0) | 0;
  let ptr = FRAME_HEADER_FLOATS;
  let first: FrameSnakeSnapshot | null = null;
  for (let i = 0; i < aliveCount; i++) {
    if (ptr + 7 >= buffer.length) break;
    const id = (buffer[ptr] ?? 0) | 0;
    const x = buffer[ptr + 3] ?? 0;
    const y = buffer[ptr + 4] ?? 0;
    const ptCount = (buffer[ptr + 7] ?? 0) | 0;
    const info = { id, x, y, ptCount };
    if (!first) first = info;
    if (targetId != null && id === targetId) return info;
    ptr += 8 + ptCount * 2;
  }
  return first;
}


/** Scan compact frame headers without constructing snake/body snapshots. */
export function frameContainsSnake(currentFrameBuffer: Float32Array | null, targetId: number): boolean {
  const buffer = currentFrameBuffer;
  if (!buffer || buffer.length < FRAME_HEADER_FLOATS) return false;
  const count = buffer[FRAME_HEADER_OFFSETS.aliveCount] ?? 0;
  let cursor = FRAME_HEADER_FLOATS;
  for (let index = 0; index < count; index++) {
    if (cursor + 8 > buffer.length) return false;
    const points = buffer[cursor + 7]!;
    const next = cursor + 8 + 2 * points;
    if (!Number.isSafeInteger(points) || points < 0 || next > buffer.length) return false;
    if (buffer[cursor] === targetId) return true;
    cursor = next;
  }
  return false;
}


/**
 * Identifies the snake closest to a specific world coordinate within a search radius.
 * 
 * Scanning Algorithm:
 * Because the binary frame buffer uses variable-length snake blocks (due to body points), 
 * we must perform a linear scan. We skip over body data by reading the `ptCount` for each 
 * snake to jump to the next block until we find the best match.
 * 
 * @param worldX - Focus X in simulation units.
 * @param worldY - Focus Y in simulation units.
 * @param maxDist - Maximum distance threshold for selection.
 * @returns A snapshot of the closest snake's metadata or null if none found.
 */
export function findSnakeNear(currentFrameBuffer: Float32Array | null, worldX: number, worldY: number, maxDist = 50): SelectedSnake | null {
  if (!currentFrameBuffer || currentFrameBuffer.length < FRAME_HEADER_FLOATS) return null;

  const buffer = currentFrameBuffer;
  const read = (idx: number): number => buffer[idx] ?? 0;

  // Retrieve count of snakes from the header to bound the search.
  const aliveCount = read(FRAME_HEADER_OFFSETS.aliveCount) | 0;

  let closestSnake = null;
  let closestDist = maxDist;
  let ptr = FRAME_HEADER_FLOATS;

  const SNAKE_STATEDATA_FLOATS = 8; // ID, Rad, Skin, X, Y, Ang, Boost, PtCount
  const PT_COUNT_OFFSET = 7;
  const RADIUS_OFFSET = 1;
  const SKIN_OFFSET = 2;
  const X_OFFSET = 3;
  const Y_OFFSET = 4;

  for (let i = 0; i < aliveCount; i++) {
    const ptCount = read(ptr + PT_COUNT_OFFSET) | 0;

    // Selection Jump Logic:
    // Because snake body point counts vary, we calculate the total byte size of the snake block 
    // [Header (8) + (ptCount * 2)] to jump the pointer precisely to the next entity.
    const blockSize = SNAKE_STATEDATA_FLOATS + ptCount * 2;

    if (ptr + blockSize > buffer.length) break;

    const id = read(ptr);
    const radius = read(ptr + RADIUS_OFFSET);
    const skin = read(ptr + SKIN_OFFSET);
    const x = read(ptr + X_OFFSET);
    const y = read(ptr + Y_OFFSET);

    const dist = Math.hypot(x - worldX, y - worldY);
    // Selection threshold includes the snake's actual physical radius to make 
    // clicking on large snakes easier.
    if (dist < closestDist && dist < radius + maxDist) {
      closestDist = dist;
      closestSnake = { id, x, y, radius, skin };
    }

    ptr += blockSize;
  }

  return closestSnake;
}
