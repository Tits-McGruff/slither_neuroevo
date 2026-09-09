import { randomUUID } from 'node:crypto';
import { DEFAULT_CORE_SETTINGS, SETTINGS_PATHS } from '../../src/protocol/settings.ts';
import { getSensorLayout, getSensorSpec } from '../../src/protocol/sensors.ts';
import type { RustBackgroundDisplay, RustStartupMetadata } from '../../src/protocol/rustBackground.ts';
import type { StatsMsg, WelcomeMsg } from '../protocol.ts';

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

/** Construct the existing browser handshake from the fixed native P0 profile. */
export function createRustWelcome(metadata: RustStartupMetadata): WelcomeMsg {
  if (metadata.parameterCount !== 13_458 || nativeSetting(metadata, 'sense.bubbleBins') !== 16) {
    throw new Error('unsupported experimental browser graph profile');
  }
  return {
    type: 'welcome', protocolVersion: 2, serializerVersion: metadata.serializerVersion,
    sessionId: randomUUID(), tickRate: 1 / metadata.fixedStepSeconds,
    worldSeed: metadata.seed, runId: metadata.runId, configHash: metadata.configHash,
    configRevision: wireInteger(metadata.configRevision), frameByteLength: 0,
    settings: {
      // Graph-editor defaults describe the one admitted P0 graph. This route
      // accepts no graph edits and never constructs native population weights.
      core: { ...DEFAULT_CORE_SETTINGS, snakeCount: nativeSetting(metadata, 'snakeCount'), simSpeed: nativeSetting(metadata, 'simSpeed') },
      updates: SETTINGS_PATHS.filter(path => metadata.settings.some(setting => setting.path === path))
        .map(path => ({ path, value: nativeSetting(metadata, path) }))
    },
    sensorSpec: getSensorSpec(getSensorLayout(nativeSetting(metadata, 'sense.bubbleBins'))),
    inferenceMode: {
      requestedBackend: 'native', activeBackend: 'native', requestedMt: false, activeWorkerCount: 0,
      poolEpoch: null, weightEpoch: null, graphKey: metadata.graphKey, parameterCount: metadata.parameterCount,
      seed: metadata.seed, nativeAddonStatus: 'ready', nativeAddonBuildIdentifier: null
    }
  };
}

/** Forward cached native facts; unsupported collision diagnostics are absent. */
export function createRustStats(display: RustBackgroundDisplay, metadata: RustStartupMetadata, pumpsPerSecond: number): StatsMsg {
  return {
    type: 'stats', tick: wireInteger(display.completedStep), gen: wireInteger(display.generation),
    generationTime: display.generationTime, generationSeconds: nativeSetting(metadata, 'generationSeconds'),
    alive: display.alivePopulation, aliveTotal: display.aliveSnakes,
    baselineBotsAlive: display.baselineBotsAlive, baselineBotsTotal: display.baselineBotsTotal,
    fps: pumpsPerSecond
  };
}
