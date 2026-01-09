import { describe, it, expect, beforeAll } from 'vitest';
import { renderWorldStruct } from './render.ts';
import { WorldSerializer } from './serializer.ts';
import { World } from './world.ts';
import { CFG, resetCFGToDefaults } from './config.ts';

/** Recorded canvas call for asserting drawing behavior. */
type CallRecord = [string, ...unknown[]];

/**
 * Creates a fake canvas context that logs drawing calls.
 * @returns Canvas context shim with call recording.
 */
function makeCtx() {
  const calls: CallRecord[] = [];
  return {
    calls,
    save: () => calls.push(['save']),
    restore: () => calls.push(['restore']),
    translate: () => calls.push(['translate']),
    scale: () => calls.push(['scale']),
    beginPath: () => calls.push(['beginPath']),
    moveTo: () => calls.push(['moveTo']),
    lineTo: () => calls.push(['lineTo']),
    arc: () => calls.push(['arc']),
    fill: () => calls.push(['fill']),
    stroke: () => calls.push(['stroke']),
    fillRect: () => calls.push(['fillRect']),
    clearRect: () => calls.push(['clearRect']),
    getTransform: () => ({ a: 1 }),
    createPattern: () => ({}),
    setTransform: () => calls.push(['setTransform']),
    set shadowBlur(value: number) {
      calls.push(['shadowBlur', value]);
    },
    set shadowColor(value: string) {
      calls.push(['shadowColor', value]);
    },
    set strokeStyle(value: string) {
      calls.push(['strokeStyle', value]);
    },
    set fillStyle(value: string) {
      calls.push(['fillStyle', value]);
    },
    set lineWidth(value: number) {
      calls.push(['lineWidth', value]);
    }
  };
}

/** OffscreenCanvas stub to satisfy grid caching in render tests. */
class StubOffscreenCanvas {
  /** Canvas width in pixels. */
  width: number;
  /** Canvas height in pixels. */
  height: number;

  constructor(width: number, height: number) {
    this.width = width;
    this.height = height;
  }
  /** Return a minimal 2D context stub. */
  getContext(): CanvasRenderingContext2D {
    return {
      beginPath() { },
      moveTo() { },
      lineTo() { },
      stroke() { },
      set strokeStyle(_: string) { },
      set lineWidth(_: number) { }
    } as unknown as CanvasRenderingContext2D;
  }
}

describe('render.ts', () => {
  beforeAll(() => {
    const globalShim = globalThis as unknown as { OffscreenCanvas?: typeof StubOffscreenCanvas };
    globalShim.OffscreenCanvas = StubOffscreenCanvas;
  });

  it('renders a serialized buffer without throwing', () => {
    const world: Parameters<typeof WorldSerializer.serialize>[0] = {
      generation: 1,
      worldRadius: 2400,
      cameraX: 0,
      cameraY: 0,
      zoom: 1,
      snakes: [
        {
          id: 1,
          radius: 5,
          color: '#fff',
          x: 0,
          y: 0,
          dir: 0,
          boost: 0,
          alive: true,
          points: [{ x: 0, y: 0 }, { x: 5, y: 0 }]
        }
      ],
      pellets: [{ x: 10, y: 0, v: 1, kind: 'ambient' }]
    };

    const buffer = WorldSerializer.serialize(world);
    const ctx = makeCtx();
    const renderCtx = ctx as unknown as CanvasRenderingContext2D;

    renderWorldStruct(renderCtx, buffer, 800, 600, 1, 0, 0);

    const arcCalls = ctx.calls.filter(call => call[0] === 'arc').length;
    const lineCalls = ctx.calls.filter(call => call[0] === 'lineTo').length;
    expect(arcCalls).toBeGreaterThan(0);
    expect(lineCalls).toBeGreaterThan(0);
  });

  it('renders the first-generation world frame with snakes present', () => {
    resetCFGToDefaults();
    const originalTarget = CFG.pelletCountTarget;
    const originalSpawn = CFG.pelletSpawnPerSecond;
    CFG.pelletCountTarget = 200;
    CFG.pelletSpawnPerSecond = 40;
    try {
      const world = new World({ snakeCount: 6, hiddenLayers: 1, neurons1: 12, neurons2: 8 });
      world.update(1 / 30, 800, 600);
      const buffer = WorldSerializer.serialize(world);
      const ctx = makeCtx();
      const renderCtx = ctx as unknown as CanvasRenderingContext2D;

      renderWorldStruct(renderCtx, buffer, 800, 600, 1, 0, 0);

      const lineCalls = ctx.calls.filter(call => call[0] === 'lineTo').length;
      expect(buffer[2]).toBeGreaterThan(0); // aliveCount
      expect(lineCalls).toBeGreaterThan(0);
    } finally {
      CFG.pelletCountTarget = originalTarget;
      CFG.pelletSpawnPerSecond = originalSpawn;
      resetCFGToDefaults();
    }
  });
  it('renders robot skin with correct colors', () => {
    const buffer = new Float32Array([
      1, 1, 1, 2400, 0, 0, 1, // Header: Gen, Total, Alive, Radius, CamX, CamY, Zoom
      10, 5, 2, 0, 0, 0, 0, 1, 0, 0, // Snake: ID 10, Skin 2
      0 // Pellets
    ]);
    const ctx = makeCtx();
    const renderCtx = ctx as unknown as CanvasRenderingContext2D;

    // We need to import THEME to check colors, but we can check calls
    renderWorldStruct(renderCtx, buffer, 800, 600, 1, 0, 0);

    const fillCalls = ctx.calls.filter(c => c[0] === 'fillStyle');
    const shadowCalls = ctx.calls.filter(c => c[0] === 'shadowColor');

    // Check for robot eye color (red) and glow (cyan)
    // Values from theme.ts: eye '#ff0000', glow '#00ffff'
    expect(fillCalls.some(c => c[1] === '#ff0000')).toBe(true);
    expect(shadowCalls.some(c => c[1] === '#00ffff')).toBe(true);
  });

  it('renders unknown skin as default (not gold)', () => {
    const buffer = new Float32Array([
      1, 1, 1, 2400, 0, 0, 1,
      11, 5, 99, 0, 0, 0, 0, 1, 0, 0, // Snake: ID 11, Skin 99 (unknown)
      0
    ]);
    const ctx = makeCtx();
    const renderCtx = ctx as unknown as CanvasRenderingContext2D;

    renderWorldStruct(renderCtx, buffer, 800, 600, 1, 0, 0);

    const strokeCalls = ctx.calls.filter(c => c[0] === 'strokeStyle');
    // Should NOT be gold (#FFD700)
    expect(strokeCalls.some(c => c[1] === '#FFD700')).toBe(false);
  });
});
