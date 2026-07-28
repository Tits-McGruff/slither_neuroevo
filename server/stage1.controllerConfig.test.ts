import { describe, expect, it } from 'vitest';
import { DEFAULT_CONFIG, normalizeConfig } from './config.ts';

describe('Stage 1 controller wall-time configuration', () => {
  it('defaults to the owner-selected 500 ms hold and 30 second grace', () => {
    expect(DEFAULT_CONFIG.controllerInputHoldMs).toBe(500);
    expect(DEFAULT_CONFIG.controllerDisconnectGraceMs).toBe(30_000);
  });

  it('normalizes configurable wall-time values independently', () => {
    expect(normalizeConfig({
      controllerInputHoldMs: 750,
      controllerDisconnectGraceMs: 45_000
    })).toMatchObject({
      controllerInputHoldMs: 750,
      controllerDisconnectGraceMs: 45_000
    });
  });
});
