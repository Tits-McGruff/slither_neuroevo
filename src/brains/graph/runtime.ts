import type { VizData, VizLayer } from '../../protocol/messages.ts';
import type { Brain } from '../types.ts';
import { DenseHead, GRU, LSTM, MLP, RRU } from '../ops.ts';
import type { CompiledGraph, CompiledNode } from './compiler.ts';
import type { GraphNodeType } from './schema.ts';

/** Runtime representation of a compiled graph node. */
interface RuntimeNode {
  id: string;
  type: GraphNodeType;
  output: Float32Array;
  outputs?: Float32Array[];
  inputRefs: Float32Array[];
  forward: () => void;
  reset: () => void;
  mlp?: MLP;
  gru?: GRU;
  lstm?: LSTM;
  rru?: RRU;
  head?: DenseHead;
}

/**
 * Build visualization layers for a runtime node.
 * @param node - Runtime node to inspect.
 * @returns Visualization layer list.
 */
function createVizLayers(node: RuntimeNode): VizLayer[] {
  if (node.mlp) {
    const layers: VizLayer[] = [];
    const sizes = node.mlp.layerSizes;
    if (sizes.length) {
      layers.push({ count: sizes[0]!, activations: null });
      for (let i = 1; i < sizes.length; i++) {
        layers.push({ count: sizes[i]!, activations: node.mlp._bufs[i - 1] ?? null });
      }
    }
    return layers;
  }
  if (node.gru) {
    return [{ count: node.gru.hiddenSize, activations: node.gru.h, isRecurrent: true }];
  }
  if (node.lstm) {
    return [{ count: node.lstm.hiddenSize, activations: node.lstm.h, isRecurrent: true }];
  }
  if (node.rru) {
    return [{ count: node.rru.hiddenSize, activations: node.rru.h, isRecurrent: true }];
  }
  if (node.head) {
    return [{ count: node.head.outSize, activations: node.head._out }];
  }
  return [];
}

/**
 * Construct a runtime node from compiled metadata and weight buffer.
 * @param node - Compiled node metadata.
 * @param weights - Full weight buffer.
 * @param inputRefs - Input buffers from upstream nodes.
 * @returns Runtime node instance.
 */
function buildRuntimeNode(node: CompiledNode, weights: Float32Array, inputRefs: Float32Array[]): RuntimeNode {
  const slice = node.paramLength
    ? weights.subarray(node.paramOffset, node.paramOffset + node.paramLength)
    : new Float32Array(0);
  switch (node.type) {
    case 'Input': {
      const output = new Float32Array(node.outputSize);
      return {
        id: node.id,
        type: node.type,
        output,
        inputRefs: [],
        forward: () => {},
        reset: () => {}
      };
    }
    case 'Concat': {
      const output = new Float32Array(node.outputSize);
      return {
        id: node.id,
        type: node.type,
        output,
        inputRefs,
        forward: () => {
          let offset = 0;
          for (const input of inputRefs) {
            output.set(input, offset);
            offset += input.length;
          }
        },
        reset: () => {}
      };
    }
    case 'Split': {
      const outputs = (node.outputSizes ?? []).map(size => new Float32Array(size));
      return {
        id: node.id,
        type: node.type,
        output: outputs[0] ?? new Float32Array(0),
        outputs,
        inputRefs,
        forward: () => {
          const src = inputRefs[0];
          if (!src) return;
          let offset = 0;
          outputs.forEach(out => {
            out.set(src.subarray(offset, offset + out.length));
            offset += out.length;
          });
        },
        reset: () => {}
      };
    }
    case 'Dense': {
      const head = new DenseHead(node.inputSize, node.outputSize, slice);
      return {
        id: node.id,
        type: node.type,
        output: head._out,
        inputRefs,
        head,
        forward: () => {
          head.forward(inputRefs[0]!);
        },
        reset: () => {}
      };
    }
    case 'MLP': {
      const sizes = [node.inputSize, ...(node.hiddenSizes ?? []), node.outputSize];
      const mlp = new MLP(sizes, slice);
      const output = mlp._bufs[mlp._bufs.length - 1] ?? new Float32Array(0);
      return {
        id: node.id,
        type: node.type,
        output,
        inputRefs,
        mlp,
        forward: () => {
          mlp.forward(inputRefs[0]!);
        },
        reset: () => {}
      };
    }
    case 'GRU': {
      const gru = new GRU(node.inputSize, node.hiddenSize ?? node.outputSize, slice);
      return {
        id: node.id,
        type: node.type,
        output: gru.h,
        inputRefs,
        gru,
        forward: () => {
          gru.step(inputRefs[0]!);
        },
        reset: () => {
          gru.reset();
        }
      };
    }
    case 'LSTM': {
      const lstm = new LSTM(node.inputSize, node.hiddenSize ?? node.outputSize, slice);
      return {
        id: node.id,
        type: node.type,
        output: lstm.h,
        inputRefs,
        lstm,
        forward: () => {
          lstm.step(inputRefs[0]!);
        },
        reset: () => {
          lstm.reset();
        }
      };
    }
    case 'RRU': {
      const rru = new RRU(node.inputSize, node.hiddenSize ?? node.outputSize, slice);
      return {
        id: node.id,
        type: node.type,
        output: rru.h,
        inputRefs,
        rru,
        forward: () => {
          rru.step(inputRefs[0]!);
        },
        reset: () => {
          rru.reset();
        }
      };
    }
    default:
      throw new Error(`Graph runtime: unsupported node ${node.type}.`);
  }
}

/** Graph-based brain runtime for compiled specs. */
export class GraphBrain implements Brain {
  /** Compiled graph metadata used by this runtime. */
  compiled: CompiledGraph;
  /** Weight buffer for all nodes. */
  weights: Float32Array;
  /** Runtime nodes built from compiled metadata. */
  nodes: RuntimeNode[];
  /** Flattened output buffer returned from forward passes. */
  output: Float32Array;
  /** Input buffer for the input node. */
  inputBuffer: Float32Array;
  /** Output references for each graph output port. */
  outputRefs: Float32Array[];

  /**
   * Create a graph brain runtime from a compiled spec and weights.
   * @param compiled - Compiled graph metadata.
   * @param weights - Weight buffer for all nodes.
   */
  constructor(compiled: CompiledGraph, weights: Float32Array) {
    this.compiled = compiled;
    this.weights = weights;
    if (weights.length < compiled.totalParams) {
      throw new Error(`Graph runtime: weight length ${weights.length} < ${compiled.totalParams}.`);
    }
    this.nodes = [];
    this.output = new Float32Array(compiled.outputSize);
    const inputNode = compiled.nodes.find(node => node.type === 'Input');
    if (!inputNode) throw new Error('Graph runtime: missing Input node.');
    this.inputBuffer = new Float32Array(inputNode.outputSize);
    const nodeOutputs = new Map<string, Float32Array[]>();
    for (const node of compiled.nodes) {
      const inputRefs = node.inputs.map(input => {
        const outputs = nodeOutputs.get(input.fromId);
        if (!outputs) throw new Error(`Graph runtime: missing outputs for ${input.fromId}.`);
        return outputs[input.fromPort]!;
      });
      const runtime = buildRuntimeNode(node, weights, inputRefs);
      if (node.type === 'Input') {
        runtime.output = this.inputBuffer;
      }
      this.nodes.push(runtime);
      if (runtime.outputs && runtime.outputs.length) {
        nodeOutputs.set(node.id, runtime.outputs);
      } else {
        nodeOutputs.set(node.id, [runtime.output]);
      }
    }
    this.outputRefs = compiled.outputs.map(out => {
      const outputs = nodeOutputs.get(out.nodeId);
      if (!outputs) throw new Error(`Graph runtime: missing output ref for ${out.nodeId}.`);
      return outputs[out.port ?? 0]!;
    });
  }

  /**
   * Run a forward pass through the graph.
   * @param input - Input buffer to copy into the input node.
   * @returns Output buffer.
   */
  forward(input: Float32Array): Float32Array {
    if (input.length === this.inputBuffer.length) {
      this.inputBuffer.set(input);
    } else {
      this.inputBuffer.fill(0);
      this.inputBuffer.set(input.subarray(0, this.inputBuffer.length));
    }
    for (const node of this.nodes) node.forward();
    let offset = 0;
    for (const out of this.outputRefs) {
      this.output.set(out, offset);
      offset += out.length;
    }
    return this.output;
  }

  /** Reset all node state to the initial state. */
  reset(): void {
    for (const node of this.nodes) node.reset();
  }

  /** Return the total parameter length for this graph. */
  paramLength(): number {
    return this.compiled.totalParams;
  }

  /**
   * Build visualization data for all nodes.
   * @returns Visualization payload.
   */
  getVizData(): VizData {
    const layers: VizLayer[] = [];
    for (const node of this.nodes) {
      const nodeLayers = createVizLayers(node);
      if (nodeLayers.length) layers.push(...nodeLayers);
    }
    return { kind: 'graph', layers };
  }
}
