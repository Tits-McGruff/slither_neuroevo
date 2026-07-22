/** Supported input controls for the settings panel. */
export type SettingControlType = 'range' | 'number' | 'checkbox' | 'action';

/** Scalar semantics enforced by the authoritative settings validator. */
export type SettingValueType = 'number' | 'integer' | 'boolean';

/** Server-owned derived state that must change with an accepted live value. */
export type SettingDerivedState = 'baseline-respawn-delay' | 'simulation-speed';

/** Pure metadata shared by the browser settings UI and simulation server. */
export interface SettingDefinition {
  /** Visual settings group. */
  group: string;
  /** CFG path controlled by this definition, omitted for action buttons. */
  path?: string;
  /** Human-readable control label. */
  label: string;
  /** Minimum accepted numeric value. */
  min?: number;
  /** Maximum accepted numeric value. */
  max?: number;
  /** UI increment for numeric controls. */
  step?: number;
  /** Decimal places displayed beside the control. */
  decimals?: number;
  /** Whether the value may only be applied while rebuilding the world. */
  requiresReset?: boolean;
  /** Browser control type. */
  type?: SettingControlType;
  /** Explicit scalar validation semantics. */
  valueType?: SettingValueType;
  /** Optional server-side derived-state update. */
  derivedState?: SettingDerivedState;
  /** Optional DOM id. */
  id?: string;
  /** Optional action-button label. */
  actionLabel?: string;
  /** Optional explanatory text. */
  hint?: string;
  /** Optional DOM id for explanatory text. */
  hintId?: string;
}

/** Input id for the baseline bot seed control. */
export const BASELINE_BOT_SEED_INPUT_ID = 'baselineBotSeed';
/** Hint id for invalid baseline bot seed values. */
export const BASELINE_BOT_SEED_HINT_ID = 'baselineBotSeedHint';
/** Button id for randomizing the baseline bot seed. */
export const BASELINE_BOT_SEED_RANDOMIZE_ID = 'baselineBotSeedRandomize';

/** Pure setting definitions used to build the UI and validate server updates. */
export const SETTING_DEFINITIONS: readonly SettingDefinition[] = [
  { group: 'World and food', path: 'worldRadius', label: 'World radius', min: 800, max: 10000, step: 50, decimals: 0, requiresReset: true, valueType: 'integer' },
  { group: 'World and food', path: 'pelletCountTarget', label: 'Pellet target count', min: 100, max: 25000, step: 50, decimals: 0, requiresReset: true, valueType: 'integer' },
  { group: 'World and food', path: 'pelletSpawnPerSecond', label: 'Pellet spawn per second', min: 5, max: 3500, step: 5, decimals: 0, requiresReset: true, valueType: 'number' },
  { group: 'World and food', path: 'foodValue', label: 'Food value per pellet', min: 0.1, max: 8, step: 0.1, decimals: 1, requiresReset: true, valueType: 'number' },
  { group: 'World and food', path: 'growPerFood', label: 'Growth per food', min: 0.1, max: 10, step: 0.1, decimals: 1, requiresReset: true, valueType: 'number' },
  { group: 'World and food', path: 'foodSpawn.edgeFalloffEnabled', label: 'Edge food falloff', requiresReset: false, type: 'checkbox', valueType: 'boolean' },
  { group: 'World and food', path: 'foodSpawn.edgeFadeStart', label: 'Edge fade start', min: 0.05, max: 0.85, step: 0.01, decimals: 2, requiresReset: false, valueType: 'number' },
  { group: 'World and food', path: 'foodSpawn.edgeFadePower', label: 'Edge fade sharpness', min: 1, max: 6, step: 0.1, decimals: 1, requiresReset: false, valueType: 'number' },
  { group: 'World and food', path: 'foodSpawn.filamentPower', label: 'Filament contrast', min: 1.5, max: 8, step: 0.1, decimals: 1, requiresReset: false, valueType: 'number' },
  { group: 'World and food', path: 'foodSpawn.warpScale', label: 'Filament warp scale', min: 0, max: 0.2, step: 0.005, decimals: 3, requiresReset: false, valueType: 'number' },
  { group: 'World and food', path: 'foodSpawn.warpFreq', label: 'Filament warp frequency', min: 0.0003, max: 0.003, step: 0.0001, decimals: 4, requiresReset: false, valueType: 'number' },
  { group: 'World and food', path: 'foodSpawn.freqLarge', label: 'Filament scale (large)', min: 0.001, max: 0.006, step: 0.0001, decimals: 4, requiresReset: false, valueType: 'number' },
  { group: 'World and food', path: 'foodSpawn.freqMedium', label: 'Filament scale (medium)', min: 0.0015, max: 0.01, step: 0.0001, decimals: 4, requiresReset: false, valueType: 'number' },
  { group: 'World and food', path: 'foodSpawn.freqSmall', label: 'Filament scale (small)', min: 0.0025, max: 0.02, step: 0.0002, decimals: 4, requiresReset: false, valueType: 'number' },
  { group: 'World and food', path: 'foodSpawn.dustStrength', label: 'Filament speckle strength', min: 0, max: 1, step: 0.05, decimals: 2, requiresReset: false, valueType: 'number' },

  { group: 'Sensors', path: 'sense.bubbleBins', label: 'Sensor bins', min: 8, max: 32, step: 1, decimals: 0, requiresReset: true, valueType: 'integer' },
  { group: 'Sensors', path: 'sense.rNearBase', label: 'Near radius base', min: 200, max: 900, step: 10, decimals: 0, requiresReset: false, valueType: 'number' },
  { group: 'Sensors', path: 'sense.rNearScale', label: 'Near radius scale', min: 0, max: 600, step: 10, decimals: 0, requiresReset: false, valueType: 'number' },
  { group: 'Sensors', path: 'sense.rNearMin', label: 'Near radius min', min: 150, max: 900, step: 10, decimals: 0, requiresReset: false, valueType: 'number' },
  { group: 'Sensors', path: 'sense.rNearMax', label: 'Near radius max', min: 200, max: 1200, step: 10, decimals: 0, requiresReset: false, valueType: 'number' },
  { group: 'Sensors', path: 'sense.rFarBase', label: 'Far radius base', min: 400, max: 2000, step: 20, decimals: 0, requiresReset: false, valueType: 'number' },
  { group: 'Sensors', path: 'sense.rFarScale', label: 'Far radius scale', min: 0, max: 1200, step: 20, decimals: 0, requiresReset: false, valueType: 'number' },
  { group: 'Sensors', path: 'sense.rFarMin', label: 'Far radius min', min: 400, max: 2200, step: 20, decimals: 0, requiresReset: false, valueType: 'number' },
  { group: 'Sensors', path: 'sense.rFarMax', label: 'Far radius max', min: 600, max: 3000, step: 20, decimals: 0, requiresReset: false, valueType: 'number' },
  { group: 'Sensors', path: 'sense.foodKBase', label: 'Food saturation K', min: 0.5, max: 12, step: 0.1, decimals: 1, requiresReset: false, valueType: 'number' },
  { group: 'Sensors', path: 'sense.maxPelletChecks', label: 'Max pellet checks', min: 100, max: 3000, step: 50, decimals: 0, requiresReset: false, valueType: 'integer' },
  { group: 'Sensors', path: 'sense.maxSegmentChecks', label: 'Max segment checks', min: 200, max: 4000, step: 50, decimals: 0, requiresReset: false, valueType: 'integer' },
  { group: 'Sensors', path: 'sense.debug', label: 'Sensors debug logs', requiresReset: false, type: 'checkbox', valueType: 'boolean', hint: 'Enable sensor debug logging.' },

  { group: 'Baseline bots', path: 'baselineBots.count', label: 'Baseline bot count', min: 0, max: 120, step: 1, decimals: 0, requiresReset: true, valueType: 'integer' },
  { group: 'Baseline bots', path: 'baselineBots.respawnDelay', label: 'Respawn delay (sec)', min: 0.5, max: 60, step: 0.5, decimals: 1, requiresReset: false, valueType: 'number', derivedState: 'baseline-respawn-delay' },
  { group: 'Baseline bots', path: 'baselineBots.randomizeSeedPerGen', label: 'Randomize base seed per generation', requiresReset: true, type: 'checkbox', valueType: 'boolean' },
  { group: 'Baseline bots', path: 'baselineBots.seed', label: 'Baseline bot base seed', min: 0, max: 4294967295, step: 1, decimals: 0, requiresReset: true, type: 'number', valueType: 'integer', id: BASELINE_BOT_SEED_INPUT_ID, hint: 'Seed must be a non-negative integer.', hintId: BASELINE_BOT_SEED_HINT_ID },
  { group: 'Baseline bots', label: 'Randomize base seed', type: 'action', actionLabel: 'Randomize seed', id: BASELINE_BOT_SEED_RANDOMIZE_ID },

  { group: 'Snake physics', path: 'snakeBaseSpeed', label: 'Base speed', min: 30, max: 650, step: 5, decimals: 0, requiresReset: true, valueType: 'number' },
  { group: 'Snake physics', path: 'snakeBoostSpeed', label: 'Boost speed (used as relative multiplier)', min: 40, max: 1200, step: 5, decimals: 0, requiresReset: true, valueType: 'number' },
  { group: 'Snake physics', path: 'snakeTurnRate', label: 'Turn rate', min: 0.4, max: 14, step: 0.1, decimals: 1, requiresReset: true, valueType: 'number' },
  { group: 'Snake physics', path: 'snakeRadius', label: 'Base radius', min: 3, max: 30, step: 1, decimals: 0, requiresReset: true, valueType: 'number' },
  { group: 'Snake physics', path: 'snakeRadiusMax', label: 'Max radius', min: 4, max: 50, step: 1, decimals: 0, requiresReset: true, valueType: 'number' },
  { group: 'Snake physics', path: 'snakeThicknessScale', label: 'Thickness scale', min: 0, max: 20, step: 0.1, decimals: 1, requiresReset: true, valueType: 'number' },
  { group: 'Snake physics', path: 'snakeThicknessLogDiv', label: 'Thickness log divisor', min: 1, max: 240, step: 1, decimals: 0, requiresReset: true, valueType: 'number' },
  { group: 'Snake physics', path: 'snakeSpacing', label: 'Segment spacing', min: 3, max: 20, step: 0.1, decimals: 1, requiresReset: true, valueType: 'number' },
  { group: 'Snake physics', path: 'snakeStartLen', label: 'Start length', min: 5, max: 140, step: 1, decimals: 0, requiresReset: true, valueType: 'integer' },
  { group: 'Snake physics', path: 'snakeMaxLen', label: 'Max length', min: 60, max: 100000, step: 10, decimals: 0, requiresReset: true, valueType: 'integer' },
  { group: 'Snake physics', path: 'snakeMinLen', label: 'Min length', min: 4, max: 80, step: 1, decimals: 0, requiresReset: true, valueType: 'integer' },
  { group: 'Snake physics', path: 'snakeSizeSpeedPenalty', label: 'Size speed penalty', min: 0, max: 0.7, step: 0.01, decimals: 2, requiresReset: false, valueType: 'number' },
  { group: 'Snake physics', path: 'snakeBoostSizePenalty', label: 'Size boost penalty', min: 0, max: 0.95, step: 0.01, decimals: 2, requiresReset: false, valueType: 'number' },

  { group: 'Boost and mass', path: 'boost.minPointsToBoost', label: 'Min points to boost', min: 0, max: 60, step: 0.1, decimals: 1, requiresReset: false, valueType: 'number' },
  { group: 'Boost and mass', path: 'boost.pointsCostPerSecond', label: 'Boost points cost per second', min: 0, max: 80, step: 0.5, decimals: 1, requiresReset: false, valueType: 'number' },
  { group: 'Boost and mass', path: 'boost.pointsCostSizeFactor', label: 'Boost cost size factor', min: 0, max: 4, step: 0.05, decimals: 2, requiresReset: false, valueType: 'number' },
  { group: 'Boost and mass', path: 'boost.lenLossPerPoint', label: 'Length loss per point', min: 0, max: 2, step: 0.01, decimals: 2, requiresReset: false, valueType: 'number' },
  { group: 'Boost and mass', path: 'boost.pelletValueFactor', label: 'Boost drop pellet value factor', min: 0, max: 1.5, step: 0.01, decimals: 2, requiresReset: false, valueType: 'number' },
  { group: 'Boost and mass', path: 'boost.pelletJitter', label: 'Boost drop jitter', min: 0, max: 80, step: 1, decimals: 0, requiresReset: false, valueType: 'number' },

  { group: 'Collision', path: 'collision.substepMaxDt', label: 'Substep max dt', min: 0.006, max: 0.05, step: 0.001, decimals: 3, requiresReset: false, valueType: 'number' },
  { group: 'Collision', path: 'collision.skipSegments', label: 'Skip segments near head', min: 0, max: 30, step: 1, decimals: 0, requiresReset: false, valueType: 'integer' },
  { group: 'Collision', path: 'collision.hitScale', label: 'Hit scale', min: 0.45, max: 1.2, step: 0.01, decimals: 2, requiresReset: false, valueType: 'number' },
  { group: 'Collision', path: 'collision.cellSize', label: 'Collision grid cell size', min: 20, max: 200, step: 1, decimals: 0, requiresReset: true, valueType: 'number', hint: 'Changing cell size reallocates the spatial grid.' },
  { group: 'Collision', path: 'collision.neighborRange', label: 'Collision neighbor range', min: 1, max: 3, step: 1, decimals: 0, requiresReset: false, valueType: 'integer' },

  { group: 'Evolution', path: 'generationSeconds', label: 'Generation duration seconds', min: 8, max: 480, step: 1, decimals: 0, requiresReset: true, valueType: 'number' },
  { group: 'Evolution', path: 'eliteFrac', label: 'Elite fraction', min: 0.01, max: 0.5, step: 0.01, decimals: 2, requiresReset: true, valueType: 'number' },
  { group: 'Evolution', path: 'mutationRate', label: 'Mutation rate', min: 0, max: 0.5, step: 0.005, decimals: 3, requiresReset: true, valueType: 'number' },
  { group: 'Evolution', path: 'mutationStd', label: 'Mutation std', min: 0, max: 2.5, step: 0.05, decimals: 2, requiresReset: true, valueType: 'number' },
  { group: 'Evolution', path: 'crossoverRate', label: 'Crossover rate', min: 0, max: 1, step: 0.02, decimals: 2, requiresReset: true, valueType: 'number' },

  { group: 'Observer and camera', path: 'observer.focusRecheckSeconds', label: 'Focus recheck seconds', min: 0.1, max: 6, step: 0.05, decimals: 2, requiresReset: false, valueType: 'number' },
  { group: 'Observer and camera', path: 'observer.focusSwitchMargin', label: 'Focus switch margin', min: 1, max: 1.6, step: 0.01, decimals: 2, requiresReset: false, valueType: 'number' },
  { group: 'Observer and camera', path: 'observer.earlyEndMinSeconds', label: 'Early end min seconds', min: 0, max: 50, step: 1, decimals: 0, requiresReset: false, valueType: 'number' },
  { group: 'Observer and camera', path: 'observer.earlyEndAliveThreshold', label: 'Early end alive threshold', min: 1, max: 25, step: 1, decimals: 0, requiresReset: false, valueType: 'integer' },
  { group: 'Observer and camera', path: 'observer.overviewPadding', label: 'Overview padding', min: 1, max: 1.8, step: 0.01, decimals: 2, requiresReset: false, valueType: 'number' },
  { group: 'Observer and camera', path: 'observer.zoomLerpFollow', label: 'Follow zoom lerp', min: 0, max: 0.4, step: 0.01, decimals: 2, requiresReset: false, valueType: 'number' },
  { group: 'Observer and camera', path: 'observer.zoomLerpOverview', label: 'Overview zoom lerp', min: 0, max: 0.4, step: 0.01, decimals: 2, requiresReset: false, valueType: 'number' },
  { group: 'Observer and camera', path: 'observer.overviewExtraWorldMargin', label: 'Overview extra margin', min: 0, max: 1200, step: 10, decimals: 0, requiresReset: false, valueType: 'number' },

  { group: 'Rewards', path: 'reward.pointsPerFood', label: 'Points per food', min: 0, max: 20, step: 0.1, decimals: 1, requiresReset: false, valueType: 'number' },
  { group: 'Rewards', path: 'reward.pointsPerKill', label: 'Points per kill', min: 0, max: 400, step: 1, decimals: 0, requiresReset: false, valueType: 'number' },
  { group: 'Rewards', path: 'reward.pointsPerSecondAlive', label: 'Points per second alive', min: 0, max: 10, step: 0.05, decimals: 2, requiresReset: false, valueType: 'number' },
  { group: 'Rewards', path: 'reward.fitnessSurvivalPerSecond', label: 'Fitness survival per second', min: 0, max: 10, step: 0.05, decimals: 2, requiresReset: false, valueType: 'number' },
  { group: 'Rewards', path: 'reward.fitnessFood', label: 'Fitness per food', min: 0, max: 80, step: 0.5, decimals: 1, requiresReset: false, valueType: 'number' },
  { group: 'Rewards', path: 'reward.fitnessLengthPerSegment', label: 'Fitness per grown segment', min: 0, max: 100, step: 0.05, decimals: 2, requiresReset: false, valueType: 'number' },
  { group: 'Rewards', path: 'reward.fitnessKill', label: 'Fitness per kill', min: 0, max: 400, step: 1, decimals: 0, requiresReset: false, valueType: 'number' },
  { group: 'Rewards', path: 'reward.fitnessPointsNorm', label: 'Fitness points normalization weight', min: 0, max: 300, step: 1, decimals: 0, requiresReset: false, valueType: 'number' },
  { group: 'Rewards', path: 'reward.fitnessTopPointsBonus', label: 'Fitness top points bonus', min: 0, max: 600, step: 1, decimals: 0, requiresReset: false, valueType: 'number' },

  { group: 'Brain and memory', path: 'brain.gruHidden', label: 'GRU hidden size', min: 4, max: 96, step: 1, decimals: 0, requiresReset: true, valueType: 'integer' },
  { group: 'Brain and memory', path: 'brain.lstmHidden', label: 'LSTM hidden size', min: 4, max: 96, step: 1, decimals: 0, requiresReset: true, valueType: 'integer' },
  { group: 'Brain and memory', path: 'brain.rruHidden', label: 'RRU hidden size', min: 4, max: 96, step: 1, decimals: 0, requiresReset: true, valueType: 'integer' },
  { group: 'Brain and memory', path: 'brain.controlDt', label: 'Brain control dt', min: 0.008, max: 0.06, step: 0.001, decimals: 3, requiresReset: false, valueType: 'number' },
  { group: 'Brain and memory', path: 'brain.gruMutationRate', label: 'Recurrent mutation rate (GRU/LSTM/RRU)', min: 0, max: 0.35, step: 0.005, decimals: 3, requiresReset: true, valueType: 'number' },
  { group: 'Brain and memory', path: 'brain.gruMutationStd', label: 'Recurrent mutation std (GRU/LSTM/RRU)', min: 0, max: 1.6, step: 0.02, decimals: 2, requiresReset: true, valueType: 'number' },
  { group: 'Brain and memory', path: 'brain.gruCrossoverMode', label: 'Recurrent crossover mode (0 block, 1 unit)', min: 0, max: 1, step: 1, decimals: 0, requiresReset: true, valueType: 'integer' },
  { group: 'Brain and memory', path: 'brain.gruInitUpdateBias', label: 'GRU init update gate bias (GRU only)', min: -2.5, max: 1.5, step: 0.05, decimals: 2, requiresReset: true, valueType: 'number' },
  { group: 'Brain and memory', path: 'brain.lstmInitForgetBias', label: 'LSTM init forget gate bias (LSTM only)', min: -1.5, max: 3, step: 0.05, decimals: 2, requiresReset: true, valueType: 'number' },
  { group: 'Brain and memory', path: 'brain.rruInitGateBias', label: 'RRU init gate bias (RRU only)', min: -1.5, max: 2, step: 0.05, decimals: 2, requiresReset: true, valueType: 'number' }
];

/** Pure definitions for core controls rendered directly by index.html. */
export const CORE_SETTING_DEFINITIONS: readonly SettingDefinition[] = [
  { group: 'Core', path: 'snakeCount', label: 'NPC snakes', min: 1, max: 300, step: 1, decimals: 0, requiresReset: true, valueType: 'integer' },
  { group: 'Core', path: 'simSpeed', label: 'Simulation speed', min: 0.1, max: 12, step: 0.05, decimals: 2, requiresReset: false, valueType: 'number', derivedState: 'simulation-speed' },
  { group: 'Core', path: 'hiddenLayers', label: 'AI hidden layers', min: 1, max: 5, step: 1, decimals: 0, requiresReset: true, valueType: 'integer' },
  { group: 'Core', path: 'neurons1', label: 'Neurons layer 1', min: 1, max: 256, step: 1, decimals: 0, requiresReset: true, valueType: 'integer' },
  { group: 'Core', path: 'neurons2', label: 'Neurons layer 2', min: 1, max: 256, step: 1, decimals: 0, requiresReset: true, valueType: 'integer' },
  { group: 'Core', path: 'neurons3', label: 'Neurons layer 3', min: 1, max: 256, step: 1, decimals: 0, requiresReset: true, valueType: 'integer' },
  { group: 'Core', path: 'neurons4', label: 'Neurons layer 4', min: 1, max: 256, step: 1, decimals: 0, requiresReset: true, valueType: 'integer' },
  { group: 'Core', path: 'neurons5', label: 'Neurons layer 5', min: 1, max: 256, step: 1, decimals: 0, requiresReset: true, valueType: 'integer' }
];

/** Pure definition for the core simulation-speed control. */
export const SIM_SPEED_DEFINITION = CORE_SETTING_DEFINITIONS[1]!;

/** Setting definitions keyed by their authoritative path. */
export const SETTING_DEFINITION_BY_PATH: ReadonlyMap<string, SettingDefinition> = new Map(
  [...SETTING_DEFINITIONS, ...CORE_SETTING_DEFINITIONS]
    .filter((definition) => definition.path !== undefined)
    .map((definition) => [definition.path!, definition])
);

/**
 * Normalize one numeric wire value according to shared setting metadata.
 * @param definition - Setting definition that owns the value.
 * @param value - Finite numeric wire value.
 * @returns Normalized numeric representation used by CFG and broadcasts.
 */
export function normalizeSettingValue(definition: SettingDefinition, value: number): number {
  let normalized = value;
  if (definition.valueType === 'boolean') normalized = value === 0 ? 0 : 1;
  if (definition.valueType === 'integer') normalized = Math.round(normalized);
  if (definition.min !== undefined) normalized = Math.max(definition.min, normalized);
  if (definition.max !== undefined) normalized = Math.min(definition.max, normalized);
  return normalized;
}
