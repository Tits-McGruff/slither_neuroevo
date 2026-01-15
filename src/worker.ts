/** Runs physics and neural simulation in a dedicated worker thread. */

import { World } from './world.ts';
import { CFG, resetCFGToDefaults, syncBrainInputSize } from './config.ts';
import { coerceSettingsUpdateValue, type SettingsUpdate } from './protocol/settings.ts';
import { WorldSerializer } from './serializer.ts';
import { setByPath } from './utils.ts';
import { validateGraph } from './brains/graph/validate.ts';
import type {
  FrameStats,
  MainToWorkerMessage,
  PopulationImportData,
  VizData,
  WorkerToMainMessage
} from './protocol/messages.ts';

/** Minimal worker scope typing for postMessage and onmessage. */
type WorkerScope = {
  postMessage: (message: WorkerToMainMessage, transfer?: Transferable[]) => void;
  onmessage: ((ev: MessageEvent<MainToWorkerMessage>) => void) | null;
};

/** Worker global scope wrapper with typed message helpers. */
const workerScope = self as unknown as WorkerScope;

/** Active world instance managed by the worker. */
let world: World | null = null;
/** Token used to cancel outdated loops. */
let loopToken = 0;
/** Current viewport width. */
let viewW = 0;
/** Current viewport height. */
let viewH = 0;
/** Whether brain visualization streaming is enabled. */
let vizEnabled = false;
/** Tick counter for throttling visualization payloads. */
let vizTick = 0;
/** Last sent fitness history length. */
let lastHistoryLen = 0;
/** Deferred import payload when init is not complete. */
let pendingImport: PopulationImportData | null = null;

/** Handle incoming messages from the main thread. */
workerScope.onmessage = function(e: MessageEvent<MainToWorkerMessage>) {
  const msg = e.data;
  switch (msg.type) {
    case 'init':
      if (msg.resetCfg !== false) resetCFGToDefaults();
      // Apply initial settings if any
      if (msg.updates) {
        msg.updates.forEach(u => {
          const coerced = coerceSettingsUpdateValue(u.path as SettingsUpdate['path'], u.value);
          setByPath(CFG, u.path, coerced);
        });
      }
      syncBrainInputSize();
      if ('stackOrder' in msg && Array.isArray(msg.stackOrder)) {
        CFG.brain.stackOrder = msg.stackOrder.slice();
      }
      if ('graphSpec' in msg) {
        if (msg.graphSpec) {
          const inputNodes = msg.graphSpec.nodes.filter(node => node.type === 'Input');
          const inputNode = inputNodes.length === 1 ? inputNodes[0] : null;
          if (!inputNode || inputNode.outputSize !== CFG.brain.inSize) {
            CFG.brain.graphSpec = null;
            console.warn('[Worker] Graph spec input size mismatch; ignoring.', {
              expected: CFG.brain.inSize,
              actual: inputNode?.outputSize ?? null
            });
          } else {
            const result = validateGraph(msg.graphSpec);
            if (result.ok) {
              CFG.brain.graphSpec = msg.graphSpec;
            } else {
              CFG.brain.graphSpec = null;
              console.warn('[Worker] Invalid graph spec ignored:', result.reason);
            }
          }
        } else {
          CFG.brain.graphSpec = null;
        }
      }
      world = new World(msg.settings || {});
      if (msg.viewW) viewW = msg.viewW;
      if (msg.viewH) viewH = msg.viewH;
      // We need to "load" the imported brains if persistence was used?
      // Handled via separate 'import' message or 'init' payload.
      if (msg.population) {
        const importPayload: PopulationImportData = {
          genomes: msg.population
        };
        if (msg.generation !== undefined) {
          importPayload.generation = msg.generation;
        }
        const result = world.importPopulation(importPayload);
        if (!result.ok) {
          console.warn('[Worker] Failed to import population during init:', result.reason);
        }
      }
      if (pendingImport) {
        const result = world.importPopulation(pendingImport);
        if (!result.ok) {
          console.warn('[Worker] Failed to apply pending import:', result.reason);
        }
        pendingImport = null;
      }
      lastTime = performance.now();
      accumulator = 0;
      loopToken += 1;
      loop(loopToken);
      break;

    case 'updateSettings':
      // msg.updates = [{path, value}, ...]
      if (msg.updates) {
        msg.updates.forEach(u => {
          const coerced = coerceSettingsUpdateValue(u.path as SettingsUpdate['path'], u.value);
          setByPath(CFG, u.path, coerced);
        });
        syncBrainInputSize();
      }
      break;

    case 'action':
      // User inputs (toggle view, click, etc - though clicks are UI side?)
      // We need to handle "God Mode" clicks here if we implement them.
      if (msg.action === 'toggleView') {
        if (world) world.toggleViewMode();
      } else if (msg.action === 'simSpeed') {
        if (world) world.applyLiveSimSpeed(msg.value);
      }
      break;

    case 'resize':
      viewW = msg.viewW;
      viewH = msg.viewH;
      break;

    case 'viz':
      vizEnabled = !!msg.enabled;
      break;
      
    case 'resurrect':
        if(world) world.resurrect(msg.genome);
        break;
        
    case 'import':
      if (!msg.data) break;
      if (!world) {
        pendingImport = msg.data;
        break;
      }
      {
        const result = world.importPopulation(msg.data);
      workerScope.postMessage({
          type: 'importResult',
          ok: result.ok,
          reason: result.reason || null,
          generation: world.generation,
          used: result.used || 0,
          total: result.total || 0
        });
      }
      break;

    case 'export':
      if (!world) break;
      workerScope.postMessage({ type: 'exportResult', data: world.exportPopulation() });
      break;
        
    case 'godMode':
      // God Mode interactions: kill, move, etc.
      if (!world) break;
      
      if (msg.action === 'kill') {
        // Find snake by ID and kill it
        const snake = world.snakes.find(s => s.id === msg.snakeId);
        if (snake && snake.alive) {
          snake.die(world);
          console.log(`[Worker] God Mode: Killed snake #${msg.snakeId}`);
        }
      } else if (msg.action === 'move') {
        // Move snake to specific position
        const snake = world.snakes.find(s => s.id === msg.snakeId);
        if (snake && snake.alive) {
          snake.x = msg.x;
          snake.y = msg.y;
          // Update head position
          const head = snake.points[0];
          if (head) {
            head.x = msg.x;
            head.y = msg.y;
          }
        }
      }
      break;
    default: {
      const _exhaustive: never = msg;
      void _exhaustive;
      break;
    }
  }
};

/** Last timestamp for the fixed-step loop. */
let lastTime = performance.now();
/** Fixed-step delta time for simulation updates. */
const FIXED_DT = 1 / 60;
/** Accumulator for fixed-step time integration. */
let accumulator = 0;

/**
 * Build stats payload for the worker frame loop.
 * @param world - Active world instance.
 * @param dt - Delta time in seconds since last frame.
 * @param historyLen - Last known fitness history length.
 * @param vizEnabledFlag - Whether viz streaming is enabled.
 * @param vizTickCounter - Current viz tick counter.
 * @returns Stats payload and updated counters.
 */
export function buildWorkerStats(
  world: World,
  dt: number,
  historyLen: number,
  vizEnabledFlag: boolean,
  vizTickCounter: number
): { stats: FrameStats; historyLen: number; vizTick: number } {
  const populationCount = world.population.length;
  const baselineBotsTotal = world.baselineBots.length;
  let alivePopulation = 0;
  let aliveTotal = 0;
  let baselineBotsAlive = 0;
  let maxFit = 0;
  let minFit = Infinity;
  let sumFit = 0;
  for (let i = 0; i < populationCount; i++) {
    const s = world.snakes[i];
    if (!s || !s.alive) continue;
    alivePopulation += 1;
    const fit = s.pointsScore || 0;
    maxFit = Math.max(maxFit, fit);
    minFit = Math.min(minFit, fit);
    sumFit += fit;
  }
  for (const s of world.snakes) {
    if (s.alive) aliveTotal += 1;
  }
  for (const bot of world.baselineBots) {
    if (bot && bot.alive) baselineBotsAlive += 1;
  }
  if (minFit === Infinity) minFit = 0;
  const avgFit = alivePopulation > 0 ? sumFit / alivePopulation : 0;
  const stats: FrameStats = {
    gen: world.generation,
    generationTime: world.generationTime,
    generationSeconds: CFG.generationSeconds,
    alive: alivePopulation,
    aliveTotal,
    baselineBotsAlive,
    baselineBotsTotal,
    fps: 1 / dt,
    fitnessData: {
      gen: world.generation,
      avgFitness: avgFit,
      maxFitness: maxFit,
      minFitness: minFit
    }
  };

  let nextHistoryLen = historyLen;
  if (world.fitnessHistory.length !== historyLen) {
    stats.fitnessHistory = world.fitnessHistory.slice();
    nextHistoryLen = world.fitnessHistory.length;
  }

  let nextVizTick = vizTickCounter;
  if (vizEnabledFlag && world.focusSnake && world.focusSnake.brain) {
    nextVizTick = (vizTickCounter + 1) % 6;
    if (nextVizTick === 0) {
      const viz = buildVizData(world.focusSnake.brain);
      if (viz) stats.viz = viz;
    }
  }

  if (world._lastHoFEntry) {
    stats.hofEntry = world._lastHoFEntry;
    world._lastHoFEntry = null;
  }

  return { stats, historyLen: nextHistoryLen, vizTick: nextVizTick };
}

/**
 * Run the fixed-step simulation loop and post frames to the main thread.
 * @param token - Loop token for cancellation.
 */
function loop(token: number): void {
  if (token !== loopToken) return;
  const now = performance.now();
  let dt = (now - lastTime) / 1000;
  lastTime = now;
  
  // Cap dt to avoid Spiral of Death
  if (dt > 0.2) dt = 0.2;
  
  if (world) {
      accumulator += dt;
      // Fixed time step update
      while (accumulator >= FIXED_DT) {
         world.update(FIXED_DT, viewW, viewH);
         accumulator -= FIXED_DT;
      }
      
      // Serialize and send
      const buffer = WorldSerializer.serialize(world);
      
      // Send stats. Keep payload small per frame; full history is sent only on growth.
      const statsResult = buildWorkerStats(world, dt, lastHistoryLen, vizEnabled, vizTick);
      const stats = statsResult.stats;
      lastHistoryLen = statsResult.historyLen;
      vizTick = statsResult.vizTick;
      
      // We transfer the buffer to avoid copy
      const transferBuffer =
        buffer.buffer instanceof ArrayBuffer ? buffer.buffer : buffer.slice().buffer;
      workerScope.postMessage(
        { type: 'frame', buffer: transferBuffer, stats },
        [transferBuffer]
      );
  }
  
  // Schedule next loop
  // internal loop in worker? setTimeout(0) or requestAnimationFrame?
  // Workers don't have rAF (usually). setTimeout(16) is best effort.
  // Actually, we want to run as fast as possible or sync to screen?
  // Ideally sync to screen, but worker is decoupled.
  // Let's target 60fps.
  setTimeout(() => loop(token), 16);
}

/**
 * Build visualization data from a brain instance if supported.
 * @param brain - Brain instance or null.
 * @returns Visualization payload or null.
 */
function buildVizData(brain: { getVizData?: () => VizData } | null | undefined): VizData | null {
  if (!brain || typeof brain.getVizData !== 'function') return null;
  return brain.getVizData();
}
