import { describe, it, expect } from 'vitest';
import { FitnessChart } from './FitnessChart.ts';

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
    fillRect: () => calls.push(['fillRect']),
    beginPath: () => calls.push(['beginPath']),
    moveTo: () => calls.push(['moveTo']),
    lineTo: () => calls.push(['lineTo']),
    stroke: () => calls.push(['stroke']),
    fillText: (...args: unknown[]) => calls.push(['fillText', ...args]),
    set fillStyle(value: string) {
      calls.push(['fillStyle', value]);
    },
    set strokeStyle(value: string) {
      calls.push(['strokeStyle', value]);
    },
    set lineWidth(value: number) {
      calls.push(['lineWidth', value]);
    },
    set font(value: string) {
      calls.push(['font', value]);
    }
  };
}

describe('FitnessChart.ts', () => {
  it('renders lines and labels for history', () => {
    const ctx = makeCtx();
    const chartCtx = ctx as unknown as CanvasRenderingContext2D;
    const chart = new FitnessChart(0, 0, 200, 100);
    const history = [
      { gen: 1, best: 4, avg: 2 },
      { gen: 2, best: 6, avg: 3 }
    ];

    chart.render(chartCtx, history);

    const hasMaxLabel = ctx.calls.some(
      call => call[0] === 'fillText' && String(call[1]).startsWith('Max:')
    );
    expect(hasMaxLabel).toBe(true);
  });
});
