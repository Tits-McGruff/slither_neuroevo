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

/**
 * Build the complete ASCII-ID graph shared with the Rust scalar parity fixture.
 * @returns Graph covering every currently supported runtime node.
 */
function buildRustScalarParitySpec(): GraphSpec {
  return {
    type: 'graph',
    outputSize: 2,
    nodes: [
      { id: 'in', type: 'Input', outputSize: 2 },
      { id: 'split', type: 'Split', outputSizes: [1, 1] },
      { id: 'denseA', type: 'Dense', inputSize: 1, outputSize: 1 },
      { id: 'mlpB', type: 'MLP', inputSize: 1, hiddenSizes: [2], outputSize: 1 },
      { id: 'features', type: 'Concat' },
      { id: 'gru', type: 'GRU', inputSize: 2, hiddenSize: 1 },
      { id: 'lstm', type: 'LSTM', inputSize: 2, hiddenSize: 1 },
      { id: 'rru', type: 'RRU', inputSize: 2, hiddenSize: 1 },
      { id: 'memory', type: 'Concat' },
      { id: 'head', type: 'Dense', inputSize: 3, outputSize: 2 }
    ],
    edges: [
      { from: 'in', to: 'split' },
      { from: 'split', fromPort: 0, to: 'denseA' },
      { from: 'split', fromPort: 1, to: 'mlpB' },
      { from: 'denseA', to: 'features', toPort: 0 },
      { from: 'mlpB', to: 'features', toPort: 1 },
      { from: 'features', to: 'gru' },
      { from: 'features', to: 'lstm' },
      { from: 'features', to: 'rru' },
      { from: 'gru', to: 'memory', toPort: 0 },
      { from: 'lstm', to: 'memory', toPort: 1 },
      { from: 'rru', to: 'memory', toPort: 2 },
      { from: 'memory', to: 'head' }
    ],
    outputs: [{ nodeId: 'head' }]
  };
}

/**
 * Build the hidden-width-two recurrent graph shared with the Rust indexing fixture.
 * @returns Graph whose recurrent matrices contain cross-hidden terms.
 */
function buildRustWideRecurrentParitySpec(): GraphSpec {
  return {
    type: 'graph',
    outputSize: 2,
    nodes: [
      { id: 'input', type: 'Input', outputSize: 3 },
      { id: 'gru', type: 'GRU', inputSize: 3, hiddenSize: 2 },
      { id: 'lstm', type: 'LSTM', inputSize: 3, hiddenSize: 2 },
      { id: 'rru', type: 'RRU', inputSize: 3, hiddenSize: 2 },
      { id: 'memory', type: 'Concat' },
      { id: 'head', type: 'Dense', inputSize: 6, outputSize: 2 }
    ],
    edges: [
      { from: 'input', to: 'gru' },
      { from: 'input', to: 'lstm' },
      { from: 'input', to: 'rru' },
      { from: 'gru', to: 'memory', toPort: 0 },
      { from: 'lstm', to: 'memory', toPort: 1 },
      { from: 'rru', to: 'memory', toPort: 2 },
      { from: 'memory', to: 'head' }
    ],
    outputs: [{ nodeId: 'head' }]
  };
}

/**
 * Pack current GraphBrain recurrent state in Rust compiled-node order.
 * @param brain - Runtime brain whose GRU/LSTM/RRU state is inspected.
 * @returns GRU hidden, LSTM hidden/cell, and RRU hidden values.
 */
function collectRustScalarParityState(brain: GraphBrain): Float32Array {
  const state: number[] = [];
  for (const node of brain.nodes) {
    if (node.gru) state.push(...node.gru.h);
    if (node.lstm) state.push(...node.lstm.h, ...node.lstm.c);
    if (node.rru) state.push(...node.rru.h);
  }
  return Float32Array.from(state);
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

  it('retains the complete two-step scalar fixture consumed by Rust tests', () => {
    const compiled = compileGraph(buildRustScalarParitySpec());
    expect(compiled.order).toEqual([
      'in', 'split', 'denseA', 'mlpB', 'features', 'gru', 'lstm', 'rru', 'memory', 'head'
    ]);
    expect(compiled.totalParams).toBe(53);
    expect(compiled.totalStateSize).toBe(4);
    const weights = Float32Array.from(
      { length: compiled.totalParams },
      (_, index) => (((index * 37) % 101) - 50) / 200
    );
    const brain = new GraphBrain(compiled, weights, 'js');

    const firstOutput = brain.forward(Float32Array.of(0.25, -0.75)).slice();
    const firstState = collectRustScalarParityState(brain);
    expectClose(firstOutput, Float32Array.of(0.024871822, -0.22658479), 1e-7);
    expectClose(
      firstState,
      Float32Array.of(-0.07793414, -0.04918596, -0.09008358, 0.083280325),
      1e-7
    );

    const secondOutput = brain.forward(Float32Array.of(-0.4, 0.6)).slice();
    const secondState = collectRustScalarParityState(brain);
    expectClose(secondOutput, Float32Array.of(0.016863106, -0.22644615), 1e-7);
    expectClose(
      secondState,
      Float32Array.of(-0.09880137, -0.06667301, -0.123771, 0.11877258),
      1e-7
    );
  });

  it('retains hidden-width-two recurrent indexing results consumed by Rust tests', () => {
    const compiled = compileGraph(buildRustWideRecurrentParitySpec());
    expect(compiled.order).toEqual(['input', 'gru', 'lstm', 'rru', 'memory', 'head']);
    expect(compiled.totalParams).toBe(122);
    expect(compiled.totalStateSize).toBe(8);
    const weights = Float32Array.from(
      { length: compiled.totalParams },
      (_, index) => (((index * 37) % 101) - 50) / 200
    );
    const brain = new GraphBrain(compiled, weights, 'js');
    for (const node of brain.nodes) {
      node.gru?.h.set([0.1, -0.2]);
      if (node.lstm) {
        node.lstm.h.set([0.05, -0.07]);
        node.lstm.c.set([0.2, -0.15]);
      }
      node.rru?.h.set([-0.11, 0.09]);
    }

    const firstOutput = brain.forward(Float32Array.of(0.25, -0.75, 0.5)).slice();
    const firstState = collectRustScalarParityState(brain);
    expectClose(firstOutput, Float32Array.of(0.10632071, -0.039742753), 1e-7);
    expectClose(
      firstState,
      Float32Array.of(
        -0.089948736, -0.12840696, 0.022501018, -0.014859255, 0.052411668,
        -0.026057417, -0.050840516, 0.106251605
      ),
      1e-7
    );

    const secondOutput = brain.forward(Float32Array.of(-0.4, 0.6, -0.2)).slice();
    const secondState = collectRustScalarParityState(brain);
    expectClose(secondOutput, Float32Array.of(0.20213757, -0.116188236), 1e-7);
    expectClose(
      secondState,
      Float32Array.of(
        0.01943205, 0.07960608, -0.09188931, -0.05800479, -0.1813142,
        -0.12077887, -0.19377622, -0.039415892
      ),
      1e-7
    );
  });
});
