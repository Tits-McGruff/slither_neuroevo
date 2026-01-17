import { parentPort } from 'node:worker_threads';
import { loadSimdKernels } from '../../src/brains/wasmBridge.ts';
import { compileGraph } from '../../src/brains/graph/compiler.ts';
import { GraphBrain } from '../../src/brains/graph/runtime.ts';
import type { GraphSpec } from '../../src/brains/graph/schema.ts';

/** Worker init message payload. */
interface InitMessage {
  type: 'init';
  spec: GraphSpec;
  specKey: string;
  populationCount: number;
  paramCount: number;
  inputStride: number;
  outputStride: number;
  maxBatch: number;
  weightsBuffer: SharedArrayBuffer;
  inputBuffer: SharedArrayBuffer;
  outputBuffer: SharedArrayBuffer;
  indexBuffer: SharedArrayBuffer;
}

/** Worker inference message payload. */
interface InferMessage {
  type: 'infer';
  batchId: number;
  batchStart: number;
  batchCount: number;
}

/** Worker reset message payload. */
interface ResetMessage {
  type: 'reset';
}

/** Worker shutdown message payload. */
interface ShutdownMessage {
  type: 'shutdown';
}

/** Union of messages received by the worker. */
type WorkerMessage = InitMessage | InferMessage | ResetMessage | ShutdownMessage;

/** Worker ready message payload. */
interface ReadyMessage {
  type: 'ready';
}

/** Worker done message payload. */
interface DoneMessage {
  type: 'done';
  batchId: number;
  batchStart: number;
  batchCount: number;
}

/** Worker error message payload. */
interface ErrorMessage {
  type: 'error';
  reason: string;
}

/** Shared input buffer view for batched inference. */
let inputView: Float32Array | null = null;
/** Shared output buffer view for batched inference. */
let outputView: Float32Array | null = null;
/** Shared index buffer view for batched inference. */
let indexView: Uint32Array | null = null;
/** Input stride per batch entry. */
let inputStride = 0;
/** Output stride per batch entry. */
let outputStride = 0;
/** Brain instances for each population slot. */
let brains: GraphBrain[] = [];

/**
 * Ensure the worker is running under worker_threads.
 */
function assertWorkerContext(): void {
  if (!parentPort) {
    throw new Error('inferWorker requires parentPort');
  }
}

/**
 * Post a message to the parent port.
 * @param msg - Message payload to send.
 */
function postMessage(msg: ReadyMessage | DoneMessage | ErrorMessage): void {
  parentPort?.postMessage(msg);
}

/**
 * Initialize the worker with shared buffers and compiled brains.
 * @param msg - Init message payload.
 */
async function handleInit(msg: InitMessage): Promise<void> {
  const compiled = compileGraph(msg.spec);
  if (compiled.totalParams !== msg.paramCount) {
    throw new Error('inferWorker paramCount mismatch');
  }
  const inputLen = msg.maxBatch * msg.inputStride;
  const outputLen = msg.maxBatch * msg.outputStride;
  if (msg.inputBuffer.byteLength < inputLen * Float32Array.BYTES_PER_ELEMENT) {
    throw new Error('inferWorker input buffer too small');
  }
  if (msg.outputBuffer.byteLength < outputLen * Float32Array.BYTES_PER_ELEMENT) {
    throw new Error('inferWorker output buffer too small');
  }
  if (msg.indexBuffer.byteLength < msg.maxBatch * Uint32Array.BYTES_PER_ELEMENT) {
    throw new Error('inferWorker index buffer too small');
  }
  await loadSimdKernels();
  inputView = new Float32Array(msg.inputBuffer);
  outputView = new Float32Array(msg.outputBuffer);
  indexView = new Uint32Array(msg.indexBuffer);
  inputStride = Math.max(0, Math.floor(msg.inputStride));
  outputStride = Math.max(0, Math.floor(msg.outputStride));

  brains = new Array(msg.populationCount);
  for (let i = 0; i < msg.populationCount; i++) {
    const offset = i * msg.paramCount;
    const weights = new Float32Array(
      msg.weightsBuffer,
      offset * Float32Array.BYTES_PER_ELEMENT,
      msg.paramCount
    );
    const brain = new GraphBrain(compiled, weights);
    brain.reset();
    brains[i] = brain;
  }
  postMessage({ type: 'ready' });
}

/**
 * Reset all brain state buffers without reinitializing the pool.
 */
function handleReset(): void {
  for (const brain of brains) {
    brain?.reset();
  }
  postMessage({ type: 'ready' });
}

/**
 * Handle a batched inference request.
 * @param msg - Infer message payload.
 */
function handleInfer(msg: InferMessage): void {
  if (!inputView || !outputView || !indexView) {
    postMessage({ type: 'error', reason: 'inferWorker not initialized' });
    return;
  }
  const start = Math.max(0, Math.floor(msg.batchStart));
  const count = Math.max(0, Math.floor(msg.batchCount));
  const end = start + count;
  for (let b = start; b < end; b++) {
    const snakeIndex = indexView[b] ?? 0;
    const brain = brains[snakeIndex];
    if (!brain) continue;
    const inputBase = b * inputStride;
    const outputBase = b * outputStride;
    const inputSlice = inputView.subarray(inputBase, inputBase + inputStride);
    const out = brain.forward(inputSlice);
    const limit = Math.min(out.length, outputStride);
    for (let i = 0; i < limit; i++) {
      outputView[outputBase + i] = out[i] ?? 0;
    }
    for (let i = limit; i < outputStride; i++) {
      outputView[outputBase + i] = 0;
    }
  }
  postMessage({ type: 'done', batchId: msg.batchId, batchStart: start, batchCount: count });
}

/**
 * Handle a shutdown request.
 */
function handleShutdown(): void {
  postMessage({ type: 'ready' });
  process.exit(0);
}

assertWorkerContext();
parentPort?.on('message', async (msg: WorkerMessage) => {
  try {
    if (msg.type === 'init') {
      await handleInit(msg);
      return;
    }
    if (msg.type === 'reset') {
      handleReset();
      return;
    }
    if (msg.type === 'infer') {
      handleInfer(msg);
      return;
    }
    if (msg.type === 'shutdown') {
      handleShutdown();
      return;
    }
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    postMessage({ type: 'error', reason: message });
  }
});
