import { describe, it, expect } from 'vitest';
import { compileGraph, graphKey } from './graph/compiler.ts';
import type { GraphSpec } from './graph/schema.ts';

/** Test suite label for graph compiler unit cases. */
const SUITE = 'graph compiler (unit)';

describe(SUITE, () => {
  it('rejects cycles', () => {
    const spec: GraphSpec = {
      type: 'graph',
      nodes: [
        { id: 'input', type: 'Input', outputSize: 2 },
        { id: 'a', type: 'Dense', inputSize: 2, outputSize: 2 }
      ],
      edges: [
        { from: 'input', to: 'a' },
        { from: 'a', to: 'a' }
      ],
      outputs: [{ nodeId: 'a' }],
      outputSize: 2
    };
    expect(() => compileGraph(spec)).toThrow(/cycle/i);
  });

  it('rejects split size mismatch', () => {
    const spec: GraphSpec = {
      type: 'graph',
      nodes: [
        { id: 'input', type: 'Input', outputSize: 4 },
        { id: 'split', type: 'Split', outputSizes: [1, 1] }
      ],
      edges: [{ from: 'input', to: 'split' }],
      outputs: [{ nodeId: 'split', port: 0 }],
      outputSize: 1
    };
    expect(() => compileGraph(spec)).toThrow(/Split/i);
  });

  it('graphKey stays stable with edge ordering changes', () => {
    const specA: GraphSpec = {
      type: 'graph',
      nodes: [
        { id: 'input', type: 'Input', outputSize: 2 },
        { id: 'a', type: 'Dense', inputSize: 2, outputSize: 1 },
        { id: 'b', type: 'Dense', inputSize: 2, outputSize: 1 },
        { id: 'concat', type: 'Concat' }
      ],
      edges: [
        { from: 'input', to: 'a' },
        { from: 'input', to: 'b' },
        { from: 'a', to: 'concat', fromPort: 0 },
        { from: 'b', to: 'concat', fromPort: 0 }
      ],
      outputs: [{ nodeId: 'concat' }],
      outputSize: 2
    };
    const specB: GraphSpec = { ...specA, edges: [...specA.edges].reverse() };
    expect(graphKey(specA)).toBe(graphKey(specB));
  });
});
