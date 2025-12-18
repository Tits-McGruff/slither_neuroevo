import { describe, it, expect } from 'vitest';
import { BrainViz } from './BrainViz.js';

function makeCtx() {
  const calls = [];
  return {
    calls,
    save: () => calls.push(['save']),
    restore: () => calls.push(['restore']),
    translate: () => calls.push(['translate']),
    beginPath: () => calls.push(['beginPath']),
    moveTo: () => calls.push(['moveTo']),
    lineTo: () => calls.push(['lineTo']),
    stroke: () => calls.push(['stroke']),
    arc: () => calls.push(['arc']),
    fill: () => calls.push(['fill']),
    set fillStyle(value) {
      calls.push(['fillStyle', value]);
    },
    set strokeStyle(value) {
      calls.push(['strokeStyle', value]);
    },
    set lineWidth(value) {
      calls.push(['lineWidth', value]);
    }
  };
}

describe('BrainViz.js', () => {
  it('renders expected number of neurons for MLP', () => {
    const ctx = makeCtx();
    const viz = new BrainViz(0, 0, 200, 100);
    const brain = {
      kind: 'mlp',
      mlp: {
        layerSizes: [2, 3, 1],
        _bufs: [new Float32Array([0.1, -0.2, 0.3]), new Float32Array([0.4])]
      }
    };

    viz.render(ctx, brain);

    const arcCount = ctx.calls.filter(call => call[0] === 'arc').length;
    expect(arcCount).toBe(6);
  });
});
