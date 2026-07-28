/** Reproducible Stage 2 workload and persistence fixtures. */

import { createHash } from 'node:crypto';
import { CFG, resetCFGToDefaults, syncBrainInputSize } from '../../src/config.ts';
import type { GraphSpec } from '../../src/brains/graph/schema.ts';
import { crossover, enrichArchInfo, Genome, mutate, type ArchDefinition } from '../../src/mlp.ts';
import type { CoreSettings } from '../../src/protocol/settings.ts';
import { StatefulRng } from '../../src/rng.ts';
import type { World } from '../../src/world.ts';

/** Standard workload identifiers from the approved migration plan. */
export type Stage2ScenarioName = 'P0' | 'P1' | 'P2' | 'P3' | 'P4';

/** Recurrent family used by a large-brain fixture. */
export type Stage2RecurrentKind = 'GRU' | 'LSTM' | 'RRU';

/** Fully resolved settings for a Stage 2 workload. */
export interface Stage2Scenario {
  /** Approved scenario identifier. */
  name: Stage2ScenarioName;
  /** Plain workload description. */
  description: string;
  /** World constructor settings. */
  settings: CoreSettings;
  /** Target ambient pellet count. */
  pelletCountTarget: number;
  /** Sensor bubble-bin count. */
  bubbleBins: number;
  /** Number of baseline bots. */
  baselineBotCount: number;
  /** Custom graph, or null for the current default stack builder. */
  graphSpec: GraphSpec | null;
  /** Whether to expand bodies beyond the old collision-grid capacity. */
  denseLongBodies: boolean;
}

/** Fixed seed for comparable Stage 2 world fixtures. */
export const STAGE2_WORLD_SEED = 0x5a17c0de;
/** Fixed seed for deterministic mutation/crossover fixture generation. */
export const STAGE2_EVOLUTION_SEED = 0x0e701e5d;
/** Body points per snake in the dense P4 fixture. */
const P4_BODY_POINTS_PER_SNAKE = 700;

/**
 * Construct the approved large-brain graph.
 * @param inputSize - Active v3 sensor input size.
 * @param recurrentKind - Recurrent family to include.
 * @returns Five-256-layer feature stack, recurrent 96, and Dense 2.
 */
export function buildLargeBrainGraph(
  inputSize: number,
  recurrentKind: Stage2RecurrentKind = 'GRU'
): GraphSpec {
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
      {
        id: 'memory',
        type: recurrentKind,
        inputSize: 256,
        hiddenSize: 96
      },
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
 * Apply one standard scenario to the mutable current TypeScript configuration.
 * The caller must invoke `resetCFGToDefaults()` after use.
 * @param name - Scenario identifier.
 * @param recurrentKind - Recurrent family for P2/P3.
 * @returns Resolved settings and graph.
 */
export function installStage2Scenario(
  name: Stage2ScenarioName,
  recurrentKind: Stage2RecurrentKind = 'GRU'
): Stage2Scenario {
  resetCFGToDefaults();
  const large = name === 'P2' || name === 'P3';
  const bubbleBins = large ? 32 : 16;
  CFG.sense.layoutVersion = 'v3';
  CFG.sense.bubbleBins = bubbleBins;
  syncBrainInputSize();
  const graphSpec = large ? buildLargeBrainGraph(CFG.brain.inSize, recurrentKind) : null;
  CFG.brain.graphSpec = graphSpec;
  const snakeCount = name === 'P1' || name === 'P3' || name === 'P4' ? 300 : 55;
  const pelletCountTarget = name === 'P4' ? 12_000 : 3_500;
  CFG.baselineBots.count = 10;
  CFG.pelletCountTarget = pelletCountTarget;
  const settings: CoreSettings = {
    snakeCount,
    simSpeed: 1,
    hiddenLayers: large ? 5 : 2,
    neurons1: large ? 256 : 64,
    neurons2: large ? 256 : 64,
    neurons3: large ? 256 : 64,
    neurons4: large ? 256 : 48,
    neurons5: large ? 256 : 32
  };
  const description = {
    P0: '55 evolved snakes, 10 baseline bots, default graph, 3,500 pellets',
    P1: '300 evolved snakes, 10 baseline bots, default graph, 3,500 pellets',
    P2: `55 evolved snakes, 10 baseline bots, 32-bin five-layer large ${recurrentKind} graph`,
    P3: `300 evolved snakes, 10 baseline bots, 32-bin five-layer large ${recurrentKind} graph`,
    P4: '300 evolved snakes with 700-point bodies and 12,000 pellets'
  }[name];
  return {
    name,
    description,
    settings,
    pelletCountTarget,
    bubbleBins,
    baselineBotCount: CFG.baselineBots.count,
    graphSpec,
    denseLongBodies: name === 'P4'
  };
}

/**
 * Expand every snake body deterministically beyond the old 200,000-entry grid
 * ceiling. This is a stress fixture, not a preserved gameplay state.
 * @param world - Constructed P4 world.
 * @returns Total body points installed.
 */
export function installDenseLongBodies(world: World): number {
  for (let snakeIndex = 0; snakeIndex < world.snakes.length; snakeIndex++) {
    const snake = world.snakes[snakeIndex]!;
    const points = new Array<{ x: number; y: number }>(P4_BODY_POINTS_PER_SNAKE);
    const phase = snakeIndex * 0.6180339887498948;
    for (let pointIndex = 0; pointIndex < points.length; pointIndex++) {
      const angle = phase + pointIndex * 0.025;
      const radius = Math.min(600, pointIndex * 0.75);
      points[pointIndex] = {
        x: snake.x - Math.cos(angle) * radius,
        y: snake.y - Math.sin(angle) * radius
      };
    }
    points[0] = { x: snake.x, y: snake.y };
    snake.points = points;
    snake.targetLen = points.length;
  }
  world._collGrid.build(world.snakes, CFG.collision.skipSegments);
  return world.snakes.length * P4_BODY_POINTS_PER_SNAKE;
}

/**
 * Concatenate one population into explicit little-endian Float32 bytes.
 * @param population - Dense population.
 * @returns Packed bytes without per-genome padding.
 */
export function packPopulationWeights(population: readonly Genome[]): Buffer {
  const totalFloats = population.reduce((sum, genome) => sum + genome.weights.length, 0);
  const packed = Buffer.allocUnsafe(totalFloats * Float32Array.BYTES_PER_ELEMENT);
  let byteOffset = 0;
  for (const genome of population) {
    for (let index = 0; index < genome.weights.length; index++) {
      packed.writeFloatLE(genome.weights[index]!, byteOffset);
      byteOffset += Float32Array.BYTES_PER_ELEMENT;
    }
  }
  return packed;
}

/**
 * Shuffle packed Float32 bytes by byte significance.
 * @param raw - Little-endian packed Float32 data.
 * @returns Four byte planes suitable for compression.
 */
export function shuffleFloat32Bytes(raw: Buffer): Buffer {
  if (raw.length % Float32Array.BYTES_PER_ELEMENT !== 0) {
    throw new TypeError('Float32 byte shuffle requires a multiple of four bytes');
  }
  const count = raw.length / Float32Array.BYTES_PER_ELEMENT;
  const shuffled = Buffer.allocUnsafe(raw.length);
  for (let byte = 0; byte < Float32Array.BYTES_PER_ELEMENT; byte++) {
    const planeOffset = byte * count;
    for (let index = 0; index < count; index++) {
      shuffled[planeOffset + index] = raw[index * Float32Array.BYTES_PER_ELEMENT + byte]!;
    }
  }
  return shuffled;
}

/**
 * Reverse `shuffleFloat32Bytes`.
 * @param shuffled - Four byte planes.
 * @returns Original packed Float32 bytes.
 */
export function unshuffleFloat32Bytes(shuffled: Buffer): Buffer {
  if (shuffled.length % Float32Array.BYTES_PER_ELEMENT !== 0) {
    throw new TypeError('Float32 byte unshuffle requires a multiple of four bytes');
  }
  const count = shuffled.length / Float32Array.BYTES_PER_ELEMENT;
  const raw = Buffer.allocUnsafe(shuffled.length);
  for (let byte = 0; byte < Float32Array.BYTES_PER_ELEMENT; byte++) {
    const planeOffset = byte * count;
    for (let index = 0; index < count; index++) {
      raw[index * Float32Array.BYTES_PER_ELEMENT + byte] = shuffled[planeOffset + index]!;
    }
  }
  return raw;
}

/**
 * Produce a deterministic evolved-like population with the real crossover and
 * mutation operators. It avoids running thousands of physics steps solely to
 * obtain representative post-evolution weight entropy.
 * @param source - Fresh production population.
 * @param arch - Compiled architecture definition.
 * @param generations - Number of operator generations.
 * @returns Independent evolved-like genomes.
 */
export function evolvePopulationFixture(
  source: readonly Genome[],
  arch: ArchDefinition,
  generations = 25
): Genome[] {
  const rng = new StatefulRng(STAGE2_EVOLUTION_SEED);
  let population = source.map(genome => genome.clone());
  for (let generation = 0; generation < generations; generation++) {
    population = population.map((genome, index) => {
      const mate = population[(index + generation + 1) % population.length] ?? genome;
      const child = crossover(genome, mate, arch, rng.asSource());
      mutate(child, arch, rng);
      return child;
    });
  }
  return population;
}

/**
 * Describe a population in a content-addressable, architecture-aware form.
 * @param population - Dense population.
 * @param arch - Population architecture.
 * @returns Counts and packed-data SHA-256.
 */
export function describePopulation(
  population: readonly Genome[],
  arch: ArchDefinition
): {
  populationCount: number;
  weightsPerGenome: number;
  totalWeightCount: number;
  rawWeightBytes: number;
  rawSha256: string;
  architectureKey: string;
  parameterCount: number;
} {
  const packed = packPopulationWeights(population);
  return {
    populationCount: population.length,
    weightsPerGenome: population[0]?.weights.length ?? 0,
    totalWeightCount: packed.length / Float32Array.BYTES_PER_ELEMENT,
    rawWeightBytes: packed.length,
    rawSha256: createHash('sha256').update(packed).digest('hex'),
    architectureKey: arch.key,
    parameterCount: enrichArchInfo(arch).totalCount
  };
}
