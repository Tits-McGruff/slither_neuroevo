/** Deterministic RNG helpers for baseline bots and seeded workflows. */

/** Random source returning a float in [0, 1). */
export type RandomSource = () => number;

/** FNV-1a 32-bit offset basis. */
const FNV_OFFSET_BASIS = 0x811c9dc5;
/** FNV-1a 32-bit prime. */
const FNV_PRIME = 0x01000193;

/**
 * Normalize a number into an unsigned 32-bit integer.
 * @param value - Input value to normalize.
 * @returns Unsigned 32-bit integer.
 */
export function toUint32(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return (Math.floor(value) >>> 0);
}

/**
 * Hash one or more numeric inputs into a 32-bit seed.
 * @param values - Numeric inputs to mix into the hash.
 * @returns Unsigned 32-bit hash.
 */
export function hashSeed(...values: number[]): number {
  let hash = FNV_OFFSET_BASIS;
  for (const value of values) {
    hash ^= toUint32(value);
    hash = Math.imul(hash, FNV_PRIME);
  }
  return hash >>> 0;
}

/**
 * Create a deterministic RNG function from a 32-bit seed.
 * @param seed - Unsigned 32-bit seed value.
 * @returns Random source function returning [0,1).
 */
export function createRng(seed: number): RandomSource {
  let state = toUint32(seed) || 1;
  return () => {
    state ^= state << 13;
    state ^= state >>> 17;
    state ^= state << 5;
    return (state >>> 0) / 0x100000000;
  };
}
