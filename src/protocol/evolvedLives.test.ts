import { describe, expect, it } from 'vitest';
import { parseEvolvedLifecycleTelemetry, type EvolvedLifecycleTelemetry } from './evolvedLives.ts';

/** Construct small native-shaped diagnostics with all five lifecycle states. */
function fixture(): EvolvedLifecycleTelemetry {
  return { version: 1, population: 5, alive: 1, waiting: 1, pendingPlacement: 1, exhausted: 1, deadNoRespawn: 1,
    lives: '0000000000000005', deaths: '0000000000000004', respawns: '0000000000000000', failedPlacements: '000000000000000a', pendingSeconds: 10,
    recentEventCount: 4, lifeEndCursor: '0000000000000004', placement: { step: '000000000000000a', eligible: 5, attempted: 3, placed: 1, pendingNoPlacement: 1, pendingWorkLimit: 1, deferred: 2, candidates: 768, geometryChecks: 250000 } };
}

describe('committed evolved lifecycle wire diagnostics', () => {
  it('retains valid native facts without copying or narrowing exact lifetime counters', () => {
    const value = fixture();
    expect(parseEvolvedLifecycleTelemetry(value)).toBe(value);
    value.lives = 'ffffffffffffffff'; value.respawns = 'fffffffffffffffa'; value.deaths = 'fffffffffffffffe'; value.failedPlacements = 'ffffffffffffffff';
    expect(parseEvolvedLifecycleTelemetry(value)).toBe(value);
  });
  it.each([undefined, null, [], {}, { ...fixture(), version: 2 }, { ...fixture(), waiting: -1 }, { ...fixture(), pendingPlacement: 2 },
    { ...fixture(), lives: '5' }, { ...fixture(), deaths: 'ffffffffffffffff' }, { ...fixture(), respawns: '0000000000000001' },
    { ...fixture(), pendingSeconds: Number.NaN }, { ...fixture(), recentEventCount: 257 }, { ...fixture(), lifeEndCursor: '0000000000000000' }])(
    'clears absent or invalid life metadata %#', value => { expect(parseEvolvedLifecycleTelemetry(value)).toBeUndefined(); }
  );
  it.each([{ attempted: 9 }, { placed: 4 }, { deferred: 1 }, { eligible: 6 }, { candidates: 2049 }, { geometryChecks: 250001 }, { step: 'future' }])(
    'rejects impossible work partitions/caps %#', change => { const value = fixture(); Object.assign(value.placement, change); expect(parseEvolvedLifecycleTelemetry(value)).toBeUndefined(); }
  );
});
