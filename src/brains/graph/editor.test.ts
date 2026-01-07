import { describe, expect, it } from 'vitest';
import { inferGraphSizes } from './editor.ts';
import type { GraphSpec } from './schema.ts';

/** Test suite label for graph editor size inference. */
const SUITE = 'graph editor sizes';

/**
 * Build a minimal graph spec for testing.
 * @param spec - Graph spec to return.
 * @returns Provided graph spec.
 */
function makeSpec(spec: GraphSpec): GraphSpec {
  return spec;
}

describe(SUITE, () => {
  it('infers input sizes for a linear graph', () => {
    const spec = makeSpec({
      type: 'graph',
      outputSize: 2,
      nodes: [
        { id: 'input', type: 'Input', outputSize: 6 },
        { id: 'dense', type: 'Dense', inputSize: 6, outputSize: 2 }
      ],
      edges: [{ from: 'input', to: 'dense' }],
      outputs: [{ nodeId: 'dense' }]
    });
    const state = inferGraphSizes(spec);
    expect(state.sizes.get('dense')?.inputSize).toBe(6);
  });

  it('infers concat output sizes from upstream ports', () => {
    const spec = makeSpec({
      type: 'graph',
      outputSize: 2,
      nodes: [
        { id: 'input', type: 'Input', outputSize: 4 },
        { id: 'split', type: 'Split', outputSizes: [2, 2] },
        { id: 'concat', type: 'Concat' },
        { id: 'head', type: 'Dense', inputSize: 4, outputSize: 2 }
      ],
      edges: [
        { from: 'input', to: 'split' },
        { from: 'split', to: 'concat', fromPort: 0, toPort: 0 },
        { from: 'split', to: 'concat', fromPort: 1, toPort: 1 },
        { from: 'concat', to: 'head' }
      ],
      outputs: [{ nodeId: 'head' }]
    });
    const state = inferGraphSizes(spec);
    expect(state.sizes.get('concat')?.outputSizes?.[0]).toBe(4);
    expect(state.sizes.get('head')?.inputSize).toBe(4);
  });

  it('reports split size mismatches', () => {
    const spec = makeSpec({
      type: 'graph',
      outputSize: 2,
      nodes: [
        { id: 'input', type: 'Input', outputSize: 3 },
        { id: 'split', type: 'Split', outputSizes: [1, 1] },
        { id: 'head', type: 'Dense', inputSize: 2, outputSize: 2 }
      ],
      edges: [
        { from: 'input', to: 'split' },
        { from: 'split', to: 'head', fromPort: 0 }
      ],
      outputs: [{ nodeId: 'head' }]
    });
    const state = inferGraphSizes(spec);
    const combined = state.errors.join(' ');
    expect(combined).toMatch(/Split split output sizes do not sum to input size/);
  });
});
