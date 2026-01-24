
import { World as NativeWorld } from '../native/index.js';
import type { WorldSettings } from '../native/index.js';
import type { World } from '../src/world.ts';
import { CFG } from '../src/config.ts';

/**
 * Adapter that wraps the Native Rust simulation engine.
 */
export class NativeBackend {
    private native: NativeWorld;

    constructor(jsWorld: World) {
        // Convert JS config to Native settings
        const settings: WorldSettings = {
            worldRadius: CFG.worldRadius,
            snakeCount: jsWorld.settings.snakeCount,
            pelletCount: CFG.pelletCountTarget,
            tickRate: 60 // Default placeholder
        };

        this.native = new NativeWorld(settings);
        this.syncTo(jsWorld);
    }

    step(_dt: number): void {
        // _dt is currently unused by naive step in Rust (assumes fixed step)
        this.native.step();
    }

    syncTo(jsWorld: World): void {
        const snakes = this.native.getSnakes();

        // 1. Create a map of existing snakes for fast lookup
        const jsMap = new Map(jsWorld.snakes.map(s => [s.id, s]));

        // 2. Update existing snakes
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
                // Map points_score (Rust) to pointsScore (JS)
                s.pointsScore = nSnake.pointsScore;
            }
        }
    }
}
