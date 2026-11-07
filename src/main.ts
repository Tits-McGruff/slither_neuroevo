import { createRequestId } from './net/requestId.ts';
import { exportServerSnapshot as exportArchive,importServerArchive,resolveServerHttpBase,type ArchiveTransportPort } from './archive_transport.ts';
import { finishCameraPan as finishCameraPanCamera,fitArenaView as fitArenaViewCamera,moveCameraPan as moveCameraPanCamera,publishClientCamera as publishClientCameraCamera,resetManualCamera as resetManualCameraCamera,screenToWorld as screenToWorldCamera,updateClientCamera as updateClientCameraCamera,type CameraPort } from './camera.ts';
import { DecisionInspectorView } from './decisionInspectorView.ts';
import { DistillationView } from './distillationView.ts';
import { DIAGRAM_NODE_HEIGHT,DIAGRAM_NODE_WIDTH,renderGraphDiagramView } from './graph_diagram.ts';
import { renderGraphInspectorView } from './graph_inspector.ts';
import { refreshSavedPresets as refreshGraphPresets,saveGraphPreset,type GraphPresetPort } from './graph_presets.ts';
import { applySimpleOutputSelection as applySimpleOutputSelectionSettings,renderGraphEditor as renderGraphEditorSettings,type GraphSettingsPort } from './graph_settings.ts';
import { buildUniqueNodeId,cloneGraphSpec,getGraphSpecInputSize,applyGraphSizeInference as projectGraphSizeInference } from './graph_state.ts';
import { buildDefaultNode,buildLinearMlpTemplate,buildMlpGruMlpTemplate,buildSkipTemplate,buildSplitTemplate } from './graph_templates.ts';
import { DecisionInspectorClient } from './net/decisionInspector.ts';
import { DistillationClient } from './net/distillation.ts';
import { computeTurnInput,frameContainsSnake as containsFrameSnake,normalizeVizData,findSnakeNear as selectFrameSnake,type SelectedSnake } from './player_input_and_frames.ts';
import { applyCoreSettingsToUi as applyCoreUI,parseBaselineSeedValue,applySettingsLock as projectSettingsLock,readSettingsFromCoreUI as readCoreUI,readSettingsInputValue,refreshCoreUIState as refreshCoreUI,type CoreInputElements,type CoreUiDraft } from './settings_ui.ts';
/** Entry point for the simulation UI, canvas, and client networking. */

import { BrainLibraryView,type LibraryEnvironment } from './brainLibraryView.ts';
import { graphKey } from './brains/graph/compiler.ts';
import type { GraphSizeState } from './brains/graph/editor.ts';
import { inferGraphSizes } from './brains/graph/editor.ts';
import type { GraphEdge,GraphSpec } from './brains/graph/schema.ts';
import { validateGraph } from './brains/graph/validate.ts';
import { buildStackGraphSpec } from './brains/stackBuilder.ts';
import { BrainViz } from './BrainViz.ts';
import { AdvancedCharts } from './chartUtils.ts';
import { CFG,CFG_DEFAULT,applyAuthoritativeSensorSpec,resetCFGToDefaults,syncBrainInputSize } from './config.ts';
import { hof } from './hallOfFame.ts';
import { createAuthoritativeControls } from './net/authoritativeControls.ts';
import { BrainLibraryClient } from './net/brainLibrary.ts';
import { createEntitySelectionClient,describeSelectedEntity } from './net/entitySelection.ts';
import {
PlayerActionPump,
normalizePlayerActionRate,
type LatestPlayerAction
} from './net/playerActionPump.ts';
import {
createWsClient,
formatImportBranchRuntimeStatus,
formatLegacyConversionRuntimeStatus,
formatRecoveryRuntimeStatus,
formatServerRuntimeStatus,
resolveServerUrl,
storeServerUrl,
type WelcomeInferenceMode
} from './net/wsClient.ts';
import { formatDurationTimer } from './protocol/duration.ts';
import { parseEvolvedLifecycleTelemetry } from './protocol/evolvedLives.ts';
import type { FrameStats,HallOfFameEntry,VizData } from './protocol/messages.ts';
import type {
RustImportBranchNotice,
RustLegacyConversionNotice,
RustRecoveryNotice
} from './protocol/rustBackground.ts';
import { EVOLVED_SETTING_PATHS,SETTING_DEFINITION_BY_PATH,normalizeSettingValue } from './protocol/settingDefinitions.ts';
import type {
CoreSettings,
LiveSettingPath,
LiveSettingsUpdate,
SettingsUpdate
} from './protocol/settings.ts';
import { coerceSettingsUpdateValue } from './protocol/settings.ts';
import { renderWorldStruct } from './render.ts';
import {
BASELINE_BOT_SEED_HINT_ID,
BASELINE_BOT_SEED_INPUT_ID,
BASELINE_BOT_SEED_RANDOMIZE_ID,
BROWSER_CAMERA_SETTING_PATHS,
applyValuesToSlidersFromCFG,
collectSettingsUpdates,
setupSettingsUI,
updateCFGFromUI,
validateSettingDrafts,
type SettingInputElement
} from './settings.ts';
import {
exportJsonToFile,
loadBaselineBotSettings,
saveBaselineBotSettings
} from './storage.ts';
import { getByPath,setByPath } from './utils.ts';

let cachedCameraPort: CameraPort | undefined;

/** Minimal world interface exposed to UI panels and HoF actions. */
interface ProxyWorld {
  generation: number;
  zoom: number;
  cameraX: number;
  cameraY: number;
  viewMode: string;
  fitnessHistory: FitnessHistoryUiEntry[];
  toggleViewMode: () => void;
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

/** Connection mode used by the main UI. */
type ConnectionMode = 'connecting' | 'server';

declare global {
  /** Global window extensions used by the UI. */
  interface Window {
    ctx: CanvasRenderingContext2D;
    currentWorld: ProxyWorld;
    spawnHoF: (idx: number) => void;
  }
}

/** WebSocket client when connected to the server. */
let wsClient: ReturnType<typeof createWsClient> | null = null;
/** Extracted coalescing and God Mode transport bound to the current socket. */
const authoritativeControls = createAuthoritativeControls({
  sendSettings: (requestId, updates) => wsClient?.sendSettings(requestId, updates),
  sendGodModeKill: (requestId, snakeId, token) => wsClient?.sendGodModeKill(requestId, snakeId, token),
  sendGodModeMove: (requestId, snakeId, x, y, token) =>
    wsClient?.sendGodModeMove(requestId, snakeId, x, y, token),
  sendNewRun: (requestId) => wsClient?.sendNewRun(requestId)
});
/** Current connection mode state. */
let connectionMode: ConnectionMode = 'connecting';
/** Current server URL used for connection attempts. */
let serverUrl = '';
/** Latest authoritative config revision received from the server. */
let serverConfigRevision = 0;
/** Last simulation speed accepted or advertised by the authoritative server. */
let serverSimSpeed = 1;
/** Latest server world seed from the welcome message. */
let serverWorldSeed: number | null = null;
/** Current run for generation-scoped UI commands. */
let serverRunId = '';
/** One manual finish request awaiting native durability. */
let pendingFinishRequestId: string | null = null;
/** Inference mode advertised for the active server run. */
let serverInferenceMode: WelcomeInferenceMode | null = null;
/** Durable recovery provenance advertised for the active branch. */
let serverRecovery: RustRecoveryNotice | null = null;
/** Exact archive branch provenance advertised for the active run. */
let serverImportBranch: RustImportBranchNotice | null = null;
/** Population-only source advertised for a run converted from old SQLite data. */
let serverLegacyConversion: RustLegacyConversionNotice | null = null;
/** Correlation id of the currently pending New Run request. */
let pendingNewRunRequestId: string | null = null;
/** One explicit founder request awaits the ordinary durable New Run result. */
let pendingFounderStart: {requestId: string; resolve(): void; reject(error: Error): void} | null = null;
/** Backoff delay for reconnection attempts in ms. */
let reconnectDelayMs = 1000;
/** Last player nickname used for reconnecting control. */
let lastPlayerName = '';
/** Local storage key for the player nickname. */
const PLAYER_NAME_KEY = 'slither_neuroevo_player_name';
/** Local storage key for the opaque controller reclaim token. */
const PLAYER_RESUME_TOKEN_KEY = 'slither_neuroevo_player_resume_token';
/** Opaque token for reclaiming the current server-side controller lease. */
let playerResumeToken = '';
/** Whether the next successful handshake should immediately request reclaim. */
let resumePlayerAfterReconnect = false;
/** Whether an explicitly selected spectator session should resume after reconnect. */
let resumeSpectatorAfterReconnect = false;
/** Timer id for reconnect scheduling. */
let reconnectTimer: number | null = null;
/** Whether settings controls are locked. */
let settingsLocked = true;
/** Whether join overlay is awaiting user action. */
let joinPending = false;
/** Current player snake id when controlling. */
let playerSnakeId: number | null = null;
/** Snake id to follow when spectating in server mode. */
let spectatorFollowSnakeId: number | null = null;
/** Tick id of the most recent player sensor packet. */
let playerSensorTick = 0;
/** Latest player sensor metadata for UI overlays. */
let playerSensorMeta: { x: number; y: number; dir: number } | null = null;
/** Latest pointer position relative to the canvas in screen pixels. */
let pointerScreen: { x: number; y: number } | null = null;
/** Whether boost is held down by input. */
let boostHeld = false;
/** Local storage key for graph spec persistence. */
const GRAPH_SPEC_STORAGE_KEY = 'slither_neuroevo_graph_spec';
/** Applied custom graph spec used for resets. */
let customGraphSpec: GraphSpec | null = null;
/** Current graph editor draft spec. */
let graphDraft: GraphSpec | null = null;
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
 * Resize the canvas and notify the server of the new viewport size.
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
/** Visible acknowledgement or rejection for the latest settings reset. */
const resetStatus = document.getElementById('resetStatus') as HTMLElement | null;
/** Button to restore default settings. */
const btnDefaults = document.getElementById('defaults') as HTMLButtonElement;
/** Button that starts a separately seeded, durably checkpointed run. */
const btnNewRun = document.getElementById('newRun') as HTMLButtonElement;
/** Optional manual completion action, shown only when advertised. */
const btnFinishGeneration = document.getElementById('finishGeneration') as HTMLButtonElement | null;
/** Native manual completion status. */
const finishGenerationStatus = document.getElementById('finishGenerationStatus') as HTMLElement | null;
/** Button to toggle the settings panel. */
const btnToggle = document.getElementById('toggle') as HTMLButtonElement;
/** Button that restores the full-arena camera after manual navigation. */
const btnFitArena = document.getElementById('fitArena') as HTMLButtonElement;
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
/** Generation timer HUD element. */
const genTimer = document.getElementById('genTimer') as HTMLElement | null;
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
    const savedToken = localStorage.getItem(PLAYER_RESUME_TOKEN_KEY);
    if (savedToken) {
      playerResumeToken = savedToken;
      resumePlayerAfterReconnect = true;
    }
  } catch {
    // Ignore storage failures in non-browser environments.
  }
}

/** Brain visualizer instance for the Viz tab. */
const brainViz = new BrainViz(0, 0, vizCanvas.width, vizCanvas.height);
/** Currently active tab id. */
let activeTab = 'tab-settings';
/** Applied library compatibility facts never read editable graph/settings drafts. */
const libraryEnvironment: LibraryEnvironment = {graphSourceJson: null, sensorSha256: null, cadence: 1 / 60, configHash: ''};
/** User-driven library UI; it performs no animation-frame fetch or neural execution. */
const brainLibrary = new BrainLibraryView(new BrainLibraryClient(() => wsClient?.isConnected()
  ? resolveServerHttpBase(serverUrl || resolveServerUrl()) : null), () => libraryEnvironment, snakeId => {
  spectatorFollowSnakeId = snakeId; resetManualCamera(); proxyWorld.viewMode = 'follow';
}, async selection => {
  if (!wsClient?.isConnected() || pendingNewRunRequestId || pendingFounderStart) throw new Error('Connect and wait for the current New Run request.');
  const requestId = createRequestId();
  pendingNewRunRequestId = requestId;
  btnNewRun.disabled = true;
  await new Promise<void>((resolve, reject) => {
    const timer = setTimeout(() => {
      pendingFounderStart = null;
      reject(new Error('Founder start outcome is uncertain. Reconnect and inspect the current run before starting another.'));
    }, 60_000);
    pendingFounderStart = {requestId, resolve: () => {clearTimeout(timer); pendingFounderStart = null; resolve();},
      reject: error => {clearTimeout(timer); pendingFounderStart = null; reject(error);}};
    wsClient!.sendNewRun(requestId, selection);
  });
});
/** Last advertised diagnostic controls, independent of editable settings drafts. */
let distillationAvailable = false;
/** Applied native v4/v2 capture admission advertised at welcome or run replacement. */
let teacherCaptureReady = false;
/** Explicit metadata and diagnostic commands use only the currently connected applied scope. */
const distillation = new DistillationView(new DistillationClient(() => {
  const base = wsClient?.isConnected() && distillationAvailable ? resolveServerHttpBase(serverUrl || resolveServerUrl()) : null;
  return base ? { base, runId: serverRunId, configRevision: BigInt(serverConfigRevision).toString(16).padStart(16, '0') } : null;
}));
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
    if (tabId === 'tab-library') brainLibrary.refresh();
    if (tabId === 'tab-distillation') distillation.refresh();
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
    if (!joinName || !joinName.value.trim()) return;
    enterPlayerMode();
  });
}
if (joinSpectate) {
  joinSpectate.addEventListener('click', () => {
    enterSpectatorMode();
  });
}
setJoinOverlayVisible(true);
setJoinStatus('Connecting...');

/** Project the existing browser controls without moving their ownership. */
function readSettingsFromCoreUI(): CoreUiDraft { return readCoreUI(getCoreInputElements()); }

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
  if (!validateSettingDrafts(resolveSettingsRoot())) return;
  setByPath(CFG, 'baselineBots.seed', seed);
  if (baselineBotSeedInput) {
    baselineBotSeedInput.value = String(seed);
  }
  updateBaselineSeedLabel(seed);
  setBaselineSeedHintVisible(false);
  persistBaselineBotSettings();
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

/** Project the existing browser controls without moving their ownership. */
function applyCoreSettingsToUi(settings: Partial<CoreSettings>): void { applyCoreUI(settings, getCoreInputElements(), refreshCoreUIState); }

/**
 * Resolve the root element containing CFG-backed settings sliders.
 * @returns Root element containing data-path inputs.
 */
function resolveSettingsRoot(): HTMLElement {
  const hasContainerInputs =
    typeof settingsContainer?.querySelector === 'function' &&
    settingsContainer.querySelector('input[data-path], select[data-path]') != null;
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

/** Project the existing browser controls without moving their ownership. */
function refreshCoreUIState(): void {
  refreshCoreUI(getCoreInputElements(), { snakesVal, simSpeedVal, layersVal, n1Val, n2Val, n3Val, n4Val, n5Val }, () => settingsLocked, () => !!customGraphSpec, graphModeHint);
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
        ? buildMlpGruMlpTemplate(readSettingsFromCoreUI(), CFG.brain)
        : template === 'skip'
          ? buildSkipTemplate(readSettingsFromCoreUI(), CFG.brain)
          : template === 'split'
            ? buildSplitTemplate(CFG.brain)
            : buildLinearMlpTemplate(readSettingsFromCoreUI(), CFG.brain);
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
    draft.nodes.push(buildDefaultNode('Dense', id, CFG.brain, readSettingsFromCoreUI()));
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
    draft.nodes.push(buildDefaultNode('Dense', buildUniqueNodeId(draft, 'node'), CFG.brain, readSettingsFromCoreUI()));
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
    applyResetToSimulation();
  });
}

if (graphReset) {
  graphReset.addEventListener('click', () => {
    resetGraphDraft();
  });
}

// Explicitly leave custom-graph mode without changing other pending settings.
document.getElementById('graphUseStack')?.addEventListener('click', () => {
  if (!validateSettingDrafts(resolveSettingsRoot())) return;
  updateCFGFromUI(resolveSettingsRoot());
  stageStackGraph('Using stack controls. Apply and reset to start a new population.');
});

/** Stage the selected stack without treating an initial/default preview as a custom graph. */
function stageStackGraph(note: string): void {
  customGraphSpec = null;
  CFG.brain.graphSpec = null;
  graphDraft = buildStackGraphSpec(readSettingsFromCoreUI(), CFG);
  try { localStorage.removeItem(GRAPH_SPEC_STORAGE_KEY); } catch { /* Storage is optional. */ }
  refreshCoreUIState();
  renderGraphEditor();
  setGraphSpecStatus(note);
}

if (graphPresetSave) {
  graphPresetSave.addEventListener('click', () => saveGraphPreset(getGraphPresetPort()));
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
  generationTime: 0,
  generationSeconds: CFG.generationSeconds,
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
/** Presentation-only preference for following a population owner across lives. */
const followIndividualInput = document.getElementById('followIndividual') as HTMLInputElement | null;
if (followIndividualInput) {
  followIndividualInput.checked = CFG.diagnostics.followIndividual;
  followIndividualInput.addEventListener('change', () => { CFG.diagnostics.followIndividual = followIndividualInput.checked; });
}
/** Text-only live provenance comes from the selected native owner, never a catalog lookup. */
const selectedEntityInfo = document.getElementById('selectedEntityInfo');
if (selectedEntityInfo) selectedEntityInfo.textContent = describeSelectedEntity(null);
/** On-demand decision data remains separate from the ordinary frame renderer. */
const decisionInspectorView = new DecisionInspectorView();
/** One selected native subscription; default-off presentation never samples locally. */
const decisionInspector = new DecisionInspectorClient({send: request => wsClient?.sendInspector(request),
  changed: snapshot => decisionInspectorView.snapshot(snapshot), status: message => decisionInspectorView.status(message)});
/** Explicit capture preference, independent of gameplay settings. */
const decisionInspectorInput = document.getElementById('decisionInspectorEnabled') as HTMLInputElement | null;
if (decisionInspectorInput) {
  decisionInspectorInput.checked = CFG.diagnostics.inspectorEnabled;
  decisionInspectorInput.addEventListener('change', () => {
    CFG.diagnostics.inspectorEnabled = decisionInspectorInput.checked;
    decisionInspector.setEnabled(decisionInspectorInput.checked);
  });
}
/** Explicit diagnostic buttons enable capture and choose one-shot or delivered data. */
function chooseInspectorProbe(probe: boolean): void {
  CFG.diagnostics.inspectorEnabled = true;
  if (decisionInspectorInput) decisionInspectorInput.checked = true;
  if (probe) decisionInspector.probeOnce(); else decisionInspector.showDelivered();
}
document.getElementById('decisionInspectorProbe')?.addEventListener('click', () => chooseInspectorProbe(true));
document.getElementById('decisionInspectorDelivered')?.addEventListener('click', () => chooseInspectorProbe(false));

decisionInspectorView.supported(false);
/** Bounded native selection transport; no gameplay identity is reconstructed in the browser. */
const entitySelection = createEntitySelectionClient({
  send: request => wsClient?.sendSelectEntity(request),
  kill: (id, token) => { authoritativeControls.killSnake(id, token); },
  followIndividual: () => followIndividualInput?.checked ?? true,
  changed: entity => {
    decisionInspector.selection(entity);
    if (!entity && entitySelection.followedPopulationSlot() !== null && decisionInspectorInput?.checked) {
      decisionInspectorView.status('Waiting for the selected individual’s next physical life.');
    }
    const description = describeSelectedEntity(entity);
    if (selectedEntityInfo && selectedEntityInfo.textContent !== description) selectedEntityInfo.textContent = description;
    if (selectedSnake?.token !== entity?.token) {
      authoritativeControls.cancelMoves();
      isDragging = false;
      currentVizData = null;
    }
    selectedSnake = entity ? { id: entity.snakeId, x: entity.x, y: entity.y,
      radius: entity.radius, skin: entity.skin, token: entity.token } : null;
  }
});
/** Keyboard selection names a population owner rather than guessing a physical ID. */
const decisionInspectorSlot = document.getElementById('decisionInspectorSlot') as HTMLInputElement | null;
/** Native identity lookup is shared with ordinary canvas selection. */
const decisionInspectorSelect = document.getElementById('decisionInspectorSelect');
decisionInspectorSelect?.addEventListener('click', () => {
  const slot = Number(decisionInspectorSlot?.value ?? '');
  if (!Number.isSafeInteger(slot) || slot < 0 || slot >= 300) {
    decisionInspectorView.status('Choose a population slot from 0 to 299.'); return;
  }
  entitySelection.selectIndividual(slot);
});
/** Exact generation from scalar native stats; frame floats can round large counters. */
let selectionGeneration = 1;
/** Observed server process identity for selection scope updates. */
let selectionSessionId = '';
/** Whether a pointer drag is in progress. */
let isDragging = false;
/** Client-side camera X for overlays and input. */
let clientCamX = 0;
/** Client-side camera Y for overlays and input. */
let clientCamY = 0;
/** Client-side camera zoom for overlays and input. */
let clientZoom = 1;
/** Whether local zoom/pan overrides the automatic overview or follow camera. */
let manualCamera = false;
/** Screen-space anchor and button for the current camera drag. */
let cameraPan: { button: number; startX: number; startY: number; lastX: number; lastY: number; moved: boolean } | null = null;
/** Suppress snake selection from the click generated after a camera drag. */
let suppressCanvasClick = false;

/** Current visualization payload for the brain viz tab. */
let currentVizData: VizData | null = null;
/** Whether an export request is pending. */
let pendingExport = false;
/** Whether the connected server supports an opaque browser-managed save download. */
let serverArchiveExport = false;
/** Last production stats generation whose compact Hall of Fame was requested. */
let serverHallOfFameGeneration = 0;
/** Whether the connected server accepts the selected save without browser parsing. */
let serverArchiveImport = false;

/** Proxy world exposed to UI helpers and HoF spawn. */
const proxyWorld: ProxyWorld = {
  generation: 1,
  zoom: 1.0,
  cameraX: 0,
  cameraY: 0,
  viewMode: 'overview',
  fitnessHistory: fitnessHistory,
  // Helpers mimicking World for Settings UI/Persistence
  toggleViewMode: () => {
    if (wsClient && wsClient.isConnected()) {
      resetManualCamera();
      proxyWorld.viewMode = proxyWorld.viewMode === 'overview' ? 'follow' : 'overview';
      wsClient.sendView({
        mode: proxyWorld.viewMode === 'overview' ? 'overview' : 'follow',
        viewW: cssW,
        viewH: cssH
      });
    }
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
  connectionStatus.classList.remove('connecting', 'server');
  connectionStatus.classList.add(mode);
  if (mode === 'server' && serverWorldSeed !== null && serverInferenceMode) {
    const runtime = formatServerRuntimeStatus(
      serverWorldSeed,
      serverInferenceMode
    );
    connectionStatus.textContent = serverRecovery
      ? runtime.replace('Server ·', serverRecovery.explicitResume ? 'Server · selected checkpoint ·' : 'Server · recovered ·')
      : serverImportBranch
        ? runtime.replace('Server ·', 'Server · imported branch ·')
        : serverLegacyConversion
          ? runtime.replace('Server ·', 'Server · converted save ·')
          : runtime;
    connectionStatus.setAttribute(
      'title',
      serverRecovery
        ? formatRecoveryRuntimeStatus(serverRecovery)
        : serverImportBranch
          ? formatImportBranchRuntimeStatus(serverImportBranch)
          : serverLegacyConversion
            ? formatLegacyConversionRuntimeStatus(serverLegacyConversion)
            : ''
    );
  } else if (mode === 'server') {
    connectionStatus.textContent = 'Server';
    connectionStatus.setAttribute('title', '');
  } else {
    connectionStatus.textContent = 'Connecting';
    connectionStatus.setAttribute('title', '');
  }
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
 * Format the generation timer string for display.
 * @param elapsed - Elapsed seconds in the current generation.
 * @param duration - Total generation duration in seconds.
 * @returns Display string in `elapsed/total` format.
 */
function formatGenTimer(elapsed: number, duration: number): string {
  return formatDurationTimer(elapsed) + '/' + formatDurationTimer(duration);
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
  pointerScreen = null;
  boostHeld = false;
  resumePlayerAfterReconnect = false;
  resumeSpectatorAfterReconnect = true;
  playerResumeToken = '';
  playerActionPump.stop();
  try {
    localStorage.removeItem(PLAYER_RESUME_TOKEN_KEY);
  } catch {
    // Ignore storage failures in non-browser environments.
  }
  proxyWorld.viewMode = 'overview';
  resetManualCamera();
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
  resumeSpectatorAfterReconnect = false;
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
  resetManualCamera();
  wsClient.sendJoin('player', name, playerResumeToken || undefined);
  wsClient.sendView({ mode: 'follow', viewW: cssW, viewH: cssH });
}

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
  return customGraphSpec ?? graphDraft ?? buildLinearMlpTemplate(readSettingsFromCoreUI(), CFG.brain);
}

/**
 * Return the current draft graph spec, falling back to active/default.
 * @returns Draft graph spec.
 */
function getDraftGraphSpec(): GraphSpec {
  return graphDraft ?? customGraphSpec ?? buildLinearMlpTemplate(readSettingsFromCoreUI(), CFG.brain);
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

/** Project sizes through the current browser configuration snapshot. */
function applyGraphSizeInference(spec: GraphSpec, state: GraphSizeState): void {
  projectGraphSizeInference(spec, state, CFG.brain);
}

/** Render and edit through the existing graph owner. */
function applySimpleOutputSelection(): void {
  applySimpleOutputSelectionSettings(getGraphSettingsPort());
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
 * Reset the graph draft to the applied or default graph.
 */
function resetGraphDraft(): void {
  if (customGraphSpec) {
    setGraphDraft(customGraphSpec, 'Editor reset to applied graph.');
    return;
  }
  setGraphDraft(buildLinearMlpTemplate(readSettingsFromCoreUI(), CFG.brain), 'Editor reset to default graph.');
}

/** Refresh presets through the existing browser session and graph draft owner. */
function refreshSavedPresets(): Promise<void> {
  return refreshGraphPresets(getGraphPresetPort());
}

/**
 * Load the stored graph spec from localStorage or fall back to defaults.
 */
function loadStoredGraphSpec(): void {
  try {
    const raw = localStorage.getItem(GRAPH_SPEC_STORAGE_KEY);
    if (!raw) {
      stageStackGraph('Using stack controls.');
      return;
    }
    const parsed = JSON.parse(raw) as GraphSpec;
    const result = validateGraph(parsed);
    if (result.ok) {
      const applied = applyGraphSpec(parsed, 'Loaded saved graph.');
      if (!applied) {
        stageStackGraph('Stored graph invalid. Using stack controls.');
      }
    } else {
      stageStackGraph('Stored graph invalid. Using stack controls.');
    }
  } catch {
    stageStackGraph('Stored graph invalid. Using stack controls.');
  }
}

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

/** Render through the existing editor owner without copying its mutable state. */
function renderGraphDiagram(spec: GraphSpec): void {
  renderGraphDiagramView(spec, {
    get graphDiagram() { return graphDiagram; },
    get graphDiagramNodeRects() { return graphDiagramNodeRects; },
    get graphSizeState() { return graphSizeState; },
    updateGraphSizeState,
    get graphLayoutOverrides() { return graphLayoutOverrides; },
    get graphSelectedEdgeIndex() { return graphSelectedEdgeIndex; },
    setGraphSelection,
    get graphConnectLine() { return graphConnectLine; },
    set graphConnectLine(value) { graphConnectLine = value; },
    get graphSelectedOutputIndex() { return graphSelectedOutputIndex; },
    get graphSelectedNodeId() { return graphSelectedNodeId; },
    beginGraphConnectDrag,
    getSvgPoint,
    get graphDragState() { return graphDragState; },
    set graphDragState(value) { graphDragState = value; },
    get graphConnectDrag() { return graphConnectDrag; },
    get graphPointerPos() { return graphPointerPos; },
    updateGraphConnectDrag,
  });
}

/** Render through the existing editor owner without copying its mutable state. */
function renderGraphInspector(spec: GraphSpec): void {
  renderGraphInspectorView(spec, {
    get graphDiagramInspector() { return graphDiagramInspector; },
    get graphSizeState() { return graphSizeState; },
    updateGraphSizeState,
    get graphSelectedNodeId() { return graphSelectedNodeId; },
    set graphSelectedNodeId(value) { graphSelectedNodeId = value; },
    setGraphSpecStatus,
    get graphLayoutOverrides() { return graphLayoutOverrides; },
    renderGraphEditor,
    setGraphSelection,
    get graphSelectedEdgeIndex() { return graphSelectedEdgeIndex; },
    get graphSelectedOutputIndex() { return graphSelectedOutputIndex; },
    get brainInputSize() { return CFG.brain.inSize; },
    createNode: (type, id) => buildDefaultNode(type, id, CFG.brain, readSettingsFromCoreUI()),
  });
}

/** Render and edit through the existing graph owner. */
function renderGraphEditor(): void {
  renderGraphEditorSettings(getGraphSettingsPort());
}

/** Project the existing browser controls without moving their ownership. */
function applySettingsLock(): void {
  projectSettingsLock({ settingsTab, settingsControls, toggleSettingsLock, settingsLockHint,
    get settingsLocked() { return settingsLocked; }, resolveSettingsRoot, refreshCoreUIState });
}

/**
 * Check whether player control is currently active.
 * @returns True when connected and controlling a snake.
 */
function isPlayerControlActive(): boolean {
  return connectionMode === 'server' && !!playerSnakeId;
}

/**
 * Build the newest browser-player command at transmission time.
 * Pointer screen coordinates are converted using the latest camera and player pose.
 * @returns Latest Protocol 2 action or null while ownership/state is incomplete.
 */
function buildLatestPlayerAction(): LatestPlayerAction | null {
  if (!wsClient || !wsClient.isConnected() || !isPlayerControlActive()) return null;
  const meta = playerSensorMeta;
  const target = pointerScreen ? screenToWorld(pointerScreen.x, pointerScreen.y) : null;
  const turn = meta && target ? computeTurnInput(meta, target) : 0;
  const boost = boostHeld ? 1 : 0;
  const tick = playerSensorTick ? playerSensorTick + 1 : 0;
  return { tick, snakeId: playerSnakeId!, turn, boost };
}

/** Selected 30/60 Hz Stage 2 measurement candidate for temporary browser resends. */
const playerActionRateHz = normalizePlayerActionRate(
  new URLSearchParams(window.location.search).get('playerActionHz')
);
/** Browser-player sender independent from display and sensor message delivery. */
const playerActionPump = new PlayerActionPump({
  cadenceHz: playerActionRateHz,
  isActive: isPlayerControlActive,
  buildLatestAction: buildLatestPlayerAction,
  sendAction: action =>
    wsClient?.sendAction(action.tick, action.snakeId, action.turn, action.boost)
});

/**
 * Store a new frame buffer and update generation stats.
 * @param buffer - Raw frame buffer from the server.
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

/** Use the same local camera owner for rendering, input and selection. */
function publishClientCamera(): void {
  return publishClientCameraCamera(getCameraPort());
}

/** Use the same local camera owner for rendering, input and selection. */
function resetManualCamera(): void {
  return resetManualCameraCamera(getCameraPort());
}

/** Use the same local camera owner for rendering, input and selection. */
function fitArenaView(): void {
  return fitArenaViewCamera(getCameraPort());
}

/** Use the same local camera owner for rendering, input and selection. */
function updateClientCamera(): void {
  return updateClientCameraCamera(getCameraPort());
}

/** Ask the Rust server to resurrect one retained winner by its opaque compact identity. */
async function resurrectHallOfFameOnServer(entryId: string): Promise<number | null> {
  const base = resolveServerHttpBase(serverUrl || resolveServerUrl());
  if (!base) return null;
  const response = await fetch(`${base}/api/resurrect`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ entryId })
  });
  const result = await response.json() as { ok?: boolean; snakeId?: number; message?: string };
  if (!response.ok || result.ok !== true || !Number.isSafeInteger(result.snakeId)) {
    console.error('Rust resurrection failed:', result.message ?? response.statusText);
    return null;
  }
  return result.snakeId!;
}

/**
 * Validate the applied graph spec against the active sensor input size.
 * @param settings - Core values used to build the ordinary stack graph.
 * @returns A matching graph, or null after displaying why reset is blocked.
 */
function resolveGraphSpecForReset(settings: CoreSettings): GraphSpec | null {
  const graphSpec = customGraphSpec ?? buildStackGraphSpec(settings, CFG);
  const inputSize = getGraphSpecInputSize(graphSpec);
  if (inputSize == null || inputSize !== CFG.brain.inSize) {
    setResetStatus(`The selected sensors require ${CFG.brain.inSize} graph inputs. Update the custom graph or select the default stack before resetting.`, true);
    return null;
  }
  return graphSpec;
}

/**
 * Apply a full reset through the authoritative server.
 */
function applyResetToSimulation(): void {
  if (!validateSettingDrafts(resolveSettingsRoot())) {
    setResetStatus('Correct the invalid settings before Apply and reset.', true);
    return;
  }
  updateCFGFromUI(resolveSettingsRoot());
  refreshCoreUIState();
  persistBaselineBotSettings();
  const settings = readSettingsFromCoreUI();
  const updates = collectSettingsUpdatesFromUI();
  if (wsClient && wsClient.isConnected()) {
    const graphSpec = resolveGraphSpecForReset(settings);
    if (!graphSpec) return;
    authoritativeControls.dispose();
    setResetStatus('Checking settings and preparing reset...');
    wsClient.sendReset(settings, updates, graphSpec);
    return;
  }
  setResetStatus('Reset unavailable: connect to the server first.', true);
  console.warn('[reset] not connected to server');
}

/** Show server-confirmed reset feedback beside its action button without applying local authority. */
function setResetStatus(message: string, failed = false): void {
  if (!resetStatus) return;
  resetStatus.textContent = message;
  resetStatus.hidden = message.length === 0;
  resetStatus.classList.toggle('error', failed);
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
  setConnectionStatus('connecting');
  setJoinOverlayVisible(true);
  setJoinStatus('Connecting...');
  updateJoinControls();
}

/**
 * Apply a complete authoritative settings snapshot received during handshake.
 * @param core - Active server-owned core settings.
 * @param updates - Active server-owned CFG path values.
 */
function applyAuthoritativeSettingsState(
  core: CoreSettings,
  updates: readonly SettingsUpdate[]
): void {
  // Missing extension is an explicit legacy configuration. Reset its presentation
  // drafts so a previously viewed modern run cannot silently upgrade it.
  for (const path of EVOLVED_SETTING_PATHS) setByPath(CFG, path, getByPath(CFG_DEFAULT, path));
  CFG.evolvedRespawn.contractVersion = 0;
  if (CFG.sense.layoutVersion === 'v3') CFG.sense.profile = CFG_DEFAULT.sense.profile;
  for (const key of ['rayMinimum', 'rayCap', 'rayBatch', 'rayMaximumDue'] as const) CFG.sense[key] = CFG_DEFAULT.sense[key];
  CFG.baselineBots.algorithm = 1;
  CFG.baselineBots.behaviorMode = CFG_DEFAULT.baselineBots.behaviorMode;
  CFG.baselineBots.controlDt = CFG_DEFAULT.baselineBots.controlDt;
  serverSimSpeed = core.simSpeed;
  libraryEnvironment.cadence = 1 / 60;
  applyCoreSettingsToUi(core);
  for (const update of updates) {
    if (update.path === 'brain.controlDt') libraryEnvironment.cadence = Number(update.value);
    setByPath(CFG, update.path, coerceSettingsUpdateValue(update.path, update.value));
  }
  syncBrainInputSize();
  applyValuesToSlidersFromCFG(resolveSettingsRoot(), undefined, settingsLocked);
  refreshCoreUIState();
  persistBaselineBotSettings();
}

/** Restore the exact admitted graph, keeping ordinary stack graphs in slider mode. */
function applyAuthoritativeGraphSpec(spec: GraphSpec | undefined, core: CoreSettings): void {
  libraryEnvironment.graphSourceJson = spec ? JSON.stringify(spec) : null;
  if (!spec) return;
  const stack = buildStackGraphSpec(core, CFG);
  if (graphKey(spec) === graphKey(stack)) {
    customGraphSpec = null;
    CFG.brain.graphSpec = null;
    graphDraft = null;
    try { localStorage.removeItem(GRAPH_SPEC_STORAGE_KEY); } catch { /* Storage is optional. */ }
    refreshCoreUIState();
    renderGraphEditor();
    return;
  }
  applyGraphSpec(spec, 'Loaded authoritative server graph.');
}

/**
 * Apply one authoritative normalized live patch to browser display state.
 * @param updates - Server-normalized live path/value pairs.
 */
function applyAuthoritativeLivePatch(updates: readonly LiveSettingsUpdate[]): void {
  let baselineSettingsChanged = false;
  for (const update of updates) {
    if (update.path === 'brain.controlDt') libraryEnvironment.cadence = Number(update.value);
    if (update.path === 'simSpeed') {
      serverSimSpeed = update.value;
      elSimSpeed.value = String(update.value);
      continue;
    }
    setByPath(CFG, update.path, coerceSettingsUpdateValue(update.path, update.value));
    baselineSettingsChanged ||= update.path.startsWith('baselineBots.');
  }
  syncBrainInputSize();
  applyValuesToSlidersFromCFG(resolveSettingsRoot(), undefined, settingsLocked);
  refreshCoreUIState();
  if (baselineSettingsChanged) persistBaselineBotSettings();
}

/** Refresh compact records, clearing former-run buttons when the authoritative run changes. */
function refreshServerHallOfFame(clearExisting = true): void {
  const base = resolveServerHttpBase(serverUrl || resolveServerUrl());
  if (!base) return;
  if (clearExisting) {
    serverHallOfFameGeneration = 0;
    const table = document.getElementById('hofTable');
    if (table) table.innerHTML = '';
  }
  void hof.loadFromServer(base, clearExisting).then(() => updateHoFTable(proxyWorld))
    .catch(err => console.warn('HoF load failed', err));
}

wsClient = createWsClient({
  onConnected: (info) => {
    decisionInspector.context({sessionId: info.sessionId, runId: info.runId, supported: info.capabilities?.decisionInspector === true});
    decisionInspectorView.supported(info.capabilities?.decisionInspector === true);
    entitySelection.reset();
    selectionGeneration = 1; selectionSessionId = info.sessionId;
    entitySelection.setScope({ sessionId: info.sessionId, runId: info.runId, generation: 1 });
    setResetStatus('');
    resetManualCamera();
    storeServerUrl(serverUrl);
    reconnectDelayMs = 1000;
    serverConfigRevision = info.configRevision;
    serverRecovery = info.recovery ?? null;
    serverImportBranch = info.importBranch ?? null;
    serverLegacyConversion = info.legacyConversion ?? null;
    if (serverRecovery) console.warn('[recovery]', formatRecoveryRuntimeStatus(serverRecovery));
    if (serverImportBranch) {
      console.info('[import-branch]', formatImportBranchRuntimeStatus(serverImportBranch));
    }
    if (serverLegacyConversion) {
      console.warn(
        '[legacy-conversion]',
        formatLegacyConversionRuntimeStatus(serverLegacyConversion)
      );
    }
    serverRunId = info.runId;
    pendingFinishRequestId = null;
    if (btnFinishGeneration) {
      btnFinishGeneration.hidden = info.capabilities?.finishGeneration !== true;
      btnFinishGeneration.disabled = settingsLocked;
    }
    if (finishGenerationStatus) finishGenerationStatus.hidden = true;
    serverWorldSeed = info.worldSeed;
    serverInferenceMode = info.inferenceMode;
    serverArchiveExport = info.capabilities?.archiveExport === true;
    serverArchiveImport = info.capabilities?.archiveImport === true;
    if (btnPinCheckpoint) btnPinCheckpoint.hidden = info.capabilities?.checkpointPinning !== true;
    applyAuthoritativeSettingsState(info.settings.core, info.settings.updates);
    applyAuthoritativeSensorSpec(info.sensorSpec);
    libraryEnvironment.sensorSha256 = info.sensorSpec.contract?.sha256 ?? null;
    libraryEnvironment.configHash = info.configHash;
    libraryEnvironment.founderBrainId = info.founder?.artifactSha256;
    brainLibrary.contextChanged(info.capabilities?.savedBrainLibrary === true, info.capabilities?.savedBrainArchive === true, info.capabilities?.savedBrainFounders === true);
    distillationAvailable = info.capabilities?.distillation === true;
    teacherCaptureReady = info.capabilities?.teacherCapture === true;
    distillation.contextChanged(distillationAvailable, teacherCaptureReady, serverRunId);
    applyAuthoritativeGraphSpec(info.graphSpec, info.settings.core);
    spectatorFollowSnakeId = null;
    currentStats = {
      gen: 1,
      generationTime: 0,
      generationSeconds: CFG.generationSeconds,
      alive: 0,
      aliveTotal: 0,
      baselineBotsAlive: 0,
      baselineBotsTotal: 0,
      fps: info.tickRate
    };
    currentVizData = null;
    setConnectionStatus('server');
    if (resumePlayerAfterReconnect && lastPlayerName) {
      joinPending = true;
      proxyWorld.viewMode = 'follow';
      wsClient?.sendJoin('player', lastPlayerName, playerResumeToken || undefined);
      wsClient?.sendView({ mode: 'follow', viewW: cssW, viewH: cssH });
      setJoinOverlayVisible(true);
      setJoinStatus('Reclaiming previous snake...');
    } else {
      joinPending = false;
      wsClient?.sendJoin('spectator');
      setJoinOverlayVisible(!resumeSpectatorAfterReconnect);
      setJoinStatus(resumeSpectatorAfterReconnect ? 'Spectating' : 'Enter a nickname to play');
    }
    wsClient?.sendViz(activeTab === 'tab-viz');
    updateJoinControls();
    refreshSavedPresets().catch(() => { });
    refreshServerHallOfFame();
  },
  onDisconnected: () => {
    pendingFounderStart?.reject(new Error('Connection lost during founder start. Reconnect and inspect the current run before starting another.'));
    decisionInspector.dispose();
    decisionInspectorView.supported(false);
    brainLibrary.contextChanged(false);
    distillationAvailable = teacherCaptureReady = false;
    distillation.contextChanged(false, false, '');
    entitySelection.reset();
    if (btnPinCheckpoint) btnPinCheckpoint.hidden = true;
    serverArchiveExport = false;
    serverArchiveImport = false;
    resumePlayerAfterReconnect =
      playerSnakeId !== null || joinPending || playerResumeToken.length > 0;
    playerActionPump.stop();
    authoritativeControls.dispose();
    setConnectionStatus('connecting');
    serverConfigRevision = 0;
    serverSimSpeed = 1;
    serverWorldSeed = null;
    serverInferenceMode = null;
    serverRecovery = null;
    serverImportBranch = null;
    serverLegacyConversion = null;
    pendingNewRunRequestId = null;
    btnNewRun.disabled = false;
    playerSnakeId = null;
    spectatorFollowSnakeId = null;
    playerSensorTick = 0;
    playerSensorMeta = null;
    boostHeld = false;
    currentVizData = null;
    joinPending = false;
    setJoinOverlayVisible(true);
    setJoinStatus('Connecting...');
    updateJoinControls();
    scheduleReconnect();
  },
  onFrame: (buffer) => {
    applyFrameBuffer(buffer);
    entitySelection.observeFrame(selectionGeneration, frameContainsSnake, performance.now());
  },
  onStats: (msg) => {
    if (msg.gen !== selectionGeneration) {
      selectionGeneration = msg.gen;
      entitySelection.setScope({ sessionId: selectionSessionId, runId: serverRunId, generation: msg.gen });
    }
    if (serverArchiveExport && msg.gen !== serverHallOfFameGeneration) {
      serverHallOfFameGeneration = msg.gen;
      refreshServerHallOfFame(false);
    }
    const aliveTotal = Number.isFinite(msg.aliveTotal) ? msg.aliveTotal : msg.alive;
    const baselineBotsAlive = Number.isFinite(msg.baselineBotsAlive) ? msg.baselineBotsAlive : 0;
    const baselineBotsTotal = Number.isFinite(msg.baselineBotsTotal) ? msg.baselineBotsTotal : 0;
    currentStats = {
      ...currentStats,
      gen: msg.gen,
      generationTime: msg.generationTime,
      generationSeconds: msg.generationSeconds,
      alive: msg.alive,
      aliveTotal,
      baselineBotsAlive,
      baselineBotsTotal,
      fps: msg.fps
    };
    const evolved = parseEvolvedLifecycleTelemetry(msg.evolved);
    if (evolved) currentStats.evolved = evolved;
    else delete currentStats.evolved;
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
    } else { currentVizData = null; }
    if (msg.hofEntry) {
      void hof.add(msg.hofEntry).then(() => {
        if (connectionMode === 'server') {
          const base = resolveServerHttpBase(serverUrl || resolveServerUrl());
          if (base) void hof.syncToServer(base);
        }
      });
    }
  },
  onAssign: (msg) => {
    playerSnakeId = msg.snakeId;
    resetManualCamera();
    playerResumeToken = msg.resumeToken;
    resumePlayerAfterReconnect = true;
    try {
      localStorage.setItem(PLAYER_RESUME_TOKEN_KEY, msg.resumeToken);
    } catch {
      // Ignore storage failures in non-browser environments.
    }
    joinPending = false;
    setJoinOverlayVisible(false);
    setJoinStatus(msg.reclaimed ? 'Reconnected' : 'Connected');
    updateJoinControls();
    playerActionPump.start();
    playerActionPump.requestImmediate();
  },
  onReclaimResult: (msg) => {
    if (msg.reclaimed) return;
    playerActionPump.stop();
    playerResumeToken = '';
    resumePlayerAfterReconnect = false;
    joinPending = false;
    try {
      localStorage.removeItem(PLAYER_RESUME_TOKEN_KEY);
    } catch {
      // Ignore storage failures in non-browser environments.
    }
    setJoinOverlayVisible(true);
    setJoinStatus(
      msg.reason === 'expired'
        ? 'Previous control expired; press Play to join again'
        : `Could not reclaim previous snake (${msg.reason}); press Play to join`
    );
    updateJoinControls();
  },
  onSensors: (msg) => {
    if (!playerSnakeId || msg.snakeId !== playerSnakeId) return;
    playerSensorTick = msg.tick;
    if (msg.meta) {
      playerSensorMeta = msg.meta;
    }
  },
  onSettingsApplied: (msg) => {
    if (msg.configRevision < serverConfigRevision) return;
    serverConfigRevision = msg.configRevision;
    if (msg.applied) {
      applyAuthoritativeLivePatch(msg.updates);
      libraryEnvironment.configHash = msg.configHash;
      brainLibrary.environmentChanged();
      distillation.contextChanged(distillationAvailable, teacherCaptureReady, serverRunId);
      return;
    }
    applyValuesToSlidersFromCFG(resolveSettingsRoot(), undefined, settingsLocked);
    elSimSpeed.value = String(serverSimSpeed);
    refreshCoreUIState();
    console.warn(`[settings] ${msg.reason ?? 'request rejected'}`);
  },
  onEntitySelected: msg => entitySelection.receive(msg),
  onInspectorResult: msg => decisionInspector.result(msg),
  onInspector: msg => decisionInspector.snapshot(msg.snapshot),
  onGodModeResult: (msg) => {
    godModeLog.push({
      time: Date.now(),
      action: msg.action === 'move' ? 'drag' : 'kill',
      snakeId: msg.snakeId,
      result: msg.applied ? 'success' : `rejected: ${msg.reason ?? 'unknown reason'}`
    });
    if (msg.applied && msg.action === 'move' && selectedSnake && selectedSnake.id === msg.snakeId && selectedSnake.token === msg.selectionToken) {
      if (Number.isFinite(msg.x)) selectedSnake.x = msg.x!;
      if (Number.isFinite(msg.y)) selectedSnake.y = msg.y!;
    }
  },
  onFinishGenerationResult: (msg) => {
    if (msg.requestId !== pendingFinishRequestId || msg.runId !== serverRunId) return;
    if (finishGenerationStatus) {
      finishGenerationStatus.hidden = false;
      finishGenerationStatus.textContent = msg.status === 'pending'
        ? 'Finishing generation; waiting for authoritative checkpoint and successor...'
        : msg.status === 'completed' ? 'Generation ' + msg.generation + ' finished and saved.'
          : 'Finish rejected: ' + (msg.reason ?? 'request rejected');
    }
    if (msg.status !== 'pending') {
      pendingFinishRequestId = null;
      if (btnFinishGeneration) btnFinishGeneration.disabled = settingsLocked;
    }
  },
  onNewRunResult: (msg) => {
    if (pendingFounderStart?.requestId === msg.requestId) {
      if (msg.applied) pendingFounderStart.resolve();
      else pendingFounderStart.reject(new Error(msg.reason ?? 'Founder run rejected.'));
    }
    if (pendingNewRunRequestId === msg.requestId) {
      pendingNewRunRequestId = null;
      btnNewRun.disabled = false;
    }
    if (!msg.applied) {
      console.warn(`[new-run] ${msg.reason ?? 'request rejected'}`);
      return;
    }
    if (Number.isFinite(msg.worldSeed)) {
      serverWorldSeed = msg.worldSeed!;
      serverRecovery = null;
      serverImportBranch = null;
      serverLegacyConversion = null;
      setConnectionStatus('server');
    }
    entitySelection.clear();
    console.info(`[new-run] started seed ${msg.worldSeed ?? 'unknown'}`);
  },
  onStateReplaced: (msg) => {
    setResetStatus(msg.reason === 'reset' ? 'Reset applied.' : '');
    const info = msg.welcome;
    decisionInspector.context({sessionId: info.sessionId, runId: info.runId, supported: info.capabilities?.decisionInspector === true});
    decisionInspectorView.supported(info.capabilities?.decisionInspector === true);
    entitySelection.reset();
    selectionGeneration = 1; selectionSessionId = info.sessionId;
    entitySelection.setScope({ sessionId: info.sessionId, runId: info.runId, generation: 1 });
    const rejoinPlayer = playerSnakeId !== null || joinPending;
    playerActionPump.stop();
    authoritativeControls.dispose();
    playerSnakeId = null;
    resetManualCamera();
    spectatorFollowSnakeId = null;
    playerSensorTick = 0;
    playerSensorMeta = null;
    playerResumeToken = '';
    resumePlayerAfterReconnect = false;
    boostHeld = false;
    selectedSnake = null;
    currentVizData = null;
    try { localStorage.removeItem(PLAYER_RESUME_TOKEN_KEY); } catch { /* Storage is optional. */ }
    serverConfigRevision = info.configRevision;
    serverRunId = info.runId;
    pendingFinishRequestId = null;
    if (btnFinishGeneration) {
      btnFinishGeneration.hidden = info.capabilities?.finishGeneration !== true;
      btnFinishGeneration.disabled = settingsLocked;
    }
    if (finishGenerationStatus) finishGenerationStatus.hidden = true;
    serverWorldSeed = info.worldSeed;
    serverRecovery = info.recovery ?? null;
    serverImportBranch = info.importBranch ?? null;
    serverLegacyConversion = info.legacyConversion ?? null;
    serverInferenceMode = info.inferenceMode;
    serverArchiveExport = info.capabilities?.archiveExport === true;
    serverArchiveImport = info.capabilities?.archiveImport === true;
    applyAuthoritativeSettingsState(info.settings.core, info.settings.updates);
    applyAuthoritativeSensorSpec(info.sensorSpec);
    libraryEnvironment.sensorSha256 = info.sensorSpec.contract?.sha256 ?? null;
    libraryEnvironment.configHash = info.configHash;
    libraryEnvironment.founderBrainId = info.founder?.artifactSha256;
    brainLibrary.contextChanged(info.capabilities?.savedBrainLibrary === true, info.capabilities?.savedBrainArchive === true, info.capabilities?.savedBrainFounders === true);
    distillationAvailable = info.capabilities?.distillation === true;
    teacherCaptureReady = info.capabilities?.teacherCapture === true;
    distillation.contextChanged(distillationAvailable, teacherCaptureReady, serverRunId);
    applyAuthoritativeGraphSpec(info.graphSpec, info.settings.core);
    if (btnPinCheckpoint) btnPinCheckpoint.hidden = info.capabilities?.checkpointPinning !== true;
    refreshServerHallOfFame();
    setConnectionStatus('server');
    joinPending = rejoinPlayer;
    setJoinOverlayVisible(true);
    const replacementLabel = msg.reason === 'import' ? 'imported run' :
      msg.reason === 'reset' ? 'reset run' : 'new run';
    setJoinStatus(rejoinPlayer ? `Joining ${replacementLabel}...` : `${replacementLabel[0]!.toUpperCase()}${replacementLabel.slice(1)} ready`);
    updateJoinControls();
    if (rejoinPlayer && lastPlayerName) {
      proxyWorld.viewMode = 'follow';
      wsClient?.sendJoin('player', lastPlayerName);
      wsClient?.sendView({ mode: 'follow', viewW: cssW, viewH: cssH });
    } else {
      joinPending = false;
      proxyWorld.viewMode = 'overview';
      wsClient?.sendJoin('spectator');
      wsClient?.sendView({ mode: 'overview', viewW: cssW, viewH: cssH });
      setJoinOverlayVisible(false);
    }
    wsClient?.sendViz(activeTab === 'tab-viz');
    console.info(`[import] activated ${msg.checkpointId.slice(0, 12)} from run ${info.runId}`);
  },
  onError: (msg) => {
    console.warn(`[ws] ${msg.message}`);
    if (msg.message.startsWith('reset failed:')) {
      setResetStatus(`Reset failed: ${msg.message.slice('reset failed:'.length).trim()}`, true);
    }
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
  console.error('WebSocket not supported');
} else {
  connectToServer(resolveServerUrl());
}

/**
 * Live update handler for sliders that do not require a reset.
 * @param sliderEl - Slider input element to read.
 */
function liveUpdateFromSlider(sliderEl: SettingInputElement): void {
  const path = sliderEl.dataset['path'] as LiveSettingPath | undefined;
  if (!path) return;
  const value = readSettingsInputValue(sliderEl);
  if (value == null) return;
  if (BROWSER_CAMERA_SETTING_PATHS.has(path)) {
    setByPath(CFG, path, normalizeSettingValue(SETTING_DEFINITION_BY_PATH.get(path)!, value));
    return;
  }
  if (!wsClient?.isConnected()) return;
  authoritativeControls.queueSetting(path, value);
}

// Live update simulation speed when the slider moves
elSimSpeed.addEventListener('input', () => {
  refreshCoreUIState();
  const value = Number(elSimSpeed.value);
  if (wsClient?.isConnected() && Number.isFinite(value)) {
    authoritativeControls.queueSetting('simSpeed', value);
  }
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
  applyResetToSimulation();
});
// Restore defaults
btnDefaults.addEventListener('click', () => {
  resetCFGToDefaults();
  if (followIndividualInput) followIndividualInput.checked = CFG.diagnostics.followIndividual;
  if (decisionInspectorInput) decisionInspectorInput.checked = CFG.diagnostics.inspectorEnabled;
  decisionInspector.setEnabled(CFG.diagnostics.inspectorEnabled);
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
  stageStackGraph('Default stack selected.');
  refreshCoreUIState();
  applySettingsLock();
  persistBaselineBotSettings();
  applyResetToSimulation();
});
// Confirm the observed generation and request its next complete terminal step.
btnFinishGeneration?.addEventListener('click', () => {
  if (!wsClient?.isConnected() || pendingFinishRequestId || !serverRunId) return;
  const generation = currentStats.gen;
  const elapsed = currentStats.generationTime;
  if (!window.confirm('Finish generation ' + generation + ' now at ' + elapsed.toFixed(1) + ' simulated seconds?')) return;
  pendingFinishRequestId = createRequestId();
  btnFinishGeneration.disabled = true;
  if (finishGenerationStatus) {
    finishGenerationStatus.hidden = false;
    finishGenerationStatus.textContent = 'Requesting authoritative generation completion...';
  }
  wsClient.sendFinishGeneration(pendingFinishRequestId, serverRunId, generation);
});
// Start a separately seeded run after its generation-one checkpoint commits.
btnNewRun.addEventListener('click', () => {
  if (!wsClient?.isConnected() || pendingNewRunRequestId !== null) return;
  pendingNewRunRequestId = authoritativeControls.requestNewRun();
  btnNewRun.disabled = true;
});
// Toggle view mode
btnToggle.addEventListener('click', () => proxyWorld.toggleViewMode());
btnFitArena.addEventListener('click', fitArenaView);
window.addEventListener('keydown', e => {
  const target = e.target as HTMLElement | null;
  if (target?.closest?.('input, textarea, select, [contenteditable]')) return;
  if (e.code === 'KeyV') {
    proxyWorld.toggleViewMode();
  } else if (e.code === 'Home') {
    e.preventDefault();
    fitArenaView();
  }
});

/** Use the same local camera owner for rendering, input and selection. */
function screenToWorld(screenX: number, screenY: number): { x: number; y: number } {
  return screenToWorldCamera(screenX, screenY, getCameraPort());
}

/** Query the current packed frame without copying world state. */
function frameContainsSnake(targetId: number): boolean { return containsFrameSnake(currentFrameBuffer, targetId); }

/** Select against the current packed frame. */
function findSnakeNear(worldX: number, worldY: number, maxDist = 50): SelectedSnake | null { return selectFrameSnake(currentFrameBuffer, worldX, worldY, maxDist); }

// ============== START ANIMATION LOOP ==============
requestAnimationFrame(frame);

// Zoom around the pointer so the inspected world position stays under the cursor.
canvas.addEventListener('wheel', e => {
  if (connectionMode !== 'server' || !currentFrameBuffer || !Number.isFinite(e.deltaY)) return;
  e.preventDefault();
  const rect = canvas.getBoundingClientRect();
  const screenX = e.clientX - rect.left;
  const screenY = e.clientY - rect.top;
  const anchor = screenToWorld(screenX, screenY);
  const pixels = e.deltaY * (e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? cssH : 1);
  clientZoom = clamp(clientZoom * Math.exp(clamp(-pixels * 0.0015, -4, 4)), 0.01, 4);
  clientCamX = anchor.x - (screenX - cssW / 2) / clientZoom;
  clientCamY = anchor.y - (screenY - cssH / 2) / clientZoom;
  manualCamera = true;
  publishClientCamera();
}, { passive: false });

/** Use the same local camera owner for rendering, input and selection. */
function moveCameraPan(e: MouseEvent): boolean {
  return moveCameraPanCamera(e, getCameraPort());
}

/** Use the same local camera owner for rendering, input and selection. */
function finishCameraPan(e: MouseEvent): boolean {
  return finishCameraPanCamera(e, getCameraPort());
}

window.addEventListener('mousemove', moveCameraPan);
window.addEventListener('blur', () => {
  cameraPan = null;
  canvas.style.cursor = isPlayerControlActive() ? '' : 'grab';
});

// Click to select snake
canvas.addEventListener('click', (e) => {
  if (suppressCanvasClick) {
    suppressCanvasClick = false;
    return;
  }
  if (isPlayerControlActive()) return;
  const rect = canvas.getBoundingClientRect();
  const screenX = e.clientX - rect.left;
  const screenY = e.clientY - rect.top;
  const world = screenToWorld(screenX, screenY);

  const snake = findSnakeNear(world.x, world.y);
  if (snake) {
    entitySelection.select(snake.id);
    godModeLog.push({
      time: Date.now(),
      action: 'select',
      snakeId: snake.id,
      result: 'pending native selection'
    });
    console.log(`Selected snake #${snake.id} (skin: ${snake.skin})`);
  } else {
    entitySelection.clear();
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
  if (snake && wsClient?.isConnected()) entitySelection.kill(snake.id);
});

// Left-drag pans while spectating; Shift+left-drag moves a selected snake.
canvas.addEventListener('mousedown', (e) => {
  suppressCanvasClick = false;
  if ((e.button === 1 || (e.button === 0 && !isPlayerControlActive() && !(e.shiftKey && selectedSnake))) &&
      connectionMode === 'server' && currentFrameBuffer) {
    e.preventDefault();
    cameraPan = { button: e.button, startX: e.clientX, startY: e.clientY,
      lastX: e.clientX, lastY: e.clientY, moved: false };
    return;
  }
  if (isPlayerControlActive()) {
    if (e.button === 0) boostHeld = true;
    const rect = canvas.getBoundingClientRect();
    pointerScreen = { x: e.clientX - rect.left, y: e.clientY - rect.top };
    playerActionPump.requestImmediate();
    return;
  }
  if (e.button === 0 && e.shiftKey && selectedSnake?.token) {
    isDragging = true;
  }
});

canvas.addEventListener('mousemove', (e) => {
  if (moveCameraPan(e)) return;
  if (isPlayerControlActive()) {
    const rect = canvas.getBoundingClientRect();
    pointerScreen = { x: e.clientX - rect.left, y: e.clientY - rect.top };
    playerActionPump.requestImmediate();
    return;
  }
  if (isDragging && selectedSnake?.token) {
    const rect = canvas.getBoundingClientRect();
    const screenX = e.clientX - rect.left;
    const screenY = e.clientY - rect.top;
    const world = screenToWorld(screenX, screenY);
    if (wsClient?.isConnected()) {
      authoritativeControls.moveSnake(selectedSnake.id, world.x, world.y, selectedSnake.token);
    }
  }
});

/**
 * Finish a God Mode drag and always send the release position immediately.
 * @param e - Mouse-up event whose client coordinates define the final target.
 */
function finishGodModeDrag(e: MouseEvent): void {
  if (finishCameraPan(e)) return;
  if (isPlayerControlActive()) {
    if (e.button === 0) {
      boostHeld = false;
      const rect = canvas.getBoundingClientRect();
      pointerScreen = { x: e.clientX - rect.left, y: e.clientY - rect.top };
      playerActionPump.requestImmediate();
    }
    return;
  }
  if (!isDragging || e.button !== 0) return;
  isDragging = false;
  if (!selectedSnake?.token || !wsClient?.isConnected()) return;
  const rect = canvas.getBoundingClientRect();
  const world = screenToWorld(e.clientX - rect.left, e.clientY - rect.top);
  authoritativeControls.finishMove(selectedSnake.id, world.x, world.y, selectedSnake.token);
}

canvas.addEventListener('mouseup', finishGodModeDrag);
window.addEventListener('mouseup', finishGodModeDrag);

canvas.addEventListener('mouseleave', () => {
  if (!isPlayerControlActive()) return;
  boostHeld = false;
  pointerScreen = null;
  playerActionPump.requestImmediate();
});

/** Download through the current connection owner and export admission flag. */
function exportServerSnapshot(): Promise<void> {
  return exportArchive(getArchiveTransportPort());
}

// Persistence UI Wiring
/** Button that permanently protects the Rust server's exact current checkpoint. */
const btnPinCheckpoint = document.getElementById('btnPinCheckpoint') as HTMLButtonElement | null;
if (btnPinCheckpoint) {
  btnPinCheckpoint.addEventListener('click', () => {
    const base = resolveServerHttpBase(serverUrl || resolveServerUrl());
    if (!base || btnPinCheckpoint.disabled) return;
    btnPinCheckpoint.disabled = true;
    void fetch(`${base}/api/checkpoints/current/pin`, { method: 'POST' }).then(async response => {
      const result = await response.json() as { ok?: boolean; checkpointId?: string; generation?: string; message?: string };
      if (!response.ok || !result.ok) throw new Error(result.message ?? `pin failed (${response.status})`);
      alert(`Pinned generation ${BigInt(`0x${result.generation ?? '0'}`).toString()} checkpoint ${result.checkpointId?.slice(0, 12) ?? ''}.`);
    }).catch(error => {
      console.error('Checkpoint pin failed', error);
      alert(`Pin failed: ${(error as Error).message}`);
    }).finally(() => { btnPinCheckpoint.disabled = false; });
  });
}
/** Button that triggers exporting population and HoF data. */
const btnExport = document.getElementById('btnExport') as HTMLButtonElement | null;
if (btnExport) {
  btnExport.addEventListener('click', () => {
    if (pendingExport) return;
    pendingExport = true;
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

  fileInput.addEventListener('change', e => importServerArchive(e, btnImport, getArchiveTransportPort()));
}

/**
 * Main animation frame loop for rendering and UI updates.
 */
function frame(): void {
  // console.log("Frame loop running"); // Spammy
  if (connectionMode === 'server' && currentFrameBuffer) {
    updateClientCamera();
    renderWorldStruct(ctx, currentFrameBuffer, cssW, cssH, clientZoom, clientCamX, clientCamY);
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
    // Fitness chart history is retained in the server stats stream.
  }

  // Updates UI overlay
  // ...
  if (genTimer) {
    const elapsed = currentStats.generationTime;
    const duration = Number.isFinite(currentStats.generationSeconds)
      ? currentStats.generationSeconds
      : CFG.generationSeconds;
    genTimer.textContent = formatGenTimer(elapsed, duration);
  }

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
  const life = currentStats.evolved;
  const lifeInfoHtml = life ?
    `<div class="stat-box"><span class="label">Waiting / pending</span><span class="val">${life.waiting} / ${life.pendingPlacement}</span></div>` +
    `<div class="stat-box"><span class="label">Exhausted individuals</span><span class="val">${life.exhausted}</span></div>` +
    `<div class="stat-box"><span class="label">Deaths / respawns</span><span class="val">${BigInt('0x' + life.deaths)} / ${BigInt('0x' + life.respawns)}</span></div>` +
    `<div class="stat-box"><span class="label">Individual waiting total</span><span class="val">${life.pendingSeconds.toFixed(1)} sim s</span></div>` +
    `<div class="stat-box"><span class="label">Failed placements</span><span class="val">${BigInt('0x' + life.failedPlacements)}</span></div>` +
    `<div class="note">Placement step ${BigInt('0x' + life.placement.step)}: ${life.placement.attempted} attempted, ${life.placement.deferred} deferred; ${life.placement.candidates}/2048 candidates, ${life.placement.geometryChecks}/250000 geometry checks.</div>` : '';
  const statsInfoHtml =
    `<div class="stat-box"><span class="label">Generation</span><span class="val">${currentStats.gen}</span></div>` +
    `<div class="stat-box"><span class="label">Alive (Pop)</span><span class="val">${currentStats.alive}</span></div>` +
    `<div class="stat-box"><span class="label">Alive (Total)</span><span class="val">${aliveTotal}</span></div>` +
    `<div class="stat-box"><span class="label">Baseline Bots</span><span class="val">${baselineBotsAlive}/${baselineBotsTotal}</span></div>` +
    `<div class="stat-box"><span class="label">Sim Speed</span><span class="val">${Math.round(currentStats.fps)} FPS</span></div>` +
    lifeInfoHtml +
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
async function updateHoFTable(world: ProxyWorld): Promise<void> {
  const container = document.getElementById('hofTable');
  if (!container) return;

  // Throttle updates?
  if (world.generation % 1 !== 0 && Math.random() > 0.1) return;

  const list = await hof.getAll() as HallOfFameEntry[];
  if (!list.length) {
    container.innerHTML = '<div style="padding:10px; color:#aaa">No records yet.</div>';
    return;
  }

  let html = '';
  list.forEach((entry: HallOfFameEntry, idx: number) => {
    const pointLabel = entry.assessment?.pointMethod === 'reset' ? 'Reset' : entry.assessment?.pointMethod === 'cumulative-net' ? 'Cumulative net' : entry.assessment?.pointMethod === 'cumulative-gross' ? 'Cumulative gross' : entry.assessment?.pointMethod === 'best-life' ? 'Best Life' : 'Legacy physical score';
    const lengthLabel = entry.assessment?.displayLengthKind === 'generation-peak' ? 'generation peak length' : entry.assessment?.displayLengthKind === 'selected-life-peak' ? 'selected life peak length' : entry.assessment ? 'latest life length' : 'legacy physical length';
    html += `
      <div class="hof-item">
        <span>#${idx + 1} Gen ${entry.gen} (Fit ${entry.fitness.toFixed(1)})<br><small>${pointLabel}: ${entry.points.toFixed(1)} · ${lengthLabel}: ${entry.length}</small></span>
        <button onclick="window.spawnHoF(${idx})"${entry.entryId ? '' : ' disabled'}>Spawn</button>
      </div>`;
  });
  container.innerHTML = html;
}

/** Expose global helper for HoF spawn buttons. */
window.spawnHoF = async function (idx) {
  const list = await hof.getAll();
  const entry = list[idx];
  if (entry && window.currentWorld) {
    const spawnedId = entry.entryId && connectionMode === 'server'
      ? await resurrectHallOfFameOnServer(entry.entryId)
      : null;
    if (spawnedId != null && connectionMode === 'server') {
      spectatorFollowSnakeId = spawnedId;
      proxyWorld.viewMode = 'follow';
    }
    // Switch to visualizer?
    // Actually resurrect sets viewMode to follow, so we just need to ensure correct tab?
    // Let's auto-switch tab too.
    (document.querySelector('[data-tab="tab-viz"]') as HTMLButtonElement).click();
  }
};


/** Provide live editor state without creating another mutable owner. */
function getGraphSettingsPort(): GraphSettingsPort {
  return {
    get graphOutputSimpleNode() { return graphOutputSimpleNode; },
    get graphOutputSimpleSplit() { return graphOutputSimpleSplit; },
    get graphOutputSimpleHint() { return graphOutputSimpleHint; },
    ensureGraphDraft,
    setGraphSpecStatus,
    get graphNodes() { return graphNodes; },
    get graphEdges() { return graphEdges; },
    get graphOutputs() { return graphOutputs; },
    updateGraphSizeState,
    applyGraphSizeInference,
    syncGraphSelection,
    get graphLayoutOverrides() { return graphLayoutOverrides; },
    get graphSelectedNodeId() { return graphSelectedNodeId; },
    set graphSelectedNodeId(value) { graphSelectedNodeId = value; },
    renderGraphDiagram,
    renderGraphInspector,
    get brainOutputSize() { return CFG.brain.outSize; },
    get brainInputSize() { return CFG.brain.inSize; },
    createNode: (type, id) => buildDefaultNode(type, id, CFG.brain, readSettingsFromCoreUI()),
  };
}


/** Pass the one browser connection/draft owner to preset operations. */
function getGraphPresetPort(): GraphPresetPort {
  return { graphPresetList, graphPresetName, resolveBaseUrl: resolveServerHttpUrl,
    setGraphSpecStatus, setGraphDraft, getDraftGraphSpec };
}


/** Share the existing connection/capability and download admission state. */
function getArchiveTransportPort(): ArchiveTransportPort {
  return { get serverWsUrl() { return serverUrl || resolveServerUrl(); },
    get serverArchiveExport() { return serverArchiveExport; },
    get serverArchiveImport() { return serverArchiveImport; },
    get pendingExport() { return pendingExport; },
    set pendingExport(value) { pendingExport = value; },
    isConnected: () => Boolean(wsClient?.isConnected()) };
}


/** Supply the already registered core controls at the current projection boundary. */
function getCoreInputElements(): CoreInputElements { return { elSnakes, elSimSpeed, elLayers, elN1, elN2, elN3, elN4, elN5 }; }


/** Cache the live camera port so delegation adds no per-frame port allocation. */
function getCameraPort(): CameraPort {
  return cachedCameraPort ??= {
    get proxyWorld() { return proxyWorld; },
    get clientCamX() { return clientCamX; },
    set clientCamX(value) { clientCamX = value; },
    get clientCamY() { return clientCamY; },
    set clientCamY(value) { clientCamY = value; },
    get clientZoom() { return clientZoom; },
    set clientZoom(value) { clientZoom = value; },
    get manualCamera() { return manualCamera; },
    set manualCamera(value) { manualCamera = value; },
    get cameraPan() { return cameraPan; },
    set cameraPan(value) { cameraPan = value; },
    get suppressCanvasClick() { return suppressCanvasClick; },
    set suppressCanvasClick(value) { suppressCanvasClick = value; },
    get canvas() { return canvas; },
    isPlayerControlActive,
    get currentFrameBuffer() { return currentFrameBuffer; },
    get cssW() { return cssW; },
    get cssH() { return cssH; },
    get selectedSnake() { return selectedSnake; },
    get playerSnakeId() { return playerSnakeId; },
    get spectatorFollowSnakeId() { return spectatorFollowSnakeId; },
    frameContainsSnake,
    get settings() { return CFG; },
    get isServerMode() { return connectionMode === 'server'; },
    get followIndividual() { return followIndividualInput?.checked ?? true; },
    followedPopulationSlot: () => entitySelection.followedPopulationSlot(),
    sendOverview: () => { wsClient?.sendView({ mode: 'overview', viewW: cssW, viewH: cssH }); },
  };
}
