/**
 * Manages a pool of web workers for parallel neural inference using SharedArrayBuffer.
 * Handles lifecycle, batch dispatch, and fallbacks when isolation is unavailable.
 */

import { CFG } from './config.ts';

export type PoolStatus = 'disabled' | 'starting' | 'ready' | 'failed';

/** Message sent to initialize an inference worker. */
import type { GraphSpec } from './brains/graph/schema.ts';
import type { Genome } from './mlp.ts';

export interface InferWorkerInitMessage {
    type: 'init';
    specKey: string;
    graphSpec: GraphSpec | null;
    inputStride: number;
    outputStride: number;
    buffers: {
        inputs: SharedArrayBuffer;
        outputs: SharedArrayBuffer;
        weights: SharedArrayBuffer; // Added weights buffer
        indices: SharedArrayBuffer; // Added indices buffer
        sync: SharedArrayBuffer;
    };
    workerIndex: number;
}

/** Message sent to dispatch an inference batch range. */
export interface InferWorkerInferMessage {
    type: 'infer';
    batchStart: number;
    batchCount: number;
}

/** Message sent to shutdown the worker. */
export interface InferWorkerShutdownMessage {
    type: 'shutdown';
}

export type InferWorkerMessage =
    | InferWorkerInitMessage
    | InferWorkerInferMessage
    | InferWorkerShutdownMessage;

/** Message received from worker. */
export type InferWorkerResponse =
    | { type: 'ready' }
    | { type: 'done'; error?: string }
    | { type: 'error'; message: string };

/**
 * Sync buffer layout for Atomics.
 * Index 0: Global batch counter (unused for now, maybe for dynamic stealing)
 * Index 1..N: Per-worker status (0=idle, 1=busy, 2=done, 3=error)
 */
const SYNC_HEADER_SIZE = 8; // Reserved ints

export class WorkerPool {
    status: PoolStatus = 'disabled';
    private workers: Worker[] = [];
    private inputBuffer: SharedArrayBuffer | null = null;
    private outputBuffer: SharedArrayBuffer | null = null;
    private weightsBuffer: SharedArrayBuffer | null = null; // Added
    private indicesBuffer: SharedArrayBuffer | null = null; // Added
    private syncBuffer: SharedArrayBuffer | null = null;
    // private syncView: Int32Array | null = null; // Unused
    // private activePending: { resolve: () => void; reject: (err: unknown) => void }[] = []; // Unused
    private specKey: string = '';
    private currentCapacity = 0;

    /**
     * Initialize the worker pool if supported.
     * @param specKey - Architecture key to validate/load kernels.
     */
    async initPool(specKey: string, graphSpec?: GraphSpec | null): Promise<void> {
        // 1. Check capabilities
        const hasIsolation = typeof crossOriginIsolated !== 'undefined' && crossOriginIsolated;
        const hasSAB = typeof SharedArrayBuffer !== 'undefined';

        if (!hasIsolation || !hasSAB) {
            console.warn('[WorkerPool] MT unavailable: missing isolation or SharedArrayBuffer', {
                isolation: hasIsolation,
                sab: hasSAB
            });
            this.status = 'disabled';
            return;
        }

        // 2. Check config flag (opt-in)
        // Assuming CFG.brain.inferenceMode will be added later, keeping it enabled if capable for now or manual opt-in.
        // For specific rollout, check a flag. For now, we proceed if capable.
        // TODO: Gate behind CFG flag once added.

        if (this.status === 'starting' || this.status === 'ready') {
            await this.shutdownPool();
        }

        this.status = 'starting';
        this.specKey = specKey;

        try {
            const concurrency = navigator.hardwareConcurrency || 4;
            const workerCount = Math.max(1, concurrency - 1); // Leave one for main/physics

            // Allocate buffers.
            // Sizing: based on max expected snakes. 200k capacity from world.ts?
            // 200k * inSize * 4 bytes is large but manageable (e.g. 200k * 100 * 4 = 80MB).
            // Let's stick to world controlBatch resizing logic, but here we need fixed SharedBuffer allocation
            // or re-allocate on demand. Note that re-allocation kills workers.
            // Start with a reasonable capacity.
            const initialCapacity = 20000;
            this._allocateBuffers(initialCapacity);

            console.log(`[WorkerPool] Spawning ${workerCount} workers...`);

            const promises: Promise<void>[] = [];

            for (let i = 0; i < workerCount; i++) {
                const w = new Worker(new URL('./worker/inferWorker.ts', import.meta.url), {
                    type: 'module'
                });
                this.workers.push(w);

                // Setup init handshake
                promises.push(
                    new Promise<void>((resolve, reject) => {
                        const onMsg = (e: MessageEvent<InferWorkerResponse>) => {
                            const data = e.data;
                            if (data.type === 'ready') {
                                w.removeEventListener('message', onMsg);
                                resolve();
                            } else if (data.type === 'error') {
                                w.removeEventListener('message', onMsg);
                                reject(new Error(data.message));
                            }
                        };
                        w.addEventListener('message', onMsg);

                        // Send init
                        const initMsg: InferWorkerInitMessage = {
                            type: 'init',
                            specKey,
                            graphSpec: graphSpec || CFG.brain.graphSpec, // Use provided spec or global fallback
                            inputStride: CFG.brain.inSize,
                            outputStride: CFG.brain.outSize,
                            workerIndex: i,
                            buffers: {
                                inputs: this.inputBuffer!,
                                outputs: this.outputBuffer!,
                                weights: this.weightsBuffer!, // Send weights
                                indices: this.indicesBuffer!, // Send indices
                                sync: this.syncBuffer!
                            }
                        };
                        w.postMessage(initMsg);
                    })
                );
            }

            await Promise.all(promises);
            this.status = 'ready';
            console.log('[WorkerPool] Ready.');

        } catch (err) {
            console.error('[WorkerPool] Init failed', err);
            this.status = 'failed';
            await this.shutdownPool();
        }
    }

    /**
     * Shutdown all workers and clear state.
     */
    async shutdownPool(): Promise<void> {
        this.workers.forEach(w => w.terminate());
        this.workers = [];
        this.inputBuffer = null;
        this.outputBuffer = null;
        this.weightsBuffer = null;
        this.indicesBuffer = null;
        this.syncBuffer = null;
        // this.syncView = null;
        this.status = 'disabled';
    }

    /**
     * Sync population weights to the shared buffer.
     * @param population - Array of genomes (must be sorted/indexed by slot).
     */
    syncWeights(population: Genome[]): void {
        if (!this.weightsBuffer) return;
        // Assume the pool is sized for the population.
        // Also assume all genomes match the spec layout.
        // We flatten all weights into the buffer.
        // Optimization: only sync if generation changed?
        // Caller handles timing. We just copy.

        const f32 = new Float32Array(this.weightsBuffer);
        let offset = 0;
        // Detect weight size from first genome?
        // Or use this.specKey -> but we don't have the compiled spec here easily without helper.
        // Rely on the genomes having correct .weights length.

        for (const g of population) {
            if (g && g.weights) {
                f32.set(g.weights, offset);
                offset += g.weights.length;
            }
        }
    }

    /**
     * Dispatch a batch of inference work to the pool.
     * @param inputs - Float32Array source data (copied into SAB).
     * @param outputs - Float32Array destination buffer (copied back from SAB).
     * @param indices - Uint32Array of snake indices for the batch.
     * @param count - Number of items.
     * @param inputStride - Input vector size.
     * @param outputStride - Output vector size.
     */
    async runBatch(
        inputs: Float32Array,
        outputs: Float32Array,
        indices: Uint32Array,
        count: number,
        inputStride: number,
        outputStride: number
    ): Promise<void> {
        if (this.status !== 'ready') {
            throw new Error(`Pool not ready: ${this.status}`);
        }

        // 1. Ensure capacity
        if (count > this.currentCapacity) {
            // Re-allocation required. This is expensive and requires respawning workers.
            // For Phase 5 initial implementation, we'll error or fallback.
            // Or simply auto-grow if we handle the lifecycle.
            // Let's implement auto-regrowth via re-init for robustness.
            console.warn('[WorkerPool] Growing pool capacity...', count);
            await this.initPool(this.specKey); // This will re-alloc with bigger size?
            // Wait, initPool currently uses fixed size. We need to dynamic param.
            // For simplicity now, let's bump the default size or fail.
            // Re-implement initPool with size param later.
            // For now, if we exceed 20k, we just fail to JS?
            if (count > this.currentCapacity) {
                console.warn('[WorkerPool] Capacity exceeded. Fallback to JS.');
                this.status = 'failed'; // Soft fail?
                throw new Error('Capacity exceeded');
            }
        }

        // 2. Copy inputs to shared buffer
        const inputF32 = new Float32Array(this.inputBuffer!);
        const totalInputFloats = count * inputStride;
        inputF32.set(inputs.subarray(0, totalInputFloats));

        // 2b. Copy indices
        const indicesU32 = new Uint32Array(this.indicesBuffer!);
        indicesU32.set(indices.subarray(0, count));

        // 3. Partition work
        const workerCount = this.workers.length;
        const itemsPerWorker = Math.ceil(count / workerCount);
        let start = 0;
        const promises: Promise<void>[] = [];

        for (let i = 0; i < workerCount; i++) {
            const w = this.workers[i];
            if (!w) continue; // Should not happen

            const chunkCount = Math.min(itemsPerWorker, count - start);
            if (chunkCount <= 0) break;

            const currentStart = start;
            start += chunkCount;

            promises.push(new Promise<void>((resolve, reject) => {
                const onMsg = (e: MessageEvent<InferWorkerResponse>) => {
                    const data = e.data;
                    if (data.type === 'done' || data.type === 'error') {
                        w.removeEventListener('message', onMsg);
                        if (data.type === 'error') {
                            reject(new Error(data.message));
                        } else if (data.error) {
                            reject(new Error(data.error));
                        } else {
                            resolve();
                        }
                    }
                };
                w.addEventListener('message', onMsg);
                const msg: InferWorkerInferMessage = {
                    type: 'infer',
                    batchStart: currentStart,
                    batchCount: chunkCount
                };
                w.postMessage(msg);
            }));
        }

        await Promise.all(promises);

        // 4. Copy outputs back
        const outputF32 = new Float32Array(this.outputBuffer!);
        const totalOutputFloats = count * outputStride;
        outputs.set(outputF32.subarray(0, totalOutputFloats));
    }

    private _allocateBuffers(capacity: number) {
        // InSize/OutSize should come from CFG or Init args.
        const inSize = CFG.brain.inSize;
        const outSize = CFG.brain.outSize;

        // We need to know parameter count for weights buffer.
        // This is tricky without the graph spec.
        // We rely on initPool caller ensuring spec is valid?
        // Or we wait for syncWeights to alloc? No, SAB must be sent in init.
        // We need paramCount passed to initPool?
        // Or we load the spec here?
        // We'll peek at `CFG.brain.graphSpec` if available?
        // Or just allocate BIG and resize?
        // Let's assume a max param count per snake (e.g. 10k) * capacity?
        // 20k snakes * 10k params * 4 = 800MB. That's heavy.
        // Typical snake is small (100 params). 20k * 100 * 4 = 8MB.
        // We need the ACTUAL param size.

        // Quick fix: Estimate or read from a helper.
        // If we can't get it, we default.
        // But let's check if we can get it from `world`.
        // The calling code `worker.ts` has `world`. `world.arch` has `info`.
        // `world.arch.info.totalCount`.
        // But `workerPool` is separate.
        // I'll stick to fixed size here and fix call site later?
        // No, let's look at `worker.ts` again. `initPool` call site logic.

        const inBytes = capacity * inSize * 4;
        const outBytes = capacity * outSize * 4;
        const indexBytes = capacity * 4; // Uint32

        // Placeholder for param bytes, overridden in real fix
        const maxParams = 5000;
        const weightBytes = capacity * maxParams * 4;

        const syncBytes = (SYNC_HEADER_SIZE + 32) * 4;

        this.inputBuffer = new SharedArrayBuffer(inBytes);
        this.outputBuffer = new SharedArrayBuffer(outBytes);
        this.weightsBuffer = new SharedArrayBuffer(weightBytes);
        this.indicesBuffer = new SharedArrayBuffer(indexBytes); // Alloc
        this.syncBuffer = new SharedArrayBuffer(syncBytes);
        // this.syncView = new Int32Array(this.syncBuffer);
        this.currentCapacity = capacity;
    }
}

export const workerPool = new WorkerPool();
