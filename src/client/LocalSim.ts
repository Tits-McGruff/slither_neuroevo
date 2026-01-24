/**
 * Local Simulation Orchestrator.
 * 
 * Provides a high-level API for running the simulation locally on the client.
 * Wraps SimCore and integrates WebBrainPool for parallel inference.
 */

import { SimCore, type SimCoreOptions, type CoreStats } from '../sim/SimCore.ts';
import { WebBrainPool } from '../sim/WebBrainPool.ts';
import { loadSimdKernels } from '../brains/wasmBridge.ts';
import type { PopulationImportData } from '../protocol/messages.ts';

export interface LocalSimOptions extends SimCoreOptions {
    /** Whether to enable Multi-Threading (Web Workers). */
    mtEnabled?: boolean;
}

export class LocalSim {
    public core: SimCore;
    private pool: WebBrainPool | null = null;
    private loopToken: number = 0;

    constructor(options: LocalSimOptions = {}) {
        this.core = new SimCore(options);

        if (options.mtEnabled) {
            this.pool = new WebBrainPool();
        }
    }

    /**
     * Initialize the simulation and dependencies (WASM, MT pool).
     */
    async init(): Promise<void> {
        // 1. Load SIMD
        try {
            await loadSimdKernels();
        } catch (err) {
            console.warn('[LocalSim] SIMD load failed, using JS fallback:', err);
        }

        // 2. Init MT Pool if enabled
        if (this.pool) {
            try {
                await this.pool.init({
                    specKey: this.core.world.archKey,
                    graphSpec: this.core.world.arch.spec,
                    populationCount: this.core.world.population.length
                });
                this.core.brainPool = this.pool;
                this.pool.syncWeights(this.core.world.population);
            } catch (err) {
                console.error('[LocalSim] Pool initialization failed:', err);
                this.pool = null;
                this.core.brainPool = null;
            }
        }
    }

    /**
     * Execute a simulation step.
     * @param dt - Real-world elapsed time.
     */
    async update(dt: number): Promise<void> {
        // Sync weights if generation changed
        if (this.pool && this.core.world.generation !== this.core.lastGeneration) {
            this.pool.syncWeights(this.core.world.population);
        }

        await this.core.update(dt);
    }

    /**
     * Helper to import population and sync pool.
     * @param data - Population data to import.
     */
    importPopulation(data: PopulationImportData) {
        const result = this.core.world.importPopulation(data);
        if (result.ok && this.pool) {
            this.pool.syncWeights(this.core.world.population);
        }
        return result;
    }

    /**
     * Build statistics for the current state.
     * @param includeViz - Whether to include visualization data.
     */
    getStats(includeViz: boolean = false): CoreStats {
        return this.core.buildStats(includeViz);
    }

    /**
     * Serialize the current frame.
     */
    serialize(): Float32Array {
        return this.core.serialize();
    }

    async shutdown(): Promise<void> {
        if (this.pool) {
            await this.pool.shutdown();
        }
        this.loopToken++;
    }
}
