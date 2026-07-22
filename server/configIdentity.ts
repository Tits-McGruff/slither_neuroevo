import { CFG } from '../src/config.ts';
import type { CoreSettings } from '../src/protocol/settings.ts';
import type { SensorSpec } from '../src/protocol/sensors.ts';
import type { World } from '../src/world.ts';
import { hashConfig } from './hash.ts';
import { PROTOCOL_VERSION, SERIALIZER_VERSION } from './protocol.ts';
import { buildSensorSpec } from './sensorSpec.ts';
import { buildCoreSettingsSnapshot } from './settingsSnapshot.ts';

/** Version of the explicit content assembled for configuration identity. */
export const CONFIG_IDENTITY_VERSION = 1 as const;

/** Explicit experiment fields covered by the canonical configuration hash. */
export interface ConfigIdentityContent {
  /** Identity-content schema version. */
  identityVersion: typeof CONFIG_IDENTITY_VERSION;
  /** Network protocol whose semantics depend on the settings. */
  protocolVersion: number;
  /** Binary frame contract version. */
  serializerVersion: number;
  /** World-construction and core UI settings. */
  core: CoreSettings;
  /** Active sensor layout contract. */
  sensor: SensorSpec;
  /** Complete JSON-serializable experiment behavior configuration. */
  experimentConfig: typeof CFG;
}

/**
 * Assemble the explicit versioned content covered by config identity.
 * The seed and run id are deliberately adjacent state, not hash content.
 * @param world - Active authoritative world.
 * @returns Explicit config identity payload.
 */
export function buildConfigIdentityContent(world: World): ConfigIdentityContent {
  return {
    identityVersion: CONFIG_IDENTITY_VERSION,
    protocolVersion: PROTOCOL_VERSION,
    serializerVersion: SERIALIZER_VERSION,
    core: buildCoreSettingsSnapshot(world),
    sensor: buildSensorSpec(),
    experimentConfig: CFG
  };
}

/**
 * Compute the canonical content hash for the active authoritative config.
 * @param world - Active authoritative world.
 * @returns Versioned config hash.
 */
export function buildAuthoritativeConfigHash(world: World): string {
  return hashConfig(buildConfigIdentityContent(world));
}
