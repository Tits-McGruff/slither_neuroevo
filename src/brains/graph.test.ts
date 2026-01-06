import { describe, it, expect } from 'vitest';
import { compileGraph, graphKey } from './graph/compiler.ts';
import type { GraphSpec } from './graph/schema.ts';
import { headParamCount, mlpParamCount } from './ops.ts';

/** Test suite label for graph compiler integration cases. */
const SUITE = 'graph compiler';

describe(SUITE, () => {
  it('compiles a simple graph and computes param length', () => {
    const spec: GraphSpec = {
      type: 'graph',
      nodes: [
        { id: 'input', type: 'Input', outputSize: 4 },
        { id: 'mlp', type: 'MLP', inputSize: 4, outputSize: 3, hiddenSizes: [5] },
        { id: 'head', type: 'Dense', inputSize: 3, outputSize: 2 }
      ],
      edges: [
        { from: 'input', to: 'mlp' },
        { from: 'mlp', to: 'head' }
      ],
      outputs: [{ nodeId: 'head' }],
      outputSize: 2
    };
    const compiled = compileGraph(spec);
    const expected = mlpParamCount([4, 5, 3]) + headParamCount(3, 2);
    expect(compiled.totalParams).toBe(expected);
  });

  it('graphKey ignores node ordering differences', () => {
    const specA: GraphSpec = {
      type: 'graph',
      nodes: [
        { id: 'input', type: 'Input', outputSize: 3 },
        { id: 'head', type: 'Dense', inputSize: 3, outputSize: 2 }
      ],
      edges: [{ from: 'input', to: 'head' }],
      outputs: [{ nodeId: 'head' }],
      outputSize: 2
    };
    const specB: GraphSpec = {
      ...specA,
      nodes: [...specA.nodes].reverse()
    };
    expect(graphKey(specA)).toBe(graphKey(specB));
  });
});
