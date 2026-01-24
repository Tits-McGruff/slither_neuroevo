/**
 * Node.js Brain Pool Implementation.
 * 
 * Manages a pool of 'worker_threads' for parallel inference on the Server.
 * Extends BaseBrainPool for shared buffer management.
 */

import os from 'node:os';
import { Worker } from 'node:worker_threads';
import { BaseBrainPool, type BrainPoolInitOptions } from './BaseBrainPool.ts';
import type { GraphSpec } from '../brains/graph/schema.ts';

// We need to define the message types locally or import them if shared.
// For now, let's redefine locally to match the expected protocol of the unified worker.
// Ideally, the Worker script should be capable of handling both WebWorker and Node Worker messages.
// Currently `src/worker/inferWorker.ts` is designed for WebWorkers (postMessage).
// Node.js workers share a similar API but `import 'worker_threads'` parentPort vs self.

// TODO: The worker script `src/worker/inferWorker.ts` needs to be "universal" or we need a Node adapter.
// existing `server/brainPool.ts` spawns `inferWorker.ts` via tsx/esm?
// Yes: execArgv: ['--import', 'tsx/esm']

interface WorkerInitMessage {
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
        sync?: SharedArrayBuffer; // Optional if unused
    };
    workerIndex: number;
}

interface WorkerInferMessage {
    type: 'infer';
    batchStart: number;
    batchCount: number;
}


type WorkerMessage =
    | { type: 'ready' }
    | { type: 'done'; error?: string }
    | { type: 'error'; message: string };

export class NodeBrainPool extends BaseBrainPool {
    private workers: Worker[] = [];

    constructor(private requestedWorkerCount: number = 0) {
        super();
    }

    async init(options: BrainPoolInitOptions): Promise<void> {
        this.specKey = options.specKey;
        this.status = 'starting';

        // 1. Resolve worker count
        const cpuCount = os.cpus().length;
        const maxWorkers = Math.max(1, cpuCount - 1);
        const count = this.requestedWorkerCount > 0 ? this.requestedWorkerCount : maxWorkers;
        const workerCount = Math.min(count, maxWorkers);

        // 2. Allocate buffers
        // Heuristic: Pre-allocate for provided population count or default 20k
        const capacity = Math.max(options.populationCount || 20000, 20000);
        const buffers = this.allocateBuffers(capacity);

        console.log(`[NodeBrainPool] Spawning ${workerCount} workers...`);

        // 3. Spawn Workers
        // We point to the same worker script as the browser.
        // Node requires special handling for TS execution (handled by execArgv in constructor or here)
        // Path logic might be tricky from `src/sim` vs `server`.
        // `import.meta.url` in `src/sim` -> `../worker/inferWorker.ts`
        const workerUrl = new URL('../worker/inferWorker.ts', import.meta.url);

        const promises: Promise<void>[] = [];

        for (let i = 0; i < workerCount; i++) {
            const w = new Worker(workerUrl, {
                execArgv: ['--import', 'tsx/esm'],
                // workerData can be passed here if needed
            });
            this.workers.push(w);

            promises.push(new Promise<void>((resolve, reject) => {
                const onMsg = (msg: unknown) => {
                    const data = msg as WorkerMessage;
                    if (data.type === 'ready') {
                        w.off('message', onMsg);
                        resolve();
                    } else if (data.type === 'error') {
                        w.off('message', onMsg);
                        reject(new Error(data.message));
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
                    inputStride: 100, // FIXME: fetch from CFG? SimCore should probably pass this?
                    // Actually BaseBrainPool relies on CFG for allocation, so we can use CFG here too?
                    // Or options should include strides.
                    // Let's use CFG for now as per BaseBrainPool.
                    outputStride: 0, // Placeholder, see below
                    workerIndex: i,
                    buffers: buffers
                };
                // Re-read CFG or rely on SimCore passing strides? 
                // BaseBrainPool uses CFG.brain.inSize.
                // We should probably explicitly pass strides in options.
                // But IBrainPool signature is fixed.
                // BaseBrainPool imports CFG. Let's start with that.

                // Oops, in strict TS `BaseBrainPool` uses `CFG.brain.inSize` but `init` doesn't take strides.
                // For now, I'll access CFG (imported above via BaseBrainPool logic, or re-import).
                // I'll re-import to be safe or just assume BaseBrainPool logic holds valid defaults.

                // Wait, I can't access `CFG` if I don't import it.
                // But `BaseBrainPool` implementation uses `CFG`.
                // I'll import CFG here too.

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
                const onMsg = (msg: unknown) => {
                    const data = msg as WorkerMessage;
                    if (data.type === 'done' || data.type === 'error') {
                        w.off('message', onMsg);
                        if (data.type === 'error') reject(new Error(data.message));
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
