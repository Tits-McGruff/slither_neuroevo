import { describe, it, expect } from 'vitest';
import { THEME, getPelletColor, getPelletGlow } from './theme.ts';

/** Test suite label for theme helpers. */
const SUITE = 'theme.ts';

describe(SUITE, () => {
  it('returns explicit pellet colors when provided', () => {
    const pellet = { color: '#123456', kind: 'ambient' };
    expect(getPelletColor(pellet)).toBe('#123456');
  });

  it('maps pellet kinds to theme colors and glows', () => {
    expect(getPelletColor({ kind: 'boost' })).toBe(THEME.pelletBoost);
    expect(getPelletColor({ kind: 'corpse_big' })).toBe(THEME.pelletCorpse);
    expect(getPelletGlow({ kind: 'boost' })).toBe(THEME.glowBoost);
    expect(getPelletGlow({ kind: 'corpse_small' })).toBe(THEME.glowCorpse);
    expect(getPelletGlow({ kind: 'ambient' })).toBe(THEME.glowAmbient);
  });
});
