/** Browser settings drafts and presentation defaults; Rust owns game configuration. */

import { deepClone } from './utils.ts';
import { getSensorLayout, type SensorLayout, type SensorLayoutVersion } from './protocol/sensors.ts';
import { defaultStackBrain } from './brains/stackBuilder.ts';
import type { GraphSpec } from './brains/graph/schema.ts';

/** Initial browser settings/presentation values, replaced by authoritative metadata. */
export const CFG_DEFAULT = {
  worldRadius: 3500,
  pelletCountTarget: 3500,
  pelletSpawnPerSecond: 170,
  snakeBaseSpeed: 165,
  snakeBoostSpeed: 500,
  snakeTurnRate: 3.2,
  snakeRadius: 9,
  snakeRadiusMax: 18,
  snakeThicknessScale: 2.9,
  snakeThicknessLogDiv: 30,
  snakeSpacing: 7.5,
  snakeStartLen: 5,
  snakeMaxLen: 10000,
  snakeMinLen: 4,
  snakeSizeSpeedPenalty: 0.18,
  snakeBoostSizePenalty: 0.28,
  foodValue: 1.0,
  growPerFood: 1.0,
  foodSpawn: {
    // Toggle the radial falloff used by ambient pellet spawning.
    edgeFalloffEnabled: true,
    // Radius fraction where edge fade begins (gentle -> sharp falloff).
    edgeFadeStart: 0.35,
    // Exponent applied after smoothstep to sharpen the edge fade.
    edgeFadePower: 2.6,
    // Contrast exponent for ridged filaments (higher = thinner filaments).
    filamentPower: 4.2,
    // Domain warp frequency for twisting the filaments.
    warpFreq: 0.0013,
    // Domain warp scale as a fraction of world radius.
    warpScale: 0.08,
    // Filament feature scales.
    freqLarge: 0.0026,
    freqMedium: 0.0042,
    freqSmall: 0.0068,
    // Speckle strength added to the web.
    dustStrength: 0.35
  },
  generationSeconds: 240,
  eliteFrac: 0.12,
  // With larger input vectors and higher-capacity brains, defaults that were
  // reasonable for tiny networks become overly destructive. These are tuned
  // for incremental improvement on ~10k parameter controllers.
  mutationRate: 0.03,
  mutationStd: 0.35,
  crossoverRate: 0.85,
  observer: {
    focusRecheckSeconds: 1.0,
    focusSwitchMargin: 1.08,
    earlyEndMinSeconds: 8,
    earlyEndAliveThreshold: 2,
    overviewPadding: 1.10,
    snapZoomOutInOverview: true,
    zoomLerpFollow: 0.09,
    zoomLerpOverview: 0.14,
    overviewExtraWorldMargin: 160
  },
  sense: {
    // 360° "bubble" sensing around the head.
    // The bubble radius increases with snake length using the same zoom curve
    // as the follow camera (larger snakes see farther).
    layoutVersion: 'v3' as SensorLayoutVersion,
    bubbleBins: 16,
    rNearBase: 520,
    rNearScale: 260,
    rNearMin: 420,
    rNearMax: 1100,
    rFarBase: 1200,
    rFarScale: 520,
    rFarMin: 900,
    rFarMax: 2400,
    foodKBase: 4.0,
    // Enable sensor debug logging when true.
    debug: false,

    // Caps on work per snake per tick when the local region is extremely dense.
    // These apply to bubble food/hazard sensing.
    maxPelletChecks: 900,
    maxSegmentChecks: 2200
  },
  // Brain configuration.
  // Input size is derived from the active sensor layout.
  brain: {
    ...defaultStackBrain(getSensorLayout(16).inputSize),
    graphSpec: null as GraphSpec | null,

    // Brain is evaluated on a fixed controller timestep independent of physics substeps.
    // This stabilises what “memory length” means when collision substepping changes.
    controlDt: 1 / 60,

    // Genetic operator tuning for GRU parameters.
    // Defaults are conservative; use the sliders to explore.
    gruMutationRate: 0.025,
    gruMutationStd: 0.22,

    // GRU crossover is block-structured; 0 means inherit the entire GRU block
    // from one parent, 1 means unit-wise row crossover.
    gruCrossoverMode: 1,

    // Initial bias for the GRU update gate. More negative means longer default memory.
    gruInitUpdateBias: -0.7,
    // Initial bias for the LSTM forget gate.
    lstmInitForgetBias: 0.6,
    // Initial bias for the RRU gate.
    rruInitGateBias: 0.1
  },
  baselineBots: {
    count: 10,
    seed: 1,
    randomizeSeedPerGen: false,
    respawnDelay: 20.0
  },
  collision: {
    substepMaxDt: 0.006,
    skipSegments: 0,
    hitScale: 0.82,
    cellSize: 70,
    neighborRange: 1
  },
  boost: {
    minPointsToBoost: 1.2,
    pointsCostPerSecond: 7.0,
    pointsCostSizeFactor: 1.1,
    lenLossPerPoint: 0.16,
    pelletValueFactor: 0.65,
    pelletJitter: 10
  },
  reward: {
    pointsPerFood: 20.0,
    pointsPerKill: 400.0,
    pointsPerSecondAlive: 0.60,
    fitnessSurvivalPerSecond: 0.70,
    fitnessFood: 80.0,
    fitnessLengthPerSegment: 100.0,
    fitnessKill: 400.0,
    fitnessPointsNorm: 42.0,
    fitnessTopPointsBonus: 600.0
  }
};

/** Mutable configuration object, cloned from CFG_DEFAULT on reset. */
export let CFG = deepClone(CFG_DEFAULT);

/** Track whether the v3 presentation layout log has been emitted. */
let didLogV3Layout = false;

/**
 * Resets the global configuration to its default values.
 */
export function resetCFGToDefaults(): void {
  CFG = deepClone(CFG_DEFAULT);
  syncBrainInputSize();
}

/**
 * Emit a one-time log when the v3 layout is active.
 * @param layout - Active sensor layout metadata.
 */
function logV3LayoutOnce(layout: SensorLayout): void {
  if (didLogV3Layout) return;
  if (layout.layoutVersion !== 'v3') return;
  console.info('[sensors.layout.v3_enabled]', {
    bins: layout.bins,
    scalarCount: layout.scalarCount,
    inputSize: layout.inputSize
  });
  didLogV3Layout = true;
}

/**
 * Align the brain input size with the active sensor layout.
 */
export function syncBrainInputSize(): void {
  const sense = CFG.sense ?? {};
  const layout = getSensorLayout(sense.bubbleBins ?? 16);
  CFG.brain.inSize = layout.inputSize;
  logV3LayoutOnce(layout);
}
