import os from 'node:os';
import { Worker } from 'node:worker_threads';
import type { GraphSpec } from '../src/brains/graph/schema.ts';
import type { InferenceBackend } from '../src/brains/types.ts';
import type { VizData } from '../src/protocol/messages.ts';
import type { BatchInferenceRunner } from '../src/world.ts';
import type {
  BrainPoolDoneMessage,
  BrainPoolInitMessage,
  BrainPoolReadyMessage,
  BrainPoolResetDoneMessage,
  BrainPoolResetMessage,
  BrainPoolShutdownMessage,
  BrainPoolVizMessage,
  BrainPoolVizResultMessage,
  BrainPoolWorkerResponse
} from './brainPoolProtocol.ts';

/** Conservative worker ceiling pending workload-specific benchmark evidence. */
const MAX_WORKER_COUNT = 8;
/** Largest shared buffer accepted by this pool. */
const MAX_SHARED_BUFFER_BYTES = 0x7fff_ffff;
/** Default worker initialization timeout. */
const DEFAULT_INIT_TIMEOUT_MS = 15_000;
/** Default inference dispatch timeout. */
const DEFAULT_INFERENCE_TIMEOUT_MS = 5_000;
/** Default recurrent reset timeout. */
const DEFAULT_RESET_TIMEOUT_MS = 5_000;
/** Default selected-brain visualization timeout. */
const DEFAULT_VIZ_TIMEOUT_MS = 2_000;
/** Minimum default interval between worker visualization requests. */
const DEFAULT_VIZ_MIN_INTERVAL_MS = 100;
/** Process-local monotonic source for worker-pool lifecycle epochs. */
let nextPoolEpoch = 1;

/** Worker pool lifecycle states. */
export type PoolStatus = 'disabled' | 'starting' | 'ready' | 'failed';

/** Initialization payload for a canonical population brain pool. */
export interface BrainPoolInitOptions {
  /** Graph spec compiled inside every worker. */
  spec: GraphSpec;
  /** Stable key identifying the graph layout and sizes. */
  specKey: string;
  /** Dense population slot count for this pool epoch. */
  populationCount: number;
  /** Parameter count in one population genome. */
  paramCount: number;
  /** Input stride for one current batch position. */
  inputStride: number;
  /** Output stride for one current batch position. */
  outputStride: number;
  /** Maximum batch position count backed by shared memory. */
  maxBatch: number;
  /** Packed initial weights for all population slots. */
  weights: Float32Array;
}

/** Optional timeout and visualization pacing overrides used by focused tests. */
export interface BrainPoolRuntimeOptions {
  /** Initialization timeout in milliseconds. */
  initTimeoutMs?: number;
  /** Inference timeout in milliseconds. */
  inferenceTimeoutMs?: number;
  /** Recurrent reset timeout in milliseconds. */
  resetTimeoutMs?: number;
  /** Visualization response timeout in milliseconds. */
  vizTimeoutMs?: number;
  /** Minimum interval between visualization dispatches. */
  vizMinIntervalMs?: number;
  /** Test-only worker module override used to prove bounded lifecycle failures. */
  workerUrlForTesting?: URL;
}

/** Immutable status reported by one initialized worker. */
export interface BrainPoolWorkerStatus {
  /** Stable zero-based worker identity. */
  workerId: number;
  /** Active pool lifecycle epoch. */
  poolEpoch: number;
  /** Active population-weight epoch. */
  weightEpoch: number;
  /** Backend captured by all brains owned by the worker. */
  activeBackend: InferenceBackend;
  /** Native addon identifier, or null for JS diagnostic workers. */
  nativeAddonBuildIdentifier: string | null;
  /** Number of population brains constructed by the worker. */
  ownedSlotCount: number;
}

/** Shared pending fields for operations awaiting multiple workers. */
interface PendingWorkers {
  /** Worker identities that have not acknowledged the operation. */
  remainingWorkerIds: Set<number>;
  /** Resolve callback for successful completion. */
  resolve: () => void;
  /** Reject callback for failure. */
  reject: (error: Error) => void;
  /** Timeout that bounds the operation. */
  timer: ReturnType<typeof setTimeout>;
}

/** In-flight inference dispatch. */
interface PendingBatch extends PendingWorkers {
  /** Monotonic batch identifier. */
  batchId: number;
  /** Pool epoch captured at dispatch. */
  poolEpoch: number;
  /** Weight epoch captured at dispatch. */
  weightEpoch: number;
  /** Number of current batch positions. */
  batchCount: number;
  /** Sum of entries evaluated by all workers. */
  processedCount: number;
}

/** In-flight recurrent reset. */
interface PendingReset extends PendingWorkers {
  /** Pool epoch being reset. */
  poolEpoch: number;
  /** New weight epoch being installed. */
  weightEpoch: number;
}

/** In-flight selected-brain visualization request. */
interface PendingVisualization {
  /** Monotonic request identifier. */
  requestId: number;
  /** Worker expected to answer. */
  workerId: number;
  /** Pool epoch captured by the request. */
  poolEpoch: number;
  /** Weight epoch captured by the request. */
  weightEpoch: number;
  /** Selected durable population slot. */
  populationSlot: number;
  /** Last committed simulation step associated with the request. */
  simulationStep: number;
  /** Resolve callback for a matching response. */
  resolve: (viz: VizData | null) => void;
  /** Reject callback for request failure. */
  reject: (error: Error) => void;
  /** Timeout that bounds the request. */
  timer: ReturnType<typeof setTimeout>;
}

/** Fully normalized pool timeout configuration. */
interface ResolvedRuntimeOptions {
  /** Initialization timeout in milliseconds. */
  initTimeoutMs: number;
  /** Inference timeout in milliseconds. */
  inferenceTimeoutMs: number;
  /** Recurrent reset timeout in milliseconds. */
  resetTimeoutMs: number;
  /** Visualization timeout in milliseconds. */
  vizTimeoutMs: number;
  /** Minimum interval between visualization dispatches. */
  vizMinIntervalMs: number;
  /** Optional test worker module replacing the production inference worker. */
  workerUrlForTesting: URL | null;
}

/**
 * Server-side canonical worker pool for population-owned brain inference.
 */
export class BrainPool implements BatchInferenceRunner {
  /** Immutable backend selected for every worker-owned brain. */
  readonly inferenceBackend: InferenceBackend;
  /** Requested worker count, where zero selects an automatic count. */
  private readonly requestedWorkerCount: number;
  /** Normalized operation timeouts and visualization pacing. */
  private readonly runtimeOptions: ResolvedRuntimeOptions;
  /** Current pool status. */
  status: PoolStatus = 'disabled';
  /** Resolved worker count for the current pool epoch. */
  workerCount = 0;
  /** Active worker threads, exposed for lifecycle diagnostics and fault tests. */
  workers: Worker[] = [];
  /** Stable graph key for the current pool epoch. */
  specKey: string | null = null;
  /** Dense population slot count for the current pool epoch. */
  populationCount = 0;
  /** Parameters in one population genome. */
  paramCount = 0;
  /** Input floats in one batch position. */
  inputStride = 0;
  /** Output floats in one batch position. */
  outputStride = 0;
  /** Maximum batch positions backed by shared buffers. */
  maxBatch = 0;
  /** Current pool lifecycle epoch. */
  poolEpoch: number | null = null;
  /** Current population-weight epoch. */
  weightEpoch: number | null = null;
  /** Last failure reason retained for diagnostics. */
  failureReason: string | null = null;
  /** Shared packed population weights. */
  weightsBuffer: SharedArrayBuffer | null = null;
  /** Shared current-batch inputs. */
  inputBuffer: SharedArrayBuffer | null = null;
  /** Shared current-batch outputs. */
  outputBuffer: SharedArrayBuffer | null = null;
  /** Shared current-batch population slots. */
  indexBuffer: SharedArrayBuffer | null = null;
  /** Float view over packed population weights. */
  weightsView: Float32Array | null = null;
  /** Float view over current-batch inputs. */
  inputView: Float32Array | null = null;
  /** Float view over current-batch outputs. */
  outputView: Float32Array | null = null;
  /** Unsigned integer view over current-batch population slots. */
  indexView: Uint32Array | null = null;
  /** Worker identity associated with each thread. */
  private workerIds = new Map<Worker, number>();
  /** Ready status reported by each worker. */
  private workerStatuses = new Map<number, BrainPoolWorkerStatus>();
  /** Next inference batch identifier. */
  private nextBatchId = 1;
  /** Next visualization request identifier. */
  private nextVizRequestId = 1;
  /** Current in-flight inference dispatch. */
  private inflight: PendingBatch | null = null;
  /** Current initialization acknowledgement set. */
  private pendingInit: PendingWorkers | null = null;
  /** Current recurrent reset acknowledgement set. */
  private pendingReset: PendingReset | null = null;
  /** Current selected-brain visualization request. */
  private pendingVisualization: PendingVisualization | null = null;
  /** Last accepted visualization snapshot. */
  private cachedVisualization: VizData | null = null;
  /** Wall-clock timestamp of the latest visualization dispatch. */
  private lastVizDispatchAt = Number.NEGATIVE_INFINITY;
  /** Reusable marks for duplicate-slot validation without hot-path allocation. */
  private slotValidationMarks = new Uint32Array(0);
  /** Current nonzero token stored in slot-validation marks. */
  private slotValidationToken = 0;
  /** Whether worker exits are expected during explicit shutdown. */
  private shutdownRequested = false;
  /** Whether the current failure has already been logged. */
  private didLogFailure = false;

  /**
   * Create a population brain pool.
   * @param requestedWorkerCount - Requested workers, or zero for automatic sizing.
   * @param inferenceBackend - Immutable backend prepared inside every worker.
   * @param runtimeOptions - Optional bounded-operation overrides.
   */
  constructor(
    requestedWorkerCount = 0,
    inferenceBackend: InferenceBackend = 'js',
    runtimeOptions: BrainPoolRuntimeOptions = {}
  ) {
    this.requestedWorkerCount = requestedWorkerCount;
    this.inferenceBackend = inferenceBackend;
    this.runtimeOptions = {
      initTimeoutMs: normalizeTimeout(runtimeOptions.initTimeoutMs, DEFAULT_INIT_TIMEOUT_MS),
      inferenceTimeoutMs: normalizeTimeout(
        runtimeOptions.inferenceTimeoutMs,
        DEFAULT_INFERENCE_TIMEOUT_MS
      ),
      resetTimeoutMs: normalizeTimeout(runtimeOptions.resetTimeoutMs, DEFAULT_RESET_TIMEOUT_MS),
      vizTimeoutMs: normalizeTimeout(runtimeOptions.vizTimeoutMs, DEFAULT_VIZ_TIMEOUT_MS),
      vizMinIntervalMs: normalizeNonNegativeDuration(
        runtimeOptions.vizMinIntervalMs,
        DEFAULT_VIZ_MIN_INTERVAL_MS
      ),
      workerUrlForTesting: runtimeOptions.workerUrlForTesting ?? null
    };
  }

  /**
   * Initialize a fresh immutable pool epoch and await every worker.
   * @param options - Graph, capacities, and packed initial population weights.
   */
  async init(options: BrainPoolInitOptions): Promise<void> {
    await this.shutdown();
    validateInitOptions(options);

    this.status = 'starting';
    this.failureReason = null;
    this.didLogFailure = false;
    this.shutdownRequested = false;
    this.specKey = options.specKey;
    this.populationCount = options.populationCount;
    this.paramCount = options.paramCount;
    this.inputStride = options.inputStride;
    this.outputStride = options.outputStride;
    this.maxBatch = options.maxBatch;
    this.workerCount = resolveWorkerCount(this.requestedWorkerCount, this.populationCount);
    this.poolEpoch = allocatePoolEpoch();
    this.weightEpoch = 1;
    this.nextBatchId = 1;
    this.nextVizRequestId = 1;
    this.slotValidationMarks = new Uint32Array(this.populationCount);
    this.slotValidationToken = 0;

    const weightFloats = checkedProduct(
      'population weight floats',
      this.populationCount,
      this.paramCount
    );
    const inputFloats = checkedProduct('input floats', this.maxBatch, this.inputStride);
    const outputFloats = checkedProduct('output floats', this.maxBatch, this.outputStride);
    this.weightsBuffer = allocateFloatBuffer('weights', weightFloats);
    this.inputBuffer = allocateFloatBuffer('inputs', inputFloats);
    this.outputBuffer = allocateFloatBuffer('outputs', outputFloats);
    this.indexBuffer = allocateUint32Buffer('indices', this.maxBatch);
    this.weightsView = new Float32Array(this.weightsBuffer);
    this.inputView = new Float32Array(this.inputBuffer);
    this.outputView = new Float32Array(this.outputBuffer);
    this.indexView = new Uint32Array(this.indexBuffer);
    this.weightsView.set(options.weights);

    const workerIds = Array.from({ length: this.workerCount }, (_unused, workerId) => workerId);
    const readyPromise = new Promise<void>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.failPool(new Error('mt pool init timeout'));
      }, this.runtimeOptions.initTimeoutMs);
      this.pendingInit = {
        remainingWorkerIds: new Set(workerIds),
        resolve,
        reject,
        timer
      };
    });

    const workerUrl =
      this.runtimeOptions.workerUrlForTesting ??
      new URL('./worker/inferWorker.ts', import.meta.url);
    try {
      for (const workerId of workerIds) {
        const worker = new Worker(workerUrl, {
          execArgv: ['--import', 'tsx/esm'],
          workerData: { inferenceBackend: this.inferenceBackend }
        });
        this.workers.push(worker);
        this.workerIds.set(worker, workerId);
        this.attachWorker(worker, workerId);
        const message: BrainPoolInitMessage = {
          type: 'init',
          workerId,
          workerCount: this.workerCount,
          poolEpoch: this.requirePoolEpoch(),
          weightEpoch: this.requireWeightEpoch(),
          inferenceBackend: this.inferenceBackend,
          spec: options.spec,
          specKey: options.specKey,
          populationCount: this.populationCount,
          paramCount: this.paramCount,
          inputStride: this.inputStride,
          outputStride: this.outputStride,
          maxBatch: this.maxBatch,
          buffers: {
            weights: this.weightsBuffer,
            inputs: this.inputBuffer,
            outputs: this.outputBuffer,
            indices: this.indexBuffer
          }
        };
        worker.postMessage(message);
      }
      await readyPromise;
      this.pendingInit = null;
      this.status = 'ready';
    } catch (error) {
      const normalized = asError(error);
      this.failPool(normalized);
      await this.terminateWorkers();
      throw normalized;
    }
  }

  /**
   * Return the count of workers that acknowledged the active pool epoch.
   * @returns Active worker count, or zero until the pool is ready.
   */
  getActiveWorkerCount(): number {
    return this.status === 'ready' ? this.workerStatuses.size : 0;
  }

  /**
   * Return detached worker readiness records for diagnostics.
   * @returns Worker statuses ordered by stable worker identity.
   */
  getWorkerStatuses(): BrainPoolWorkerStatus[] {
    return Array.from(this.workerStatuses.values())
      .sort((left, right) => left.workerId - right.workerId)
      .map((status) => ({ ...status }));
  }

  /**
   * Run one non-concurrent batch using stable population-slot ownership.
   * @param inputs - Packed inputs indexed by current batch position.
   * @param outputs - Destination outputs indexed by current batch position.
   * @param indices - Durable population slots indexed by current batch position.
   * @param count - Number of populated batch positions.
   * @param inputStride - Input float count per batch position.
   * @param outputStride - Output float count per batch position.
   */
  async runBatch(
    inputs: Float32Array,
    outputs: Float32Array,
    indices: Uint32Array,
    count: number,
    inputStride: number,
    outputStride: number
  ): Promise<void> {
    this.assertReady();
    if (this.inflight) {
      throw new Error('mt pool dispatch already in flight');
    }
    if (this.pendingReset) {
      throw new Error('mt pool recurrent reset is in flight');
    }
    if (inputStride !== this.inputStride || outputStride !== this.outputStride) {
      throw new Error('mt pool stride mismatch');
    }
    if (!Number.isSafeInteger(count) || count < 0 || count > this.maxBatch) {
      throw new Error(`mt pool invalid batch count ${count}`);
    }
    if (count === 0) return;

    const inputCount = checkedProduct('batch input floats', count, inputStride);
    const outputCount = checkedProduct('batch output floats', count, outputStride);
    if (inputs.length < inputCount || outputs.length < outputCount || indices.length < count) {
      throw new Error('mt pool batch buffers too small');
    }
    const inputView = this.requireInputView();
    const outputView = this.requireOutputView();
    const indexView = this.requireIndexView();
    if (inputView.length < inputCount || outputView.length < outputCount || indexView.length < count) {
      throw new Error('mt pool shared buffers too small');
    }
    this.validatePopulationSlots(indices, count);

    inputView.set(inputs.subarray(0, inputCount), 0);
    indexView.set(indices.subarray(0, count), 0);
    outputView.fill(0, 0, outputCount);

    const batchId = this.nextBatchId++;
    const poolEpoch = this.requirePoolEpoch();
    const weightEpoch = this.requireWeightEpoch();
    let resolveCompletion!: () => void;
    let rejectCompletion!: (error: Error) => void;
    const completion = new Promise<void>((resolve, reject) => {
      resolveCompletion = resolve;
      rejectCompletion = reject;
    });
    const pendingBatch: PendingBatch = {
      batchId,
      poolEpoch,
      weightEpoch,
      batchCount: count,
      processedCount: 0,
      remainingWorkerIds: new Set(this.workerStatuses.keys()),
      resolve: resolveCompletion,
      reject: rejectCompletion,
      timer: setTimeout(() => {
        this.failPool(new Error(`mt pool inference timeout for batch ${batchId}`));
      }, this.runtimeOptions.inferenceTimeoutMs)
    };
    this.inflight = pendingBatch;

    const message = {
      type: 'infer',
      poolEpoch,
      weightEpoch,
      batchId,
      batchCount: count
    } as const;
    try {
      for (const worker of this.workers) {
        worker.postMessage(message);
      }
    } catch (error) {
      this.failPool(asError(error));
    }

    try {
      await completion;
    } finally {
      if (this.inflight === pendingBatch) {
        clearTimeout(pendingBatch.timer);
        this.inflight = null;
      }
    }
    outputs.set(outputView.subarray(0, outputCount), 0);
  }

  /**
   * Copy a complete new population, advance the weight epoch, reset recurrent
   * state, and await every worker acknowledgement.
   * @param weights - Packed weights for every dense population slot.
   */
  async replacePopulationWeights(weights: Float32Array): Promise<void> {
    this.assertReady();
    if (this.inflight) {
      throw new Error('mt pool cannot replace weights during inference');
    }
    if (this.pendingReset) {
      throw new Error('mt pool recurrent reset already in flight');
    }
    const weightsView = this.requireWeightsView();
    if (weights.length !== weightsView.length) {
      throw new Error(
        `mt pool weights length mismatch: expected ${weightsView.length}, received ${weights.length}`
      );
    }
    this.cancelPendingVisualization();
    weightsView.set(weights);
    const poolEpoch = this.requirePoolEpoch();
    const weightEpoch = this.requireWeightEpoch() + 1;
    this.weightEpoch = weightEpoch;
    this.cachedVisualization = null;

    let resolveCompletion!: () => void;
    let rejectCompletion!: (error: Error) => void;
    const completion = new Promise<void>((resolve, reject) => {
      resolveCompletion = resolve;
      rejectCompletion = reject;
    });
    const pendingReset: PendingReset = {
      poolEpoch,
      weightEpoch,
      remainingWorkerIds: new Set(this.workerStatuses.keys()),
      resolve: resolveCompletion,
      reject: rejectCompletion,
      timer: setTimeout(() => {
        this.failPool(new Error(`mt pool reset timeout for weight epoch ${weightEpoch}`));
      }, this.runtimeOptions.resetTimeoutMs)
    };
    this.pendingReset = pendingReset;
    const message: BrainPoolResetMessage = {
      type: 'reset',
      poolEpoch,
      weightEpoch
    };
    try {
      for (const worker of this.workers) {
        worker.postMessage(message);
      }
    } catch (error) {
      this.failPool(asError(error));
    }

    try {
      await completion;
    } finally {
      if (this.pendingReset === pendingReset) {
        clearTimeout(pendingReset.timer);
        this.pendingReset = null;
      }
    }
  }

  /**
   * Request or reuse a rate-limited visualization for a worker-owned slot.
   * @param populationSlot - Durable population slot to visualize.
   * @param simulationStep - Last committed authoritative simulation step.
   * @returns Tagged visualization, cached data, or null while another request is pending.
   */
  async requestVisualization(
    populationSlot: number,
    simulationStep: number
  ): Promise<VizData | null> {
    this.assertReady();
    if (
      !Number.isSafeInteger(populationSlot) ||
      populationSlot < 0 ||
      populationSlot >= this.populationCount
    ) {
      throw new Error(`mt pool invalid visualization slot ${populationSlot}`);
    }
    const now = Date.now();
    const cacheMatches =
      this.cachedVisualization?.populationSlot === populationSlot &&
      this.cachedVisualization.poolEpoch === this.poolEpoch &&
      this.cachedVisualization.weightEpoch === this.weightEpoch;
    if (now - this.lastVizDispatchAt < this.runtimeOptions.vizMinIntervalMs) {
      return cacheMatches ? this.cachedVisualization : null;
    }
    if (this.pendingVisualization) {
      return cacheMatches ? this.cachedVisualization : null;
    }

    const requestId = this.nextVizRequestId++;
    const workerId = populationSlot % this.workerCount;
    const poolEpoch = this.requirePoolEpoch();
    const weightEpoch = this.requireWeightEpoch();
    const worker = this.workers[workerId];
    if (!worker) {
      throw new Error(`mt pool missing visualization owner worker ${workerId}`);
    }
    this.lastVizDispatchAt = now;

    const completion = new Promise<VizData | null>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.failPool(new Error(`mt pool visualization timeout for request ${requestId}`));
      }, this.runtimeOptions.vizTimeoutMs);
      this.pendingVisualization = {
        requestId,
        workerId,
        poolEpoch,
        weightEpoch,
        populationSlot,
        simulationStep,
        resolve,
        reject,
        timer
      };
    });
    const message: BrainPoolVizMessage = {
      type: 'viz',
      poolEpoch,
      weightEpoch,
      requestId,
      populationSlot,
      simulationStep
    };
    try {
      worker.postMessage(message);
    } catch (error) {
      this.failPool(asError(error));
    }
    return completion;
  }

  /**
   * Return the latest accepted tagged worker visualization.
   * @returns Detached cached payload, or null before a matching response.
   */
  getCachedVisualization(): VizData | null {
    return this.cachedVisualization;
  }

  /**
   * Shutdown all workers, reject pending work, and release shared buffers.
   */
  async shutdown(): Promise<void> {
    const shutdownError = new Error('mt pool shut down');
    this.rejectPending(shutdownError);
    await this.terminateWorkers();
    this.status = 'disabled';
    this.workerCount = 0;
    this.specKey = null;
    this.populationCount = 0;
    this.paramCount = 0;
    this.inputStride = 0;
    this.outputStride = 0;
    this.maxBatch = 0;
    this.poolEpoch = null;
    this.weightEpoch = null;
    this.failureReason = null;
    this.weightsBuffer = null;
    this.inputBuffer = null;
    this.outputBuffer = null;
    this.indexBuffer = null;
    this.weightsView = null;
    this.inputView = null;
    this.outputView = null;
    this.indexView = null;
    this.slotValidationMarks = new Uint32Array(0);
    this.slotValidationToken = 0;
    this.cachedVisualization = null;
    this.lastVizDispatchAt = Number.NEGATIVE_INFINITY;
  }

  /**
   * Attach lifecycle and protocol handlers to one worker.
   * @param worker - Worker thread to observe.
   * @param expectedWorkerId - Stable identity assigned at creation.
   */
  private attachWorker(worker: Worker, expectedWorkerId: number): void {
    worker.on('message', (message: BrainPoolWorkerResponse) => {
      this.handleWorkerMessage(expectedWorkerId, message);
    });
    worker.on('error', (error) => {
      if (this.shutdownRequested) return;
      this.failPool(error);
    });
    worker.on('exit', (code) => {
      if (this.shutdownRequested) return;
      this.failPool(new Error(`mt worker ${expectedWorkerId} exited unexpectedly with code ${code}`));
    });
  }

  /**
   * Route one worker response through epoch and operation validation.
   * @param expectedWorkerId - Identity associated with the emitting thread.
   * @param message - Typed worker response.
   */
  private handleWorkerMessage(
    expectedWorkerId: number,
    message: BrainPoolWorkerResponse
  ): void {
    if (!message || typeof message !== 'object') {
      this.failPool(new Error(`mt worker ${expectedWorkerId} sent an invalid response`));
      return;
    }
    if (message.workerId !== expectedWorkerId) {
      this.failPool(
        new Error(
          `mt worker identity mismatch: expected ${expectedWorkerId}, received ${message.workerId}`
        )
      );
      return;
    }
    if (message.type === 'error') {
      if (this.isStalePoolEpoch(message.poolEpoch)) return;
      this.failPool(
        new Error(`mt worker ${message.workerId} ${message.context} failed: ${message.reason}`)
      );
      return;
    }
    if (this.isStalePoolEpoch(message.poolEpoch)) return;
    if (message.poolEpoch !== this.poolEpoch) {
      this.failPool(
        new Error(
          `mt worker ${message.workerId} reported future pool epoch ${message.poolEpoch}`
        )
      );
      return;
    }

    switch (message.type) {
      case 'ready':
        this.handleReady(message);
        return;
      case 'done':
        this.handleDone(message);
        return;
      case 'resetDone':
        this.handleResetDone(message);
        return;
      case 'vizResult':
        this.handleVizResult(message);
        return;
      default:
        this.failPool(new Error(`mt worker ${expectedWorkerId} sent an unknown response`));
    }
  }

  /**
   * Validate and record one initialization acknowledgement.
   * @param message - Worker ready response.
   */
  private handleReady(message: BrainPoolReadyMessage): void {
    const pending = this.pendingInit;
    if (!pending || !pending.remainingWorkerIds.has(message.workerId)) {
      this.failPool(new Error(`mt worker ${message.workerId} sent an unexpected ready response`));
      return;
    }
    if (message.weightEpoch !== this.weightEpoch) {
      this.failPool(
        new Error(
          `mt worker ${message.workerId} ready weight epoch mismatch: ${message.weightEpoch}`
        )
      );
      return;
    }
    if (message.activeBackend !== this.inferenceBackend) {
      this.failPool(
        new Error(
          `mt worker ${message.workerId} backend mismatch: expected ${this.inferenceBackend}, received ${message.activeBackend}`
        )
      );
      return;
    }
    if (
      this.inferenceBackend === 'native' &&
      (!message.nativeAddonBuildIdentifier || message.nativeAddonBuildIdentifier.trim().length === 0)
    ) {
      this.failPool(new Error(`mt native worker ${message.workerId} did not report an addon build id`));
      return;
    }
    const expectedOwnedSlots = ownedSlotCount(
      message.workerId,
      this.workerCount,
      this.populationCount
    );
    if (message.ownedSlotCount !== expectedOwnedSlots) {
      this.failPool(
        new Error(
          `mt worker ${message.workerId} owned-slot mismatch: expected ${expectedOwnedSlots}, received ${message.ownedSlotCount}`
        )
      );
      return;
    }
    const nativeIds = new Set(
      Array.from(this.workerStatuses.values())
        .map((status) => status.nativeAddonBuildIdentifier)
        .filter((identifier): identifier is string => identifier !== null)
    );
    if (
      message.nativeAddonBuildIdentifier &&
      nativeIds.size > 0 &&
      !nativeIds.has(message.nativeAddonBuildIdentifier)
    ) {
      this.failPool(new Error('mt native workers loaded different addon builds'));
      return;
    }

    this.workerStatuses.set(message.workerId, {
      workerId: message.workerId,
      poolEpoch: message.poolEpoch,
      weightEpoch: message.weightEpoch,
      activeBackend: message.activeBackend,
      nativeAddonBuildIdentifier: message.nativeAddonBuildIdentifier,
      ownedSlotCount: message.ownedSlotCount
    });
    pending.remainingWorkerIds.delete(message.workerId);
    if (pending.remainingWorkerIds.size === 0) {
      clearTimeout(pending.timer);
      pending.resolve();
    }
  }

  /**
   * Validate one batch completion without allowing stale responses to finish it.
   * @param message - Worker batch completion.
   */
  private handleDone(message: BrainPoolDoneMessage): void {
    const pending = this.inflight;
    if (!pending) {
      if (message.batchId < this.nextBatchId) return;
      this.failPool(new Error(`mt worker reported future batch ${message.batchId}`));
      return;
    }
    if (message.batchId < pending.batchId) return;
    if (message.batchId !== pending.batchId) {
      this.failPool(
        new Error(
          `mt worker ${message.workerId} batch mismatch: expected ${pending.batchId}, received ${message.batchId}`
        )
      );
      return;
    }
    if (
      message.weightEpoch !== pending.weightEpoch ||
      message.batchCount !== pending.batchCount
    ) {
      this.failPool(new Error(`mt worker ${message.workerId} batch epoch or capacity mismatch`));
      return;
    }
    if (!pending.remainingWorkerIds.delete(message.workerId)) {
      this.failPool(new Error(`mt worker ${message.workerId} duplicated batch completion`));
      return;
    }
    pending.processedCount += message.processedCount;
    if (pending.remainingWorkerIds.size !== 0) return;
    if (pending.processedCount !== pending.batchCount) {
      this.failPool(
        new Error(
          `mt batch ownership mismatch: processed ${pending.processedCount} of ${pending.batchCount}`
        )
      );
      return;
    }
    clearTimeout(pending.timer);
    pending.resolve();
  }

  /**
   * Validate one recurrent-reset acknowledgement.
   * @param message - Worker reset completion.
   */
  private handleResetDone(message: BrainPoolResetDoneMessage): void {
    const pending = this.pendingReset;
    if (!pending) {
      if (message.weightEpoch < this.requireWeightEpoch()) return;
      this.failPool(
        new Error(`mt worker ${message.workerId} sent an unexpected reset acknowledgement`)
      );
      return;
    }
    if (message.weightEpoch < pending.weightEpoch) return;
    if (
      message.poolEpoch !== pending.poolEpoch ||
      message.weightEpoch !== pending.weightEpoch
    ) {
      this.failPool(new Error(`mt worker ${message.workerId} reset epoch mismatch`));
      return;
    }
    if (!pending.remainingWorkerIds.delete(message.workerId)) {
      this.failPool(new Error(`mt worker ${message.workerId} duplicated reset acknowledgement`));
      return;
    }
    const status = this.workerStatuses.get(message.workerId);
    if (status) status.weightEpoch = message.weightEpoch;
    if (pending.remainingWorkerIds.size === 0) {
      clearTimeout(pending.timer);
      pending.resolve();
    }
  }

  /**
   * Accept only the exact in-flight tagged visualization response.
   * @param message - Worker visualization response.
   */
  private handleVizResult(message: BrainPoolVizResultMessage): void {
    const pending = this.pendingVisualization;
    if (!pending) {
      if (message.requestId < this.nextVizRequestId) return;
      this.failPool(new Error(`mt worker reported future viz request ${message.requestId}`));
      return;
    }
    if (message.requestId < pending.requestId) return;
    if (
      message.requestId !== pending.requestId ||
      message.workerId !== pending.workerId ||
      message.poolEpoch !== pending.poolEpoch ||
      message.weightEpoch !== pending.weightEpoch ||
      message.populationSlot !== pending.populationSlot ||
      message.simulationStep !== pending.simulationStep
    ) {
      this.failPool(new Error(`mt worker ${message.workerId} visualization tag mismatch`));
      return;
    }
    clearTimeout(pending.timer);
    const tagged: VizData = {
      ...message.viz,
      populationSlot: message.populationSlot,
      simulationStep: message.simulationStep,
      poolEpoch: message.poolEpoch,
      weightEpoch: message.weightEpoch
    };
    this.cachedVisualization = tagged;
    this.pendingVisualization = null;
    pending.resolve(tagged);
  }

  /**
   * Reject all pending operations and mark the pool unusable.
   * @param error - Failure that invalidated the pool epoch.
   */
  private failPool(error: Error): void {
    if (this.status !== 'failed') {
      this.status = 'failed';
      this.failureReason = error.message;
    }
    this.rejectPending(error);
    if (!this.didLogFailure) {
      console.warn('[mt.pool.failed]', {
        poolEpoch: this.poolEpoch,
        weightEpoch: this.weightEpoch,
        reason: error.message
      });
      this.didLogFailure = true;
    }
  }

  /**
   * Reject and clear every pending operation.
   * @param error - Rejection delivered to each waiter.
   */
  private rejectPending(error: Error): void {
    if (this.inflight) {
      clearTimeout(this.inflight.timer);
      const pending = this.inflight;
      this.inflight = null;
      pending.reject(error);
    }
    if (this.pendingInit) {
      clearTimeout(this.pendingInit.timer);
      const pending = this.pendingInit;
      this.pendingInit = null;
      pending.reject(error);
    }
    if (this.pendingReset) {
      clearTimeout(this.pendingReset.timer);
      const pending = this.pendingReset;
      this.pendingReset = null;
      pending.reject(error);
    }
    if (this.pendingVisualization) {
      clearTimeout(this.pendingVisualization.timer);
      const pending = this.pendingVisualization;
      this.pendingVisualization = null;
      pending.reject(error);
    }
  }

  /**
   * Resolve a pending visualization as discarded without failing the pool.
   */
  private cancelPendingVisualization(): void {
    const pending = this.pendingVisualization;
    if (!pending) return;
    clearTimeout(pending.timer);
    this.pendingVisualization = null;
    pending.resolve(null);
  }

  /**
   * Terminate every worker while suppressing expected exit failures.
   */
  private async terminateWorkers(): Promise<void> {
    if (this.workers.length === 0) {
      this.workerIds.clear();
      this.workerStatuses.clear();
      return;
    }
    this.shutdownRequested = true;
    const message: BrainPoolShutdownMessage = { type: 'shutdown' };
    const workers = this.workers.slice();
    for (const worker of workers) {
      try {
        worker.postMessage(message);
      } catch {
        // Termination below remains the authoritative cleanup.
      }
    }
    await Promise.all(workers.map(async (worker) => {
      try {
        await worker.terminate();
      } catch {
        // Preserve the original pool failure while releasing every other worker.
      }
    }));
    this.workers = [];
    this.workerIds.clear();
    this.workerStatuses.clear();
    this.shutdownRequested = false;
  }

  /**
   * Reject an operation unless the exact pool epoch is ready.
   */
  private assertReady(): void {
    if (this.status !== 'ready') {
      const suffix = this.failureReason ? `: ${this.failureReason}` : '';
      throw new Error(`mt pool not ready${suffix}`);
    }
  }

  /**
   * Return the current pool epoch or throw when uninitialized.
   * @returns Installed pool epoch.
   */
  private requirePoolEpoch(): number {
    if (this.poolEpoch === null) throw new Error('mt pool epoch is not initialized');
    return this.poolEpoch;
  }

  /**
   * Return the current weight epoch or throw when uninitialized.
   * @returns Installed weight epoch.
   */
  private requireWeightEpoch(): number {
    if (this.weightEpoch === null) throw new Error('mt weight epoch is not initialized');
    return this.weightEpoch;
  }

  /**
   * Return the shared weights view or throw when uninitialized.
   * @returns Shared packed weight view.
   */
  private requireWeightsView(): Float32Array {
    if (!this.weightsView) throw new Error('mt pool weights are not initialized');
    return this.weightsView;
  }

  /**
   * Return the shared input view or throw when uninitialized.
   * @returns Shared input view.
   */
  private requireInputView(): Float32Array {
    if (!this.inputView) throw new Error('mt pool inputs are not initialized');
    return this.inputView;
  }

  /**
   * Return the shared output view or throw when uninitialized.
   * @returns Shared output view.
   */
  private requireOutputView(): Float32Array {
    if (!this.outputView) throw new Error('mt pool outputs are not initialized');
    return this.outputView;
  }

  /**
   * Return the shared slot-index view or throw when uninitialized.
   * @returns Shared population-slot view.
   */
  private requireIndexView(): Uint32Array {
    if (!this.indexView) throw new Error('mt pool indices are not initialized');
    return this.indexView;
  }

  /**
   * Determine whether a response belongs to a retired pool epoch.
   * @param messagePoolEpoch - Epoch reported by a worker.
   * @returns True only for a strictly older initialized epoch.
   */
  private isStalePoolEpoch(messagePoolEpoch: number | null): boolean {
    return (
      messagePoolEpoch !== null &&
      this.poolEpoch !== null &&
      messagePoolEpoch < this.poolEpoch
    );
  }

  /**
   * Validate range and uniqueness of every population slot in a batch.
   * @param indices - Durable population slots indexed by batch position.
   * @param count - Number of populated batch positions.
   */
  private validatePopulationSlots(indices: Uint32Array, count: number): void {
    this.slotValidationToken = (this.slotValidationToken + 1) >>> 0;
    if (this.slotValidationToken === 0) {
      this.slotValidationMarks.fill(0);
      this.slotValidationToken = 1;
    }
    const token = this.slotValidationToken;
    for (let batchPosition = 0; batchPosition < count; batchPosition++) {
      const populationSlot = indices[batchPosition] ?? this.populationCount;
      if (populationSlot >= this.populationCount) {
        throw new Error(`mt pool population slot ${populationSlot} is out of range`);
      }
      if (this.slotValidationMarks[populationSlot] === token) {
        throw new Error(`mt pool population slot ${populationSlot} appears twice in one batch`);
      }
      this.slotValidationMarks[populationSlot] = token;
    }
  }
}

/**
 * Normalize an optional positive timeout.
 * @param value - Optional caller-provided duration.
 * @param fallback - Default duration.
 * @returns Positive integer milliseconds.
 */
function normalizeTimeout(value: number | undefined, fallback: number): number {
  if (!Number.isFinite(value) || (value ?? 0) <= 0) return fallback;
  return Math.max(1, Math.floor(value as number));
}

/**
 * Normalize an optional non-negative duration.
 * @param value - Optional caller-provided duration.
 * @param fallback - Default duration.
 * @returns Non-negative integer milliseconds.
 */
function normalizeNonNegativeDuration(value: number | undefined, fallback: number): number {
  if (!Number.isFinite(value) || (value ?? -1) < 0) return fallback;
  return Math.max(0, Math.floor(value as number));
}

/**
 * Validate every immutable pool dimension before allocating shared memory.
 * @param options - Candidate initialization payload.
 */
function validateInitOptions(options: BrainPoolInitOptions): void {
  assertPositiveSafeInteger('populationCount', options.populationCount);
  assertPositiveSafeInteger('paramCount', options.paramCount);
  assertPositiveSafeInteger('inputStride', options.inputStride);
  assertPositiveSafeInteger('outputStride', options.outputStride);
  assertPositiveSafeInteger('maxBatch', options.maxBatch);
  if (options.maxBatch < options.populationCount) {
    throw new Error('mt pool maxBatch must cover every population slot');
  }
  const expectedWeights = checkedProduct(
    'population weight floats',
    options.populationCount,
    options.paramCount
  );
  if (options.weights.length !== expectedWeights) {
    throw new Error(
      `mt pool initial weights length mismatch: expected ${expectedWeights}, received ${options.weights.length}`
    );
  }
  checkedBufferBytes('weights', expectedWeights, Float32Array.BYTES_PER_ELEMENT);
  checkedBufferBytes(
    'inputs',
    checkedProduct('input floats', options.maxBatch, options.inputStride),
    Float32Array.BYTES_PER_ELEMENT
  );
  checkedBufferBytes(
    'outputs',
    checkedProduct('output floats', options.maxBatch, options.outputStride),
    Float32Array.BYTES_PER_ELEMENT
  );
  checkedBufferBytes('indices', options.maxBatch, Uint32Array.BYTES_PER_ELEMENT);
}

/**
 * Assert a finite positive safe integer dimension.
 * @param label - Dimension name used in errors.
 * @param value - Candidate dimension.
 */
function assertPositiveSafeInteger(label: string, value: number): void {
  if (!Number.isSafeInteger(value) || value <= 0) {
    throw new Error(`mt pool ${label} must be a positive safe integer`);
  }
}

/**
 * Multiply dimensions without accepting unsafe integer overflow.
 * @param label - Product name used in errors.
 * @param left - First factor.
 * @param right - Second factor.
 * @returns Safe integer product.
 */
function checkedProduct(label: string, left: number, right: number): number {
  const product = left * right;
  if (!Number.isSafeInteger(product) || product < 0) {
    throw new Error(`mt pool ${label} exceeds safe integer capacity`);
  }
  return product;
}

/**
 * Validate and return a typed-buffer byte length.
 * @param label - Buffer name used in errors.
 * @param elementCount - Number of typed elements.
 * @param bytesPerElement - Typed element width.
 * @returns Validated byte length.
 */
function checkedBufferBytes(
  label: string,
  elementCount: number,
  bytesPerElement: number
): number {
  const byteLength = checkedProduct(`${label} bytes`, elementCount, bytesPerElement);
  if (byteLength > MAX_SHARED_BUFFER_BYTES) {
    throw new Error(
      `mt pool ${label} buffer requires ${byteLength} bytes, above ${MAX_SHARED_BUFFER_BYTES}`
    );
  }
  return byteLength;
}

/**
 * Allocate a validated Float32 shared buffer.
 * @param label - Buffer name used in errors.
 * @param elementCount - Number of float elements.
 * @returns Allocated shared buffer.
 */
function allocateFloatBuffer(label: string, elementCount: number): SharedArrayBuffer {
  return new SharedArrayBuffer(
    checkedBufferBytes(label, elementCount, Float32Array.BYTES_PER_ELEMENT)
  );
}

/**
 * Allocate a validated Uint32 shared buffer.
 * @param label - Buffer name used in errors.
 * @param elementCount - Number of integer elements.
 * @returns Allocated shared buffer.
 */
function allocateUint32Buffer(label: string, elementCount: number): SharedArrayBuffer {
  return new SharedArrayBuffer(
    checkedBufferBytes(label, elementCount, Uint32Array.BYTES_PER_ELEMENT)
  );
}

/**
 * Resolve a worker count from request, CPU availability, population, and cap.
 * @param requested - Requested workers, or zero for automatic sizing.
 * @param populationCount - Dense population slots available for ownership.
 * @returns Worker count for the new immutable pool epoch.
 */
function resolveWorkerCount(requested: number, populationCount: number): number {
  const available = typeof os.availableParallelism === 'function'
    ? os.availableParallelism()
    : os.cpus().length;
  const cpuBound = Math.max(1, available - 1);
  const parsed = Number.isFinite(requested) ? Math.floor(requested) : 0;
  const desired = parsed > 0 ? parsed : cpuBound;
  return Math.max(1, Math.min(desired, cpuBound, populationCount, MAX_WORKER_COUNT));
}

/**
 * Allocate a positive process-local pool epoch.
 * @returns Monotonic lifecycle epoch.
 */
function allocatePoolEpoch(): number {
  const epoch = nextPoolEpoch++;
  if (!Number.isSafeInteger(epoch) || epoch <= 0) {
    throw new Error('mt pool epoch source exhausted');
  }
  return epoch;
}

/**
 * Count dense slots owned by one modulo-mapped worker.
 * @param workerId - Stable worker identity.
 * @param workerCount - Immutable pool worker count.
 * @param populationCount - Dense population slot count.
 * @returns Number of slots owned by the worker.
 */
function ownedSlotCount(
  workerId: number,
  workerCount: number,
  populationCount: number
): number {
  if (workerId >= populationCount) return 0;
  return Math.floor((populationCount - 1 - workerId) / workerCount) + 1;
}

/**
 * Normalize an unknown thrown value.
 * @param error - Unknown caught value.
 * @returns Error instance preserving the available message.
 */
function asError(error: unknown): Error {
  return error instanceof Error ? error : new Error(String(error));
}
