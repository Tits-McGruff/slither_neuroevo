/** Generate the compact selected-TypeScript Stage 6A P0 fresh-run oracle. */

import { createHash } from 'node:crypto';
import { CFG } from '../../src/config.ts';
import { deriveBotSeed } from '../../src/bots/baselineBots.ts';
import { graphKey } from '../../src/brains/graph/compiler.ts';
import { buildStackGraphSpec } from '../../src/brains/stackBuilder.ts';
import { Genome, enrichArchInfo } from '../../src/mlp.ts';
import { DEFAULT_CORE_SETTINGS } from '../../src/protocol/settings.ts';
import { deriveSeed, StatefulRng } from '../../src/rng.ts';

/** Clean Git revision containing every selected TypeScript source used here. */
const SOURCE_REVISION = '7925faf7aef33bd3de3e1b6d3c021c4320a8dd68';
/** Fixed compatibility seed; this is not a benchmark or production default. */
const SEED = 0x12345678;

/** Exact scalar record retained for one normalized setting. */
type FixtureNormalizedSetting =
  | { kind: 'bool'; value: boolean }
  | { kind: 'integer'; valueDecimal: string }
  | { kind: 'float'; valueHex: string };

/** Encode one complete finite Float32 weight vector as explicit little-endian bytes. */
function weightBytes(weights: Float32Array): Buffer {
  const bytes = Buffer.allocUnsafe(weights.length * 4);
  for (let index = 0; index < weights.length; index++) {
    bytes.writeFloatLE(weights[index]!, index * 4);
  }
  return bytes;
}

/** Return lowercase SHA-256 for one exact byte sequence. */
function sha256(bytes: Uint8Array): string {
  return createHash('sha256').update(bytes).digest('hex');
}

/** Encode one finite Float64 as an exact big-endian IEEE-754 bit pattern. */
function float64Hex(value: number): string {
  if (!Number.isFinite(value)) throw new TypeError('Fixture Float64 value must be finite');
  const bytes = Buffer.allocUnsafe(8);
  bytes.writeDoubleBE(value);
  return `0x${bytes.toString('hex')}`;
}

/** Retain a Boolean normalized setting without a numeric coercion. */
function boolSetting(value: boolean): FixtureNormalizedSetting {
  return { kind: 'bool', value };
}

/** Retain a safe-integer normalized setting as canonical base-ten text. */
function integerSetting(value: number): FixtureNormalizedSetting {
  if (!Number.isSafeInteger(value)) throw new TypeError('Fixture integer must be safe and integral');
  return { kind: 'integer', valueDecimal: String(value) };
}

/** Retain a normalized Float64 setting as exact bits, including integral floats. */
function floatSetting(value: number): FixtureNormalizedSetting {
  return { kind: 'float', valueHex: float64Hex(value) };
}

const graph = buildStackGraphSpec(DEFAULT_CORE_SETTINGS, CFG);
const architecture = { spec: graph, key: graphKey(graph) };
const info = enrichArchInfo(architecture);
const evolution = new StatefulRng(deriveSeed(SEED, 'evolution'));
const populationHasher = createHash('sha256');
const genomeWeightSha256: string[] = [];

for (let slot = 0; slot < DEFAULT_CORE_SETTINGS.snakeCount; slot++) {
  const genome = Genome.random(architecture, evolution.asSource());
  const bytes = weightBytes(genome.weights);
  genomeWeightSha256.push(sha256(bytes));
  populationHasher.update(bytes);
}

const baselineStateHex = Array.from({ length: CFG.baselineBots.count }, (_, slot) => {
  const seed = deriveBotSeed(
    CFG.baselineBots.seed,
    1,
    slot,
    CFG.baselineBots.randomizeSeedPerGen,
    SEED
  );
  return new StatefulRng(seed).exportState().stateHex;
});

/** Every current-TypeScript scalar consumed by the fixed Rust P0 profile. */
const normalizedSettings = Object.fromEntries(
  Object.entries({
    'baselineBots.count': integerSetting(CFG.baselineBots.count),
    'baselineBots.randomizeSeedPerGen': boolSetting(CFG.baselineBots.randomizeSeedPerGen),
    'baselineBots.respawnDelay': floatSetting(CFG.baselineBots.respawnDelay),
    'baselineBots.seed': integerSetting(CFG.baselineBots.seed),
    'boost.lenLossPerPoint': floatSetting(CFG.boost.lenLossPerPoint),
    'boost.minPointsToBoost': floatSetting(CFG.boost.minPointsToBoost),
    'boost.pelletJitter': floatSetting(CFG.boost.pelletJitter),
    'boost.pelletValueFactor': floatSetting(CFG.boost.pelletValueFactor),
    'boost.pointsCostPerSecond': floatSetting(CFG.boost.pointsCostPerSecond),
    'boost.pointsCostSizeFactor': floatSetting(CFG.boost.pointsCostSizeFactor),
    'brain.controlDt': floatSetting(CFG.brain.controlDt),
    'brain.gruCrossoverMode': integerSetting(CFG.brain.gruCrossoverMode),
    'brain.gruInitUpdateBias': floatSetting(CFG.brain.gruInitUpdateBias),
    'brain.gruMutationRate': floatSetting(CFG.brain.gruMutationRate),
    'brain.gruMutationStd': floatSetting(CFG.brain.gruMutationStd),
    'brain.lstmInitForgetBias': floatSetting(CFG.brain.lstmInitForgetBias),
    'brain.rruInitGateBias': floatSetting(CFG.brain.rruInitGateBias),
    'brain.sensorVersion': integerSetting(3),
    'collision.cellSize': floatSetting(CFG.collision.cellSize),
    'collision.hitScale': floatSetting(CFG.collision.hitScale),
    'collision.neighborRange': integerSetting(CFG.collision.neighborRange),
    'collision.skipSegments': integerSetting(CFG.collision.skipSegments),
    'collision.substepMaxDt': floatSetting(CFG.collision.substepMaxDt),
    crossoverRate: floatSetting(CFG.crossoverRate),
    'death.bigPelletValueFactor': floatSetting(CFG.death.bigPelletValueFactor),
    'death.bigShare': floatSetting(CFG.death.bigShare),
    'death.clusterJitter': floatSetting(CFG.death.clusterJitter),
    'death.dropFracLarge': floatSetting(CFG.death.dropFracLarge),
    'death.dropFracPow': floatSetting(CFG.death.dropFracPow),
    'death.dropFracSmall': floatSetting(CFG.death.dropFracSmall),
    'death.jitter': floatSetting(CFG.death.jitter),
    'death.maxPellets': integerSetting(CFG.death.maxPellets),
    'death.smallPelletValueFactor': floatSetting(CFG.death.smallPelletValueFactor),
    'death.useSnakeColor': boolSetting(CFG.death.useSnakeColor),
    'foodSpawn.dustStrength': floatSetting(CFG.foodSpawn.dustStrength),
    'foodSpawn.edgeFadePower': floatSetting(CFG.foodSpawn.edgeFadePower),
    'foodSpawn.edgeFadeStart': floatSetting(CFG.foodSpawn.edgeFadeStart),
    'foodSpawn.edgeFalloffEnabled': boolSetting(CFG.foodSpawn.edgeFalloffEnabled),
    'foodSpawn.filamentPower': floatSetting(CFG.foodSpawn.filamentPower),
    'foodSpawn.freqLarge': floatSetting(CFG.foodSpawn.freqLarge),
    'foodSpawn.freqMedium': floatSetting(CFG.foodSpawn.freqMedium),
    'foodSpawn.freqSmall': floatSetting(CFG.foodSpawn.freqSmall),
    'foodSpawn.warpFreq': floatSetting(CFG.foodSpawn.warpFreq),
    'foodSpawn.warpScale': floatSetting(CFG.foodSpawn.warpScale),
    foodValue: floatSetting(CFG.foodValue),
    generationSeconds: floatSetting(CFG.generationSeconds),
    growPerFood: floatSetting(CFG.growPerFood),
    eliteFrac: floatSetting(CFG.eliteFrac),
    mutationRate: floatSetting(CFG.mutationRate),
    mutationStd: floatSetting(CFG.mutationStd),
    'observer.earlyEndAliveThreshold': integerSetting(CFG.observer.earlyEndAliveThreshold),
    'observer.earlyEndMinSeconds': floatSetting(CFG.observer.earlyEndMinSeconds),
    'pelletGrid.cellSize': floatSetting(CFG.pelletGrid.cellSize),
    pelletCountTarget: integerSetting(CFG.pelletCountTarget),
    pelletSpawnPerSecond: floatSetting(CFG.pelletSpawnPerSecond),
    'reward.fitnessFood': floatSetting(CFG.reward.fitnessFood),
    'reward.fitnessKill': floatSetting(CFG.reward.fitnessKill),
    'reward.fitnessLengthPerSegment': floatSetting(CFG.reward.fitnessLengthPerSegment),
    'reward.fitnessPointsNorm': floatSetting(CFG.reward.fitnessPointsNorm),
    'reward.fitnessSurvivalPerSecond': floatSetting(CFG.reward.fitnessSurvivalPerSecond),
    'reward.fitnessTopPointsBonus': floatSetting(CFG.reward.fitnessTopPointsBonus),
    'reward.pointsPerFood': floatSetting(CFG.reward.pointsPerFood),
    'reward.pointsPerKill': floatSetting(CFG.reward.pointsPerKill),
    'reward.pointsPerSecondAlive': floatSetting(CFG.reward.pointsPerSecondAlive),
    'sense.bubbleBins': integerSetting(CFG.sense.bubbleBins),
    'sense.foodKBase': floatSetting(CFG.sense.foodKBase),
    'sense.layoutVersion': integerSetting(CFG.sense.layoutVersion === 'v3' ? 3 : 0),
    'sense.maxPelletChecks': integerSetting(CFG.sense.maxPelletChecks),
    'sense.maxSegmentChecks': integerSetting(CFG.sense.maxSegmentChecks),
    'sense.rFarBase': floatSetting(CFG.sense.rFarBase),
    'sense.rFarMax': floatSetting(CFG.sense.rFarMax),
    'sense.rFarMin': floatSetting(CFG.sense.rFarMin),
    'sense.rFarScale': floatSetting(CFG.sense.rFarScale),
    'sense.rNearBase': floatSetting(CFG.sense.rNearBase),
    'sense.rNearMax': floatSetting(CFG.sense.rNearMax),
    'sense.rNearMin': floatSetting(CFG.sense.rNearMin),
    'sense.rNearScale': floatSetting(CFG.sense.rNearScale),
    simSpeed: floatSetting(DEFAULT_CORE_SETTINGS.simSpeed),
    snakeBaseSpeed: floatSetting(CFG.snakeBaseSpeed),
    snakeBoostSizePenalty: floatSetting(CFG.snakeBoostSizePenalty),
    snakeBoostSpeed: floatSetting(CFG.snakeBoostSpeed),
    snakeCount: integerSetting(DEFAULT_CORE_SETTINGS.snakeCount),
    snakeMaxLen: integerSetting(CFG.snakeMaxLen),
    snakeMinLen: integerSetting(CFG.snakeMinLen),
    snakeRadius: floatSetting(CFG.snakeRadius),
    snakeRadiusMax: floatSetting(CFG.snakeRadiusMax),
    snakeSizeSpeedPenalty: floatSetting(CFG.snakeSizeSpeedPenalty),
    snakeSpacing: floatSetting(CFG.snakeSpacing),
    snakeStartLen: integerSetting(CFG.snakeStartLen),
    snakeThicknessLogDiv: floatSetting(CFG.snakeThicknessLogDiv),
    snakeThicknessScale: floatSetting(CFG.snakeThicknessScale),
    snakeTurnPenalty: floatSetting(CFG.snakeTurnPenalty),
    snakeTurnRate: floatSetting(CFG.snakeTurnRate),
    worldRadius: integerSetting(CFG.worldRadius)
  }).sort(([left], [right]) => (left < right ? -1 : left > right ? 1 : 0))
);

const fixture = {
  evidenceKind: 'selected TypeScript current-source execution',
  sourceRevision: SOURCE_REVISION,
  command:
    'node .\\node_modules\\tsx\\dist\\cli.mjs scripts\\stage6\\generate-fresh-run-reference.ts',
  seed: `0x${SEED.toString(16).padStart(8, '0')}`,
  typescriptGraphKey: architecture.key,
  graph,
  compiledNodeRanges: info.nodes.map(node => ({
    id: node.id,
    type: node.type,
    offset: node.offset,
    length: node.length
  })),
  totalParameters: info.totalCount,
  recurrentStateFloats: info.compiled.totalStateSize,
  normalizedSettings,
  populationCount: DEFAULT_CORE_SETTINGS.snakeCount,
  baselineCount: CFG.baselineBots.count,
  genomeWeightSha256,
  populationWeightSha256: populationHasher.digest('hex'),
  worldStateHex: new StatefulRng(deriveSeed(SEED, 'world')).exportState().stateHex,
  nextEvolutionStateHex: evolution.exportState().stateHex,
  externalControllerStateHex: new StatefulRng(
    deriveSeed(SEED, 'external-controller')
  ).exportState().stateHex,
  baselineStateHex
};

process.stdout.write(`${JSON.stringify(fixture, null, 2)}\n`);
