import { describe, it, expect } from 'vitest';
import { FitnessChart } from './FitnessChart.js';

function makeCtx() {
  const calls = [];
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
    fillText: (...args) => calls.push(['fillText', ...args]),
    set fillStyle(value) {
      calls.push(['fillStyle', value]);
    },
    set strokeStyle(value) {
      calls.push(['strokeStyle', value]);
    },
    set lineWidth(value) {
      calls.push(['lineWidth', value]);
    },
    set font(value) {
      calls.push(['font', value]);
    }
  };
}

describe('FitnessChart.js', () => {
  it('renders lines and labels for history', () => {
    const ctx = makeCtx();
    const chart = new FitnessChart(0, 0, 200, 100);
    const history = [
      { gen: 1, best: 4, avg: 2 },
      { gen: 2, best: 6, avg: 3 }
    ];

    chart.render(ctx, history);

    const hasMaxLabel = ctx.calls.some(
      call => call[0] === 'fillText' && String(call[1]).startsWith('Max:')
    );
    expect(hasMaxLabel).toBe(true);
  });
});
