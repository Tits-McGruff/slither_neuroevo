/** Deterministic, cross-language Stage 4 heterogeneous-inference fixtures. */

import { createHash, type Hash } from 'node:crypto';
import os from 'node:os';
import { GraphBrain } from '../../src/brains/graph/runtime.ts';
import { compileGraph, type CompiledGraph } from '../../src/brains/graph/compiler.ts';
import type { GraphSpec } from '../../src/brains/graph/schema.ts';
import type { InferenceBackend } from '../../src/brains/types.ts';

/** Version of the deterministic numeric generator and workload description. */
export const STAGE4_INFERENCE_FIXTURE_VERSION = 1;

/** Supported inference-only workloads from the approved migration plan. */
export type Stage4InferenceScenarioName = 'P0' | 'P1' | 'P2' | 'P3';

/** Kind-specific bit pattern generated for one fixture value. */
type FixtureValueKind = 'weight' | 'observation' | 'recurrent';

/** Resolved source-shaped scenario description. */
export interface Stage4InferenceScenario {
  /** Approved scenario label. */
  name: Stage4InferenceScenarioName;
  /** Number of differently weighted synthetic population brains due in one pass. */
  populationCount: number;
  /** Complete v3 sensor width. */
  inputSize: number;
  /** Stable human-readable workload description. */
  description: string;
  /** Current source-shaped graph. */
  graphSpec: GraphSpec;
}

/** Fully allocated inputs for one current-TypeScript comparison path. */
export interface Stage4InferenceFixture {
  /** Resolved scenario. */
  scenario: Stage4InferenceScenario;
  /** Current TypeScript compiled graph. */
  compiled: CompiledGraph;
  /** One distinct packed parameter buffer per evolved slot. */
  weights: Float32Array[];
  /** One immutable observation view per evolved slot. */
  observations: Float32Array[];
  /** SHA-256 over all little-endian weight bits in slot order. */
  weightsSha256: string;
  /** SHA-256 over all little-endian observation bits in slot order. */
  observationsSha256: string;
  /** SHA-256 over the generated nonzero initial recurrent bits in slot order. */
  initialRecurrentSha256: string;
}

/** Reused four-byte conversion storage; evidence generation is single-threaded. */
const valueBytes = new ArrayBuffer(4);
/** Reused view for exact integer-to-Float32 bit conversion. */
const valueView = new DataView(valueBytes);

/**
 * Build the exact default graph currently used by P0/P1.
 * @param inputSize - Active sensor width.
 * @returns Input, 64/64 MLP, GRU-16, and Dense-2 graph.
 */
function buildDefaultGraph(inputSize: number): GraphSpec {
  return {
    type: 'graph',
    nodes: [
      { id: 'input', type: 'Input', outputSize: inputSize },
      { id: 'mlp', type: 'MLP', inputSize, hiddenSizes: [64], outputSize: 64 },
      { id: 'gru', type: 'GRU', inputSize: 64, hiddenSize: 16 },
      { id: 'head', type: 'Dense', inputSize: 16, outputSize: 2 }
    ],
    edges: [
      { from: 'input', to: 'mlp' },
      { from: 'mlp', to: 'gru' },
      { from: 'gru', to: 'head' }
    ],
    outputs: [{ nodeId: 'head' }],
    outputSize: 2
  };
}

/**
 * Build the exact Stage 2 large-GRU graph currently used by P2/P3.
 * @param inputSize - Active sensor width.
 * @returns Five-256-layer feature stack, GRU-96, and Dense-2 graph.
 */
function buildLargeGraph(inputSize: number): GraphSpec {
  return {
    type: 'graph',
    nodes: [
      { id: 'input', type: 'Input', outputSize: inputSize },
      {
        id: 'features',
        type: 'MLP',
        inputSize,
        hiddenSizes: [256, 256, 256, 256],
        outputSize: 256
      },
      { id: 'memory', type: 'GRU', inputSize: 256, hiddenSize: 96 },
      { id: 'output', type: 'Dense', inputSize: 96, outputSize: 2 }
    ],
    edges: [
      { from: 'input', to: 'features' },
      { from: 'features', to: 'memory' },
      { from: 'memory', to: 'output' }
    ],
    outputs: [{ nodeId: 'output' }],
    outputSize: 2
  };
}

/**
 * Resolve one approved inference workload without mutating global configuration.
 * @param name - P0, P1, P2, or P3.
 * @returns Source-shaped graph and due-population dimensions.
 */
export function stage4InferenceScenario(
  name: Stage4InferenceScenarioName
): Stage4InferenceScenario {
  const large = name === 'P2' || name === 'P3';
  const many = name === 'P1' || name === 'P3';
  const inputSize = large ? 147 : 83;
  const populationCount = many ? 300 : 55;
  return {
    name,
    populationCount,
    inputSize,
    description: large
      ? `${populationCount} differently weighted synthetic population brains, v3/32-bin input shape, five-layer 256-wide MLP, GRU-96, Dense-2`
      : `${populationCount} differently weighted synthetic population brains, v3/16-bin input shape, 64/64 MLP, GRU-16, Dense-2`,
    graphSpec: large ? buildLargeGraph(inputSize) : buildDefaultGraph(inputSize)
  };
}

/**
 * Return a stable scenario seed shared with the Rust evidence runner.
 * @param scenario - Resolved workload.
 * @returns Unsigned 32-bit seed.
 */
function scenarioWord(scenario: Stage4InferenceScenario): number {
  return scenario.inputSize === 147 ? 0x13198a2e : 0x85a308d3;
}

/**
 * Produce one deterministic xorshift word without advancing authoritative RNG.
 * @param scenario - Resolved workload.
 * @param kind - Numeric payload role.
 * @param slot - Dense population slot.
 * @param index - Float index within that slot's role.
 * @returns Unsigned 32-bit mixed word.
 */
function fixtureWord(
  scenario: Stage4InferenceScenario,
  kind: FixtureValueKind,
  slot: number,
  index: number
): number {
  const kindWord = {
    weight: 0x243f6a88,
    observation: 0xb7e15162,
    recurrent: 0x9e3779b9
  }[kind];
  let word = (
    scenarioWord(scenario)
    ^ kindWord
    ^ Math.imul((slot + 1) >>> 0, 0x9e3779b9)
    ^ Math.imul((index + 1) >>> 0, 0x7f4a7c15)
  ) >>> 0;
  word = (word ^ (word << 13)) >>> 0;
  word = (word ^ (word >>> 17)) >>> 0;
  word = (word ^ (word << 5)) >>> 0;
  return word;
}

/**
 * Convert one generated word into a bounded, nonzero, bit-exact Float32.
 * @param scenario - Resolved workload.
 * @param kind - Numeric payload role.
 * @param slot - Dense population slot.
 * @param index - Float index within that role.
 * @returns Deterministic finite value shared with Rust.
 */
export function stage4FixtureValue(
  scenario: Stage4InferenceScenario,
  kind: FixtureValueKind,
  slot: number,
  index: number
): number {
  const word = fixtureWord(scenario, kind, slot, index);
  const exponent = {
    weight: 0x3d000000,
    observation: 0x3e000000,
    recurrent: 0x3c000000
  }[kind];
  const sign = word & 0x80000000;
  valueView.setUint32(0, (sign | exponent | (word & 0x007fffff)) >>> 0, true);
  return valueView.getFloat32(0, true);
}

/**
 * Add Float32 backing bytes to a logical SHA-256 stream.
 * @param hash - Hash receiving bytes.
 * @param values - Exact Float32 values.
 */
function updateFloat32Hash(hash: Hash, values: Float32Array): void {
  if (os.endianness() !== 'LE') {
    throw new Error('Stage 4 inference evidence currently supports little-endian targets only.');
  }
  hash.update(Buffer.from(values.buffer, values.byteOffset, values.byteLength));
}

/**
 * Build distinct weights, observations, and the initial-state digest once.
 * @param name - Approved workload name.
 * @returns Fully allocated deterministic fixture.
 */
export function buildStage4InferenceFixture(
  name: Stage4InferenceScenarioName
): Stage4InferenceFixture {
  const scenario = stage4InferenceScenario(name);
  const compiled = compileGraph(scenario.graphSpec);
  const weightHash = createHash('sha256');
  const observationHash = createHash('sha256');
  const recurrentHash = createHash('sha256');
  const weights = new Array<Float32Array>(scenario.populationCount);
  const packedObservations = new Float32Array(scenario.populationCount * scenario.inputSize);
  const observations = new Array<Float32Array>(scenario.populationCount);

  for (let slot = 0; slot < scenario.populationCount; slot++) {
    const genome = new Float32Array(compiled.totalParams);
    for (let index = 0; index < genome.length; index++) {
      genome[index] = stage4FixtureValue(scenario, 'weight', slot, index);
    }
    weights[slot] = genome;
    updateFloat32Hash(weightHash, genome);

    const observationStart = slot * scenario.inputSize;
    const observation = packedObservations.subarray(
      observationStart,
      observationStart + scenario.inputSize
    );
    for (let index = 0; index < observation.length; index++) {
      observation[index] = stage4FixtureValue(scenario, 'observation', slot, index);
    }
    observations[slot] = observation;
    updateFloat32Hash(observationHash, observation);

    const recurrent = new Float32Array(compiled.totalStateSize);
    for (let index = 0; index < recurrent.length; index++) {
      recurrent[index] = stage4FixtureValue(scenario, 'recurrent', slot, index);
    }
    updateFloat32Hash(recurrentHash, recurrent);
  }

  return {
    scenario,
    compiled,
    weights,
    observations,
    weightsSha256: weightHash.digest('hex'),
    observationsSha256: observationHash.digest('hex'),
    initialRecurrentSha256: recurrentHash.digest('hex')
  };
}

/**
 * Install the shared nonzero recurrent fixture into one current GraphBrain.
 * @param brain - Brain whose node state is initialized.
 * @param scenario - Resolved workload.
 * @param slot - Dense population slot.
 */
export function initializeStage4BrainState(
  brain: GraphBrain,
  scenario: Stage4InferenceScenario,
  slot: number
): void {
  let stateIndex = 0;
  for (const node of brain.nodes) {
    if (node.gru) {
      for (let index = 0; index < node.gru.h.length; index++) {
        node.gru.h[index] = stage4FixtureValue(scenario, 'recurrent', slot, stateIndex++);
      }
    }
    if (node.lstm) {
      for (let index = 0; index < node.lstm.h.length; index++) {
        node.lstm.h[index] = stage4FixtureValue(scenario, 'recurrent', slot, stateIndex++);
      }
      for (let index = 0; index < node.lstm.c.length; index++) {
        node.lstm.c[index] = stage4FixtureValue(scenario, 'recurrent', slot, stateIndex++);
      }
    }
    if (node.rru) {
      for (let index = 0; index < node.rru.h.length; index++) {
        node.rru.h[index] = stage4FixtureValue(scenario, 'recurrent', slot, stateIndex++);
      }
    }
  }
  if (stateIndex !== brain.compiled.totalStateSize) {
    throw new Error(
      `Stage 4 recurrent fixture wrote ${stateIndex} floats; expected ${brain.compiled.totalStateSize}.`
    );
  }
}

/**
 * Construct current GraphBrain objects with their own weights and recurrent state.
 * @param fixture - Allocated deterministic fixture.
 * @param backend - Current JS graph or count-one native-kernel path.
 * @returns One current runtime brain per evolved slot.
 */
export function buildStage4CurrentBrains(
  fixture: Stage4InferenceFixture,
  backend: InferenceBackend
): GraphBrain[] {
  const brains = new Array<GraphBrain>(fixture.scenario.populationCount);
  for (let slot = 0; slot < brains.length; slot++) {
    const brain = new GraphBrain(fixture.compiled, fixture.weights[slot]!, backend);
    initializeStage4BrainState(brain, fixture.scenario, slot);
    brains[slot] = brain;
  }
  return brains;
}

/**
 * Add one brain's complete recurrent state to a logical digest.
 * @param hash - Hash receiving state bits.
 * @param brain - Current runtime brain.
 */
function updateBrainStateHash(hash: Hash, brain: GraphBrain): void {
  for (const node of brain.nodes) {
    if (node.gru) updateFloat32Hash(hash, node.gru.h);
    if (node.lstm) {
      updateFloat32Hash(hash, node.lstm.h);
      updateFloat32Hash(hash, node.lstm.c);
    }
    if (node.rru) updateFloat32Hash(hash, node.rru.h);
  }
}

/**
 * Hash final outputs and recurrent state so benchmark work cannot be discarded.
 * @param brains - Completed current runtime brains.
 * @returns Separate logical SHA-256 values.
 */
export function stage4ResultDigests(brains: readonly GraphBrain[]): {
  outputsSha256: string;
  recurrentSha256: string;
  distinctOutputPairs: number;
} {
  const outputHash = createHash('sha256');
  const recurrentHash = createHash('sha256');
  const pairs = new Set<string>();
  for (const brain of brains) {
    updateFloat32Hash(outputHash, brain.output);
    updateBrainStateHash(recurrentHash, brain);
    pairs.add(Array.from(brain.output, value => value.toString()).join(','));
  }
  return {
    outputsSha256: outputHash.digest('hex'),
    recurrentSha256: recurrentHash.digest('hex'),
    distinctOutputPairs: pairs.size
  };
}

/**
 * Retain one complete one-step result for a real element-by-element tolerance check.
 * @param brains - Brains immediately after one pass from the shared initial state.
 * @returns Raw little-endian Float32 values plus exact logical hashes and counts.
 */
export function stage4OneStepComparisonData(brains: readonly GraphBrain[]): {
  absoluteTolerance: number;
  outputsF32LeHex: string;
  recurrentF32LeHex: string;
  outputFloats: number;
  recurrentFloats: number;
  outputsSha256: string;
  recurrentSha256: string;
} {
  const outputChunks: string[] = [];
  const recurrentChunks: string[] = [];
  const outputHash = createHash('sha256');
  const recurrentHash = createHash('sha256');
  let outputFloats = 0;
  let recurrentFloats = 0;
  const append = (values: Float32Array, chunks: string[], hash: Hash): void => {
    for (const value of values) {
      if (!Number.isFinite(value)) throw new Error('Comparison probe received a non-finite value.');
    }
    const bytes = Buffer.from(values.buffer, values.byteOffset, values.byteLength);
    chunks.push(bytes.toString('hex'));
    hash.update(bytes);
  };
  for (const brain of brains) {
    append(brain.output, outputChunks, outputHash);
    outputFloats += brain.output.length;
    for (const node of brain.nodes) {
      if (node.gru) {
        append(node.gru.h, recurrentChunks, recurrentHash);
        recurrentFloats += node.gru.h.length;
      }
      if (node.lstm) {
        append(node.lstm.h, recurrentChunks, recurrentHash);
        append(node.lstm.c, recurrentChunks, recurrentHash);
        recurrentFloats += node.lstm.h.length + node.lstm.c.length;
      }
      if (node.rru) {
        append(node.rru.h, recurrentChunks, recurrentHash);
        recurrentFloats += node.rru.h.length;
      }
    }
  }
  return {
    absoluteTolerance: 1e-4,
    outputsF32LeHex: outputChunks.join(''),
    recurrentF32LeHex: recurrentChunks.join(''),
    outputFloats,
    recurrentFloats,
    outputsSha256: outputHash.digest('hex'),
    recurrentSha256: recurrentHash.digest('hex')
  };
}
