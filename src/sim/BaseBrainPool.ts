/**
 * Base Brain Pool Implementation.
 * 
 * Abstract base class that manages SharedArrayBuffers and common state 
 * for both Browser (Web Worker) and Server (Node Worker) implementations.
 */

import { CFG } from '../config.ts';
import type { BatchInferenceRunner } from '../world.ts';
import type { GraphSpec } from '../brains/graph/schema.ts';
import type { Genome } from '../mlp.ts';

export type PoolStatus = 'disabled' | 'starting' | 'ready' | 'failed';

export interface BrainPoolInitOptions {
    specKey: string;
    graphSpec?: GraphSpec | null;
    populationCount?: number; // Hint for pre-allocation
}

/**
 * Common interface for Brain Pools.
 */
export interface IBrainPool extends BatchInferenceRunner {
    status: PoolStatus;
    init(options: BrainPoolInitOptions): Promise<void>;
    shutdown(): Promise<void>;
    syncWeights(population: Genome[]): void;
}

/**
 * Buffer Layout for Shared Memory.
 */
export interface PoolBuffers {
    inputs: SharedArrayBuffer;
    outputs: SharedArrayBuffer;
    weights: SharedArrayBuffer;
    indices: SharedArrayBuffer;
}

export abstract class BaseBrainPool implements IBrainPool {
    status: PoolStatus = 'disabled';

    // Buffers
    protected inputBuffer: SharedArrayBuffer | null = null;
    protected outputBuffer: SharedArrayBuffer | null = null;
    protected weightsBuffer: SharedArrayBuffer | null = null;
    protected indicesBuffer: SharedArrayBuffer | null = null;

    protected currentCapacity: number = 0;
    protected specKey: string = '';

    // Abstract methods to be implemented by platform subclasses
    abstract init(options: BrainPoolInitOptions): Promise<void>;
    abstract shutdown(): Promise<void>;
    abstract runBatch(inputs: Float32Array, outputs: Float32Array, indices: Uint32Array, count: number, inputStride: number, outputStride: number): Promise<void>;

    /**
     * Check if SharedArrayBuffer is supported in the current environment.
     */
    static isSupported(): boolean {
        // Basic check, subclasses might refine (e.g. valid headers check in browser)
        return typeof SharedArrayBuffer !== 'undefined';
    }

    /**
     * Allocate or re-allocate shared buffers based on capacity.
     */
    protected allocateBuffers(capacity: number, maxParams: number = 5000): PoolBuffers {
        const inSize = CFG.brain.inSize;
        const outSize = CFG.brain.outSize;

        const inBytes = capacity * inSize * 4;
        const outBytes = capacity * outSize * 4;
        const indexBytes = capacity * 4; // Uint32
        const weightBytes = capacity * maxParams * 4;

        this.inputBuffer = new SharedArrayBuffer(inBytes);
        this.outputBuffer = new SharedArrayBuffer(outBytes);
        this.weightsBuffer = new SharedArrayBuffer(weightBytes);
        this.indicesBuffer = new SharedArrayBuffer(indexBytes);

        this.currentCapacity = capacity;

        return {
            inputs: this.inputBuffer,
            outputs: this.outputBuffer,
            weights: this.weightsBuffer,
            indices: this.indicesBuffer
        };
    }

    /**
     * Sync population weights to the shared buffer.
     */
    syncWeights(population: Genome[]): void {
        if (!this.weightsBuffer || this.status !== 'ready') return;

        const f32 = new Float32Array(this.weightsBuffer);
        let offset = 0;

        // We assume linear packing. Subclasses/Workers must match this layout.
        for (const g of population) {
            if (g && g.weights) {
                // Safety check for buffer overflow?
                if (offset + g.weights.length > f32.length) {
                    console.warn('[BaseBrainPool] Weight buffer overflow', { offset, len: f32.length });
                    break;
                }
                f32.set(g.weights, offset);
                offset += g.weights.length;
            }
        }
    }
}
