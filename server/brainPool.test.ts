import { describe, it, expect, afterEach } from 'vitest';
import { compileGraph, graphKey } from '../src/brains/graph/compiler.ts';
import { GraphBrain } from '../src/brains/graph/runtime.ts';
import type { GraphSpec } from '../src/brains/graph/schema.ts';
import { BrainPool } from './brainPool.ts';

/** Test suite label for server brain pool coverage. */
const SUITE = 'server/brainPool';

/** Active pool used by the test to ensure cleanup. */
let activePool: BrainPool | null = null;

afterEach(async () => {
  if (activePool) {
    await activePool.shutdown();
    activePool = null;
  }
});

/**
 * Build a deterministic weight buffer for a population.
 * @param populationCount - Number of brains to pack.
 * @param paramCount - Parameters per brain.
 * @returns Packed weight buffer.
 */
function buildPopulationWeights(populationCount: number, paramCount: number): Float32Array {
  const weights = new Float32Array(populationCount * paramCount);
  for (let i = 0; i < populationCount; i++) {
    const base = i * paramCount;
    for (let j = 0; j < paramCount; j++) {
      weights[base + j] = ((i + j) % 17 - 8) * 0.02;
    }
  }
  return weights;
}

/**
 * Build deterministic inputs for a batch.
 * @param count - Batch entry count.
 * @param stride - Input stride per entry.
 * @returns Packed input buffer.
 */
function buildBatchInputs(count: number, stride: number): Float32Array {
  const inputs = new Float32Array(count * stride);
  for (let i = 0; i < inputs.length; i++) {
    inputs[i] = ((i % 9) - 4) * 0.11;
  }
  return inputs;
}

/**
 * Expect output buffers to match within tolerance.
 * @param actual - Actual output buffer.
 * @param expected - Expected output buffer.
 * @param tol - Absolute tolerance.
 */
function expectClose(actual: Float32Array, expected: Float32Array, tol: number): void {
  expect(actual.length).toBe(expected.length);
  for (let i = 0; i < actual.length; i++) {
    const a = actual[i] ?? 0;
    const b = expected[i] ?? 0;
    const diff = Math.abs(a - b);
    if (diff > tol) {
      throw new Error(`mismatch at ${i}: ${a} vs ${b} (diff ${diff})`);
    }
  }
}

describe(SUITE, () => {
  it('runs batched inference with stable ordering', async () => {
    const spec: GraphSpec = {
      type: 'graph',
      nodes: [
        { id: 'input', type: 'Input', outputSize: 3 },
        { id: 'dense', type: 'Dense', inputSize: 3, outputSize: 2 }
      ],
      edges: [{ from: 'input', to: 'dense' }],
      outputs: [{ nodeId: 'dense' }],
      outputSize: 2
    };
    const compiled = compileGraph(spec);
    const populationCount = 4;
    const paramCount = compiled.totalParams;
    const weights = buildPopulationWeights(populationCount, paramCount);

    const pool = new BrainPool(2);
    activePool = pool;
    await pool.init({
      spec,
      specKey: graphKey(spec),
      populationCount,
      paramCount,
      inputStride: 3,
      outputStride: 2,
      maxBatch: populationCount,
      weights
    });

    const indices = new Uint32Array([2, 0, 3, 1]);
    const inputs = buildBatchInputs(indices.length, 3);
    const outputs = new Float32Array(indices.length * 2);

    await pool.runBatch(inputs, outputs, indices, indices.length, 3, 2);

    const expected = new Float32Array(outputs.length);
    for (let b = 0; b < indices.length; b++) {
      const snakeIndex = indices[b] ?? 0;
      const offset = snakeIndex * paramCount;
      const weightSlice = weights.subarray(offset, offset + paramCount);
      const brain = new GraphBrain(compiled, weightSlice);
      const inputSlice = inputs.subarray(b * 3, b * 3 + 3);
      const out = brain.forward(inputSlice);
      expected.set(out, b * 2);
    }

    expectClose(outputs, expected, 1e-4);
  });
});
