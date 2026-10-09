/** Reusable Phase 2 full-World fixture using normal production construction. */

import type { CoreSettings } from '../../src/protocol/settings.ts';
import { SimCore } from '../../src/sim/SimCore.ts';
import type { GenerationBoundaryHook } from '../../src/world.ts';

/** Options accepted by the seeded production fixture. */
export interface SeededWorldFixtureOptions {
  /** Root run seed. */
  seed: number;
  /** Optional authoritative world settings. */
  settings?: Partial<CoreSettings>;
  /** Optional deterministic test run id. */
  runId?: string;
  /** Fixed scheduler tick rate. */
  tickRateHz?: number;
  /** Optional exact pre-spawn generation-boundary observer. */
  onGenerationBoundary?: GenerationBoundaryHook;
}

/**
 * Construct a complete seeded World through the production SimCore path.
 * @param options - Seed, settings, identity, and tick-rate controls.
 * @returns Production SimCore owning the seeded World.
 */
export function createSeededWorldFixture(options: SeededWorldFixtureOptions): SimCore {
  const boundary = options.onGenerationBoundary
    ? { onGenerationBoundary: options.onGenerationBoundary }
    : {};
  return new SimCore({
    settings: options.settings ?? {},
    worldSeed: options.seed,
    runId: options.runId ?? `fixture-${options.seed >>> 0}`,
    tickRateHz: options.tickRateHz ?? 60,
    ...boundary
  });
}
