/** System-entropy helpers for run seeds and non-simulation identities. */

import { randomBytes, randomUUID } from 'node:crypto';
import { normalizeSeed } from '../src/rng.ts';

/** Maximum entropy redraws before using a deterministic unequal fallback. */
const MAX_ENTROPY_REDRAWS = 8;
/** Odd Uint32 increment used only after repeated entropy collisions. */
const SEED_FALLBACK_INCREMENT = 0x9e3779b9;

/**
 * Create a Uint32 seed from operating-system entropy.
 * @param exclude - Optional current seed that the result must differ from.
 * @returns Entropy-derived normalized seed.
 */
export function createEntropySeed(exclude?: number): number {
  const excluded = exclude == null ? null : normalizeSeed(exclude);
  for (let attempt = 0; attempt < MAX_ENTROPY_REDRAWS; attempt++) {
    const seed = randomBytes(4).readUInt32LE(0);
    if (excluded === null || seed !== excluded) return seed;
  }
  return ((excluded ?? 0) + SEED_FALLBACK_INCREMENT) >>> 0;
}

/**
 * Create a lineage id independently from every simulation random stream.
 * @returns Cryptographically random UUID.
 */
export function createRunId(): string {
  return randomUUID();
}

/**
 * Create a transport-session id independently from run identity and simulation RNG.
 * @returns Cryptographically random UUID.
 */
export function createSessionId(): string {
  return randomUUID();
}
