import type { GraphSizeState } from './brains/graph/editor.ts';
import type { GraphNodeSpec, GraphNodeType, GraphSpec } from './brains/graph/schema.ts';

/** Live view of the existing editor state; this module owns no draft or session. */
export interface GraphInspectorPort {
  readonly graphDiagramInspector: HTMLDivElement | null;
  readonly graphSizeState: GraphSizeState | null;
  readonly updateGraphSizeState: (spec: GraphSpec) => GraphSizeState;
  graphSelectedNodeId: string | null;
  readonly setGraphSpecStatus: (text: string, isError?: boolean) => void;
  readonly graphLayoutOverrides: Map<string, { x: number; y: number; }>;
  readonly renderGraphEditor: () => void;
  readonly setGraphSelection: (next: { nodeId?: string | null; edgeIndex?: number | null; outputIndex?: number | null; }) => void;
  readonly graphSelectedEdgeIndex: number | null;
  readonly graphSelectedOutputIndex: number | null;
  readonly brainInputSize: number;
  readonly createNode: (type: GraphNodeType, id: string) => GraphNodeSpec;
}

/**
 * Render the graph inspector panel for the current selection.
 * @param ports - Live access to the existing editor owner and its callbacks.
 * @param spec - Graph spec to inspect and edit.
 */
export function renderGraphInspectorView(spec: GraphSpec, ports: GraphInspectorPort): void {
  const graphDiagramInspector = ports.graphDiagramInspector;
  if (!graphDiagramInspector) return;
  graphDiagramInspector.innerHTML = '';
  const sizeState = ports.graphSizeState ?? ports.updateGraphSizeState(spec);

  const makeRow = (labelText: string, input: HTMLElement): HTMLDivElement => {
    const row = document.createElement('div');
    row.className = 'graph-inspector-row';
    const label = document.createElement('label');
    label.textContent = labelText;
    input.setAttribute('aria-label', labelText);
    row.appendChild(label);
    row.appendChild(input);
    return row;
  };

  const makeNumberInput = (value: number): HTMLInputElement => {
    const input = document.createElement('input');
    input.type = 'number';
    input.value = Number.isFinite(value) ? String(value) : '';
    return input;
  };

  /**
   * Build a read-only number input for inferred sizes.
   * @param value - Value to show, or null to leave blank.
   * @param placeholder - Placeholder text for unresolved sizes.
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

  const nodeIds = spec.nodes.map(node => node.id);

  const addActions = (actions: Array<{ label: string; onClick: () => void }>): void => {
    const row = document.createElement('div');
    row.className = 'graph-inspector-actions';
    actions.forEach(action => {
      const btn = document.createElement('button');
      btn.className = 'btn small';
      btn.type = 'button';
      btn.textContent = action.label;
      btn.addEventListener('click', action.onClick);
      row.appendChild(btn);
    });
    graphDiagramInspector.appendChild(row);
  };

  if (ports.graphSelectedNodeId) {
    const nodeIndex = spec.nodes.findIndex(node => node.id === ports.graphSelectedNodeId);
    const node = nodeIndex >= 0 ? spec.nodes[nodeIndex] : null;
    if (!node) return;
    graphDiagramInspector.classList.remove('hidden');

    const title = document.createElement('div');
    title.textContent = `Node: ${node.id}`;
    graphDiagramInspector.appendChild(title);

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
      if (nextId !== node.id && spec.nodes.some(n => n.id === nextId)) {
        idInput.value = node.id;
        ports.setGraphSpecStatus('Node id must be unique.', true);
        return;
      }
      if (nextId === node.id) return;
      const oldId = node.id;
      node.id = nextId;
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
      ports.renderGraphEditor();
      ports.setGraphSpecStatus('Node id updated. Stage graph to use it.');
    });
    graphDiagramInspector.appendChild(makeRow('Id', idInput));

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
      spec.nodes[nodeIndex] = ports.createNode(nextType, node.id);
      ports.renderGraphEditor();
      ports.setGraphSpecStatus('Node type updated. Stage graph to use it.');
    });
    graphDiagramInspector.appendChild(makeRow('Type', typeSelect));

    if (node.type === 'Input') {
      graphDiagramInspector.appendChild(makeRow('Output', makeReadOnlyNumberInput(ports.brainInputSize, 'fixed')));
    }
    if (node.type === 'Dense' || node.type === 'MLP') {
      const inferred = sizeState.sizes.get(node.id)?.inputSize ?? null;
      graphDiagramInspector.appendChild(makeRow('Input', makeReadOnlyNumberInput(inferred)));

      const output = makeNumberInput(node.outputSize);
      output.addEventListener('input', () => {
        const next = Number(output.value);
        if (!Number.isFinite(next)) return;
        node.outputSize = next;
        ports.setGraphSpecStatus('Updated node sizes. Stage graph to use it.');
      });
      graphDiagramInspector.appendChild(makeRow('Output', output));
    }
    if (node.type === 'MLP') {
      const hidden = document.createElement('input');
      hidden.type = 'text';
      hidden.value = (node.hiddenSizes ?? []).join(', ');
      hidden.addEventListener('change', () => {
        node.hiddenSizes = parseNumberList(hidden.value);
        ports.setGraphSpecStatus('Updated hidden sizes. Stage graph to use it.');
      });
      graphDiagramInspector.appendChild(makeRow('Hidden', hidden));
    }
    if (node.type === 'GRU' || node.type === 'LSTM' || node.type === 'RRU') {
      const inferred = sizeState.sizes.get(node.id)?.inputSize ?? null;
      graphDiagramInspector.appendChild(makeRow('Input', makeReadOnlyNumberInput(inferred)));

      const hidden = makeNumberInput(node.hiddenSize);
      hidden.addEventListener('input', () => {
        const next = Number(hidden.value);
        if (!Number.isFinite(next)) return;
        node.hiddenSize = next;
        ports.setGraphSpecStatus('Updated hidden size. Stage graph to use it.');
      });
      graphDiagramInspector.appendChild(makeRow('Hidden', hidden));
    }
    if (node.type === 'Split') {
      const sizes = document.createElement('input');
      sizes.type = 'text';
      sizes.value = node.outputSizes.join(', ');
      sizes.addEventListener('change', () => {
        const next = parseNumberList(sizes.value);
        if (!next.length) return;
        node.outputSizes = next;
        ports.setGraphSpecStatus('Updated split sizes. Stage graph to use it.');
      });
      graphDiagramInspector.appendChild(makeRow('Outputs', sizes));
    }
    if (node.type === 'Concat') {
      const outSize = sizeState.sizes.get(node.id)?.outputSizes?.[0] ?? null;
      graphDiagramInspector.appendChild(makeRow('Output', makeReadOnlyNumberInput(outSize)));
    }

    addActions([
      {
        label: 'Delete',
        onClick: () => {
          if (node.type === 'Input') {
            ports.setGraphSpecStatus('Input node cannot be removed.', true);
            return;
          }
          const removedId = node.id;
          spec.nodes.splice(nodeIndex, 1);
          spec.edges = spec.edges.filter(edge => edge.from !== removedId && edge.to !== removedId);
          spec.outputs = spec.outputs.filter(out => out.nodeId !== removedId);
          ports.graphLayoutOverrides.delete(removedId);
          ports.setGraphSelection({});
          ports.renderGraphEditor();
          ports.setGraphSpecStatus('Node removed. Stage graph to use it.');
        }
      }
    ]);
    return;
  }

  if (ports.graphSelectedEdgeIndex != null) {
    const edge = spec.edges[ports.graphSelectedEdgeIndex];
    if (!edge) return;
    graphDiagramInspector.classList.remove('hidden');
    const title = document.createElement('div');
    title.textContent = 'Edge';
    graphDiagramInspector.appendChild(title);

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
      ports.renderGraphEditor();
      ports.setGraphSpecStatus('Edge updated. Stage graph to use it.');
    });
    graphDiagramInspector.appendChild(makeRow('From', fromSelect));

    const fromPort = makeNumberInput(edge.fromPort ?? 0);
    fromPort.value = edge.fromPort == null ? '' : String(edge.fromPort);
    fromPort.addEventListener('change', () => {
      const value = fromPort.value.trim();
      if (value === '') {
        if ('fromPort' in edge) delete edge.fromPort;
      } else {
        edge.fromPort = Number(value);
      }
      ports.setGraphSpecStatus('Edge updated. Stage graph to use it.');
    });
    graphDiagramInspector.appendChild(makeRow('From port', fromPort));

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
      ports.renderGraphEditor();
      ports.setGraphSpecStatus('Edge updated. Stage graph to use it.');
    });
    graphDiagramInspector.appendChild(makeRow('To', toSelect));

    const toPort = makeNumberInput(edge.toPort ?? 0);
    toPort.value = edge.toPort == null ? '' : String(edge.toPort);
    toPort.addEventListener('change', () => {
      const value = toPort.value.trim();
      if (value === '') {
        if ('toPort' in edge) delete edge.toPort;
      } else {
        edge.toPort = Number(value);
      }
      ports.setGraphSpecStatus('Edge updated. Stage graph to use it.');
    });
    graphDiagramInspector.appendChild(makeRow('To port', toPort));

    const edgeIndex = ports.graphSelectedEdgeIndex;
    addActions([
      {
        label: 'Delete',
        onClick: () => {
          if (edgeIndex == null) return;
          spec.edges.splice(edgeIndex, 1);
          ports.setGraphSelection({});
          ports.renderGraphEditor();
          ports.setGraphSpecStatus('Edge removed. Stage graph to use it.');
        }
      }
    ]);
    return;
  }

  if (ports.graphSelectedOutputIndex != null) {
    const output = spec.outputs[ports.graphSelectedOutputIndex];
    if (!output) return;
    graphDiagramInspector.classList.remove('hidden');
    const title = document.createElement('div');
    title.textContent = `Output ${ports.graphSelectedOutputIndex + 1}`;
    graphDiagramInspector.appendChild(title);

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
      ports.renderGraphEditor();
      ports.setGraphSpecStatus('Output updated. Stage graph to use it.');
    });
    graphDiagramInspector.appendChild(makeRow('Node', nodeSelect));

    const portInput = makeNumberInput(output.port ?? 0);
    portInput.value = output.port == null ? '' : String(output.port);
    portInput.addEventListener('change', () => {
      const value = portInput.value.trim();
      if (value === '') {
        if ('port' in output) delete output.port;
      } else {
        output.port = Number(value);
      }
      ports.setGraphSpecStatus('Output updated. Stage graph to use it.');
    });
    graphDiagramInspector.appendChild(makeRow('Port', portInput));

    const outputIndex = ports.graphSelectedOutputIndex;
    addActions([
      {
        label: 'Delete',
        onClick: () => {
          if (outputIndex == null) return;
          spec.outputs.splice(outputIndex, 1);
          ports.setGraphSelection({});
          ports.renderGraphEditor();
          ports.setGraphSpecStatus('Output removed. Stage graph to use it.');
        }
      }
    ]);
    return;
  }

  graphDiagramInspector.classList.add('hidden');
}
