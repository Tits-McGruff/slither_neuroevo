/** Cross-cutting fixed-seed behavior evidence for the TypeScript reference. */

import { createHash } from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { compileGraph } from '../../src/brains/graph/compiler.ts';
import { resetCFGToDefaults } from '../../src/config.ts';
import { hashConfig } from '../../server/hash.ts';
import { buildGenerationCheckpoint } from '../../server/checkpoint.ts';
import type { PopulationCheckpoint } from '../../server/persistence.ts';
import {
  captureAuthoritativeWorldDigest
} from '../../server/test/authoritativeWorldDigest.ts';
import { World, type GenerationBoundaryState } from '../../src/world.ts';
import { WorldSerializer } from '../../src/serializer.ts';
import {
  describePopulation,
  installStage2Scenario,
  STAGE2_WORLD_SEED
} from './fixtures.ts';

/** Optional output file parsed from the command line. */
interface BehaviorOptions {
  /** JSON artifact destination, or null for standard output. */
  outputPath: string | null;
}

/**
 * Parse the sole optional output argument.
 * @param argv - Arguments after script path.
 * @returns Validated output option.
 */
function parseOptions(argv: readonly string[]): BehaviorOptions {
  if (argv.length === 0) return { outputPath: null };
  if (argv.length !== 2 || argv[0] !== '--output' || !argv[1]) {
    throw new Error('Usage: behavior-baseline.ts [--output PATH]');
  }
  return { outputPath: path.resolve(argv[1]) };
}

/**
 * Return raw IEEE-754 Float32 words as hexadecimal strings.
 * @param values - Float32 values.
 * @returns Big-endian display words preserving exact bits.
 */
function float32Words(values: Float32Array): string[] {
  const scratch = new ArrayBuffer(4);
  const view = new DataView(scratch);
  return Array.from(values, value => {
    view.setFloat32(0, value, false);
    return `0x${view.getUint32(0, false).toString(16).padStart(8, '0')}`;
  });
}

/**
 * Hash one Float32 vector by explicit little-endian words.
 * @param values - Vector to hash.
 * @returns SHA-256.
 */
function hashFloat32(values: Float32Array): string {
  const bytes = Buffer.allocUnsafe(values.length * 4);
  for (let index = 0; index < values.length; index++) bytes.writeFloatLE(values[index]!, index * 4);
  return createHash('sha256').update(bytes).digest('hex');
}

/**
 * Read exact source identity without changing Git.
 * @returns Commit and dirty flag.
 */
function sourceIdentity(): { commit: string; dirty: boolean } {
  const commit = spawnSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' });
  const status = spawnSync('git', ['status', '--porcelain'], { encoding: 'utf8' });
  return {
    commit: commit.status === 0 ? commit.stdout.trim() : 'unavailable',
    dirty: status.status !== 0 || status.stdout.trim().length > 0
  };
}

/**
 * Collect the Stage 2 cross-cutting fixture corpus.
 * @returns Behavior evidence object.
 */
async function captureBehaviorBaseline(): Promise<Record<string, unknown>> {
  const scenario = installStage2Scenario('P0');
  let runStartCheckpoint: PopulationCheckpoint | null = null;
  let generationCheckpoint: PopulationCheckpoint | null = null;
  const onInitialBoundary = (boundary: GenerationBoundaryState, candidate: World): void => {
    const checkpoint = buildGenerationCheckpoint(candidate, boundary, 0);
    if (boundary.kind === 'run-start') runStartCheckpoint = checkpoint;
  };
  const world = new World(scenario.settings, {
    seed: STAGE2_WORLD_SEED,
    runId: 'stage2-behavior-p0',
    inferenceBackend: 'js',
    onGenerationBoundary: onInitialBoundary
  });
  if (!runStartCheckpoint) throw new Error('run-start checkpoint was not captured');
  const compiled = compileGraph(world.arch.spec);
  const initialDigest = captureAuthoritativeWorldDigest(world);
  const initialFrame = WorldSerializer.serialize(world);
  const firstSnake = world.snakes[0];
  if (!firstSnake) throw new Error('fixture has no first snake');
  const sensors = firstSnake.computeSensors(world).slice();
  const fixtureBrain = world.population[0]!.buildBrain(world.arch, 'js');
  const brainSequence = [];
  for (let index = 0; index < 4; index++) {
    const input = sensors.slice();
    input[index % input.length] = Math.fround((input[index % input.length] ?? 0) + index * 0.03125);
    const output = fixtureBrain.forward(input).slice();
    brainSequence.push({
      index,
      inputSha256: hashFloat32(input),
      outputWords: float32Words(output),
      outputSha256: hashFloat32(output)
    });
  }
  await world.step(1 / 60, world.worldRadius * 2, world.worldRadius * 2, undefined, 1);
  const afterOneStep = captureAuthoritativeWorldDigest(world);
  const afterOneStepFrame = WorldSerializer.serialize(world);

  const generationWorld = new World(scenario.settings, {
    seed: STAGE2_WORLD_SEED,
    runId: 'stage2-generation-p0',
    inferenceBackend: 'js',
    onGenerationBoundary: (boundary, candidate) => {
      if (boundary.kind === 'generation') {
        generationCheckpoint = buildGenerationCheckpoint(candidate, boundary, 0);
      }
    }
  });
  for (let slot = 0; slot < generationWorld.population.length; slot++) {
    const snake = generationWorld.snakes[slot]!;
    snake.age = slot * 0.25;
    snake.foodEaten = slot % 7;
    snake.killScore = slot % 3;
    snake.pointsScore = slot * 1.5;
  }
  generationWorld._endGeneration(1);
  if (!generationCheckpoint) throw new Error('generation checkpoint was not captured');
  const runStart = runStartCheckpoint as PopulationCheckpoint;
  const generation = generationCheckpoint as PopulationCheckpoint;
  return {
    schema: 'slither-stage2-behavior-baseline',
    version: 1,
    evidenceClass: 'new reproducible fixture',
    source: sourceIdentity(),
    environment: {
      capturedAt: new Date().toISOString(),
      platform: process.platform,
      architecture: process.arch,
      osType: os.type(),
      osRelease: os.release(),
      locale: Intl.DateTimeFormat().resolvedOptions().locale,
      node: process.version,
      v8: process.versions.v8
    },
    fixture: {
      classification: 'preserve except for defects separately identified by correction fixtures',
      seed: STAGE2_WORLD_SEED,
      scenario,
      runId: world.runId
    },
    configurationAndGraph: {
      architectureKey: world.archKey,
      graphOrder: compiled.order,
      parameterCount: compiled.totalParams,
      recurrentStateFloats: compiled.totalStateSize,
      nodes: compiled.nodes,
      outputs: compiled.outputs
    },
    rngAtConstructedBoundary: world.exportRngState(),
    populationAtConstructedBoundary: describePopulation(world.population, world.arch),
    initialWorld: {
      digestVersion: initialDigest.version,
      digest: initialDigest.digest,
      entryCount: initialDigest.entries.length,
      frameBytes: initialFrame.byteLength,
      frameSha256: hashFloat32(initialFrame)
    },
    firstDeliveredObservationCandidate: {
      snakeId: firstSnake.id,
      layoutVersion: 'v3',
      length: sensors.length,
      words: float32Words(sensors),
      sha256: hashFloat32(sensors)
    },
    heterogeneousBrainSequence: brainSequence,
    afterOneCompleteStep: {
      tick: world.tickId,
      digest: afterOneStep.digest,
      entryCount: afterOneStep.entries.length,
      frameBytes: afterOneStepFrame.byteLength,
      frameSha256: hashFloat32(afterOneStepFrame)
    },
    runStartCheckpointIdentity: {
      metadataHash: hashConfig(runStart.metadata),
      boundaryKind: runStart.metadata.boundaryKind,
      generation: runStart.metadata.generation,
      simulationStep: runStart.metadata.simulationStep,
      configHash: runStart.metadata.configHash,
      graphKey: runStart.metadata.archKey,
      populationCount: runStart.metadata.populationCount
    },
    generationBoundary: {
      metadataHash: hashConfig(generation.metadata),
      boundaryKind: generation.metadata.boundaryKind,
      generation: generation.metadata.generation,
      simulationStep: generation.metadata.simulationStep,
      history: generation.metadata.fitnessHistory,
      population: describePopulation(generationWorld.population, generationWorld.arch),
      worldDigest: captureAuthoritativeWorldDigest(generationWorld).digest
    }
  };
}

/** Execute the CLI. */
async function main(): Promise<void> {
  const options = parseOptions(process.argv.slice(2));
  try {
    const result = await captureBehaviorBaseline();
    const json = `${JSON.stringify(result, null, 2)}\n`;
    if (options.outputPath) {
      fs.mkdirSync(path.dirname(options.outputPath), { recursive: true });
      fs.writeFileSync(options.outputPath, json, 'utf8');
      console.info(`[stage2.behavior] wrote ${options.outputPath}`);
    } else {
      process.stdout.write(json);
    }
  } finally {
    resetCFGToDefaults();
  }
}

main().catch(error => {
  console.error(error);
  process.exitCode = 1;
});
