/** Entry point for the simulation UI, canvas, and client networking. */

import { CFG, resetCFGToDefaults } from './config.ts';
import {
  BASELINE_BOT_SEED_HINT_ID,
  BASELINE_BOT_SEED_INPUT_ID,
  BASELINE_BOT_SEED_RANDOMIZE_ID,
  applyValuesToSlidersFromCFG,
  setupSettingsUI,
  updateCFGFromUI
} from './settings.ts';
import { lerp, setByPath } from './utils.ts';
import { renderWorldStruct } from './render.ts';
// import { World } from './world.ts'; // Logic moved to worker
import {
  exportJsonToFile,
  exportToFile,
  importFromFile,
  loadBaselineBotSettings,
  saveBaselineBotSettings,
  savePopulationJSON,
  type PopulationFilePayload
} from './storage.ts';
import { hof } from './hallOfFame.ts';
import { BrainViz } from './BrainViz.ts';
import { AdvancedCharts } from './chartUtils.ts';
import { FRAME_HEADER_FLOATS, FRAME_HEADER_OFFSETS } from './protocol/frame.ts';
import { createWsClient, resolveServerUrl, storeServerUrl } from './net/wsClient.ts';
import { inferGraphSizes } from './brains/graph/editor.ts';
import type { GraphSizeState } from './brains/graph/editor.ts';
import { validateGraph } from './brains/graph/validate.ts';
import type { GraphEdge, GraphNodeSpec, GraphNodeType, GraphSpec } from './brains/graph/schema.ts';
import type { FrameStats, HallOfFameEntry, VizData, WorkerToMainMessage } from './protocol/messages.ts';
import { SETTINGS_PATHS } from './protocol/settings.ts';
import type { CoreSettings, SettingsUpdate } from './protocol/settings.ts';

/** Minimal world interface exposed to UI panels and HoF actions. */
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

/** UI-friendly fitness history entry used by charts. */
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

/** God mode log entry shown in the UI panel. */
interface GodModeLogEntry {
  time: number;
  action: string;
  snakeId: number;
  result: string;
}

/** Selected snake snapshot used by God Mode interactions. */
interface SelectedSnake {
  id: number;
  x: number;
  y: number;
  radius: number;
  skin: number;
}

/** Connection mode used by the main UI. */
type ConnectionMode = 'connecting' | 'server' | 'worker';

declare global {
  /** Global window extensions used by the UI. */
  interface Window {
    ctx: CanvasRenderingContext2D;
    currentWorld: ProxyWorld;
    spawnHoF: (idx: number) => void;
  }
}

/** Active worker instance when running locally. */
let worker: Worker | null = null;
/** WebSocket client when connected to the server. */
let wsClient: ReturnType<typeof createWsClient> | null = null;
/** Current connection mode state. */
let connectionMode: ConnectionMode = 'connecting';
/** Current server URL used for connection attempts. */
let serverUrl = '';
/** Latest server config hash from the welcome message. */
let serverCfgHash: string | null = null;
/** Latest server world seed from the welcome message. */
let serverWorldSeed: number | null = null;
/** Latest tick id observed from server stats. */
let lastServerTick = 0;
/** Pending server reset promise used to sequence imports. */
let pendingServerReset: {
  priorTick: number;
  resolve: () => void;
  timeoutId: number;
  promise: Promise<void>;
} | null = null;
/** Backoff delay for reconnection attempts in ms. */
let reconnectDelayMs = 1000;
/** Last player nickname used for reconnecting control. */
let lastPlayerName = '';
/** Local storage key for the player nickname. */
const PLAYER_NAME_KEY = 'slither_neuroevo_player_name';
/** Timer id for reconnect scheduling. */
let reconnectTimer: number | null = null;
/** Timer id for worker fallback scheduling. */
let fallbackTimer: number | null = null;
/** Whether settings controls are locked. */
let settingsLocked = true;
/** Whether join overlay is awaiting user action. */
let joinPending = false;
/** Current player snake id when controlling. */
let playerSnakeId: number | null = null;
/** Tick id of the most recent player sensor packet. */
let playerSensorTick = 0;
/** Latest player sensor metadata for UI overlays. */
let playerSensorMeta: { x: number; y: number; dir: number } | null = null;
/** Current pointer position in world coordinates. */
let pointerWorld: { x: number; y: number } | null = null;
/** Whether boost is held down by input. */
let boostHeld = false;
/** Local storage key for graph spec persistence. */
const GRAPH_SPEC_STORAGE_KEY = 'slither_neuroevo_graph_spec';
/** Set of valid settings update paths for import validation. */
const SETTINGS_PATH_SET = new Set(SETTINGS_PATHS);
/** Applied custom graph spec used for resets. */
let customGraphSpec: GraphSpec | null = null;
/** Current graph editor draft spec. */
let graphDraft: GraphSpec | null = null;
/** Diagram node width in pixels. */
const DIAGRAM_NODE_WIDTH = 140;
/** Diagram node height in pixels. */
const DIAGRAM_NODE_HEIGHT = 44;
/** Per-node layout overrides for the graph diagram. */
const graphLayoutOverrides = new Map<string, { x: number; y: number }>();
/** Selected node id in the graph editor. */
let graphSelectedNodeId: string | null = null;
/** Selected edge index in the graph editor. */
let graphSelectedEdgeIndex: number | null = null;
/** Selected output index in the graph editor. */
let graphSelectedOutputIndex: number | null = null;
/** Drag state for graph node positioning. */
let graphDragState: { id: string; offsetX: number; offsetY: number } | null = null;
/** Pointer position within the graph diagram. */
let graphPointerPos: { x: number; y: number } | null = null;
/** Active drag-to-connect state for the graph diagram. */
let graphConnectDrag: { fromId: string; pointerId: number } | null = null;
/** Current hover target during a connect drag. */
let graphConnectHoverId: string | null = null;
/** Live SVG path for rendering a connection drag. */
let graphConnectLine: SVGPathElement | null = null;
/** Cached bounds for diagram nodes used in hit testing. */
const graphDiagramNodeRects = new Map<string, { x: number; y: number; width: number; height: number }>();
/** Latest inferred graph size state for the editor. */
let graphSizeState: GraphSizeState | null = null;
/** Main canvas element. */
const canvas = document.getElementById('c') as HTMLCanvasElement;
// HUD removed, using tab info panels instead
// Expose the rendering context globally so render helpers can draw.
/** 2D rendering context for the main canvas. */
const ctx = canvas.getContext('2d')!;
window.ctx = ctx;
/** Cached CSS width, height, and device pixel ratio. */
let cssW = 0,
  cssH = 0,
  dpr = 1;

/**
 * Resize the canvas and notify the worker of the new viewport size.
 */
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

/** Slider input for snake count. */
const elSnakes = document.getElementById('snakes') as HTMLInputElement;
/** Slider input for simulation speed. */
const elSimSpeed = document.getElementById('simSpeed') as HTMLInputElement;
/** Slider input for hidden layer count. */
const elLayers = document.getElementById('layers') as HTMLInputElement;
/** Slider input for neurons in layer 1. */
const elN1 = document.getElementById('n1') as HTMLInputElement;
/** Slider input for neurons in layer 2. */
const elN2 = document.getElementById('n2') as HTMLInputElement;
/** Slider input for neurons in layer 3. */
const elN3 = document.getElementById('n3') as HTMLInputElement;
/** Slider input for neurons in layer 4. */
const elN4 = document.getElementById('n4') as HTMLInputElement;
/** Slider input for neurons in layer 5. */
const elN5 = document.getElementById('n5') as HTMLInputElement;
/** Graph node list container. */
const graphNodes = document.getElementById('graphNodes') as HTMLElement | null;
/** Graph edge list container. */
const graphEdges = document.getElementById('graphEdges') as HTMLElement | null;
/** Graph output list container. */
const graphOutputs = document.getElementById('graphOutputs') as HTMLElement | null;
/** Select input for the simple output node picker. */
const graphOutputSimpleNode = document.getElementById('graphOutputNode') as HTMLSelectElement | null;
/** Checkbox toggle for split outputs in the simple output picker. */
const graphOutputSimpleSplit = document.getElementById('graphOutputSplit') as HTMLInputElement | null;
/** Hint element for simple output selection. */
const graphOutputSimpleHint = document.getElementById('graphOutputHint') as HTMLElement | null;
/** Button to add a graph node. */
const graphNodeAdd = document.getElementById('graphNodeAdd') as HTMLButtonElement | null;
/** Button to add a graph edge. */
const graphEdgeAdd = document.getElementById('graphEdgeAdd') as HTMLButtonElement | null;
/** Button to add a graph output. */
const graphOutputAdd = document.getElementById('graphOutputAdd') as HTMLButtonElement | null;
/** Button to apply the current graph spec. */
const graphApply = document.getElementById('graphApply') as HTMLButtonElement | null;
/** Button to reset the graph editor to defaults. */
const graphReset = document.getElementById('graphReset') as HTMLButtonElement | null;
/** Container listing saved graph presets. */
const graphPresetList = document.getElementById('graphPresetList') as HTMLElement | null;
/** Buttons that apply built-in graph templates. */
const graphTemplateButtons = document.querySelectorAll<HTMLButtonElement>('[data-template]');
/** Input for naming a graph preset. */
const graphPresetName = document.getElementById('graphPresetName') as HTMLInputElement | null;
/** Button to save a graph preset. */
const graphPresetSave = document.getElementById('graphPresetSave') as HTMLButtonElement | null;
/** Textarea for JSON graph spec editing. */
const graphSpecInput = document.getElementById('graphSpecInput') as HTMLTextAreaElement | null;
/** Button to apply JSON graph spec input. */
const graphSpecApply = document.getElementById('graphSpecApply') as HTMLButtonElement | null;
/** Button to copy the current graph spec to clipboard. */
const graphSpecCopy = document.getElementById('graphSpecCopy') as HTMLButtonElement | null;
/** Button to export the graph spec as JSON. */
const graphSpecExport = document.getElementById('graphSpecExport') as HTMLButtonElement | null;
/** Status element for graph spec operations. */
const graphSpecStatus = document.getElementById('graphSpecStatus') as HTMLElement | null;
/** Status element for graph size inference warnings. */
const graphSizeHint = document.getElementById('graphSizeHint') as HTMLElement | null;
/** Wrapper for the SVG graph diagram. */
const graphDiagramWrap = document.getElementById('graphDiagramWrap') as HTMLElement | null;
/** SVG element for the graph diagram. */
const graphDiagram = document.getElementById('graphDiagram') as SVGSVGElement | null;
/** Button to toggle full-screen diagram mode. */
const graphDiagramToggle = document.getElementById('graphDiagramToggle') as HTMLButtonElement | null;
/** Backdrop element for full-screen diagram mode. */
const graphDiagramBackdrop = document.getElementById('graphDiagramBackdrop') as HTMLDivElement | null;
/** Button to add a node from the diagram toolbar. */
const graphDiagramAddNode = document.getElementById('graphDiagramAddNode') as HTMLButtonElement | null;
/** Button to add an output from the diagram toolbar. */
const graphDiagramAddOutput = document.getElementById('graphDiagramAddOutput') as HTMLButtonElement | null;
/** Button to auto-layout the diagram. */
const graphDiagramAuto = document.getElementById('graphDiagramAuto') as HTMLButtonElement | null;
/** Button to delete the selected node or edge. */
const graphDiagramDelete = document.getElementById('graphDiagramDelete') as HTMLButtonElement | null;
/** Inspector panel for graph diagram selection. */
const graphDiagramInspector = document.getElementById('graphDiagramInspector') as HTMLDivElement | null;
/** Label showing the current snake count slider value. */
const snakesVal = document.getElementById('snakesVal') as HTMLElement;
/** Label showing the current sim speed slider value. */
const simSpeedVal = document.getElementById('simSpeedVal') as HTMLElement;
/** Label showing the current layer count slider value. */
const layersVal = document.getElementById('layersVal') as HTMLElement;
/** Label showing the current layer 1 size slider value. */
const n1Val = document.getElementById('n1Val') as HTMLElement;
/** Label showing the current layer 2 size slider value. */
const n2Val = document.getElementById('n2Val') as HTMLElement;
/** Label showing the current layer 3 size slider value. */
const n3Val = document.getElementById('n3Val') as HTMLElement;
/** Label showing the current layer 4 size slider value. */
const n4Val = document.getElementById('n4Val') as HTMLElement;
/** Label showing the current layer 5 size slider value. */
const n5Val = document.getElementById('n5Val') as HTMLElement;
/** Button to apply core slider settings. */
const btnApply = document.getElementById('apply') as HTMLButtonElement;
/** Button to restore default settings. */
const btnDefaults = document.getElementById('defaults') as HTMLButtonElement;
/** Button to toggle the settings panel. */
const btnToggle = document.getElementById('toggle') as HTMLButtonElement;
/** Settings tab container element. */
const settingsContainer = document.getElementById('settingsContainer') as HTMLElement;
/** Baseline bot seed input element (rebuilt with settings UI). */
let baselineBotSeedInput: HTMLInputElement | null = null;
/** Baseline bot seed validation hint element (rebuilt with settings UI). */
let baselineBotSeedHint: HTMLElement | null = null;
/** Baseline bot seed randomize button (rebuilt with settings UI). */
let baselineBotSeedRandomize: HTMLButtonElement | null = null;
/** Connection status badge element. */
const connectionStatus = document.getElementById('connectionStatus') as HTMLElement | null;
/** Join overlay element shown before player entry. */
const joinOverlay = document.getElementById('joinOverlay') as HTMLElement | null;
/** Join overlay name input. */
const joinName = document.getElementById('joinName') as HTMLInputElement | null;
/** Join overlay play button. */
const joinPlay = document.getElementById('joinPlay') as HTMLButtonElement | null;
/** Join overlay spectate button. */
const joinSpectate = document.getElementById('joinSpectate') as HTMLButtonElement | null;
/** Join overlay status text element. */
const joinStatus = document.getElementById('joinStatus') as HTMLElement | null;
/** Button to lock or unlock settings. */
const toggleSettingsLock = document.getElementById('toggleSettingsLock') as HTMLButtonElement | null;
/** Wrapper for slider controls that are lockable. */
const settingsControls = document.getElementById('settingsControls') as HTMLElement | null;
/** Settings tab content element. */
const settingsTab = document.getElementById('tab-settings') as HTMLElement | null;
/** Hint text for settings lock state. */
const settingsLockHint = document.getElementById('settingsLockHint') as HTMLElement | null;
/** Hint text for graph mode slider overrides. */
const graphModeHint = document.getElementById('graphModeHint') as HTMLElement | null;

/** Tab button elements for the control panel. */
const tabBtns = document.querySelectorAll<HTMLButtonElement>('.tab-btn');
/** Tab content elements corresponding to buttons. */
const tabContents = document.querySelectorAll<HTMLElement>('.tab-content');
/** Canvas element for brain visualizer rendering. */
const vizCanvas = document.getElementById('vizCanvas') as HTMLCanvasElement;
/** Canvas element for stats chart rendering. */
const statsCanvas = document.getElementById('statsCanvas') as HTMLCanvasElement;
/** 2D context for the visualizer canvas. */
const ctxViz = vizCanvas.getContext('2d')!;
/** 2D context for the stats canvas. */
const ctxStats = statsCanvas.getContext('2d')!;

/**
 * Load a previously used player nickname from localStorage.
 */
function loadSavedPlayerName(): void {
  if (!joinName) return;
  try {
    const saved = localStorage.getItem(PLAYER_NAME_KEY);
    if (saved && !joinName.value.trim()) {
      lastPlayerName = saved;
      joinName.value = saved;
    } else if (saved) {
      lastPlayerName = saved;
    }
  } catch {
    // Ignore storage failures in non-browser environments.
  }
}

/** Brain visualizer instance for the Viz tab. */
const brainViz = new BrainViz(0, 0, vizCanvas.width, vizCanvas.height);
/** Currently active tab id. */
let activeTab = 'tab-settings';
/** Currently selected stats view key. */
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

/** Buttons for switching between stats views. */
const statsViewBtns = document.querySelectorAll<HTMLButtonElement>('.stats-view-btn');
/** Title element for the stats panel. */
const statsTitle = document.getElementById('statsTitle') as HTMLElement | null;
/** Subtitle element for the stats panel. */
const statsSubtitle = document.getElementById('statsSubtitle') as HTMLElement | null;
/** Mapping of stats view keys to titles/subtitles. */
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

/**
 * Switch the stats view and update UI labels.
 * @param view - Stats view key to activate.
 */
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
loadSavedPlayerName();
if (joinPlay) {
  joinPlay.addEventListener('click', () => {
    if (!joinName) return;
    const name = joinName.value.trim();
    if (!name) return;
    lastPlayerName = name;
    try {
      localStorage.setItem(PLAYER_NAME_KEY, name);
    } catch {
      // Ignore storage failures in non-browser environments.
    }
    joinPending = true;
    setJoinStatus('Joining...');
    updateJoinControls();
    wsClient?.sendJoin('player', name);
    wsClient?.sendView({ mode: 'follow', viewW: cssW, viewH: cssH });
  });
}
if (joinSpectate) {
  joinSpectate.addEventListener('click', () => {
    enterSpectatorMode();
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

/**
 * Parse a baseline bot seed from a raw input string.
 * @param value - Raw input value.
 * @returns Normalized seed value or null when invalid.
 */
function parseBaselineSeedValue(value: string): number | null {
  const parsed = Number(value);
  if (!Number.isFinite(parsed) || !Number.isInteger(parsed)) return null;
  return Math.max(0, parsed);
}

/**
 * Toggle the baseline seed validation hint.
 * @param visible - Whether to show the hint.
 */
function setBaselineSeedHintVisible(visible: boolean): void {
  if (!baselineBotSeedHint) return;
  baselineBotSeedHint.style.display = visible ? 'block' : 'none';
  baselineBotSeedHint.classList.toggle('invalid', visible);
}

/**
 * Update the baseline seed UI value label.
 * @param seed - Seed value to display.
 */
function updateBaselineSeedLabel(seed: number): void {
  const output = document.getElementById('val_baselineBots_seed');
  if (output) output.textContent = String(seed);
}

/**
 * Read baseline bot settings from CFG with basic clamping.
 * @returns Normalized baseline bot settings.
 */
function readBaselineBotSettingsFromCfg(): {
  count: number;
  seed: number;
  randomizeSeedPerGen: boolean;
  respawnDelay: number;
} {
  const rawCount = CFG.baselineBots?.count ?? 0;
  const rawSeed = CFG.baselineBots?.seed ?? 0;
  const count = Number.isFinite(rawCount) ? Math.max(0, Math.floor(rawCount)) : 0;
  const seed = Number.isFinite(rawSeed) ? Math.max(0, Math.floor(rawSeed)) : 0;
  const randomizeSeedPerGen = Boolean(CFG.baselineBots?.randomizeSeedPerGen);
  const respawnDelay = CFG.baselineBots?.respawnDelay ?? 3.0;
  return { count, seed, randomizeSeedPerGen, respawnDelay };
}

/**
 * Persist baseline bot settings to localStorage.
 */
function persistBaselineBotSettings(): void {
  const settings = readBaselineBotSettingsFromCfg();
  saveBaselineBotSettings(settings);
}

/**
 * Apply stored baseline bot settings to CFG before UI initialization.
 */
function applyStoredBaselineBotSettings(): void {
  const stored = loadBaselineBotSettings();
  if (!stored) return;
  setByPath(CFG, 'baselineBots.count', stored.count);
  setByPath(CFG, 'baselineBots.seed', stored.seed);
  setByPath(CFG, 'baselineBots.randomizeSeedPerGen', stored.randomizeSeedPerGen);
  if (stored.respawnDelay != null && Number.isFinite(stored.respawnDelay)) {
    setByPath(CFG, 'baselineBots.respawnDelay', stored.respawnDelay);
  }
}

/**
 * Apply a baseline seed value to the UI and active settings.
 * @param seed - Finite non-negative integer seed.
 */
function applyBaselineSeed(seed: number): void {
  if (!Number.isFinite(seed) || !Number.isInteger(seed) || seed < 0) return;
  setByPath(CFG, 'baselineBots.seed', seed);
  if (baselineBotSeedInput) {
    baselineBotSeedInput.value = String(seed);
  }
  updateBaselineSeedLabel(seed);
  setBaselineSeedHintVisible(false);
  persistBaselineBotSettings();
  if (worker) {
    worker.postMessage({
      type: 'updateSettings',
      updates: [{ path: 'baselineBots.seed', value: seed }]
    });
    return;
  }
  if (wsClient && wsClient.isConnected()) {
    const settings = readSettingsFromCoreUI();
    const updates = collectSettingsUpdatesFromUI();
    wsClient.sendReset(settings, updates, customGraphSpec ?? null);
  }
}

/**
 * Generate a random baseline seed value.
 * @returns New seed value.
 */
function randomizeBaselineSeed(): number {
  return Math.floor(Math.random() * 0x100000000);
}

/**
 * Refresh baseline bot control references after settings UI rebuild.
 */
function wireBaselineBotControls(): void {
  baselineBotSeedInput = document.getElementById(BASELINE_BOT_SEED_INPUT_ID) as HTMLInputElement | null;
  baselineBotSeedHint = document.getElementById(BASELINE_BOT_SEED_HINT_ID) as HTMLElement | null;
  baselineBotSeedRandomize = document.getElementById(BASELINE_BOT_SEED_RANDOMIZE_ID) as HTMLButtonElement | null;
  setBaselineSeedHintVisible(false);
  if (baselineBotSeedInput) {
    baselineBotSeedInput.addEventListener('input', () => {
      const parsed = parseBaselineSeedValue(baselineBotSeedInput?.value ?? '');
      setBaselineSeedHintVisible(parsed == null);
    });
    baselineBotSeedInput.addEventListener('blur', () => {
      const parsed = parseBaselineSeedValue(baselineBotSeedInput?.value ?? '');
      if (parsed == null) {
        setBaselineSeedHintVisible(true);
        const currentSeed = readBaselineBotSettingsFromCfg().seed;
        updateBaselineSeedLabel(currentSeed);
        if (baselineBotSeedInput) {
          baselineBotSeedInput.value = String(currentSeed);
        }
        return;
      }
      applyBaselineSeed(parsed);
    });
  }
  if (baselineBotSeedRandomize) {
    baselineBotSeedRandomize.addEventListener('click', () => {
      const seed = randomizeBaselineSeed();
      applyBaselineSeed(seed);
    });
  }
}

/**
 * Apply core settings values to the UI sliders.
 * @param settings - Partial core settings to apply to the UI.
 */
function applyCoreSettingsToUi(settings: Partial<CoreSettings>): void {
  if (Number.isFinite(settings.snakeCount)) {
    elSnakes.value = String(settings.snakeCount);
  }
  if (Number.isFinite(settings.simSpeed)) {
    elSimSpeed.value = String(settings.simSpeed);
  }
  if (Number.isFinite(settings.hiddenLayers)) {
    elLayers.value = String(settings.hiddenLayers);
  }
  if (Number.isFinite(settings.neurons1)) {
    elN1.value = String(settings.neurons1);
  }
  if (Number.isFinite(settings.neurons2)) {
    elN2.value = String(settings.neurons2);
  }
  if (Number.isFinite(settings.neurons3)) {
    elN3.value = String(settings.neurons3);
  }
  if (Number.isFinite(settings.neurons4)) {
    elN4.value = String(settings.neurons4);
  }
  if (Number.isFinite(settings.neurons5)) {
    elN5.value = String(settings.neurons5);
  }
  refreshCoreUIState();
}

/**
 * Read a numeric value from a CFG-backed settings input.
 * @param input - Input element to read.
 * @returns Parsed numeric value or null when invalid.
 */
function readSettingsInputValue(input: HTMLInputElement): number | null {
  if (input.type === "checkbox") return input.checked ? 1 : 0;
  if (input.dataset['path'] === 'baselineBots.seed') {
    return parseBaselineSeedValue(input.value);
  }
  const value = Number(input.value);
  if (!Number.isFinite(value)) return null;
  return value;
}

/**
 * Collect slider updates under a root element.
 * @param root - Root element containing settings inputs.
 * @returns List of settings updates.
 */
function collectSettingsUpdates(root: HTMLElement): SettingsUpdate[] {
  const sliders = root.querySelectorAll<HTMLInputElement>('input[data-path]');
  const updates: SettingsUpdate[] = [];
  sliders.forEach(sl => {
    const value = readSettingsInputValue(sl);
    if (value == null) return;
    updates.push({ path: sl.dataset['path']! as SettingsUpdate['path'], value });
  });
  return updates;
}

/**
 * Resolve the root element containing CFG-backed settings sliders.
 * @returns Root element containing data-path inputs.
 */
function resolveSettingsRoot(): HTMLElement {
  const hasContainerInputs =
    typeof settingsContainer?.querySelector === 'function' &&
    settingsContainer.querySelector('input[data-path]') != null;
  if (hasContainerInputs) return settingsContainer;
  return settingsControls ?? settingsContainer;
}

/**
 * Collect settings updates from the active settings root.
 * @returns List of settings updates.
 */
function collectSettingsUpdatesFromUI(): SettingsUpdate[] {
  return collectSettingsUpdates(resolveSettingsRoot());
}

/**
 * Validate and normalize imported settings updates.
 * @param value - Raw updates payload from an import file.
 * @returns Parsed updates array or null when absent.
 * @throws Error when updates contain unknown paths or invalid values.
 */
function normalizeImportUpdates(value: unknown): SettingsUpdate[] | null {
  if (!Array.isArray(value)) return null;
  const normalized: SettingsUpdate[] = [];
  for (const entry of value) {
    if (!entry || typeof entry !== 'object') {
      throw new Error('Invalid settings updates payload.');
    }
    const path = (entry as { path?: unknown }).path;
    const updateValue = (entry as { value?: unknown }).value;
    if (typeof path !== 'string' || !SETTINGS_PATH_SET.has(path as SettingsUpdate['path'])) {
      throw new Error(`Unknown settings path in import: ${String(path)}`);
    }
    if (typeof updateValue !== 'number' || !Number.isFinite(updateValue)) {
      throw new Error(`Invalid settings value for ${String(path)}`);
    }
    normalized.push({ path: path as SettingsUpdate['path'], value: updateValue });
  }
  return normalized;
}

/**
 * Apply settings updates to CFG and sync the settings UI.
 * @param updates - Settings updates to apply.
 */
function applySettingsUpdatesToUi(updates: SettingsUpdate[]): void {
  updates.forEach(update => {
    setByPath(CFG, update.path, update.value);
  });
  applyValuesToSlidersFromCFG(resolveSettingsRoot());
  setBaselineSeedHintVisible(false);
}

/**
 * Apply graph mode styling to stack slider rows and hints.
 * @param graphActive - Whether a custom graph spec is active.
 */
function applyGraphModeUiState(graphActive: boolean): void {
  const sliders = [elLayers, elN1, elN2, elN3, elN4, elN5];
  sliders.forEach(slider => {
    const row = slider.closest('.row');
    if (!row) return;
    row.classList.toggle('graph-stack-disabled', graphActive);
  });
  if (graphModeHint) {
    graphModeHint.textContent = graphActive
      ? 'Custom graph active; stack sliders are ignored.'
      : '';
  }
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
  const graphActive = !!customGraphSpec;
  const disableAll = settingsLocked || graphActive;
  elLayers.disabled = disableAll;
  elN1.disabled = disableAll;
  elN2.disabled = disableAll || L < 2;
  elN3.disabled = disableAll || L < 3;
  elN4.disabled = disableAll || L < 4;
  elN5.disabled = disableAll || L < 5;
  applyGraphModeUiState(graphActive);
  const applyOpacity = (el: HTMLInputElement) => {
    el.style.opacity = el.disabled ? '0.45' : '1';
  };
  applyOpacity(elN1);
  applyOpacity(elN2);
  applyOpacity(elN3);
  applyOpacity(elN4);
  applyOpacity(elN5);
  applyOpacity(elLayers);
}

// Restore baseline bot settings before rendering settings UI.
applyStoredBaselineBotSettings();
// Build dynamic settings UI and initialise defaults
setupSettingsUI(settingsContainer, liveUpdateFromSlider);
wireBaselineBotControls();
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

/**
 * Toggle full screen mode for the graph diagram.
 * @param isFullscreen - Whether fullscreen is enabled.
 */
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
    if (graphConnectDrag && point) {
      updateGraphConnectDrag(point);
    }
  });
  graphDiagram.addEventListener('pointerup', (event) => {
    const point = getSvgPoint(event);
    if (graphConnectDrag) {
      finishGraphConnectDrag(point);
    }
    graphDragState = null;
  });
  graphDiagram.addEventListener('pointerleave', () => {
    graphDragState = null;
    if (graphConnectDrag) {
      clearGraphConnectDrag();
    }
  });
  graphDiagram.addEventListener('click', (event) => {
    const target = event.target as Element | null;
    if (target && target !== graphDiagram) return;
    if (graphConnectDrag) {
      clearGraphConnectDrag();
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

if (graphOutputSimpleNode) {
  graphOutputSimpleNode.addEventListener('change', () => {
    applySimpleOutputSelection();
  });
}

if (graphOutputSimpleSplit) {
  graphOutputSimpleSplit.addEventListener('change', () => {
    applySimpleOutputSelection();
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

/** Latest received frame buffer for rendering. */
let currentFrameBuffer: Float32Array | null = null;
/** Latest stats payload for UI panels. */
let currentStats: FrameStats = {
  gen: 1,
  alive: 0,
  aliveTotal: 0,
  baselineBotsAlive: 0,
  baselineBotsTotal: 0,
  fps: 60
};
/** Fitness history displayed in charts. */
let fitnessHistory: FitnessHistoryUiEntry[] = [];
/** God mode log entries for the UI panel. */
let godModeLog: GodModeLogEntry[] = [];
/** Currently selected snake for God mode actions. */
let selectedSnake: SelectedSnake | null = null;
/** Whether a pointer drag is in progress. */
let isDragging = false;
/** Client-side camera X for overlays and input. */
let clientCamX = 0;
/** Client-side camera Y for overlays and input. */
let clientCamY = 0;
/** Client-side camera zoom for overlays and input. */
let clientZoom = 1;

/** Current visualization payload for the brain viz tab. */
let currentVizData: VizData | null = null;
/** Whether an export request is pending. */
let pendingExport = false;

/** Proxy world exposed to UI helpers and HoF spawn. */
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

/**
 * Update the connection status UI indicator.
 * @param mode - Connection mode to display.
 */
function setConnectionStatus(mode: ConnectionMode): void {
  connectionMode = mode;
  if (!connectionStatus) return;
  connectionStatus.classList.remove('connecting', 'server', 'worker');
  connectionStatus.classList.add(mode);
  if (mode === 'server') connectionStatus.textContent = 'Server';
  else if (mode === 'worker') connectionStatus.textContent = 'Worker';
  else connectionStatus.textContent = 'Connecting';
}

/**
 * Show or hide the join overlay UI.
 * @param visible - Whether the overlay should be visible.
 */
function setJoinOverlayVisible(visible: boolean): void {
  if (!joinOverlay) return;
  joinOverlay.classList.toggle('hidden', !visible);
}

/**
 * Update the join overlay status text.
 * @param text - Status message to display.
 */
function setJoinStatus(text: string): void {
  if (!joinStatus) return;
  joinStatus.textContent = text;
}

/**
 * Enable or disable join controls based on connection state.
 */
function updateJoinControls(): void {
  if (!joinPlay || !joinName) return;
  const connected = wsClient?.isConnected() ?? false;
  const hasName = joinName.value.trim().length > 0;
  joinPlay.disabled = !connected || !hasName || joinPending;
  if (joinSpectate) joinSpectate.disabled = !connected || joinPending;
}

/**
 * Switch the current connection into spectator mode.
 */
function enterSpectatorMode(): void {
  if (!wsClient?.isConnected()) return;
  joinPending = false;
  playerSnakeId = null;
  playerSensorTick = 0;
  playerSensorMeta = null;
  pointerWorld = null;
  boostHeld = false;
  proxyWorld.viewMode = 'overview';
  setJoinStatus('Spectating');
  updateJoinControls();
  wsClient.sendJoin('spectator');
  wsClient.sendView({ mode: 'overview', viewW: cssW, viewH: cssH });
  setJoinOverlayVisible(false);
}

/**
 * Switch the current connection into player mode using the saved nickname.
 */
function enterPlayerMode(): void {
  if (!wsClient?.isConnected()) return;
  const fallbackName = 'player';
  const name = joinName?.value.trim() || lastPlayerName || fallbackName;
  if (joinName && !joinName.value.trim()) {
    joinName.value = name;
  }
  lastPlayerName = name;
  try {
    localStorage.setItem(PLAYER_NAME_KEY, name);
  } catch {
    // Ignore storage failures in non-browser environments.
  }
  joinPending = true;
  setJoinStatus('Joining...');
  updateJoinControls();
  proxyWorld.viewMode = 'follow';
  wsClient.sendJoin('player', name);
  wsClient.sendView({ mode: 'follow', viewW: cssW, viewH: cssH });
}

/** Saved graph preset summary from the server. */
type SavedGraphPreset = { id: number; name: string; createdAt: number };
/** Loaded graph preset including the graph spec. */
type LoadedGraphPreset = SavedGraphPreset & { spec: GraphSpec };

/**
 * Resolve the HTTP base URL for graph preset APIs from the WS URL.
 * @returns HTTP base URL or null when invalid.
 */
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

/**
 * Return the active graph spec (custom, draft, or default).
 * @returns Active graph spec.
 */
function getActiveGraphSpec(): GraphSpec {
  return customGraphSpec ?? graphDraft ?? buildLinearMlpTemplate();
}

/**
 * Return the current draft graph spec, falling back to active/default.
 * @returns Draft graph spec.
 */
function getDraftGraphSpec(): GraphSpec {
  return graphDraft ?? customGraphSpec ?? buildLinearMlpTemplate();
}

/**
 * Clone a graph spec to avoid mutating the original.
 * @param spec - Graph spec to clone.
 * @returns Cloned graph spec.
 */
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

/**
 * Ensure a graph draft exists and return it.
 * @returns Draft graph spec.
 */
function ensureGraphDraft(): GraphSpec {
  if (!graphDraft) {
    graphDraft = cloneGraphSpec(getActiveGraphSpec());
  }
  return graphDraft;
}

/**
 * Update the graph spec status message in the UI.
 * @param text - Status message to display.
 * @param isError - Whether the message indicates an error.
 */
function setGraphSpecStatus(text: string, isError = false): void {
  if (!graphSpecStatus) return;
  graphSpecStatus.textContent = text;
  graphSpecStatus.style.color = isError ? '#ff9b9b' : '';
  if (graphDiagram) {
    updateGraphSizeState(ensureGraphDraft());
    renderGraphDiagram(ensureGraphDraft());
  }
}

/**
 * Update inferred size state and surface sizing errors in the UI.
 * @param spec - Graph spec to analyze.
 * @returns Size inference state for the spec.
 */
function updateGraphSizeState(spec: GraphSpec): GraphSizeState {
  const state = inferGraphSizes(spec);
  graphSizeState = state;
  if (graphSizeHint) {
    if (!state.errors.length) {
      graphSizeHint.textContent = '';
      graphSizeHint.classList.remove('error');
    } else {
      const tail = state.errors.length > 1 ? ` (+${state.errors.length - 1} more)` : '';
      graphSizeHint.textContent = `${state.errors[0]}${tail}`;
      graphSizeHint.classList.add('error');
    }
  }
  return state;
}

/**
 * Apply inferred input sizes back into the draft spec.
 * @param spec - Graph spec to update in-place.
 * @param state - Size inference state for the spec.
 */
function applyGraphSizeInference(spec: GraphSpec, state: GraphSizeState): void {
  spec.outputSize = CFG.brain.outSize;
  spec.nodes.forEach(node => {
    if (node.type === 'Input') {
      node.outputSize = CFG.brain.inSize;
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

/**
 * Normalize viz payloads that arrive from JSON by reconstructing activation arrays.
 * @param viz - Incoming visualization payload.
 * @returns Normalized visualization payload or null.
 */
function normalizeVizData(viz: VizData | null | undefined): VizData | null {
  if (!viz) return null;
  const layers = viz.layers.map(layer => {
    const activations = layer.activations;
    if (!activations) return layer;
    if (Array.isArray(activations) || ArrayBuffer.isView(activations)) return layer;
    const count = Math.max(0, layer.count);
    const next = new Array<number>(count);
    const source = activations as unknown as Record<string, number>;
    for (let i = 0; i < count; i += 1) {
      const raw = source[i] ?? 0;
      next[i] = Number.isFinite(raw) ? raw : 0;
    }
    return { ...layer, activations: next };
  });
  return { ...viz, layers };
}

/** Simple output selector state derived from the graph spec. */
type SimpleOutputState =
  | { mode: 'simple'; nodeId: string; split: boolean }
  | { mode: 'custom'; nodeId: string | null };

/**
 * Derive the simple output selector state from a graph spec.
 * @param spec - Graph spec to inspect.
 * @returns Simple output selection state.
 */
function resolveSimpleOutputState(spec: GraphSpec): SimpleOutputState {
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
 * Render the simple outputs UI based on the current spec and size state.
 * @param spec - Graph spec to reflect in the UI.
 * @param sizeState - Inferred size state for sizing hints.
 */
function renderSimpleOutputUi(spec: GraphSpec, sizeState: GraphSizeState): void {
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
  updateSimpleOutputHint(state, sizeState);
}

/**
 * Update the simple output hint text based on current selection.
 * @param state - Current simple output selector state.
 * @param sizeState - Inferred size state for the graph.
 */
function updateSimpleOutputHint(state: SimpleOutputState, sizeState: GraphSizeState): void {
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
    if (sizes[0] !== CFG.brain.outSize) {
      graphOutputSimpleHint.textContent = `Output size must be ${CFG.brain.outSize}.`;
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
  if (total !== CFG.brain.outSize) {
    graphOutputSimpleHint.textContent = `Split ports 0+1 must sum to ${CFG.brain.outSize}.`;
    graphOutputSimpleHint.classList.add('error');
    return;
  }
  graphOutputSimpleHint.textContent = 'Port 0 feeds turn, port 1 feeds boost.';
}

/**
 * Apply the simple output selector values to the current draft.
 */
function applySimpleOutputSelection(): void {
  if (!graphOutputSimpleNode || !graphOutputSimpleSplit) return;
  const draft = ensureGraphDraft();
  const nodeId = graphOutputSimpleNode.value;
  if (!nodeId) return;
  if (graphOutputSimpleSplit.checked) {
    draft.outputs = [{ nodeId, port: 0 }, { nodeId, port: 1 }];
  } else {
    draft.outputs = [{ nodeId }];
  }
  renderGraphEditor();
  setGraphSpecStatus('Outputs updated. Apply graph to use it.');
}

/**
 * Set the current graph draft and refresh the editor.
 * @param spec - Graph spec to set as draft.
 * @param note - Optional status note to display.
 */
function setGraphDraft(spec: GraphSpec, note = ''): void {
  graphDraft = cloneGraphSpec(spec);
  renderGraphEditor();
  if (note) setGraphSpecStatus(note);
}

/**
 * Validate and apply a graph spec to CFG and local state.
 * @param spec - Graph spec to apply.
 * @param note - Status message to display on success.
 * @returns True when applied successfully.
 */
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
  refreshCoreUIState();
  renderGraphEditor();
  setGraphSpecStatus(note);
  return true;
}

/**
 * Clear the applied custom graph spec and revert to the default stack graph.
 * @param note - Status message to display after clearing.
 */
function clearCustomGraphSpec(note = 'Custom graph cleared.'): void {
  customGraphSpec = null;
  CFG.brain.graphSpec = null;
  graphDraft = null;
  if (graphSpecInput) {
    graphSpecInput.value = JSON.stringify(buildLinearMlpTemplate(), null, 2);
  }
  try {
    localStorage.removeItem(GRAPH_SPEC_STORAGE_KEY);
  } catch {
    // Ignore storage failures.
  }
  refreshCoreUIState();
  renderGraphEditor();
  setGraphSpecStatus(note);
}

/**
 * Reset the graph draft to the applied or default graph.
 */
function resetGraphDraft(): void {
  if (customGraphSpec) {
    setGraphDraft(customGraphSpec, 'Editor reset to applied graph.');
    return;
  }
  setGraphDraft(buildLinearMlpTemplate(), 'Editor reset to default graph.');
}

/**
 * Render the saved graph preset list in the UI.
 * @param presets - Presets to display.
 */
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

/**
 * Load a saved graph preset by id from the server.
 * @param presetId - Preset id to load.
 */
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

/**
 * Refresh the saved presets list from the server.
 */
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

/**
 * Load the stored graph spec from localStorage or fall back to defaults.
 */
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

/**
 * Build hidden layer sizes from the current core UI settings.
 * @param settings - Core UI settings snapshot.
 * @returns Array of hidden layer sizes.
 */
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

/**
 * Build a linear MLP-only graph template.
 * @returns Graph spec for a linear MLP.
 */
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

/**
 * Build a template with MLP to GRU to MLP stages.
 * @returns Graph spec for MLP-GRU-MLP.
 */
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

/**
 * Build a template with skip connection and concat.
 * @returns Graph spec for a skip-style template.
 */
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

/**
 * Build a template with a split output head.
 * @returns Graph spec for a split output template.
 */
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

/**
 * Build a default node spec for a given node type.
 * @param type - Node type to create.
 * @param id - Node id to assign.
 * @returns Node specification.
 */
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

/**
 * Build a unique node id within a graph spec.
 * @param spec - Graph spec to scan.
 * @param prefix - Prefix to use for the id.
 * @returns Unique node id.
 */
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

/**
 * Clamp a number to a range.
 * @param value - Input value.
 * @param min - Minimum value.
 * @param max - Maximum value.
 * @returns Clamped value.
 */
function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, value));
}

/**
 * Update the current graph selection state.
 * @param next - Next selection state values.
 */
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

/**
 * Sync selection state against the current graph spec.
 * @param spec - Graph spec to validate selection against.
 */
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

/**
 * Map a pointer event to SVG coordinate space.
 * @param evt - Pointer or mouse event.
 * @returns SVG coordinates or null when unavailable.
 */
function getSvgPoint(evt: PointerEvent | MouseEvent): { x: number; y: number } | null {
  if (!graphDiagram) return null;
  const rect = graphDiagram.getBoundingClientRect();
  const viewBox = graphDiagram.viewBox?.baseVal;
  if (!viewBox) return null;
  const x = ((evt.clientX - rect.left) / rect.width) * viewBox.width + viewBox.x;
  const y = ((evt.clientY - rect.top) / rect.height) * viewBox.height + viewBox.y;
  return { x, y };
}

/**
 * Find a graph node under a diagram pointer position.
 * @param point - Pointer position in SVG coordinates.
 * @param excludeId - Optional node id to exclude from hit testing.
 * @returns Node id under the pointer or null.
 */
function findGraphNodeAtPoint(point: { x: number; y: number }, excludeId?: string | null): string | null {
  for (const [id, rect] of graphDiagramNodeRects.entries()) {
    if (excludeId && id === excludeId) continue;
    if (point.x < rect.x || point.x > rect.x + rect.width) continue;
    if (point.y < rect.y || point.y > rect.y + rect.height) continue;
    return id;
  }
  return null;
}

/**
 * Update the highlighted target node during a connect drag.
 * @param nextId - Node id to highlight or null to clear.
 */
function setGraphConnectHover(nextId: string | null): void {
  if (graphConnectHoverId === nextId) return;
  const clearHighlight = (id: string | null) => {
    if (!id || !graphDiagram) return;
    const group = graphDiagram.querySelector(`g[data-node-id="${id}"]`);
    const rect = group?.querySelector('rect');
    rect?.classList.remove('connect-target');
  };
  const applyHighlight = (id: string | null) => {
    if (!id || !graphDiagram) return;
    const group = graphDiagram.querySelector(`g[data-node-id="${id}"]`);
    const rect = group?.querySelector('rect');
    rect?.classList.add('connect-target');
  };
  clearHighlight(graphConnectHoverId);
  graphConnectHoverId = nextId;
  applyHighlight(graphConnectHoverId);
}

/**
 * Update the live connection path during a connect drag.
 * @param point - Pointer position in SVG coordinates.
 * @param targetId - Optional target node id to snap to.
 */
function updateGraphConnectLine(point: { x: number; y: number } | null, targetId?: string | null): void {
  if (!graphConnectLine || !graphConnectDrag || !point) return;
  const fromRect = graphDiagramNodeRects.get(graphConnectDrag.fromId);
  if (!fromRect) return;
  const fromX = fromRect.x + fromRect.width;
  const fromY = fromRect.y + fromRect.height / 2;
  let toX = point.x;
  let toY = point.y;
  if (targetId) {
    const targetRect = graphDiagramNodeRects.get(targetId);
    if (targetRect) {
      toX = targetRect.x;
      toY = targetRect.y + targetRect.height / 2;
    }
  }
  const curve = Math.max(20, (toX - fromX) * 0.35);
  graphConnectLine.setAttribute('d', `M ${fromX} ${fromY} C ${fromX + curve} ${fromY} ${toX - curve} ${toY} ${toX} ${toY}`);
  graphConnectLine.classList.add('visible');
}

/**
 * Clear the live connection line and hover highlight.
 */
function clearGraphConnectDrag(): void {
  if (graphConnectLine) {
    graphConnectLine.classList.remove('visible');
    graphConnectLine.removeAttribute('d');
  }
  setGraphConnectHover(null);
  graphConnectDrag = null;
}

/**
 * Begin a drag-to-connect interaction from a node handle.
 * @param fromId - Node id where the drag originates.
 * @param event - Pointer event initiating the drag.
 */
function beginGraphConnectDrag(fromId: string, event: PointerEvent): void {
  if (!graphDiagram) return;
  graphConnectDrag = { fromId, pointerId: event.pointerId };
  graphDragState = null;
  setGraphSelection({ nodeId: fromId });
  const point = getSvgPoint(event);
  if (point) {
    updateGraphConnectLine(point);
    setGraphConnectHover(findGraphNodeAtPoint(point, fromId));
  }
  graphDiagram.setPointerCapture?.(event.pointerId);
}

/**
 * Update drag-to-connect state for pointer movement.
 * @param point - Pointer position in SVG coordinates.
 */
function updateGraphConnectDrag(point: { x: number; y: number }): void {
  if (!graphConnectDrag) return;
  const targetId = findGraphNodeAtPoint(point, graphConnectDrag.fromId);
  setGraphConnectHover(targetId);
  updateGraphConnectLine(point, targetId);
}

/**
 * Finish a drag-to-connect interaction and create an edge if valid.
 * @param point - Pointer position in SVG coordinates, if available.
 */
function finishGraphConnectDrag(point: { x: number; y: number } | null): void {
  if (!graphConnectDrag) return;
  const fromId = graphConnectDrag.fromId;
  const targetId = point ? findGraphNodeAtPoint(point, fromId) : graphConnectHoverId;
  clearGraphConnectDrag();
  if (!targetId || targetId === fromId) return;
  const spec = ensureGraphDraft();
  const created = addGraphEdge(fromId, targetId, spec);
  if (created != null) {
    setGraphSelection({ edgeIndex: created });
    setGraphSpecStatus('Edge added. Apply graph to use it.');
  }
}

/**
 * Add an edge to the graph spec, honoring split/concat ports.
 * @param fromId - Source node id.
 * @param toId - Target node id.
 * @param spec - Graph spec to mutate.
 * @returns Edge index or null when not created.
 */
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

/**
 * Render the graph diagram SVG from the spec.
 * @param spec - Graph spec to render.
 */
function renderGraphDiagram(spec: GraphSpec): void {
  if (!graphDiagram) return;
  graphDiagram.innerHTML = '';
  graphDiagramNodeRects.clear();
  if (!spec.nodes.length) return;

  const svgNs = 'http://www.w3.org/2000/svg';
  const sizeState = graphSizeState ?? updateGraphSizeState(spec);
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
    edgesGroup.appendChild(path);
    if (edge.edgeIndex != null) {
      const hit = document.createElementNS(svgNs, 'path');
      hit.setAttribute('class', 'graph-diagram-edge-hit');
      hit.setAttribute('d', path.getAttribute('d') ?? '');
      hit.addEventListener('click', (event) => {
        event.stopPropagation();
        setGraphSelection({ edgeIndex: edge.edgeIndex ?? null });
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
  graphConnectLine = connectPath;
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
        ? graphSelectedOutputIndex === node.outputIndex
        : graphSelectedNodeId === node.id;
    const isOutput = node.outputIndex != null || node.id.startsWith('__out-');
    if (!isOutput) {
      graphDiagramNodeRects.set(node.id, { x: pos.x, y: pos.y, width: nodeWidth, height: nodeHeight });
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
        beginGraphConnectDrag(node.id, event);
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
      const point = getSvgPoint(event);
      if (!point) return;
      const current = positions.get(node.id);
      if (!current) return;
      graphDragState = {
        id: node.id,
        offsetX: point.x - current.x,
        offsetY: point.y - current.y
      };
      group.setPointerCapture?.(event.pointerId);
    });

    group.addEventListener('click', (event) => {
      event.stopPropagation();
      if (node.outputIndex != null) {
        setGraphSelection({ outputIndex: node.outputIndex });
        return;
      }
      setGraphSelection({ nodeId: node.id });
    });

    nodesGroup.appendChild(group);
  });
  graphDiagram.appendChild(nodesGroup);
  if (graphConnectDrag && graphPointerPos) {
    updateGraphConnectDrag(graphPointerPos);
  }
}

/**
 * Render the graph inspector panel for the current selection.
 * @param spec - Graph spec to inspect and edit.
 */
function renderGraphInspector(spec: GraphSpec): void {
  if (!graphDiagramInspector) return;
  graphDiagramInspector.innerHTML = '';
  const sizeState = graphSizeState ?? updateGraphSizeState(spec);

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

    if (node.type === 'Input') {
      graphDiagramInspector.appendChild(makeRow('Output', makeReadOnlyNumberInput(CFG.brain.inSize, 'fixed')));
    }
    if (node.type === 'Dense' || node.type === 'MLP') {
      const inferred = sizeState.sizes.get(node.id)?.inputSize ?? null;
      graphDiagramInspector.appendChild(makeRow('Input', makeReadOnlyNumberInput(inferred)));

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
      const inferred = sizeState.sizes.get(node.id)?.inputSize ?? null;
      graphDiagramInspector.appendChild(makeRow('Input', makeReadOnlyNumberInput(inferred)));

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
    if (node.type === 'Concat') {
      const outSize = sizeState.sizes.get(node.id)?.outputSizes?.[0] ?? null;
      graphDiagramInspector.appendChild(makeRow('Output', makeReadOnlyNumberInput(outSize)));
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

/**
 * Render the graph editor form controls for nodes, edges, and outputs.
 */
function renderGraphEditor(): void {
  if (!graphNodes || !graphEdges || !graphOutputs) return;
  const spec = ensureGraphDraft();
  spec.nodes.forEach(node => {
    if (node.type === 'Input') node.outputSize = CFG.brain.inSize;
  });
  const sizeState = updateGraphSizeState(spec);
  applyGraphSizeInference(spec, sizeState);
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
      const inferred = sizeState.sizes.get(node.id)?.inputSize ?? null;
      row.appendChild(makeField('Input', makeReadOnlyNumberInput(inferred)));

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
      const inferred = sizeState.sizes.get(node.id)?.inputSize ?? null;
      row.appendChild(makeField('Input', makeReadOnlyNumberInput(inferred)));

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
        graphLayoutOverrides.delete(removedId);
        if (graphSelectedNodeId === removedId) graphSelectedNodeId = null;
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

  renderSimpleOutputUi(spec, sizeState);
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

/**
 * Apply the settings lock state to UI controls.
 */
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

/**
 * Check whether player control is currently active.
 * @returns True when connected and controlling a snake.
 */
function isPlayerControlActive(): boolean {
  return connectionMode === 'server' && !!playerSnakeId;
}

/**
 * Compute a normalized turn input toward a target position.
 * @param meta - Current snake position and direction.
 * @param target - Target world position.
 * @returns Turn input in [-1,1].
 */
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

/**
 * Send a player action message to the server.
 */
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

/**
 * Store a new frame buffer and update generation stats.
 * @param buffer - Raw frame buffer from worker/server.
 */
function applyFrameBuffer(buffer: ArrayBuffer): void {
  currentFrameBuffer = new Float32Array(buffer);
  const gen = currentFrameBuffer[0] ?? Number.NaN;
  if (Number.isFinite(gen)) {
    const nextGen = Math.max(1, Math.floor(gen));
    proxyWorld.generation = nextGen;
    currentStats = { ...currentStats, gen: nextGen };
  }
}

/** Minimal snapshot of a snake parsed from the frame buffer. */
type FrameSnakeSnapshot = { id: number; x: number; y: number; ptCount: number };

/**
 * Find a snake in the frame buffer by id or return the first alive snake.
 * @param buffer - Frame buffer to scan.
 * @param targetId - Optional target snake id.
 * @returns Snapshot of the snake or null when none found.
 */
function findSnakeInFrame(buffer: Float32Array, targetId: number | null): FrameSnakeSnapshot | null {
  if (buffer.length < FRAME_HEADER_FLOATS) return null;
  const aliveCount = (buffer[FRAME_HEADER_OFFSETS.aliveCount] ?? 0) | 0;
  let ptr = FRAME_HEADER_FLOATS;
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

/**
 * Update client-side camera state when connected to the server.
 */
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

/**
 * Initialize or reset the worker with current settings.
 * @param resetCfg - Whether to reset CFG to defaults before applying updates.
 */
function initWorker(resetCfg = true): void {
  if (!worker) return;
  const settings = readSettingsFromCoreUI();
  const updates = collectSettingsUpdatesFromUI();
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

/**
 * Apply a full reset using the active simulation backend.
 * @param resetCfg - Whether to reset CFG before applying updates in worker mode.
 */
function applyResetToSimulation(resetCfg = true): void {
  const settings = readSettingsFromCoreUI();
  const updates = collectSettingsUpdatesFromUI();
  if (wsClient && wsClient.isConnected()) {
    wsClient.sendReset(settings, updates, customGraphSpec ?? null);
    return;
  }
  initWorker(resetCfg);
}

/**
 * Handle messages arriving from the worker.
 * @param msg - Worker message payload.
 */
function handleWorkerMessage(msg: WorkerToMainMessage): void {
  switch (msg.type) {
    case 'exportResult': {
      pendingExport = false;
      if (!msg.data || !Array.isArray(msg.data.genomes)) {
        alert('Export failed: invalid payload from worker.');
        return;
      }
      const settings = readSettingsFromCoreUI();
      const updates = collectSettingsUpdatesFromUI();
      const exportData = {
        generation: msg.data.generation || 1,
        archKey: msg.data.archKey,
        genomes: msg.data.genomes,
        graphSpec: customGraphSpec ?? null,
        settings,
        updates,
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
        currentVizData = normalizeVizData(msg.stats.viz);
      }
      return;
    }
    default: {
      const _exhaustive: never = msg;
      return _exhaustive;
    }
  }
}

/**
 * Attach message handlers to the worker instance.
 * @param target - Worker instance to bind.
 */
function bindWorkerHandlers(target: Worker): void {
  target.onmessage = (e: MessageEvent<WorkerToMainMessage>) => {
    handleWorkerMessage(e.data);
  };
}

/**
 * Start the worker-based simulation mode.
 * @param resetCfg - Whether to reset CFG before init.
 */
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

/**
 * Stop and dispose the worker simulation.
 */
function stopWorker(): void {
  if (!worker) return;
  worker.terminate();
  worker = null;
  pendingExport = false;
  currentVizData = null;
}

/**
 * Schedule a fallback to worker mode if server connection fails.
 */
function scheduleWorkerFallback(): void {
  if (fallbackTimer !== null) return;
  fallbackTimer = window.setTimeout(() => {
    fallbackTimer = null;
    if (wsClient?.isConnected()) return;
    startWorker(true);
  }, 2000);
}

/**
 * Schedule a reconnect attempt to the server with backoff.
 */
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

/**
 * Connect to the simulation server and set UI state.
 * @param url - WebSocket URL to connect to.
 */
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
    serverCfgHash = info.cfgHash;
    serverWorldSeed = info.worldSeed;
    lastServerTick = 0;
    if (fallbackTimer !== null) {
      clearTimeout(fallbackTimer);
      fallbackTimer = null;
    }
    if (worker) stopWorker();
    currentStats = {
      gen: 1,
      alive: 0,
      aliveTotal: 0,
      baselineBotsAlive: 0,
      baselineBotsTotal: 0,
      fps: info.tickRate
    };
    currentVizData = null;
    setConnectionStatus('server');
    joinPending = false;
    wsClient?.sendJoin('spectator');
    wsClient?.sendViz(activeTab === 'tab-viz');
    setJoinOverlayVisible(true);
    setJoinStatus('Enter a nickname to play');
    updateJoinControls();
    refreshSavedPresets().catch(() => { });
    const base = resolveServerHttpBase(serverUrl || resolveServerUrl());
    if (base) {
      hof.loadFromServer(base).catch(err => console.warn('HoF load failed', err));
    }
  },
  onDisconnected: () => {
    const hasWorker = !!worker;
    if (hasWorker) {
      setConnectionStatus('worker');
    } else {
      setConnectionStatus('connecting');
    }
    serverCfgHash = null;
    serverWorldSeed = null;
    lastServerTick = 0;
    resolvePendingServerReset();
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
    lastServerTick = msg.tick;
    if (pendingServerReset) {
      if (msg.tick < pendingServerReset.priorTick || msg.tick <= 1) {
        resolvePendingServerReset();
      }
    }
    const aliveTotal = Number.isFinite(msg.aliveTotal) ? msg.aliveTotal : msg.alive;
    const baselineBotsAlive = Number.isFinite(msg.baselineBotsAlive) ? msg.baselineBotsAlive : 0;
    const baselineBotsTotal = Number.isFinite(msg.baselineBotsTotal) ? msg.baselineBotsTotal : 0;
    currentStats = {
      ...currentStats,
      gen: msg.gen,
      alive: msg.alive,
      aliveTotal,
      baselineBotsAlive,
      baselineBotsTotal,
      fps: msg.fps
    };
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
      currentVizData = normalizeVizData(msg.viz);
    }
    if (msg.hofEntry) {
      void hof.add(msg.hofEntry);
      if (connectionMode === 'server') {
        const base = resolveServerHttpBase(serverUrl || resolveServerUrl());
        if (base) void hof.syncToServer(base);
      }
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
 * Live update handler for sliders that do not require a reset.
 * @param sliderEl - Slider input element to read.
 */
function liveUpdateFromSlider(sliderEl: HTMLInputElement): void {
  const path = sliderEl.dataset['path']!;
  const value = readSettingsInputValue(sliderEl);
  if (value == null) return;
  setByPath(CFG, path, value);
  if (path.startsWith('baselineBots.')) {
    persistBaselineBotSettings();
  }
  if (!worker) return;
  worker.postMessage({
    type: 'updateSettings',
    updates: [{
      path: path as SettingsUpdate['path'],
      value
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
  updateCFGFromUI(resolveSettingsRoot());
  persistBaselineBotSettings();
  applyResetToSimulation(true);
});
// Restore defaults
btnDefaults.addEventListener('click', () => {
  resetCFGToDefaults();
  setupSettingsUI(settingsContainer, liveUpdateFromSlider); // Re-apply defaults to dynamic UI
  wireBaselineBotControls();
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
  persistBaselineBotSettings();
  applyResetToSimulation(true);
});
// Toggle view mode
btnToggle.addEventListener('click', () => proxyWorld.toggleViewMode());
window.addEventListener('keydown', e => {
  if (e.code === 'KeyV') {
    if (connectionMode === 'server') {
      if (isPlayerControlActive()) {
        enterSpectatorMode();
      } else {
        enterPlayerMode();
      }
      return;
    }
    proxyWorld.toggleViewMode();
  }
});

// ============== GOD MODE: Canvas Event Handlers ==============

/**
 * Converts screen-space coordinates (CSS pixels) into simulation world coordinates.
 * 
 * Transformation Logic:
 * 1. Offset by viewport center: Map (0,0) at top-left to centered origin.
 * 2. Scale by inverse zoom: Convert CSS pixels to world units.
 * 3. Offset by camera position: Map centered local view to absolute world space.
 * 
 * @param screenX - Screen X in CSS pixels (relative to canvas).
 * @param screenY - Screen Y in CSS pixels (relative to canvas).
 * @returns Object containing absolute simulation world coordinates \{x, y\}.
 */
function screenToWorld(screenX: number, screenY: number): { x: number; y: number } {
  // Retrieve camera state from the current data source (Server stream or Local worker buffer).
  let camX = 0, camY = 0, zoom = 1;
  if (connectionMode === 'server') {
    camX = clientCamX;
    camY = clientCamY;
    zoom = clientZoom;
  } else if (currentFrameBuffer && currentFrameBuffer.length >= FRAME_HEADER_FLOATS) {
    camX = currentFrameBuffer[FRAME_HEADER_OFFSETS.cameraX] ?? 0;
    camY = currentFrameBuffer[FRAME_HEADER_OFFSETS.cameraY] ?? 0;
    zoom = currentFrameBuffer[FRAME_HEADER_OFFSETS.zoom] ?? 1;
  }

  const centerX = cssW / 2;
  const centerY = cssH / 2;

  // Inverse Transformation Formula: World = Camera + (Screen - Center) / Zoom
  // This formula reverses the rendering transform to map a mouse click (screen space)
  // back into simulation world coordinates.
  const worldX = camX + (screenX - centerX) / zoom;
  const worldY = camY + (screenY - centerY) / zoom;
  return { x: worldX, y: worldY };
}

/**
 * Identifies the snake closest to a specific world coordinate within a search radius.
 * 
 * Scanning Algorithm:
 * Because the binary frame buffer uses variable-length snake blocks (due to body points), 
 * we must perform a linear scan. We skip over body data by reading the `ptCount` for each 
 * snake to jump to the next block until we find the best match.
 * 
 * @param worldX - Focus X in simulation units.
 * @param worldY - Focus Y in simulation units.
 * @param maxDist - Maximum distance threshold for selection.
 * @returns A snapshot of the closest snake's metadata or null if none found.
 */
function findSnakeNear(worldX: number, worldY: number, maxDist = 50): SelectedSnake | null {
  if (!currentFrameBuffer || currentFrameBuffer.length < FRAME_HEADER_FLOATS) return null;

  const buffer = currentFrameBuffer;
  const read = (idx: number): number => buffer[idx] ?? 0;

  // Retrieve count of snakes from the header to bound the search.
  const aliveCount = read(FRAME_HEADER_OFFSETS.aliveCount) | 0;

  let closestSnake = null;
  let closestDist = maxDist;
  let ptr = FRAME_HEADER_FLOATS;

  const SNAKE_STATEDATA_FLOATS = 8; // ID, Rad, Skin, X, Y, Ang, Boost, PtCount
  const PT_COUNT_OFFSET = 7;
  const RADIUS_OFFSET = 1;
  const SKIN_OFFSET = 2;
  const X_OFFSET = 3;
  const Y_OFFSET = 4;

  for (let i = 0; i < aliveCount; i++) {
    const ptCount = read(ptr + PT_COUNT_OFFSET) | 0;

    // Selection Jump Logic:
    // Because snake body point counts vary, we calculate the total byte size of the snake block 
    // [Header (8) + (ptCount * 2)] to jump the pointer precisely to the next entity.
    const blockSize = SNAKE_STATEDATA_FLOATS + ptCount * 2;

    if (ptr + blockSize > buffer.length) break;

    const id = read(ptr);
    const radius = read(ptr + RADIUS_OFFSET);
    const skin = read(ptr + SKIN_OFFSET);
    const x = read(ptr + X_OFFSET);
    const y = read(ptr + Y_OFFSET);

    const dist = Math.hypot(x - worldX, y - worldY);
    // Selection threshold includes the snake's actual physical radius to make 
    // clicking on large snakes easier.
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
    console.log(`Selected snake #${snake.id} (skin: ${snake.skin})`);
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

/**
 * Resolve the HTTP base URL for server API requests from a WS URL.
 * @param wsUrl - WebSocket URL used for the server connection.
 * @returns HTTP base URL or null when parsing fails.
 */
function resolveServerHttpBase(wsUrl: string): string | null {
  try {
    const url = new URL(wsUrl);
    const protocol = url.protocol === 'wss:' ? 'https:' : 'http:';
    return `${protocol}//${url.host}`;
  } catch {
    return null;
  }
}

/**
 * Extract a human-readable error message from a failed fetch response.
 * @param res - Failed fetch response.
 * @returns Error message for UI display.
 */
async function readErrorMessage(res: Response): Promise<string> {
  try {
    const body = await res.json() as { message?: string } | null;
    if (body?.message && typeof body.message === 'string') {
      return body.message;
    }
  } catch {
    // Fall back to status text when JSON parsing fails.
  }
  return res.statusText || `HTTP ${res.status}`;
}

/**
 * Resolve any pending server reset promise.
 */
function resolvePendingServerReset(): void {
  if (!pendingServerReset) return;
  clearTimeout(pendingServerReset.timeoutId);
  const { resolve } = pendingServerReset;
  pendingServerReset = null;
  resolve();
}

/**
 * Wait for the server to reset its tick counter after a reset request.
 * @returns Promise resolved after observing the tick counter drop or timeout.
 */
function waitForServerReset(): Promise<void> {
  if (pendingServerReset) return pendingServerReset.promise;
  const priorTick = lastServerTick;
  let resolve!: () => void;
  const promise = new Promise<void>((res) => {
    resolve = res;
  });
  const timeoutId = window.setTimeout(() => {
    resolvePendingServerReset();
  }, 2000);
  pendingServerReset = { priorTick, resolve, timeoutId, promise };
  return promise;
}

/**
 * Import a snapshot into the server and return the applied counts.
 * @param data - Parsed population file payload.
 * @returns Import result counts from the server.
 */
async function importServerSnapshot(data: PopulationFilePayload): Promise<{ used: number; total: number }> {
  const base = resolveServerHttpBase(serverUrl || resolveServerUrl());
  if (!base) {
    throw new Error('invalid server URL.');
  }
  const metadata = data as PopulationFilePayload & {
    archKey?: unknown;
    cfgHash?: unknown;
    worldSeed?: unknown;
  };
  const archKey = typeof metadata.archKey === 'string' && metadata.archKey.trim()
    ? metadata.archKey.trim()
    : (data.genomes?.[0]?.archKey ?? '');
  if (!archKey) {
    throw new Error('missing archKey for server import.');
  }
  const cfgHash = typeof metadata.cfgHash === 'string' && metadata.cfgHash.trim()
    ? metadata.cfgHash.trim()
    : serverCfgHash;
  if (!cfgHash) {
    throw new Error('missing cfgHash for server import (export from server or reconnect).');
  }
  const seedFromFile = typeof metadata.worldSeed === 'number' ? metadata.worldSeed : NaN;
  const worldSeed = Number.isFinite(seedFromFile) ? seedFromFile : serverWorldSeed;
  if (!Number.isFinite(worldSeed ?? NaN)) {
    throw new Error('missing worldSeed for server import (export from server or reconnect).');
  }
  const payload = {
    generation: Number.isFinite(data.generation) ? data.generation : 1,
    archKey,
    genomes: data.genomes,
    cfgHash,
    worldSeed: worldSeed as number
  };
  const force = typeof serverCfgHash === 'string' && serverCfgHash.trim() && cfgHash !== serverCfgHash;
  const res = await fetch(`${base}/api/import${force ? '?force=1' : ''}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(payload)
  });
  if (!res.ok) {
    const message = await readErrorMessage(res);
    throw new Error(`server import failed (${res.status}): ${message}`);
  }
  const result = await res.json() as { ok?: boolean; used?: number; total?: number; message?: string };
  if (!result?.ok) {
    throw new Error(result?.message || 'server import failed');
  }
  return { used: result.used ?? 0, total: result.total ?? 0 };
}

/**
 * Export the latest server snapshot and HoF entries to a local file.
 */
async function exportServerSnapshot(): Promise<void> {
  const base = resolveServerHttpBase(serverUrl || resolveServerUrl());
  if (!base) {
    pendingExport = false;
    alert('Export failed: invalid server URL.');
    return;
  }
  try {
    const saveRes = await fetch(`${base}/api/save`, { method: 'POST' });
    if (!saveRes.ok) {
      const message = await readErrorMessage(saveRes);
      throw new Error(`snapshot save failed (${saveRes.status}): ${message}`);
    }
    const exportRes = await fetch(`${base}/api/export/latest`);
    if (!exportRes.ok) {
      const message = await readErrorMessage(exportRes);
      throw new Error(`snapshot export failed (${exportRes.status}): ${message}`);
    }
    const exportData = await exportRes.json() as {
      generation?: number;
      archKey?: string;
      genomes?: unknown;
      cfgHash?: string;
      worldSeed?: number;
    };
    if (!exportData || !Array.isArray(exportData.genomes)) {
      throw new Error('invalid export payload from server.');
    }
    const archKey = typeof exportData.archKey === 'string' ? exportData.archKey : '';
    if (!archKey) {
      throw new Error('invalid export payload from server (missing archKey).');
    }
    const settings = readSettingsFromCoreUI();
    const updates = collectSettingsUpdatesFromUI();
    const payload: PopulationFilePayload = {
      generation: exportData.generation || 1,
      archKey,
      genomes: exportData.genomes,
      graphSpec: customGraphSpec ?? null,
      settings,
      updates,
      hof: hof.getAll()
    };
    if (typeof exportData.cfgHash === 'string' && exportData.cfgHash.trim()) {
      payload.cfgHash = exportData.cfgHash;
    }
    const worldSeed = exportData.worldSeed;
    if (typeof worldSeed === 'number' && Number.isFinite(worldSeed)) {
      payload.worldSeed = worldSeed;
    }
    exportToFile(payload, `slither_neuroevo_gen${payload.generation}.json`);
  } catch (err) {
    console.error('Server export failed', err);
    alert(`Export failed: ${(err as Error).message}`);
  } finally {
    pendingExport = false;
  }
}

// Persistence UI Wiring
/** Button that triggers exporting population and HoF data. */
const btnExport = document.getElementById('btnExport') as HTMLButtonElement | null;
if (btnExport) {
  btnExport.addEventListener('click', () => {
    if (pendingExport) return;
    pendingExport = true;
    if (worker) {
      worker.postMessage({ type: 'export' });
      return;
    }
    if (wsClient && wsClient.isConnected()) {
      void exportServerSnapshot();
      return;
    }
    pendingExport = false;
  });
}

/** Button that opens the import file picker. */
const btnImport = document.getElementById('btnImport') as HTMLButtonElement | null;
/** Hidden file input used for population imports. */
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
        await hof.replace(data.hof);
      }
      const hasGraphSpecField = Object.prototype.hasOwnProperty.call(data, 'graphSpec');
      const fileGraphSpec = hasGraphSpecField ? (data.graphSpec ?? null) : undefined;
      const fileSettings = data.settings;
      const fileUpdates = normalizeImportUpdates(data.updates);
      const shouldReset =
        hasGraphSpecField ||
        (fileSettings != null && typeof fileSettings === 'object') ||
        (fileUpdates != null && fileUpdates.length > 0);
      if (shouldReset) {
        if (fileGraphSpec && typeof fileGraphSpec === 'object') {
          if (!applyGraphSpec(fileGraphSpec, 'Graph loaded from import.')) {
            throw new Error('Import file graph spec is invalid.');
          }
        } else if (hasGraphSpecField && fileGraphSpec === null) {
          clearCustomGraphSpec('Graph cleared from import.');
        }
        if (fileSettings && typeof fileSettings === 'object') {
          applyCoreSettingsToUi(fileSettings);
        }
        if (fileUpdates && fileUpdates.length > 0) {
          applySettingsUpdatesToUi(fileUpdates);
        }
        persistBaselineBotSettings();
      }
      if (wsClient && wsClient.isConnected()) {
        if (shouldReset) {
          const resetSettings = fileSettings ?? readSettingsFromCoreUI();
          const resetUpdates = fileUpdates ?? collectSettingsUpdatesFromUI();
          const resetGraphSpec = hasGraphSpecField ? (fileGraphSpec ?? null) : (customGraphSpec ?? null);
          wsClient.sendReset(resetSettings, resetUpdates, resetGraphSpec);
          await waitForServerReset();
        }
        const result = await importServerSnapshot(data);
        alert(`Import applied on server. Loaded ${result.used}/${result.total} genomes.`);
        return;
      }
      if (!worker) {
        throw new Error('No active simulation backend for import.');
      }
      if (shouldReset) {
        applyResetToSimulation(true);
      }
      let persistWarning = '';
      const ok = await savePopulationJSON(data.generation, data.genomes);
      if (!ok) {
        persistWarning = 'Import succeeded, but persistence failed (quota exceeded). It will not persist after reload.';
      }
      worker.postMessage({ type: 'import', data });
      if (persistWarning) {
        alert(persistWarning);
      }
    } catch (err) {
      console.error("Import failed", err);
      const error = err as Error;
      if (wsClient?.isConnected() && error.message.includes('no compatible genomes')) {
        alert('Import failed: the file does not match the server brain. Re-export with the latest build (includes graph/settings metadata) or reset the server to the matching graph before importing.');
      } else {
        alert("Failed to import file: " + error.message);
      }
    } finally {
      if (target) target.value = '';
    }
  });
}

/**
 * Main animation frame loop for rendering and UI updates.
 */
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

  const aliveTotal = Number.isFinite(currentStats.aliveTotal)
    ? currentStats.aliveTotal
    : currentStats.alive;
  const baselineBotsAlive = Number.isFinite(currentStats.baselineBotsAlive)
    ? currentStats.baselineBotsAlive
    : 0;
  const baselineBotsTotal = Number.isFinite(currentStats.baselineBotsTotal)
    ? currentStats.baselineBotsTotal
    : 0;
  const stepInfo = `Gen: ${currentStats.gen}  Alive: ${currentStats.alive}  Total: ${aliveTotal}  Baseline: ${baselineBotsAlive}/${baselineBotsTotal}  FPS: ${Math.round(currentStats.fps)}`;
  const statsInfoHtml =
    `<div class="stat-box"><span class="label">Generation</span><span class="val">${currentStats.gen}</span></div>` +
    `<div class="stat-box"><span class="label">Alive (Pop)</span><span class="val">${currentStats.alive}</span></div>` +
    `<div class="stat-box"><span class="label">Alive (Total)</span><span class="val">${aliveTotal}</span></div>` +
    `<div class="stat-box"><span class="label">Baseline Bots</span><span class="val">${baselineBotsAlive}/${baselineBotsTotal}</span></div>` +
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

/**
 * Update the Hall of Fame table UI.
 * @param world - Proxy world providing current generation state.
 */
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
        <span>#${idx + 1} Gen ${entry.gen} (Fit ${entry.fitness.toFixed(1)})</span>
        <button onclick="window.spawnHoF(${idx})">Spawn</button>
      </div>`;
  });
  container.innerHTML = html;
}

/** Expose global helper for HoF spawn buttons. */
window.spawnHoF = function (idx) {
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
