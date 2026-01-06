// main.ts
// Entry point for the slither simulation.  Sets up the canvas, UI,
// Entry point for the slither simulation. Sets up the canvas, UI,
// constructs the World and runs the animation loop. All global
// functions and classes defined in other modules must be loaded before
// this script executes.

import { CFG, resetCFGToDefaults } from './config.ts';
import { setupSettingsUI, updateCFGFromUI } from './settings.ts';
import { lerp, setByPath } from './utils.ts';
import { renderWorldStruct } from './render.ts';
// import { World } from './world.ts'; // Logic moved to worker
import { exportJsonToFile, exportToFile, importFromFile } from './storage.ts';
import { hof } from './hallOfFame.ts';
import { BrainViz } from './BrainViz.ts';
import { AdvancedCharts } from './chartUtils.ts';
import { createWsClient, resolveServerUrl, storeServerUrl } from './net/wsClient.ts';
import { validateGraph } from './brains/graph/validate.ts';
import type { GraphEdge, GraphNodeSpec, GraphNodeType, GraphSpec } from './brains/graph/schema.ts';
import type { FrameStats, HallOfFameEntry, VizData, WorkerToMainMessage } from './protocol/messages.ts';
import type { SettingsUpdate } from './protocol/settings.ts';

interface ProxyWorld {
  generation: number;
  population: unknown[];
  snakes: unknown[];
  zoom: number;
  cameraX: number;
  cameraY: number;
  viewMode: string;
  fitnessHistory: FitnessHistoryUiEntry[];
  toggleViewMode: () => void;
  resurrect: (genome: unknown) => void;
}

interface FitnessHistoryUiEntry {
  gen: number;
  avgFitness: number;
  maxFitness: number;
  minFitness: number;
  speciesCount?: number;
  topSpeciesSize?: number;
  avgWeight?: number;
  weightVariance?: number;
}

interface GodModeLogEntry {
  time: number;
  action: string;
  snakeId: number;
  result: string;
}

interface SelectedSnake {
  id: number;
  x: number;
  y: number;
  radius: number;
  skin: number;
}

type ConnectionMode = 'connecting' | 'server' | 'worker';

declare global {
  interface Window {
    ctx: CanvasRenderingContext2D;
    currentWorld: ProxyWorld;
    spawnHoF: (idx: number) => void;
  }
}

// Canvas and HUD
let worker: Worker | null = null;
let wsClient: ReturnType<typeof createWsClient> | null = null;
let connectionMode: ConnectionMode = 'connecting';
let serverUrl = '';
let reconnectDelayMs = 1000;
let reconnectTimer: number | null = null;
let fallbackTimer: number | null = null;
let settingsLocked = true;
let joinPending = false;
let playerSnakeId: number | null = null;
let playerSensorTick = 0;
let playerSensorMeta: { x: number; y: number; dir: number } | null = null;
let pointerWorld: { x: number; y: number } | null = null;
let boostHeld = false;
const GRAPH_SPEC_STORAGE_KEY = 'slither_neuroevo_graph_spec';
let customGraphSpec: GraphSpec | null = null;
let graphDraft: GraphSpec | null = null;
const DIAGRAM_NODE_WIDTH = 140;
const DIAGRAM_NODE_HEIGHT = 44;
const graphLayoutOverrides = new Map<string, { x: number; y: number }>();
let graphConnectMode = false;
let graphConnectFromId: string | null = null;
let graphSelectedNodeId: string | null = null;
let graphSelectedEdgeIndex: number | null = null;
let graphSelectedOutputIndex: number | null = null;
let graphDragState: { id: string; offsetX: number; offsetY: number } | null = null;
let graphPointerPos: { x: number; y: number } | null = null;
const canvas = document.getElementById('c') as HTMLCanvasElement;
// HUD removed, using tab info panels instead
// Expose the rendering context globally so render helpers can draw.
const ctx = canvas.getContext('2d')!;
window.ctx = ctx;
let cssW = 0,
  cssH = 0,
  dpr = 1;

function resize(): void {
  dpr = window.devicePixelRatio || 1;
  cssW = Math.floor(window.innerWidth);
  cssH = Math.floor(window.innerHeight);
  canvas.style.width = cssW + 'px';
  canvas.style.height = cssH + 'px';
  canvas.width = Math.floor(cssW * dpr);
  canvas.height = Math.floor(cssH * dpr);
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  if (worker) {
    worker.postMessage({ type: 'resize', viewW: cssW, viewH: cssH });
  }
}
window.addEventListener('resize', resize);
resize();

// Core UI elements
const elSnakes = document.getElementById('snakes') as HTMLInputElement;
const elSimSpeed = document.getElementById('simSpeed') as HTMLInputElement;
const elLayers = document.getElementById('layers') as HTMLInputElement;
const elN1 = document.getElementById('n1') as HTMLInputElement;
const elN2 = document.getElementById('n2') as HTMLInputElement;
const elN3 = document.getElementById('n3') as HTMLInputElement;
const elN4 = document.getElementById('n4') as HTMLInputElement;
const elN5 = document.getElementById('n5') as HTMLInputElement;
const graphNodes = document.getElementById('graphNodes') as HTMLElement | null;
const graphEdges = document.getElementById('graphEdges') as HTMLElement | null;
const graphOutputs = document.getElementById('graphOutputs') as HTMLElement | null;
const graphNodeAdd = document.getElementById('graphNodeAdd') as HTMLButtonElement | null;
const graphEdgeAdd = document.getElementById('graphEdgeAdd') as HTMLButtonElement | null;
const graphOutputAdd = document.getElementById('graphOutputAdd') as HTMLButtonElement | null;
const graphApply = document.getElementById('graphApply') as HTMLButtonElement | null;
const graphReset = document.getElementById('graphReset') as HTMLButtonElement | null;
const graphPresetList = document.getElementById('graphPresetList') as HTMLElement | null;
const graphTemplateButtons = document.querySelectorAll<HTMLButtonElement>('[data-template]');
const graphPresetName = document.getElementById('graphPresetName') as HTMLInputElement | null;
const graphPresetSave = document.getElementById('graphPresetSave') as HTMLButtonElement | null;
const graphSpecInput = document.getElementById('graphSpecInput') as HTMLTextAreaElement | null;
const graphSpecApply = document.getElementById('graphSpecApply') as HTMLButtonElement | null;
const graphSpecCopy = document.getElementById('graphSpecCopy') as HTMLButtonElement | null;
const graphSpecExport = document.getElementById('graphSpecExport') as HTMLButtonElement | null;
const graphSpecStatus = document.getElementById('graphSpecStatus') as HTMLElement | null;
const graphDiagramWrap = document.getElementById('graphDiagramWrap') as HTMLElement | null;
const graphDiagram = document.getElementById('graphDiagram') as SVGSVGElement | null;
const graphDiagramToggle = document.getElementById('graphDiagramToggle') as HTMLButtonElement | null;
const graphDiagramBackdrop = document.getElementById('graphDiagramBackdrop') as HTMLDivElement | null;
const graphDiagramAddNode = document.getElementById('graphDiagramAddNode') as HTMLButtonElement | null;
const graphDiagramConnect = document.getElementById('graphDiagramConnect') as HTMLButtonElement | null;
const graphDiagramAddOutput = document.getElementById('graphDiagramAddOutput') as HTMLButtonElement | null;
const graphDiagramAuto = document.getElementById('graphDiagramAuto') as HTMLButtonElement | null;
const graphDiagramDelete = document.getElementById('graphDiagramDelete') as HTMLButtonElement | null;
const graphDiagramInspector = document.getElementById('graphDiagramInspector') as HTMLDivElement | null;
const snakesVal = document.getElementById('snakesVal') as HTMLElement;
const simSpeedVal = document.getElementById('simSpeedVal') as HTMLElement;
const layersVal = document.getElementById('layersVal') as HTMLElement;
const n1Val = document.getElementById('n1Val') as HTMLElement;
const n2Val = document.getElementById('n2Val') as HTMLElement;
const n3Val = document.getElementById('n3Val') as HTMLElement;
const n4Val = document.getElementById('n4Val') as HTMLElement;
const n5Val = document.getElementById('n5Val') as HTMLElement;
const btnApply = document.getElementById('apply') as HTMLButtonElement;
const btnDefaults = document.getElementById('defaults') as HTMLButtonElement;
const btnToggle = document.getElementById('toggle') as HTMLButtonElement;
const settingsContainer = document.getElementById('settingsContainer') as HTMLElement;
const connectionStatus = document.getElementById('connectionStatus') as HTMLElement | null;
const joinOverlay = document.getElementById('joinOverlay') as HTMLElement | null;
const joinName = document.getElementById('joinName') as HTMLInputElement | null;
const joinPlay = document.getElementById('joinPlay') as HTMLButtonElement | null;
const joinSpectate = document.getElementById('joinSpectate') as HTMLButtonElement | null;
const joinStatus = document.getElementById('joinStatus') as HTMLElement | null;
const toggleSettingsLock = document.getElementById('toggleSettingsLock') as HTMLButtonElement | null;
const settingsControls = document.getElementById('settingsControls') as HTMLElement | null;
const settingsTab = document.getElementById('tab-settings') as HTMLElement | null;
const settingsLockHint = document.getElementById('settingsLockHint') as HTMLElement | null;

// Tabs and visualizers
const tabBtns = document.querySelectorAll<HTMLButtonElement>('.tab-btn');
const tabContents = document.querySelectorAll<HTMLElement>('.tab-content');
const vizCanvas = document.getElementById('vizCanvas') as HTMLCanvasElement;
const statsCanvas = document.getElementById('statsCanvas') as HTMLCanvasElement;
const ctxViz = vizCanvas.getContext('2d')!;
const ctxStats = statsCanvas.getContext('2d')!;

const brainViz = new BrainViz(0, 0, vizCanvas.width, vizCanvas.height);
let activeTab = 'tab-settings';
let statsView = 'fitness';
tabBtns.forEach(btn => {
  btn.addEventListener('click', () => {
    // UI toggle
    tabBtns.forEach(b => b.classList.remove('active'));
    tabContents.forEach(c => c.classList.remove('active'));
    btn.classList.add('active');
    const tabId = btn.dataset['tab']!;
    const tabEl = document.getElementById(tabId);
    if (tabEl) tabEl.classList.add('active');
    activeTab = tabId;
    if (worker) {
      worker.postMessage({ type: 'viz', enabled: activeTab === 'tab-viz' });
    }
    if (wsClient && wsClient.isConnected()) {
      wsClient.sendViz(activeTab === 'tab-viz');
    }
  });
});

const statsViewBtns = document.querySelectorAll<HTMLButtonElement>('.stats-view-btn');
const statsTitle = document.getElementById('statsTitle') as HTMLElement | null;
const statsSubtitle = document.getElementById('statsSubtitle') as HTMLElement | null;
const statsViewMeta: Record<string, { title: string; subtitle: string }> = {
  fitness: {
    title: 'Fitness History',
    subtitle: 'Population fitness over generations.'
  },
  diversity: {
    title: 'Species Diversity',
    subtitle: 'Genome clustering by weight distance.'
  },
  complexity: {
    title: 'Network Complexity',
    subtitle: 'Average absolute weight and variance.'
  }
};

function setStatsView(view: string): void {
  statsView = view;
  statsViewBtns.forEach(btn => {
    btn.classList.toggle('active', btn.dataset['stats'] === view);
  });
  const meta = statsViewMeta[view];
  if (statsTitle && meta) statsTitle.textContent = meta.title;
  if (statsSubtitle && meta) statsSubtitle.textContent = meta.subtitle;
}

statsViewBtns.forEach(btn => {
  btn.addEventListener('click', () => setStatsView(btn.dataset['stats']!));
});
if (statsViewBtns.length) setStatsView(statsView);
if (joinName) {
  joinName.addEventListener('input', () => updateJoinControls());
  joinName.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') {
      joinPlay?.click();
    }
  });
}
if (joinPlay) {
  joinPlay.addEventListener('click', () => {
    if (!joinName) return;
    const name = joinName.value.trim();
    if (!name) return;
    joinPending = true;
    setJoinStatus('Joining...');
    updateJoinControls();
    wsClient?.sendJoin('player', name);
    wsClient?.sendView({ mode: 'follow', viewW: cssW, viewH: cssH });
  });
}
if (joinSpectate) {
  joinSpectate.addEventListener('click', () => {
    if (!wsClient?.isConnected()) return;
    joinPending = false;
    setJoinStatus('Spectating');
    updateJoinControls();
    wsClient.sendJoin('spectator');
    wsClient.sendView({ mode: 'overview', viewW: cssW, viewH: cssH });
    setJoinOverlayVisible(false);
  });
}
setJoinOverlayVisible(true);
setJoinStatus('Connecting...');

/**
 * Reads the core UI slider values into a settings object for the world.
 */
function readSettingsFromCoreUI(): {
  snakeCount: number;
  simSpeed: number;
  hiddenLayers: number;
  neurons1: number;
  neurons2: number;
  neurons3: number;
  neurons4: number;
  neurons5: number;
} {
  const hiddenLayers = parseInt(elLayers.value, 10);
  return {
    snakeCount: parseInt(elSnakes.value, 10),
    simSpeed: parseFloat(elSimSpeed.value),
    hiddenLayers,
    neurons1: parseInt(elN1.value, 10),
    neurons2: parseInt(elN2.value, 10),
    neurons3: parseInt(elN3.value, 10),
    neurons4: parseInt(elN4.value, 10),
    neurons5: parseInt(elN5.value, 10)
  };
}

function collectSettingsUpdates(root: HTMLElement): SettingsUpdate[] {
  const sliders = root.querySelectorAll<HTMLInputElement>('input[data-path]');
  const updates: SettingsUpdate[] = [];
  sliders.forEach(sl => {
    const value = sl.type === "checkbox" ? (sl.checked ? 1 : 0) : Number(sl.value);
    updates.push({ path: sl.dataset['path']! as SettingsUpdate['path'], value });
  });
  return updates;
}

/**
 * Synchronises the displayed numbers next to the core UI sliders and
 * disables or enables the neuron sliders based on the number of hidden
 * layers.
 */
function refreshCoreUIState(): void {
  snakesVal.textContent = elSnakes.value;
  simSpeedVal.textContent = Number(elSimSpeed.value).toFixed(2);
  layersVal.textContent = elLayers.value;
  n1Val.textContent = elN1.value;
  n2Val.textContent = elN2.value;
  n3Val.textContent = elN3.value;
  n4Val.textContent = elN4.value;
  n5Val.textContent = elN5.value;
  const L = parseInt(elLayers.value, 10);
  elN2.disabled = L < 2;
  elN3.disabled = L < 3;
  elN4.disabled = L < 4;
  elN5.disabled = L < 5;
  const applyOpacity = (el: HTMLInputElement) => {
    el.style.opacity = el.disabled ? '0.45' : '1';
  };
  applyOpacity(elN1);
  applyOpacity(elN2);
  applyOpacity(elN3);
  applyOpacity(elN4);
  applyOpacity(elN5);
}

// Build dynamic settings UI and initialise defaults
setupSettingsUI(settingsContainer, liveUpdateFromSlider);
// Set default values on core sliders (these match CFG_DEFAULT and original defaults)
elSnakes.value = '55';
elSimSpeed.value = '1.00';
elLayers.value = '2';
elN1.value = '64';
elN2.value = '64';
elN3.value = '64';
elN4.value = '48';
elN5.value = '32';
refreshCoreUIState();
loadStoredGraphSpec();
if (toggleSettingsLock) {
  toggleSettingsLock.addEventListener('click', () => {
    settingsLocked = !settingsLocked;
    applySettingsLock();
  });
}
applySettingsLock();

graphTemplateButtons.forEach(btn => {
  btn.addEventListener('click', () => {
    const template = btn.dataset['template'];
    const spec =
      template === 'mlp-gru-mlp'
        ? buildMlpGruMlpTemplate()
        : template === 'skip'
          ? buildSkipTemplate()
          : template === 'split'
            ? buildSplitTemplate()
            : buildLinearMlpTemplate();
    setGraphDraft(spec, 'Template loaded. Apply graph to use it.');
  });
});

function setGraphDiagramFullscreen(isFullscreen: boolean): void {
  if (!graphDiagramWrap) return;
  graphDiagramWrap.classList.toggle('fullscreen', isFullscreen);
  graphDiagramBackdrop?.classList.toggle('active', isFullscreen);
  document.body.classList.toggle('graph-diagram-open', isFullscreen);
  if (graphDiagramToggle) {
    graphDiagramToggle.textContent = isFullscreen ? 'Close' : 'Full screen';
  }
}

if (graphDiagramToggle) {
  graphDiagramToggle.addEventListener('click', () => {
    const isFullscreen = graphDiagramWrap?.classList.contains('fullscreen') ?? false;
    setGraphDiagramFullscreen(!isFullscreen);
  });
}
if (graphDiagramBackdrop) {
  graphDiagramBackdrop.addEventListener('click', () => {
    setGraphDiagramFullscreen(false);
  });
}

if (graphDiagramConnect) {
  graphDiagramConnect.addEventListener('click', () => {
    setGraphConnectMode(!graphConnectMode);
  });
}

if (graphDiagramAddNode) {
  graphDiagramAddNode.addEventListener('click', () => {
    const draft = ensureGraphDraft();
    const id = buildUniqueNodeId(draft, 'node');
    draft.nodes.push(buildDefaultNode('Dense', id));
    if (graphDiagram?.viewBox?.baseVal) {
      const box = graphDiagram.viewBox.baseVal;
      const centerX = graphPointerPos?.x ?? box.x + box.width / 2 - DIAGRAM_NODE_WIDTH / 2;
      const centerY = graphPointerPos?.y ?? box.y + box.height / 2 - DIAGRAM_NODE_HEIGHT / 2;
      graphLayoutOverrides.set(id, { x: centerX, y: centerY });
    }
    renderGraphEditor();
    setGraphSelection({ nodeId: id });
    setGraphSpecStatus('Node added. Apply graph to use it.');
  });
}

if (graphDiagramAddOutput) {
  graphDiagramAddOutput.addEventListener('click', () => {
    const draft = ensureGraphDraft();
    const targetId =
      graphSelectedNodeId ??
      draft.nodes.find(node => node.type !== 'Input')?.id ??
      draft.nodes[0]?.id;
    if (!targetId) {
      setGraphSpecStatus('Add a node before adding outputs.', true);
      return;
    }
    draft.outputs.push({ nodeId: targetId });
    renderGraphEditor();
    setGraphSelection({ outputIndex: draft.outputs.length - 1 });
    setGraphSpecStatus('Output added. Apply graph to use it.');
  });
}

if (graphDiagramAuto) {
  graphDiagramAuto.addEventListener('click', () => {
    graphLayoutOverrides.clear();
    renderGraphEditor();
    setGraphSpecStatus('Layout reset to auto.', false);
  });
}

if (graphDiagramDelete) {
  graphDiagramDelete.addEventListener('click', () => {
    const draft = ensureGraphDraft();
    if (graphSelectedNodeId) {
      const idx = draft.nodes.findIndex(node => node.id === graphSelectedNodeId);
      if (idx < 0) return;
      if (draft.nodes[idx]?.type === 'Input') {
        setGraphSpecStatus('Input node cannot be removed.', true);
        return;
      }
      const removedId = draft.nodes[idx]!.id;
      draft.nodes.splice(idx, 1);
      draft.edges = draft.edges.filter(edge => edge.from !== removedId && edge.to !== removedId);
      draft.outputs = draft.outputs.filter(out => out.nodeId !== removedId);
      graphLayoutOverrides.delete(removedId);
      setGraphSelection({});
      renderGraphEditor();
      setGraphSpecStatus('Node removed. Apply graph to use it.');
      return;
    }
    if (graphSelectedEdgeIndex != null) {
      draft.edges.splice(graphSelectedEdgeIndex, 1);
      setGraphSelection({});
      renderGraphEditor();
      setGraphSpecStatus('Edge removed. Apply graph to use it.');
      return;
    }
    if (graphSelectedOutputIndex != null) {
      draft.outputs.splice(graphSelectedOutputIndex, 1);
      setGraphSelection({});
      renderGraphEditor();
      setGraphSpecStatus('Output removed. Apply graph to use it.');
    }
  });
}

if (graphDiagram) {
  graphDiagram.addEventListener('pointermove', (event) => {
    const point = getSvgPoint(event);
    if (point) {
      graphPointerPos = point;
    }
    if (graphDragState && point) {
      const x = point.x - graphDragState.offsetX;
      const y = point.y - graphDragState.offsetY;
      graphLayoutOverrides.set(graphDragState.id, { x, y });
      renderGraphDiagram(ensureGraphDraft());
    }
  });
  graphDiagram.addEventListener('pointerup', () => {
    graphDragState = null;
  });
  graphDiagram.addEventListener('pointerleave', () => {
    graphDragState = null;
  });
  graphDiagram.addEventListener('click', () => {
    if (graphConnectMode) {
      graphConnectFromId = null;
      setGraphSpecStatus('Connect mode: select a start node.');
    }
    setGraphSelection({});
  });
}

if (graphNodeAdd) {
  graphNodeAdd.addEventListener('click', () => {
    const draft = ensureGraphDraft();
    draft.nodes.push(buildDefaultNode('Dense', buildUniqueNodeId(draft, 'node')));
    renderGraphEditor();
    setGraphSpecStatus('Node added. Apply graph to use it.');
  });
}

if (graphEdgeAdd) {
  graphEdgeAdd.addEventListener('click', () => {
    const draft = ensureGraphDraft();
    const ids = draft.nodes.map(node => node.id);
    if (ids.length < 2) {
      setGraphSpecStatus('Add at least two nodes before adding edges.', true);
      return;
    }
    draft.edges.push({ from: ids[0]!, to: ids[1]! });
    renderGraphEditor();
    setGraphSpecStatus('Edge added. Apply graph to use it.');
  });
}

if (graphOutputAdd) {
  graphOutputAdd.addEventListener('click', () => {
    const draft = ensureGraphDraft();
    const ids = draft.nodes.map(node => node.id);
    if (!ids.length) {
      setGraphSpecStatus('Add a node before adding outputs.', true);
      return;
    }
    draft.outputs.push({ nodeId: ids[ids.length - 1]! });
    renderGraphEditor();
    setGraphSpecStatus('Output added. Apply graph to use it.');
  });
}

if (graphApply) {
  graphApply.addEventListener('click', () => {
    const draft = ensureGraphDraft();
    if (!applyGraphSpec(draft, 'Graph applied.')) return;
    initWorker(true);
  });
}

if (graphReset) {
  graphReset.addEventListener('click', () => {
    resetGraphDraft();
  });
}

if (graphPresetSave) {
  graphPresetSave.addEventListener('click', async () => {
    const name = graphPresetName?.value.trim() ?? '';
    if (!name) {
      setGraphSpecStatus('Preset name is required.', true);
      return;
    }
    const baseUrl = resolveServerHttpUrl();
    if (!baseUrl || typeof fetch === 'undefined') {
      setGraphSpecStatus('Server not available for saving presets.', true);
      return;
    }
    const spec = getDraftGraphSpec();
    try {
      const response = await fetch(`${baseUrl}/api/graph-presets`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ name, spec })
      });
      const data = (await response.json()) as { ok?: boolean; presetId?: number; message?: string };
      if (!response.ok || !data.ok) {
        setGraphSpecStatus(data.message || 'Failed to save preset.', true);
        return;
      }
      setGraphSpecStatus(`Saved preset "${name}".`);
      if (graphPresetName) graphPresetName.value = '';
      await refreshSavedPresets();
    } catch (err) {
      console.warn('[graph-presets] save failed', err);
      setGraphSpecStatus('Failed to save preset.', true);
    }
  });
}

if (graphSpecApply && graphSpecInput) {
  graphSpecApply.addEventListener('click', () => {
    const raw = graphSpecInput.value.trim();
    if (!raw) {
      setGraphSpecStatus('Graph spec is empty.', true);
      return;
    }
    let parsed: GraphSpec;
    try {
      parsed = JSON.parse(raw) as GraphSpec;
    } catch {
      setGraphSpecStatus('Graph spec is not valid JSON.', true);
      return;
    }
    const result = validateGraph(parsed);
    if (!result.ok) {
      setGraphSpecStatus(`Graph spec error: ${result.reason}`, true);
      return;
    }
    if (!Array.isArray(parsed.nodes)) {
      setGraphSpecStatus('Graph spec must include a nodes array.', true);
      return;
    }
    const inputNode = parsed.nodes.find(node => node.type === 'Input');
    const inputSize = inputNode && 'outputSize' in inputNode ? inputNode.outputSize : null;
    if (inputSize !== CFG.brain.inSize) {
      setGraphSpecStatus(`Input size mismatch (expected ${CFG.brain.inSize}).`, true);
      return;
    }
    if (parsed.outputSize !== CFG.brain.outSize) {
      setGraphSpecStatus(`Output size mismatch (expected ${CFG.brain.outSize}).`, true);
      return;
    }
    setGraphDraft(parsed, 'Loaded JSON into editor. Apply graph to use it.');
  });
}

if (graphSpecCopy) {
  graphSpecCopy.addEventListener('click', () => {
    const spec = getDraftGraphSpec();
    if (graphSpecInput) {
      graphSpecInput.value = JSON.stringify(spec, null, 2);
    }
    setGraphSpecStatus('Copied current graph into the JSON editor.');
  });
}

if (graphSpecExport) {
  graphSpecExport.addEventListener('click', () => {
    const spec = getDraftGraphSpec();
    exportJsonToFile(spec, 'slither_neuroevo_graph_spec.json');
    setGraphSpecStatus('Exported current graph spec.');
  });
}

let currentFrameBuffer: Float32Array | null = null;
let currentStats: FrameStats = { gen: 1, alive: 0, fps: 60 };
let fitnessHistory: FitnessHistoryUiEntry[] = []; // Track fitness over generations for charts
let godModeLog: GodModeLogEntry[] = []; // Track God Mode interactions
let selectedSnake: SelectedSnake | null = null; // Currently selected snake for God Mode
let isDragging = false;
let clientCamX = 0;
let clientCamY = 0;
let clientZoom = 1;

let currentVizData: VizData | null = null;
let pendingExport = false;

const proxyWorld: ProxyWorld = {
  generation: 1,
  population: [],
  snakes: [],
  zoom: 1.0,
  cameraX: 0,
  cameraY: 0,
  viewMode: 'overview',
  fitnessHistory: fitnessHistory,
  // Helpers mimicking World for Settings UI/Persistence
  toggleViewMode: () => {
    if (worker) {
      worker.postMessage({ type: 'action', action: 'toggleView' });
      proxyWorld.viewMode = proxyWorld.viewMode === 'overview' ? 'follow' : 'overview';
      return;
    }
    if (wsClient && wsClient.isConnected()) {
      proxyWorld.viewMode = proxyWorld.viewMode === 'overview' ? 'follow' : 'overview';
    }
  },
  resurrect: (genome: unknown) => {
    if (!worker) return;
    worker.postMessage({ type: 'resurrect', genome });
  }
};
window.currentWorld = proxyWorld; // For HoF

function setConnectionStatus(mode: ConnectionMode): void {
  connectionMode = mode;
  if (!connectionStatus) return;
  connectionStatus.classList.remove('connecting', 'server', 'worker');
  connectionStatus.classList.add(mode);
  if (mode === 'server') connectionStatus.textContent = 'Server';
  else if (mode === 'worker') connectionStatus.textContent = 'Worker';
  else connectionStatus.textContent = 'Connecting';
}

function setJoinOverlayVisible(visible: boolean): void {
  if (!joinOverlay) return;
  joinOverlay.classList.toggle('hidden', !visible);
}

function setJoinStatus(text: string): void {
  if (!joinStatus) return;
  joinStatus.textContent = text;
}

function updateJoinControls(): void {
  if (!joinPlay || !joinName) return;
  const connected = wsClient?.isConnected() ?? false;
  const hasName = joinName.value.trim().length > 0;
  joinPlay.disabled = !connected || !hasName || joinPending;
  if (joinSpectate) joinSpectate.disabled = !connected || joinPending;
}

type SavedGraphPreset = { id: number; name: string; createdAt: number };
type LoadedGraphPreset = SavedGraphPreset & { spec: GraphSpec };

function resolveServerHttpUrl(): string | null {
  const wsUrl = serverUrl || resolveServerUrl();
  if (!wsUrl) return null;
  try {
    const url = new URL(wsUrl);
    url.protocol = url.protocol === 'wss:' ? 'https:' : 'http:';
    url.hash = '';
    return url.toString().replace(/\/$/, '');
  } catch {
    return null;
  }
}

function getActiveGraphSpec(): GraphSpec {
  return customGraphSpec ?? graphDraft ?? buildLinearMlpTemplate();
}

function getDraftGraphSpec(): GraphSpec {
  return graphDraft ?? customGraphSpec ?? buildLinearMlpTemplate();
}

function cloneGraphSpec(spec: GraphSpec): GraphSpec {
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

function ensureGraphDraft(): GraphSpec {
  if (!graphDraft) {
    graphDraft = cloneGraphSpec(getActiveGraphSpec());
  }
  return graphDraft;
}

function setGraphSpecStatus(text: string, isError = false): void {
  if (!graphSpecStatus) return;
  graphSpecStatus.textContent = text;
  graphSpecStatus.style.color = isError ? '#ff9b9b' : '';
  if (graphDiagram) {
    renderGraphDiagram(ensureGraphDraft());
  }
}

function setGraphDraft(spec: GraphSpec, note = ''): void {
  graphDraft = cloneGraphSpec(spec);
  renderGraphEditor();
  if (note) setGraphSpecStatus(note);
}

function applyGraphSpec(spec: GraphSpec, note = 'Graph applied.'): boolean {
  const next = cloneGraphSpec(spec);
  next.outputSize = CFG.brain.outSize;
  const inputNodes = next.nodes.filter(node => node.type === 'Input');
  if (inputNodes.length !== 1) {
    setGraphSpecStatus('Graph must include exactly one Input node.', true);
    return false;
  }
  const inputNode = inputNodes[0]!;
  if (inputNode.outputSize !== CFG.brain.inSize) {
    setGraphSpecStatus(`Input size mismatch (expected ${CFG.brain.inSize}).`, true);
    return false;
  }
  const result = validateGraph(next);
  if (!result.ok) {
    setGraphSpecStatus(`Graph error: ${result.reason}`, true);
    return false;
  }
  customGraphSpec = next;
  CFG.brain.graphSpec = next;
  graphDraft = cloneGraphSpec(next);
  if (graphSpecInput) graphSpecInput.value = JSON.stringify(next, null, 2);
  try {
    localStorage.setItem(GRAPH_SPEC_STORAGE_KEY, JSON.stringify(next));
  } catch {
    // Ignore storage failures.
  }
  renderGraphEditor();
  setGraphSpecStatus(note);
  return true;
}

function resetGraphDraft(): void {
  if (customGraphSpec) {
    setGraphDraft(customGraphSpec, 'Editor reset to applied graph.');
    return;
  }
  setGraphDraft(buildLinearMlpTemplate(), 'Editor reset to default graph.');
}

function renderSavedPresetList(presets: SavedGraphPreset[]): void {
  if (!graphPresetList) return;
  graphPresetList.innerHTML = '';
  if (!presets.length) {
    const empty = document.createElement('div');
    empty.className = 'meta';
    empty.textContent = 'No saved presets yet.';
    graphPresetList.appendChild(empty);
    return;
  }
  presets.forEach(preset => {
    const row = document.createElement('div');
    row.className = 'graph-preset-item';
    const name = document.createElement('div');
    name.className = 'graph-preset-name';
    name.textContent = preset.name;
    const load = document.createElement('button');
    load.className = 'btn small';
    load.textContent = 'Load';
    load.addEventListener('click', () => {
      void loadPresetById(preset.id);
    });
    row.appendChild(name);
    row.appendChild(load);
    graphPresetList.appendChild(row);
  });
}

async function loadPresetById(presetId: number): Promise<void> {
  const baseUrl = resolveServerHttpUrl();
  if (!baseUrl || typeof fetch === 'undefined') {
    setGraphSpecStatus('Server not available for loading presets.', true);
    return;
  }
  try {
    const response = await fetch(`${baseUrl}/api/graph-presets/${presetId}`);
    const data = (await response.json()) as { ok?: boolean; preset?: LoadedGraphPreset; message?: string };
    if (!response.ok || !data.ok || !data.preset) {
      setGraphSpecStatus(data.message || 'Failed to load preset.', true);
      return;
    }
    const result = validateGraph(data.preset.spec);
    if (!result.ok) {
      setGraphSpecStatus(`Preset graph invalid: ${result.reason}`, true);
      return;
    }
    setGraphDraft(data.preset.spec, `Loaded preset "${data.preset.name}". Apply graph to use it.`);
  } catch (err) {
    console.warn('[graph-presets] load failed', err);
    setGraphSpecStatus('Failed to load preset.', true);
  }
}

async function refreshSavedPresets(): Promise<void> {
  if (!graphPresetList) return;
  const baseUrl = resolveServerHttpUrl();
  if (!baseUrl) {
    renderSavedPresetList([]);
    return;
  }
  if (typeof fetch === 'undefined') {
    setGraphSpecStatus('Fetch not available for saved presets.', true);
    return;
  }
  try {
    const response = await fetch(`${baseUrl}/api/graph-presets?limit=50`);
    const data = (await response.json()) as { ok?: boolean; presets?: SavedGraphPreset[]; message?: string };
    if (!response.ok || !data.ok) {
      setGraphSpecStatus(data.message || 'Failed to load saved presets.', true);
      renderSavedPresetList([]);
      return;
    }
    renderSavedPresetList(data.presets ?? []);
  } catch (err) {
    console.warn('[graph-presets] load failed', err);
    setGraphSpecStatus('Failed to load saved presets.', true);
    renderSavedPresetList([]);
  }
}

function loadStoredGraphSpec(): void {
  try {
    const raw = localStorage.getItem(GRAPH_SPEC_STORAGE_KEY);
    if (!raw) {
      const fallback = buildLinearMlpTemplate();
      applyGraphSpec(fallback, 'Default graph applied.');
      return;
    }
    const parsed = JSON.parse(raw) as GraphSpec;
    const result = validateGraph(parsed);
    if (result.ok) {
      const applied = applyGraphSpec(parsed, 'Loaded saved graph.');
      if (!applied) {
        applyGraphSpec(buildLinearMlpTemplate(), 'Stored graph invalid. Using default.');
      }
    } else {
      applyGraphSpec(buildLinearMlpTemplate(), 'Stored graph invalid. Using default.');
    }
  } catch {
    applyGraphSpec(buildLinearMlpTemplate(), 'Stored graph invalid. Using default.');
  }
}

function buildHiddenSizesForExample(settings: ReturnType<typeof readSettingsFromCoreUI>): number[] {
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

function buildLinearMlpTemplate(): GraphSpec {
  const hiddenSizes = buildHiddenSizesForExample(readSettingsFromCoreUI());
  const inputSize = CFG.brain.inSize;
  const outputSize = CFG.brain.outSize;
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

function buildMlpGruMlpTemplate(): GraphSpec {
  const hiddenSizes = buildHiddenSizesForExample(readSettingsFromCoreUI());
  const inputSize = CFG.brain.inSize;
  const outputSize = CFG.brain.outSize;
  const featureSize = hiddenSizes[hiddenSizes.length - 1] ?? Math.max(4, Math.floor(inputSize * 0.75));
  const mlpHidden = hiddenSizes.length > 1 ? hiddenSizes.slice(0, -1) : hiddenSizes;
  const gruHidden = Math.max(2, Math.floor(CFG.brain.gruHidden || featureSize));
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

function buildSkipTemplate(): GraphSpec {
  const hiddenSizes = buildHiddenSizesForExample(readSettingsFromCoreUI());
  const inputSize = CFG.brain.inSize;
  const outputSize = CFG.brain.outSize;
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

function buildSplitTemplate(): GraphSpec {
  const inputSize = CFG.brain.inSize;
  const outputSize = CFG.brain.outSize;
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

function buildDefaultNode(type: GraphNodeType, id: string): GraphNodeSpec {
  const inputSize = CFG.brain.inSize;
  const outputSize = CFG.brain.outSize;
  const hiddenSizes = buildHiddenSizesForExample(readSettingsFromCoreUI());
  switch (type) {
    case 'Input':
      return { id, type: 'Input', outputSize: inputSize };
    case 'Dense':
      return { id, type: 'Dense', inputSize, outputSize };
    case 'MLP':
      return { id, type: 'MLP', inputSize, outputSize, hiddenSizes };
    case 'GRU':
      return { id, type: 'GRU', inputSize, hiddenSize: Math.max(2, Math.floor(CFG.brain.gruHidden || 8)) };
    case 'LSTM':
      return { id, type: 'LSTM', inputSize, hiddenSize: Math.max(2, Math.floor(CFG.brain.lstmHidden || CFG.brain.gruHidden || 8)) };
    case 'RRU':
      return { id, type: 'RRU', inputSize, hiddenSize: Math.max(2, Math.floor(CFG.brain.rruHidden || CFG.brain.gruHidden || 8)) };
    case 'Split': {
      const first = Math.max(1, Math.floor(inputSize / 2));
      const second = Math.max(1, inputSize - first);
      return { id, type: 'Split', outputSizes: [first, second] };
    }
    case 'Concat':
      return { id, type: 'Concat' };
  }
}

function buildUniqueNodeId(spec: GraphSpec, prefix: string): string {
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

type DiagramNode = {
  id: string;
  label: string;
  type: string;
  layer: number;
  outputIndex?: number;
};

type DiagramEdge = {
  from: string;
  to: string;
  fromPort?: number;
  toPort?: number;
  edgeIndex?: number;
  outputIndex?: number;
};

function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, value));
}

function setGraphSelection(next: { nodeId?: string | null; edgeIndex?: number | null; outputIndex?: number | null }): void {
  graphSelectedNodeId = next.nodeId ?? null;
  graphSelectedEdgeIndex = next.edgeIndex ?? null;
  graphSelectedOutputIndex = next.outputIndex ?? null;
  if (graphSelectedNodeId) {
    graphSelectedEdgeIndex = null;
    graphSelectedOutputIndex = null;
  }
  if (graphSelectedEdgeIndex != null) {
    graphSelectedNodeId = null;
    graphSelectedOutputIndex = null;
  }
  if (graphSelectedOutputIndex != null) {
    graphSelectedNodeId = null;
    graphSelectedEdgeIndex = null;
  }
  renderGraphEditor();
}

function syncGraphSelection(spec: GraphSpec): void {
  if (graphSelectedNodeId && !spec.nodes.some(node => node.id === graphSelectedNodeId)) {
    graphSelectedNodeId = null;
  }
  if (graphSelectedEdgeIndex != null && (graphSelectedEdgeIndex < 0 || graphSelectedEdgeIndex >= spec.edges.length)) {
    graphSelectedEdgeIndex = null;
  }
  if (
    graphSelectedOutputIndex != null &&
    (graphSelectedOutputIndex < 0 || graphSelectedOutputIndex >= spec.outputs.length)
  ) {
    graphSelectedOutputIndex = null;
  }
}

function getSvgPoint(evt: PointerEvent | MouseEvent): { x: number; y: number } | null {
  if (!graphDiagram) return null;
  const rect = graphDiagram.getBoundingClientRect();
  const viewBox = graphDiagram.viewBox?.baseVal;
  if (!viewBox) return null;
  const x = ((evt.clientX - rect.left) / rect.width) * viewBox.width + viewBox.x;
  const y = ((evt.clientY - rect.top) / rect.height) * viewBox.height + viewBox.y;
  return { x, y };
}

function setGraphConnectMode(next: boolean): void {
  graphConnectMode = next;
  graphConnectFromId = null;
  if (graphDiagramConnect) {
    graphDiagramConnect.classList.toggle('active', next);
    graphDiagramConnect.textContent = next ? 'Connecting' : 'Connect';
  }
  setGraphSpecStatus(next ? 'Connect mode: click a start node.' : 'Connect mode off.');
}

function handleDiagramConnect(targetId: string): void {
  const spec = ensureGraphDraft();
  if (!graphConnectFromId) {
    graphConnectFromId = targetId;
    setGraphSelection({ nodeId: targetId });
    setGraphSpecStatus('Connect mode: select a target node.');
    return;
  }
  if (graphConnectFromId === targetId) {
    graphConnectFromId = null;
    setGraphSpecStatus('Connect mode cancelled.');
    return;
  }
  const created = addGraphEdge(graphConnectFromId, targetId, spec);
  graphConnectFromId = null;
  if (created != null) {
    setGraphSelection({ edgeIndex: created });
    setGraphSpecStatus('Edge added. Apply graph to use it.');
  }
}

function addGraphEdge(fromId: string, toId: string, spec: GraphSpec): number | null {
  const fromNode = spec.nodes.find(node => node.id === fromId);
  const toNode = spec.nodes.find(node => node.id === toId);
  if (!fromNode || !toNode) return null;
  if (fromNode.id === toNode.id) return null;
  const next: GraphEdge = { from: fromId, to: toId };

  if (fromNode.type === 'Split') {
    const maxPorts = fromNode.outputSizes.length;
    const used = new Set(
      spec.edges
        .filter(edge => edge.from === fromId && edge.fromPort != null)
        .map(edge => edge.fromPort as number)
    );
    let chosen = 0;
    while (used.has(chosen) && chosen < maxPorts) chosen += 1;
    next.fromPort = clamp(chosen, 0, Math.max(0, maxPorts - 1));
  }

  if (toNode.type === 'Concat') {
    const used = new Set(
      spec.edges
        .filter(edge => edge.to === toId && edge.toPort != null)
        .map(edge => edge.toPort as number)
    );
    let chosen = 0;
    while (used.has(chosen)) chosen += 1;
    next.toPort = chosen;
  }

  const duplicate = spec.edges.some(
    edge =>
      edge.from === next.from &&
      edge.to === next.to &&
      (edge.fromPort ?? null) === (next.fromPort ?? null) &&
      (edge.toPort ?? null) === (next.toPort ?? null)
  );
  if (duplicate) {
    setGraphSpecStatus('Edge already exists.', true);
    return null;
  }
  spec.edges.push(next);
  renderGraphEditor();
  return spec.edges.length - 1;
}

function renderGraphDiagram(spec: GraphSpec): void {
  if (!graphDiagram) return;
  graphDiagram.innerHTML = '';
  if (!spec.nodes.length) return;

  const svgNs = 'http://www.w3.org/2000/svg';
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
    const override = graphLayoutOverrides.get(node.id);
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
    const isSelected = edge.edgeIndex != null && edge.edgeIndex === graphSelectedEdgeIndex;
    path.setAttribute('class', isSelected ? 'graph-diagram-edge selected' : 'graph-diagram-edge');
    path.setAttribute('d', `M ${x1} ${y1} C ${x1 + curve} ${y1} ${x2 - curve} ${y2} ${x2} ${y2}`);
    if (edge.edgeIndex != null) {
      path.addEventListener('click', (event) => {
        event.stopPropagation();
        setGraphSelection({ edgeIndex: edge.edgeIndex ?? null });
      });
    }
    edgesGroup.appendChild(path);

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
        ? graphSelectedOutputIndex === node.outputIndex
        : graphSelectedNodeId === node.id;
    const rect = document.createElementNS(svgNs, 'rect');
    rect.setAttribute('x', String(pos.x));
    rect.setAttribute('y', String(pos.y));
    rect.setAttribute('rx', '10');
    rect.setAttribute('ry', '10');
    rect.setAttribute('width', String(nodeWidth));
    rect.setAttribute('height', String(nodeHeight));
    rect.setAttribute(
      'class',
      `graph-diagram-node graph-diagram-${node.type.toLowerCase()}${isSelected ? ' selected' : ''}`
    );
    group.appendChild(rect);

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
      if (graphConnectMode || node.outputIndex != null || node.id.startsWith('__out-')) return;
      const point = getSvgPoint(event);
      if (!point) return;
      const current = positions.get(node.id);
      if (!current) return;
      graphDragState = {
        id: node.id,
        offsetX: point.x - current.x,
        offsetY: point.y - current.y
      };
      graphDiagram?.setPointerCapture?.(event.pointerId);
    });

    group.addEventListener('click', (event) => {
      event.stopPropagation();
      if (node.outputIndex != null) {
        setGraphSelection({ outputIndex: node.outputIndex });
        return;
      }
      if (graphConnectMode) {
        handleDiagramConnect(node.id);
        return;
      }
      setGraphSelection({ nodeId: node.id });
    });

    nodesGroup.appendChild(group);
  });
  graphDiagram.appendChild(nodesGroup);
}

function renderGraphInspector(spec: GraphSpec): void {
  if (!graphDiagramInspector) return;
  graphDiagramInspector.innerHTML = '';

  const makeRow = (labelText: string, input: HTMLElement): HTMLDivElement => {
    const row = document.createElement('div');
    row.className = 'graph-inspector-row';
    const label = document.createElement('label');
    label.textContent = labelText;
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

  if (graphSelectedNodeId) {
    const nodeIndex = spec.nodes.findIndex(node => node.id === graphSelectedNodeId);
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
        setGraphSpecStatus('Node id cannot be empty.', true);
        return;
      }
      if (nextId !== node.id && spec.nodes.some(n => n.id === nextId)) {
        idInput.value = node.id;
        setGraphSpecStatus('Node id must be unique.', true);
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
      if (graphLayoutOverrides.has(oldId)) {
        const override = graphLayoutOverrides.get(oldId)!;
        graphLayoutOverrides.delete(oldId);
        graphLayoutOverrides.set(nextId, override);
      }
      if (graphSelectedNodeId === oldId) graphSelectedNodeId = nextId;
      if (graphConnectFromId === oldId) graphConnectFromId = nextId;
      renderGraphEditor();
      setGraphSpecStatus('Node id updated. Apply graph to use it.');
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
      spec.nodes[nodeIndex] = buildDefaultNode(nextType, node.id);
      renderGraphEditor();
      setGraphSpecStatus('Node type updated. Apply graph to use it.');
    });
    graphDiagramInspector.appendChild(makeRow('Type', typeSelect));

    if (node.type === 'Dense' || node.type === 'MLP') {
      const input = makeNumberInput(node.inputSize);
      input.addEventListener('input', () => {
        const next = Number(input.value);
        if (!Number.isFinite(next)) return;
        node.inputSize = next;
        setGraphSpecStatus('Updated node sizes. Apply graph to use it.');
      });
      graphDiagramInspector.appendChild(makeRow('Input', input));

      const output = makeNumberInput(node.outputSize);
      output.addEventListener('input', () => {
        const next = Number(output.value);
        if (!Number.isFinite(next)) return;
        node.outputSize = next;
        setGraphSpecStatus('Updated node sizes. Apply graph to use it.');
      });
      graphDiagramInspector.appendChild(makeRow('Output', output));
    }
    if (node.type === 'MLP') {
      const hidden = document.createElement('input');
      hidden.type = 'text';
      hidden.value = (node.hiddenSizes ?? []).join(', ');
      hidden.addEventListener('change', () => {
        node.hiddenSizes = parseNumberList(hidden.value);
        setGraphSpecStatus('Updated hidden sizes. Apply graph to use it.');
      });
      graphDiagramInspector.appendChild(makeRow('Hidden', hidden));
    }
    if (node.type === 'GRU' || node.type === 'LSTM' || node.type === 'RRU') {
      const input = makeNumberInput(node.inputSize);
      input.addEventListener('input', () => {
        const next = Number(input.value);
        if (!Number.isFinite(next)) return;
        node.inputSize = next;
        setGraphSpecStatus('Updated node sizes. Apply graph to use it.');
      });
      graphDiagramInspector.appendChild(makeRow('Input', input));

      const hidden = makeNumberInput(node.hiddenSize);
      hidden.addEventListener('input', () => {
        const next = Number(hidden.value);
        if (!Number.isFinite(next)) return;
        node.hiddenSize = next;
        setGraphSpecStatus('Updated hidden size. Apply graph to use it.');
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
        setGraphSpecStatus('Updated split sizes. Apply graph to use it.');
      });
      graphDiagramInspector.appendChild(makeRow('Outputs', sizes));
    }

    addActions([
      {
        label: 'Delete',
        onClick: () => {
          if (node.type === 'Input') {
            setGraphSpecStatus('Input node cannot be removed.', true);
            return;
          }
          const removedId = node.id;
          spec.nodes.splice(nodeIndex, 1);
          spec.edges = spec.edges.filter(edge => edge.from !== removedId && edge.to !== removedId);
          spec.outputs = spec.outputs.filter(out => out.nodeId !== removedId);
          graphLayoutOverrides.delete(removedId);
          setGraphSelection({});
          renderGraphEditor();
          setGraphSpecStatus('Node removed. Apply graph to use it.');
        }
      }
    ]);
    return;
  }

  if (graphSelectedEdgeIndex != null) {
    const edge = spec.edges[graphSelectedEdgeIndex];
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
      renderGraphEditor();
      setGraphSpecStatus('Edge updated. Apply graph to use it.');
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
      setGraphSpecStatus('Edge updated. Apply graph to use it.');
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
      renderGraphEditor();
      setGraphSpecStatus('Edge updated. Apply graph to use it.');
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
      setGraphSpecStatus('Edge updated. Apply graph to use it.');
    });
    graphDiagramInspector.appendChild(makeRow('To port', toPort));

    const edgeIndex = graphSelectedEdgeIndex;
    addActions([
      {
        label: 'Delete',
        onClick: () => {
          if (edgeIndex == null) return;
          spec.edges.splice(edgeIndex, 1);
          setGraphSelection({});
          renderGraphEditor();
          setGraphSpecStatus('Edge removed. Apply graph to use it.');
        }
      }
    ]);
    return;
  }

  if (graphSelectedOutputIndex != null) {
    const output = spec.outputs[graphSelectedOutputIndex];
    if (!output) return;
    graphDiagramInspector.classList.remove('hidden');
    const title = document.createElement('div');
    title.textContent = `Output ${graphSelectedOutputIndex + 1}`;
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
      renderGraphEditor();
      setGraphSpecStatus('Output updated. Apply graph to use it.');
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
      setGraphSpecStatus('Output updated. Apply graph to use it.');
    });
    graphDiagramInspector.appendChild(makeRow('Port', portInput));

    const outputIndex = graphSelectedOutputIndex;
    addActions([
      {
        label: 'Delete',
        onClick: () => {
          if (outputIndex == null) return;
          spec.outputs.splice(outputIndex, 1);
          setGraphSelection({});
          renderGraphEditor();
          setGraphSpecStatus('Output removed. Apply graph to use it.');
        }
      }
    ]);
    return;
  }

  graphDiagramInspector.classList.add('hidden');
}

function renderGraphEditor(): void {
  if (!graphNodes || !graphEdges || !graphOutputs) return;
  const spec = ensureGraphDraft();
  syncGraphSelection(spec);
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
        setGraphSpecStatus('Node id cannot be empty.', true);
        return;
      }
      if (nextId !== node.id && spec.nodes.some((n, idx) => idx !== index && n.id === nextId)) {
        idInput.value = node.id;
        setGraphSpecStatus('Node id must be unique.', true);
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
      if (graphLayoutOverrides.has(oldId)) {
        const override = graphLayoutOverrides.get(oldId)!;
        graphLayoutOverrides.delete(oldId);
        graphLayoutOverrides.set(nextId, override);
      }
      if (graphSelectedNodeId === oldId) graphSelectedNodeId = nextId;
      if (graphConnectFromId === oldId) graphConnectFromId = nextId;
      renderGraphEditor();
      setGraphSpecStatus('Node id updated. Apply graph to use it.');
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
      spec.nodes[index] = buildDefaultNode(nextType, node.id);
      renderGraphEditor();
      setGraphSpecStatus('Node type updated. Apply graph to use it.');
    });
    row.appendChild(makeField('Type', typeSelect));

    if (node.type === 'Input') {
      const out = makeNumberInput(CFG.brain.inSize, 1);
      out.disabled = true;
      row.appendChild(makeField('Output', out));
    }
    if (node.type === 'Dense' || node.type === 'MLP') {
      const input = makeNumberInput(node.inputSize, 1);
      input.addEventListener('input', () => {
        const next = Number(input.value);
        if (!Number.isFinite(next)) return;
        node.inputSize = next;
        setGraphSpecStatus('Updated node sizes. Apply graph to use it.');
      });
      row.appendChild(makeField('Input', input));

      const output = makeNumberInput(node.outputSize, 1);
      output.addEventListener('input', () => {
        const next = Number(output.value);
        if (!Number.isFinite(next)) return;
        node.outputSize = next;
        setGraphSpecStatus('Updated node sizes. Apply graph to use it.');
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
        setGraphSpecStatus('Updated hidden sizes. Apply graph to use it.');
      });
      row.appendChild(makeField('Hidden', hidden));
    }
    if (node.type === 'GRU' || node.type === 'LSTM' || node.type === 'RRU') {
      const input = makeNumberInput(node.inputSize, 1);
      input.addEventListener('input', () => {
        const next = Number(input.value);
        if (!Number.isFinite(next)) return;
        node.inputSize = next;
        setGraphSpecStatus('Updated node sizes. Apply graph to use it.');
      });
      row.appendChild(makeField('Input', input));

      const hidden = makeNumberInput(node.hiddenSize, 1);
      hidden.addEventListener('input', () => {
        const next = Number(hidden.value);
        if (!Number.isFinite(next)) return;
        node.hiddenSize = next;
        setGraphSpecStatus('Updated hidden size. Apply graph to use it.');
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
        setGraphSpecStatus('Updated split sizes. Apply graph to use it.');
      });
      row.appendChild(makeField('Outputs', sizes));
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
        graphLayoutOverrides.delete(removedId);
        if (graphSelectedNodeId === removedId) graphSelectedNodeId = null;
        if (graphConnectFromId === removedId) graphConnectFromId = null;
        renderGraphEditor();
        setGraphSpecStatus('Node removed. Apply graph to use it.');
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
      setGraphSpecStatus('Edge updated. Apply graph to use it.');
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
      setGraphSpecStatus('Edge updated. Apply graph to use it.');
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
      setGraphSpecStatus('Edge updated. Apply graph to use it.');
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
      setGraphSpecStatus('Edge updated. Apply graph to use it.');
    });
    row.appendChild(makeField('To port', toPort));

    const remove = document.createElement('button');
    remove.className = 'btn small';
    remove.textContent = 'Remove';
    remove.addEventListener('click', () => {
      spec.edges.splice(index, 1);
      renderGraphEditor();
      setGraphSpecStatus('Edge removed. Apply graph to use it.');
    });
    row.appendChild(remove);

    graphEdges.appendChild(row);
  });

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
      setGraphSpecStatus('Output updated. Apply graph to use it.');
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
      setGraphSpecStatus('Output updated. Apply graph to use it.');
    });
    row.appendChild(makeField('Port', portInput));

    const remove = document.createElement('button');
    remove.className = 'btn small';
    remove.textContent = 'Remove';
    remove.addEventListener('click', () => {
      spec.outputs.splice(index, 1);
      renderGraphEditor();
      setGraphSpecStatus('Output removed. Apply graph to use it.');
    });
    row.appendChild(remove);

    graphOutputs.appendChild(row);
  });

  renderGraphDiagram(spec);
  renderGraphInspector(spec);
}

function applySettingsLock(): void {
  if (!settingsTab || !settingsControls || !toggleSettingsLock) return;
  settingsTab.classList.toggle('settings-locked', settingsLocked);
  toggleSettingsLock.textContent = settingsLocked ? 'Unlock' : 'Lock';
  if (settingsLockHint) {
    settingsLockHint.textContent = settingsLocked
      ? 'Locked to keep little hands off the sliders.'
      : 'Unlocked. Changes apply immediately.';
  }
  const controls = settingsControls.querySelectorAll('input, button, select, textarea');
  controls.forEach(control => {
    (control as HTMLInputElement).disabled = settingsLocked;
  });
  if (!settingsLocked) refreshCoreUIState();
}

function isPlayerControlActive(): boolean {
  return connectionMode === 'server' && !!playerSnakeId;
}

function computeTurnInput(
  meta: { x: number; y: number; dir: number },
  target: { x: number; y: number }
): number {
  const dx = target.x - meta.x;
  const dy = target.y - meta.y;
  if (dx === 0 && dy === 0) return 0;
  const desired = Math.atan2(dy, dx);
  const delta = Math.atan2(Math.sin(desired - meta.dir), Math.cos(desired - meta.dir));
  const scaled = delta / (Math.PI / 2);
  return Math.max(-1, Math.min(1, scaled));
}

function sendPlayerAction(): void {
  if (!wsClient || !wsClient.isConnected()) return;
  if (!isPlayerControlActive()) return;
  const meta = playerSensorMeta;
  const target = pointerWorld;
  const turn = meta && target ? computeTurnInput(meta, target) : 0;
  const boost = boostHeld ? 1 : 0;
  const tick = playerSensorTick ? playerSensorTick + 1 : 0;
  wsClient.sendAction(tick, playerSnakeId!, turn, boost);
}

function applyFrameBuffer(buffer: ArrayBuffer): void {
  currentFrameBuffer = new Float32Array(buffer);
  const gen = currentFrameBuffer[0] ?? Number.NaN;
  if (Number.isFinite(gen)) {
    const nextGen = Math.max(1, Math.floor(gen));
    proxyWorld.generation = nextGen;
    currentStats = { ...currentStats, gen: nextGen };
  }
}

type FrameSnakeSnapshot = { id: number; x: number; y: number; ptCount: number };

function findSnakeInFrame(buffer: Float32Array, targetId: number | null): FrameSnakeSnapshot | null {
  if (buffer.length < 6) return null;
  const aliveCount = (buffer[2] ?? 0) | 0;
  let ptr = 6;
  let first: FrameSnakeSnapshot | null = null;
  for (let i = 0; i < aliveCount; i++) {
    if (ptr + 7 >= buffer.length) break;
    const id = (buffer[ptr] ?? 0) | 0;
    const x = buffer[ptr + 3] ?? 0;
    const y = buffer[ptr + 4] ?? 0;
    const ptCount = (buffer[ptr + 7] ?? 0) | 0;
    const info = { id, x, y, ptCount };
    if (!first) first = info;
    if (targetId != null && id === targetId) return info;
    ptr += 8 + ptCount * 2;
  }
  return first;
}

function updateClientCamera(): void {
  if (connectionMode !== 'server') return;
  const frame = currentFrameBuffer;
  if (!frame) return;
  const mode = proxyWorld.viewMode === 'follow' ? 'follow' : 'overview';
  if (mode === 'overview') {
    clientCamX = 0;
    clientCamY = 0;
    const effectiveR = CFG.worldRadius + CFG.observer.overviewExtraWorldMargin;
    const fit = Math.min(cssW, cssH) / (2 * effectiveR * CFG.observer.overviewPadding);
    const targetZoom = clamp(fit, 0.01, 2.0);
    if (CFG.observer.snapZoomOutInOverview && clientZoom > targetZoom) {
      clientZoom = targetZoom;
    } else {
      clientZoom = lerp(clientZoom, targetZoom, CFG.observer.zoomLerpOverview);
    }
    proxyWorld.cameraX = clientCamX;
    proxyWorld.cameraY = clientCamY;
    proxyWorld.zoom = clientZoom;
    return;
  }

  const focus = findSnakeInFrame(frame, playerSnakeId);
  if (focus) {
    clientCamX = focus.x;
    clientCamY = focus.y;
    const denom = Math.max(1, CFG.snakeMaxLen);
    const targetZoom = clamp(1.15 - (focus.ptCount / denom) * 0.55, 0.45, 1.12);
    clientZoom = lerp(clientZoom, targetZoom, CFG.observer.zoomLerpFollow);
  } else {
    clientCamX = 0;
    clientCamY = 0;
    clientZoom = lerp(clientZoom, 0.95, 0.05);
  }
  proxyWorld.cameraX = clientCamX;
  proxyWorld.cameraY = clientCamY;
  proxyWorld.zoom = clientZoom;
}

function initWorker(resetCfg = true): void {
  if (!worker) return;
  const settings = readSettingsFromCoreUI();
  const updates = collectSettingsUpdates(settingsControls ?? settingsContainer);
  worker.postMessage({
    type: 'init',
    settings,
    updates,
    resetCfg,
    viewW: cssW,
    viewH: cssH,
    graphSpec: customGraphSpec
  });
}

function handleWorkerMessage(msg: WorkerToMainMessage): void {
  switch (msg.type) {
    case 'exportResult': {
      pendingExport = false;
      if (!msg.data || !Array.isArray(msg.data.genomes)) {
        alert('Export failed: invalid payload from worker.');
        return;
      }
      const exportData = {
        generation: msg.data.generation || 1,
        genomes: msg.data.genomes,
        hof: hof.getAll()
      };
      exportToFile(exportData, `slither_neuroevo_gen${exportData.generation}.json`);
      return;
    }
    case 'importResult': {
      if (!msg.ok) {
        alert(`Import failed: ${msg.reason || 'unknown error'}`);
      } else {
        const used = msg.used || 0;
        const total = msg.total || 0;
        alert(`Import applied. Loaded ${used}/${total} genomes.`);
      }
      return;
    }
    case 'frame': {
      if (!currentFrameBuffer) console.log("First frame received!");
      applyFrameBuffer(msg.buffer);
      currentStats = msg.stats;
      proxyWorld.generation = currentStats.gen;

      // Track fitness history for charts
      // Full history arrives occasionally; keep UI buffer synced and capped.
      if (msg.stats.fitnessHistory) {
        fitnessHistory.length = 0;
        msg.stats.fitnessHistory.forEach(entry => {
          fitnessHistory.push({
            gen: entry.gen,
            avgFitness: entry.avg,
            maxFitness: entry.best,
            minFitness: entry.min ?? 0,
            speciesCount: entry.speciesCount ?? 0,
            topSpeciesSize: entry.topSpeciesSize ?? 0,
            avgWeight: entry.avgWeight ?? 0,
            weightVariance: entry.weightVariance ?? 0
          });
        });
      }
      if (msg.stats.fitnessData) {
        const data = msg.stats.fitnessData;
        const entry = {
          gen: data.gen,
          avgFitness: data.avgFitness,
          maxFitness: data.maxFitness,
          minFitness: data.minFitness
        };
        const existingIdx = fitnessHistory.findIndex(f => f.gen === data.gen);
        if (existingIdx >= 0) {
          fitnessHistory[existingIdx] = { ...fitnessHistory[existingIdx], ...entry };
        } else {
          fitnessHistory.push(entry);
        }
        if (fitnessHistory.length > 120) fitnessHistory.shift();
      }
      if (msg.stats.hofEntry) {
        hof.add(msg.stats.hofEntry);
      }
      if (msg.stats.viz) {
        currentVizData = msg.stats.viz;
      }
      return;
    }
    default: {
      const _exhaustive: never = msg;
      return _exhaustive;
    }
  }
}

function bindWorkerHandlers(target: Worker): void {
  target.onmessage = (e: MessageEvent<WorkerToMainMessage>) => {
    handleWorkerMessage(e.data);
  };
}

function startWorker(resetCfg = true): void {
  if (worker) return;
  worker = new Worker(new URL('./worker.ts', import.meta.url), { type: 'module' });
  bindWorkerHandlers(worker);
  initWorker(resetCfg);
  worker.postMessage({ type: 'resize', viewW: cssW, viewH: cssH });
  if (activeTab === 'tab-viz') {
    worker.postMessage({ type: 'viz', enabled: true });
  }
  playerSnakeId = null;
  setJoinOverlayVisible(false);
  setConnectionStatus('worker');
}

function stopWorker(): void {
  if (!worker) return;
  worker.terminate();
  worker = null;
  pendingExport = false;
  currentVizData = null;
}

function scheduleWorkerFallback(): void {
  if (fallbackTimer !== null) return;
  fallbackTimer = window.setTimeout(() => {
    fallbackTimer = null;
    if (wsClient?.isConnected()) return;
    startWorker(true);
  }, 2000);
}

function scheduleReconnect(): void {
  if (reconnectTimer !== null) return;
  reconnectTimer = window.setTimeout(() => {
    reconnectTimer = null;
    if (!wsClient) return;
    wsClient.connect(serverUrl);
    setConnectionStatus('connecting');
    scheduleWorkerFallback();
    reconnectDelayMs = Math.min(Math.floor(reconnectDelayMs * 1.5), 10000);
  }, reconnectDelayMs);
}

function connectToServer(url: string): void {
  if (!wsClient) return;
  serverUrl = url;
  wsClient.connect(url);
  joinPending = false;
  if (!worker) {
    setConnectionStatus('connecting');
    setJoinOverlayVisible(true);
    setJoinStatus('Connecting...');
    updateJoinControls();
  } else {
    setConnectionStatus('worker');
    setJoinOverlayVisible(false);
  }
  scheduleWorkerFallback();
}

wsClient = createWsClient({
  onConnected: (info) => {
    storeServerUrl(serverUrl);
    reconnectDelayMs = 1000;
    if (fallbackTimer !== null) {
      clearTimeout(fallbackTimer);
      fallbackTimer = null;
    }
    if (worker) stopWorker();
    currentStats = { gen: 1, alive: 0, fps: info.tickRate };
    currentVizData = null;
    setConnectionStatus('server');
    joinPending = false;
    wsClient?.sendJoin('spectator');
    wsClient?.sendViz(activeTab === 'tab-viz');
    setJoinOverlayVisible(true);
    setJoinStatus('Enter a nickname to play');
    updateJoinControls();
    refreshSavedPresets().catch(() => {});
  },
  onDisconnected: () => {
    const hasWorker = !!worker;
    if (hasWorker) {
      setConnectionStatus('worker');
    } else {
      setConnectionStatus('connecting');
    }
    playerSnakeId = null;
    playerSensorTick = 0;
    playerSensorMeta = null;
    pointerWorld = null;
    boostHeld = false;
    currentVizData = null;
    joinPending = false;
    if (!hasWorker) {
      setJoinOverlayVisible(true);
      setJoinStatus('Connecting...');
      updateJoinControls();
    } else {
      setJoinOverlayVisible(false);
    }
    scheduleWorkerFallback();
    scheduleReconnect();
  },
  onFrame: (buffer) => {
    applyFrameBuffer(buffer);
  },
  onStats: (msg) => {
    currentStats = { ...currentStats, gen: msg.gen, alive: msg.alive, fps: msg.fps };
    proxyWorld.generation = msg.gen;
    if (msg.fitnessHistory) {
      fitnessHistory.length = 0;
      msg.fitnessHistory.forEach(entry => {
        fitnessHistory.push({
          gen: entry.gen,
          avgFitness: entry.avg,
          maxFitness: entry.best,
          minFitness: entry.min ?? 0,
          speciesCount: entry.speciesCount ?? 0,
          topSpeciesSize: entry.topSpeciesSize ?? 0,
          avgWeight: entry.avgWeight ?? 0,
          weightVariance: entry.weightVariance ?? 0
        });
      });
    }
    if (msg.fitnessData) {
      const data = msg.fitnessData;
      const entry = {
        gen: data.gen,
        avgFitness: data.avgFitness,
        maxFitness: data.maxFitness,
        minFitness: data.minFitness
      };
      const existingIdx = fitnessHistory.findIndex(f => f.gen === data.gen);
      if (existingIdx >= 0) {
        fitnessHistory[existingIdx] = { ...fitnessHistory[existingIdx], ...entry };
      } else {
        fitnessHistory.push(entry);
      }
      if (fitnessHistory.length > 120) fitnessHistory.shift();
    }
    if (msg.viz) {
      currentVizData = msg.viz;
    }
  },
  onAssign: (msg) => {
    playerSnakeId = msg.snakeId;
    joinPending = false;
    setJoinOverlayVisible(false);
    setJoinStatus('Connected');
    updateJoinControls();
  },
  onSensors: (msg) => {
    if (!playerSnakeId || msg.snakeId !== playerSnakeId) return;
    playerSensorTick = msg.tick;
    if (msg.meta) {
      playerSensorMeta = msg.meta;
    }
    sendPlayerAction();
  },
  onError: (msg) => {
    console.warn(`[ws] ${msg.message}`);
    if (joinPending) {
      joinPending = false;
      updateJoinControls();
    }
    if (joinOverlay && !joinOverlay.classList.contains('hidden')) {
      setJoinStatus(msg.message);
    }
  }
});

if (typeof WebSocket === 'undefined') {
  startWorker(true);
} else {
  connectToServer(resolveServerUrl());
}

/**
 * Live update handler for sliders that do not require a reset. Updates
 * the corresponding CFG value and allows world to respond immediately.
 * @param {HTMLInputElement} sliderEl
 */
function liveUpdateFromSlider(sliderEl: HTMLInputElement): void {
  setByPath(CFG, sliderEl.dataset['path']!, Number(sliderEl.value));
  if (!worker) return;
  worker.postMessage({ 
      type: 'updateSettings', 
      updates: [{
        path: sliderEl.dataset['path']! as SettingsUpdate['path'],
        value: Number(sliderEl.value)
      }] 
  });
}

// Live update simulation speed when the slider moves
elSimSpeed.addEventListener('input', () => {
  refreshCoreUIState();
  if (!worker) return;
  worker.postMessage({ type: 'action', action: 'simSpeed', value: parseFloat(elSimSpeed.value) });
});
// Update other core UI labels live
elSnakes.addEventListener('input', refreshCoreUIState);
elLayers.addEventListener('input', refreshCoreUIState);
elN1.addEventListener('input', refreshCoreUIState);
elN2.addEventListener('input', refreshCoreUIState);
elN3.addEventListener('input', refreshCoreUIState);
elN4.addEventListener('input', refreshCoreUIState);
elN5.addEventListener('input', refreshCoreUIState);

// Apply new configuration and reset world
btnApply.addEventListener('click', () => {
  refreshCoreUIState();
  updateCFGFromUI(settingsControls ?? settingsContainer);
  initWorker(true); // Restart worker world with updated settings
});
// Restore defaults
btnDefaults.addEventListener('click', () => {
  resetCFGToDefaults();
  setupSettingsUI(settingsContainer, liveUpdateFromSlider); // Re-apply defaults to dynamic UI
  elSnakes.value = '55';
  elSimSpeed.value = '1.00';
  elLayers.value = '2';
  elN1.value = '64';
  elN2.value = '64';
  elN3.value = '64';
  elN4.value = '48';
  elN5.value = '32';
  applyGraphSpec(buildLinearMlpTemplate(), 'Default graph applied.');
  refreshCoreUIState();
  applySettingsLock();
  initWorker(true);
});
// Toggle view mode
btnToggle.addEventListener('click', () => proxyWorld.toggleViewMode());
window.addEventListener('keydown', e => {
  if (e.code === 'KeyV') proxyWorld.toggleViewMode();
});

// ============== GOD MODE: Canvas Event Handlers ==============

/**
 * Convert screen coordinates to world coordinates
 */
function screenToWorld(screenX: number, screenY: number): { x: number; y: number } {
  // Get camera data from buffer if available
  let camX = 0, camY = 0, zoom = 1;
  if (connectionMode === 'server') {
    camX = clientCamX;
    camY = clientCamY;
    zoom = clientZoom;
  } else if (currentFrameBuffer && currentFrameBuffer.length >= 6) {
    camX = currentFrameBuffer[3] ?? 0;
    camY = currentFrameBuffer[4] ?? 0;
    zoom = currentFrameBuffer[5] ?? 1;
  }
  
  const centerX = cssW / 2;
  const centerY = cssH / 2;
  const worldX = camX + (screenX - centerX) / zoom;
  const worldY = camY + (screenY - centerY) / zoom;
  return { x: worldX, y: worldY };
}

/**
 * Find snake near world coordinates
 */
function findSnakeNear(worldX: number, worldY: number, maxDist = 50): SelectedSnake | null {
  if (!currentFrameBuffer || currentFrameBuffer.length < 6) return null;
  
  let ptr = 6; // Skip header
  const buffer = currentFrameBuffer;
  const read = (idx: number): number => buffer[idx] ?? 0;
  const aliveCount = read(2) | 0;
  let closestSnake = null;
  let closestDist = maxDist;
  
  for (let i = 0; i < aliveCount; i++) {
    const ptCount = read(ptr + 7) | 0;
    const blockSize = 8 + ptCount * 2;
    if (ptr + blockSize > buffer.length) break;
    const id = read(ptr);
    const radius = read(ptr + 1);
    const skin = read(ptr + 2);
    const x = read(ptr + 3);
    const y = read(ptr + 4);
    
    const dist = Math.hypot(x - worldX, y - worldY);
    if (dist < closestDist && dist < radius + maxDist) {
      closestDist = dist;
      closestSnake = { id, x, y, radius, skin };
    }
    
    ptr += blockSize;
  }
  
  return closestSnake;
}

// ============== START ANIMATION LOOP ==============
requestAnimationFrame(frame);

// Click to select snake
canvas.addEventListener('click', (e) => {
  if (isPlayerControlActive()) return;
  const rect = canvas.getBoundingClientRect();
  const screenX = e.clientX - rect.left;
  const screenY = e.clientY - rect.top;
  const world = screenToWorld(screenX, screenY);
  
  const snake = findSnakeNear(world.x, world.y);
  if (snake) {
    selectedSnake = snake;
    godModeLog.push({
      time: Date.now(),
      action: 'select',
      snakeId: snake.id,
      result: 'success'
    });
    console.log('Selected snake #' + snake.id);
  } else {
    selectedSnake = null;
  }
});

// Right-click to kill snake
canvas.addEventListener('contextmenu', (e) => {
  e.preventDefault();
  if (isPlayerControlActive()) return;
  
  const rect = canvas.getBoundingClientRect();
  const screenX = e.clientX - rect.left;
  const screenY = e.clientY - rect.top;
  const world = screenToWorld(screenX, screenY);
  
  const snake = findSnakeNear(world.x, world.y);
  if (snake) {
    if (worker) {
      worker.postMessage({ type: 'godMode', action: 'kill', snakeId: snake.id });
    }
    godModeLog.push({
      time: Date.now(),
      action: 'kill',
      snakeId: snake.id,
      result: 'sent'
    });
    console.log('Killed snake #' + snake.id);
  }
});

// Drag to move snake (hold left mouse button)
canvas.addEventListener('mousedown', (e) => {
  if (isPlayerControlActive()) {
    if (e.button === 0) boostHeld = true;
    const rect = canvas.getBoundingClientRect();
    pointerWorld = screenToWorld(e.clientX - rect.left, e.clientY - rect.top);
    return;
  }
  if (e.button === 0 && selectedSnake) {
    isDragging = true;
  }
});

canvas.addEventListener('mousemove', (e) => {
  if (isPlayerControlActive()) {
    const rect = canvas.getBoundingClientRect();
    pointerWorld = screenToWorld(e.clientX - rect.left, e.clientY - rect.top);
    return;
  }
  if (isDragging && selectedSnake) {
    const rect = canvas.getBoundingClientRect();
    const screenX = e.clientX - rect.left;
    const screenY = e.clientY - rect.top;
    const world = screenToWorld(screenX, screenY);

    if (worker) {
      worker.postMessage({ 
        type: 'godMode', 
        action: 'move', 
        snakeId: selectedSnake.id,
        x: world.x,
        y: world.y
      });
    }
  }
});

canvas.addEventListener('mouseup', (e) => {
  if (isPlayerControlActive()) {
    if (e.button === 0) boostHeld = false;
    return;
  }
  if (isDragging) {
    isDragging = false;
    if (selectedSnake) {
      godModeLog.push({
        time: Date.now(),
        action: 'drag',
        snakeId: selectedSnake.id,
        result: 'completed'
      });
    }
  }
});

canvas.addEventListener('mouseleave', () => {
  if (!isPlayerControlActive()) return;
  boostHeld = false;
  pointerWorld = null;
});

// Persistence UI Wiring
const btnExport = document.getElementById('btnExport') as HTMLButtonElement | null;
if (btnExport) {
  btnExport.addEventListener('click', () => {
    if (pendingExport) return;
    if (!worker) return;
    pendingExport = true;
    worker.postMessage({ type: 'export' });
  });
}

const btnImport = document.getElementById('btnImport') as HTMLButtonElement | null;
const fileInput = document.getElementById('fileInput') as HTMLInputElement | null;
if (btnImport && fileInput) {
  btnImport.addEventListener('click', () => {
    fileInput.click();
  });

  fileInput.addEventListener('change', async (e) => {
    const target = e.target as HTMLInputElement | null;
    if (!target?.files?.length) return;
    const file = target.files.item(0);
    if (!file) return;
    try {
      const data = await importFromFile(file);
      if (!data || !Array.isArray(data.genomes)) {
        throw new Error('Invalid import file: missing genomes array.');
      }
      if (Array.isArray(data.hof)) {
        hof.replace(data.hof);
      }
      localStorage.setItem('slither_neuroevo_pop', JSON.stringify({ generation: data.generation, genomes: data.genomes }));
      if (worker) {
        worker.postMessage({ type: 'import', data });
      }
    } catch (err) {
      console.error("Import failed", err);
      const error = err as Error;
      alert("Failed to import file: " + error.message);
    } finally {
      if (target) target.value = '';
    }
  });
}

function frame(): void {
  // console.log("Frame loop running"); // Spammy
  if (currentFrameBuffer) {
      if (connectionMode === 'server') {
        updateClientCamera();
        renderWorldStruct(ctx, currentFrameBuffer, cssW, cssH, clientZoom, clientCamX, clientCamY);
      } else {
        // Camera/zoom come from the worker buffer; avoid local overrides here.
        renderWorldStruct(ctx, currentFrameBuffer, cssW, cssH);
      }
  }
  
  // Render active tab content
  if (activeTab === 'tab-viz') {
    ctxViz.clearRect(0, 0, vizCanvas.width, vizCanvas.height);
    if (currentVizData) {
      brainViz.render(ctxViz, currentVizData);
    } else {
      ctxViz.fillStyle = '#fff';
      ctxViz.fillText("Waiting for visualization data...", 20, 20);
    }
  } else if (activeTab === 'tab-fitness') {
    // Fitness Chart needs history.
    // worker stats has gen.
    // We can maintain history locally in proxyWorld?
    // proxyWorld needs specific structure for FitnessChart.
  }
  
  // Updates UI overlay
  // ...
  
  const stepInfo = `Gen: ${currentStats.gen}  Alive: ${currentStats.alive}  FPS: ${Math.round(currentStats.fps)}`;
  const statsInfoHtml = 
    `<div class="stat-box"><span class="label">Generation</span><span class="val">${currentStats.gen}</span></div>` +
    `<div class="stat-box"><span class="label">Alive</span><span class="val">${currentStats.alive}</span></div>` +
    `<div class="stat-box"><span class="label">Sim Speed</span><span class="val">${Math.round(currentStats.fps)} FPS</span></div>` +
    `<div class="note" style="margin-top: 10px; font-size: 11px;">${stepInfo}</div>`;
  
  const statsEl = document.getElementById('statsInfo');
  if (statsEl && activeTab === 'tab-stats') {
    statsEl.innerHTML = statsInfoHtml;
    
    // Render Advanced Charts
    if (statsCanvas) {
      statsCanvas.width = statsCanvas.clientWidth;
      statsCanvas.height = Math.max(statsCanvas.clientHeight, 300);
      ctxStats.clearRect(0, 0, statsCanvas.width, statsCanvas.height);
      if (statsView === 'diversity') {
        AdvancedCharts.renderSpeciesDiversity(ctxStats, fitnessHistory, statsCanvas.width, statsCanvas.height);
      } else if (statsView === 'complexity') {
        AdvancedCharts.renderNetworkComplexity(ctxStats, fitnessHistory, statsCanvas.width, statsCanvas.height);
      } else {
        AdvancedCharts.renderAverageFitness(ctxStats, fitnessHistory, statsCanvas.width, statsCanvas.height);
      }
    }
    
    // Display God Mode Log
    const godModeLogEl = document.getElementById('godModeLog');
    if (godModeLogEl) {
      const logHtml = godModeLog.slice(-10).reverse().map(entry => {
        const time = new Date(entry.time).toLocaleTimeString();
        return `<div class="log-entry">[${time}] ${entry.action.toUpperCase()} snake #${entry.snakeId} - ${entry.result}</div>`;
      }).join('');
      godModeLogEl.innerHTML = logHtml || '<div class="log-entry">No God Mode interactions yet</div>';
    }
  }
  
  if (activeTab === 'tab-hof') {
    updateHoFTable(proxyWorld);
  }

  requestAnimationFrame(frame);
}

function updateHoFTable(world: ProxyWorld): void {
  const container = document.getElementById('hofTable');
  if (!container) return;
  
  // Throttle updates?
  if (world.generation % 1 !== 0 && Math.random() > 0.1) return;

  const list = hof.getAll() as HallOfFameEntry[];
  if (!list.length) {
    container.innerHTML = '<div style="padding:10px; color:#aaa">No records yet.</div>';
    return;
  }

  let html = '';
  list.forEach((entry: HallOfFameEntry, idx: number) => {
    html += `
      <div class="hof-item">
        <span>#${idx+1} Gen ${entry.gen} (Fit ${entry.fitness.toFixed(1)})</span>
        <button onclick="window.spawnHoF(${idx})">Spawn</button>
      </div>`;
  });
  container.innerHTML = html;
}

// Expose global helper for the onclick handlers
window.spawnHoF = function(idx) {
  const list = hof.getAll();
  const entry = list[idx];
  if (entry && window.currentWorld) {
    window.currentWorld.resurrect(entry.genome);
    // Switch to visualizer?
    // Actually resurrect sets viewMode to follow, so we just need to ensure correct tab?
    // Let's auto-switch tab too.
    (document.querySelector('[data-tab="tab-viz"]') as HTMLButtonElement).click();
  }
};
