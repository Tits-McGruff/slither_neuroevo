import {
  gruParamCount,
  headParamCount,
  lstmParamCount,
  mlpParamCount,
  rruParamCount
} from '../ops.ts';
import type { GraphEdge, GraphNodeSpec, GraphNodeType, GraphOutputRef, GraphSpec } from './schema.ts';

/** Input reference for a compiled node. */
export interface GraphInputRef {
  fromId: string;
  fromPort: number;
}

/** Compiled node metadata with resolved sizes and parameter ranges. */
export interface CompiledNode {
  id: string;
  type: GraphNodeType;
  inputSize: number;
  outputSize: number;
  outputSizes?: number[];
  hiddenSize?: number;
  hiddenSizes?: number[];
  paramOffset: number;
  paramLength: number;
  inputs: GraphInputRef[];
}

/** Compiled graph containing topological order and parameter metadata. */
export interface CompiledGraph {
  key: string;
  spec: GraphSpec;
  nodes: CompiledNode[];
  order: string[];
  totalParams: number;
  outputSize: number;
  outputs: GraphOutputRef[];
}

/**
 * Build a stable signature string for a node.
 * @param node - Graph node spec.
 * @returns Signature string used for hashing.
 */
function nodeSignature(node: GraphNodeSpec): string {
  switch (node.type) {
    case 'Input':
      return `${node.id}:Input:${node.outputSize}`;
    case 'Dense':
      return `${node.id}:Dense:${node.inputSize}x${node.outputSize}`;
    case 'MLP': {
      const hidden = node.hiddenSizes?.length ? node.hiddenSizes.join('x') : 'none';
      return `${node.id}:MLP:${node.inputSize}x${hidden}x${node.outputSize}`;
    }
    case 'GRU':
      return `${node.id}:GRU:${node.inputSize}x${node.hiddenSize}`;
    case 'LSTM':
      return `${node.id}:LSTM:${node.inputSize}x${node.hiddenSize}`;
    case 'RRU':
      return `${node.id}:RRU:${node.inputSize}x${node.hiddenSize}`;
    case 'Split':
      return `${node.id}:Split:${node.outputSizes.join(',')}`;
    case 'Concat':
      return `${node.id}:Concat`;
  }
}

/**
 * Compute a stable key for a graph spec.
 * @param spec - Graph spec to hash.
 * @returns Stable key string.
 */
export function graphKey(spec: GraphSpec): string {
  const nodes = [...spec.nodes].sort((a, b) => a.id.localeCompare(b.id)).map(nodeSignature);
  const edges = [...spec.edges]
    .sort((a, b) => {
      const aKey = `${a.from}:${a.to}:${a.fromPort ?? 0}:${a.toPort ?? 0}`;
      const bKey = `${b.from}:${b.to}:${b.fromPort ?? 0}:${b.toPort ?? 0}`;
      return aKey.localeCompare(bKey);
    })
    .map(e => `${e.from}->${e.to}:${e.fromPort ?? 0}:${e.toPort ?? 0}`);
  const outputs = spec.outputs.map(o => `${o.nodeId}:${o.port ?? 0}`);
  return `graph|out:${spec.outputSize}|nodes:${nodes.join(';')}|edges:${edges.join(';')}|outs:${outputs.join(';')}`;
}

/**
 * Order incoming edges for a node, validating port assignments.
 * @param edges - Incoming edges to order.
 * @param nodeId - Node id for error reporting.
 * @returns Ordered edges.
 */
function orderIncomingEdges(edges: GraphEdge[], nodeId: string): GraphEdge[] {
  if (edges.length <= 1) return edges;
  const hasPort = edges.some(edge => edge.toPort != null);
  const lacksPort = edges.some(edge => edge.toPort == null);
  if (hasPort && lacksPort) {
    throw new Error(`Graph: mixed toPort usage for node ${nodeId}.`);
  }
  if (hasPort) {
    const used = new Set<number>();
    edges.forEach(edge => {
      const port = edge.toPort ?? 0;
      if (port < 0) throw new Error(`Graph: negative toPort on ${nodeId}.`);
      if (used.has(port)) throw new Error(`Graph: duplicate toPort ${port} on ${nodeId}.`);
      used.add(port);
    });
    return [...edges].sort((a, b) => (a.toPort ?? 0) - (b.toPort ?? 0));
  }
  return [...edges].sort((a, b) => a.from.localeCompare(b.from));
}

/**
 * Compile a graph spec into a runtime-ready structure.
 * @param spec - Graph spec to compile.
 * @returns Compiled graph metadata.
 */
export function compileGraph(spec: GraphSpec): CompiledGraph {
  if (!spec.nodes.length) throw new Error('Graph: no nodes defined.');
  const nodeById = new Map<string, GraphNodeSpec>();
  for (const node of spec.nodes) {
    if (nodeById.has(node.id)) throw new Error(`Graph: duplicate node id ${node.id}.`);
    nodeById.set(node.id, node);
  }
  const incomingById = new Map<string, GraphEdge[]>();
  const outgoingById = new Map<string, GraphEdge[]>();
  const indegree = new Map<string, number>();
  spec.nodes.forEach(node => {
    incomingById.set(node.id, []);
    outgoingById.set(node.id, []);
    indegree.set(node.id, 0);
  });
  for (const edge of spec.edges) {
    const from = nodeById.get(edge.from);
    const to = nodeById.get(edge.to);
    if (!from) throw new Error(`Graph: edge from unknown node ${edge.from}.`);
    if (!to) throw new Error(`Graph: edge to unknown node ${edge.to}.`);
    incomingById.get(edge.to)!.push(edge);
    outgoingById.get(edge.from)!.push(edge);
    indegree.set(edge.to, (indegree.get(edge.to) ?? 0) + 1);
  }

  const queue = [...spec.nodes]
    .map(node => node.id)
    .filter(id => (indegree.get(id) ?? 0) === 0)
    .sort((a, b) => a.localeCompare(b));
  const order: string[] = [];
  while (queue.length) {
    const id = queue.shift()!;
    order.push(id);
    for (const edge of outgoingById.get(id) ?? []) {
      const next = edge.to;
      const nextDegree = (indegree.get(next) ?? 0) - 1;
      indegree.set(next, nextDegree);
      if (nextDegree === 0) {
        queue.push(next);
        queue.sort((a, b) => a.localeCompare(b));
      }
    }
  }
  if (order.length !== spec.nodes.length) throw new Error('Graph: cycle detected.');

  const outputSizes = new Map<string, number[]>();
  const compiledNodes: CompiledNode[] = [];
  let paramOffset = 0;
  for (const id of order) {
    const node = nodeById.get(id)!;
    const incoming = orderIncomingEdges(incomingById.get(id) ?? [], id);
    const inputs: GraphInputRef[] = incoming.map(edge => ({
      fromId: edge.from,
      fromPort: edge.fromPort ?? 0
    }));
    const inputSizes = inputs.map(input => {
      const sizes = outputSizes.get(input.fromId);
      if (!sizes) throw new Error(`Graph: missing output sizes for ${input.fromId}.`);
      const port = input.fromPort ?? 0;
      if (port < 0 || port >= sizes.length) {
        throw new Error(`Graph: invalid fromPort ${port} on edge to ${id}.`);
      }
      return sizes[port]!;
    });

    let inputSize = 0;
    let outputSize = 0;
    let nodeOutputSizes: number[] = [];
    let paramLength = 0;
    let hiddenSize: number | undefined;
    let hiddenSizes: number[] | undefined;

    switch (node.type) {
      case 'Input': {
        if (incoming.length) throw new Error(`Graph: Input node ${id} has incoming edges.`);
        inputSize = node.outputSize;
        outputSize = node.outputSize;
        nodeOutputSizes = [node.outputSize];
        break;
      }
      case 'Concat': {
        if (!incoming.length) throw new Error(`Graph: Concat node ${id} has no inputs.`);
        inputSize = inputSizes.reduce((a, b) => a + b, 0);
        outputSize = inputSize;
        nodeOutputSizes = [outputSize];
        break;
      }
      case 'Split': {
        if (incoming.length !== 1) throw new Error(`Graph: Split node ${id} must have 1 input.`);
        inputSize = inputSizes[0] ?? 0;
        nodeOutputSizes = node.outputSizes.slice();
        outputSize = nodeOutputSizes.reduce((a, b) => a + b, 0);
        if (outputSize !== inputSize) {
          throw new Error(`Graph: Split ${id} output sizes do not sum to input size.`);
        }
        break;
      }
      case 'Dense': {
        if (incoming.length !== 1) throw new Error(`Graph: Dense node ${id} must have 1 input.`);
        inputSize = inputSizes[0] ?? 0;
        if (inputSize !== node.inputSize) {
          throw new Error(`Graph: Dense ${id} input size mismatch.`);
        }
        outputSize = node.outputSize;
        nodeOutputSizes = [outputSize];
        paramLength = headParamCount(inputSize, outputSize);
        break;
      }
      case 'MLP': {
        if (incoming.length !== 1) throw new Error(`Graph: MLP node ${id} must have 1 input.`);
        inputSize = inputSizes[0] ?? 0;
        if (inputSize !== node.inputSize) {
          throw new Error(`Graph: MLP ${id} input size mismatch.`);
        }
        hiddenSizes = node.hiddenSizes ? node.hiddenSizes.slice() : [];
        outputSize = node.outputSize;
        nodeOutputSizes = [outputSize];
        paramLength = mlpParamCount([inputSize, ...hiddenSizes, outputSize]);
        break;
      }
      case 'GRU': {
        if (incoming.length !== 1) throw new Error(`Graph: GRU node ${id} must have 1 input.`);
        inputSize = inputSizes[0] ?? 0;
        if (inputSize !== node.inputSize) throw new Error(`Graph: GRU ${id} input size mismatch.`);
        hiddenSize = node.hiddenSize;
        outputSize = node.hiddenSize;
        nodeOutputSizes = [outputSize];
        paramLength = gruParamCount(inputSize, hiddenSize);
        break;
      }
      case 'LSTM': {
        if (incoming.length !== 1) throw new Error(`Graph: LSTM node ${id} must have 1 input.`);
        inputSize = inputSizes[0] ?? 0;
        if (inputSize !== node.inputSize) throw new Error(`Graph: LSTM ${id} input size mismatch.`);
        hiddenSize = node.hiddenSize;
        outputSize = node.hiddenSize;
        nodeOutputSizes = [outputSize];
        paramLength = lstmParamCount(inputSize, hiddenSize);
        break;
      }
      case 'RRU': {
        if (incoming.length !== 1) throw new Error(`Graph: RRU node ${id} must have 1 input.`);
        inputSize = inputSizes[0] ?? 0;
        if (inputSize !== node.inputSize) throw new Error(`Graph: RRU ${id} input size mismatch.`);
        hiddenSize = node.hiddenSize;
        outputSize = node.hiddenSize;
        nodeOutputSizes = [outputSize];
        paramLength = rruParamCount(inputSize, hiddenSize);
        break;
      }
      default:
        throw new Error(`Graph: unknown node type ${(node as GraphNodeSpec).type}.`);
    }

    const compiled: CompiledNode = {
      id,
      type: node.type,
      inputSize,
      outputSize,
      paramOffset,
      paramLength,
      inputs,
      ...(nodeOutputSizes.length > 1 ? { outputSizes: nodeOutputSizes.slice() } : {}),
      ...(hiddenSize != null ? { hiddenSize } : {}),
      ...(hiddenSizes && hiddenSizes.length ? { hiddenSizes: hiddenSizes.slice() } : {})
    };
    compiledNodes.push(compiled);
    outputSizes.set(id, nodeOutputSizes);
    paramOffset += paramLength;
  }

  let totalOutput = 0;
  for (const out of spec.outputs) {
    const sizes = outputSizes.get(out.nodeId);
    if (!sizes) throw new Error(`Graph: output references unknown node ${out.nodeId}.`);
    const port = out.port ?? 0;
    if (port < 0 || port >= sizes.length) {
      throw new Error(`Graph: output port ${port} out of bounds on ${out.nodeId}.`);
    }
    totalOutput += sizes[port] ?? 0;
  }
  if (totalOutput !== spec.outputSize) {
    throw new Error(`Graph: output size mismatch (expected ${spec.outputSize}, got ${totalOutput}).`);
  }

  return {
    key: graphKey(spec),
    spec,
    nodes: compiledNodes,
    order,
    totalParams: paramOffset,
    outputSize: totalOutput,
    outputs: spec.outputs
  };
}
