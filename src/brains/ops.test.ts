import { describe, it, expect } from 'vitest';
import { DenseHead, MLP, mlpParamCount } from './ops.ts';

/** Test suite label for brain op batch helpers. */
const SUITE = 'brains/ops batch';

/**
 * Build deterministic weights for a given length.
 * @param length - Total weight count.
 * @returns Filled weight buffer.
 */
function buildDeterministicWeights(length: number): Float32Array {
  const weights = new Float32Array(length);
  for (let i = 0; i < length; i++) {
    weights[i] = ((i % 13) - 6) * 0.01;
  }
  return weights;
}

describe(SUITE, () => {
  it('DenseHead forwardBatch matches per-sample forward', () => {
    const inSize = 3;
    const outSize = 2;
    const outputStride = outSize + 1;
    const count = 3;
    const weights = buildDeterministicWeights(outSize * (inSize + 1));
    const dense = new DenseHead(inSize, outSize, weights);
    const inputStride = inSize + 2;
    const inputs = new Float32Array(count * inputStride);
    for (let i = 0; i < inputs.length; i++) {
      inputs[i] = ((i % 7) - 3) * 0.2;
    }
    const outputs = new Float32Array(count * outputStride);

    dense.forwardBatch(inputs, outputs, count, inputStride, outputStride);

    for (let b = 0; b < count; b++) {
      const inputBase = b * inputStride;
      const expected = Array.from(
        dense.forward(inputs.subarray(inputBase, inputBase + inSize))
      );
      const outputBase = b * outputStride;
      for (let o = 0; o < outSize; o++) {
        expect(outputs[outputBase + o]).toBeCloseTo(expected[o] ?? 0, 7);
      }
      for (let o = outSize; o < outputStride; o++) {
        expect(outputs[outputBase + o]).toBe(0);
      }
    }
  });

  it('MLP forwardBatch matches per-sample forward', () => {
    const layerSizes = [4, 5, 3];
    const inputSize = layerSizes[0]!;
    const outputSize = layerSizes[layerSizes.length - 1]!;
    const outputStride = outputSize + 2;
    const inputStride = inputSize + 1;
    const count = 2;
    const weights = buildDeterministicWeights(mlpParamCount(layerSizes));
    const mlp = new MLP(layerSizes, weights);
    const inputs = new Float32Array(count * inputStride);
    for (let i = 0; i < inputs.length; i++) {
      inputs[i] = ((i % 9) - 4) * 0.15;
    }
    const outputs = new Float32Array(count * outputStride);

    mlp.forwardBatch(inputs, outputs, count, inputStride, outputStride);

    for (let b = 0; b < count; b++) {
      const inputBase = b * inputStride;
      const expected = Array.from(
        mlp.forward(inputs.subarray(inputBase, inputBase + inputSize))
      );
      const outputBase = b * outputStride;
      for (let o = 0; o < outputSize; o++) {
        expect(outputs[outputBase + o]).toBeCloseTo(expected[o] ?? 0, 7);
      }
      for (let o = outputSize; o < outputStride; o++) {
        expect(outputs[outputBase + o]).toBe(0);
      }
    }
  });
});
