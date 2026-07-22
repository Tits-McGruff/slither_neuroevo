import type { GraphSpec } from '../src/brains/graph/schema.ts';
import type { InferenceBackend } from '../src/brains/types.ts';
import type { VizData } from '../src/protocol/messages.ts';

/** Operation being performed when a worker reports an error. */
export type BrainPoolWorkerErrorContext = 'init' | 'infer' | 'reset' | 'viz' | 'protocol';

/** Shared-buffer references sent once when a worker joins a pool epoch. */
export interface BrainPoolSharedBuffers {
  /** Packed population weights, indexed by population slot. */
  weights: SharedArrayBuffer;
  /** Packed inputs, indexed by current batch position. */
  inputs: SharedArrayBuffer;
  /** Packed outputs, indexed by current batch position. */
  outputs: SharedArrayBuffer;
  /** Population slots, indexed by current batch position. */
  indices: SharedArrayBuffer;
}

/** Initialize one worker as a stable owner inside a pool epoch. */
export interface BrainPoolInitMessage {
  /** Message discriminator. */
  type: 'init';
  /** Stable zero-based identity of this worker. */
  workerId: number;
  /** Immutable worker count for this pool epoch. */
  workerCount: number;
  /** Monotonic pool lifecycle epoch. */
  poolEpoch: number;
  /** Current population-weight epoch. */
  weightEpoch: number;
  /** Immutable backend selected before brains are constructed. */
  inferenceBackend: InferenceBackend;
  /** Graph specification compiled by the worker. */
  spec: GraphSpec;
  /** Stable key for the graph specification. */
  specKey: string;
  /** Number of dense population slots in this pool epoch. */
  populationCount: number;
  /** Number of weights in one population genome. */
  paramCount: number;
  /** Number of input floats in one batch position. */
  inputStride: number;
  /** Number of output floats in one batch position. */
  outputStride: number;
  /** Maximum number of batch positions in shared memory. */
  maxBatch: number;
  /** Shared buffers owned by the parent pool. */
  buffers: BrainPoolSharedBuffers;
}

/** Dispatch one batch to every worker without changing slot ownership. */
export interface BrainPoolInferMessage {
  /** Message discriminator. */
  type: 'infer';
  /** Pool epoch expected by the dispatch. */
  poolEpoch: number;
  /** Weight epoch expected by the dispatch. */
  weightEpoch: number;
  /** Monotonic dispatch identifier within the pool. */
  batchId: number;
  /** Number of populated batch positions. */
  batchCount: number;
}

/** Reset recurrent state after installing a new population weight epoch. */
export interface BrainPoolResetMessage {
  /** Message discriminator. */
  type: 'reset';
  /** Pool epoch whose brains must be reset. */
  poolEpoch: number;
  /** New population-weight epoch now visible in shared memory. */
  weightEpoch: number;
}

/** Request a snapshot of one worker-owned population brain. */
export interface BrainPoolVizMessage {
  /** Message discriminator. */
  type: 'viz';
  /** Pool epoch expected by the request. */
  poolEpoch: number;
  /** Weight epoch expected by the request. */
  weightEpoch: number;
  /** Monotonic visualization request identifier. */
  requestId: number;
  /** Population slot whose owning worker should respond. */
  populationSlot: number;
  /** Last committed authoritative simulation step. */
  simulationStep: number;
}

/** Ask a worker to stop accepting work before termination. */
export interface BrainPoolShutdownMessage {
  /** Message discriminator. */
  type: 'shutdown';
}

/** Union of messages sent from the parent pool to workers. */
export type BrainPoolWorkerRequest =
  | BrainPoolInitMessage
  | BrainPoolInferMessage
  | BrainPoolResetMessage
  | BrainPoolVizMessage
  | BrainPoolShutdownMessage;

/** Worker acknowledgement after backend preparation and owned-brain construction. */
export interface BrainPoolReadyMessage {
  /** Message discriminator. */
  type: 'ready';
  /** Worker sending the acknowledgement. */
  workerId: number;
  /** Pool epoch installed by the worker. */
  poolEpoch: number;
  /** Weight epoch installed by the worker. */
  weightEpoch: number;
  /** Backend captured by every owned brain. */
  activeBackend: InferenceBackend;
  /** Native addon identifier, or null for JS diagnostic workers. */
  nativeAddonBuildIdentifier: string | null;
  /** Number of population brains constructed by this worker. */
  ownedSlotCount: number;
}

/** Worker acknowledgement after processing all owned entries in a batch. */
export interface BrainPoolDoneMessage {
  /** Message discriminator. */
  type: 'done';
  /** Worker sending the acknowledgement. */
  workerId: number;
  /** Pool epoch used for inference. */
  poolEpoch: number;
  /** Weight epoch used for inference. */
  weightEpoch: number;
  /** Completed batch identifier. */
  batchId: number;
  /** Number of batch positions inspected by the worker. */
  batchCount: number;
  /** Number of entries owned and evaluated by the worker. */
  processedCount: number;
}

/** Worker acknowledgement after recurrent state reset. */
export interface BrainPoolResetDoneMessage {
  /** Message discriminator. */
  type: 'resetDone';
  /** Worker sending the acknowledgement. */
  workerId: number;
  /** Pool epoch whose state was reset. */
  poolEpoch: number;
  /** Newly installed population-weight epoch. */
  weightEpoch: number;
}

/** Worker response containing a tagged brain visualization snapshot. */
export interface BrainPoolVizResultMessage {
  /** Message discriminator. */
  type: 'vizResult';
  /** Worker sending the response. */
  workerId: number;
  /** Pool epoch used to read brain state. */
  poolEpoch: number;
  /** Weight epoch used to read brain state. */
  weightEpoch: number;
  /** Visualization request identifier. */
  requestId: number;
  /** Population slot whose state was read. */
  populationSlot: number;
  /** Authoritative simulation step supplied by the requester. */
  simulationStep: number;
  /** Serializable activation snapshot. */
  viz: VizData;
}

/** Structured worker failure reported to the parent pool. */
export interface BrainPoolErrorMessage {
  /** Message discriminator. */
  type: 'error';
  /** Worker reporting the error, or -1 before worker identity is installed. */
  workerId: number;
  /** Operation that failed. */
  context: BrainPoolWorkerErrorContext;
  /** Human-readable failure reason. */
  reason: string;
  /** Pool epoch observed by the worker, when initialized. */
  poolEpoch: number | null;
  /** Weight epoch observed by the worker, when initialized. */
  weightEpoch: number | null;
  /** Related batch identifier, when applicable. */
  batchId?: number;
  /** Related visualization request identifier, when applicable. */
  requestId?: number;
}

/** Union of messages sent from workers to the parent pool. */
export type BrainPoolWorkerResponse =
  | BrainPoolReadyMessage
  | BrainPoolDoneMessage
  | BrainPoolResetDoneMessage
  | BrainPoolVizResultMessage
  | BrainPoolErrorMessage;
