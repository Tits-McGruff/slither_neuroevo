import { describe, it, expect } from 'vitest';
import { performance } from 'node:perf_hooks';
import { World } from '../src/world.ts';
import { WorldSerializer } from '../src/serializer.ts';
import { CFG, resetCFGToDefaults, syncBrainInputSize } from '../src/config.ts';
import { mlpParamCount } from '../src/brains/ops.ts';
import { requireDenseKernel, requireMlpKernel } from '../src/brains/wasmBridge.ts';

/** Test suite label for server performance checks. */
const SUITE = 'performance: world tick + serialize';

/**
 * Build deterministic weights for SIMD perf tests.
 * @param length - Total weight count.
 * @returns Filled weight buffer.
 */
function buildPerfWeights(length: number): Float32Array {
  const weights = new Float32Array(length);
  for (let i = 0; i < length; i++) {
    weights[i] = ((i % 23) - 11) * 0.01;
  }
  return weights;
}

/**
 * Build deterministic inputs for SIMD perf tests.
 * @param length - Total input count.
 * @returns Filled input buffer.
 */
function buildPerfInputs(length: number): Float32Array {
  const inputs = new Float32Array(length);
  for (let i = 0; i < length; i++) {
    inputs[i] = ((i % 17) - 8) * 0.05;
  }
  return inputs;
}

/**
 * Sum absolute values in a buffer.
 * @param buffer - Buffer to scan.
 * @returns Sum of absolute values.
 */
function sumAbs(buffer: Float32Array): number {
  let total = 0;
  for (let i = 0; i < buffer.length; i++) {
    total += Math.abs(buffer[i] ?? 0);
  }
  return total;
}

describe(SUITE, () => {
  it('ticks 60 frames under a reasonable budget', () => {
    resetCFGToDefaults();
    const originalBaselineBots = CFG.baselineBots.count;
    CFG.baselineBots.count = 0;
    syncBrainInputSize();
    try {
      const world = new World({ snakeCount: 20, simSpeed: 1 });
      const frames = 60;
      const start = performance.now();
      for (let i = 0; i < frames; i++) {
        world.update(1 / 60, 800, 600);
        WorldSerializer.serialize(world);
      }
      const elapsed = performance.now() - start;
      const msPerFrame = elapsed / frames;
      // Generous budget to avoid CI flakiness.
      expect(msPerFrame).toBeLessThan(40);
    } finally {
      CFG.baselineBots.count = originalBaselineBots;
      resetCFGToDefaults();
    }
  });

  it('runs SIMD dense + MLP batches under a reasonable budget', () => {
    const denseKernel = requireDenseKernel();
    const mlpKernel = requireMlpKernel();

    const denseIn = 64;
    const denseOut = 32;
    const denseCount = 256;
    const denseWeights = buildPerfWeights(denseOut * (denseIn + 1));
    const denseInputs = buildPerfInputs(denseCount * denseIn);
    const denseOutputs = new Float32Array(denseCount * denseOut);

    const denseStart = performance.now();
    for (let i = 0; i < 10; i++) {
      denseKernel.forwardBatch(
        denseWeights,
        denseInputs,
        denseOutputs,
        denseIn,
        denseOut,
        denseCount,
        denseIn,
        denseOut
      );
    }
    const denseElapsed = performance.now() - denseStart;
    expect(sumAbs(denseOutputs)).toBeGreaterThan(0);
    expect(denseElapsed).toBeLessThan(200);

    const mlpLayers = [64, 48, 16];
    const mlpCount = 256;
    const mlpWeights = buildPerfWeights(mlpParamCount(mlpLayers));
    const mlpInputs = buildPerfInputs(mlpCount * mlpLayers[0]!);
    const mlpOutputs = new Float32Array(mlpCount * mlpLayers[2]!);

    const mlpStart = performance.now();
    for (let i = 0; i < 6; i++) {
      mlpKernel.forwardBatch(
        mlpWeights,
        new Int32Array(mlpLayers),
        mlpInputs,
        mlpOutputs,
        mlpCount,
        mlpLayers[0]!,
        mlpLayers[2]!
      );
    }
    const mlpElapsed = performance.now() - mlpStart;
    expect(sumAbs(mlpOutputs)).toBeGreaterThan(0);
    expect(mlpElapsed).toBeLessThan(200);
  });
});
