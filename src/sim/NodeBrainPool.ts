/**
 * Node.js Brain Pool Implementation.
 * 
 * Manages a pool of 'worker_threads' for parallel inference on the Server.
 * Extends BaseBrainPool for shared buffer management.
 */

import os from 'node:os';
import { Worker } from 'node:worker_threads';
import { BaseBrainPool, type BrainPoolInitOptions } from './BaseBrainPool.ts';
import type { InferenceBackend } from '../brains/types.ts';
import type {
    InferWorkerResponse,
    WorkerInitMessage,
    WorkerInferMessage
} from './poolProtocol.ts';


export class NodeBrainPool extends BaseBrainPool {
    private workers: Worker[] = [];

    /** Immutable backend selected for every worker brain in this pool. */
    readonly inferenceBackend: InferenceBackend;

    /**
     * Create a Node worker pool.
     * @param requestedWorkerCount - Requested worker count, or zero for auto.
     * @param inferenceBackend - Immutable math backend prepared inside each worker.
     */
    constructor(
        private requestedWorkerCount: number = 0,
        inferenceBackend: InferenceBackend = 'js'
    ) {
        super();
        this.inferenceBackend = inferenceBackend;
    }

    /**
     * Return the number of worker threads that completed pool initialization.
     * @returns Ready worker count, or zero while the pool is not ready.
     */
    getActiveWorkerCount(): number {
        return this.status === 'ready' ? this.workers.length : 0;
    }

    async init(options: BrainPoolInitOptions): Promise<void> {
        this.specKey = options.specKey;
        this.paramCount = options.paramCount;
        this.inputStride = options.inputStride;
        this.outputStride = options.outputStride;
        this.status = 'starting';

        // 1. Resolve worker count
        const cpuCount = os.cpus().length;
        const maxWorkers = Math.max(1, cpuCount - 1);
        const count = this.requestedWorkerCount > 0 ? this.requestedWorkerCount : maxWorkers;
        const workerCount = Math.min(count, maxWorkers);

        // 2. Allocate buffers
        // Sane dynamic capacity: max(popCount * 1.25, 256)
        const capacity = Math.max(Math.ceil(options.populationCount * 1.25), 256);
        const stateSize = options.stateSize || 0;
        const buffers = this.allocateBuffers(capacity, this.paramCount, this.inputStride, this.outputStride, stateSize);

        console.log(`[NodeBrainPool] Spawning ${workerCount} workers (capacity=${capacity})...`);

        // 3. Resolve Worker Script Path
        // We use a relative path from this module to the worker script.
        const workerUrl = new URL('../worker/inferWorker.ts', import.meta.url);

        const promises: Promise<void>[] = [];

        for (let i = 0; i < workerCount; i++) {
            const w = new Worker(workerUrl, {
                execArgv: ['--import', 'tsx/esm'],
                workerData: { inferenceBackend: this.inferenceBackend }
            });
            this.workers.push(w);

            promises.push(new Promise<void>((resolve, reject) => {
                const onMsg = (msg: InferWorkerResponse) => {
                    if (msg.type === 'ready') {
                        w.off('message', onMsg);
                        resolve();
                    } else if (msg.type === 'error') {
                        w.off('message', onMsg);
                        reject(new Error(msg.message));
                    }
                };
                w.on('message', onMsg);
                w.on('error', (err) => reject(err));
                w.on('exit', (code) => {
                    if (code !== 0) reject(new Error(`Worker exited with code ${code}`));
                });

                const initMsg: WorkerInitMessage = {
                    type: 'init',
                    specKey: this.specKey,
                    graphSpec: options.graphSpec || null,
                    inputStride: this.inputStride,
                    outputStride: this.outputStride,
                    workerIndex: i,
                    buffers: buffers
                };
                // Post the initialization message to the worker. 
                // We pass the current brain configuration and shared buffers.

                w.postMessage(initMsg);
            }));
        }

        try {
            await Promise.all(promises);
            this.status = 'ready';
            console.log('[NodeBrainPool] Ready.');
        } catch (err) {
            console.error('[NodeBrainPool] Init failed', err);
            this.status = 'failed';
            await this.shutdown();
            throw err;
        }
    }

    async shutdown(): Promise<void> {
        const terms = this.workers.map(w => w.terminate());
        await Promise.all(terms);
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

        // Copy inputs to shared buffer
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
            const batchStart = start;
            start += chunk;

            promises.push(new Promise<void>((resolve, reject) => {
                const onMsg = (msg: InferWorkerResponse) => {
                    if (msg.type === 'done' || msg.type === 'error') {
                        w.off('message', onMsg);
                        if (msg.type === 'error') reject(new Error(msg.message));
                        else resolve();
                    }
                };
                w.on('message', onMsg);
                w.postMessage({
                    type: 'infer',
                    batchStart,
                    batchCount: chunk
                } as WorkerInferMessage);
            }));
        }

        await Promise.all(promises);

        // Copy outputs back
        const outputF32 = new Float32Array(this.outputBuffer!);
        outputs.set(outputF32.subarray(0, count * outputStride));
    }
}
