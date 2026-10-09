import { parentPort, workerData } from 'node:worker_threads';
import { compileGraph } from '../../src/brains/graph/compiler.ts';
import { GraphBrain } from '../../src/brains/graph/runtime.ts';
import {
  getNativeAddonBuildIdentifier,
  prepareInferenceBackend
} from '../../src/brains/nativeBridge.ts';
import type { InferenceBackend } from '../../src/brains/types.ts';
import type { VizData, VizLayer } from '../../src/protocol/messages.ts';
import type {
  BrainPoolErrorMessage,
  BrainPoolInitMessage,
  BrainPoolInferMessage,
  BrainPoolResetMessage,
  BrainPoolShutdownMessage,
  BrainPoolVizMessage,
  BrainPoolWorkerErrorContext,
  BrainPoolWorkerRequest,
  BrainPoolWorkerResponse
} from '../brainPoolProtocol.ts';

/** Worker bootstrap payload supplied before message handling is enabled. */
interface InferenceWorkerData {
  /** Immutable math backend for every worker-owned brain. */
  inferenceBackend?: InferenceBackend;
}

/** Backend selected by the parent before this worker accepts messages. */
const inferenceBackend = (workerData as InferenceWorkerData | null)?.inferenceBackend ?? 'js';
if (inferenceBackend !== 'js' && inferenceBackend !== 'native') {
  throw new Error(`Unsupported inference backend: ${String(inferenceBackend)}`);
}
await prepareInferenceBackend(inferenceBackend);

/** Parent port required by this worker-thread-only module. */
if (!parentPort) {
  throw new Error('inferWorker requires parentPort');
}
/** Non-null parent port after the worker-context assertion. */
const port = parentPort;

/** Shared input buffer indexed by current batch position. */
let inputView: Float32Array | null = null;
/** Shared output buffer indexed by current batch position. */
let outputView: Float32Array | null = null;
/** Shared population-slot buffer indexed by current batch position. */
let indexView: Uint32Array | null = null;
/** Input float count for one batch position. */
let inputStride = 0;
/** Output float count for one batch position. */
let outputStride = 0;
/** Maximum batch position count backed by shared memory. */
let maxBatch = 0;
/** Dense population slot count for the current pool epoch. */
let populationCount = 0;
/** Stable identity assigned to this worker. */
let workerId = -1;
/** Immutable worker count for the current pool epoch. */
let workerCount = 0;
/** Installed pool lifecycle epoch. */
let poolEpoch: number | null = null;
/** Installed population-weight epoch. */
let weightEpoch: number | null = null;
/** Brains constructed only for population slots owned by this worker. */
let ownedBrains = new Map<number, GraphBrain>();

/**
 * Post a typed response to the parent pool.
 * @param message - Worker response to send.
 */
function post(message: BrainPoolWorkerResponse): void {
  port.postMessage(message);
}

/**
 * Copy graph activation views into a detached structured-clone-safe payload.
 * @param viz - Live graph visualization returned by a worker-owned brain.
 * @returns Visualization whose activation arrays cannot mutate after posting.
 */
function snapshotViz(viz: VizData): VizData {
  const layers: VizLayer[] = viz.layers.map((layer) => ({
    count: layer.count,
    activations: layer.activations ? Array.from(layer.activations) : null,
    ...(layer.isRecurrent === undefined ? {} : { isRecurrent: layer.isRecurrent })
  }));
  return { kind: viz.kind, layers };
}

/**
 * Assert that an operation targets the exact installed epochs.
 * @param expectedPoolEpoch - Pool epoch supplied by the parent.
 * @param expectedWeightEpoch - Weight epoch supplied by the parent.
 */
function assertEpochs(expectedPoolEpoch: number, expectedWeightEpoch: number): void {
  if (poolEpoch !== expectedPoolEpoch) {
    throw new Error(
      `inferWorker pool epoch mismatch: installed ${String(poolEpoch)}, received ${expectedPoolEpoch}`
    );
  }
  if (weightEpoch !== expectedWeightEpoch) {
    throw new Error(
      `inferWorker weight epoch mismatch: installed ${String(weightEpoch)}, received ${expectedWeightEpoch}`
    );
  }
}

/**
 * Initialize shared buffers and construct only deterministic worker-owned slots.
 * @param message - Initialization payload for a fresh pool epoch.
 */
function handleInit(message: BrainPoolInitMessage): void {
  if (poolEpoch !== null) {
    throw new Error('inferWorker received duplicate initialization');
  }
  if (message.inferenceBackend !== inferenceBackend) {
    throw new Error(
      `inferWorker backend mismatch: prepared ${inferenceBackend}, received ${message.inferenceBackend}`
    );
  }
  const compiled = compileGraph(message.spec);
  if (compiled.totalParams !== message.paramCount) {
    throw new Error(
      `inferWorker paramCount mismatch: compiled ${compiled.totalParams}, received ${message.paramCount}`
    );
  }
  if (compiled.outputSize !== message.outputStride) {
    throw new Error(
      `inferWorker output stride mismatch: compiled ${compiled.outputSize}, received ${message.outputStride}`
    );
  }

  const expectedWeightFloats = message.populationCount * message.paramCount;
  const expectedInputFloats = message.maxBatch * message.inputStride;
  const expectedOutputFloats = message.maxBatch * message.outputStride;
  if (
    message.buffers.weights.byteLength !==
    expectedWeightFloats * Float32Array.BYTES_PER_ELEMENT
  ) {
    throw new Error('inferWorker weights buffer capacity mismatch');
  }
  if (
    message.buffers.inputs.byteLength !==
    expectedInputFloats * Float32Array.BYTES_PER_ELEMENT
  ) {
    throw new Error('inferWorker input buffer capacity mismatch');
  }
  if (
    message.buffers.outputs.byteLength !==
    expectedOutputFloats * Float32Array.BYTES_PER_ELEMENT
  ) {
    throw new Error('inferWorker output buffer capacity mismatch');
  }
  if (
    message.buffers.indices.byteLength !==
    message.maxBatch * Uint32Array.BYTES_PER_ELEMENT
  ) {
    throw new Error('inferWorker index buffer capacity mismatch');
  }

  workerId = message.workerId;
  workerCount = message.workerCount;
  poolEpoch = message.poolEpoch;
  weightEpoch = message.weightEpoch;
  populationCount = message.populationCount;
  inputStride = message.inputStride;
  outputStride = message.outputStride;
  maxBatch = message.maxBatch;
  inputView = new Float32Array(message.buffers.inputs);
  outputView = new Float32Array(message.buffers.outputs);
  indexView = new Uint32Array(message.buffers.indices);

  const weightsView = new Float32Array(message.buffers.weights);
  ownedBrains = new Map<number, GraphBrain>();
  for (let slot = workerId; slot < populationCount; slot += workerCount) {
    const weightOffset = slot * message.paramCount;
    const weights = weightsView.subarray(weightOffset, weightOffset + message.paramCount);
    const brain = new GraphBrain(compiled, weights, inferenceBackend);
    brain.reset();
    ownedBrains.set(slot, brain);
  }

  post({
    type: 'ready',
    workerId,
    poolEpoch,
    weightEpoch,
    activeBackend: inferenceBackend,
    nativeAddonBuildIdentifier: getNativeAddonBuildIdentifier(),
    ownedSlotCount: ownedBrains.size
  });
}

/**
 * Evaluate only entries whose durable population slots belong to this worker.
 * @param message - Batch dispatch shared by every worker.
 */
function handleInfer(message: BrainPoolInferMessage): void {
  assertEpochs(message.poolEpoch, message.weightEpoch);
  if (!inputView || !outputView || !indexView) {
    throw new Error('inferWorker is not initialized');
  }
  if (!Number.isSafeInteger(message.batchCount) || message.batchCount < 0) {
    throw new Error('inferWorker received an invalid batch count');
  }
  if (message.batchCount > maxBatch) {
    throw new Error(
      `inferWorker batch count ${message.batchCount} exceeds capacity ${maxBatch}`
    );
  }

  let processedCount = 0;
  for (let batchPosition = 0; batchPosition < message.batchCount; batchPosition++) {
    const populationSlot = indexView[batchPosition] ?? populationCount;
    if (populationSlot >= populationCount) {
      throw new Error(`inferWorker received out-of-range population slot ${populationSlot}`);
    }
    if (populationSlot % workerCount !== workerId) continue;
    const brain = ownedBrains.get(populationSlot);
    if (!brain) {
      throw new Error(`inferWorker missing owned population slot ${populationSlot}`);
    }
    const inputOffset = batchPosition * inputStride;
    const outputOffset = batchPosition * outputStride;
    const result = brain.forward(inputView.subarray(inputOffset, inputOffset + inputStride));
    const copyCount = Math.min(result.length, outputStride);
    outputView.set(result.subarray(0, copyCount), outputOffset);
    if (copyCount < outputStride) {
      outputView.fill(0, outputOffset + copyCount, outputOffset + outputStride);
    }
    processedCount += 1;
  }

  post({
    type: 'done',
    workerId,
    poolEpoch: message.poolEpoch,
    weightEpoch: message.weightEpoch,
    batchId: message.batchId,
    batchCount: message.batchCount,
    processedCount
  });
}

/**
 * Install a new population-weight epoch and clear every owned recurrent state.
 * @param message - Reset command following the parent's shared-weight copy.
 */
function handleReset(message: BrainPoolResetMessage): void {
  if (poolEpoch !== message.poolEpoch) {
    throw new Error(
      `inferWorker reset pool epoch mismatch: installed ${String(poolEpoch)}, received ${message.poolEpoch}`
    );
  }
  if (weightEpoch === null || message.weightEpoch !== weightEpoch + 1) {
    throw new Error(
      `inferWorker reset weight epoch mismatch: installed ${String(weightEpoch)}, received ${message.weightEpoch}`
    );
  }
  for (const brain of ownedBrains.values()) {
    brain.bindWeights(brain.weights);
    brain.reset();
  }
  weightEpoch = message.weightEpoch;
  post({
    type: 'resetDone',
    workerId,
    poolEpoch: message.poolEpoch,
    weightEpoch: message.weightEpoch
  });
}

/**
 * Return a detached visualization for a slot owned by this worker.
 * @param message - Tagged visualization request.
 */
function handleViz(message: BrainPoolVizMessage): void {
  assertEpochs(message.poolEpoch, message.weightEpoch);
  if (message.populationSlot % workerCount !== workerId) {
    throw new Error(
      `inferWorker ${workerId} does not own population slot ${message.populationSlot}`
    );
  }
  const brain = ownedBrains.get(message.populationSlot);
  if (!brain) {
    throw new Error(`inferWorker cannot visualize missing slot ${message.populationSlot}`);
  }
  post({
    type: 'vizResult',
    workerId,
    poolEpoch: message.poolEpoch,
    weightEpoch: message.weightEpoch,
    requestId: message.requestId,
    populationSlot: message.populationSlot,
    simulationStep: message.simulationStep,
    viz: snapshotViz(brain.getVizData())
  });
}

/**
 * Stop accepting parent messages and allow the worker event loop to close.
 * @param _message - Typed shutdown command.
 */
function handleShutdown(_message: BrainPoolShutdownMessage): void {
  port.removeAllListeners('message');
  port.close();
}

/**
 * Map a parent request to its structured error context.
 * @param message - Request that failed.
 * @returns Error context used in the worker response.
 */
function errorContext(message: BrainPoolWorkerRequest): BrainPoolWorkerErrorContext {
  switch (message.type) {
    case 'init':
      return 'init';
    case 'infer':
      return 'infer';
    case 'reset':
      return 'reset';
    case 'viz':
      return 'viz';
    default:
      return 'protocol';
  }
}

port.on('message', (message: BrainPoolWorkerRequest) => {
  try {
    switch (message.type) {
      case 'init':
        handleInit(message);
        return;
      case 'infer':
        handleInfer(message);
        return;
      case 'reset':
        handleReset(message);
        return;
      case 'viz':
        handleViz(message);
        return;
      case 'shutdown':
        handleShutdown(message);
        return;
      default:
        throw new Error('inferWorker received an unknown message');
    }
  } catch (error) {
    const response: BrainPoolErrorMessage = {
      type: 'error',
      workerId,
      context: errorContext(message),
      reason: error instanceof Error ? error.message : String(error),
      poolEpoch,
      weightEpoch,
      ...(message.type === 'infer' ? { batchId: message.batchId } : {}),
      ...(message.type === 'viz' ? { requestId: message.requestId } : {})
    };
    post(response);
  }
});
