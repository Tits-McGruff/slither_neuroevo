// worker.js
// Runs the rigid-body physics and neural network simulation in a separate thread.

import { World } from './world.js';
import { CFG, resetCFGToDefaults } from './config.js';
import { WorldSerializer } from './serializer.js';
import { setByPath } from './utils.js';

let world = null;
let loopToken = 0;
let viewW = 0;
let viewH = 0;
let vizEnabled = false;
let vizTick = 0;
let lastHistoryLen = 0;

self.onmessage = function(e) {
  const msg = e.data;
  switch (msg.type) {
    case 'init':
      if (msg.resetCfg !== false) resetCFGToDefaults();
      // Apply initial settings if any
      if (msg.updates) {
        msg.updates.forEach(u => setByPath(CFG, u.path, u.value));
      }
      world = new World(msg.settings || {});
      if (msg.viewW) viewW = msg.viewW;
      if (msg.viewH) viewH = msg.viewH;
      // We need to "load" the imported brains if persistence was used?
      // Handled via separate 'import' message or 'init' payload.
      if (msg.population) {
        // TODO: Resurrect/Link population
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
          if (snake.points && snake.points.length > 0) {
            snake.points[0].x = msg.x;
            snake.points[0].y = msg.y;
          }
        }
      }
      break;
  }
};

let lastTime = performance.now();
const FIXED_DT = 1 / 60;
let accumulator = 0;

function loop(token) {
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
      const aliveSnakes = world.snakes.filter(s => s.alive);
      let maxFit = 0;
      let minFit = Infinity;
      let sumFit = 0;
      
      aliveSnakes.forEach(s => {
        // Calculate approximate fitness (we don't have exact formula here)
        const fit = s.pointsScore || 0;
        maxFit = Math.max(maxFit, fit);
        minFit = Math.min(minFit, fit);
        sumFit += fit;
      });
      
      const avgFit = aliveSnakes.length > 0 ? sumFit / aliveSnakes.length : 0;
      if (minFit === Infinity) minFit = 0;
      
      // Send stats
      const stats = {
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

      if (world.fitnessHistory.length !== lastHistoryLen) {
        stats.fitnessHistory = world.fitnessHistory.slice();
        lastHistoryLen = world.fitnessHistory.length;
      }

      if (vizEnabled && world.focusSnake && world.focusSnake.brain) {
        vizTick = (vizTick + 1) % 6;
        if (vizTick === 0) {
          stats.viz = buildVizData(world.focusSnake.brain);
        }
      }
      
      // We transfer the buffer to avoid copy
      self.postMessage({ type: 'frame', buffer: buffer.buffer, stats }, [buffer.buffer]);
  }
  
  // Schedule next loop
  // internal loop in worker? setTimeout(0) or requestAnimationFrame?
  // Workers don't have rAF (usually). setTimeout(16) is best effort.
  // Actually, we want to run as fast as possible or sync to screen?
  // Ideally sync to screen, but worker is decoupled.
  // Let's target 60fps.
  setTimeout(() => loop(token), 16);
}

function buildVizData(brain) {
  if (!brain) return null;
  if (brain.kind === 'mlp') {
    return {
      kind: 'mlp',
      mlp: {
        layerSizes: brain.mlp.layerSizes.slice(),
        _bufs: brain.mlp._bufs.map(buf => buf.slice())
      }
    };
  }
  return {
    kind: brain.kind,
    mlp: {
      layerSizes: brain.mlp.layerSizes.slice(),
      _bufs: brain.mlp._bufs.map(buf => buf.slice())
    },
    gru: brain.gru ? { hiddenSize: brain.gru.hiddenSize, h: brain.gru.h.slice() } : null,
    head: brain.head ? { outSize: brain.head.outSize } : null
  };
}
