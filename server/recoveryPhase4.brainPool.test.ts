import { afterEach, describe, expect, it, vi } from 'vitest';
import { compileGraph, graphKey } from '../src/brains/graph/compiler.ts';
import { GraphBrain } from '../src/brains/graph/runtime.ts';
import type { GraphSpec } from '../src/brains/graph/schema.ts';
import {
  prepareInferenceBackend
} from '../src/brains/nativeBridge.ts';
import type { InferenceBackend } from '../src/brains/types.ts';
import { BrainPool, type BrainPoolRuntimeOptions } from './brainPool.ts';

/** Phase 4 canonical pool contract suite label. */
const SUITE = 'recovery Phase 4 — canonical population brain pool';
/** Input width shared by the compact Phase 4 graph fixtures. */
const INPUT_SIZE = 3;
/** Output width shared by the compact Phase 4 graph fixtures. */
const OUTPUT_SIZE = 4;
/** Worker requests required by the Phase 4 comparison matrix. */
const WORKER_COUNTS = [1, 2, 4] as const;
/** Pools awaiting cleanup if an assertion interrupts a test. */
const activePools = new Set<BrainPool>();

/** Brain families covered by the canonical worker abstraction. */
type BrainFamily = 'MLP' | 'GRU' | 'LSTM' | 'RRU';

afterEach(async () => {
  await Promise.all(Array.from(activePools, async (pool) => pool.shutdown()));
  activePools.clear();
});

/**
 * Build one compact graph for a required brain family.
 * @param family - MLP or recurrent family to materialize.
 * @returns Valid graph with a common input/output shape.
 */
function buildFamilySpec(family: BrainFamily): GraphSpec {
  if (family === 'MLP') {
    return {
      type: 'graph',
      nodes: [
        { id: 'input', type: 'Input', outputSize: INPUT_SIZE },
        {
          id: 'mlp',
          type: 'MLP',
          inputSize: INPUT_SIZE,
          outputSize: OUTPUT_SIZE,
          hiddenSizes: [5]
        }
      ],
      edges: [{ from: 'input', to: 'mlp' }],
      outputs: [{ nodeId: 'mlp' }],
      outputSize: OUTPUT_SIZE
    };
  }
  return {
    type: 'graph',
    nodes: [
      { id: 'input', type: 'Input', outputSize: INPUT_SIZE },
      { id: 'memory', type: family, inputSize: INPUT_SIZE, hiddenSize: OUTPUT_SIZE }
    ],
    edges: [{ from: 'input', to: 'memory' }],
    outputs: [{ nodeId: 'memory' }],
    outputSize: OUTPUT_SIZE
  };
}

/**
 * Build nonzero deterministic packed weights for every population slot.
 * @param populationCount - Dense population slot count.
 * @param paramCount - Parameters in one graph brain.
 * @param offset - Deterministic phase used for generation replacement.
 * @returns Packed fixed-stride population weights.
 */
function buildWeights(
  populationCount: number,
  paramCount: number,
  offset = 0
): Float32Array {
  const weights = new Float32Array(populationCount * paramCount);
  for (let slot = 0; slot < populationCount; slot++) {
    for (let parameter = 0; parameter < paramCount; parameter++) {
      const index = slot * paramCount + parameter;
      weights[index] = (((slot + 3) * 11 + parameter * 7 + offset) % 29 - 14) * 0.0125;
    }
  }
  return weights;
}

/**
 * Build deterministic input values for one shuffled batch.
 * @param slots - Population slots present in current batch order.
 * @param step - Scenario step used to vary observations.
 * @returns Packed inputs indexed by batch position.
 */
function buildInputs(slots: readonly number[], step: number): Float32Array {
  const inputs = new Float32Array(slots.length * INPUT_SIZE);
  for (let position = 0; position < slots.length; position++) {
    const slot = slots[position] ?? 0;
    const base = position * INPUT_SIZE;
    inputs[base] = (slot + 1) * 0.07 + step * 0.01;
    inputs[base + 1] = (position - 2) * 0.09 - step * 0.015;
    inputs[base + 2] = ((slot + step) % 5 - 2) * 0.11;
  }
  return inputs;
}

/**
 * Assert two floating-point buffers match within native-kernel tolerance.
 * @param actual - Observed buffer.
 * @param expected - Reference buffer.
 * @param tolerance - Maximum accepted absolute difference.
 */
function expectClose(
  actual: Float32Array,
  expected: Float32Array,
  tolerance = 1e-4
): void {
  expect(actual.length).toBe(expected.length);
  for (let index = 0; index < actual.length; index++) {
    const difference = Math.abs((actual[index] ?? 0) - (expected[index] ?? 0));
    if (difference > tolerance) {
      throw new Error(
        `buffer mismatch at ${index}: ${actual[index]} vs ${expected[index]} (${difference})`
      );
    }
  }
}

/**
 * Create and track a ready canonical pool.
 * @param spec - Graph compiled by workers.
 * @param weights - Packed population weights copied during initialization.
 * @param requestedWorkers - Requested worker count.
 * @param backend - Immutable worker backend.
 * @param runtimeOptions - Optional focused lifecycle overrides.
 * @returns Ready pool owned by the test.
 */
async function createPool(
  spec: GraphSpec,
  weights: Float32Array,
  requestedWorkers: number,
  backend: InferenceBackend,
  runtimeOptions: BrainPoolRuntimeOptions = {}
): Promise<BrainPool> {
  const compiled = compileGraph(spec);
  const populationCount = weights.length / compiled.totalParams;
  const pool = new BrainPool(requestedWorkers, backend, {
    vizMinIntervalMs: 0,
    ...runtimeOptions
  });
  activePools.add(pool);
  await pool.init({
    spec,
    specKey: graphKey(spec),
    populationCount,
    paramCount: compiled.totalParams,
    inputStride: INPUT_SIZE,
    outputStride: OUTPUT_SIZE,
    maxBatch: populationCount,
    weights
  });
  return pool;
}

/**
 * Run shuffled and shrinking batches against pooled and serial brains.
 * @param family - Brain family under test.
 * @param backend - JS diagnostic or native backend.
 * @param requestedWorkers - Requested pool worker count.
 * @returns Concatenated pooled output history for cross-count comparison.
 */
async function runOwnershipScenario(
  family: BrainFamily,
  backend: InferenceBackend,
  requestedWorkers: number
): Promise<Float32Array> {
  await prepareInferenceBackend(backend);
  const spec = buildFamilySpec(family);
  const compiled = compileGraph(spec);
  const populationCount = 6;
  const weights = buildWeights(populationCount, compiled.totalParams);
  const pool = await createPool(spec, weights, requestedWorkers, backend);

  expect(Array.from(pool.weightsView ?? [])).toEqual(Array.from(weights));
  const statuses = pool.getWorkerStatuses();
  expect(statuses).toHaveLength(pool.workerCount);
  expect(statuses.reduce((sum, status) => sum + status.ownedSlotCount, 0)).toBe(
    populationCount
  );
  for (const status of statuses) {
    expect(status.activeBackend).toBe(backend);
    expect(status.poolEpoch).toBe(pool.poolEpoch);
    expect(status.weightEpoch).toBe(1);
    if (backend === 'native') {
      expect(status.nativeAddonBuildIdentifier).toMatch(
        /^slither_native\/0\.1\.0\+[0-9a-f]{12}\.[0-9a-f]{16}$/u
      );
    } else {
      expect(status.nativeAddonBuildIdentifier).toBeNull();
    }
  }

  const serialBrains = Array.from({ length: populationCount }, (_unused, slot) => {
    const offset = slot * compiled.totalParams;
    return new GraphBrain(
      compiled,
      weights.subarray(offset, offset + compiled.totalParams),
      backend
    );
  });
  const batches: readonly (readonly number[])[] = [
    [5, 0, 3, 1, 4, 2],
    [2, 5, 0],
    [4, 2],
    [3, 1, 5],
    [1, 0, 4, 3]
  ];
  const history: number[] = [];
  for (let step = 0; step < batches.length; step++) {
    const slots = batches[step] ?? [];
    const inputs = buildInputs(slots, step);
    const outputs = new Float32Array(slots.length * OUTPUT_SIZE);
    await pool.runBatch(
      inputs,
      outputs,
      Uint32Array.from(slots),
      slots.length,
      INPUT_SIZE,
      OUTPUT_SIZE
    );
    const expected = new Float32Array(outputs.length);
    for (let position = 0; position < slots.length; position++) {
      const slot = slots[position] ?? 0;
      const inputOffset = position * INPUT_SIZE;
      expected.set(
        serialBrains[slot]!.forward(inputs.subarray(inputOffset, inputOffset + INPUT_SIZE)),
        position * OUTPUT_SIZE
      );
    }
    expectClose(outputs, expected);
    history.push(...outputs);
  }
  expect(history.some((value) => value !== 0)).toBe(true);
  return Float32Array.from(history);
}

describe(SUITE, () => {
  for (const backend of ['js', 'native'] as const) {
    for (const family of ['MLP', 'GRU', 'LSTM', 'RRU'] as const) {
      it(`${family} ${backend} matches serial state across 1, 2, and 4 worker requests`, async () => {
        const histories: Float32Array[] = [];
        for (const workerCount of WORKER_COUNTS) {
          histories.push(await runOwnershipScenario(family, backend, workerCount));
        }
        expectClose(histories[1]!, histories[0]!);
        expectClose(histories[2]!, histories[0]!);
      });
    }
  }

  it('advances the weight epoch and zeroes recurrent state before new inference', async () => {
    await prepareInferenceBackend('js');
    const spec = buildFamilySpec('GRU');
    const compiled = compileGraph(spec);
    const populationCount = 4;
    const firstWeights = buildWeights(populationCount, compiled.totalParams);
    const pool = await createPool(spec, firstWeights, 2, 'js');
    const slots = Uint32Array.from([3, 0, 2]);
    const inputs = buildInputs(Array.from(slots), 0);
    await pool.runBatch(
      inputs,
      new Float32Array(slots.length * OUTPUT_SIZE),
      slots,
      slots.length,
      INPUT_SIZE,
      OUTPUT_SIZE
    );

    const nextWeights = buildWeights(populationCount, compiled.totalParams, 9);
    await pool.replacePopulationWeights(nextWeights);
    expect(pool.weightEpoch).toBe(2);
    expect(pool.getWorkerStatuses().every((status) => status.weightEpoch === 2)).toBe(true);

    const resetOutputs = new Float32Array(slots.length * OUTPUT_SIZE);
    await pool.runBatch(
      inputs,
      resetOutputs,
      slots,
      slots.length,
      INPUT_SIZE,
      OUTPUT_SIZE
    );
    const expected = new Float32Array(resetOutputs.length);
    for (let position = 0; position < slots.length; position++) {
      const slot = slots[position] ?? 0;
      const offset = slot * compiled.totalParams;
      const fresh = new GraphBrain(
        compiled,
        nextWeights.subarray(offset, offset + compiled.totalParams),
        'js'
      );
      const inputOffset = position * INPUT_SIZE;
      expected.set(
        fresh.forward(inputs.subarray(inputOffset, inputOffset + INPUT_SIZE)),
        position * OUTPUT_SIZE
      );
    }
    expectClose(resetOutputs, expected);
  });

  it('reinitializes architecture only by creating a new pool epoch', async () => {
    await prepareInferenceBackend('js');
    const firstSpec = buildFamilySpec('MLP');
    const firstCompiled = compileGraph(firstSpec);
    const pool = await createPool(
      firstSpec,
      buildWeights(3, firstCompiled.totalParams),
      2,
      'js'
    );
    const firstEpoch = pool.poolEpoch;
    const nextSpec = buildFamilySpec('LSTM');
    const nextCompiled = compileGraph(nextSpec);
    await pool.init({
      spec: nextSpec,
      specKey: graphKey(nextSpec),
      populationCount: 3,
      paramCount: nextCompiled.totalParams,
      inputStride: INPUT_SIZE,
      outputStride: OUTPUT_SIZE,
      maxBatch: 3,
      weights: buildWeights(3, nextCompiled.totalParams)
    });
    expect(pool.poolEpoch).toBeGreaterThan(firstEpoch ?? 0);
    expect(pool.specKey).toBe(graphKey(nextSpec));
    expect(pool.weightEpoch).toBe(1);
  });

  it('returns tagged selected-slot visualization from the owning worker', async () => {
    await prepareInferenceBackend('js');
    const spec = buildFamilySpec('LSTM');
    const compiled = compileGraph(spec);
    const pool = await createPool(spec, buildWeights(4, compiled.totalParams), 2, 'js');
    const inputs = buildInputs([3], 0);
    await pool.runBatch(
      inputs,
      new Float32Array(OUTPUT_SIZE),
      Uint32Array.of(3),
      1,
      INPUT_SIZE,
      OUTPUT_SIZE
    );
    const viz = await pool.requestVisualization(3, 27);
    expect(viz).toMatchObject({
      kind: 'graph',
      populationSlot: 3,
      simulationStep: 27,
      poolEpoch: pool.poolEpoch,
      weightEpoch: pool.weightEpoch
    });
    expect(viz?.layers.some((layer) => layer.isRecurrent)).toBe(true);
    expect(viz?.layers.some((layer) => (layer.activations?.length ?? 0) > 0)).toBe(true);
  });

  it('ignores a stale completion while a later batch remains in flight', async () => {
    await prepareInferenceBackend('js');
    const spec = buildFamilySpec('MLP');
    const compiled = compileGraph(spec);
    const pool = await createPool(spec, buildWeights(3, compiled.totalParams), 1, 'js');
    const slots = Uint32Array.from([0, 1, 2]);
    const inputs = buildInputs(Array.from(slots), 0);
    const firstOutputs = new Float32Array(slots.length * OUTPUT_SIZE);
    await pool.runBatch(
      inputs,
      firstOutputs,
      slots,
      slots.length,
      INPUT_SIZE,
      OUTPUT_SIZE
    );

    const secondOutputs = new Float32Array(slots.length * OUTPUT_SIZE);
    const laterBatch = pool.runBatch(
      inputs,
      secondOutputs,
      slots,
      slots.length,
      INPUT_SIZE,
      OUTPUT_SIZE
    );
    pool.workers[0]?.emit('message', {
      type: 'done',
      workerId: 0,
      poolEpoch: pool.poolEpoch,
      weightEpoch: pool.weightEpoch,
      batchId: 1,
      batchCount: slots.length,
      processedCount: slots.length
    });
    await laterBatch;
    expectClose(secondOutputs, firstOutputs);
    expect(pool.status).toBe('ready');
  });

  it('rejects in-flight work and leaves the pool failed after a worker error', async () => {
    await prepareInferenceBackend('js');
    const spec = buildFamilySpec('GRU');
    const compiled = compileGraph(spec);
    const pool = await createPool(spec, buildWeights(4, compiled.totalParams), 2, 'js');
    const slots = Uint32Array.from([0, 1, 2, 3]);
    const pending = pool.runBatch(
      buildInputs(Array.from(slots), 0),
      new Float32Array(slots.length * OUTPUT_SIZE),
      slots,
      slots.length,
      INPUT_SIZE,
      OUTPUT_SIZE
    );
    pool.workers[0]?.emit('error', new Error('phase4 injected worker failure'));
    await expect(pending).rejects.toThrow('phase4 injected worker failure');
    expect(pool.status).toBe('failed');
    await expect(pool.runBatch(
      new Float32Array(INPUT_SIZE),
      new Float32Array(OUTPUT_SIZE),
      Uint32Array.of(0),
      1,
      INPUT_SIZE,
      OUTPUT_SIZE
    )).rejects.toThrow('mt pool not ready');
  });

  it('rejects duplicate and out-of-range population slots before dispatch', async () => {
    await prepareInferenceBackend('js');
    const spec = buildFamilySpec('MLP');
    const compiled = compileGraph(spec);
    const pool = await createPool(spec, buildWeights(3, compiled.totalParams), 2, 'js');
    const inputs = buildInputs([0, 0], 0);
    await expect(pool.runBatch(
      inputs,
      new Float32Array(2 * OUTPUT_SIZE),
      Uint32Array.of(0, 0),
      2,
      INPUT_SIZE,
      OUTPUT_SIZE
    )).rejects.toThrow('appears twice');
    await expect(pool.runBatch(
      new Float32Array(INPUT_SIZE),
      new Float32Array(OUTPUT_SIZE),
      Uint32Array.of(3),
      1,
      INPUT_SIZE,
      OUTPUT_SIZE
    )).rejects.toThrow('out of range');
    expect(pool.status).toBe('ready');
  });

  it('caps worker ownership by population size and the conservative ceiling', async () => {
    await prepareInferenceBackend('js');
    const spec = buildFamilySpec('MLP');
    const compiled = compileGraph(spec);
    const pool = await createPool(spec, buildWeights(2, compiled.totalParams), 100, 'js');
    expect(pool.workerCount).toBeGreaterThanOrEqual(1);
    expect(pool.workerCount).toBeLessThanOrEqual(2);
    expect(pool.workerCount).toBeLessThanOrEqual(8);
  });

  it('fails initialization when a worker does not acknowledge before timeout', async () => {
    await prepareInferenceBackend('js');
    const spec = buildFamilySpec('MLP');
    const compiled = compileGraph(spec);
    const pool = new BrainPool(1, 'js', {
      initTimeoutMs: 25,
      workerUrlForTesting: new URL('./test/stallBrainPoolWorker.ts', import.meta.url)
    });
    activePools.add(pool);
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    try {
      await expect(pool.init({
        spec,
        specKey: graphKey(spec),
        populationCount: 2,
        paramCount: compiled.totalParams,
        inputStride: INPUT_SIZE,
        outputStride: OUTPUT_SIZE,
        maxBatch: 2,
        weights: buildWeights(2, compiled.totalParams)
      })).rejects.toThrow('mt pool init timeout');
    } finally {
      warnSpy.mockRestore();
    }
    expect(pool.status).toBe('failed');
  });

  it('times out inference and rejects a concurrent batch', async () => {
    await prepareInferenceBackend('js');
    const spec = buildFamilySpec('GRU');
    const compiled = compileGraph(spec);
    const pool = await createPool(
      spec,
      buildWeights(3, compiled.totalParams),
      2,
      'js',
      { inferenceTimeoutMs: 25 }
    );
    for (const worker of pool.workers) worker.removeAllListeners('message');
    const slots = Uint32Array.from([0, 1, 2]);
    const inputs = buildInputs(Array.from(slots), 0);
    const pending = pool.runBatch(
      inputs,
      new Float32Array(slots.length * OUTPUT_SIZE),
      slots,
      slots.length,
      INPUT_SIZE,
      OUTPUT_SIZE
    );
    await expect(pool.runBatch(
      inputs,
      new Float32Array(slots.length * OUTPUT_SIZE),
      slots,
      slots.length,
      INPUT_SIZE,
      OUTPUT_SIZE
    )).rejects.toThrow('dispatch already in flight');
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    try {
      await expect(pending).rejects.toThrow('mt pool inference timeout');
    } finally {
      warnSpy.mockRestore();
    }
    expect(pool.status).toBe('failed');
  });

  it('times out an unacknowledged recurrent reset', async () => {
    await prepareInferenceBackend('js');
    const spec = buildFamilySpec('LSTM');
    const compiled = compileGraph(spec);
    const pool = await createPool(
      spec,
      buildWeights(3, compiled.totalParams),
      2,
      'js',
      { resetTimeoutMs: 25 }
    );
    for (const worker of pool.workers) worker.removeAllListeners('message');
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    try {
      await expect(pool.replacePopulationWeights(
        buildWeights(3, compiled.totalParams, 5)
      )).rejects.toThrow('mt pool reset timeout');
    } finally {
      warnSpy.mockRestore();
    }
    expect(pool.status).toBe('failed');
  });

  it('fails the pool after any unexpected worker exit', async () => {
    await prepareInferenceBackend('js');
    const spec = buildFamilySpec('RRU');
    const compiled = compileGraph(spec);
    const pool = await createPool(spec, buildWeights(2, compiled.totalParams), 1, 'js');
    const worker = pool.workers[0];
    if (!worker) throw new Error('expected a worker thread');
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    try {
      await worker.terminate();
      await new Promise<void>((resolve) => setImmediate(resolve));
    } finally {
      warnSpy.mockRestore();
    }
    expect(pool.status).toBe('failed');
    expect(pool.failureReason).toContain('exited unexpectedly');
  });
});
