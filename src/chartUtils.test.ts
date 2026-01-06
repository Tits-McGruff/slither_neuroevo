import { describe, it, expect } from 'vitest';
import { AdvancedCharts } from './chartUtils.ts';

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
    clearRect: (...args: unknown[]) => calls.push(['clearRect', ...args]),
    beginPath: () => calls.push(['beginPath']),
    moveTo: (...args: unknown[]) => calls.push(['moveTo', ...args]),
    lineTo: (...args: unknown[]) => calls.push(['lineTo', ...args]),
    stroke: () => calls.push(['stroke']),
    fillRect: (...args: unknown[]) => calls.push(['fillRect', ...args]),
    fillText: (...args: unknown[]) => calls.push(['fillText', ...args]),
    set strokeStyle(value: string) {
      calls.push(['strokeStyle', value]);
    },
    set fillStyle(value: string) {
      calls.push(['fillStyle', value]);
    },
    set lineWidth(value: number) {
      calls.push(['lineWidth', value]);
    },
    set font(value: string) {
      calls.push(['font', value]);
    },
    set textAlign(value: string) {
      calls.push(['textAlign', value]);
    }
  };
}

describe('chartUtils.ts', () => {
  it('renders average fitness without throwing', () => {
    const ctx = makeCtx();
    const chartCtx = ctx as unknown as CanvasRenderingContext2D;
    const history = [
      { gen: 1, avgFitness: 2, maxFitness: 3, minFitness: 1 },
      { gen: 2, avgFitness: 4, maxFitness: 6, minFitness: 2 }
    ];

    AdvancedCharts.renderAverageFitness(chartCtx, history, 400, 200);

    const hasTitle = ctx.calls.some(
      (call) => call[0] === 'fillText' && String(call[1]).includes('Average Fitness')
    );
    expect(hasTitle).toBe(true);
  });

  it('renders species diversity with labels', () => {
    const ctx = makeCtx();
    const chartCtx = ctx as unknown as CanvasRenderingContext2D;
    const history = [
      { gen: 1, speciesCount: 3, topSpeciesSize: 2 },
      { gen: 2, speciesCount: 4, topSpeciesSize: 3 }
    ];

    AdvancedCharts.renderSpeciesDiversity(chartCtx, history, 400, 200);

    const hasTitle = ctx.calls.some(
      (call) => call[0] === 'fillText' && String(call[1]).includes('Species Diversity')
    );
    expect(hasTitle).toBe(true);
  });

  it('renders network complexity with labels', () => {
    const ctx = makeCtx();
    const chartCtx = ctx as unknown as CanvasRenderingContext2D;
    const history = [
      { gen: 1, avgWeight: 0.2, weightVariance: 0.05 },
      { gen: 2, avgWeight: 0.25, weightVariance: 0.08 }
    ];

    AdvancedCharts.renderNetworkComplexity(chartCtx, history, 400, 200);

    const hasTitle = ctx.calls.some(
      (call) => call[0] === 'fillText' && String(call[1]).includes('Network Complexity')
    );
    expect(hasTitle).toBe(true);
  });
});
