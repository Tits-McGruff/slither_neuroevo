/**
 * Dedicated inference worker for the SIMD/MT pipeline.
 * Executes batched neural network kernels on shared memory.
 */

import { parentPort } from 'node:worker_threads';
import { loadSimdKernels } from '../brains/wasmBridge.ts';
import type { InferWorkerMessage, InferWorkerResponse } from '../sim/poolProtocol.ts';
import { compileGraph } from '../brains/graph/compiler.ts';
import { GraphBrain, type RuntimeNode } from '../brains/graph/runtime.ts';
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

// Recurrent isolation state
let stateStore: Float32Array | null = null;
let totalStateFloats = 0;

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

                    paramCount = compiled.totalParams;
                    totalStateFloats = compiled.totalStateSize;

                    const dummyWeights = new Float32Array(paramCount);
                    brain = new GraphBrain(compiled, dummyWeights);

                    // Use shared state store from pool 
                    if (msg.buffers.states) {
                        stateStore = new Float32Array(msg.buffers.states);
                    } else if (totalStateFloats > 0) {
                        const capacity = msg.buffers.inputs.byteLength / (inputStride * 4);
                        stateStore = new Float32Array(capacity * totalStateFloats);
                        stateStore.fill(0);
                    }
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
                for (let i = start; i < start + count; i++) {
                    // 1. Rebind weights for this snake
                    const index = indices[i] ?? 0;
                    const wOffset = index * paramCount;
                    if (wOffset + paramCount > weights.length) {
                        console.error('[InferWorker] Weight buffer overflow', { index, wOffset, total: weights.length });
                        continue;
                    }
                    const w = weights.subarray(wOffset, wOffset + paramCount);
                    brain.bindWeights(w);

                    // 2. Isolate recurrent state (Copy-In)
                    if (totalStateFloats > 0 && stateStore) {
                        const stateBase = index * totalStateFloats;
                        let offset = 0;
                        for (const info of brain.compiled.recurrentNodes) {
                            const node = brain.nodes[info.nodeIndex] as RuntimeNode;
                            if (!node) continue;
                            if (info.type === 'GRU' && node.gru) {
                                node.gru.h.set(stateStore.subarray(stateBase + offset, stateBase + offset + info.hiddenSize));
                                offset += info.hiddenSize;
                            } else if (info.type === 'RRU' && node.rru) {
                                node.rru.h.set(stateStore.subarray(stateBase + offset, stateBase + offset + info.hiddenSize));
                                offset += info.hiddenSize;
                            } else if (info.type === 'LSTM' && node.lstm) {
                                node.lstm.h.set(stateStore.subarray(stateBase + offset, stateBase + offset + info.hiddenSize));
                                offset += info.hiddenSize;
                                node.lstm.c.set(stateStore.subarray(stateBase + offset, stateBase + offset + info.hiddenSize));
                                offset += info.hiddenSize;
                            }
                        }
                    }

                    // 3. Get input view
                    const inOffset = i * inputStride;
                    const inputVec = inputs.subarray(inOffset, inOffset + inputStride);

                    // 4. Run inference
                    const outputVec = brain.forward(inputVec);

                    // 5. Isolate recurrent state (Copy-Out)
                    if (totalStateFloats > 0 && stateStore) {
                        const stateBase = index * totalStateFloats;
                        let offset = 0;
                        for (const info of brain.compiled.recurrentNodes) {
                            const node = brain.nodes[info.nodeIndex] as RuntimeNode;
                            if (!node) continue;
                            if (info.type === 'GRU' && node.gru) {
                                stateStore.set(node.gru.h, stateBase + offset);
                                offset += info.hiddenSize;
                            } else if (info.type === 'RRU' && node.rru) {
                                stateStore.set(node.rru.h, stateBase + offset);
                                offset += info.hiddenSize;
                            } else if (info.type === 'LSTM' && node.lstm) {
                                stateStore.set(node.lstm.h, stateBase + offset);
                                offset += info.hiddenSize;
                                stateStore.set(node.lstm.c, stateBase + offset);
                                offset += info.hiddenSize;
                            }
                        }
                    }

                    // 6. Copy output back to shared buffer
                    const outOffset = i * outputStride;
                    outputs.set(outputVec, outOffset);
                }

                parentPort!.postMessage({ type: 'done' } as InferWorkerResponse);
                break;
            }

            case 'shutdown': {
                process.exit(0);
            }
        }
    } catch (err) {
        console.error('[InferWorker] Internal Error:', err);
        parentPort!.postMessage({
            type: 'error',
            message: err instanceof Error ? err.message : String(err)
        } as InferWorkerResponse);
    }
});
