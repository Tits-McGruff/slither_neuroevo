/** WASM SIMD kernel loader and runtime accessors. */

/** Path to the bundled SIMD wasm asset. */
const SIMD_WASM_ASSET = './wasm/brains_simd.wasm';
/** Bytes per 32-bit float. */
const BYTES_PER_F32 = 4;
/** Bytes per 32-bit integer. */
const BYTES_PER_I32 = 4;
/** WebAssembly memory page size in bytes. */
const WASM_PAGE_SIZE = 65536;

/** Load state for SIMD kernels. */
export type SimdKernelStatus = 'unavailable' | 'loading' | 'ready' | 'failed';

/** WASM exports required for SIMD kernels. */
interface SimdWasmExports {
  /** Linear memory export for SIMD kernels. */
  memory: WebAssembly.Memory;
  /** Dense forward kernel export. */
  dense_forward: (
    weightsPtr: number,
    inputPtr: number,
    outputPtr: number,
    inSize: number,
    outSize: number,
    batchCount: number,
    inputStride: number,
    outputStride: number
  ) => void;
  /** MLP forward kernel export. */
  mlp_forward: (
    weightsPtr: number,
    layerSizesPtr: number,
    inputPtr: number,
    outputPtr: number,
    layerCount: number,
    batchCount: number,
    inputStride: number,
    outputStride: number,
    scratchPtr: number,
    scratchLen: number
  ) => void;
  /** GRU step kernel export. */
  gru_step: (
    weightsPtr: number,
    inputPtr: number,
    hPtr: number,
    zPtr: number,
    rPtr: number,
    hPrevPtr: number,
    inSize: number,
    hiddenSize: number,
    batchCount: number,
    inputStride: number
  ) => void;
  /** LSTM step kernel export. */
  lstm_step: (
    weightsPtr: number,
    inputPtr: number,
    hPtr: number,
    cPtr: number,
    hPrevPtr: number,
    cPrevPtr: number,
    inSize: number,
    hiddenSize: number,
    batchCount: number,
    inputStride: number
  ) => void;
  /** RRU step kernel export. */
  rru_step: (
    weightsPtr: number,
    inputPtr: number,
    hPtr: number,
    hPrevPtr: number,
    inSize: number,
    hiddenSize: number,
    batchCount: number,
    inputStride: number
  ) => void;
  /** Optional heap base global for JS-side allocation. */
  __heap_base?: WebAssembly.Global;
}

/** Simple WASM heap allocator reused across kernel calls. */
interface WasmAllocator {
  /** Reset the allocator to the heap base. */
  reset: () => void;
  /**
   * Allocate a byte range in wasm memory.
   * @param bytes - Size in bytes to reserve.
   * @returns Pointer to the start of the allocation.
   */
  alloc: (bytes: number) => number;
}

/** Dense kernel interface exposed by the WASM bridge. */
export interface DenseKernel {
  /**
   * Execute a batched dense forward pass.
   * @param weights - Packed weight buffer.
   * @param inputs - Packed input buffer.
   * @param outputs - Packed output buffer.
   * @param inSize - Input size per batch entry.
   * @param outSize - Output size per batch entry.
   * @param count - Number of batch entries.
   * @param inputStride - Stride between batch inputs.
   * @param outputStride - Stride between batch outputs.
   */
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

/** MLP kernel interface exposed by the WASM bridge. */
export interface MlpKernel {
  /**
   * Execute a batched MLP forward pass.
   * @param weights - Packed weight buffer.
   * @param layerSizes - Layer sizes for the MLP.
   * @param inputs - Packed input buffer.
   * @param outputs - Packed output buffer.
   * @param count - Number of batch entries.
   * @param inputStride - Stride between batch inputs.
   * @param outputStride - Stride between batch outputs.
   */
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

/** GRU kernel interface exposed by the WASM bridge. */
export interface GruKernel {
  /**
   * Execute a batched GRU step.
   * @param weights - Packed weight buffer.
   * @param inputs - Packed input buffer.
   * @param h - Hidden state buffer (updated in-place).
   * @param z - Update gate scratch buffer.
   * @param r - Reset gate scratch buffer.
   * @param hPrev - Previous hidden state scratch buffer.
   * @param inSize - Input size per batch entry.
   * @param hiddenSize - Hidden size per batch entry.
   * @param count - Number of batch entries.
   * @param inputStride - Stride between batch inputs.
   */
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

/** LSTM kernel interface exposed by the WASM bridge. */
export interface LstmKernel {
  /**
   * Execute a batched LSTM step.
   * @param weights - Packed weight buffer.
   * @param inputs - Packed input buffer.
   * @param h - Hidden state buffer (updated in-place).
   * @param c - Cell state buffer (updated in-place).
   * @param hPrev - Previous hidden state scratch buffer.
   * @param cPrev - Previous cell state scratch buffer.
   * @param inSize - Input size per batch entry.
   * @param hiddenSize - Hidden size per batch entry.
   * @param count - Number of batch entries.
   * @param inputStride - Stride between batch inputs.
   */
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

/** RRU kernel interface exposed by the WASM bridge. */
export interface RruKernel {
  /**
   * Execute a batched RRU step.
   * @param weights - Packed weight buffer.
   * @param inputs - Packed input buffer.
   * @param h - Hidden state buffer (updated in-place).
   * @param hPrev - Previous hidden state scratch buffer.
   * @param inSize - Input size per batch entry.
   * @param hiddenSize - Hidden size per batch entry.
   * @param count - Number of batch entries.
   * @param inputStride - Stride between batch inputs.
   */
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

/** Current SIMD kernel availability state. */
let simdStatus: SimdKernelStatus = 'unavailable';
/** Cached dense kernel when SIMD is ready. */
let denseKernel: DenseKernel | null = null;
/** Cached MLP kernel when SIMD is ready. */
let mlpKernel: MlpKernel | null = null;
/** Cached GRU kernel when SIMD is ready. */
let gruKernel: GruKernel | null = null;
/** Cached LSTM kernel when SIMD is ready. */
let lstmKernel: LstmKernel | null = null;
/** Cached RRU kernel when SIMD is ready. */
let rruKernel: RruKernel | null = null;
/** Whether a SIMD failure has been logged. */
let didLogSimdFailure = false;
/** Shared load promise for concurrent kernel loading. */
let simdLoadPromise: Promise<void> | null = null;

/**
 * Return true when running in a Node.js runtime.
 * @returns True for Node.js runtime contexts.
 */
function isNodeRuntime(): boolean {
  return typeof process !== 'undefined' && !!process.versions?.node;
}

/**
 * Resolve the SIMD wasm asset URL.
 * @returns URL for the wasm asset.
 */
function resolveSimdWasmUrl(): URL {
  return new URL(SIMD_WASM_ASSET, import.meta.url);
}

/**
 * Read wasm bytes from disk or fetch them in the browser.
 * @param url - URL to the wasm asset.
 * @returns BufferSource containing the wasm bytes.
 */
async function loadWasmBytes(url: URL): Promise<BufferSource> {
  if (typeof WebAssembly === 'undefined') {
    throw new Error('WebAssembly unavailable');
  }
  if (isNodeRuntime()) {
    const { readFile } = await import('node:fs/promises');
    const { fileURLToPath } = await import('node:url');
    return readFile(fileURLToPath(url));
  }
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`Failed to fetch wasm: ${response.status} ${response.statusText}`);
  }
  return response.arrayBuffer();
}

/**
 * Instantiate the SIMD wasm module and validate exports.
 * @returns Validated wasm exports for SIMD kernels.
 */
async function instantiateSimdWasm(): Promise<SimdWasmExports> {
  const url = resolveSimdWasmUrl();
  const bytes = await loadWasmBytes(url);
  const result = await WebAssembly.instantiate(bytes, {});
  const exports = result.instance.exports as Partial<SimdWasmExports>;
  if (!exports || !(exports.memory instanceof WebAssembly.Memory)) {
    throw new Error('WASM memory export missing');
  }
  if (typeof exports.dense_forward !== 'function') {
    throw new Error('dense_forward export missing');
  }
  if (typeof exports.mlp_forward !== 'function') {
    throw new Error('mlp_forward export missing');
  }
  if (typeof exports.gru_step !== 'function') {
    throw new Error('gru_step export missing');
  }
  if (typeof exports.lstm_step !== 'function') {
    throw new Error('lstm_step export missing');
  }
  if (typeof exports.rru_step !== 'function') {
    throw new Error('rru_step export missing');
  }
  return exports as SimdWasmExports;
}

/**
 * Fetch the wasm heap base from exports when available.
 * @param exports - WASM exports.
 * @returns Heap base offset in bytes.
 */
function getHeapBase(exports: SimdWasmExports): number {
  const heapBase = exports.__heap_base;
  if (heapBase && typeof heapBase.value === 'number') return heapBase.value;
  return 0;
}

/**
 * Ensure wasm memory is large enough for a requested size.
 * @param memory - WASM linear memory.
 * @param requiredBytes - Required byte length.
 */
function ensureWasmMemory(memory: WebAssembly.Memory, requiredBytes: number): void {
  const currentBytes = memory.buffer.byteLength;
  if (requiredBytes <= currentBytes) return;
  const missing = requiredBytes - currentBytes;
  const pages = Math.ceil(missing / WASM_PAGE_SIZE);
  memory.grow(pages);
}

/**
 * Create a simple bump allocator for wasm memory.
 * @param exports - WASM exports.
 * @returns Allocator for wasm memory.
 */
function createWasmAllocator(exports: SimdWasmExports): WasmAllocator {
  const base = getHeapBase(exports);
  let offset = base;
  return {
    reset: () => {
      offset = base;
    },
    alloc: (bytes: number) => {
      const aligned = (bytes + 15) & ~15;
      const ptr = offset;
      const next = offset + aligned;
      ensureWasmMemory(exports.memory, next);
      offset = next;
      return ptr;
    }
  };
}

/**
 * Copy Float32 data into wasm memory.
 * @param memory - WASM linear memory.
 * @param ptr - Destination pointer.
 * @param data - Source data buffer.
 */
function writeFloat32(memory: WebAssembly.Memory, ptr: number, data: Float32Array): void {
  const view = new Float32Array(memory.buffer, ptr, data.length);
  view.set(data);
}

/**
 * Copy Int32 data into wasm memory.
 * @param memory - WASM linear memory.
 * @param ptr - Destination pointer.
 * @param data - Source data buffer.
 */
function writeInt32(memory: WebAssembly.Memory, ptr: number, data: Int32Array): void {
  const view = new Int32Array(memory.buffer, ptr, data.length);
  view.set(data);
}

/**
 * Copy Float32 data from wasm memory into a JS buffer.
 * @param memory - WASM linear memory.
 * @param ptr - Source pointer.
 * @param out - Destination buffer.
 * @param length - Number of floats to copy.
 */
function readFloat32(memory: WebAssembly.Memory, ptr: number, out: Float32Array, length: number): void {
  const view = new Float32Array(memory.buffer, ptr, length);
  out.set(view.subarray(0, length));
}

/**
 * Build a dense kernel adapter from wasm exports.
 * @param exports - WASM exports.
 * @param heap - Shared wasm allocator.
 * @returns Dense kernel adapter.
 */
function buildDenseKernel(exports: SimdWasmExports, heap: WasmAllocator): DenseKernel {
  return {
    forwardBatch: (
      weights,
      inputs,
      outputs,
      inSize,
      outSize,
      count,
      inputStride,
      outputStride
    ) => {
      const safeCount = Math.max(0, Math.floor(count));
      const safeInSize = Math.max(0, Math.floor(inSize));
      const safeOutSize = Math.max(0, Math.floor(outSize));
      const safeInputStride = Math.max(0, Math.floor(inputStride));
      const safeOutputStride = Math.max(0, Math.floor(outputStride));
      if (safeCount === 0) return;
      const inputCount = safeCount * safeInputStride;
      const outputCount = safeCount * safeOutputStride;
      if (inputs.length < inputCount) {
        throw new Error('dense_forward input buffer too small');
      }
      if (outputs.length < outputCount) {
        throw new Error('dense_forward output buffer too small');
      }
      heap.reset();
      const weightsPtr = heap.alloc(weights.length * BYTES_PER_F32);
      const inputPtr = heap.alloc(inputCount * BYTES_PER_F32);
      const outputPtr = heap.alloc(outputCount * BYTES_PER_F32);
      writeFloat32(exports.memory, weightsPtr, weights);
      writeFloat32(exports.memory, inputPtr, inputs.subarray(0, inputCount));
      const outputView = new Float32Array(exports.memory.buffer, outputPtr, outputCount);
      outputView.fill(0);
      exports.dense_forward(
        weightsPtr,
        inputPtr,
        outputPtr,
        safeInSize,
        safeOutSize,
        safeCount,
        safeInputStride,
        safeOutputStride
      );
      readFloat32(exports.memory, outputPtr, outputs, outputCount);
      if (safeOutputStride > safeOutSize) {
        for (let b = 0; b < safeCount; b++) {
          const base = b * safeOutputStride;
          for (let o = safeOutSize; o < safeOutputStride; o++) {
            outputs[base + o] = 0;
          }
        }
      }
    }
  };
}

/**
 * Build an MLP kernel adapter from wasm exports.
 * @param exports - WASM exports.
 * @param heap - Shared wasm allocator.
 * @returns MLP kernel adapter.
 */
function buildMlpKernel(exports: SimdWasmExports, heap: WasmAllocator): MlpKernel {
  return {
    forwardBatch: (
      weights,
      layerSizes,
      inputs,
      outputs,
      count,
      inputStride,
      outputStride
    ) => {
      const safeCount = Math.max(0, Math.floor(count));
      const safeInputStride = Math.max(0, Math.floor(inputStride));
      const safeOutputStride = Math.max(0, Math.floor(outputStride));
      if (safeCount === 0) return;
      const inputCount = safeCount * safeInputStride;
      const outputCount = safeCount * safeOutputStride;
      const layerCount = Math.max(0, Math.floor(layerSizes.length));
      let maxSize = 0;
      for (let i = 0; i < layerCount; i++) {
        const size = layerSizes[i] ?? 0;
        if (size > maxSize) maxSize = size;
      }
      if (inputs.length < inputCount) {
        throw new Error('mlp_forward input buffer too small');
      }
      if (outputs.length < outputCount) {
        throw new Error('mlp_forward output buffer too small');
      }
      if (layerCount < 2 || maxSize <= 0) {
        outputs.fill(0, 0, outputCount);
        return;
      }
      heap.reset();
      const weightsPtr = heap.alloc(weights.length * BYTES_PER_F32);
      const layerSizesPtr = heap.alloc(layerCount * BYTES_PER_I32);
      const inputPtr = heap.alloc(inputCount * BYTES_PER_F32);
      const outputPtr = heap.alloc(outputCount * BYTES_PER_F32);
      const scratchLen = maxSize * 2;
      const scratchPtr = heap.alloc(scratchLen * BYTES_PER_F32);
      writeFloat32(exports.memory, weightsPtr, weights);
      writeInt32(exports.memory, layerSizesPtr, layerSizes);
      writeFloat32(exports.memory, inputPtr, inputs.subarray(0, inputCount));
      const outputView = new Float32Array(exports.memory.buffer, outputPtr, outputCount);
      outputView.fill(0);
      exports.mlp_forward(
        weightsPtr,
        layerSizesPtr,
        inputPtr,
        outputPtr,
        layerCount,
        safeCount,
        safeInputStride,
        safeOutputStride,
        scratchPtr,
        scratchLen
      );
      readFloat32(exports.memory, outputPtr, outputs, outputCount);
      const outSize = Math.max(0, layerSizes[layerCount - 1] ?? 0);
      if (safeOutputStride > outSize) {
        for (let b = 0; b < safeCount; b++) {
          const base = b * safeOutputStride;
          for (let o = outSize; o < safeOutputStride; o++) {
            outputs[base + o] = 0;
          }
        }
      }
    }
  };
}

/**
 * Build a GRU kernel adapter from wasm exports.
 * @param exports - WASM exports.
 * @param heap - Shared wasm allocator.
 * @returns GRU kernel adapter.
 */
function buildGruKernel(exports: SimdWasmExports, heap: WasmAllocator): GruKernel {
  return {
    stepBatch: (
      weights,
      inputs,
      h,
      z,
      r,
      hPrev,
      inSize,
      hiddenSize,
      count,
      inputStride
    ) => {
      const safeCount = Math.max(0, Math.floor(count));
      const safeInSize = Math.max(0, Math.floor(inSize));
      const safeHidden = Math.max(0, Math.floor(hiddenSize));
      const safeInputStride = Math.max(0, Math.floor(inputStride));
      if (safeCount === 0 || safeInSize === 0 || safeHidden === 0) return;
      const inputCount = safeCount * safeInputStride;
      const stateCount = safeCount * safeHidden;
      if (inputs.length < inputCount) {
        throw new Error('gru_step input buffer too small');
      }
      if (h.length < stateCount) {
        throw new Error('gru_step hidden buffer too small');
      }
      if (z.length < stateCount || r.length < stateCount || hPrev.length < stateCount) {
        throw new Error('gru_step scratch buffer too small');
      }
      heap.reset();
      const weightsPtr = heap.alloc(weights.length * BYTES_PER_F32);
      const inputPtr = heap.alloc(inputCount * BYTES_PER_F32);
      const hPtr = heap.alloc(stateCount * BYTES_PER_F32);
      const zPtr = heap.alloc(stateCount * BYTES_PER_F32);
      const rPtr = heap.alloc(stateCount * BYTES_PER_F32);
      const hPrevPtr = heap.alloc(stateCount * BYTES_PER_F32);
      writeFloat32(exports.memory, weightsPtr, weights);
      writeFloat32(exports.memory, inputPtr, inputs.subarray(0, inputCount));
      writeFloat32(exports.memory, hPtr, h.subarray(0, stateCount));
      writeFloat32(exports.memory, zPtr, z.subarray(0, stateCount));
      writeFloat32(exports.memory, rPtr, r.subarray(0, stateCount));
      writeFloat32(exports.memory, hPrevPtr, hPrev.subarray(0, stateCount));
      exports.gru_step(
        weightsPtr,
        inputPtr,
        hPtr,
        zPtr,
        rPtr,
        hPrevPtr,
        safeInSize,
        safeHidden,
        safeCount,
        safeInputStride
      );
      readFloat32(exports.memory, hPtr, h, stateCount);
      readFloat32(exports.memory, zPtr, z, stateCount);
      readFloat32(exports.memory, rPtr, r, stateCount);
      readFloat32(exports.memory, hPrevPtr, hPrev, stateCount);
    }
  };
}

/**
 * Build an LSTM kernel adapter from wasm exports.
 * @param exports - WASM exports.
 * @param heap - Shared wasm allocator.
 * @returns LSTM kernel adapter.
 */
function buildLstmKernel(exports: SimdWasmExports, heap: WasmAllocator): LstmKernel {
  return {
    stepBatch: (
      weights,
      inputs,
      h,
      c,
      hPrev,
      cPrev,
      inSize,
      hiddenSize,
      count,
      inputStride
    ) => {
      const safeCount = Math.max(0, Math.floor(count));
      const safeInSize = Math.max(0, Math.floor(inSize));
      const safeHidden = Math.max(0, Math.floor(hiddenSize));
      const safeInputStride = Math.max(0, Math.floor(inputStride));
      if (safeCount === 0 || safeInSize === 0 || safeHidden === 0) return;
      const inputCount = safeCount * safeInputStride;
      const stateCount = safeCount * safeHidden;
      if (inputs.length < inputCount) {
        throw new Error('lstm_step input buffer too small');
      }
      if (h.length < stateCount || c.length < stateCount) {
        throw new Error('lstm_step state buffer too small');
      }
      if (hPrev.length < stateCount || cPrev.length < stateCount) {
        throw new Error('lstm_step scratch buffer too small');
      }
      heap.reset();
      const weightsPtr = heap.alloc(weights.length * BYTES_PER_F32);
      const inputPtr = heap.alloc(inputCount * BYTES_PER_F32);
      const hPtr = heap.alloc(stateCount * BYTES_PER_F32);
      const cPtr = heap.alloc(stateCount * BYTES_PER_F32);
      const hPrevPtr = heap.alloc(stateCount * BYTES_PER_F32);
      const cPrevPtr = heap.alloc(stateCount * BYTES_PER_F32);
      writeFloat32(exports.memory, weightsPtr, weights);
      writeFloat32(exports.memory, inputPtr, inputs.subarray(0, inputCount));
      writeFloat32(exports.memory, hPtr, h.subarray(0, stateCount));
      writeFloat32(exports.memory, cPtr, c.subarray(0, stateCount));
      writeFloat32(exports.memory, hPrevPtr, hPrev.subarray(0, stateCount));
      writeFloat32(exports.memory, cPrevPtr, cPrev.subarray(0, stateCount));
      exports.lstm_step(
        weightsPtr,
        inputPtr,
        hPtr,
        cPtr,
        hPrevPtr,
        cPrevPtr,
        safeInSize,
        safeHidden,
        safeCount,
        safeInputStride
      );
      readFloat32(exports.memory, hPtr, h, stateCount);
      readFloat32(exports.memory, cPtr, c, stateCount);
      readFloat32(exports.memory, hPrevPtr, hPrev, stateCount);
      readFloat32(exports.memory, cPrevPtr, cPrev, stateCount);
    }
  };
}

/**
 * Build an RRU kernel adapter from wasm exports.
 * @param exports - WASM exports.
 * @param heap - Shared wasm allocator.
 * @returns RRU kernel adapter.
 */
function buildRruKernel(exports: SimdWasmExports, heap: WasmAllocator): RruKernel {
  return {
    stepBatch: (
      weights,
      inputs,
      h,
      hPrev,
      inSize,
      hiddenSize,
      count,
      inputStride
    ) => {
      const safeCount = Math.max(0, Math.floor(count));
      const safeInSize = Math.max(0, Math.floor(inSize));
      const safeHidden = Math.max(0, Math.floor(hiddenSize));
      const safeInputStride = Math.max(0, Math.floor(inputStride));
      if (safeCount === 0 || safeInSize === 0 || safeHidden === 0) return;
      const inputCount = safeCount * safeInputStride;
      const stateCount = safeCount * safeHidden;
      if (inputs.length < inputCount) {
        throw new Error('rru_step input buffer too small');
      }
      if (h.length < stateCount || hPrev.length < stateCount) {
        throw new Error('rru_step state buffer too small');
      }
      heap.reset();
      const weightsPtr = heap.alloc(weights.length * BYTES_PER_F32);
      const inputPtr = heap.alloc(inputCount * BYTES_PER_F32);
      const hPtr = heap.alloc(stateCount * BYTES_PER_F32);
      const hPrevPtr = heap.alloc(stateCount * BYTES_PER_F32);
      writeFloat32(exports.memory, weightsPtr, weights);
      writeFloat32(exports.memory, inputPtr, inputs.subarray(0, inputCount));
      writeFloat32(exports.memory, hPtr, h.subarray(0, stateCount));
      writeFloat32(exports.memory, hPrevPtr, hPrev.subarray(0, stateCount));
      exports.rru_step(
        weightsPtr,
        inputPtr,
        hPtr,
        hPrevPtr,
        safeInSize,
        safeHidden,
        safeCount,
        safeInputStride
      );
      readFloat32(exports.memory, hPtr, h, stateCount);
      readFloat32(exports.memory, hPrevPtr, hPrev, stateCount);
    }
  };
}

/**
 * Load SIMD kernels when available.
 */
export async function loadSimdKernels(): Promise<void> {
  if (simdStatus === 'ready') return;
  if (simdLoadPromise) return simdLoadPromise;
  simdStatus = 'loading';
  simdLoadPromise = (async () => {
    try {
      const exports = await instantiateSimdWasm();
      const heap = createWasmAllocator(exports);
      denseKernel = buildDenseKernel(exports, heap);
      mlpKernel = buildMlpKernel(exports, heap);
      gruKernel = buildGruKernel(exports, heap);
      lstmKernel = buildLstmKernel(exports, heap);
      rruKernel = buildRruKernel(exports, heap);
      simdStatus = 'ready';
      didLogSimdFailure = false;
    } catch (err) {
      simdStatus = 'failed';
      denseKernel = null;
      mlpKernel = null;
      gruKernel = null;
      lstmKernel = null;
      rruKernel = null;
      const message = err instanceof Error ? err.message : String(err);
      if (!didLogSimdFailure) {
        console.warn('[simd] load failed', { reason: message });
        didLogSimdFailure = true;
      }
      throw err;
    } finally {
      simdLoadPromise = null;
    }
  })();
  return simdLoadPromise;
}

/**
 * Check whether SIMD kernels are ready.
 * @returns True when kernels are loaded and available.
 */
export function isSimdAvailable(): boolean {
  return simdStatus === 'ready';
}

/**
 * Return the loaded dense kernel, if any.
 * @returns Dense kernel or null when unavailable.
 */
export function getDenseKernel(): DenseKernel | null {
  return denseKernel;
}

/**
 * Return the loaded MLP kernel, if any.
 * @returns MLP kernel or null when unavailable.
 */
export function getMlpKernel(): MlpKernel | null {
  return mlpKernel;
}

/**
 * Return the loaded GRU kernel, if any.
 * @returns GRU kernel or null when unavailable.
 */
export function getGruKernel(): GruKernel | null {
  return gruKernel;
}

/**
 * Return the loaded LSTM kernel, if any.
 * @returns LSTM kernel or null when unavailable.
 */
export function getLstmKernel(): LstmKernel | null {
  return lstmKernel;
}

/**
 * Return the loaded RRU kernel, if any.
 * @returns RRU kernel or null when unavailable.
 */
export function getRruKernel(): RruKernel | null {
  return rruKernel;
}

/**
 * Return the loaded dense kernel or throw if unavailable.
 * @returns Dense kernel instance.
 */
export function requireDenseKernel(): DenseKernel {
  if (!denseKernel) {
    throw new Error('SIMD Dense kernel unavailable; call loadSimdKernels() first.');
  }
  return denseKernel;
}

/**
 * Return the loaded MLP kernel or throw if unavailable.
 * @returns MLP kernel instance.
 */
export function requireMlpKernel(): MlpKernel {
  if (!mlpKernel) {
    throw new Error('SIMD MLP kernel unavailable; call loadSimdKernels() first.');
  }
  return mlpKernel;
}

/**
 * Return the loaded GRU kernel or throw if unavailable.
 * @returns GRU kernel instance.
 */
export function requireGruKernel(): GruKernel {
  if (!gruKernel) {
    throw new Error('SIMD GRU kernel unavailable; call loadSimdKernels() first.');
  }
  return gruKernel;
}

/**
 * Return the loaded LSTM kernel or throw if unavailable.
 * @returns LSTM kernel instance.
 */
export function requireLstmKernel(): LstmKernel {
  if (!lstmKernel) {
    throw new Error('SIMD LSTM kernel unavailable; call loadSimdKernels() first.');
  }
  return lstmKernel;
}

/**
 * Return the loaded RRU kernel or throw if unavailable.
 * @returns RRU kernel instance.
 */
export function requireRruKernel(): RruKernel {
  if (!rruKernel) {
    throw new Error('SIMD RRU kernel unavailable; call loadSimdKernels() first.');
  }
  return rruKernel;
}
