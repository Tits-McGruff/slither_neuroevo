import { describe, expect, it } from 'vitest';
import { SETTING_DEFINITION_BY_PATH, normalizeSettingValue } from './settingDefinitions.ts';
import {
  coerceSettingsUpdateValue,
  normalizeLiveSettingsUpdates
} from './settings.ts';

describe('authoritative settings contract', () => {
  it('rejects malformed life codes and numeric bounds without rounding, clamping or live application', () => {
    for (const [path, value] of [['evolvedRespawn.delayMode', 0.5], ['world.pelletOverflow', 2],
      ['evolvedRespawn.maxLives', 1.5], ['evolvedRespawn.enabled', 2], ['evolvedRespawn.contractVersion', 0],
      ['reward.lifePenalty', 10001]] as const) {
      expect(() => normalizeSettingValue(SETTING_DEFINITION_BY_PATH.get(path)!, value)).toThrow();
      expect(normalizeLiveSettingsUpdates([{ path, value: 0 }]).ok).toBe(false);
    }
  });
  it('rejects reset-required paths without partially accepting a batch', () => {
    const result = normalizeLiveSettingsUpdates([
      { path: 'simSpeed', value: 2 },
      { path: 'collision.cellSize', value: 80 }
    ]);

    expect(result).toEqual({
      ok: false,
      reason: 'setting requires reset: collision.cellSize'
    });
  });

  it('clamps and normalizes values from shared metadata', () => {
    const result = normalizeLiveSettingsUpdates([
      { path: 'simSpeed', value: 100 },
      { path: 'sense.maxPelletChecks', value: 123.6 },
      { path: 'sense.debug', value: -4 }
    ]);

    expect(result).toEqual({
      ok: true,
      updates: [
        { path: 'simSpeed', value: 12 },
        { path: 'sense.maxPelletChecks', value: 124 },
        { path: 'sense.debug', value: 1 }
      ]
    });
  });

  it('normalizes every legacy sensor-layout snapshot marker to v3', () => {
    expect(coerceSettingsUpdateValue('sense.layoutVersion', 0)).toBe('v3');
    expect(coerceSettingsUpdateValue('sense.layoutVersion', 1)).toBe('v3');
  });
});
