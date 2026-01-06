import { describe, it, expect } from 'vitest';
import { BrainViz } from './BrainViz.ts';
import type { VizData } from './protocol/messages.ts';

/** Recorded canvas call for asserting drawing behavior. */
type CallRecord = [string, unknown?];

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
    beginPath: () => calls.push(['beginPath']),
    moveTo: () => calls.push(['moveTo']),
    lineTo: () => calls.push(['lineTo']),
    stroke: () => calls.push(['stroke']),
    arc: () => calls.push(['arc']),
    fill: () => calls.push(['fill']),
    fillRect: () => calls.push(['fillRect']),
    set fillStyle(value: string) {
      calls.push(['fillStyle', value]);
    },
    set strokeStyle(value: string) {
      calls.push(['strokeStyle', value]);
    },
    set lineWidth(value: number) {
      calls.push(['lineWidth', value]);
    }
  };
}

describe('BrainViz.ts', () => {
  it('renders expected number of neurons for MLP', () => {
    const ctx = makeCtx();
    const vizCtx = ctx as unknown as CanvasRenderingContext2D;
    const viz = new BrainViz(0, 0, 200, 100);
    const brain: VizData = {
      kind: 'graph',
      layers: [
        { count: 2, activations: null },
        { count: 3, activations: new Float32Array([0.1, -0.2, 0.3]) },
        { count: 1, activations: new Float32Array([0.4]) }
      ]
    };

    viz.render(vizCtx, brain);

    const arcCount = ctx.calls.filter(call => call[0] === 'arc').length;
    expect(arcCount).toBe(6);
  });
});
