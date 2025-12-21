import type { GraphSpec, GraphNodeSpec, GraphEdge, GraphOutputRef } from './graph/schema.ts';
type HiddenSettings = {
  hiddenLayers: number;
  neurons1: number;
  neurons2: number;
  neurons3: number;
  neurons4: number;
  neurons5: number;
};

type BrainCfg = {
  inSize: number;
  outSize: number;
  useMlp?: boolean | number;
  gruHidden?: number;
  lstmHidden?: number;
  rruHidden?: number;
  stack?: {
    gru?: number;
    lstm?: number;
    rru?: number;
  };
  stackOrder?: string[];
};

function buildHiddenSizes(settings: HiddenSettings): number[] {
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

export function buildStackGraphSpec(settings: HiddenSettings, cfg: { brain: BrainCfg }): GraphSpec {
  const hiddenSizes = buildHiddenSizes(settings);
  const inputSize = cfg.brain.inSize;
  const outputSize = cfg.brain.outSize;
  const useMlp = cfg.brain.useMlp !== false && cfg.brain.useMlp !== 0;
  const stack = cfg.brain.stack || {};
  const defaultOrder: Array<'gru' | 'lstm' | 'rru'> = ['gru', 'lstm', 'rru'];
  const rawOrder = Array.isArray(cfg.brain.stackOrder) ? cfg.brain.stackOrder : defaultOrder;
  const stackOrder = rawOrder
    .map(key => key.toLowerCase())
    .filter((key): key is 'gru' | 'lstm' | 'rru' => key === 'gru' || key === 'lstm' || key === 'rru')
    .filter((key, idx, arr) => arr.indexOf(key) === idx);
  if (!stackOrder.length) stackOrder.push(...defaultOrder);
  const enabled = stackOrder.filter(key => (stack as Record<string, number | undefined>)[key]);

  const nodes: GraphNodeSpec[] = [];
  const edges: GraphEdge[] = [];
  const outputs: GraphOutputRef[] = [];

  nodes.push({ id: 'input', type: 'Input', outputSize: inputSize });

  let prevId = 'input';
  let currentSize = inputSize;
  if (useMlp) {
    if (enabled.length > 0) {
      const featureSize = hiddenSizes[hiddenSizes.length - 1]!;
      const mlpHidden = hiddenSizes.slice(0, -1);
      nodes.push({
        id: 'mlp',
        type: 'MLP',
        inputSize,
        outputSize: featureSize,
        hiddenSizes: mlpHidden
      });
      edges.push({ from: prevId, to: 'mlp' });
      prevId = 'mlp';
      currentSize = featureSize;
    } else {
      nodes.push({
        id: 'mlp',
        type: 'MLP',
        inputSize,
        outputSize,
        hiddenSizes
      });
      edges.push({ from: prevId, to: 'mlp' });
      prevId = 'mlp';
      currentSize = outputSize;
    }
  } else if (enabled.length === 0) {
    nodes.push({ id: 'head', type: 'Dense', inputSize, outputSize });
    edges.push({ from: prevId, to: 'head' });
    outputs.push({ nodeId: 'head' });
    return {
      type: 'graph',
      nodes,
      edges,
      outputs,
      outputSize
    };
  }

  if (enabled.includes('gru')) {
    const hidden = Math.max(2, Math.floor(cfg.brain.gruHidden || 8));
    nodes.push({ id: 'gru', type: 'GRU', inputSize: currentSize, hiddenSize: hidden });
    edges.push({ from: prevId, to: 'gru' });
    prevId = 'gru';
    currentSize = hidden;
  }
  if (enabled.includes('lstm')) {
    const hidden = Math.max(2, Math.floor(cfg.brain.lstmHidden || cfg.brain.gruHidden || 8));
    nodes.push({ id: 'lstm', type: 'LSTM', inputSize: currentSize, hiddenSize: hidden });
    edges.push({ from: prevId, to: 'lstm' });
    prevId = 'lstm';
    currentSize = hidden;
  }
  if (enabled.includes('rru')) {
    const hidden = Math.max(2, Math.floor(cfg.brain.rruHidden || cfg.brain.gruHidden || 8));
    nodes.push({ id: 'rru', type: 'RRU', inputSize: currentSize, hiddenSize: hidden });
    edges.push({ from: prevId, to: 'rru' });
    prevId = 'rru';
    currentSize = hidden;
  }

  if (enabled.length > 0) {
    nodes.push({ id: 'head', type: 'Dense', inputSize: currentSize, outputSize });
    edges.push({ from: prevId, to: 'head' });
    outputs.push({ nodeId: 'head' });
  } else if (useMlp) {
    outputs.push({ nodeId: 'mlp' });
  } else {
    nodes.push({ id: 'head', type: 'Dense', inputSize: currentSize, outputSize });
    edges.push({ from: prevId, to: 'head' });
    outputs.push({ nodeId: 'head' });
  }

  return {
    type: 'graph',
    nodes,
    edges,
    outputs,
    outputSize
  };
}
