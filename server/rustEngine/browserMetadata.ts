import { DEFAULT_CORE_SETTINGS, SETTINGS_PATHS } from '../../src/protocol/settings.ts';
import { getSensorLayout, getSensorSpec } from '../../src/protocol/sensors.ts';
import type {
  RustBackgroundDisplay,
  RustBackgroundVisualization,
  RustStartupMetadata
} from '../../src/protocol/rustBackground.ts';
import type { FitnessHistoryEntry } from '../../src/protocol/messages.ts';
import type { StatsMsg, WelcomeMsg } from '../protocol.ts';
import { deriveStackPresentation } from './stackGraph.ts';

/** Require exact Protocol 2 numbers rather than silently narrowing Rust counters. */
export function wireInteger(value: string): number {
  if (!/^[0-9a-f]{16}$/u.test(value)) throw new TypeError('invalid native wire integer');
  const integer = BigInt(`0x${value}`);
  if (integer > BigInt(Number.MAX_SAFE_INTEGER)) throw new RangeError('native counter exceeds Protocol 2');
  return Number(integer);
}

/** Read one native normalized numeric setting, including numeric boolean encoding. */
export function nativeSetting(metadata: RustStartupMetadata, path: string): number {
  const raw = metadata.settings.find(setting => setting.path === path)?.value;
  if (typeof raw === 'boolean') return Number(raw);
  if (typeof raw !== 'number' || !Number.isFinite(raw)) throw new TypeError(`missing native setting ${path}`);
  return raw;
}

/** Project graph controls and scalar settings without assigning transport identities. */
export function createRustSettings(metadata: RustStartupMetadata): WelcomeMsg['settings'] {
  const core = { ...DEFAULT_CORE_SETTINGS, snakeCount: nativeSetting(metadata, 'snakeCount'), simSpeed: nativeSetting(metadata, 'simSpeed') };
  const inputSize = getSensorLayout(nativeSetting(metadata, 'sense.bubbleBins')).inputSize;
  const stack = deriveStackPresentation(metadata.graphSpec, core, inputSize);
  const updates = new Map(SETTINGS_PATHS.filter(path => metadata.settings.some(setting => setting.path === path))
    .map(path => [path, nativeSetting(metadata, path)]));
  for (const update of stack?.updates ?? []) updates.set(update.path, update.value);
  return { core: stack?.core ?? core, updates: [...updates].map(([path, value]) => ({ path, value })) };
}

/** Construct the browser handshake using the identity owned by the server process. */
export function createRustWelcome(
  metadata: RustStartupMetadata,
  sessionId: string,
  nativeBuildIdentifier: string | null = null,
  calculationWorkers = 1
): WelcomeMsg {
  return {
    type: 'welcome', protocolVersion: 2, serializerVersion: metadata.serializerVersion,
    ...(metadata.legacyConversion ? { legacyConversion: metadata.legacyConversion } : {}),
    sessionId, tickRate: 1 / metadata.fixedStepSeconds,
    worldSeed: metadata.seed, runId: metadata.runId, configHash: metadata.configHash,
    configRevision: wireInteger(metadata.configRevision), frameByteLength: 0,
    graphSpec: metadata.graphSpec,
    capabilities: { checkpointPinning: true, archiveExport: true, archiveImport: true },
    settings: createRustSettings(metadata),
    sensorSpec: getSensorSpec(getSensorLayout(nativeSetting(metadata, 'sense.bubbleBins'))),
    inferenceMode: {
      requestedBackend: 'native', activeBackend: 'native', requestedMt: calculationWorkers > 1,
      activeWorkerCount: calculationWorkers > 1 ? calculationWorkers : 0,
      poolEpoch: null, weightEpoch: null, graphKey: metadata.graphKey, parameterCount: metadata.parameterCount,
      seed: metadata.seed, nativeAddonStatus: 'ready', nativeAddonBuildIdentifier: nativeBuildIdentifier
    }
  };
}

/** Forward cached native facts and compact persisted history; unsupported collision diagnostics are absent. */
export function createRustStats(
  display: RustBackgroundDisplay,
  metadata: RustStartupMetadata,
  pumpsPerSecond: number,
  fitnessHistory: FitnessHistoryEntry[] = [],
  visualization?: RustBackgroundVisualization
): StatsMsg {
  return {
    type: 'stats', tick: wireInteger(display.completedStep), gen: wireInteger(display.generation),
    generationTime: display.generationTime, generationSeconds: nativeSetting(metadata, 'generationSeconds'),
    alive: display.alivePopulation, aliveTotal: display.aliveSnakes,
    baselineBotsAlive: display.baselineBotsAlive, baselineBotsTotal: display.baselineBotsTotal,
    fps: pumpsPerSecond, fitnessHistory,
    ...(visualization ? { viz: visualization } : {})
  };
}
