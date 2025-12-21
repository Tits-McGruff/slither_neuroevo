// main.ts
// Entry point for the slither simulation.  Sets up the canvas, UI,
// Entry point for the slither simulation. Sets up the canvas, UI,
// constructs the World and runs the animation loop. All global
// functions and classes defined in other modules must be loaded before
// this script executes.

import { CFG, resetCFGToDefaults } from './config.ts';
import { setupSettingsUI, updateCFGFromUI } from './settings.ts';
import { setByPath } from './utils.ts';
import { renderWorldStruct } from './render.ts';
// import { World } from './world.ts'; // Logic moved to worker
import { exportJsonToFile, exportToFile, importFromFile } from './storage.ts';
import { hof } from './hallOfFame.ts';
import { BrainViz } from './BrainViz.ts';
import { AdvancedCharts } from './chartUtils.ts';
import { createWsClient, resolveServerUrl, storeServerUrl } from './net/wsClient.ts';
import { buildStackGraphSpec } from './brains/stackBuilder.ts';
import { validateGraph } from './brains/graph/validate.ts';
import type { GraphSpec } from './brains/graph/schema.ts';
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
const STACK_ORDER_STORAGE_KEY = 'slither_neuroevo_stack_order';
const DEFAULT_STACK_ORDER: Array<'gru' | 'lstm' | 'rru'> = ['gru', 'lstm', 'rru'];
let stackOrder: Array<'gru' | 'lstm' | 'rru'> = DEFAULT_STACK_ORDER.slice();
let customGraphSpec: GraphSpec | null = null;
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
const elUseMlp = document.getElementById('useMLP') as HTMLInputElement | null;
const elStackGru = document.getElementById('stackGRU') as HTMLInputElement | null;
const elStackLstm = document.getElementById('stackLSTM') as HTMLInputElement | null;
const elStackRru = document.getElementById('stackRRU') as HTMLInputElement | null;
const stackOrderList = document.getElementById('stackOrder') as HTMLElement | null;
const graphPresetSelect = document.getElementById('graphPreset') as HTMLSelectElement | null;
const graphPresetApply = document.getElementById('graphPresetApply') as HTMLButtonElement | null;
const graphPresetSaved = document.getElementById('graphPresetSaved') as HTMLSelectElement | null;
const graphPresetLoad = document.getElementById('graphPresetLoad') as HTMLButtonElement | null;
const graphPresetName = document.getElementById('graphPresetName') as HTMLInputElement | null;
const graphPresetSave = document.getElementById('graphPresetSave') as HTMLButtonElement | null;
const graphExampleSelect = document.getElementById('graphExample') as HTMLSelectElement | null;
const graphExampleApply = document.getElementById('graphExampleApply') as HTMLButtonElement | null;
const graphSpecInput = document.getElementById('graphSpecInput') as HTMLTextAreaElement | null;
const graphSpecApply = document.getElementById('graphSpecApply') as HTMLButtonElement | null;
const graphSpecClear = document.getElementById('graphSpecClear') as HTMLButtonElement | null;
const graphSpecCopy = document.getElementById('graphSpecCopy') as HTMLButtonElement | null;
const graphSpecExport = document.getElementById('graphSpecExport') as HTMLButtonElement | null;
const graphSpecStatus = document.getElementById('graphSpecStatus') as HTMLElement | null;
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
  });
}
if (joinSpectate) {
  joinSpectate.addEventListener('click', () => {
    if (!wsClient?.isConnected()) return;
    joinPending = false;
    setJoinStatus('Spectating');
    updateJoinControls();
    wsClient.sendJoin('spectator');
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
  const useMlp = elUseMlp ? elUseMlp.checked : true;
  const L = parseInt(elLayers.value, 10);
  elLayers.disabled = !useMlp;
  elN1.disabled = !useMlp;
  if (useMlp) {
    elN2.disabled = L < 2;
    elN3.disabled = L < 3;
    elN4.disabled = L < 4;
    elN5.disabled = L < 5;
  } else {
    elN2.disabled = true;
    elN3.disabled = true;
    elN4.disabled = true;
    elN5.disabled = true;
  }
  const applyOpacity = (el: HTMLInputElement) => {
    el.style.opacity = el.disabled ? '0.45' : '1';
  };
  applyOpacity(elLayers);
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
if (elStackGru) elStackGru.checked = !!CFG.brain?.stack?.gru;
if (elStackLstm) elStackLstm.checked = !!CFG.brain?.stack?.lstm;
if (elStackRru) elStackRru.checked = !!CFG.brain?.stack?.rru;
if (elUseMlp) {
  const useMlp = CFG.brain?.useMlp;
  elUseMlp.checked = useMlp == null ? true : Boolean(useMlp);
}
refreshCoreUIState();
loadStoredStackOrder();
if (CFG.brain) CFG.brain.stackOrder = stackOrder.slice();
renderStackOrder();
loadStoredGraphSpec();
if (toggleSettingsLock) {
  toggleSettingsLock.addEventListener('click', () => {
    settingsLocked = !settingsLocked;
    applySettingsLock();
  });
}
applySettingsLock();

if (graphPresetApply && graphPresetSelect) {
  graphPresetApply.addEventListener('click', () => {
    const presetId = graphPresetSelect.value;
    const preset = buildPresetSpec(presetId);
    if (!preset) {
      setGraphSpecStatus('Unknown preset selection.', true);
      return;
    }
    applyStackPreset(preset);
    const label = graphPresetSelect.options[graphPresetSelect.selectedIndex]?.text || presetId;
    const note = `Using layout "${label}".`;
    clearCustomGraphSpec(note);
    refreshCoreUIState();
    initWorker(true);
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
    const spec = getActiveGraphSpec();
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
      await refreshSavedPresets(data.presetId);
    } catch (err) {
      console.warn('[graph-presets] save failed', err);
      setGraphSpecStatus('Failed to save preset.', true);
    }
  });
}

if (graphPresetLoad) {
  graphPresetLoad.addEventListener('click', async () => {
    const rawId = graphPresetSaved?.value ?? '';
    const presetId = Number(rawId);
    if (!Number.isFinite(presetId)) {
      setGraphSpecStatus('Select a saved preset to load.', true);
      return;
    }
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
      applyCustomGraphSpec(data.preset.spec, `Loaded preset "${data.preset.name}".`);
      initWorker(true);
    } catch (err) {
      console.warn('[graph-presets] load failed', err);
      setGraphSpecStatus('Failed to load preset.', true);
    }
  });
}

if (graphExampleApply && graphExampleSelect) {
  graphExampleApply.addEventListener('click', () => {
    const exampleId = graphExampleSelect.value;
    const spec = exampleId === 'split' ? buildSplitExampleSpec() : buildSkipExampleSpec();
    if (graphSpecInput) {
      graphSpecInput.value = JSON.stringify(spec, null, 2);
    }
    setGraphSpecStatus('Loaded example. Click "Use graph spec" to apply.');
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
    applyCustomGraphSpec(parsed);
    initWorker(true);
  });
}

if (graphSpecClear) {
  graphSpecClear.addEventListener('click', () => {
    clearCustomGraphSpec('Using layout controls.');
    initWorker(true);
  });
}

if (graphSpecCopy) {
  graphSpecCopy.addEventListener('click', () => {
    const spec = getActiveGraphSpec();
    if (graphSpecInput) {
      graphSpecInput.value = JSON.stringify(spec, null, 2);
    }
    setGraphSpecStatus('Copied active graph into the editor.');
  });
}

if (graphSpecExport) {
  graphSpecExport.addEventListener('click', () => {
    const spec = getActiveGraphSpec();
    exportJsonToFile(spec, 'slither_neuroevo_graph_spec.json');
    setGraphSpecStatus('Exported active graph spec.');
  });
}

let currentFrameBuffer: Float32Array | null = null;
let currentStats: FrameStats = { gen: 1, alive: 0, fps: 60 };
let fitnessHistory: FitnessHistoryUiEntry[] = []; // Track fitness over generations for charts
let godModeLog: GodModeLogEntry[] = []; // Track God Mode interactions
let selectedSnake: SelectedSnake | null = null; // Currently selected snake for God Mode
let isDragging = false;

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
    if (!worker) return;
    worker.postMessage({ type: 'action', action: 'toggleView' });
    proxyWorld.viewMode = proxyWorld.viewMode === 'overview' ? 'follow' : 'overview';
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

function normalizeStackOrder(order: string[]): Array<'gru' | 'lstm' | 'rru'> {
  const cleaned = order
    .map(key => key.toLowerCase())
    .filter((key): key is 'gru' | 'lstm' | 'rru' => key === 'gru' || key === 'lstm' || key === 'rru')
    .filter((key, idx, arr) => arr.indexOf(key) === idx);
  if (!cleaned.length) return DEFAULT_STACK_ORDER.slice();
  return cleaned;
}

function loadStoredStackOrder(): void {
  try {
    const raw = localStorage.getItem(STACK_ORDER_STORAGE_KEY);
    if (!raw) return;
    const parsed = JSON.parse(raw);
    if (Array.isArray(parsed)) stackOrder = normalizeStackOrder(parsed);
  } catch {
    stackOrder = DEFAULT_STACK_ORDER.slice();
  }
}

function storeStackOrder(): void {
  try {
    localStorage.setItem(STACK_ORDER_STORAGE_KEY, JSON.stringify(stackOrder));
  } catch {
    // Ignore storage failures.
  }
}

function renderStackOrder(): void {
  if (!stackOrderList) return;
  stackOrderList.innerHTML = '';
  stackOrder.forEach((key, idx) => {
    const row = document.createElement('div');
    row.className = 'stack-order-item';
    const label = document.createElement('div');
    label.className = 'stack-order-label';
    label.textContent = key.toUpperCase();
    const up = document.createElement('button');
    up.className = 'stack-order-btn';
    up.textContent = 'Up';
    up.disabled = idx === 0;
    const down = document.createElement('button');
    down.className = 'stack-order-btn';
    down.textContent = 'Down';
    down.disabled = idx === stackOrder.length - 1;
    up.addEventListener('click', () => {
      if (idx === 0) return;
      const next = stackOrder.slice();
      const swap = next[idx - 1];
      next[idx - 1] = next[idx]!;
      next[idx] = swap!;
      stackOrder = normalizeStackOrder(next);
      if (CFG.brain) CFG.brain.stackOrder = stackOrder.slice();
      storeStackOrder();
      renderStackOrder();
    });
    down.addEventListener('click', () => {
      if (idx >= stackOrder.length - 1) return;
      const next = stackOrder.slice();
      const swap = next[idx + 1];
      next[idx + 1] = next[idx]!;
      next[idx] = swap!;
      stackOrder = normalizeStackOrder(next);
      if (CFG.brain) CFG.brain.stackOrder = stackOrder.slice();
      storeStackOrder();
      renderStackOrder();
    });
    row.appendChild(label);
    row.appendChild(up);
    row.appendChild(down);
    stackOrderList.appendChild(row);
  });
}

function getActiveGraphSpec(): GraphSpec {
  if (customGraphSpec) return customGraphSpec;
  const settings = readSettingsFromCoreUI();
  const stack = readStackToggles();
  const useMlp = readUseMlpToggle();
  return buildStackGraphSpec(settings, {
    brain: {
      ...CFG.brain,
      useMlp,
      stack,
      stackOrder
    }
  });
}

function setGraphSpecStatus(text: string, isError = false): void {
  if (!graphSpecStatus) return;
  graphSpecStatus.textContent = text;
  graphSpecStatus.style.color = isError ? '#ff9b9b' : '';
}

function applyCustomGraphSpec(spec: GraphSpec, note = 'Using custom graph spec (layout toggles ignored).'): void {
  customGraphSpec = spec;
  CFG.brain.graphSpec = spec;
  if (graphSpecInput) graphSpecInput.value = JSON.stringify(spec, null, 2);
  try {
    localStorage.setItem(GRAPH_SPEC_STORAGE_KEY, JSON.stringify(spec));
  } catch {
    // Ignore storage failures.
  }
  setGraphSpecStatus(note);
}

function clearCustomGraphSpec(note = 'Using layout controls.'): void {
  customGraphSpec = null;
  CFG.brain.graphSpec = null;
  try {
    localStorage.removeItem(GRAPH_SPEC_STORAGE_KEY);
  } catch {
    // Ignore storage failures.
  }
  setGraphSpecStatus(note);
}

function updateSavedPresetSelect(presets: SavedGraphPreset[], selectedId?: number): void {
  if (!graphPresetSaved) return;
  const current = selectedId ?? Number(graphPresetSaved.value);
  graphPresetSaved.innerHTML = '';
  if (!presets.length) {
    const option = document.createElement('option');
    option.value = '';
    option.textContent = 'No saved presets';
    graphPresetSaved.appendChild(option);
    return;
  }
  presets.forEach(preset => {
    const option = document.createElement('option');
    option.value = String(preset.id);
    option.textContent = preset.name;
    if (preset.id === current) option.selected = true;
    graphPresetSaved.appendChild(option);
  });
}

async function refreshSavedPresets(selectedId?: number): Promise<void> {
  if (!graphPresetSaved) return;
  const baseUrl = resolveServerHttpUrl();
  if (!baseUrl) {
    updateSavedPresetSelect([], selectedId);
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
      updateSavedPresetSelect([], selectedId);
      return;
    }
    updateSavedPresetSelect(data.presets ?? [], selectedId);
  } catch (err) {
    console.warn('[graph-presets] load failed', err);
    setGraphSpecStatus('Failed to load saved presets.', true);
    updateSavedPresetSelect([], selectedId);
  }
}

function loadStoredGraphSpec(): void {
  try {
    const raw = localStorage.getItem(GRAPH_SPEC_STORAGE_KEY);
    if (!raw) return;
    const parsed = JSON.parse(raw) as GraphSpec;
    const result = validateGraph(parsed);
    if (result.ok) {
      customGraphSpec = parsed;
      CFG.brain.graphSpec = parsed;
      if (graphSpecInput) graphSpecInput.value = JSON.stringify(parsed, null, 2);
      setGraphSpecStatus('Using custom graph spec (loaded).');
    } else {
      clearCustomGraphSpec('Invalid graph spec in storage. Using layout controls.');
    }
  } catch {
    clearCustomGraphSpec('Invalid graph spec in storage. Using layout controls.');
  }
}

type StackPreset = {
  useMlp: boolean;
  stack: { gru: number; lstm: number; rru: number };
};

function applyStackPreset(preset: StackPreset): void {
  const stack = preset.stack;
  if (elUseMlp) elUseMlp.checked = preset.useMlp;
  if (elStackGru) elStackGru.checked = !!stack.gru;
  if (elStackLstm) elStackLstm.checked = !!stack.lstm;
  if (elStackRru) elStackRru.checked = !!stack.rru;
  if (CFG.brain) {
    CFG.brain.useMlp = preset.useMlp;
    if (CFG.brain.stack) {
      CFG.brain.stack.gru = stack.gru;
      CFG.brain.stack.lstm = stack.lstm;
      CFG.brain.stack.rru = stack.rru;
    }
  }
}

function readStackToggles(): { gru: number; lstm: number; rru: number } {
  return {
    gru: elStackGru?.checked ? 1 : 0,
    lstm: elStackLstm?.checked ? 1 : 0,
    rru: elStackRru?.checked ? 1 : 0
  };
}

function readUseMlpToggle(): boolean {
  if (elUseMlp) return elUseMlp.checked;
  const useMlp = CFG.brain?.useMlp;
  return useMlp == null ? true : Boolean(useMlp);
}

function buildPresetSpec(presetId: string): StackPreset | null {
  const presets: Record<string, StackPreset> = {
    mlp: { useMlp: true, stack: { gru: 0, lstm: 0, rru: 0 } },
    'mlp-gru': { useMlp: true, stack: { gru: 1, lstm: 0, rru: 0 } },
    'mlp-lstm': { useMlp: true, stack: { gru: 0, lstm: 1, rru: 0 } },
    'mlp-rru': { useMlp: true, stack: { gru: 0, lstm: 0, rru: 1 } },
    'mlp-gru-lstm': { useMlp: true, stack: { gru: 1, lstm: 1, rru: 0 } },
    'mlp-gru-rru': { useMlp: true, stack: { gru: 1, lstm: 0, rru: 1 } },
    'mlp-gru-lstm-rru': { useMlp: true, stack: { gru: 1, lstm: 1, rru: 1 } },
    gru: { useMlp: false, stack: { gru: 1, lstm: 0, rru: 0 } },
    lstm: { useMlp: false, stack: { gru: 0, lstm: 1, rru: 0 } },
    rru: { useMlp: false, stack: { gru: 0, lstm: 0, rru: 1 } },
    'gru-lstm': { useMlp: false, stack: { gru: 1, lstm: 1, rru: 0 } },
    'gru-rru': { useMlp: false, stack: { gru: 1, lstm: 0, rru: 1 } },
    'gru-lstm-rru': { useMlp: false, stack: { gru: 1, lstm: 1, rru: 1 } }
  };
  return presets[presetId] ?? null;
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

function buildSkipExampleSpec(): GraphSpec {
  const settings = readSettingsFromCoreUI();
  const hiddenSizes = buildHiddenSizesForExample(settings);
  const inputSize = CFG.brain.inSize;
  const outputSize = CFG.brain.outSize;
  const featureSize = Math.max(2, Math.floor(hiddenSizes[hiddenSizes.length - 1] ?? 8));
  const mlpHidden = hiddenSizes.slice(0, -1);
  const concatSize = featureSize * 2;
  return {
    type: 'graph',
    outputSize,
    nodes: [
      { id: 'input', type: 'Input', outputSize: inputSize },
      { id: 'mlp', type: 'MLP', inputSize, outputSize: featureSize, hiddenSizes: mlpHidden },
      { id: 'skip', type: 'Dense', inputSize, outputSize: featureSize },
      { id: 'concat', type: 'Concat' },
      { id: 'head', type: 'Dense', inputSize: concatSize, outputSize }
    ],
    edges: [
      { from: 'input', to: 'mlp' },
      { from: 'input', to: 'skip' },
      { from: 'mlp', to: 'concat', toPort: 0 },
      { from: 'skip', to: 'concat', toPort: 1 },
      { from: 'concat', to: 'head' }
    ],
    outputs: [{ nodeId: 'head' }]
  };
}

function buildSplitExampleSpec(): GraphSpec {
  const settings = readSettingsFromCoreUI();
  const hiddenSizes = buildHiddenSizesForExample(settings);
  const inputSize = CFG.brain.inSize;
  const outputSize = CFG.brain.outSize;
  const baseSize = Math.max(2, Math.floor(hiddenSizes[hiddenSizes.length - 1] ?? 8));
  const splitSize = Math.max(2, baseSize);
  const splitA = Math.max(1, Math.floor(splitSize / 2));
  const splitB = Math.max(1, splitSize - splitA);
  const branchOutA = Math.max(1, Math.floor(outputSize / 2));
  const branchOutB = Math.max(1, outputSize - branchOutA);
  return {
    type: 'graph',
    outputSize,
    nodes: [
      { id: 'input', type: 'Input', outputSize: inputSize },
      { id: 'mlp', type: 'MLP', inputSize, outputSize: splitSize, hiddenSizes: hiddenSizes.slice(0, -1) },
      { id: 'split', type: 'Split', outputSizes: [splitA, splitB] },
      { id: 'headA', type: 'Dense', inputSize: splitA, outputSize: branchOutA },
      { id: 'headB', type: 'Dense', inputSize: splitB, outputSize: branchOutB },
      { id: 'concat', type: 'Concat' }
    ],
    edges: [
      { from: 'input', to: 'mlp' },
      { from: 'mlp', to: 'split' },
      { from: 'split', to: 'headA', fromPort: 0 },
      { from: 'split', to: 'headB', fromPort: 1 },
      { from: 'headA', to: 'concat', toPort: 0 },
      { from: 'headB', to: 'concat', toPort: 1 }
    ],
    outputs: [{ nodeId: 'concat' }]
  };
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

function initWorker(resetCfg = true): void {
  if (!worker) return;
  const settings = readSettingsFromCoreUI();
  const updates = collectSettingsUpdates(settingsControls ?? settingsContainer);
  if (CFG.brain) CFG.brain.stackOrder = stackOrder.slice();
  worker.postMessage({
    type: 'init',
    settings,
    updates,
    resetCfg,
    viewW: cssW,
    viewH: cssH,
    graphSpec: customGraphSpec,
    stackOrder
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
    setConnectionStatus('server');
    joinPending = false;
    wsClient?.sendJoin('spectator');
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
if (elUseMlp) {
  elUseMlp.addEventListener('input', refreshCoreUIState);
}

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
  if (elStackGru) elStackGru.checked = !!CFG.brain?.stack?.gru;
  if (elStackLstm) elStackLstm.checked = !!CFG.brain?.stack?.lstm;
  if (elStackRru) elStackRru.checked = !!CFG.brain?.stack?.rru;
  if (elUseMlp) {
    const useMlp = CFG.brain?.useMlp;
    elUseMlp.checked = useMlp == null ? true : Boolean(useMlp);
  }
  stackOrder = DEFAULT_STACK_ORDER.slice();
  if (CFG.brain) CFG.brain.stackOrder = stackOrder.slice();
  storeStackOrder();
  renderStackOrder();
  clearCustomGraphSpec('Using layout controls.');
  if (graphSpecInput) graphSpecInput.value = '';
  if (graphPresetSelect) graphPresetSelect.value = 'mlp';
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
  if (currentFrameBuffer && currentFrameBuffer.length >= 6) {
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
      // Camera/zoom come from the worker buffer; avoid local overrides here.
      renderWorldStruct(ctx, currentFrameBuffer, cssW, cssH);
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
