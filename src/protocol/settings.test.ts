import { describe, expect, it } from 'vitest';
import {
  coerceSettingsUpdateValue,
  normalizeLiveSettingsUpdates
} from './settings.ts';

describe('authoritative settings contract', () => {
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
