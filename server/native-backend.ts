import { createRequire } from 'node:module';
import type { World } from '../src/world.ts';
import { CFG } from '../src/config.ts';

/** Settings payload expected by the native World constructor. */
interface NativeWorldSettings {
  worldRadius: number;
  snakeCount: number;
  pelletCount: number;
  tickRate: number;
  hiddenLayers: number;
  neurons1: number;
  neurons2: number;
  neurons3: number;
  neurons4: number;
  neurons5: number;
  useMlp: boolean;
  stackGru: number;
  stackLstm: number;
  stackRru: number;
  gruHidden: number;
  lstmHidden: number;
  rruHidden: number;
  pelletSpawnPerSecond: number;
  controlDt: number;
}

/** Constructor signature for the native World export. */
type NativeWorldCtor = new (settings: NativeWorldSettings) => {
  step(): void;
  getSnakes(): Array<{
    id: number;
    x: number;
    y: number;
    dir: number;
    alive: boolean;
    pointsScore: number;
  }>;
};

/**
 * Load the native World constructor from the N-API binding.
 * @returns Native world constructor.
 */
function loadNativeWorldCtor(): NativeWorldCtor {
  const require = createRequire(import.meta.url);
  const native = require('../native/index.js') as { World?: NativeWorldCtor };
  if (!native?.World) {
    throw new Error('Native World export missing. Native backend is unavailable.');
  }
  return native.World;
}

/**
 * Adapter that wraps the Native Rust simulation engine.
 */
export class NativeBackend {
  /** Native backend instance. */
  private native: {
    step(): void;
    getSnakes(): Array<{
      id: number;
      x: number;
      y: number;
      dir: number;
      alive: boolean;
      pointsScore: number;
    }>;
  };

  /**
   * Create a native backend adapter for an existing JS world.
   * @param jsWorld - JS world instance to synchronize with native state.
   */
  constructor(jsWorld: World) {
    const NativeWorld = loadNativeWorldCtor();
    // Convert JS config to Native settings.
    const settings: NativeWorldSettings = {
      worldRadius: CFG.worldRadius,
      snakeCount: jsWorld.settings.snakeCount,
      pelletCount: CFG.pelletCountTarget,
      tickRate: 60,
      hiddenLayers: jsWorld.settings.hiddenLayers,
      neurons1: jsWorld.settings.neurons1,
      neurons2: jsWorld.settings.neurons2,
      neurons3: jsWorld.settings.neurons3,
      neurons4: jsWorld.settings.neurons4,
      neurons5: jsWorld.settings.neurons5,
      useMlp: CFG.brain.useMlp !== false,
      stackGru: CFG.brain.stack?.gru ?? 0,
      stackLstm: CFG.brain.stack?.lstm ?? 0,
      stackRru: CFG.brain.stack?.rru ?? 0,
      gruHidden: CFG.brain.gruHidden ?? 16,
      lstmHidden: CFG.brain.lstmHidden ?? 16,
      rruHidden: CFG.brain.rruHidden ?? 16,
      pelletSpawnPerSecond: CFG.pelletSpawnPerSecond ?? 0,
      controlDt: CFG.brain.controlDt ?? (1 / 60)
    };

    this.native = new NativeWorld(settings);
    this.syncTo(jsWorld);
  }

  /**
   * Advance the native backend by a single simulation step.
   * @param _dt - Unused placeholder for future variable-step support.
   */
  step(_dt: number): void {
    // _dt is currently unused by naive step in Rust (assumes fixed step).
    this.native.step();
  }

  /**
   * Synchronize JS snake state with the native backend.
   * @param jsWorld - JS world to update with native snake state.
   */
  syncTo(jsWorld: World): void {
    const snakes = this.native.getSnakes();

    // 1. Create a map of existing snakes for fast lookup.
    const jsMap = new Map(jsWorld.snakes.map((s) => [s.id, s]));

    // 2. Update existing snakes.
    // Note: We currently don't handle spawning new snakes here extensively
    // because the Native engine relies on external spawning or pre-seeding.
    // Ideally we'd sync spawning too, but for parity check, we assume 1:1.

    for (const nSnake of snakes) {
      const s = jsMap.get(nSnake.id);
      if (s) {
        s.x = nSnake.x;
        s.y = nSnake.y;
        s.dir = nSnake.dir;
        s.alive = nSnake.alive;
        // Map points_score (Rust) to pointsScore (JS).
        s.pointsScore = nSnake.pointsScore;
      }
    }
  }
}
