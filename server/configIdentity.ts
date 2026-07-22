import { CFG } from '../src/config.ts';
import type { GraphSpec } from '../src/brains/graph/schema.ts';
import { graphKey } from '../src/brains/graph/compiler.ts';
import { buildStackGraphSpec } from '../src/brains/stackBuilder.ts';
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
 * Assemble configuration identity with one explicit graph representation.
 * @param world - Active authoritative world.
 * @param graphSpec - Graph representation covered by the identity.
 * @returns Explicit config identity payload.
 */
function buildConfigIdentityContentWithGraph(
  world: World,
  graphSpec: GraphSpec | null
): ConfigIdentityContent {
  return {
    identityVersion: CONFIG_IDENTITY_VERSION,
    protocolVersion: PROTOCOL_VERSION,
    serializerVersion: SERIALIZER_VERSION,
    core: buildCoreSettingsSnapshot(world),
    sensor: buildSensorSpec(),
    experimentConfig: {
      ...CFG,
      brain: {
        ...CFG.brain,
        graphSpec
      }
    }
  };
}

/**
 * Assemble the explicit versioned content covered by config identity.
 * The seed and run id are deliberately adjacent state, not hash content.
 * @param world - Active authoritative world.
 * @returns Explicit config identity payload.
 */
export function buildConfigIdentityContent(world: World): ConfigIdentityContent {
  return buildConfigIdentityContentWithGraph(world, world.arch.spec);
}

/**
 * Compute the canonical content hash for the active authoritative config.
 * @param world - Active authoritative world.
 * @returns Versioned config hash.
 */
export function buildAuthoritativeConfigHash(world: World): string {
  return hashConfig(buildConfigIdentityContent(world));
}

/**
 * Reproduce the pre-Phase-9 fallback-stack hash for read-only compatibility.
 * Older checkpoints hashed the raw null graph setting even though they stored
 * and reconstructed the compiled graph that actually ran.
 * @param world - Reconstructed authoritative world.
 * @returns Historical null-graph configuration hash, or null for a custom graph.
 */
export function buildLegacyNullGraphConfigHash(world: World): string | null {
  const fallbackSpec = buildStackGraphSpec(world.settings, CFG);
  if (graphKey(fallbackSpec) !== world.archKey) return null;
  return hashConfig(buildConfigIdentityContentWithGraph(world, null));
}
