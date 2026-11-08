import type { GraphSizeState } from './brains/graph/editor.ts';
import type { GraphNodeSpec, GraphSpec } from './brains/graph/schema.ts';
import { clamp } from './utils.ts';

/** Diagram node width in pixels. */
export const DIAGRAM_NODE_WIDTH = 140;

/** Diagram node height in pixels. */
export const DIAGRAM_NODE_HEIGHT = 44;

/** Diagram node derived from graph spec for SVG rendering. */
type DiagramNode = {
  id: string;
  label: string;
  type: string;
  layer: number;
  outputIndex?: number;
};

/** Diagram edge derived from graph spec for SVG rendering. */
type DiagramEdge = {
  from: string;
  to: string;
  fromPort?: number;
  toPort?: number;
  edgeIndex?: number;
  outputIndex?: number;
};

/** Live view of the existing editor state; this module owns no draft or session. */
export interface GraphDiagramPort {
  readonly graphDiagram: SVGSVGElement | null;
  readonly graphDiagramNodeRects: Map<string, { x: number; y: number; width: number; height: number; }>;
  readonly graphSizeState: GraphSizeState | null;
  readonly updateGraphSizeState: (spec: GraphSpec) => GraphSizeState;
  readonly graphLayoutOverrides: Map<string, { x: number; y: number; }>;
  readonly graphSelectedEdgeIndex: number | null;
  readonly setGraphSelection: (next: { nodeId?: string | null; edgeIndex?: number | null; outputIndex?: number | null; }) => void;
  graphConnectLine: SVGPathElement | null;
  readonly graphSelectedOutputIndex: number | null;
  readonly graphSelectedNodeId: string | null;
  readonly beginGraphConnectDrag: (fromId: string, event: PointerEvent) => void;
  readonly getSvgPoint: (evt: PointerEvent | MouseEvent) => { x: number; y: number; } | null;
  graphDragState: { id: string; offsetX: number; offsetY: number; } | null;
  readonly graphConnectDrag: { fromId: string; pointerId: number; } | null;
  readonly graphPointerPos: { x: number; y: number; } | null;
  readonly updateGraphConnectDrag: (point: { x: number; y: number; }) => void;
}

/**
 * Render the graph diagram SVG from the spec.
 * @param ports - Live access to the existing editor owner and its callbacks.
 * @param spec - Graph spec to render.
 */
export function renderGraphDiagramView(spec: GraphSpec, ports: GraphDiagramPort): void {
  const graphDiagram = ports.graphDiagram;
  if (!graphDiagram) return;
  graphDiagram.innerHTML = '';
  ports.graphDiagramNodeRects.clear();
  if (!spec.nodes.length) return;

  const svgNs = 'http://www.w3.org/2000/svg';
  const sizeState = ports.graphSizeState ?? ports.updateGraphSizeState(spec);
  const nodeErrors = sizeState.nodeErrors;
  const nodeById = new Map<string, GraphNodeSpec>();
  spec.nodes.forEach(node => nodeById.set(node.id, node));

  const edges = spec.edges
    .map((edge, index) => ({ edge, index }))
    .filter(({ edge }) => nodeById.has(edge.from) && nodeById.has(edge.to));
  const outgoing = new Map<string, string[]>();
  const incoming = new Map<string, string[]>();
  spec.nodes.forEach(node => {
    outgoing.set(node.id, []);
    incoming.set(node.id, []);
  });
  edges.forEach(({ edge }) => {
    outgoing.get(edge.from)?.push(edge.to);
    incoming.get(edge.to)?.push(edge.from);
  });

  const layerById = new Map<string, number>();
  const roots = spec.nodes
    .filter(node => node.type === 'Input' || (incoming.get(node.id)?.length ?? 0) === 0)
    .map(node => node.id)
    .sort((a, b) => a.localeCompare(b));
  roots.forEach(id => layerById.set(id, 0));
  const queue = [...roots];
  while (queue.length) {
    const id = queue.shift();
    if (!id) break;
    const baseLayer = layerById.get(id) ?? 0;
    for (const next of outgoing.get(id) ?? []) {
      const nextLayer = baseLayer + 1;
      const current = layerById.get(next);
      if (current == null || nextLayer > current) {
        layerById.set(next, nextLayer);
        queue.push(next);
      }
    }
  }
  let maxLayer = Math.max(0, ...Array.from(layerById.values()));
  spec.nodes.forEach(node => {
    if (!layerById.has(node.id)) {
      maxLayer += 1;
      layerById.set(node.id, maxLayer);
    }
  });

  const outputNodes = spec.outputs.map((output, index) => ({
    id: `__out-${index + 1}`,
    label: `Out ${index + 1}`,
    type: 'Output',
    layer: maxLayer + 1,
    outputIndex: index,
    fromId: output.nodeId,
    port: output.port
  }));

  const diagramNodes: DiagramNode[] = [
    ...spec.nodes.map(node => ({
      id: node.id,
      label: node.id,
      type: node.type,
      layer: layerById.get(node.id) ?? 0
    })),
    ...outputNodes.map(node => ({
      id: node.id,
      label: node.label,
      type: node.type,
      layer: node.layer,
      outputIndex: node.outputIndex
    }))
  ];

  const diagramEdges: DiagramEdge[] = [
    ...edges.map(({ edge, index }) => {
      const entry: DiagramEdge = { from: edge.from, to: edge.to, edgeIndex: index };
      if (edge.fromPort != null) entry.fromPort = edge.fromPort;
      if (edge.toPort != null) entry.toPort = edge.toPort;
      return entry;
    }),
    ...outputNodes.map(node => {
      const entry: DiagramEdge = { from: node.fromId, to: node.id, outputIndex: node.outputIndex };
      if (node.port != null) entry.fromPort = node.port;
      return entry;
    })
  ];

  const layerGroups = new Map<number, DiagramNode[]>();
  diagramNodes.forEach(node => {
    const group = layerGroups.get(node.layer) ?? [];
    group.push(node);
    layerGroups.set(node.layer, group);
  });

  const sortedLayers = Array.from(layerGroups.keys()).sort((a, b) => a - b);
  const maxPerLayer = Math.max(1, ...sortedLayers.map(layer => layerGroups.get(layer)?.length ?? 0));

  const nodeWidth = DIAGRAM_NODE_WIDTH;
  const nodeHeight = DIAGRAM_NODE_HEIGHT;
  const layerGap = 90;
  const rowGap = 18;
  const padding = 20;
  const totalLayers = sortedLayers.length;
  const width = padding * 2 + totalLayers * nodeWidth + Math.max(0, totalLayers - 1) * layerGap;
  const height = padding * 2 + maxPerLayer * nodeHeight + Math.max(0, maxPerLayer - 1) * rowGap;
  graphDiagram.setAttribute('viewBox', `0 0 ${width} ${height}`);

  const positions = new Map<string, { x: number; y: number }>();
  sortedLayers.forEach((layer, index) => {
    const nodes = layerGroups.get(layer) ?? [];
    nodes.sort((a, b) => a.id.localeCompare(b.id));
    const totalHeight = nodes.length * nodeHeight + Math.max(0, nodes.length - 1) * rowGap;
    const startY = padding + (height - padding * 2 - totalHeight) / 2;
    const x = padding + index * (nodeWidth + layerGap);
    nodes.forEach((node, idx) => {
      const y = startY + idx * (nodeHeight + rowGap);
      positions.set(node.id, { x, y });
    });
  });

  diagramNodes.forEach(node => {
    if (node.id.startsWith('__out-')) return;
    const override = ports.graphLayoutOverrides.get(node.id);
    if (!override) return;
    const x = clamp(override.x, padding, width - padding - nodeWidth);
    const y = clamp(override.y, padding, height - padding - nodeHeight);
    positions.set(node.id, { x, y });
  });

  const edgesGroup = document.createElementNS(svgNs, 'g');
  diagramEdges.forEach(edge => {
    const from = positions.get(edge.from);
    const to = positions.get(edge.to);
    if (!from || !to) return;
    const x1 = from.x + nodeWidth;
    const y1 = from.y + nodeHeight / 2;
    const x2 = to.x;
    const y2 = to.y + nodeHeight / 2;
    const curve = Math.max(20, (x2 - x1) * 0.35);
    const path = document.createElementNS(svgNs, 'path');
    const isSelected = edge.edgeIndex != null && edge.edgeIndex === ports.graphSelectedEdgeIndex;
    path.setAttribute('class', isSelected ? 'graph-diagram-edge selected' : 'graph-diagram-edge');
    path.setAttribute('d', `M ${x1} ${y1} C ${x1 + curve} ${y1} ${x2 - curve} ${y2} ${x2} ${y2}`);
    edgesGroup.appendChild(path);
    if (edge.edgeIndex != null) {
      const hit = document.createElementNS(svgNs, 'path');
      hit.setAttribute('class', 'graph-diagram-edge-hit');
      hit.setAttribute('d', path.getAttribute('d') ?? '');
      hit.addEventListener('click', (event) => {
        event.stopPropagation();
        ports.setGraphSelection({ edgeIndex: edge.edgeIndex ?? null });
      });
      edgesGroup.appendChild(hit);
    }

    const addPortLabel = (value: number | undefined, x: number, y: number) => {
      if (value == null) return;
      const label = document.createElementNS(svgNs, 'text');
      label.setAttribute('class', 'graph-diagram-port');
      label.setAttribute('x', String(x));
      label.setAttribute('y', String(y));
      label.textContent = `p${value}`;
      edgesGroup.appendChild(label);
    };

    addPortLabel(edge.fromPort, x1 + 6, y1 - 6);
    addPortLabel(edge.toPort, x2 - 18, y2 - 6);
  });
  const connectPath = document.createElementNS(svgNs, 'path');
  connectPath.setAttribute('class', 'graph-diagram-edge ghost');
  ports.graphConnectLine = connectPath;
  edgesGroup.appendChild(connectPath);
  graphDiagram.appendChild(edgesGroup);

  const nodesGroup = document.createElementNS(svgNs, 'g');
  diagramNodes.forEach(node => {
    const pos = positions.get(node.id);
    if (!pos) return;
    const group = document.createElementNS(svgNs, 'g');
    group.setAttribute('data-node-id', node.id);
    if (node.outputIndex != null) {
      group.setAttribute('data-output-index', String(node.outputIndex));
    }
    group.setAttribute('cursor', 'pointer');

    const isSelected =
      node.outputIndex != null
        ? ports.graphSelectedOutputIndex === node.outputIndex
        : ports.graphSelectedNodeId === node.id;
    const isOutput = node.outputIndex != null || node.id.startsWith('__out-');
    if (!isOutput) {
      ports.graphDiagramNodeRects.set(node.id, { x: pos.x, y: pos.y, width: nodeWidth, height: nodeHeight });
    }
    const sizeInfo = sizeState.sizes.get(node.id);
    const hasError = !!nodeErrors.get(node.id)?.length;
    const isPending =
      !hasError &&
      !isOutput &&
      (node.type === 'Concat'
        ? !sizeInfo?.outputSizes
        : node.type !== 'Input' && sizeInfo?.inputSize == null);
    const rect = document.createElementNS(svgNs, 'rect');
    rect.setAttribute('x', String(pos.x));
    rect.setAttribute('y', String(pos.y));
    rect.setAttribute('rx', '10');
    rect.setAttribute('ry', '10');
    rect.setAttribute('width', String(nodeWidth));
    rect.setAttribute('height', String(nodeHeight));
    rect.setAttribute(
      'class',
      `graph-diagram-node graph-diagram-${node.type.toLowerCase()}${isSelected ? ' selected' : ''}${hasError ? ' error' : ''}${isPending ? ' pending' : ''}`
    );
    group.appendChild(rect);

    if (!isOutput) {
      const handle = document.createElementNS(svgNs, 'circle');
      handle.setAttribute('class', 'graph-diagram-handle');
      handle.setAttribute('cx', String(pos.x + nodeWidth));
      handle.setAttribute('cy', String(pos.y + nodeHeight / 2));
      handle.setAttribute('r', '6');
      handle.addEventListener('pointerdown', (event) => {
        event.stopPropagation();
        ports.beginGraphConnectDrag(node.id, event);
      });
      group.appendChild(handle);
    }

    const text = document.createElementNS(svgNs, 'text');
    text.setAttribute('class', 'graph-diagram-label');
    text.setAttribute('x', String(pos.x + nodeWidth / 2));
    text.setAttribute('y', String(pos.y + nodeHeight / 2 - 6));
    const line1 = document.createElementNS(svgNs, 'tspan');
    line1.textContent = node.label;
    line1.setAttribute('x', String(pos.x + nodeWidth / 2));
    const line2 = document.createElementNS(svgNs, 'tspan');
    line2.textContent = node.type;
    line2.setAttribute('class', 'graph-diagram-label-sub');
    line2.setAttribute('x', String(pos.x + nodeWidth / 2));
    line2.setAttribute('dy', '14');
    text.appendChild(line1);
    text.appendChild(line2);
    group.appendChild(text);

    group.addEventListener('pointerdown', (event) => {
      event.stopPropagation();
      if (event.button !== 0) return;
      if (node.outputIndex != null || node.id.startsWith('__out-')) return;
      const point = ports.getSvgPoint(event);
      if (!point) return;
      const current = positions.get(node.id);
      if (!current) return;
      ports.graphDragState = {
        id: node.id,
        offsetX: point.x - current.x,
        offsetY: point.y - current.y
      };
      group.setPointerCapture?.(event.pointerId);
    });

    group.addEventListener('click', (event) => {
      event.stopPropagation();
      if (node.outputIndex != null) {
        ports.setGraphSelection({ outputIndex: node.outputIndex });
        return;
      }
      ports.setGraphSelection({ nodeId: node.id });
    });

    nodesGroup.appendChild(group);
  });
  graphDiagram.appendChild(nodesGroup);
  if (ports.graphConnectDrag && ports.graphPointerPos) {
    ports.updateGraphConnectDrag(ports.graphPointerPos);
  }
}
