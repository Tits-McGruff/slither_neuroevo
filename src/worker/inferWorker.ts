/**
 * Dedicated inference worker for the SIMD/MT pipeline.
 * Executes batched neural network kernels on shared memory.
 */

import { parentPort } from 'node:worker_threads';
import { loadSimdKernels } from '../brains/wasmBridge.ts';
import type { InferWorkerMessage, InferWorkerResponse } from '../sim/poolProtocol.ts';
import { compileGraph } from '../brains/graph/compiler.ts';
import { GraphBrain } from '../brains/graph/runtime.ts';
import type { GraphSpec } from '../brains/graph/schema.ts';

// Local state
let inputStride = 0;
let outputStride = 0;
let inputs: Float32Array | null = null;
let outputs: Float32Array | null = null;
let weights: Float32Array | null = null;
let indices: Uint32Array | null = null;

let brain: GraphBrain | null = null;
let paramCount = 0;

if (!parentPort) {
    throw new Error('This script must be run as a worker thread');
}

parentPort.on('message', async (msg: InferWorkerMessage) => {
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
                indices = new Uint32Array(msg.buffers.indices);

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

                parentPort!.postMessage({ type: 'ready' } as InferWorkerResponse);
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
                parentPort!.postMessage({ type: 'done' } as InferWorkerResponse);
                break;
            }

            case 'shutdown':
                process.exit(0);
        }
    } catch (err: unknown) {
        console.error('[InferWorker] Error:', err);
        const errorMsg = err instanceof Error ? err.message : String(err);
        parentPort!.postMessage({ type: 'error', message: errorMsg } as InferWorkerResponse);
    }
});
