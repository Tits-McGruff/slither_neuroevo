/** Native SIMD kernel loader and runtime accessors. */

/** Load state for native kernels. */
export type SimdKernelStatus = 'unavailable' | 'loading' | 'ready' | 'failed';

/** Dense kernel interface exposed by the native bridge. */
export interface DenseKernel {
  forwardBatch: (
    weights: Float32Array,
    inputs: Float32Array,
    outputs: Float32Array,
    inSize: number,
    outSize: number,
    count: number,
    inputStride: number,
    outputStride: number
  ) => void;
}

/** MLP kernel interface exposed by the native bridge. */
export interface MlpKernel {
  forwardBatch: (
    weights: Float32Array,
    layerSizes: Int32Array,
    inputs: Float32Array,
    outputs: Float32Array,
    count: number,
    inputStride: number,
    outputStride: number
  ) => void;
}

/** GRU kernel interface exposed by the native bridge. */
export interface GruKernel {
  stepBatch: (
    weights: Float32Array,
    inputs: Float32Array,
    h: Float32Array,
    z: Float32Array,
    r: Float32Array,
    hPrev: Float32Array,
    inSize: number,
    hiddenSize: number,
    count: number,
    inputStride: number
  ) => void;
}

/** LSTM kernel interface exposed by the native bridge. */
export interface LstmKernel {
  stepBatch: (
    weights: Float32Array,
    inputs: Float32Array,
    h: Float32Array,
    c: Float32Array,
    hPrev: Float32Array,
    cPrev: Float32Array,
    inSize: number,
    hiddenSize: number,
    count: number,
    inputStride: number
  ) => void;
}

/** RRU kernel interface exposed by the native bridge. */
export interface RruKernel {
  stepBatch: (
    weights: Float32Array,
    inputs: Float32Array,
    h: Float32Array,
    hPrev: Float32Array,
    inSize: number,
    hiddenSize: number,
    count: number,
    inputStride: number
  ) => void;
}

const isNode = typeof process !== 'undefined' && !!process.versions?.node;
let simdStatus: SimdKernelStatus = 'unavailable';
let simdLoadPromise: Promise<void> | null = null;

let denseKernel: DenseKernel | null = null;
let mlpKernel: MlpKernel | null = null;
let gruKernel: GruKernel | null = null;
let lstmKernel: LstmKernel | null = null;
let rruKernel: RruKernel | null = null;

let nativeBinding: any | null = null;
let didLogNativeFailure = false;

function requireNativeBinding(): any {
  if (!nativeBinding) {
    throw new Error('Native kernels unavailable; call loadSimdKernels() first.');
  }
  return nativeBinding;
}

function buildDenseKernel(): DenseKernel {
  return {
    forwardBatch: (weights, inputs, outputs, inSize, outSize, count, inputStride, outputStride) => {
      const native = requireNativeBinding();
      const forward =
        native.denseForwardNative ??
        native.dense_forward_native;
      if (!forward) {
        throw new Error('Native dense kernel missing; expected denseForwardNative.');
      }
      forward(weights, inputs, outputs, inSize, outSize, count, inputStride, outputStride);
    }
  };
}

function buildMlpKernel(): MlpKernel {
  let scratch = new Float32Array(0);
  return {
    forwardBatch: (weights, layerSizes, inputs, outputs, count, inputStride, outputStride) => {
      const native = requireNativeBinding();
      let maxSize = 0;
      for (let i = 0; i < layerSizes.length; i++) {
        const size = layerSizes[i] ?? 0;
        if (size > maxSize) maxSize = size;
      }
      const required = Math.max(0, maxSize * 2);
      if (scratch.length < required) {
        scratch = new Float32Array(required);
      }
      const forward =
        native.mlpForwardNative ??
        native.mlp_forward_native;
      if (!forward) {
        throw new Error('Native MLP kernel missing; expected mlpForwardNative.');
      }
      forward(
        weights,
        layerSizes,
        inputs,
        outputs,
        layerSizes.length,
        count,
        inputStride,
        outputStride,
        scratch
      );
    }
  };
}

function buildGruKernel(): GruKernel {
  return {
    stepBatch: (weights, inputs, h, z, r, hPrev, inSize, hiddenSize, count, inputStride) => {
      const native = requireNativeBinding();
      const step =
        native.gruStepNative ??
        native.gru_step_native;
      if (!step) {
        throw new Error('Native GRU kernel missing; expected gruStepNative.');
      }
      step(weights, inputs, h, z, r, hPrev, inSize, hiddenSize, count, inputStride);
    }
  };
}

function buildLstmKernel(): LstmKernel {
  return {
    stepBatch: (weights, inputs, h, c, hPrev, cPrev, inSize, hiddenSize, count, inputStride) => {
      const native = requireNativeBinding();
      const step =
        native.lstmStepNative ??
        native.lstm_step_native;
      if (!step) {
        throw new Error('Native LSTM kernel missing; expected lstmStepNative.');
      }
      step(weights, inputs, h, c, hPrev, cPrev, inSize, hiddenSize, count, inputStride);
    }
  };
}

function buildRruKernel(): RruKernel {
  return {
    stepBatch: (weights, inputs, h, hPrev, inSize, hiddenSize, count, inputStride) => {
      const native = requireNativeBinding();
      const step =
        native.rruStepNative ??
        native.rru_step_native;
      if (!step) {
        throw new Error('Native RRU kernel missing; expected rruStepNative.');
      }
      step(weights, inputs, h, hPrev, inSize, hiddenSize, count, inputStride);
    }
  };
}

/**
 * Load native kernels when available.
 */
export async function loadSimdKernels(): Promise<void> {
  if (simdStatus === 'ready') return;
  if (simdLoadPromise) return simdLoadPromise;
  simdStatus = 'loading';
  simdLoadPromise = (async () => {
    try {
      if (!isNode) {
        throw new Error('Native kernels are only available in Node.js.');
      }
      // Avoid Vite dep-scan trying to resolve platform packages from native/index.js.
      nativeBinding = await import(/* @vite-ignore */ '../../native/index.js');
      denseKernel = buildDenseKernel();
      mlpKernel = buildMlpKernel();
      gruKernel = buildGruKernel();
      lstmKernel = buildLstmKernel();
      rruKernel = buildRruKernel();
      simdStatus = 'ready';
      didLogNativeFailure = false;
    } catch (err) {
      simdStatus = 'failed';
      denseKernel = null;
      mlpKernel = null;
      gruKernel = null;
      lstmKernel = null;
      rruKernel = null;
      nativeBinding = null;
      const message = err instanceof Error ? err.message : String(err);
      if (!didLogNativeFailure) {
        console.warn('[native] load failed', { reason: message });
        didLogNativeFailure = true;
      }
      throw err;
    } finally {
      simdLoadPromise = null;
    }
  })();
  return simdLoadPromise;
}

/**
 * Check whether native kernels are ready.
 */
export function isSimdAvailable(): boolean {
  return simdStatus === 'ready';
}

export function getDenseKernel(): DenseKernel | null {
  return denseKernel;
}

export function getMlpKernel(): MlpKernel | null {
  return mlpKernel;
}

export function getGruKernel(): GruKernel | null {
  return gruKernel;
}

export function getLstmKernel(): LstmKernel | null {
  return lstmKernel;
}

export function getRruKernel(): RruKernel | null {
  return rruKernel;
}

export function requireDenseKernel(): DenseKernel {
  if (!denseKernel) {
    throw new Error('Native Dense kernel unavailable; call loadSimdKernels() first.');
  }
  return denseKernel;
}

export function requireMlpKernel(): MlpKernel {
  if (!mlpKernel) {
    throw new Error('Native MLP kernel unavailable; call loadSimdKernels() first.');
  }
  return mlpKernel;
}

export function requireGruKernel(): GruKernel {
  if (!gruKernel) {
    throw new Error('Native GRU kernel unavailable; call loadSimdKernels() first.');
  }
  return gruKernel;
}

export function requireLstmKernel(): LstmKernel {
  if (!lstmKernel) {
    throw new Error('Native LSTM kernel unavailable; call loadSimdKernels() first.');
  }
  return lstmKernel;
}

export function requireRruKernel(): RruKernel {
  if (!rruKernel) {
    throw new Error('Native RRU kernel unavailable; call loadSimdKernels() first.');
  }
  return rruKernel;
}
