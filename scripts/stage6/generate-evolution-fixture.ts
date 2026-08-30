/** Generate a retained current-TypeScript generation-evolution fixture. */

import type { GraphSpec } from '../../src/brains/graph/schema.ts';
import type { GenerationBoundaryState, World as WorldType } from '../../src/world.ts';

/** Git revision whose relevant TypeScript source is executed by this fixture. */
const SOURCE_REVISION = '7925faf7aef33bd3de3e1b6d3c021c4320a8dd68';
/** Fixed run seed. */
const SEED = 0x55667788;
/** Small recurrent graph that still exercises ordinary and unit crossover. */
const GRAPH: GraphSpec = {
  type: 'graph',
  nodes: [
    { id: 'input', type: 'Input', outputSize: 83 },
    { id: 'features', type: 'MLP', inputSize: 83, hiddenSizes: [3], outputSize: 3 },
    { id: 'memory', type: 'GRU', inputSize: 3, hiddenSize: 2 },
    { id: 'head', type: 'Dense', inputSize: 2, outputSize: 2 }
  ],
  edges: [
    { from: 'input', to: 'features' },
    { from: 'features', to: 'memory' },
    { from: 'memory', to: 'head' }
  ],
  outputs: [{ nodeId: 'head' }],
  outputSize: 2
};

/** Exact behavior values used by this fixture. */
const EVOLUTION = {
  eliteFrac: 0.25,
  mutationRate: 0.2,
  mutationStd: 0.17,
  crossoverRate: 0.7,
  gruMutationRate: 0.3,
  gruMutationStd: 0.11,
  gruCrossoverMode: 1,
  snakeStartLen: 5,
  fitnessSurvivalPerSecond: 0.7,
  fitnessFood: 80,
  fitnessLengthPerSegment: 100,
  fitnessKill: 400,
  fitnessPointsNorm: 42,
  fitnessTopPointsBonus: 600
} as const;

/** Encode one Float32 by its exact big-endian-readable bit pattern. */
function float32Hex(value: number): string {
  const buffer = new ArrayBuffer(4);
  const view = new DataView(buffer);
  view.setFloat32(0, value, false);
  return `0x${view.getUint32(0, false).toString(16).padStart(8, '0')}`;
}

/** Capture only the fields Rust evolution consumes from a population snake. */
function snakeInput(world: WorldType, slot: number): object {
  const snake = world.snakes[slot];
  if (!snake) throw new Error(`missing population snake ${slot}`);
  return {
    slot,
    id: snake.id,
    ageSeconds: snake.age,
    food: snake.foodEaten,
    points: snake.pointsScore,
    kills: snake.killScore,
    bodyLength: snake.points.length
  };
}

const originalConsoleInfo = console.info;
const originalConsoleLog = console.log;
console.info = () => {};
console.log = () => {};
const configModule = await import('../../src/config.ts');
configModule.resetCFGToDefaults();
const CFG = configModule.CFG;
try {
  const { World } = await import('../../src/world.ts');
  CFG.brain.graphSpec = GRAPH;
  CFG.baselineBots.count = 0;
  CFG.pelletCountTarget = 0;
  CFG.eliteFrac = EVOLUTION.eliteFrac;
  CFG.mutationRate = EVOLUTION.mutationRate;
  CFG.mutationStd = EVOLUTION.mutationStd;
  CFG.crossoverRate = EVOLUTION.crossoverRate;
  CFG.brain.gruMutationRate = EVOLUTION.gruMutationRate;
  CFG.brain.gruMutationStd = EVOLUTION.gruMutationStd;
  CFG.brain.gruCrossoverMode = EVOLUTION.gruCrossoverMode;
  CFG.snakeStartLen = EVOLUTION.snakeStartLen;
  CFG.reward.fitnessSurvivalPerSecond = EVOLUTION.fitnessSurvivalPerSecond;
  CFG.reward.fitnessFood = EVOLUTION.fitnessFood;
  CFG.reward.fitnessLengthPerSegment = EVOLUTION.fitnessLengthPerSegment;
  CFG.reward.fitnessKill = EVOLUTION.fitnessKill;
  CFG.reward.fitnessPointsNorm = EVOLUTION.fitnessPointsNorm;
  CFG.reward.fitnessTopPointsBonus = EVOLUTION.fitnessTopPointsBonus;

  let generationBoundary: {
    boundary: GenerationBoundaryState;
    population: Array<{ fitness: number; weightBits: string[] }>;
    history: object;
    hallOfFame: object;
    bestFitnessEver: number;
  } | null = null;
  const world = new World(
    {
      snakeCount: 4,
      hiddenLayers: 1,
      neurons1: 3,
      neurons2: 3,
      neurons3: 3,
      neurons4: 3,
      neurons5: 3,
      worldRadius: 3500,
      simSpeed: 1
    },
    {
      seed: SEED,
      runId: 'stage6-evolution-fixture',
      onGenerationBoundary: (boundary, current) => {
        if (boundary.kind !== 'generation') return;
        const history = current.fitnessHistory[current.fitnessHistory.length - 1];
        if (!history || !current._lastHoFEntry) {
          throw new Error('generation summary or Hall-of-Fame candidate missing');
        }
        generationBoundary = {
          boundary,
          population: current.population.map(genome => ({
            fitness: genome.fitness,
            weightBits: Array.from(genome.weights, float32Hex)
          })),
          history: { ...history },
          hallOfFame: {
            gen: current._lastHoFEntry.gen,
            seed: current._lastHoFEntry.seed,
            fitness: current._lastHoFEntry.fitness,
            points: current._lastHoFEntry.points,
            length: current._lastHoFEntry.length,
            weightBits: current._lastHoFEntry.genome.weights.map(float32Hex)
          },
          bestFitnessEver: current.bestFitnessEver
        };
      }
    }
  );

  for (let slot = 0; slot < world.population.length; slot++) {
    const snake = world.snakes[slot];
    if (!snake) throw new Error(`missing source population snake ${slot}`);
    snake.age = 12 + slot;
    snake.foodEaten = slot * 2;
    snake.killScore = slot % 3;
    snake.pointsScore = 5 + slot * 7;
    const tail = snake.points[snake.points.length - 1];
    if (!tail) throw new Error(`missing source tail ${slot}`);
    for (let extra = 0; extra < slot; extra++) {
      snake.points.push({ x: tail.x - (extra + 1) * 7.5, y: tail.y });
    }
  }

  const source = {
    generation: world.generation,
    bestFitnessEver: world.bestFitnessEver,
    evolutionRng: world.evolutionRng.exportState(),
    snakes: world.population.map((_genome, slot) => snakeInput(world, slot)),
    population: world.population.map((genome, slot) => ({
      slot,
      fitness: genome.fitness,
      weightBits: Array.from(genome.weights, float32Hex)
    }))
  };
  world._endGeneration(123);
  if (!generationBoundary) throw new Error('generation boundary was not emitted');

  const fixture = {
    evidenceKind: 'current-source execution',
    sourceRevision: SOURCE_REVISION,
    command:
      'node .\\node_modules\\tsx\\dist\\cli.mjs scripts\\stage6\\generate-evolution-fixture.ts',
    seed: `0x${SEED.toString(16).padStart(8, '0')}`,
    graph: GRAPH,
    evolution: EVOLUTION,
    source,
    expected: generationBoundary
  };
  process.stdout.write(`${JSON.stringify(fixture, null, 2)}\n`);
} finally {
  console.info = originalConsoleInfo;
  console.log = originalConsoleLog;
  configModule.resetCFGToDefaults();
}
