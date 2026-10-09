/** Generate the retained TypeScript Genome.random compatibility fixture. */

import { CFG } from '../../src/config.ts';
import { graphKey } from '../../src/brains/graph/compiler.ts';
import type { GraphSpec } from '../../src/brains/graph/schema.ts';
import { Genome, enrichArchInfo } from '../../src/mlp.ts';
import { StatefulRng } from '../../src/rng.ts';

/** TypeScript source revision whose implementation this fixture executes. */
const SOURCE_REVISION = '258ac69e80df411fa724ad16f1b2cb19e1ae210c';
/** Seed chosen to exercise the exact xorshift32 continuation. */
const SEED = 0x12345678;
/** Non-default biases prove that each recurrent gate uses the projected setting. */
const BIASES = {
  gruInitUpdateBias: -0.55,
  lstmInitForgetBias: 0.75,
  rruInitGateBias: 0.2
} as const;

/**
 * Graph containing every parameter-bearing node and draw-free Split/Concat nodes.
 * Its chain topology makes TypeScript and canonical Rust node order identical.
 */
const GRAPH: GraphSpec = {
  type: 'graph',
  nodes: [
    { id: 'input', type: 'Input', outputSize: 4 },
    { id: 'split', type: 'Split', outputSizes: [2, 2] },
    { id: 'concat', type: 'Concat' },
    { id: 'mlp', type: 'MLP', inputSize: 4, hiddenSizes: [3], outputSize: 3 },
    { id: 'gru', type: 'GRU', inputSize: 3, hiddenSize: 2 },
    { id: 'lstm', type: 'LSTM', inputSize: 2, hiddenSize: 2 },
    { id: 'rru', type: 'RRU', inputSize: 2, hiddenSize: 2 },
    { id: 'head', type: 'Dense', inputSize: 2, outputSize: 2 }
  ],
  edges: [
    { from: 'input', to: 'split' },
    { from: 'split', fromPort: 0, to: 'concat', toPort: 0 },
    { from: 'split', fromPort: 1, to: 'concat', toPort: 1 },
    { from: 'concat', to: 'mlp' },
    { from: 'mlp', to: 'gru' },
    { from: 'gru', to: 'lstm' },
    { from: 'lstm', to: 'rru' },
    { from: 'rru', to: 'head' }
  ],
  outputs: [{ nodeId: 'head' }],
  outputSize: 2
};

/** Encode one Float32 value by its exact little-endian-independent bit pattern. */
function float32Hex(value: number): string {
  const buffer = new ArrayBuffer(4);
  const view = new DataView(buffer);
  view.setFloat32(0, value, false);
  return `0x${view.getUint32(0, false).toString(16).padStart(8, '0')}`;
}

CFG.brain.gruInitUpdateBias = BIASES.gruInitUpdateBias;
CFG.brain.lstmInitForgetBias = BIASES.lstmInitForgetBias;
CFG.brain.rruInitGateBias = BIASES.rruInitGateBias;

const architecture = { spec: GRAPH, key: graphKey(GRAPH) };
const info = enrichArchInfo(architecture);
const rng = new StatefulRng(SEED);
const genome = Genome.random(architecture, rng.asSource());

const fixture = {
  evidenceKind: 'current-source execution',
  sourceRevision: SOURCE_REVISION,
  command:
    'node .\\node_modules\\tsx\\dist\\cli.mjs scripts\\stage5\\generate-genome-init-fixture.ts',
  seed: `0x${SEED.toString(16).padStart(8, '0')}`,
  biases: BIASES,
  graph: GRAPH,
  compiledNodeRanges: info.nodes.map(node => ({
    id: node.id,
    type: node.type,
    offset: node.offset,
    length: node.length
  })),
  totalParameters: info.totalCount,
  weightBits: Array.from(genome.weights, float32Hex),
  nextRng: rng.exportState()
};

process.stdout.write(`${JSON.stringify(fixture, null, 2)}\n`);
