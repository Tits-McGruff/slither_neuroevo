// worker.ts
// Runs the rigid-body physics and neural network simulation in a separate thread.

import { World } from './world.ts';
import { CFG, resetCFGToDefaults } from './config.ts';
import { WorldSerializer } from './serializer.ts';
import { setByPath } from './utils.ts';
import { validateGraph } from './brains/graph/validate.ts';
import type {
  FrameStats,
  MainToWorkerMessage,
  PopulationImportData,
  VizData
} from './protocol/messages.ts';

type WorkerScope = {
  postMessage: (message: any, transfer?: Transferable[]) => void;
  onmessage: ((ev: MessageEvent<MainToWorkerMessage>) => void) | null;
};

const workerScope = self as unknown as WorkerScope;

let world: any = null;
let loopToken = 0;
let viewW = 0;
let viewH = 0;
let vizEnabled = false;
let vizTick = 0;
let lastHistoryLen = 0;
let pendingImport: PopulationImportData | null = null;

workerScope.onmessage = function(e: MessageEvent<MainToWorkerMessage>) {
  const msg = e.data;
  switch (msg.type) {
    case 'init':
      if (msg.resetCfg !== false) resetCFGToDefaults();
      // Apply initial settings if any
      if (msg.updates) {
        msg.updates.forEach(u => setByPath(CFG, u.path, u.value));
      }
      if ('stackOrder' in msg && Array.isArray(msg.stackOrder)) {
        CFG.brain.stackOrder = msg.stackOrder.slice();
      }
      if ('graphSpec' in msg) {
        if (msg.graphSpec) {
          const result = validateGraph(msg.graphSpec);
          if (result.ok) {
            CFG.brain.graphSpec = msg.graphSpec;
          } else {
            CFG.brain.graphSpec = null;
            console.warn('[Worker] Invalid graph spec ignored:', result.reason);
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
        const result = world.importPopulation({
          generation: msg.generation,
          genomes: msg.population
        });
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
        msg.updates.forEach(u => setByPath(CFG, u.path, u.value));
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
        const snake = world.snakes.find((s: any) => s.id === msg.snakeId);
        if (snake && snake.alive) {
          snake.die(world);
          console.log(`[Worker] God Mode: Killed snake #${msg.snakeId}`);
        }
      } else if (msg.action === 'move') {
        // Move snake to specific position
        const snake = world.snakes.find((s: any) => s.id === msg.snakeId);
        if (snake && snake.alive) {
          snake.x = msg.x;
          snake.y = msg.y;
          // Update head position
          if (snake.points && snake.points.length > 0) {
            snake.points[0].x = msg.x;
            snake.points[0].y = msg.y;
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

let lastTime = performance.now();
const FIXED_DT = 1 / 60;
let accumulator = 0;

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
      
      // Calculate fitness stats for this generation
      const aliveSnakes = world.snakes.filter((s: any) => s.alive);
      let maxFit = 0;
      let minFit = Infinity;
      let sumFit = 0;
      
      aliveSnakes.forEach((s: any) => {
        // Calculate approximate fitness (we don't have exact formula here)
        const fit = s.pointsScore || 0;
        maxFit = Math.max(maxFit, fit);
        minFit = Math.min(minFit, fit);
        sumFit += fit;
      });
      
      const avgFit = aliveSnakes.length > 0 ? sumFit / aliveSnakes.length : 0;
      if (minFit === Infinity) minFit = 0;
      
      // Send stats. Keep payload small per frame; full history is sent only on growth.
      const stats: FrameStats = {
          gen: world.generation,
          alive: aliveSnakes.length,
          fps: 1/dt, // Approx
          fitnessData: {
            gen: world.generation,
            avgFitness: avgFit,
            maxFitness: maxFit,
            minFitness: minFit
          }
      };

      // Ship full fitness history only when it grows; UI keeps a rolling buffer.
      if (world.fitnessHistory.length !== lastHistoryLen) {
        stats.fitnessHistory = world.fitnessHistory.slice();
        lastHistoryLen = world.fitnessHistory.length;
      }

      if (vizEnabled && world.focusSnake && world.focusSnake.brain) {
        vizTick = (vizTick + 1) % 6;
        if (vizTick === 0) {
          const viz = buildVizData(world.focusSnake.brain);
          if (viz) stats.viz = viz;
        }
      }

      if (world._lastHoFEntry) {
        stats.hofEntry = world._lastHoFEntry;
        world._lastHoFEntry = null;
      }
      
      // We transfer the buffer to avoid copy
      workerScope.postMessage({ type: 'frame', buffer: buffer.buffer, stats }, [buffer.buffer]);
  }
  
  // Schedule next loop
  // internal loop in worker? setTimeout(0) or requestAnimationFrame?
  // Workers don't have rAF (usually). setTimeout(16) is best effort.
  // Actually, we want to run as fast as possible or sync to screen?
  // Ideally sync to screen, but worker is decoupled.
  // Let's target 60fps.
  setTimeout(() => loop(token), 16);
}

function buildVizData(brain: any): VizData | null {
  if (!brain || typeof brain.getVizData !== 'function') return null;
  return brain.getVizData();
}
