/**
 * Web Worker Brain Pool Implementation.
 * 
 * Manages a pool of Web Workers for parallel inference in the Browser.
 * Extends BaseBrainPool for shared buffer management.
 */

import { BaseBrainPool, type BrainPoolInitOptions } from './BaseBrainPool.ts';
import type { GraphSpec } from '../brains/graph/schema.ts';

// Redefine message types for Web Worker protocol
// These match src/workerPool.ts definitions

interface InferWorkerInitMessage {
    type: 'init';
    specKey: string;
    graphSpec: GraphSpec | null;
    inputStride: number;
    outputStride: number;
    buffers: {
        inputs: SharedArrayBuffer;
        outputs: SharedArrayBuffer;
        weights: SharedArrayBuffer;
        indices: SharedArrayBuffer;
        sync?: SharedArrayBuffer; // Optional
    };
    workerIndex: number;
}

interface InferWorkerInferMessage {
    type: 'infer';
    batchStart: number;
    batchCount: number;
}

interface InferWorkerResponse {
    type: 'ready' | 'done' | 'error';
    error?: string;
    message?: string;
}

export class WebBrainPool extends BaseBrainPool {
    private workers: Worker[] = [];

    async init(options: BrainPoolInitOptions): Promise<void> {
        this.specKey = options.specKey;
        this.status = 'starting';

        // 1. Check Capabilities
        if (typeof crossOriginIsolated === 'undefined' || !crossOriginIsolated) {
            console.warn('[WebBrainPool] MT unavailable: missing crossOriginIsolated');
            this.status = 'disabled';
            throw new Error('Missing crossOriginIsolated');
        }
        if (typeof SharedArrayBuffer === 'undefined') {
            console.warn('[WebBrainPool] MT unavailable: missing SharedArrayBuffer');
            this.status = 'disabled';
            throw new Error('Missing SharedArrayBuffer');
        }

        // 2. Resolve concurrency
        const concurrency = navigator.hardwareConcurrency || 4;
        const workerCount = Math.max(1, concurrency - 1);

        // 3. Allocate Buffers
        const capacity = Math.max(options.populationCount || 20000, 20000);
        const buffers = this.allocateBuffers(capacity);

        console.log(`[WebBrainPool] Spawning ${workerCount} workers...`);

        // 4. Spawn Workers
        const workerUrl = new URL('../worker/inferWorker.ts', import.meta.url);
        const promises: Promise<void>[] = [];

        for (let i = 0; i < workerCount; i++) {
            const w = new Worker(workerUrl, { type: 'module' });
            this.workers.push(w);

            promises.push(new Promise<void>((resolve, reject) => {
                const onMsg = (e: MessageEvent<InferWorkerResponse>) => {
                    const data = e.data;
                    if (data.type === 'ready') {
                        w.removeEventListener('message', onMsg);
                        resolve();
                    } else if (data.type === 'error') {
                        w.removeEventListener('message', onMsg);
                        reject(new Error(data.message || 'Worker error'));
                    }
                };
                w.addEventListener('message', onMsg);

                const initMsg: InferWorkerInitMessage = {
                    type: 'init',
                    specKey: this.specKey,
                    graphSpec: options.graphSpec || null,
                    inputStride: 100, // FIXME: Read from CFG or options
                    outputStride: 0, // FIXME: Read from CFG or options
                    workerIndex: i,
                    buffers: buffers
                };
                // NOTE: We assume CFG is globally available or imported in BaseBrainPool.
                // In generic SimCore setup, we might need to pass strides explicitly.
                // For now, relying on BaseBrainPool logic (which imports default CFG).

                w.postMessage(initMsg);
            }));
        }

        try {
            await Promise.all(promises);
            this.status = 'ready';
            console.log('[WebBrainPool] Ready.');
        } catch (err) {
            console.error('[WebBrainPool] Init failed', err);
            this.status = 'failed';
            await this.shutdown();
        }
    }

    async shutdown(): Promise<void> {
        this.workers.forEach(w => w.terminate());
        this.workers = [];
        this.status = 'disabled';
        this.inputBuffer = null;
        this.outputBuffer = null;
        this.weightsBuffer = null;
        this.indicesBuffer = null;
    }

    async runBatch(
        inputs: Float32Array,
        outputs: Float32Array,
        indices: Uint32Array,
        count: number,
        inputStride: number,
        outputStride: number
    ): Promise<void> {
        if (this.status !== 'ready') throw new Error('Pool not ready');

        // Copy inputs
        const inputF32 = new Float32Array(this.inputBuffer!);
        inputF32.set(inputs.subarray(0, count * inputStride));

        // Copy indices
        const indicesU32 = new Uint32Array(this.indicesBuffer!);
        indicesU32.set(indices.subarray(0, count));

        // Dispatch
        const workerCount = this.workers.length;
        const itemsPerWorker = Math.ceil(count / workerCount);
        let start = 0;
        const promises: Promise<void>[] = [];

        for (let i = 0; i < workerCount; i++) {
            const chunk = Math.min(itemsPerWorker, count - start);
            if (chunk <= 0) break;

            const w = this.workers[i];
            if (!w) {
                start += chunk;
                continue;
            }
            const currentStart = start;
            start += chunk;

            promises.push(new Promise<void>((resolve, reject) => {
                const onMsg = (e: MessageEvent<InferWorkerResponse>) => {
                    const data = e.data;
                    if (data.type === 'done' || data.type === 'error') {
                        w.removeEventListener('message', onMsg);
                        if (data.type === 'error') {
                            reject(new Error(data.message || data.error));
                        } else {
                            resolve();
                        }
                    }
                };
                w.addEventListener('message', onMsg);

                const msg: InferWorkerInferMessage = {
                    type: 'infer',
                    batchStart: currentStart,
                    batchCount: chunk
                };
                w.postMessage(msg);
            }));
        }

        await Promise.all(promises);

        // Copy outputs back
        const outputF32 = new Float32Array(this.outputBuffer!);
        outputs.set(outputF32.subarray(0, count * outputStride));
    }
}
