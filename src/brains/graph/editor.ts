import type { GraphEdge, GraphNodeSpec, GraphSpec } from './schema.ts';

/** Inferred sizing data for a single graph node. */
export interface GraphSizeInfo {
  /** Inferred input size for the node, or null when unresolved. */
  inputSize: number | null;
  /** Inferred output sizes for the node, or null when unresolved. */
  outputSizes: number[] | null;
}

/** Aggregate size inference results for a graph spec. */
export interface GraphSizeState {
  /** Map of node id to inferred sizes. */
  sizes: Map<string, GraphSizeInfo>;
  /** Map of node id to size-related error messages. */
  nodeErrors: Map<string, string[]>;
  /** Flat list of size-related errors. */
  errors: string[];
}

/** Internal shape for ordered incoming edges. */
interface OrderedIncoming {
  /** Ordered edge list for size inference. */
  edges: GraphEdge[];
  /** Error messages encountered while ordering. */
  errors: string[];
}

/**
 * Record an error against a node and the global error list.
 * @param nodeId - Node id associated with the error.
 * @param message - Error message to record.
 * @param nodeErrors - Map of node errors to update.
 * @param errors - Global error list to append to.
 */
function recordNodeError(
  nodeId: string,
  message: string,
  nodeErrors: Map<string, string[]>,
  errors: string[]
): void {
  const bucket = nodeErrors.get(nodeId) ?? [];
  bucket.push(message);
  nodeErrors.set(nodeId, bucket);
  errors.push(message);
}

/**
 * Order incoming edges based on explicit toPort values when present.
 * @param edges - Incoming edges to order.
 * @param nodeId - Node id for error messaging.
 * @returns Ordered edges with any ordering errors.
 */
function orderIncomingEdges(edges: GraphEdge[], nodeId: string): OrderedIncoming {
  if (edges.length <= 1) return { edges, errors: [] };
  const errors: string[] = [];
  const hasPort = edges.some(edge => edge.toPort != null);
  const lacksPort = edges.some(edge => edge.toPort == null);
  if (hasPort && lacksPort) {
    errors.push(`Graph: mixed toPort usage for node ${nodeId}.`);
  }
  if (hasPort) {
    const used = new Set<number>();
    edges.forEach(edge => {
      const port = edge.toPort ?? 0;
      if (port < 0) errors.push(`Graph: negative toPort on ${nodeId}.`);
      if (used.has(port)) errors.push(`Graph: duplicate toPort ${port} on ${nodeId}.`);
      used.add(port);
    });
    return { edges: [...edges].sort((a, b) => (a.toPort ?? 0) - (b.toPort ?? 0)), errors };
  }
  return { edges: [...edges].sort((a, b) => a.from.localeCompare(b.from)), errors };
}

/**
 * Resolve an output size for an incoming edge using inferred output sizes.
 * @param edge - Edge to resolve.
 * @param sizes - Size lookup for upstream nodes.
 * @returns Output size or null when unresolved.
 */
function resolveIncomingSize(edge: GraphEdge, sizes: Map<string, GraphSizeInfo>): number | null {
  const outputs = sizes.get(edge.from)?.outputSizes;
  if (!outputs || !outputs.length) return null;
  const port = edge.fromPort ?? 0;
  if (port < 0 || port >= outputs.length) return null;
  return outputs[port] ?? null;
}

/**
 * Infer input/output sizes for a graph spec using topology and node metadata.
 * @param spec - Graph spec to inspect.
 * @returns Inferred sizing state with any errors.
 */
export function inferGraphSizes(spec: GraphSpec): GraphSizeState {
  const sizes = new Map<string, GraphSizeInfo>();
  const nodeErrors = new Map<string, string[]>();
  const errors: string[] = [];
  const nodeById = new Map<string, GraphNodeSpec>();
  spec.nodes.forEach(node => {
    if (nodeById.has(node.id)) {
      recordNodeError(node.id, `Graph: duplicate node id ${node.id}.`, nodeErrors, errors);
    }
    nodeById.set(node.id, node);
  });
  const incomingById = new Map<string, GraphEdge[]>();
  spec.nodes.forEach(node => incomingById.set(node.id, []));
  spec.edges.forEach(edge => {
    const from = nodeById.get(edge.from);
    const to = nodeById.get(edge.to);
    if (!from) {
      recordNodeError(edge.from, `Graph: edge from unknown node ${edge.from}.`, nodeErrors, errors);
      return;
    }
    if (!to) {
      recordNodeError(edge.to, `Graph: edge to unknown node ${edge.to}.`, nodeErrors, errors);
      return;
    }
    incomingById.get(edge.to)?.push(edge);
  });

  spec.nodes.forEach(node => {
    switch (node.type) {
      case 'Input':
        if (!Number.isFinite(node.outputSize) || node.outputSize <= 0) {
          recordNodeError(node.id, `Graph: Input node ${node.id} has invalid output size.`, nodeErrors, errors);
        }
        sizes.set(node.id, { inputSize: null, outputSizes: [node.outputSize] });
        break;
      case 'Dense':
      case 'MLP':
        if (!Number.isFinite(node.outputSize) || node.outputSize <= 0) {
          recordNodeError(node.id, `Graph: ${node.type} node ${node.id} has invalid output size.`, nodeErrors, errors);
        }
        sizes.set(node.id, { inputSize: null, outputSizes: [node.outputSize] });
        break;
      case 'GRU':
      case 'LSTM':
      case 'RRU':
        if (!Number.isFinite(node.hiddenSize) || node.hiddenSize <= 0) {
          recordNodeError(node.id, `Graph: ${node.type} node ${node.id} has invalid hidden size.`, nodeErrors, errors);
        }
        sizes.set(node.id, { inputSize: null, outputSizes: [node.hiddenSize] });
        break;
      case 'Split':
        sizes.set(node.id, { inputSize: null, outputSizes: node.outputSizes.slice() });
        if (!node.outputSizes.length) {
          recordNodeError(node.id, `Graph: Split ${node.id} has no output sizes.`, nodeErrors, errors);
        }
        node.outputSizes.forEach((size, index) => {
          if (!Number.isFinite(size) || size <= 0) {
            recordNodeError(node.id, `Graph: Split ${node.id} has invalid output size at index ${index}.`, nodeErrors, errors);
          }
        });
        break;
      case 'Concat':
        sizes.set(node.id, { inputSize: null, outputSizes: null });
        break;
      default: {
        const fallback = node as unknown as { id?: string; type?: string };
        const nodeId = fallback.id ?? 'unknown';
        const nodeType = fallback.type ?? 'unknown';
        recordNodeError(nodeId, `Graph: unknown node type ${nodeType}.`, nodeErrors, errors);
        sizes.set(nodeId, { inputSize: null, outputSizes: null });
        break;
      }
    }
  });

  const maxPasses = Math.max(1, spec.nodes.length + 1);
  for (let pass = 0; pass < maxPasses; pass += 1) {
    let changed = false;
    spec.nodes.forEach(node => {
      const info = sizes.get(node.id);
      if (!info) return;
      if (node.type === 'Input') return;
      const incoming = incomingById.get(node.id) ?? [];
      if (!incoming.length) return;
      const ordered = orderIncomingEdges(incoming, node.id);
      ordered.errors.forEach(message => recordNodeError(node.id, message, nodeErrors, errors));
      const incomingSizes: number[] = [];
      let unresolved = false;
      ordered.edges.forEach(edge => {
        const size = resolveIncomingSize(edge, sizes);
        if (size == null) {
          const outputs = sizes.get(edge.from)?.outputSizes;
          if (outputs && outputs.length) {
            const port = edge.fromPort ?? 0;
            if (port < 0 || port >= outputs.length) {
              recordNodeError(node.id, `Graph: invalid fromPort ${port} on edge to ${node.id}.`, nodeErrors, errors);
            }
          }
          unresolved = true;
          return;
        }
        incomingSizes.push(size);
      });
      if (unresolved || incomingSizes.length !== ordered.edges.length) return;
      if (info.inputSize == null) {
        info.inputSize = incomingSizes.reduce((sum, value) => sum + value, 0);
        changed = true;
      }
      if (node.type === 'Concat' && info.outputSizes == null) {
        info.outputSizes = [info.inputSize ?? 0];
        changed = true;
      }
    });
    if (!changed) break;
  }

  spec.nodes.forEach(node => {
    const incoming = incomingById.get(node.id) ?? [];
    const info = sizes.get(node.id);
    if (!info) return;
    if (node.type === 'Input') {
      if (incoming.length) {
        recordNodeError(node.id, `Graph: Input node ${node.id} has incoming edges.`, nodeErrors, errors);
      }
      return;
    }
    if (node.type === 'Concat') {
      if (!incoming.length) {
        recordNodeError(node.id, `Graph: Concat node ${node.id} has no inputs.`, nodeErrors, errors);
      }
      if (info.outputSizes == null) {
        recordNodeError(node.id, `Graph: Concat node ${node.id} inputs unresolved.`, nodeErrors, errors);
      }
      return;
    }
    if (node.type === 'Split') {
      if (incoming.length !== 1) {
        recordNodeError(node.id, `Graph: Split node ${node.id} must have 1 input.`, nodeErrors, errors);
      }
      if (info.inputSize != null && info.outputSizes) {
        const total = info.outputSizes.reduce((sum, size) => sum + size, 0);
        if (total !== info.inputSize) {
          recordNodeError(node.id, `Graph: Split ${node.id} output sizes do not sum to input size.`, nodeErrors, errors);
        }
      }
      return;
    }
    if (incoming.length !== 1) {
      recordNodeError(node.id, `Graph: ${node.type} node ${node.id} must have 1 input.`, nodeErrors, errors);
      return;
    }
    if (info.inputSize == null) {
      recordNodeError(node.id, `Graph: ${node.type} node ${node.id} input size unresolved.`, nodeErrors, errors);
    }
  });

  return { sizes, nodeErrors, errors };
}
