import { CFG } from '../src/config.ts';
import { getByPath } from '../src/utils.ts';
import { SETTINGS_PATHS } from '../src/protocol/settings.ts';
import type { CoreSettings, SettingsUpdate } from '../src/protocol/settings.ts';
import type { World } from '../src/world.ts';

/**
 * Build a core settings snapshot from the active world instance.
 * @param world - World instance to read from.
 * @returns Core settings snapshot used for resets.
 */
export function buildCoreSettingsSnapshot(world: World): CoreSettings {
  return {
    snakeCount: world.settings.snakeCount,
    simSpeed: world.settings.simSpeed,
    hiddenLayers: world.settings.hiddenLayers,
    neurons1: world.settings.neurons1,
    neurons2: world.settings.neurons2,
    neurons3: world.settings.neurons3,
    neurons4: world.settings.neurons4,
    neurons5: world.settings.neurons5
  };
}

/**
 * Build settings updates from the current CFG values.
 * @returns Settings updates snapshot.
 */
export function buildSettingsUpdatesSnapshot(): SettingsUpdate[] {
  return SETTINGS_PATHS.map((path) => {
    const raw = getByPath(CFG, path);
    return { path, value: mapSettingsSnapshotValue(path, raw) };
  });
}

/**
 * Normalize a CFG value into a numeric snapshot entry.
 * @param path - Settings path being serialized.
 * @param raw - Raw CFG value at the path.
 * @returns Numeric snapshot value.
 */
function mapSettingsSnapshotValue(path: SettingsUpdate['path'], raw: unknown): number {
  if (path === 'sense.layoutVersion') {
    return raw === 'v2' ? 1 : 0;
  }
  if (typeof raw === 'number' && Number.isFinite(raw)) {
    return raw;
  }
  if (typeof raw === 'boolean') {
    return raw ? 1 : 0;
  }
  return 0;
}
