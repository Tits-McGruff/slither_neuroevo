import {
  SETTING_DEFINITION_BY_PATH,
  normalizeSettingValue,
  type SettingDefinition
} from './settingDefinitions.ts';

/** Ordered list of config paths accepted by reset/import snapshots. */
export const SETTINGS_PATHS = [
  'evolvedRespawn.contractVersion', 'evolvedRespawn.enabled', 'evolvedRespawn.delaySeconds',
  'evolvedRespawn.delayMode', 'evolvedRespawn.maxLives', 'evolvedRespawn.recurrentState',
  'evolvedRespawn.rngMode', 'evolvedRespawn.corpseMode', 'lifeScoring.mode',
  'lifeScoring.cumulativeBasis', 'lifeScoring.cumulativeWallet', 'reward.lifeAccounting',
  'reward.durationScaling', 'reward.lifePenalty', 'reward.lifePenaltyBasis',
  'observer.earlyEndMode', 'world.pelletOverflow',
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
  'sense.layoutVersion',
  'sense.profile',
  'sense.rayMinimum', 'sense.rayCap', 'sense.rayBatch', 'sense.rayMaximumDue',
  'sense.bubbleBins',
  'sense.rNearBase',
  'sense.rNearScale',
  'sense.rNearMin',
  'sense.rNearMax',
  'sense.rFarBase',
  'sense.rFarScale',
  'sense.rFarMin',
  'sense.rFarMax',
  'sense.foodKBase',
  'sense.maxPelletChecks',
  'sense.maxSegmentChecks',
  'sense.debug',
  'baselineBots.count',
  'baselineBots.algorithm', 'baselineBots.behaviorMode', 'baselineBots.controlDt',
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
  'brain.rruInitGateBias'
] as const;

/** Union type of all supported settings paths. */
export type SettingsPath = (typeof SETTINGS_PATHS)[number];

/** Update payload for a single settings path. */
export interface SettingsUpdate {
  path: SettingsPath;
  value: number;
}

/** Path accepted by the Protocol 2 authoritative live-settings surface. */
export type LiveSettingPath = SettingsPath | 'simSpeed';

/** One path/value pair in an authoritative live-settings request or result. */
export interface LiveSettingsUpdate {
  /** Shared setting path. */
  path: LiveSettingPath;
  /** Finite numeric wire representation; booleans use zero or one. */
  value: number;
}

/** Successful atomic normalization of one live-settings request. */
export interface NormalizedLiveSettings {
  /** Whether every update was accepted for live application. */
  ok: true;
  /** Authoritative clamped and type-normalized updates. */
  updates: LiveSettingsUpdate[];
}

/** Rejected atomic normalization of one live-settings request. */
export interface RejectedLiveSettings {
  /** Whether every update was accepted for live application. */
  ok: false;
  /** Stable human-readable rejection reason. */
  reason: string;
}

/** Result of atomically validating and normalizing live setting updates. */
export type LiveSettingsNormalization = NormalizedLiveSettings | RejectedLiveSettings;

/**
 * Coerce a numeric settings update value into the CFG-compatible representation.
 * @param path - Settings path being updated.
 * @param value - Numeric value from the UI or import payload.
 * @returns Coerced value for writing into CFG.
 */
export function coerceSettingsUpdateValue(
  path: SettingsPath,
  value: number
): number | string | boolean {
  if (path === 'sense.layoutVersion') {
    if (value === 4) return 'v4';
    // Historical UI snapshots encoded v2/v3 as zero/one; retain their v3 presentation conversion.
    if (value === 0 || value === 1 || value === 3) return 'v3';
    throw new TypeError('unsupported sensor layout version');
  }
  if (path === 'brain.useMlp') {
    return value !== 0;
  }
  if (path === 'sense.profile') {
    if (value === 0) return 'basic';
    if (value === 1) return 'motion';
    throw new TypeError('unsupported sensor profile');
  }
  const definition = SETTING_DEFINITION_BY_PATH.get(path);
  if (!definition) return value;
  const normalized = normalizeSettingValue(definition, value);
  return definition.valueType === 'boolean' ? normalized === 1 : normalized;
}

/** Encode the two string-valued presentation choices as canonical wire codes. */
export function encodeSettingsUpdateValue(path: string, value: unknown): number {
  if (path === 'sense.layoutVersion') {
    if (value === 'v3') return 3;
    if (value === 'v4') return 4;
    throw new TypeError('unsupported sensor layout version');
  }
  if (path === 'sense.profile') {
    if (value === 'basic') return 0;
    if (value === 'motion') return 1;
    throw new TypeError('unsupported sensor profile');
  }
  if (typeof value === 'boolean') return Number(value);
  if (typeof value !== 'number' || !Number.isFinite(value)) throw new TypeError(`non-numeric setting ${path}`);
  return value;
}

/**
 * Resolve shared metadata for a supported live-settings path.
 * @param path - Untrusted path string from a Protocol 2 request.
 * @returns Definition when the path exists, otherwise undefined.
 */
export function getLiveSettingDefinition(path: string): SettingDefinition | undefined {
  return SETTING_DEFINITION_BY_PATH.get(path);
}

/**
 * Validate and normalize one live-settings request atomically.
 * @param updates - Untrusted but structurally numeric path/value pairs.
 * @returns All normalized updates, or one rejection with no partial values.
 */
export function normalizeLiveSettingsUpdates(
  updates: readonly { path: string; value: number }[]
): LiveSettingsNormalization {
  if (updates.length === 0) return { ok: false, reason: 'at least one settings update is required' };
  if (updates.length > 64) return { ok: false, reason: 'too many settings updates' };
  const seen = new Set<string>();
  const normalized: LiveSettingsUpdate[] = [];
  for (const update of updates) {
    if (seen.has(update.path)) {
      return { ok: false, reason: `duplicate setting path: ${update.path}` };
    }
    seen.add(update.path);
    const definition = getLiveSettingDefinition(update.path);
    if (!definition?.path) {
      return { ok: false, reason: `unknown setting path: ${update.path}` };
    }
    if (definition.requiresReset !== false) {
      return { ok: false, reason: `setting requires reset: ${update.path}` };
    }
    if (!Number.isFinite(update.value)) {
      return { ok: false, reason: `setting value must be finite: ${update.path}` };
    }
    normalized.push({
      path: definition.path as LiveSettingPath,
      value: normalizeSettingValue(definition, update.value)
    });
  }
  return { ok: true, updates: normalized };
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

/** Canonical generation-one core settings used when no snapshot overrides exist. */
export const DEFAULT_CORE_SETTINGS: Readonly<CoreSettings> = Object.freeze({
  snakeCount: 55,
  simSpeed: 1,
  hiddenLayers: 2,
  neurons1: 64,
  neurons2: 64,
  neurons3: 64,
  neurons4: 48,
  neurons5: 32
});
