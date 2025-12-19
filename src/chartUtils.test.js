import { describe, it, expect } from 'vitest';
import { AdvancedCharts } from './chartUtils.js';

function makeCtx() {
  const calls = [];
  return {
    calls,
    clearRect: (...args) => calls.push(['clearRect', ...args]),
    beginPath: () => calls.push(['beginPath']),
    moveTo: (...args) => calls.push(['moveTo', ...args]),
    lineTo: (...args) => calls.push(['lineTo', ...args]),
    stroke: () => calls.push(['stroke']),
    fillRect: (...args) => calls.push(['fillRect', ...args]),
    fillText: (...args) => calls.push(['fillText', ...args]),
    set strokeStyle(value) {
      calls.push(['strokeStyle', value]);
    },
    set fillStyle(value) {
      calls.push(['fillStyle', value]);
    },
    set lineWidth(value) {
      calls.push(['lineWidth', value]);
    },
    set font(value) {
      calls.push(['font', value]);
    },
    set textAlign(value) {
      calls.push(['textAlign', value]);
    }
  };
}

describe('chartUtils.js', () => {
  it('renders average fitness without throwing', () => {
    const ctx = makeCtx();
    const history = [
      { gen: 1, avgFitness: 2, maxFitness: 3, minFitness: 1 },
      { gen: 2, avgFitness: 4, maxFitness: 6, minFitness: 2 }
    ];

    AdvancedCharts.renderAverageFitness(ctx, history, 400, 200);

    const hasTitle = ctx.calls.some(
      (call) => call[0] === 'fillText' && String(call[1]).includes('Average Fitness')
    );
    expect(hasTitle).toBe(true);
  });

  it('renders species diversity with labels', () => {
    const ctx = makeCtx();
    const history = [
      { gen: 1, speciesCount: 3, topSpeciesSize: 2 },
      { gen: 2, speciesCount: 4, topSpeciesSize: 3 }
    ];

    AdvancedCharts.renderSpeciesDiversity(ctx, history, 400, 200);

    const hasTitle = ctx.calls.some(
      (call) => call[0] === 'fillText' && String(call[1]).includes('Species Diversity')
    );
    expect(hasTitle).toBe(true);
  });

  it('renders network complexity with labels', () => {
    const ctx = makeCtx();
    const history = [
      { gen: 1, avgWeight: 0.2, weightVariance: 0.05 },
      { gen: 2, avgWeight: 0.25, weightVariance: 0.08 }
    ];

    AdvancedCharts.renderNetworkComplexity(ctx, history, 400, 200);

    const hasTitle = ctx.calls.some(
      (call) => call[0] === 'fillText' && String(call[1]).includes('Network Complexity')
    );
    expect(hasTitle).toBe(true);
  });
});
