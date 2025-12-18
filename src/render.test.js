import { describe, it, expect, beforeAll } from 'vitest';
import { renderWorldStruct } from './render.js';
import { WorldSerializer } from './serializer.js';

function makeCtx() {
  const calls = [];
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
    set shadowBlur(value) {
      calls.push(['shadowBlur', value]);
    },
    set shadowColor(value) {
      calls.push(['shadowColor', value]);
    },
    set strokeStyle(value) {
      calls.push(['strokeStyle', value]);
    },
    set fillStyle(value) {
      calls.push(['fillStyle', value]);
    },
    set lineWidth(value) {
      calls.push(['lineWidth', value]);
    }
  };
}

class StubOffscreenCanvas {
  constructor(width, height) {
    this.width = width;
    this.height = height;
  }
  getContext() {
    return {
      beginPath() {},
      moveTo() {},
      lineTo() {},
      stroke() {},
      set strokeStyle(_) {},
      set lineWidth(_) {}
    };
  }
}

describe('render.js', () => {
  beforeAll(() => {
    globalThis.OffscreenCanvas = StubOffscreenCanvas;
  });

  it('renders a serialized buffer without throwing', () => {
    const world = {
      generation: 1,
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

    renderWorldStruct(ctx, buffer, 800, 600, 1, 0, 0);

    const arcCalls = ctx.calls.filter(call => call[0] === 'arc').length;
    const lineCalls = ctx.calls.filter(call => call[0] === 'lineTo').length;
    expect(arcCalls).toBeGreaterThan(0);
    expect(lineCalls).toBeGreaterThan(0);
  });
});
