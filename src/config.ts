// config.ts
// Default configuration values and mutable configuration state for the simulation.

import { deepClone } from './utils.ts';

// Default configuration values.  These mirror the values from the original
// monolithic implementation and expose every adjustable parameter via
// sliders.  See settings.ts for the slider specifications.
export const CFG_DEFAULT = {
  worldRadius: 2400,
  pelletCountTarget: 2400,
  pelletSpawnPerSecond: 170,
  snakeBaseSpeed: 165,
  snakeBoostSpeed: 260,
  snakeTurnRate: 3.2,
  snakeRadius: 9,
  snakeRadiusMax: 18,
  snakeThicknessScale: 2.9,
  snakeThicknessLogDiv: 30,
  snakeSpacing: 7.5,
  snakeStartLen: 22,
  snakeMaxLen: 560,
  snakeMinLen: 8,
  snakeSizeSpeedPenalty: 0.18,
  snakeBoostSizePenalty: 0.28,
  foodValue: 1.0,
  growPerFood: 1.0,
  generationSeconds: 55,
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
  bubbleBins: 12,
  bubbleRadiusBase: 760,
  bubbleRadiusMin: 420,
  bubbleRadiusMax: 1700,
  // Saturation constant for per-bin food accumulation.
  bubbleFoodK: 4.0,

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
  // Input size is 5 + 3*bubbleBins as detailed in buildSensors.
  brain: {
    inSize: 41,
    outSize: 2,

    // Recurrent memory.
    // When enabled the controller is:
    // inputs -> MLP feature extractor -> GRU -> output heads.
    useGRU: 1,

    // GRU hidden state size.
    gruHidden: 16,

    // Brain is evaluated on a fixed controller timestep independent of physics substeps.
    // This stabilises what “memory length” means when collision substepping changes.
    controlDt: 1 / 60,

    // Genetic operator tuning for GRU parameters.
    // Defaults are conservative; use the sliders to explore.
    gruMutationRate: 0.025,
    gruMutationStd: 0.22,

    // GRU crossover is block-structured; 0 means inherit the entire GRU block
    // from one parent, 1 means unit-wise row crossover.
    gruCrossoverMode: 0,

    // Initial bias for the GRU update gate. More negative means longer default memory.
    gruInitUpdateBias: -0.7
  },
  collision: {
    substepMaxDt: 0.018,
    skipSegments: 6,
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
    pointsPerFood: 2.0,
    pointsPerKill: 45.0,
    pointsPerSecondAlive: 0.60,
    fitnessSurvivalPerSecond: 0.70,
    fitnessFood: 7.5,
    fitnessLengthPerSegment: 1.25,
    fitnessKill: 55.0,
    fitnessPointsNorm: 42.0,
    fitnessTopPointsBonus: 65.0
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
  dtClamp: 0.045
};

// Mutable configuration object.  Always clone the defaults so that
// resetting settings does not modify CFG_DEFAULT.
export let CFG = deepClone(CFG_DEFAULT);

/**
 * Resets the global configuration to its default values.
 */
export function resetCFGToDefaults() {
  CFG = deepClone(CFG_DEFAULT);
}
