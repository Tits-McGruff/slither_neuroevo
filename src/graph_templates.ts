/** Pure graph examples built from explicit settings and brain dimensions. */
import type { CFG_DEFAULT } from './config.ts';
import type { CoreSettings } from './protocol/settings.ts';
import type { GraphNodeSpec, GraphNodeType, GraphSpec } from './brains/graph/schema.ts';

/** Dimensions and recurrent defaults used by graph example builders. */
export type GraphTemplateBrain = Pick<typeof CFG_DEFAULT.brain,
  'inSize' | 'outSize' | 'gruHidden' | 'lstmHidden' | 'rruHidden'>;
/** Hidden-layer settings read by the browser at the example-selection boundary. */
export type GraphTemplateSettings = Pick<CoreSettings,
  'hiddenLayers' | 'neurons1' | 'neurons2' | 'neurons3' | 'neurons4' | 'neurons5'>;

/**
 * Build hidden layer sizes from the current core UI settings.
 * @param settings - Core UI settings snapshot.
 * @returns Array of hidden layer sizes.
 */
function buildHiddenSizesForExample(settings: GraphTemplateSettings): number[] {
  const layers = settings.hiddenLayers;
  const hidden: number[] = [];
  if (layers >= 1) hidden.push(settings.neurons1);
  if (layers >= 2) hidden.push(settings.neurons2);
  if (layers >= 3) hidden.push(settings.neurons3);
  if (layers >= 4) hidden.push(settings.neurons4);
  if (layers >= 5) hidden.push(settings.neurons5);
  if (!hidden.length) hidden.push(Math.max(4, settings.neurons1 || 8));
  return hidden;
}

/**
 * Build a linear MLP-only graph template.
 * @param settings - Hidden-layer settings captured at example selection.
 * @param brain - Current brain dimensions and recurrent defaults.
 * @returns Graph spec for a linear MLP.
 */
export function buildLinearMlpTemplate(settings: GraphTemplateSettings, brain: GraphTemplateBrain): GraphSpec {
  const hiddenSizes = buildHiddenSizesForExample(settings);
  const inputSize = brain.inSize;
  const outputSize = brain.outSize;
  return {
    type: 'graph',
    outputSize,
    nodes: [
      { id: 'input', type: 'Input', outputSize: inputSize },
      { id: 'mlp', type: 'MLP', inputSize, outputSize, hiddenSizes }
    ],
    edges: [{ from: 'input', to: 'mlp' }],
    outputs: [{ nodeId: 'mlp' }]
  };
}

/**
 * Build a template with MLP to GRU to MLP stages.
 * @param settings - Hidden-layer settings captured at example selection.
 * @param brain - Current brain dimensions and recurrent defaults.
 * @returns Graph spec for MLP-GRU-MLP.
 */
export function buildMlpGruMlpTemplate(settings: GraphTemplateSettings, brain: GraphTemplateBrain): GraphSpec {
  const hiddenSizes = buildHiddenSizesForExample(settings);
  const inputSize = brain.inSize;
  const outputSize = brain.outSize;
  const featureSize = hiddenSizes[hiddenSizes.length - 1] ?? Math.max(4, Math.floor(inputSize * 0.75));
  const mlpHidden = hiddenSizes.length > 1 ? hiddenSizes.slice(0, -1) : hiddenSizes;
  const gruHidden = Math.max(2, Math.floor(brain.gruHidden || featureSize));
  return {
    type: 'graph',
    outputSize,
    nodes: [
      { id: 'input', type: 'Input', outputSize: inputSize },
      { id: 'mlp-a', type: 'MLP', inputSize, outputSize: featureSize, hiddenSizes: mlpHidden },
      { id: 'gru', type: 'GRU', inputSize: featureSize, hiddenSize: gruHidden },
      { id: 'mlp-b', type: 'MLP', inputSize: gruHidden, outputSize, hiddenSizes }
    ],
    edges: [
      { from: 'input', to: 'mlp-a' },
      { from: 'mlp-a', to: 'gru' },
      { from: 'gru', to: 'mlp-b' }
    ],
    outputs: [{ nodeId: 'mlp-b' }]
  };
}

/**
 * Build a template with skip connection and concat.
 * @param settings - Hidden-layer settings captured at example selection.
 * @param brain - Current brain dimensions and recurrent defaults.
 * @returns Graph spec for a skip-style template.
 */
export function buildSkipTemplate(settings: GraphTemplateSettings, brain: GraphTemplateBrain): GraphSpec {
  const hiddenSizes = buildHiddenSizesForExample(settings);
  const inputSize = brain.inSize;
  const outputSize = brain.outSize;
  const mlpOut = inputSize;
  return {
    type: 'graph',
    outputSize,
    nodes: [
      { id: 'input', type: 'Input', outputSize: inputSize },
      { id: 'mlp-skip', type: 'MLP', inputSize, outputSize: mlpOut, hiddenSizes },
      { id: 'concat', type: 'Concat' },
      { id: 'head', type: 'Dense', inputSize: inputSize + mlpOut, outputSize }
    ],
    edges: [
      { from: 'input', to: 'mlp-skip' },
      { from: 'input', to: 'concat', toPort: 0 },
      { from: 'mlp-skip', to: 'concat', toPort: 1 },
      { from: 'concat', to: 'head' }
    ],
    outputs: [{ nodeId: 'head' }]
  };
}

/**
 * Build a template with a split output head.
 * @param brain - Current brain dimensions and recurrent defaults.
 * @returns Graph spec for a split output template.
 */
export function buildSplitTemplate(brain: GraphTemplateBrain): GraphSpec {
  const inputSize = brain.inSize;
  const outputSize = brain.outSize;
  const leftIn = Math.max(1, Math.floor(inputSize / 2));
  const rightIn = Math.max(1, inputSize - leftIn);
  const leftOut = Math.max(1, Math.floor(outputSize / 2));
  const rightOut = Math.max(1, outputSize - leftOut);
  return {
    type: 'graph',
    outputSize,
    nodes: [
      { id: 'input', type: 'Input', outputSize: inputSize },
      { id: 'split', type: 'Split', outputSizes: [leftIn, rightIn] },
      { id: 'head-a', type: 'Dense', inputSize: leftIn, outputSize: leftOut },
      { id: 'head-b', type: 'Dense', inputSize: rightIn, outputSize: rightOut }
    ],
    edges: [
      { from: 'input', to: 'split' },
      { from: 'split', to: 'head-a', fromPort: 0 },
      { from: 'split', to: 'head-b', fromPort: 1 }
    ],
    outputs: [{ nodeId: 'head-a' }, { nodeId: 'head-b' }]
  };
}

/**
 * Build a default node spec for a given node type.
 * @param type - Node type to create.
 * @param id - Node id to assign.
 * @param brain - Current brain dimensions and recurrent defaults.
 * @param settings - Hidden-layer settings captured at node creation.
 * @returns Node specification.
 */
export function buildDefaultNode(type: GraphNodeType, id: string, brain: GraphTemplateBrain, settings: GraphTemplateSettings): GraphNodeSpec {
  const inputSize = brain.inSize;
  const outputSize = brain.outSize;
  const hiddenSizes = buildHiddenSizesForExample(settings);
  switch (type) {
    case 'Input':
      return { id, type: 'Input', outputSize: inputSize };
    case 'Dense':
      return { id, type: 'Dense', inputSize, outputSize };
    case 'MLP':
      return { id, type: 'MLP', inputSize, outputSize, hiddenSizes };
    case 'GRU':
      return { id, type: 'GRU', inputSize, hiddenSize: Math.max(2, Math.floor(brain.gruHidden || 8)) };
    case 'LSTM':
      return { id, type: 'LSTM', inputSize, hiddenSize: Math.max(2, Math.floor(brain.lstmHidden || brain.gruHidden || 8)) };
    case 'RRU':
      return { id, type: 'RRU', inputSize, hiddenSize: Math.max(2, Math.floor(brain.rruHidden || brain.gruHidden || 8)) };
    case 'Split': {
      const first = Math.max(1, Math.floor(inputSize / 2));
      const second = Math.max(1, inputSize - first);
      return { id, type: 'Split', outputSizes: [first, second] };
    }
    case 'Concat':
      return { id, type: 'Concat' };
  }
}
