import { describe, expect, it } from 'vitest';
import type { RustBackgroundHealth } from '../../src/protocol/rustBackground.ts';
import { AuthorityProgressDeadline } from './authorityProgressWatchdog.ts';

/** Supply the exact fields sampled by the progress watchdog. */
function health(boundary: number, loopState = 'ready', worldEpoch = '0000000000000001'): Pick<
  RustBackgroundHealth, 'lifecycle' | 'loopState' | 'worldEpoch' | 'commandServiceBoundaries'> {
  return { lifecycle: 'running', loopState, worldEpoch,
    commandServiceBoundaries: boundary.toString(16).padStart(16, '0') };
}

describe('Rust authority progress watchdog', () => {
  it('faults only after a continuous ready-state stall', () => {
    const deadline = new AuthorityProgressDeadline();
    expect(deadline.observe(health(1), 0, 5_000)).toBeNull();
    expect(deadline.observe(health(2), 4_000, 5_000)).toBeNull();
    expect(deadline.observe(health(2), 8_999, 5_000)).toBeNull();
    expect(deadline.observe(health(2), 9_000, 5_000)?.message).toMatch(/no scheduler progress/u);
  });

  it('does not time intentional barriers, replacements, or a paused Node process', () => {
    const deadline = new AuthorityProgressDeadline();
    expect(deadline.observe(health(8), 0, 5_000)).toBeNull();
    expect(deadline.observe(health(8, 'generationTransitionPending'), 4_000, 5_000)).toBeNull();
    expect(deadline.observe(health(8), 4_500, 5_000)).toBeNull();
    expect(deadline.observe(health(8), 40_000, 5_000)).toBeNull();
    expect(deadline.observe(health(8), 40_500, 5_000)).toBeNull();
    expect(deadline.observe(health(0, 'ready', '0000000000000002'), 41_000, 5_000)).toBeNull();
  });
});
