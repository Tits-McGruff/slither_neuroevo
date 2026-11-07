import type { GraphSizeState } from './brains/graph/editor.ts';
import type { GraphNodeSpec,GraphNodeType,GraphSpec } from './brains/graph/schema.ts';
import { resolveSimpleOutputState,type SimpleOutputState } from './graph_state.ts';

/** Live graph form access; draft, selection and session stay with the entry owner. */
export interface GraphSettingsPort {
  readonly graphOutputSimpleNode: HTMLSelectElement | null;
  readonly graphOutputSimpleSplit: HTMLInputElement | null;
  readonly graphOutputSimpleHint: HTMLElement | null;
  readonly ensureGraphDraft: () => GraphSpec;
  readonly setGraphSpecStatus: (text: string, isError?: boolean) => void;
  readonly graphNodes: HTMLElement | null;
  readonly graphEdges: HTMLElement | null;
  readonly graphOutputs: HTMLElement | null;
  readonly updateGraphSizeState: (spec: GraphSpec) => GraphSizeState;
  readonly applyGraphSizeInference: (spec: GraphSpec, state: GraphSizeState) => void;
  readonly syncGraphSelection: (spec: GraphSpec) => void;
  readonly graphLayoutOverrides: Map<string, { x: number; y: number; }>;
  graphSelectedNodeId: string | null;
  readonly renderGraphDiagram: (spec: GraphSpec) => void;
  readonly renderGraphInspector: (spec: GraphSpec) => void;
  readonly brainOutputSize: number;
  readonly brainInputSize: number;
  readonly createNode: (type: GraphNodeType, id: string) => GraphNodeSpec;
}



/**
 * Render the simple outputs UI based on the current spec and size state.
 * @param spec - Graph spec to reflect in the UI.
 * @param sizeState - Inferred size state for sizing hints.
 */
export function renderSimpleOutputUi(spec: GraphSpec, sizeState: GraphSizeState, ports: GraphSettingsPort): void {
  const graphOutputSimpleNode = ports.graphOutputSimpleNode;
  const graphOutputSimpleSplit = ports.graphOutputSimpleSplit;
  const graphOutputSimpleHint = ports.graphOutputSimpleHint;
  if (!graphOutputSimpleNode || !graphOutputSimpleSplit) return;
  const selectableNodes = spec.nodes.slice();
  graphOutputSimpleNode.innerHTML = '';
  if (!selectableNodes.length) {
    graphOutputSimpleNode.disabled = true;
    graphOutputSimpleSplit.disabled = true;
    if (graphOutputSimpleHint) {
      graphOutputSimpleHint.textContent = 'Add a node before selecting outputs.';
      graphOutputSimpleHint.classList.remove('error');
    }
    return;
  }
  selectableNodes.forEach(node => {
    const option = document.createElement('option');
    option.value = node.id;
    const sizes = sizeState.sizes.get(node.id)?.outputSizes;
    const sizeLabel = sizes ? sizes.join('+') : '?';
    option.textContent = `${node.id} (${sizeLabel})`;
    graphOutputSimpleNode.appendChild(option);
  });
  const state = resolveSimpleOutputState(spec);
  const hasNode = state.nodeId && selectableNodes.some(node => node.id === state.nodeId);
  const fallbackId = hasNode ? (state.nodeId as string) : selectableNodes[0]?.id ?? '';
  graphOutputSimpleNode.value = fallbackId;
  graphOutputSimpleSplit.checked = state.mode === 'simple' && state.split;
  graphOutputSimpleNode.disabled = !selectableNodes.length;
  graphOutputSimpleSplit.disabled = !selectableNodes.length;
  updateSimpleOutputHint(state, sizeState, ports);
}


/**
 * Update the simple output hint text based on current selection.
 * @param state - Current simple output selector state.
 * @param sizeState - Inferred size state for the graph.
 */
export function updateSimpleOutputHint(state: SimpleOutputState, sizeState: GraphSizeState, ports: GraphSettingsPort): void {
  const graphOutputSimpleHint = ports.graphOutputSimpleHint;
  if (!graphOutputSimpleHint) return;
  graphOutputSimpleHint.classList.remove('error');
  if (state.mode === 'custom') {
    graphOutputSimpleHint.textContent =
      'Custom outputs active. Choosing a node below will replace outputs.';
    return;
  }
  const sizes = sizeState.sizes.get(state.nodeId)?.outputSizes;
  if (!sizes) {
    graphOutputSimpleHint.textContent = 'Output sizes unresolved. Check wiring first.';
    graphOutputSimpleHint.classList.add('error');
    return;
  }
  if (!state.split) {
    if (sizes.length !== 1) {
      graphOutputSimpleHint.textContent = 'Selected node has multiple ports. Enable split or use advanced outputs.';
      graphOutputSimpleHint.classList.add('error');
      return;
    }
    if (sizes[0] !== ports.brainOutputSize) {
      graphOutputSimpleHint.textContent = `Output size must be ${ports.brainOutputSize}.`;
      graphOutputSimpleHint.classList.add('error');
      return;
    }
    graphOutputSimpleHint.textContent = 'Outputs map to turn + boost.';
    return;
  }
  if (sizes.length < 2) {
    graphOutputSimpleHint.textContent = 'Split outputs require at least two ports.';
    graphOutputSimpleHint.classList.add('error');
    return;
  }
  const total = (sizes[0] ?? 0) + (sizes[1] ?? 0);
  if (total !== ports.brainOutputSize) {
    graphOutputSimpleHint.textContent = `Split ports 0+1 must sum to ${ports.brainOutputSize}.`;
    graphOutputSimpleHint.classList.add('error');
    return;
  }
  graphOutputSimpleHint.textContent = 'Port 0 feeds turn, port 1 feeds boost.';
}


/**
 * Apply the simple output selector values to the current draft.
 */
export function applySimpleOutputSelection(ports: GraphSettingsPort): void {
  const graphOutputSimpleNode = ports.graphOutputSimpleNode;
  const graphOutputSimpleSplit = ports.graphOutputSimpleSplit;
  if (!graphOutputSimpleNode || !graphOutputSimpleSplit) return;
  const draft = ports.ensureGraphDraft();
  const nodeId = graphOutputSimpleNode.value;
  if (!nodeId) return;
  if (graphOutputSimpleSplit.checked) {
    draft.outputs = [{ nodeId, port: 0 }, { nodeId, port: 1 }];
  } else {
    draft.outputs = [{ nodeId }];
  }
  renderGraphEditor(ports);
  ports.setGraphSpecStatus('Outputs updated. Apply graph to use it.');
}


/**
 * Render the graph editor form controls for nodes, edges, and outputs.
 */
export function renderGraphEditor(ports: GraphSettingsPort): void {
  const graphNodes = ports.graphNodes;
  const graphEdges = ports.graphEdges;
  const graphOutputs = ports.graphOutputs;
  if (!graphNodes || !graphEdges || !graphOutputs) return;
  const spec = ports.ensureGraphDraft();
  spec.nodes.forEach(node => {
    if (node.type === 'Input') node.outputSize = ports.brainInputSize;
  });
  const sizeState = ports.updateGraphSizeState(spec);
  ports.applyGraphSizeInference(spec, sizeState);
  ports.syncGraphSelection(spec);
  const nodeIds = spec.nodes.map(node => node.id);

  const makeField = (labelText: string, input: HTMLInputElement | HTMLSelectElement): HTMLDivElement => {
    const field = document.createElement('div');
    field.className = 'graph-field';
    const label = document.createElement('label');
    label.textContent = labelText;
    field.appendChild(label);
    field.appendChild(input);
    return field;
  };

  const makeNumberInput = (value: number, min?: number): HTMLInputElement => {
    const input = document.createElement('input');
    input.type = 'number';
    input.value = Number.isFinite(value) ? String(value) : '';
    if (min != null) input.min = String(min);
    return input;
  };

  /**
   * Build a read-only number input for inferred sizes.
   * @param value - Value to show, or null to leave blank.
   * @param placeholder - Placeholder when size is unresolved.
   * @returns Disabled number input element.
   */
  const makeReadOnlyNumberInput = (value: number | null, placeholder = 'auto'): HTMLInputElement => {
    const input = document.createElement('input');
    input.type = 'number';
    input.value = value != null && Number.isFinite(value) ? String(value) : '';
    input.placeholder = placeholder;
    input.disabled = true;
    return input;
  };

  const parseNumberList = (value: string): number[] => {
    return value
      .split(',')
      .map(part => Number(part.trim()))
      .filter(num => Number.isFinite(num) && num > 0);
  };

  graphNodes.innerHTML = '';
  spec.nodes.forEach((node, index) => {
    const row = document.createElement('div');
    row.className = 'graph-row';

    const idInput = document.createElement('input');
    idInput.type = 'text';
    idInput.value = node.id;
    idInput.addEventListener('change', () => {
      const nextId = idInput.value.trim();
      if (!nextId) {
        idInput.value = node.id;
        ports.setGraphSpecStatus('Node id cannot be empty.', true);
        return;
      }
      if (nextId !== node.id && spec.nodes.some((n, idx) => idx !== index && n.id === nextId)) {
        idInput.value = node.id;
        ports.setGraphSpecStatus('Node id must be unique.', true);
        return;
      }
      if (nextId === node.id) return;
      const oldId = node.id;
      spec.nodes[index] = { ...node, id: nextId };
      spec.edges.forEach(edge => {
        if (edge.from === oldId) edge.from = nextId;
        if (edge.to === oldId) edge.to = nextId;
      });
      spec.outputs.forEach(output => {
        if (output.nodeId === oldId) output.nodeId = nextId;
      });
      if (ports.graphLayoutOverrides.has(oldId)) {
        const override = ports.graphLayoutOverrides.get(oldId)!;
        ports.graphLayoutOverrides.delete(oldId);
        ports.graphLayoutOverrides.set(nextId, override);
      }
      if (ports.graphSelectedNodeId === oldId) ports.graphSelectedNodeId = nextId;
      renderGraphEditor(ports);
      ports.setGraphSpecStatus('Node id updated. Apply graph to use it.');
    });
    row.appendChild(makeField('Id', idInput));

    const typeSelect = document.createElement('select');
    const availableTypes: GraphNodeType[] =
      node.type === 'Input'
        ? ['Input']
        : ['Dense', 'MLP', 'GRU', 'LSTM', 'RRU', 'Split', 'Concat'];
    availableTypes.forEach(type => {
      const option = document.createElement('option');
      option.value = type;
      option.textContent = type;
      typeSelect.appendChild(option);
    });
    typeSelect.value = node.type;
    typeSelect.addEventListener('change', () => {
      const nextType = typeSelect.value as GraphNodeType;
      spec.nodes[index] = ports.createNode(nextType, node.id);
      renderGraphEditor(ports);
      ports.setGraphSpecStatus('Node type updated. Apply graph to use it.');
    });
    row.appendChild(makeField('Type', typeSelect));

    if (node.type === 'Input') {
      const out = makeNumberInput(ports.brainInputSize, 1);
      out.disabled = true;
      row.appendChild(makeField('Output', out));
    }
    if (node.type === 'Dense' || node.type === 'MLP') {
      const inferred = sizeState.sizes.get(node.id)?.inputSize ?? null;
      row.appendChild(makeField('Input', makeReadOnlyNumberInput(inferred)));

      const output = makeNumberInput(node.outputSize, 1);
      output.addEventListener('input', () => {
        const next = Number(output.value);
        if (!Number.isFinite(next)) return;
        node.outputSize = next;
        ports.setGraphSpecStatus('Updated node sizes. Apply graph to use it.');
      });
      row.appendChild(makeField('Output', output));
    }
    if (node.type === 'MLP') {
      const hidden = document.createElement('input');
      hidden.type = 'text';
      hidden.value = (node.hiddenSizes ?? []).join(', ');
      hidden.placeholder = '16, 16';
      hidden.addEventListener('change', () => {
        node.hiddenSizes = parseNumberList(hidden.value);
        ports.setGraphSpecStatus('Updated hidden sizes. Apply graph to use it.');
      });
      row.appendChild(makeField('Hidden', hidden));
    }
    if (node.type === 'GRU' || node.type === 'LSTM' || node.type === 'RRU') {
      const inferred = sizeState.sizes.get(node.id)?.inputSize ?? null;
      row.appendChild(makeField('Input', makeReadOnlyNumberInput(inferred)));

      const hidden = makeNumberInput(node.hiddenSize, 1);
      hidden.addEventListener('input', () => {
        const next = Number(hidden.value);
        if (!Number.isFinite(next)) return;
        node.hiddenSize = next;
        ports.setGraphSpecStatus('Updated hidden size. Apply graph to use it.');
      });
      row.appendChild(makeField('Hidden', hidden));
    }
    if (node.type === 'Split') {
      const sizes = document.createElement('input');
      sizes.type = 'text';
      sizes.value = node.outputSizes.join(', ');
      sizes.placeholder = '8, 8';
      sizes.addEventListener('change', () => {
        const next = parseNumberList(sizes.value);
        if (!next.length) return;
        node.outputSizes = next;
        ports.setGraphSpecStatus('Updated split sizes. Apply graph to use it.');
      });
      row.appendChild(makeField('Outputs', sizes));
    }
    if (node.type === 'Concat') {
      const outSize = sizeState.sizes.get(node.id)?.outputSizes?.[0] ?? null;
      row.appendChild(makeField('Output', makeReadOnlyNumberInput(outSize)));
    }

    if (node.type !== 'Input') {
      const remove = document.createElement('button');
      remove.className = 'btn small';
      remove.textContent = 'Remove';
      remove.addEventListener('click', () => {
        const removedId = node.id;
        spec.nodes.splice(index, 1);
        spec.edges = spec.edges.filter(edge => edge.from !== removedId && edge.to !== removedId);
        spec.outputs = spec.outputs.filter(out => out.nodeId !== removedId);
        ports.graphLayoutOverrides.delete(removedId);
        if (ports.graphSelectedNodeId === removedId) ports.graphSelectedNodeId = null;
        renderGraphEditor(ports);
        ports.setGraphSpecStatus('Node removed. Apply graph to use it.');
      });
      row.appendChild(remove);
    }

    graphNodes.appendChild(row);
  });

  graphEdges.innerHTML = '';
  spec.edges.forEach((edge, index) => {
    const row = document.createElement('div');
    row.className = 'graph-row';

    const fromSelect = document.createElement('select');
    nodeIds.forEach(id => {
      const opt = document.createElement('option');
      opt.value = id;
      opt.textContent = id;
      fromSelect.appendChild(opt);
    });
    fromSelect.value = edge.from;
    fromSelect.addEventListener('change', () => {
      edge.from = fromSelect.value;
      ports.setGraphSpecStatus('Edge updated. Apply graph to use it.');
    });
    row.appendChild(makeField('From', fromSelect));

    const fromPort = makeNumberInput(edge.fromPort ?? 0, 0);
    fromPort.value = edge.fromPort == null ? '' : String(edge.fromPort);
    fromPort.addEventListener('change', () => {
      const value = fromPort.value.trim();
      if (value === '') {
        if ('fromPort' in edge) delete edge.fromPort;
      } else {
        edge.fromPort = Number(value);
      }
      ports.setGraphSpecStatus('Edge updated. Apply graph to use it.');
    });
    row.appendChild(makeField('From port', fromPort));

    const toSelect = document.createElement('select');
    nodeIds.forEach(id => {
      const opt = document.createElement('option');
      opt.value = id;
      opt.textContent = id;
      toSelect.appendChild(opt);
    });
    toSelect.value = edge.to;
    toSelect.addEventListener('change', () => {
      edge.to = toSelect.value;
      ports.setGraphSpecStatus('Edge updated. Apply graph to use it.');
    });
    row.appendChild(makeField('To', toSelect));

    const toPort = makeNumberInput(edge.toPort ?? 0, 0);
    toPort.value = edge.toPort == null ? '' : String(edge.toPort);
    toPort.addEventListener('change', () => {
      const value = toPort.value.trim();
      if (value === '') {
        if ('toPort' in edge) delete edge.toPort;
      } else {
        edge.toPort = Number(value);
      }
      ports.setGraphSpecStatus('Edge updated. Apply graph to use it.');
    });
    row.appendChild(makeField('To port', toPort));

    const remove = document.createElement('button');
    remove.className = 'btn small';
    remove.textContent = 'Remove';
    remove.addEventListener('click', () => {
      spec.edges.splice(index, 1);
      renderGraphEditor(ports);
      ports.setGraphSpecStatus('Edge removed. Apply graph to use it.');
    });
    row.appendChild(remove);

    graphEdges.appendChild(row);
  });

  renderSimpleOutputUi(spec, sizeState, ports);
  graphOutputs.innerHTML = '';
  spec.outputs.forEach((output, index) => {
    const row = document.createElement('div');
    row.className = 'graph-row';

    const nodeSelect = document.createElement('select');
    nodeIds.forEach(id => {
      const opt = document.createElement('option');
      opt.value = id;
      opt.textContent = id;
      nodeSelect.appendChild(opt);
    });
    nodeSelect.value = output.nodeId;
    nodeSelect.addEventListener('change', () => {
      output.nodeId = nodeSelect.value;
      ports.setGraphSpecStatus('Output updated. Apply graph to use it.');
    });
    row.appendChild(makeField('Node', nodeSelect));

    const portInput = makeNumberInput(output.port ?? 0, 0);
    portInput.value = output.port == null ? '' : String(output.port);
    portInput.addEventListener('change', () => {
      const value = portInput.value.trim();
      if (value === '') {
        if ('port' in output) delete output.port;
      } else {
        output.port = Number(value);
      }
      ports.setGraphSpecStatus('Output updated. Apply graph to use it.');
    });
    row.appendChild(makeField('Port', portInput));

    const remove = document.createElement('button');
    remove.className = 'btn small';
    remove.textContent = 'Remove';
    remove.addEventListener('click', () => {
      spec.outputs.splice(index, 1);
      renderGraphEditor(ports);
      ports.setGraphSpecStatus('Output removed. Apply graph to use it.');
    });
    row.appendChild(remove);

    graphOutputs.appendChild(row);
  });

  ports.renderGraphDiagram(spec);
  ports.renderGraphInspector(spec);
}
