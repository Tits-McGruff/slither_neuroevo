import os from 'node:os';
import { Worker } from 'node:worker_threads';
import type { GraphSpec } from '../src/brains/graph/schema.ts';
import type { BatchInferenceRunner } from '../src/world.ts';

/** Worker pool lifecycle states. */
export type PoolStatus = 'disabled' | 'starting' | 'ready' | 'failed';

/** Initialization payload for the brain pool. */
export interface BrainPoolInitOptions {
  /** Graph spec to compile inside each worker. */
  spec: GraphSpec;
  /** Spec key identifying the graph layout and sizes. */
  specKey: string;
  /** Total population count of brains to materialize. */
  populationCount: number;
  /** Parameter count per brain. */
  paramCount: number;
  /** Input stride per batch entry. */
  inputStride: number;
  /** Output stride per batch entry. */
  outputStride: number;
  /** Maximum batch capacity to allocate for shared buffers. */
  maxBatch: number;
  /** Packed weights for every population slot. */
  weights: Float32Array;
}

/** Worker init message payload. */
interface WorkerInitMessage {
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
interface WorkerInferMessage {
  type: 'infer';
  batchId: number;
  batchStart: number;
  batchCount: number;
}

/** Worker shutdown message payload. */
interface WorkerShutdownMessage {
  type: 'shutdown';
}

/** Worker reset message payload. */
interface WorkerResetMessage {
  type: 'reset';
}

/** Worker ready message payload. */
interface WorkerReadyMessage {
  type: 'ready';
}

/** Worker done message payload. */
interface WorkerDoneMessage {
  type: 'done';
  batchId: number;
  batchStart: number;
  batchCount: number;
}

/** Worker error message payload. */
interface WorkerErrorMessage {
  type: 'error';
  reason: string;
}

/** Union of messages received from workers. */
type WorkerMessage = WorkerReadyMessage | WorkerDoneMessage | WorkerErrorMessage;

/** Pending batch tracking record. */
interface PendingBatch {
  /** Batch id assigned to the dispatch. */
  batchId: number;
  /** Remaining worker responses required for completion. */
  remaining: number;
  /** Resolve callback for batch completion. */
  resolve: () => void;
  /** Reject callback for batch failure. */
  reject: (err: Error) => void;
}

/** Worker init tracking record. */
interface PendingInit {
  /** Resolve callback when init is acknowledged. */
  resolve: () => void;
  /** Reject callback when init fails. */
  reject: (err: Error) => void;
}

/**
 * Server-side worker pool for batched brain inference.
 */
export class BrainPool implements BatchInferenceRunner {
  /** Current pool status. */
  status: PoolStatus;
  /** Configured worker count. */
  workerCount: number;
  /** Active worker threads. */
  workers: Worker[];
  /** Spec key for the current pool configuration. */
  specKey: string | null;
  /** Population count for the current pool. */
  populationCount: number;
  /** Parameter count per brain. */
  paramCount: number;
  /** Input stride per batch entry. */
  inputStride: number;
  /** Output stride per batch entry. */
  outputStride: number;
  /** Shared weights buffer for all brains. */
  weightsBuffer: SharedArrayBuffer | null;
  /** Shared input buffer for batched inference. */
  inputBuffer: SharedArrayBuffer | null;
  /** Shared output buffer for batched inference. */
  outputBuffer: SharedArrayBuffer | null;
  /** Shared index buffer for batched inference. */
  indexBuffer: SharedArrayBuffer | null;
  /** Float32 view of the shared weights buffer. */
  weightsView: Float32Array | null;
  /** Float32 view of the shared input buffer. */
  inputView: Float32Array | null;
  /** Float32 view of the shared output buffer. */
  outputView: Float32Array | null;
  /** Uint32 view of the shared index buffer. */
  indexView: Uint32Array | null;
  /** Next batch id to assign. */
  nextBatchId: number;
  /** Currently in-flight batch, if any. */
  inflight: PendingBatch | null;
  /** Pending init acknowledgements per worker. */
  pendingInit: Map<Worker, PendingInit>;
  /** Whether a failure has been logged. */
  didLogFailure: boolean;
  /** Whether a shutdown has been requested. */
  shutdownRequested: boolean;

  /**
   * Create a new brain pool with a requested worker count.
   * @param workerCount - Requested worker thread count.
   */
  constructor(workerCount: number) {
    this.workerCount = resolveWorkerCount(workerCount);
    this.status = this.workerCount > 0 ? 'disabled' : 'failed';
    this.workers = [];
    this.specKey = null;
    this.populationCount = 0;
    this.paramCount = 0;
    this.inputStride = 0;
    this.outputStride = 0;
    this.weightsBuffer = null;
    this.inputBuffer = null;
    this.outputBuffer = null;
    this.indexBuffer = null;
    this.weightsView = null;
    this.inputView = null;
    this.outputView = null;
    this.indexView = null;
    this.nextBatchId = 1;
    this.inflight = null;
    this.pendingInit = new Map();
    this.didLogFailure = false;
    this.shutdownRequested = false;
  }

  /**
   * Initialize or reinitialize the worker pool.
   * @param options - Pool configuration and buffers.
   */
  async init(options: BrainPoolInitOptions): Promise<void> {
    if (this.workerCount <= 0) {
      this.status = 'failed';
      throw new Error('mt pool disabled (worker count <= 0)');
    }
    this.status = 'starting';
    await this.shutdown();

    this.specKey = options.specKey;
    this.populationCount = options.populationCount;
    this.paramCount = options.paramCount;
    this.inputStride = options.inputStride;
    this.outputStride = options.outputStride;

    const weightsLength = options.weights.length;
    const weightBytes = weightsLength * Float32Array.BYTES_PER_ELEMENT;
    this.weightsBuffer = new SharedArrayBuffer(weightBytes);
    this.weightsView = new Float32Array(this.weightsBuffer);
    this.weightsView.set(options.weights);

    const maxBatch = Math.max(0, Math.floor(options.maxBatch));
    const inputLength = maxBatch * this.inputStride;
    const outputLength = maxBatch * this.outputStride;
    const inputBytes = inputLength * Float32Array.BYTES_PER_ELEMENT;
    const outputBytes = outputLength * Float32Array.BYTES_PER_ELEMENT;
    const indexBytes = maxBatch * Uint32Array.BYTES_PER_ELEMENT;

    this.inputBuffer = new SharedArrayBuffer(inputBytes);
    this.outputBuffer = new SharedArrayBuffer(outputBytes);
    this.indexBuffer = new SharedArrayBuffer(indexBytes);
    this.inputView = new Float32Array(this.inputBuffer);
    this.outputView = new Float32Array(this.outputBuffer);
    this.indexView = new Uint32Array(this.indexBuffer);

    const workerUrl = new URL('./worker/inferWorker.ts', import.meta.url);
    const readyTasks: Promise<void>[] = [];
    for (let i = 0; i < this.workerCount; i++) {
      const worker = new Worker(workerUrl, {
        execArgv: ['--import', 'tsx/esm']
      });
      this.attachWorker(worker);
      this.workers.push(worker);
      readyTasks.push(this.awaitWorkerReady(worker));
      const initMessage: WorkerInitMessage = {
        type: 'init',
        spec: options.spec,
        specKey: options.specKey,
        populationCount: options.populationCount,
        paramCount: options.paramCount,
        inputStride: this.inputStride,
        outputStride: this.outputStride,
        maxBatch,
        weightsBuffer: this.weightsBuffer,
        inputBuffer: this.inputBuffer,
        outputBuffer: this.outputBuffer,
        indexBuffer: this.indexBuffer
      };
      worker.postMessage(initMessage);
    }

    try {
      await Promise.all(readyTasks);
      this.status = 'ready';
      this.didLogFailure = false;
    } catch (err) {
      this.status = 'failed';
      if (!this.didLogFailure) {
        const message = err instanceof Error ? err.message : String(err);
        console.warn('[mt.pool.failed]', { reason: message });
        this.didLogFailure = true;
      }
      throw err;
    }
  }

  /**
   * Run batched inference using the worker pool.
   * @param inputs - Packed input buffer.
   * @param outputs - Packed output buffer.
   * @param indices - Snake index mapping for the batch.
   * @param count - Batch entry count.
   * @param inputStride - Input stride per batch entry.
   * @param outputStride - Output stride per batch entry.
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
      throw new Error('mt pool not ready');
    }
    if (!this.inputView || !this.outputView || !this.indexView) {
      throw new Error('mt pool buffers not initialized');
    }
    if (inputStride !== this.inputStride || outputStride !== this.outputStride) {
      throw new Error('mt pool stride mismatch');
    }
    const safeCount = Math.max(0, Math.floor(count));
    if (safeCount === 0) return;
    if (this.inflight) {
      throw new Error('mt pool dispatch already in flight');
    }
    const inputCount = safeCount * inputStride;
    const outputCount = safeCount * outputStride;
    if (inputs.length < inputCount || outputs.length < outputCount) {
      throw new Error('mt pool batch buffers too small');
    }
    if (indices.length < safeCount) {
      throw new Error('mt pool index buffer too small');
    }
    if (this.inputView.length < inputCount || this.outputView.length < outputCount) {
      throw new Error('mt pool shared buffers too small');
    }

    this.inputView.set(inputs.subarray(0, inputCount));
    this.indexView.set(indices.subarray(0, safeCount));
    this.outputView.fill(0, 0, outputCount);

    const batchId = this.nextBatchId++;
    const chunk = Math.ceil(safeCount / this.workerCount);
    let dispatched = 0;
    for (let i = 0; i < this.workerCount; i++) {
      const start = i * chunk;
      const remaining = safeCount - start;
      const batchCount = remaining > 0 ? Math.min(chunk, remaining) : 0;
      if (batchCount <= 0) continue;
      dispatched += 1;
      const message: WorkerInferMessage = {
        type: 'infer',
        batchId,
        batchStart: start,
        batchCount
      };
      this.workers[i]?.postMessage(message);
    }

    if (dispatched === 0) {
      outputs.set(this.outputView.subarray(0, outputCount));
      return;
    }

    await new Promise<void>((resolve, reject) => {
      this.inflight = { batchId, remaining: dispatched, resolve, reject };
    });
    this.inflight = null;
    outputs.set(this.outputView.subarray(0, outputCount));
  }

  /**
   * Update shared weights without reinitializing the pool.
   * @param weights - Packed weight buffer to copy.
   */
  updateWeights(weights: Float32Array): void {
    if (!this.weightsView) {
      throw new Error('mt pool weights buffer not initialized');
    }
    if (weights.length !== this.weightsView.length) {
      throw new Error('mt pool weights length mismatch');
    }
    this.weightsView.set(weights);
  }

  /**
   * Reset brain state on all workers after weight updates.
   */
  async resetBrains(): Promise<void> {
    if (this.status !== 'ready') return;
    const resets = this.workers.map((worker) =>
      new Promise<void>((resolve, reject) => {
        const handler = (msg: WorkerMessage) => {
          if (msg.type !== 'ready') return;
          worker.off('message', handler);
          resolve();
        };
        worker.on('message', handler);
        worker.postMessage({ type: 'reset' } satisfies WorkerResetMessage);
        setTimeout(() => {
          worker.off('message', handler);
          reject(new Error('mt pool reset timeout'));
        }, 5000);
      })
    );
    await Promise.all(resets);
  }

  /**
   * Shutdown all workers and release buffers.
   */
  async shutdown(): Promise<void> {
    if (this.workers.length === 0) return;
    this.shutdownRequested = true;
    const tasks = this.workers.map(async (worker) => {
      worker.postMessage({ type: 'shutdown' } satisfies WorkerShutdownMessage);
      try {
        await worker.terminate();
      } catch {
        // Ignore terminate errors on shutdown.
      }
    });
    await Promise.all(tasks);
    this.workers = [];
    this.pendingInit.clear();
    this.inflight = null;
    this.status = 'disabled';
    this.specKey = null;
    this.populationCount = 0;
    this.paramCount = 0;
    this.weightsBuffer = null;
    this.inputBuffer = null;
    this.outputBuffer = null;
    this.indexBuffer = null;
    this.weightsView = null;
    this.inputView = null;
    this.outputView = null;
    this.indexView = null;
    this.shutdownRequested = false;
  }

  /**
   * Attach message/error handlers for a worker thread.
   * @param worker - Worker instance to wire up.
   */
  private attachWorker(worker: Worker): void {
    worker.on('message', (msg: WorkerMessage) => {
      if (!msg || typeof msg !== 'object') return;
      if (msg.type === 'ready') {
        const pending = this.pendingInit.get(worker);
        if (pending) {
          pending.resolve();
          this.pendingInit.delete(worker);
        }
        return;
      }
      if (msg.type === 'done') {
        const inflight = this.inflight;
        if (inflight && inflight.batchId === msg.batchId) {
          inflight.remaining -= 1;
          if (inflight.remaining <= 0) inflight.resolve();
        }
        return;
      }
      if (msg.type === 'error') {
        this.failPool(new Error(msg.reason));
      }
    });
    worker.on('error', (err) => {
      if (this.shutdownRequested) return;
      this.failPool(err);
    });
    worker.on('exit', (code) => {
      if (this.shutdownRequested) return;
      if (code !== 0) this.failPool(new Error(`worker exited with code ${code}`));
    });
  }

  /**
   * Await a ready acknowledgement from a worker.
   * @param worker - Worker instance awaiting init.
   * @returns Promise resolved when ready.
   */
  private awaitWorkerReady(worker: Worker): Promise<void> {
    return new Promise<void>((resolve, reject) => {
      this.pendingInit.set(worker, { resolve, reject });
      setTimeout(() => {
        if (this.pendingInit.has(worker)) {
          this.pendingInit.delete(worker);
          reject(new Error('mt pool init timeout'));
        }
      }, 15000);
    });
  }

  /**
   * Mark the pool as failed and reject any in-flight work.
   * @param err - Error that caused the failure.
   */
  private failPool(err: Error): void {
    if (this.status === 'failed') return;
    this.status = 'failed';
    if (this.inflight) {
      this.inflight.reject(err);
      this.inflight = null;
    }
    for (const pending of this.pendingInit.values()) {
      pending.reject(err);
    }
    this.pendingInit.clear();
    if (!this.didLogFailure) {
      console.warn('[mt.pool.failed]', { reason: err.message });
      this.didLogFailure = true;
    }
  }
}

/**
 * Resolve a worker count from a requested value and CPU availability.
 * @param requested - Requested worker count.
 * @returns Sanitized worker count to use.
 */
function resolveWorkerCount(requested: number): number {
  const cpuCount = os.cpus().length;
  const maxWorkers = Math.max(1, cpuCount - 1);
  const parsed = Number.isFinite(requested) ? Math.floor(requested) : 0;
  if (parsed <= 0) return maxWorkers;
  return Math.min(parsed, maxWorkers);
}
