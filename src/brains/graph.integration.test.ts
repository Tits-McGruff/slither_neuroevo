import { describe, it, expect } from 'vitest';
import { compileGraph } from './graph/compiler.ts';
import { GraphBrain } from './graph/runtime.ts';
import type { GraphSpec } from './graph/schema.ts';
import { DenseHead, GRU, LSTM, MLP, RRU } from './ops.ts';

/** Test suite label for graph brain integration. */
const SUITE = 'graph brain (integration)';

/**
 * Build deterministic weights for parity tests.
 * @param length - Weight buffer length to allocate.
 * @returns Filled weight buffer.
 */
function buildDeterministicWeights(length: number): Float32Array {
  const weights = new Float32Array(length);
  for (let i = 0; i < length; i++) {
    weights[i] = ((i % 19) - 9) * 0.03;
  }
  return weights;
}

/**
 * Expect buffers to be close within tolerance.
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
  const spec: GraphSpec = {
    type: 'graph',
    nodes: [
      { id: 'input', type: 'Input', outputSize: 3 },
      { id: 'mlp', type: 'MLP', inputSize: 3, outputSize: 2, hiddenSizes: [4] },
      { id: 'head', type: 'Dense', inputSize: 2, outputSize: 2 }
    ],
    edges: [
      { from: 'input', to: 'mlp' },
      { from: 'mlp', to: 'head' }
    ],
    outputs: [{ nodeId: 'head' }],
    outputSize: 2
  };

  it('runs a simple graph forward pass', () => {
    const compiled = compileGraph(spec);
    const weights = new Float32Array(compiled.totalParams).fill(0.01);
    const brain = new GraphBrain(compiled, weights);
    const out = brain.forward(new Float32Array([0.2, -0.1, 0.3]));
    expect(out.length).toBe(2);
  });

  it('matches JS outputs for a simple graph', () => {
    const compiled = compileGraph(spec);
    const weights = buildDeterministicWeights(compiled.totalParams);
    const brain = new GraphBrain(compiled, weights);
    const input = new Float32Array([0.25, -0.15, 0.35]);
    const out = brain.forward(input);

    const mlpNode = compiled.nodes.find((node) => node.type === 'MLP');
    const headNode = compiled.nodes.find((node) => node.type === 'Dense');
    if (!mlpNode || !headNode) {
      throw new Error('missing MLP or Dense node');
    }
    const mlpWeights = weights.subarray(mlpNode.paramOffset, mlpNode.paramOffset + mlpNode.paramLength);
    const headWeights = weights.subarray(headNode.paramOffset, headNode.paramOffset + headNode.paramLength);
    const mlpSizes = [mlpNode.inputSize, ...(mlpNode.hiddenSizes ?? []), mlpNode.outputSize];
    const mlp = new MLP(mlpSizes, mlpWeights);
    const head = new DenseHead(headNode.inputSize, headNode.outputSize, headWeights);
    const ref = head.forward(mlp.forward(input));

    expectClose(out, ref, 1e-4);
  });

  it('matches JS outputs for a GRU graph', () => {
    const recurrentSpec: GraphSpec = {
      type: 'graph',
      nodes: [
        { id: 'input', type: 'Input', outputSize: 3 },
        { id: 'gru', type: 'GRU', inputSize: 3, hiddenSize: 4 }
      ],
      edges: [{ from: 'input', to: 'gru' }],
      outputs: [{ nodeId: 'gru' }],
      outputSize: 4
    };
    const compiled = compileGraph(recurrentSpec);
    const weights = buildDeterministicWeights(compiled.totalParams);
    const brain = new GraphBrain(compiled, weights);
    const input = new Float32Array([0.1, -0.2, 0.3]);
    const out = brain.forward(input);

    const gruNode = compiled.nodes.find((node) => node.type === 'GRU');
    if (!gruNode) {
      throw new Error('missing GRU node');
    }
    const gruWeights = weights.subarray(gruNode.paramOffset, gruNode.paramOffset + gruNode.paramLength);
    const gru = new GRU(gruNode.inputSize, gruNode.hiddenSize ?? gruNode.outputSize, gruWeights);
    const ref = gru.stepReference(input);

    expectClose(out, ref, 1e-4);
  });

  it('matches JS outputs for an LSTM graph', () => {
    const recurrentSpec: GraphSpec = {
      type: 'graph',
      nodes: [
        { id: 'input', type: 'Input', outputSize: 3 },
        { id: 'lstm', type: 'LSTM', inputSize: 3, hiddenSize: 4 }
      ],
      edges: [{ from: 'input', to: 'lstm' }],
      outputs: [{ nodeId: 'lstm' }],
      outputSize: 4
    };
    const compiled = compileGraph(recurrentSpec);
    const weights = buildDeterministicWeights(compiled.totalParams);
    const brain = new GraphBrain(compiled, weights);
    const input = new Float32Array([0.12, -0.18, 0.28]);
    const out = brain.forward(input);

    const lstmNode = compiled.nodes.find((node) => node.type === 'LSTM');
    if (!lstmNode) {
      throw new Error('missing LSTM node');
    }
    const lstmWeights = weights.subarray(lstmNode.paramOffset, lstmNode.paramOffset + lstmNode.paramLength);
    const lstm = new LSTM(lstmNode.inputSize, lstmNode.hiddenSize ?? lstmNode.outputSize, lstmWeights);
    const ref = lstm.stepReference(input);

    expectClose(out, ref, 1e-4);
  });

  it('matches JS outputs for an RRU graph', () => {
    const recurrentSpec: GraphSpec = {
      type: 'graph',
      nodes: [
        { id: 'input', type: 'Input', outputSize: 3 },
        { id: 'rru', type: 'RRU', inputSize: 3, hiddenSize: 4 }
      ],
      edges: [{ from: 'input', to: 'rru' }],
      outputs: [{ nodeId: 'rru' }],
      outputSize: 4
    };
    const compiled = compileGraph(recurrentSpec);
    const weights = buildDeterministicWeights(compiled.totalParams);
    const brain = new GraphBrain(compiled, weights);
    const input = new Float32Array([0.09, -0.21, 0.31]);
    const out = brain.forward(input);

    const rruNode = compiled.nodes.find((node) => node.type === 'RRU');
    if (!rruNode) {
      throw new Error('missing RRU node');
    }
    const rruWeights = weights.subarray(rruNode.paramOffset, rruNode.paramOffset + rruNode.paramLength);
    const rru = new RRU(rruNode.inputSize, rruNode.hiddenSize ?? rruNode.outputSize, rruWeights);
    const ref = rru.stepReference(input);

    expectClose(out, ref, 1e-4);
  });
});
