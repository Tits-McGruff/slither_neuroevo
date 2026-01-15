import { describe, it, expect } from 'vitest';
import { DenseHead, GRU, LSTM, MLP, RRU, gruParamCount, lstmParamCount, mlpParamCount, rruParamCount } from './ops.ts';
import { requireDenseKernel, requireGruKernel, requireLstmKernel, requireMlpKernel, requireRruKernel } from './wasmBridge.ts';

/** Test suite label for SIMD wasm parity. */
const SUITE = 'brains/wasmBridge parity';

/**
 * Build deterministic weights for a given length.
 * @param length - Total weight count.
 * @returns Filled weight buffer.
 */
function buildDeterministicWeights(length: number): Float32Array {
  const weights = new Float32Array(length);
  for (let i = 0; i < length; i++) {
    weights[i] = ((i % 17) - 8) * 0.02;
  }
  return weights;
}

/**
 * Build deterministic inputs for batched tests.
 * @param count - Number of batch entries.
 * @param stride - Input stride.
 * @returns Filled input buffer.
 */
function buildDeterministicInputs(count: number, stride: number): Float32Array {
  const inputs = new Float32Array(count * stride);
  for (let i = 0; i < inputs.length; i++) {
    inputs[i] = ((i % 11) - 5) * 0.1;
  }
  return inputs;
}

/**
 * Build deterministic step inputs.
 * @param steps - Number of steps.
 * @param size - Input size per step.
 * @returns Packed step inputs.
 */
function buildStepInputs(steps: number, size: number): Float32Array {
  const inputs = new Float32Array(steps * size);
  for (let i = 0; i < inputs.length; i++) {
    inputs[i] = ((i % 19) - 9) * 0.07;
  }
  return inputs;
}

/**
 * Build deterministic state buffers for batched recurrent tests.
 * @param count - Number of batch entries.
 * @param size - Hidden state size per entry.
 * @returns Filled state buffer.
 */
function buildDeterministicState(count: number, size: number): Float32Array {
  const state = new Float32Array(count * size);
  for (let i = 0; i < state.length; i++) {
    state[i] = ((i % 13) - 6) * 0.05;
  }
  return state;
}

/**
 * Expect that two buffers are close within a tolerance.
 * @param actual - Actual values.
 * @param expected - Expected values.
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
  it('Dense kernel matches JS outputs for batched inputs', () => {
    const inSize = 5;
    const outSize = 3;
    const count = 4;
    const inputStride = inSize + 2;
    const outputStride = outSize + 1;
    const weights = buildDeterministicWeights(outSize * (inSize + 1));
    const dense = new DenseHead(inSize, outSize, weights);
    const inputs = buildDeterministicInputs(count, inputStride);
    const outputs = new Float32Array(count * outputStride);

    const kernel = requireDenseKernel();
    kernel.forwardBatch(weights, inputs, outputs, inSize, outSize, count, inputStride, outputStride);

    const expected = new Float32Array(outputs.length);
    dense.forwardBatch(inputs, expected, count, inputStride, outputStride);
    expectClose(outputs, expected, 1e-4);
  });

  it('MLP kernel matches JS outputs for batched inputs', () => {
    const layerSizes = [4, 6, 3];
    const inSize = layerSizes[0]!;
    const outSize = layerSizes[layerSizes.length - 1]!;
    const count = 3;
    const inputStride = inSize + 1;
    const outputStride = outSize + 2;
    const weights = buildDeterministicWeights(mlpParamCount(layerSizes));
    const mlp = new MLP(layerSizes, weights);
    const inputs = buildDeterministicInputs(count, inputStride);
    const outputs = new Float32Array(count * outputStride);

    const kernel = requireMlpKernel();
    kernel.forwardBatch(
      weights,
      new Int32Array(layerSizes),
      inputs,
      outputs,
      count,
      inputStride,
      outputStride
    );

    const expected = new Float32Array(outputs.length);
    mlp.forwardBatch(inputs, expected, count, inputStride, outputStride);
    expectClose(outputs, expected, 1e-4);
  });

  it('GRU kernel matches JS outputs across steps', () => {
    const inSize = 4;
    const hiddenSize = 5;
    const steps = 6;
    const weights = buildDeterministicWeights(gruParamCount(inSize, hiddenSize));
    const gru = new GRU(inSize, hiddenSize, weights);
    const gruRef = new GRU(inSize, hiddenSize, weights);
    const inputs = buildStepInputs(steps, inSize);
    for (let s = 0; s < steps; s++) {
      const x = inputs.subarray(s * inSize, (s + 1) * inSize);
      const out = gru.step(x);
      const ref = gruRef.stepReference(x);
      expectClose(out, ref, 1e-4);
    }
    expectClose(gru.h, gruRef.h, 1e-4);
  });

  it('GRU kernel matches JS outputs for batched inputs', () => {
    const inSize = 4;
    const hiddenSize = 6;
    const count = 3;
    const weights = buildDeterministicWeights(gruParamCount(inSize, hiddenSize));
    const inputs = buildDeterministicInputs(count, inSize);
    const h = buildDeterministicState(count, hiddenSize);
    const hInitial = h.slice();
    const z = new Float32Array(count * hiddenSize);
    const r = new Float32Array(count * hiddenSize);
    const hPrev = new Float32Array(count * hiddenSize);

    const kernel = requireGruKernel();
    kernel.stepBatch(weights, inputs, h, z, r, hPrev, inSize, hiddenSize, count, inSize);

    const expected = new Float32Array(count * hiddenSize);
    for (let b = 0; b < count; b++) {
      const gru = new GRU(inSize, hiddenSize, weights);
      const base = b * hiddenSize;
      gru.h.set(hInitial.subarray(base, base + hiddenSize));
      const x = inputs.subarray(b * inSize, (b + 1) * inSize);
      expected.set(gru.stepReference(x), base);
    }
    expectClose(h, expected, 1e-4);
    expectClose(hPrev, hInitial, 1e-4);
  });

  it('LSTM kernel matches JS outputs across steps', () => {
    const inSize = 3;
    const hiddenSize = 4;
    const steps = 5;
    const weights = buildDeterministicWeights(lstmParamCount(inSize, hiddenSize));
    const lstm = new LSTM(inSize, hiddenSize, weights);
    const lstmRef = new LSTM(inSize, hiddenSize, weights);
    const inputs = buildStepInputs(steps, inSize);
    for (let s = 0; s < steps; s++) {
      const x = inputs.subarray(s * inSize, (s + 1) * inSize);
      const out = lstm.step(x);
      const ref = lstmRef.stepReference(x);
      expectClose(out, ref, 1e-4);
    }
    expectClose(lstm.h, lstmRef.h, 1e-4);
    expectClose(lstm.c, lstmRef.c, 1e-4);
  });

  it('LSTM kernel matches JS outputs for batched inputs', () => {
    const inSize = 3;
    const hiddenSize = 5;
    const count = 4;
    const weights = buildDeterministicWeights(lstmParamCount(inSize, hiddenSize));
    const inputs = buildDeterministicInputs(count, inSize);
    const h = buildDeterministicState(count, hiddenSize);
    const c = buildDeterministicState(count, hiddenSize);
    for (let i = 0; i < c.length; i++) c[i] = (c[i] ?? 0) + 0.02;
    const hInitial = h.slice();
    const cInitial = c.slice();
    const hPrev = new Float32Array(count * hiddenSize);
    const cPrev = new Float32Array(count * hiddenSize);

    const kernel = requireLstmKernel();
    kernel.stepBatch(weights, inputs, h, c, hPrev, cPrev, inSize, hiddenSize, count, inSize);

    const expectedH = new Float32Array(count * hiddenSize);
    const expectedC = new Float32Array(count * hiddenSize);
    for (let b = 0; b < count; b++) {
      const lstm = new LSTM(inSize, hiddenSize, weights);
      const base = b * hiddenSize;
      lstm.h.set(hInitial.subarray(base, base + hiddenSize));
      lstm.c.set(cInitial.subarray(base, base + hiddenSize));
      const x = inputs.subarray(b * inSize, (b + 1) * inSize);
      expectedH.set(lstm.stepReference(x), base);
      expectedC.set(lstm.c, base);
    }
    expectClose(h, expectedH, 1e-4);
    expectClose(c, expectedC, 1e-4);
    expectClose(hPrev, hInitial, 1e-4);
    expectClose(cPrev, cInitial, 1e-4);
  });

  it('RRU kernel matches JS outputs across steps', () => {
    const inSize = 3;
    const hiddenSize = 4;
    const steps = 7;
    const weights = buildDeterministicWeights(rruParamCount(inSize, hiddenSize));
    const rru = new RRU(inSize, hiddenSize, weights);
    const rruRef = new RRU(inSize, hiddenSize, weights);
    const inputs = buildStepInputs(steps, inSize);
    for (let s = 0; s < steps; s++) {
      const x = inputs.subarray(s * inSize, (s + 1) * inSize);
      const out = rru.step(x);
      const ref = rruRef.stepReference(x);
      expectClose(out, ref, 1e-4);
    }
    expectClose(rru.h, rruRef.h, 1e-4);
  });

  it('RRU kernel matches JS outputs for batched inputs', () => {
    const inSize = 3;
    const hiddenSize = 4;
    const count = 3;
    const weights = buildDeterministicWeights(rruParamCount(inSize, hiddenSize));
    const inputs = buildDeterministicInputs(count, inSize);
    const h = buildDeterministicState(count, hiddenSize);
    const hInitial = h.slice();
    const hPrev = new Float32Array(count * hiddenSize);

    const kernel = requireRruKernel();
    kernel.stepBatch(weights, inputs, h, hPrev, inSize, hiddenSize, count, inSize);

    const expected = new Float32Array(count * hiddenSize);
    for (let b = 0; b < count; b++) {
      const rru = new RRU(inSize, hiddenSize, weights);
      const base = b * hiddenSize;
      rru.h.set(hInitial.subarray(base, base + hiddenSize));
      const x = inputs.subarray(b * inSize, (b + 1) * inSize);
      expected.set(rru.stepReference(x), base);
    }
    expectClose(h, expected, 1e-4);
    expectClose(hPrev, hInitial, 1e-4);
  });
});
