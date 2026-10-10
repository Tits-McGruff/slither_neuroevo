import type { GraphSpec } from '../../src/brains/graph/schema.ts';

/** Recurrent families supported by native graph fixtures. */
export type Stage2RecurrentKind = 'GRU' | 'LSTM' | 'RRU';

/**
 * Construct the approved large-brain graph.
 * @param inputSize - Active v3 sensor input size.
 * @param recurrentKind - Recurrent family to include.
 * @returns Five-256-layer feature stack, recurrent 96, and Dense 2.
 */
export function buildLargeBrainGraph(
  inputSize: number,
  recurrentKind: Stage2RecurrentKind = 'GRU'
): GraphSpec {
  return {
    type: 'graph',
    nodes: [
      { id: 'input', type: 'Input', outputSize: inputSize },
      {
        id: 'features',
        type: 'MLP',
        inputSize,
        hiddenSizes: [256, 256, 256, 256],
        outputSize: 256
      },
      {
        id: 'memory',
        type: recurrentKind,
        inputSize: 256,
        hiddenSize: 96
      },
      { id: 'output', type: 'Dense', inputSize: 96, outputSize: 2 }
    ],
    edges: [
      { from: 'input', to: 'features' },
      { from: 'features', to: 'memory' },
      { from: 'memory', to: 'output' }
    ],
    outputs: [{ nodeId: 'output' }],
    outputSize: 2
  };
}
