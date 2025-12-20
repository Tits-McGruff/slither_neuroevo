// main.ts
// Entry point for the slither simulation.  Sets up the canvas, UI,
// Entry point for the slither simulation. Sets up the canvas, UI,
// constructs the World and runs the animation loop. All global
// functions and classes defined in other modules must be loaded before
// this script executes.

import { CFG, resetCFGToDefaults } from './config.ts';
import { setupSettingsUI, updateCFGFromUI } from './settings.ts';
import { setByPath, clamp, TAU } from './utils.ts';
import { renderWorldStruct } from './render.ts';
// import { World } from './world.ts'; // Logic moved to worker
import { savePopulation, loadPopulation, exportToFile, importFromFile } from './storage.ts';
import { hof } from './hallOfFame.ts';
import { BrainViz } from './BrainViz.ts';
import { FitnessChart } from './FitnessChart.ts';
import { AdvancedCharts } from './chartUtils.ts';
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

declare global {
  interface Window {
    ctx: CanvasRenderingContext2D;
    currentWorld: ProxyWorld;
    spawnHoF: (idx: number) => void;
  }
}

// Canvas and HUD
let worker: Worker | null = null;
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

// Tabs and visualizers
const tabBtns = document.querySelectorAll<HTMLButtonElement>('.tab-btn');
const tabContents = document.querySelectorAll<HTMLElement>('.tab-content');
const vizCanvas = document.getElementById('vizCanvas') as HTMLCanvasElement;
const statsCanvas = document.getElementById('statsCanvas') as HTMLCanvasElement;
const ctxViz = vizCanvas.getContext('2d')!;
const ctxStats = statsCanvas.getContext('2d')!;

const brainViz = new BrainViz(0, 0, vizCanvas.width, vizCanvas.height);
const fitChart = new FitnessChart(0, 0, statsCanvas.width, statsCanvas.height);

let activeTab = 'tab-settings';
let statsView = 'fitness';
tabBtns.forEach(btn => {
  btn.addEventListener('click', () => {
    // UI toggle
    tabBtns.forEach(b => b.classList.remove('active'));
    tabContents.forEach(c => c.classList.remove('active'));
    btn.classList.add('active');
    const tabId = btn.dataset.tab!;
    document.getElementById(tabId)!.classList.add('active');
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
    btn.classList.toggle('active', btn.dataset.stats === view);
  });
  const meta = statsViewMeta[view];
  if (statsTitle && meta) statsTitle.textContent = meta.title;
  if (statsSubtitle && meta) statsSubtitle.textContent = meta.subtitle;
}

statsViewBtns.forEach(btn => {
  btn.addEventListener('click', () => setStatsView(btn.dataset.stats!));
});
if (statsViewBtns.length) setStatsView(statsView);

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
  const sliders = root.querySelectorAll<HTMLInputElement>('input[type="range"][data-path]');
  const updates: SettingsUpdate[] = [];
  sliders.forEach(sl => {
    updates.push({ path: sl.dataset.path! as SettingsUpdate['path'], value: Number(sl.value) });
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
  elN2.style.opacity = elN2.disabled ? '0.45' : '1';
  elN3.style.opacity = elN3.disabled ? '0.45' : '1';
  elN4.style.opacity = elN4.disabled ? '0.45' : '1';
  elN5.style.opacity = elN5.disabled ? '0.45' : '1';
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

// Worker Setup
worker = new Worker(new URL('./worker.ts', import.meta.url), { type: 'module' });

let currentFrameBuffer: Float32Array | null = null;
let currentStats: FrameStats = { gen: 1, alive: 0, fps: 60 };
let fitnessHistory: FitnessHistoryUiEntry[] = []; // Track fitness over generations for charts
let godModeLog: GodModeLogEntry[] = []; // Track God Mode interactions
let selectedSnake: SelectedSnake | null = null; // Currently selected snake for God Mode
let isDragging = false;
let dragStartX = 0;
let dragStartY = 0;

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
    worker!.postMessage({ type: 'action', action: 'toggleView' });
    proxyWorld.viewMode = proxyWorld.viewMode === 'overview' ? 'follow' : 'overview';
  },
  resurrect: (genome: unknown) => {
    worker!.postMessage({ type: 'resurrect', genome });
  }
};
window.currentWorld = proxyWorld; // For HoF

function initWorker(resetCfg = true): void {
  const settings = readSettingsFromCoreUI();
  const updates = collectSettingsUpdates(settingsContainer);
  worker!.postMessage({ type: 'init', settings, updates, resetCfg, viewW: cssW, viewH: cssH });
}

// Init Worker
initWorker(true);
worker!.postMessage({ type: 'resize', viewW: cssW, viewH: cssH });

// Message Handler
worker.onmessage = (e: MessageEvent<WorkerToMainMessage>) => {
  // console.log("Received message from worker", e.data.type); // Spammy
  const msg = e.data;
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
      currentFrameBuffer = new Float32Array(msg.buffer);
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
      return;
    }
  }
};

/**
 * Live update handler for sliders that do not require a reset. Updates
 * the corresponding CFG value and allows world to respond immediately.
 * @param {HTMLInputElement} sliderEl
 */
function liveUpdateFromSlider(sliderEl: HTMLInputElement): void {
  setByPath(CFG, sliderEl.dataset.path!, Number(sliderEl.value));
  worker!.postMessage({ 
      type: 'updateSettings', 
      updates: [{
        path: sliderEl.dataset.path! as SettingsUpdate['path'],
        value: Number(sliderEl.value)
      }] 
  });
}

// Live update simulation speed when the slider moves
elSimSpeed.addEventListener('input', () => {
  refreshCoreUIState();
  worker!.postMessage({ type: 'action', action: 'simSpeed', value: parseFloat(elSimSpeed.value) });
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
  updateCFGFromUI(settingsContainer);
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
  refreshCoreUIState();
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
    camX = currentFrameBuffer[3];
    camY = currentFrameBuffer[4];
    zoom = currentFrameBuffer[5];
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
  const aliveCount = currentFrameBuffer[2];
  let closestSnake = null;
  let closestDist = maxDist;
  
  for (let i = 0; i < aliveCount; i++) {
    const id = currentFrameBuffer[ptr];
    const radius = currentFrameBuffer[ptr + 1];
    const skin = currentFrameBuffer[ptr + 2];
    const x = currentFrameBuffer[ptr + 3];
    const y = currentFrameBuffer[ptr + 4];
    const ptCount = currentFrameBuffer[ptr + 7];
    
    const dist = Math.hypot(x - worldX, y - worldY);
    if (dist < closestDist && dist < radius + maxDist) {
      closestDist = dist;
      closestSnake = { id, x, y, radius, skin };
    }
    
    ptr += 8 + ptCount * 2;
  }
  
  return closestSnake;
}

// ============== START ANIMATION LOOP ==============
requestAnimationFrame(frame);

// Click to select snake
canvas.addEventListener('click', (e) => {
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
  
  const rect = canvas.getBoundingClientRect();
  const screenX = e.clientX - rect.left;
  const screenY = e.clientY - rect.top;
  const world = screenToWorld(screenX, screenY);
  
  const snake = findSnakeNear(world.x, world.y);
  if (snake) {
    worker!.postMessage({ type: 'godMode', action: 'kill', snakeId: snake.id });
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
  if (e.button === 0 && selectedSnake) {
    isDragging = true;
    const rect = canvas.getBoundingClientRect();
    dragStartX = e.clientX - rect.left;
    dragStartY = e.clientY - rect.top;
  }
});

canvas.addEventListener('mousemove', (e) => {
  if (isDragging && selectedSnake) {
    const rect = canvas.getBoundingClientRect();
    const screenX = e.clientX - rect.left;
    const screenY = e.clientY - rect.top;
    const world = screenToWorld(screenX, screenY);
    
    worker!.postMessage({ 
      type: 'godMode', 
      action: 'move', 
      snakeId: selectedSnake.id,
      x: world.x,
      y: world.y
    });
  }
});

canvas.addEventListener('mouseup', (e) => {
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

// Persistence UI Wiring
const btnExport = document.getElementById('btnExport') as HTMLButtonElement | null;
if (btnExport) {
  btnExport.addEventListener('click', () => {
    if (pendingExport) return;
    pendingExport = true;
    worker!.postMessage({ type: 'export' });
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
    try {
      const data = await importFromFile(target.files[0]);
      if (!data || !Array.isArray(data.genomes)) {
        throw new Error('Invalid import file: missing genomes array.');
      }
      if (Array.isArray(data.hof)) {
        hof.replace(data.hof);
      }
      localStorage.setItem('slither_neuroevo_pop', JSON.stringify({ generation: data.generation, genomes: data.genomes }));
      worker!.postMessage({ type: 'import', data });
    } catch (err) {
      console.error("Import failed", err);
      const error = err as Error;
      alert("Failed to import file: " + error.message);
    } finally {
      if (target) target.value = '';
    }
  });
}

// Fixed simulation step to decouple physics from rendering
const FIXED_DT = 1 / 60;
function frame(t: number): void {
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

function updateInfoPanels(world: any): void {
  // 1. Stats Tab Info
  const alive = world.snakes.reduce((acc: number, s: any) => acc + (s.alive ? 1 : 0), 0);
  let maxPts = 0;
  for (const s of world.snakes) maxPts = Math.max(maxPts, s.pointsScore);
  if (maxPts <= 0) maxPts = 1;
  const logDenRender = Math.log(1 + maxPts);
  let bestNow = 0;
  for (const s of world.snakes) {
    if (!s.alive) continue;
    const pointsNorm = clamp(Math.log(1 + s.pointsScore) / logDenRender, 0, 1);
    bestNow = Math.max(bestNow, s.computeFitness(pointsNorm, 0));
  }

  const stepInfo = `substep ${clamp(CFG.collision.substepMaxDt, 0.004, 0.08).toFixed(3)}s max`;
  
  const statsInfoHtml = 
    `<div class="row"><strong>Simulation Status</strong></div>` +
    `<div class="row">Generation: ${world.generation}</div>` +
    `<div class="row">Time: ${world.generationTime.toFixed(1)} / ${CFG.generationSeconds}s</div>` +
    `<div class="row">Speed: ${world.simSpeed.toFixed(2)}x</div>` +
    `<div class="row">Alive: ${alive} / ${world.settings.snakeCount}</div>` +
    `<div class="row">Pellets: ${world.pellets.length}</div>` +
    `<div class="row">Best Fitness (Alive): ~${bestNow.toFixed(1)}</div>` +
    `<div class="row">Best Fitness (Ever): ${world.bestFitnessEver.toFixed(1)}</div>` +
    `<div class="row">Best Points (Gen): ${world.bestPointsThisGen.toFixed(1)}</div>` +
    `<div class="note" style="margin-top: 10px; font-size: 11px;">${stepInfo}</div>`;
  
  const statsEl = document.getElementById('statsInfo');
  if (statsEl && activeTab === 'tab-stats') {
    statsEl.innerHTML = statsInfoHtml;
    updateHoFTable(world);
  }

  // 2. Visualizer Tab Info
  const focus = world.focusSnake && world.focusSnake.alive ? world.focusSnake : null;
  let vizInfoHtml = '';
  
  if (focus) {
    vizInfoHtml += 
      `<div class="row"><strong>Focused Snake (ID ${focus.id})</strong></div>` +
      `<div class="row">Length: ${focus.length()}</div>` +
      `<div class="row">Radius: ${focus.radius.toFixed(1)}</div>` +
      `<div class="row">Points: ${focus.pointsScore.toFixed(1)}</div>` +
      `<div class="row">Food: ${focus.foodEaten.toFixed(0)}</div>` +
      `<div class="row">Kills: ${focus.killScore}</div>` +
      `<div class="row">Age: ${focus.age.toFixed(1)}s</div>` +
      `<div class="row">Boost: ${focus.boost ? 'ON' : 'off'}</div>`;
      
    if (focus.lastSensors && focus.lastOutputs) {
      const sens = focus.lastSensors.map((v: number) => v.toFixed(2)).join(', ');
      const outs = focus.lastOutputs.map((v: number) => v.toFixed(2)).join(', ');
      vizInfoHtml += 
        `<div class="row" style="margin-top:8px"><strong>Inputs</strong></div>` +
        `<div class="row" style="font-size:10px; word-break:break-all">[${sens}]</div>` +
        `<div class="row" style="margin-top:4px"><strong>Outputs</strong></div>` +
        `<div class="row" style="font-size:10px; word-break:break-all">[${outs}]</div>`;
    }
  } else {
    vizInfoHtml = `<div class="row">No snake focused. Click a snake or press V to auto-follow.</div>`;
  }
  
  const vizEl = document.getElementById('vizInfo');
  if (vizEl && activeTab === 'tab-viz') vizEl.innerHTML = vizInfoHtml;
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
