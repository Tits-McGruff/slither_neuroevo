import type { GraphSpec } from '../brains/graph/schema.ts';

/** Message sent to the inference worker to initialize it. */
export interface WorkerInitMessage {
    type: 'init';
    /** Unique key for the current brain architecture. */
    specKey: string;
    /** The graph specification to compile and run. */
    graphSpec: GraphSpec | null;
    /** Size of the input vector per snake. */
    inputStride: number;
    /** Size of the output vector per snake. */
    outputStride: number;
    /** Shared memory buffers for zero-copy data transfer. */
    buffers: {
        inputs: SharedArrayBuffer;
        outputs: SharedArrayBuffer;
        weights: SharedArrayBuffer;
        indices: SharedArrayBuffer;
        states?: SharedArrayBuffer;
        sync?: SharedArrayBuffer;
    };
    /** Index of the worker in the pool. */
    workerIndex: number;
}

/** Message sent to the inference worker to trigger a batch processing. */
export interface WorkerInferMessage {
    type: 'infer';
    /** Starting index in the shared buffers for this batch. */
    batchStart: number;
    /** Number of snakes to process in this batch. */
    batchCount: number;
}

/** Message sent to the inference worker to stop it. */
export interface WorkerShutdownMessage {
    type: 'shutdown';
}

/** Aggregate type for all messages sent TO the inference worker. */
export type InferWorkerMessage =
    | WorkerInitMessage
    | WorkerInferMessage
    | WorkerShutdownMessage;

/** Message sent FROM the inference worker back to the pool. */
export type InferWorkerResponse =
    | { type: 'ready' }
    | { type: 'done'; error?: string }
    | { type: 'error'; message: string };
