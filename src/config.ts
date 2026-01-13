// config.ts
// Default configuration values and mutable configuration state for the simulation.

import { deepClone } from './utils.ts';
import { getSensorLayout, type SensorLayout, type SensorLayoutVersion } from './protocol/sensors.ts';
import type { GraphSpec } from './brains/graph/schema.ts';

/** Default configuration values for the simulation and UI sliders. */
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
  /** Multiplier for turn rate decay based on length. */
  snakeTurnPenalty: 1.4,
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
    defaultViewMode: "overview",
    overviewPadding: 1.10,
    snapZoomOutInOverview: true,
    zoomLerpFollow: 0.09,
    zoomLerpOverview: 0.14,
    overviewExtraWorldMargin: 160
  },
  pelletGrid: {
    // Spatial hash for pellets used by sensing and eating.
    // Larger cells reduce bookkeeping, smaller cells reduce per-query scan.
    cellSize: 120
  },
  sense: {
    // 360° "bubble" sensing around the head.
    // The bubble radius increases with snake length using the same zoom curve
    // as the follow camera (larger snakes see farther).
    layoutVersion: 'v2' as SensorLayoutVersion,
    bubbleBins: 16,
    bubbleRadiusBase: 760,
    bubbleRadiusMin: 420,
    bubbleRadiusMax: 1700,
    // Saturation constant for per-bin food accumulation.
    bubbleFoodK: 4.0,
    // V2 sensing radii (near/far) and food saturation controls.
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
    maxSegmentChecks: 2200,

    // Legacy parameters retained for compatibility with older sensor code.
    rayLen: 420,
    coneOffset: 0.75,
    coneHalfAngle: 0.42,
    nearestPelletRadius: 900,
    wallRayLen: 720
  },
  // Brain configuration.
  // Input size is derived from the active sensor layout.
  brain: {
    inSize: getSensorLayout(16, 'v2').inputSize,
    outSize: 2,

    // Recurrent memory.
    // Stackable memory units sit after the MLP feature extractor.
    useMlp: true,
    stack: {
      gru: 1,
      lstm: 0,
      rru: 0
    },
    stackOrder: ["gru", "lstm", "rru"],
    graphSpec: null as GraphSpec | null,

    // GRU hidden state size.
    gruHidden: 16,
    lstmHidden: 16,
    rruHidden: 16,

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
  },
  // Death-to-pellets conversion tuned to resemble slither.io: smaller snakes
  // recycle a higher fraction of their mass; very large snakes recycle less.
  // Total dropped pellet value is derived from "mass" (segment count) and growPerFood.
  death: {
    dropFracSmall: 0.95,
    dropFracLarge: 0.33,
    dropFracPow: 1.6,
    bigPelletValueFactor: 3.0,
    smallPelletValueFactor: 1.0,
    bigShare: 0.78,
    jitter: 8,
    clusterJitter: 14,
    maxPellets: 420,
    useSnakeColor: true
  },
  dtClamp: 0.01
};

/** Mutable configuration object, cloned from CFG_DEFAULT on reset. */
export let CFG = deepClone(CFG_DEFAULT);

/** Track whether the default v2 layout log has been emitted. */
let didLogDefaultV2Layout = false;

/**
 * Resets the global configuration to its default values.
 */
export function resetCFGToDefaults(): void {
  CFG = deepClone(CFG_DEFAULT);
  syncBrainInputSize();
}

/**
 * Emit a one-time log when the default v2 layout is active.
 * @param layout - Active sensor layout metadata.
 */
function logDefaultV2LayoutOnce(layout: SensorLayout): void {
  if (didLogDefaultV2Layout) return;
  if (layout.layoutVersion !== 'v2') return;
  console.info('[sensors.layout.default_v2_enabled]', {
    bins: layout.bins,
    inputSize: layout.inputSize
  });
  didLogDefaultV2Layout = true;
}

/**
 * Align the brain input size with the active sensor layout.
 */
export function syncBrainInputSize(): void {
  const sense = CFG.sense ?? {};
  const layoutVersion: SensorLayoutVersion = sense.layoutVersion ?? 'v2';
  const layout = getSensorLayout(sense.bubbleBins ?? 16, layoutVersion);
  CFG.brain.inSize = layout.inputSize;
  logDefaultV2LayoutOnce(layout);
}
