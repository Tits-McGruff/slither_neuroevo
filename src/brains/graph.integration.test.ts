import { describe, it, expect } from 'vitest';
import { compileGraph } from './graph/compiler.ts';
import { GraphBrain } from './graph/runtime.ts';
import type { GraphSpec } from './graph/schema.ts';

describe('graph brain (integration)', () => {
  it('runs a simple graph forward pass', () => {
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
    const compiled = compileGraph(spec);
    const weights = new Float32Array(compiled.totalParams).fill(0.01);
    const brain = new GraphBrain(compiled, weights);
    const out = brain.forward(new Float32Array([0.2, -0.1, 0.3]));
    expect(out.length).toBe(2);
  });
});
