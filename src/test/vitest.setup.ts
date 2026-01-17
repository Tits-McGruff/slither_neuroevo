import { beforeAll } from 'vitest';
import { loadSimdKernels } from '../brains/wasmBridge.ts';

/** Ensure SIMD kernels are available before tests run. */
beforeAll(async () => {
  await loadSimdKernels();
});
