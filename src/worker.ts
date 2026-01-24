/** Runs physics and neural simulation in a dedicated worker thread. */

import { LocalSim } from './client/LocalSim.ts';
import { CFG, resetCFGToDefaults, syncBrainInputSize } from './config.ts';
import { coerceSettingsUpdateValue, type SettingsUpdate } from './protocol/settings.ts';
import { setByPath } from './utils.ts';
import { validateGraph } from './brains/graph/validate.ts';
import type {
  MainToWorkerMessage,
  PopulationImportData,
  WorkerToMainMessage
} from './protocol/messages.ts';

/** Minimal worker scope typing for postMessage and onmessage. */
type WorkerScope = {
  postMessage: (message: WorkerToMainMessage, transfer?: Transferable[]) => void;
  onmessage: ((ev: MessageEvent<MainToWorkerMessage>) => void) | null;
};

/** Worker global scope wrapper with typed message helpers. */
const workerScope = self as unknown as WorkerScope;

/** Active local simulation instance. */
let sim: LocalSim | null = null;
/** Token used to cancel outdated loops. */
let loopToken = 0;
/** Whether brain visualization streaming is enabled. */
let vizEnabled = false;
/** Tick counter for throttling visualization payloads. */
let vizTick = 0;
/** Last timestamp for the real-world delta calculation. */
let lastTime = performance.now();

/** Handle incoming messages from the main thread. */
workerScope.onmessage = async function (e: MessageEvent<MainToWorkerMessage>) {
  const msg = e.data;
  switch (msg.type) {
    case 'init': {
      if (msg.resetCfg !== false) resetCFGToDefaults();
      // Apply initial settings
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

      // Handle graph spec override in CFG
      if ('graphSpec' in msg) {
        if (msg.graphSpec) {
          const result = validateGraph(msg.graphSpec);
          if (result.ok) CFG.brain.graphSpec = msg.graphSpec;
        } else {
          CFG.brain.graphSpec = null;
        }
      }

      // Create and initialize LocalSim
      sim = new LocalSim({
        settings: msg.settings || {},
        mtEnabled: true // Enable MT workers for inference
      });

      if (msg.viewW) sim.core.viewW = msg.viewW;
      if (msg.viewH) sim.core.viewH = msg.viewH;

      await sim.init();

      if (msg.population) {
        const importData: PopulationImportData = { genomes: msg.population };
        if (msg.generation !== undefined) importData.generation = msg.generation;
        sim.importPopulation(importData);
      }

      lastTime = performance.now();
      loopToken += 1;
      void loop(loopToken);
      break;
    }

    case 'updateSettings':
      if (msg.updates) {
        msg.updates.forEach(u => {
          const coerced = coerceSettingsUpdateValue(u.path as SettingsUpdate['path'], u.value);
          setByPath(CFG, u.path, coerced);
        });
        syncBrainInputSize();
      }
      break;

    case 'action':
      if (sim) {
        if (msg.action === 'toggleView') sim.core.world.toggleViewMode();
        else if (msg.action === 'simSpeed') sim.core.world.applyLiveSimSpeed(msg.value);
      }
      break;

    case 'resize':
      if (sim) {
        sim.core.viewW = msg.viewW;
        sim.core.viewH = msg.viewH;
      }
      break;

    case 'viz':
      vizEnabled = !!msg.enabled;
      break;

    case 'resurrect':
      if (sim) sim.core.world.resurrect(msg.genome);
      break;

    case 'import':
      if (msg.data && sim) {
        const result = sim.importPopulation(msg.data);
        workerScope.postMessage({
          type: 'importResult',
          ok: result.ok,
          reason: result.reason || null,
          generation: sim.core.world.generation,
          used: result.used || 0,
          total: result.total || 0
        });
      }
      break;

    case 'export':
      if (sim) {
        workerScope.postMessage({ type: 'exportResult', data: sim.core.world.exportPopulation() });
      }
      break;

    case 'godMode':
      if (!sim) break;
      if (msg.action === 'kill') {
        const snake = sim.core.world.snakes.find(s => s.id === msg.snakeId);
        if (snake && snake.alive) snake.die(sim.core.world);
      } else if (msg.action === 'move') {
        const snake = sim.core.world.snakes.find(s => s.id === msg.snakeId);
        if (snake && snake.alive) {
          snake.x = msg.x;
          snake.y = msg.y;
          const head = snake.points[0];
          if (head) { head.x = msg.x; head.y = msg.y; }
        }
      }
      break;
  }
};

/** High-frequency simulation loop. */
async function loop(token: number): Promise<void> {
  if (token !== loopToken || !sim) return;

  const now = performance.now();
  const dt = (now - lastTime) / 1000;
  lastTime = now;

  // 1. Advance Simulation (handles fixed-step internally)
  await sim.update(dt);

  // 2. Transmit Frame
  const buffer = sim.serialize();

  // Throttle Viz
  let sendViz = false;
  if (vizEnabled) {
    vizTick = (vizTick + 1) % 6;
    if (vizTick === 0) sendViz = true;
  }

  const stats = sim.getStats(sendViz);

  const transferBuffer =
    buffer.buffer instanceof ArrayBuffer ? buffer.buffer : buffer.slice().buffer;

  workerScope.postMessage(
    { type: 'frame', buffer: transferBuffer, stats } as WorkerToMainMessage,
    [transferBuffer]
  );

  // 3. Schedule next step
  setTimeout(() => void loop(token), 10);
}
