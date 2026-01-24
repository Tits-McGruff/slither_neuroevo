/**
 * Base Brain Pool Implementation.
 * 
 * Abstract base class that manages SharedArrayBuffers and common state 
 * for both Browser (Web Worker) and Server (Node Worker) implementations.
 */

import type { BatchInferenceRunner } from '../world.ts';
import type { GraphSpec } from '../brains/graph/schema.ts';
import type { Genome } from '../mlp.ts';

export type PoolStatus = 'disabled' | 'starting' | 'ready' | 'failed';

export interface BrainPoolInitOptions {
    specKey: string;
    graphSpec?: GraphSpec | null;
    populationCount: number;
    paramCount: number;
    inputStride: number;
    outputStride: number;
    stateSize?: number;
}

/**
 * Common interface for Brain Pools.
 */
export interface IBrainPool extends BatchInferenceRunner {
    status: PoolStatus;
    specKey: string;
    paramCount: number;
    inputStride: number;
    outputStride: number;
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
    states?: SharedArrayBuffer;
}

export abstract class BaseBrainPool implements IBrainPool {
    status: PoolStatus = 'disabled';

    // Buffers
    protected inputBuffer: SharedArrayBuffer | null = null;
    protected outputBuffer: SharedArrayBuffer | null = null;
    protected weightsBuffer: SharedArrayBuffer | null = null;
    protected indicesBuffer: SharedArrayBuffer | null = null;
    protected statesBuffer: SharedArrayBuffer | null = null;

    protected currentCapacity: number = 0;
    public specKey: string = '';
    public paramCount: number = 0;
    public inputStride: number = 0;
    public outputStride: number = 0;
    public totalStateSize: number = 0;

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
     * Allocate or re-allocate shared buffers based on exact capacity and parameter requirements.
     */
    protected allocateBuffers(capacity: number, paramCount: number, inStride: number, outStride: number, stateSize: number = 0): PoolBuffers {
        const inSize = inStride;
        const outSize = outStride;

        const inBytes = capacity * inSize * 4;
        const outBytes = capacity * outSize * 4;
        const indexBytes = capacity * 4; // Uint32
        const weightBytes = capacity * paramCount * 4;

        this.inputBuffer = new SharedArrayBuffer(inBytes);
        this.outputBuffer = new SharedArrayBuffer(outBytes);
        this.weightsBuffer = new SharedArrayBuffer(weightBytes);
        this.indicesBuffer = new SharedArrayBuffer(indexBytes);

        if (stateSize > 0) {
            this.statesBuffer = new SharedArrayBuffer(capacity * stateSize * 4);
        } else {
            this.statesBuffer = null;
        }

        this.currentCapacity = capacity;
        this.totalStateSize = stateSize;

        const buffers: PoolBuffers = {
            inputs: this.inputBuffer,
            outputs: this.outputBuffer,
            weights: this.weightsBuffer,
            indices: this.indicesBuffer
        };

        if (this.statesBuffer) buffers.states = this.statesBuffer;

        return buffers;
    }

    /**
     * Sync population weights to the shared buffer using fixed-stride packing.
     */
    syncWeights(population: Genome[]): void {
        if (!this.weightsBuffer || this.status !== 'ready') return;

        const f32 = new Float32Array(this.weightsBuffer);
        const stride = this.paramCount;

        for (let i = 0; i < population.length; i++) {
            const g = population[i];
            if (!g || !g.weights) continue;

            const base = i * stride;
            if (base + stride > f32.length) {
                console.warn('[BaseBrainPool] Capacity exceeded during weight sync', { index: i, capacity: this.currentCapacity });
                break;
            }

            if (g.weights.length !== stride) {
                console.error(`[BaseBrainPool] Weight length mismatch for snake ${i}: expected ${stride}, got ${g.weights.length}.`);
                // For safety, we fill with 0 and copy subset to avoid corrupting neighboring strides
                f32.fill(0, base, base + stride);
                const copyLen = Math.min(stride, g.weights.length);
                f32.set(g.weights.subarray(0, copyLen), base);
                continue;
            }

            f32.set(g.weights, base);
        }
    }
}
