/** Pure graph copying, sizing and selection projections. */
import type { GraphSizeState } from './brains/graph/editor.ts';
import type { GraphSpec } from './brains/graph/schema.ts';
import type { GraphTemplateBrain } from './graph_templates.ts';


/**
 * Clone a graph spec to avoid mutating the original.
 * @param spec - Graph spec to clone.
 * @returns Cloned graph spec.
 */
export function cloneGraphSpec(spec: GraphSpec): GraphSpec {
  return {
    type: 'graph',
    outputSize: spec.outputSize,
    nodes: spec.nodes.map(node => {
      if (node.type === 'MLP') {
        return { ...node, hiddenSizes: node.hiddenSizes ? node.hiddenSizes.slice() : [] };
      }
      if (node.type === 'Split') {
        return { ...node, outputSizes: node.outputSizes.slice() };
      }
      return { ...node };
    }),
    edges: spec.edges.map(edge => ({ ...edge })),
    outputs: spec.outputs.map(out => ({ ...out }))
  };
}


/**
 * Apply inferred input sizes back into the draft spec.
 * @param spec - Graph spec to update in-place.
 * @param state - Size inference state for the spec.
 */
export function applyGraphSizeInference(spec: GraphSpec, state: GraphSizeState, brain: Pick<GraphTemplateBrain, 'inSize' | 'outSize'>): void {
  spec.outputSize = brain.outSize;
  spec.nodes.forEach(node => {
    if (node.type === 'Input') {
      node.outputSize = brain.inSize;
      return;
    }
    if (node.type === 'Dense' || node.type === 'MLP' || node.type === 'GRU' || node.type === 'LSTM' || node.type === 'RRU') {
      const inputSize = state.sizes.get(node.id)?.inputSize;
      if (inputSize != null && Number.isFinite(inputSize) && inputSize > 0) {
        node.inputSize = inputSize;
      }
    }
  });
}


/** Simple output selector state derived from the graph spec. */
export type SimpleOutputState =
  | { mode: 'simple'; nodeId: string; split: boolean }
  | { mode: 'custom'; nodeId: string | null };


/**
 * Derive the simple output selector state from a graph spec.
 * @param spec - Graph spec to inspect.
 * @returns Simple output selection state.
 */
export function resolveSimpleOutputState(spec: GraphSpec): SimpleOutputState {
  if (spec.outputs.length === 1) {
    const output = spec.outputs[0]!;
    return { mode: 'simple', nodeId: output.nodeId, split: false };
  }
  if (spec.outputs.length === 2) {
    const first = spec.outputs[0];
    const second = spec.outputs[1];
    if (first && second && first.nodeId === second.nodeId && (first.port ?? 0) === 0 && (second.port ?? 1) === 1) {
      return { mode: 'simple', nodeId: first.nodeId, split: true };
    }
  }
  const fallback = spec.outputs[0]?.nodeId ?? null;
  return { mode: 'custom', nodeId: fallback };
}


/**
 * Resolve the input size encoded by a graph spec's Input node.
 * @param spec - Graph spec to inspect.
 * @returns Input node output size or null if invalid/missing.
 */
export function getGraphSpecInputSize(spec: GraphSpec): number | null {
  const inputNodes = spec.nodes.filter(node => node.type === 'Input');
  if (inputNodes.length !== 1) return null;
  const size = inputNodes[0]?.outputSize;
  return Number.isFinite(size) ? (size as number) : null;
}


/**
 * Build a unique node id within a graph spec.
 * @param spec - Graph spec to scan.
 * @param prefix - Prefix to use for the id.
 * @returns Unique node id.
 */
export function buildUniqueNodeId(spec: GraphSpec, prefix: string): string {
  let idx = 1;
  let next = `${prefix}-${idx}`;
  const used = new Set(spec.nodes.map(node => node.id));
  if (!used.has(prefix)) return prefix;
  while (used.has(next)) {
    idx += 1;
    next = `${prefix}-${idx}`;
  }
  return next;
}
