/** Ordered list of config paths that can be updated live. */
export const SETTINGS_PATHS = [
  'worldRadius',
  'pelletCountTarget',
  'pelletSpawnPerSecond',
  'foodValue',
  'growPerFood',
  'foodSpawn.edgeFalloffEnabled',
  'foodSpawn.edgeFadeStart',
  'foodSpawn.edgeFadePower',
  'foodSpawn.filamentPower',
  'foodSpawn.warpFreq',
  'foodSpawn.warpScale',
  'foodSpawn.freqLarge',
  'foodSpawn.freqMedium',
  'foodSpawn.freqSmall',
  'foodSpawn.dustStrength',
  'baselineBots.count',
  'baselineBots.seed',
  'baselineBots.randomizeSeedPerGen',
  'baselineBots.respawnDelay',
  'snakeBaseSpeed',
  'snakeBoostSpeed',
  'snakeTurnRate',
  'snakeRadius',
  'snakeRadiusMax',
  'snakeThicknessScale',
  'snakeThicknessLogDiv',
  'snakeSpacing',
  'snakeStartLen',
  'snakeMaxLen',
  'snakeMinLen',
  'snakeSizeSpeedPenalty',
  'snakeBoostSizePenalty',
  'boost.minPointsToBoost',
  'boost.pointsCostPerSecond',
  'boost.pointsCostSizeFactor',
  'boost.lenLossPerPoint',
  'boost.pelletValueFactor',
  'boost.pelletJitter',
  'collision.substepMaxDt',
  'collision.skipSegments',
  'collision.hitScale',
  'collision.cellSize',
  'collision.neighborRange',
  'generationSeconds',
  'eliteFrac',
  'mutationRate',
  'mutationStd',
  'crossoverRate',
  'observer.focusRecheckSeconds',
  'observer.focusSwitchMargin',
  'observer.earlyEndMinSeconds',
  'observer.earlyEndAliveThreshold',
  'observer.overviewPadding',
  'observer.zoomLerpFollow',
  'observer.zoomLerpOverview',
  'observer.overviewExtraWorldMargin',
  'reward.pointsPerFood',
  'reward.pointsPerKill',
  'reward.pointsPerSecondAlive',
  'reward.fitnessSurvivalPerSecond',
  'reward.fitnessFood',
  'reward.fitnessLengthPerSegment',
  'reward.fitnessKill',
  'reward.fitnessPointsNorm',
  'reward.fitnessTopPointsBonus',
  'brain.useMlp',
  'brain.stack.gru',
  'brain.stack.lstm',
  'brain.stack.rru',
  'brain.gruHidden',
  'brain.lstmHidden',
  'brain.rruHidden',
  'brain.controlDt',
  'brain.gruMutationRate',
  'brain.gruMutationStd',
  'brain.gruCrossoverMode',
  'brain.gruInitUpdateBias',
  'brain.lstmInitForgetBias',
  'brain.rruInitGateBias',
  'dtClamp'
] as const;

/** Union type of all supported settings paths. */
export type SettingsPath = (typeof SETTINGS_PATHS)[number];

/** Update payload for a single settings path. */
export interface SettingsUpdate {
  path: SettingsPath;
  value: number;
}

/** Core UI settings that are controlled outside of CFG. */
export interface CoreSettings {
  snakeCount: number;
  simSpeed: number;
  hiddenLayers: number;
  neurons1: number;
  neurons2: number;
  neurons3: number;
  neurons4: number;
  neurons5: number;
}
