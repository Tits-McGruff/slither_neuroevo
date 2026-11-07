/** Local presentation-camera operations; the root remains the only camera/session state owner. */
import type { CFG_DEFAULT } from './config.ts';
import { FRAME_HEADER_OFFSETS } from './protocol/frame.ts';
import { clamp, lerp } from './utils.ts';
import { findSnakeInFrame, type SelectedSnake } from './player_input_and_frames.ts';

/** Only the presentation settings used by the existing camera equations. */
type CameraSettings = Pick<typeof CFG_DEFAULT, 'worldRadius' | 'snakeMaxLen'> & {
  observer: Pick<typeof CFG_DEFAULT.observer, 'overviewExtraWorldMargin' | 'overviewPadding' | 'snapZoomOutInOverview' | 'zoomLerpOverview' | 'zoomLerpFollow'>;
};
/** Live access to the existing presentation state, with no frame cloning or new scheduler. */
export interface CameraPort {
  readonly proxyWorld: { cameraX: number; cameraY: number; zoom: number; viewMode: string };
  clientCamX: number;
  clientCamY: number;
  clientZoom: number;
  manualCamera: boolean;
  cameraPan: { button: number; startX: number; startY: number; lastX: number; lastY: number; moved: boolean; } | null;
  suppressCanvasClick: boolean;
  readonly canvas: HTMLCanvasElement;
  readonly isPlayerControlActive: () => boolean;
  readonly currentFrameBuffer: Float32Array<ArrayBufferLike> | null;
  readonly cssW: number;
  readonly cssH: number;
  readonly selectedSnake: SelectedSnake | null;
  readonly playerSnakeId: number | null;
  readonly spectatorFollowSnakeId: number | null;
  readonly frameContainsSnake: (targetId: number) => boolean;
  readonly settings: CameraSettings;
  readonly isServerMode: boolean;
  readonly followIndividual: boolean;
  readonly followedPopulationSlot: () => number | null;
  readonly sendOverview: () => void;
}


/** Publish the same local camera used for drawing, steering, and God Mode selection. */
export function publishClientCamera(ports: CameraPort): void {
  ports.proxyWorld.cameraX = ports.clientCamX;
  ports.proxyWorld.cameraY = ports.clientCamY;
  ports.proxyWorld.zoom = ports.clientZoom;
}


/** Restore automatic camera movement and cancel any unfinished camera drag. */
export function resetManualCamera(ports: CameraPort): void {
  ports.manualCamera = false;
  ports.cameraPan = null;
  ports.suppressCanvasClick = false;
  ports.canvas.style.cursor = ports.isPlayerControlActive() ? '' : 'grab';
}


/** Fit the authoritative arena and presentation margins into the current viewport. */
export function arenaFitZoom(ports: CameraPort): number {
  const radius = ports.currentFrameBuffer?.[FRAME_HEADER_OFFSETS.worldRadius] ?? ports.settings.worldRadius;
  const effectiveRadius = radius + ports.settings.observer.overviewExtraWorldMargin;
  return clamp(Math.min(ports.cssW, ports.cssH) / (2 * effectiveRadius * ports.settings.observer.overviewPadding), 0.01, 2);
}


/** Return immediately to a centered overview without changing simulation state. */
export function fitArenaView(ports: CameraPort): void {
  resetManualCamera(ports);
  ports.proxyWorld.viewMode = 'overview';
  ports.clientCamX = 0;
  ports.clientCamY = 0;
  ports.clientZoom = arenaFitZoom(ports);
  publishClientCamera(ports);
  ports.sendOverview();
}


/** Update the automatic camera unless the user has taken control of the local view. */
export function updateClientCamera(ports: CameraPort): void {
  if (!ports.isServerMode) return;
  const frame = ports.currentFrameBuffer;
  if (!frame) return;
  if (ports.manualCamera) {
    publishClientCamera(ports);
    return;
  }
  const mode = ports.proxyWorld.viewMode === 'follow' ? 'follow' : 'overview';
  if (mode === 'overview') {
    ports.clientCamX = 0;
    ports.clientCamY = 0;
    const targetZoom = arenaFitZoom(ports);
    if (ports.settings.observer.snapZoomOutInOverview && ports.clientZoom > targetZoom) {
      ports.clientZoom = targetZoom;
    } else {
      ports.clientZoom = lerp(ports.clientZoom, targetZoom, ports.settings.observer.zoomLerpOverview);
    }
    publishClientCamera(ports);
    return;
  }

  const waitingForIndividual = !ports.isPlayerControlActive() && (ports.followIndividual)
    && ports.followedPopulationSlot() !== null;
  if (waitingForIndividual && !ports.selectedSnake) { publishClientCamera(ports); return; }
  const targetId = ports.isPlayerControlActive() ? ports.playerSnakeId : ports.selectedSnake?.id ?? ports.spectatorFollowSnakeId;
  const focus = targetId !== null && !ports.frameContainsSnake(targetId) ? null : findSnakeInFrame(frame, targetId);
  if (!focus && waitingForIndividual) { publishClientCamera(ports); return; }
  if (focus) {
    ports.clientCamX = focus.x;
    ports.clientCamY = focus.y;
    const denom = Math.max(1, ports.settings.snakeMaxLen);
    const targetZoom = clamp(1.15 - (focus.ptCount / denom) * 0.55, 0.45, 1.12);
    ports.clientZoom = lerp(ports.clientZoom, targetZoom, ports.settings.observer.zoomLerpFollow);
  } else {
    ports.clientCamX = 0;
    ports.clientCamY = 0;
    ports.clientZoom = lerp(ports.clientZoom, 0.95, 0.05);
  }
  publishClientCamera(ports);
}


// ============== GOD MODE: Canvas Event Handlers ==============

/**
 * Converts screen-space coordinates (CSS pixels) into simulation world coordinates.
 * 
 * Transformation Logic:
 * 1. Offset by viewport center: Map (0,0) at top-left to centered origin.
 * 2. Scale by inverse zoom: Convert CSS pixels to world units.
 * 3. Offset by camera position: Map centered local view to absolute world space.
 * 
 * @param screenX - Screen X in CSS pixels (relative to canvas).
 * @param screenY - Screen Y in CSS pixels (relative to canvas).
 * @returns Object containing absolute simulation world coordinates \{x, y\}.
 */
export function screenToWorld(screenX: number, screenY: number, ports: CameraPort): { x: number; y: number } {
  // Use the local presentation camera, including manual zoom and panning.
  let camX = 0, camY = 0, zoom = 1;
  if (ports.isServerMode) {
    camX = ports.clientCamX;
    camY = ports.clientCamY;
    zoom = ports.clientZoom;
  }

  const centerX = ports.cssW / 2;
  const centerY = ports.cssH / 2;

  // Inverse Transformation Formula: World = Camera + (Screen - Center) / Zoom
  // This formula reverses the rendering transform to map a mouse click (screen space)
  // back into simulation world coordinates.
  const worldX = camX + (screenX - centerX) / zoom;
  const worldY = camY + (screenY - centerY) / zoom;
  return { x: worldX, y: worldY };
}


/** Pan by CSS-pixel deltas, keeping the grabbed point fixed even outside the canvas. */
export function moveCameraPan(e: MouseEvent, ports: CameraPort): boolean {
  const pan = ports.cameraPan;
  if (!pan) return false;
  if (!pan.moved && Math.hypot(e.clientX - pan.startX, e.clientY - pan.startY) < 3) return true;
  pan.moved = true;
  ports.manualCamera = true;
  ports.clientCamX -= (e.clientX - pan.lastX) / ports.clientZoom;
  ports.clientCamY -= (e.clientY - pan.lastY) / ports.clientZoom;
  pan.lastX = e.clientX;
  pan.lastY = e.clientY;
  ports.canvas.style.cursor = 'grabbing';
  publishClientCamera(ports);
  return true;
}


/** Finish a camera drag at its release point without selecting or moving a snake. */
export function finishCameraPan(e: MouseEvent, ports: CameraPort): boolean {
  if (!ports.cameraPan || e.button !== ports.cameraPan.button) return false;
  moveCameraPan(e, ports);
  ports.suppressCanvasClick = ports.cameraPan.moved;
  ports.cameraPan = null;
  ports.canvas.style.cursor = ports.isPlayerControlActive() ? '' : 'grab';
  return true;
}
