/**
 * Dedicated inference worker for the SIMD/MT pipeline.
 * Executes batched neural network kernels on shared memory.
 */

import { loadSimdKernels } from '../brains/wasmBridge.ts';
import type { InferWorkerMessage, InferWorkerResponse } from '../workerPool.ts';
import { compileGraph } from '../brains/graph/compiler.ts';
import { GraphBrain } from '../brains/graph/runtime.ts';
import type { GraphSpec } from '../brains/graph/schema.ts';

// Local state
let inputStride = 0;
let outputStride = 0;
let inputs: Float32Array | null = null;
let outputs: Float32Array | null = null;
let weights: Float32Array | null = null;
let indices: Uint32Array | null = null; // Added
// let sync: Int32Array | null = null; // Unused

let brain: GraphBrain | null = null;
let paramCount = 0;

// Minimal scope definition to satisfy TS if lib.webworker is missing
interface WorkerScope {
    postMessage(message: unknown, transfer?: Transferable[]): void;
    onmessage: ((this: WorkerScope, ev: MessageEvent) => void) | null;
    close(): void;
}

const scope = self as unknown as WorkerScope;

scope.onmessage = async (e: MessageEvent<InferWorkerMessage>) => {
    const msg = e.data;

    try {
        switch (msg.type) {
            case 'init': {
                // 1. Load kernels
                await loadSimdKernels();

                // 2. Setup buffers
                inputStride = msg.inputStride;
                outputStride = msg.outputStride;
                // Use SharedArrayBuffer views
                inputs = new Float32Array(msg.buffers.inputs);
                outputs = new Float32Array(msg.buffers.outputs);
                weights = new Float32Array(msg.buffers.weights);
                indices = new Uint32Array(msg.buffers.indices); // Added
                // sync = new Int32Array(msg.buffers.sync);

                // 3. Compile GraphBrain template
                if (msg.graphSpec) {
                    const spec = msg.graphSpec as GraphSpec;
                    const compiled = compileGraph(spec);

                    // We need a dummy weight buffer to init the brain structure.
                    // The brain will be rebound per-snake in the loop.
                    paramCount = compiled.totalParams;
                    const dummyWeights = new Float32Array(paramCount);
                    brain = new GraphBrain(compiled, dummyWeights);
                } else {
                    console.warn('[InferWorker] No graphSpec provided in init. Inference will fail.');
                }

                scope.postMessage({ type: 'ready' } as InferWorkerResponse);
                break;
            }

            case 'infer': {
                if (!inputs || !outputs || !weights || !indices || !brain) {
                    throw new Error('Worker not initialized or missing brain');
                }

                const count = msg.batchCount;
                const start = msg.batchStart;

                // Batched Inference Loop
                // Iterate over the assigned range of snakes.
                for (let i = start; i < start + count; i++) {
                    // 1. Rebind weights for this snake
                    const index = indices[i] ?? 0;
                    const wOffset = index * paramCount;
                    // subarray creates a lightweight view
                    const w = weights.subarray(wOffset, wOffset + paramCount);
                    brain.bindWeights(w);

                    // 2. Get input view
                    const inOffset = i * inputStride;
                    const inputVec = inputs.subarray(inOffset, inOffset + inputStride);

                    // 3. Run inference
                    const outputVec = brain.forward(inputVec);

                    // 4. Copy output
                    const outOffset = i * outputStride;
                    outputs.set(outputVec, outOffset);
                }

                // Notify completion
                scope.postMessage({ type: 'done' } as InferWorkerResponse);
                break;
            }

            case 'shutdown':
                scope.close();
                break;
        }
    } catch (err: unknown) {
        console.error('[InferWorker] Error:', err);
        const msg = err instanceof Error ? err.message : String(err);
        scope.postMessage({ type: 'error', message: msg } as InferWorkerResponse);
    }
};
