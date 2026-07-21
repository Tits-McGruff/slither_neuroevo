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

/** Dense kernel forward signature exposed by the native binding. */
type NativeDenseForward = (
  weights: Float32Array,
  inputs: Float32Array,
  outputs: Float32Array,
  inSize: number,
  outSize: number,
  count: number,
  inputStride: number,
  outputStride: number
) => void;

/** MLP kernel forward signature exposed by the native binding. */
type NativeMlpForward = (
  weights: Float32Array,
  layerSizes: Int32Array,
  inputs: Float32Array,
  outputs: Float32Array,
  layerCount: number,
  count: number,
  inputStride: number,
  outputStride: number,
  scratch: Float32Array
) => void;

/** GRU kernel step signature exposed by the native binding. */
type NativeGruStep = (
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

/** LSTM kernel step signature exposed by the native binding. */
type NativeLstmStep = (
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

/** RRU kernel step signature exposed by the native binding. */
type NativeRruStep = (
  weights: Float32Array,
  inputs: Float32Array,
  h: Float32Array,
  hPrev: Float32Array,
  inSize: number,
  hiddenSize: number,
  count: number,
  inputStride: number
) => void;

/** Subset of native exports used by the SIMD bridge. */
interface NativeBinding {
  denseForwardNative?: NativeDenseForward;
  dense_forward_native?: NativeDenseForward;
  mlpForwardNative?: NativeMlpForward;
  mlp_forward_native?: NativeMlpForward;
  gruStepNative?: NativeGruStep;
  gru_step_native?: NativeGruStep;
  lstmStepNative?: NativeLstmStep;
  lstm_step_native?: NativeLstmStep;
  rruStepNative?: NativeRruStep;
  rru_step_native?: NativeRruStep;
}

/** True when executing in a Node.js runtime. */
const isNode = typeof process !== 'undefined' && !!process.versions?.node;
/** Current load status for native SIMD kernels. */
let simdStatus: SimdKernelStatus = 'unavailable';
/** Shared promise for in-flight native loads. */
let simdLoadPromise: Promise<void> | null = null;

/** Loaded dense kernel accessor. */
let denseKernel: DenseKernel | null = null;
/** Loaded MLP kernel accessor. */
let mlpKernel: MlpKernel | null = null;
/** Loaded GRU kernel accessor. */
let gruKernel: GruKernel | null = null;
/** Loaded LSTM kernel accessor. */
let lstmKernel: LstmKernel | null = null;
/** Loaded RRU kernel accessor. */
let rruKernel: RruKernel | null = null;

/** Cached native binding module. */
let nativeBinding: NativeBinding | null = null;
/** Tracks whether we already warned about native load failures. */
let didLogNativeFailure = false;

/**
 * Get the native binding or throw if it has not been loaded.
 * @returns Loaded native binding.
 */
function requireNativeBinding(): NativeBinding {
  if (!nativeBinding) {
    throw new Error('Native kernels unavailable; call loadSimdKernels() first.');
  }
  return nativeBinding;
}

/**
 * Build the dense kernel wrapper from the native binding.
 * @returns Dense kernel adapter.
 */
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

/**
 * Build the MLP kernel wrapper from the native binding.
 * @returns MLP kernel adapter.
 */
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

/**
 * Build the GRU kernel wrapper from the native binding.
 * @returns GRU kernel adapter.
 */
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

/**
 * Build the LSTM kernel wrapper from the native binding.
 * @returns LSTM kernel adapter.
 */
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

/**
 * Build the RRU kernel wrapper from the native binding.
 * @returns RRU kernel adapter.
 */
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
      const { createRequire } = await import(/* @vite-ignore */ 'node:module');
      const require = createRequire(import.meta.url);
      const loaded = require('../../native/index.js') as { default?: NativeBinding } | NativeBinding;
      nativeBinding = (loaded as { default?: NativeBinding }).default ?? (loaded as NativeBinding);
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

/**
 * Return the current native-kernel load state without attempting a load.
 * @returns Current native-kernel load state.
 */
export function getSimdKernelStatus(): SimdKernelStatus {
  return simdStatus;
}

/**
 * Return the identifier exported by the loaded native addon.
 * @returns Null because the current addon does not export a build identifier.
 */
export function getNativeAddonBuildIdentifier(): string | null {
  return null;
}

/**
 * Get the dense kernel wrapper if loaded.
 * @returns Dense kernel instance or null.
 */
export function getDenseKernel(): DenseKernel | null {
  return denseKernel;
}

/**
 * Get the MLP kernel wrapper if loaded.
 * @returns MLP kernel instance or null.
 */
export function getMlpKernel(): MlpKernel | null {
  return mlpKernel;
}

/**
 * Get the GRU kernel wrapper if loaded.
 * @returns GRU kernel instance or null.
 */
export function getGruKernel(): GruKernel | null {
  return gruKernel;
}

/**
 * Get the LSTM kernel wrapper if loaded.
 * @returns LSTM kernel instance or null.
 */
export function getLstmKernel(): LstmKernel | null {
  return lstmKernel;
}

/**
 * Get the RRU kernel wrapper if loaded.
 * @returns RRU kernel instance or null.
 */
export function getRruKernel(): RruKernel | null {
  return rruKernel;
}

/**
 * Require a loaded dense kernel wrapper.
 * @returns Dense kernel instance.
 */
export function requireDenseKernel(): DenseKernel {
  if (!denseKernel) {
    throw new Error('Native Dense kernel unavailable; call loadSimdKernels() first.');
  }
  return denseKernel;
}

/**
 * Require a loaded MLP kernel wrapper.
 * @returns MLP kernel instance.
 */
export function requireMlpKernel(): MlpKernel {
  if (!mlpKernel) {
    throw new Error('Native MLP kernel unavailable; call loadSimdKernels() first.');
  }
  return mlpKernel;
}

/**
 * Require a loaded GRU kernel wrapper.
 * @returns GRU kernel instance.
 */
export function requireGruKernel(): GruKernel {
  if (!gruKernel) {
    throw new Error('Native GRU kernel unavailable; call loadSimdKernels() first.');
  }
  return gruKernel;
}

/**
 * Require a loaded LSTM kernel wrapper.
 * @returns LSTM kernel instance.
 */
export function requireLstmKernel(): LstmKernel {
  if (!lstmKernel) {
    throw new Error('Native LSTM kernel unavailable; call loadSimdKernels() first.');
  }
  return lstmKernel;
}

/**
 * Require a loaded RRU kernel wrapper.
 * @returns RRU kernel instance.
 */
export function requireRruKernel(): RruKernel {
  if (!rruKernel) {
    throw new Error('Native RRU kernel unavailable; call loadSimdKernels() first.');
  }
  return rruKernel;
}
