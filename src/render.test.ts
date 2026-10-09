import { describe, it, expect, beforeAll } from 'vitest';
import { drawSnakeStruct, renderWorldStruct } from './render.ts';
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
    moveTo: (x: number, y: number) => calls.push(['moveTo', x, y]),
    lineTo: (x: number, y: number) => calls.push(['lineTo', x, y]),
    arc: (...args: number[]) => calls.push(['arc', ...args]),
    fill: () => calls.push(['fill']),
    stroke: () => calls.push(['stroke']),
    fillRect: (...args: number[]) => calls.push(['fillRect', ...args]),
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

  it('keeps distant body endpoints and bounds curve error below half a screen pixel', () => {
    const zoom = 0.03;
    const points = new Float32Array(2000);
    for (let point = 0; point < 1000; point++) {
      points[point * 2] = point * 3;
      points[point * 2 + 1] = Math.sin(point / 15) * 60;
    }
    const original = points.slice();
    const ctx = makeCtx();
    drawSnakeStruct(ctx as unknown as CanvasRenderingContext2D, {
      id: 701, radius: 3, skin: 0, x: 0, y: 0, ang: 0, boost: 0, pts: points
    }, zoom);
    const path = ctx.calls.filter(call => call[0] === 'moveTo' || call[0] === 'lineTo');
    expect(path[0]?.slice(1)).toEqual([points[0], points[1]]);
    expect(path.at(-1)?.slice(1)).toEqual([...points.slice(-2)]);
    expect(path.length).toBeLessThan(500);
    let segment = 0;
    for (let index = 0; index < points.length; index += 2) {
      const x = points[index]!;
      const y = points[index + 1]!;
      while (segment + 2 < path.length && Number(path[segment + 1]![1]) < x) segment++;
      const a = path[segment]!;
      const b = path[segment + 1]!;
      const ax = Number(a[1]);
      const ay = Number(a[2]);
      const dx = Number(b[1]) - ax;
      const dy = Number(b[2]) - ay;
      const t = Math.max(0, Math.min(1, ((x - ax) * dx + (y - ay) * dy) / (dx * dx + dy * dy)));
      expect(Math.hypot(x - ax - t * dx, y - ay - t * dy) * zoom).toBeLessThanOrEqual(0.5);
    }
    expect(points).toEqual(original);
    const closeCtx = makeCtx();
    drawSnakeStruct(closeCtx as unknown as CanvasRenderingContext2D, {
      id: 701, radius: 3, skin: 0, x: 0, y: 0, ang: 0, boost: 0, pts: points
    }, 1);
    expect(closeCtx.calls.filter(call => call[0] === 'lineTo')).toHaveLength(999);
  });

  it('retains subpixel food centers, circle area and value-dependent brightness', () => {
    const buffer = new Float32Array([
      1, 0, 0, 2400, 0, 0, 1,
      2, 10, 20, 1, 0, 0, 30, 40, 4, 0, 0
    ]);
    const ctx = makeCtx();
    renderWorldStruct(ctx as unknown as CanvasRenderingContext2D, buffer, 800, 600, 0.05, 0, 0);
    // The first rectangle is the background grid; the last two are food.
    const rectangles = ctx.calls.filter(call => call[0] === 'fillRect').slice(-2);
    expect(rectangles).toHaveLength(2);
    for (const [index, radius] of [4, 6].entries()) {
      const rectangle = rectangles[index]!;
      const size = Number(rectangle[3]);
      expect(size * size).toBeCloseTo(Math.PI * radius * radius, 10);
      expect(Number(rectangle[1]) + size / 2).toBe(index === 0 ? 10 : 30);
      expect(Number(rectangle[2]) + size / 2).toBe(index === 0 ? 20 : 40);
      expect(rectangle[4]).toBe(size);
    }
    const closeCtx = makeCtx();
    renderWorldStruct(closeCtx as unknown as CanvasRenderingContext2D, buffer, 800, 600, 1, 0, 0);
    expect(closeCtx.calls.filter(call => call[0] === 'arc' && Number(call[1]) > 0)
      .map(call => call.slice(1, 4))).toEqual([[10, 20, 4], [30, 40, 6]]);
    expect(closeCtx.calls.some(call => call[0] === 'shadowBlur' && Number(call[1]) > 0)).toBe(true);
  });

  it('preserves returning tails and sharp turns across multiple drawing chunks', () => {
    const returning = makeCtx();
    drawSnakeStruct(returning as unknown as CanvasRenderingContext2D, {
      id: 702, radius: 3, skin: 0, x: 0, y: 0, ang: 0, boost: 0,
      pts: new Float32Array([0, 0, 100, 0, 10, 0])
    }, 0.03);
    expect(returning.calls.filter(call => call[0] === 'lineTo').map(call => call.slice(1)))
      .toEqual([[100, 0], [10, 0]]);
    const points = new Float32Array(400);
    for (let point = 0; point < 200; point++) {
      points[point * 2] = point * 40;
      points[point * 2 + 1] = point % 2 ? 100 : -100;
    }
    const zigzag = makeCtx();
    drawSnakeStruct(zigzag as unknown as CanvasRenderingContext2D, {
      id: 703, radius: 3, skin: 0, x: 0, y: -100, ang: 0, boost: 0, pts: points
    }, 0.03);
    const lines = zigzag.calls.filter(call => call[0] === 'lineTo');
    expect(lines).toHaveLength(199);
    expect(lines.at(-1)?.slice(1)).toEqual([...points.slice(-2)]);
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

  it('renders the first-generation world frame with snakes present', async () => {
    resetCFGToDefaults();
    const originalTarget = CFG.pelletCountTarget;
    const originalSpawn = CFG.pelletSpawnPerSecond;
    CFG.pelletCountTarget = 200;
    CFG.pelletSpawnPerSecond = 40;
    try {
      const world = new World({ snakeCount: 6, hiddenLayers: 1, neurons1: 12, neurons2: 8 });
      await world.step(1 / 60, 800, 600, undefined, 1);
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
      1, 1, 1, 2400, 0, 0, 1,
      10, 5, 2, 0, 0, 0, 0, 1, 0, 0,
      0
    ]);
    const ctx = makeCtx();
    const renderCtx = ctx as unknown as CanvasRenderingContext2D;

    // We need to import THEME to check colors, but we can check calls
    renderWorldStruct(renderCtx, buffer, 800, 600, 1, 0, 0);

    const fillCalls = ctx.calls.filter(c => c[0] === 'fillStyle');
    const shadowCalls = ctx.calls.filter(c => c[0] === 'shadowColor');

    // Verify robot skin uses eye color and glow from THEME.
    // Default values: eye '#ff0000', glow '#00ffff'
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
    // Ensure unknown skin IDs fall back to default instead of gold (#FFD700).
    expect(strokeCalls.some(c => c[1] === '#FFD700')).toBe(false);
  });
});
