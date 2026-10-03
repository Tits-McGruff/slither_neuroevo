/** Bound generation publication times from consecutive real health observations. */

/** One generation counter snapshot inside its HTTP request/reply interval. */
export interface GenerationRead {
  /** Canonical Uint64 generation identity. */
  generation: string;
  /** Monotonic time before requesting the snapshot. */
  beforeMs: number;
  /** Monotonic time after reading the response. */
  afterMs: number;
}

/** A publication lies after the last old snapshot and before the first new one. */
export type GenerationPublication = GenerationRead;

/** Validate observation brackets before interpreting a generation counter. */
function validate(read: GenerationRead): bigint {
  if (!/^[0-9a-f]{16}$/u.test(read.generation) ||
      ![read.beforeMs, read.afterMs].every(value => Number.isFinite(value) && value >= 0) ||
      read.afterMs < read.beforeMs) throw new Error('invalid generation observation');
  return BigInt(`0x${read.generation}`);
}

/** Preserve uncertainty; a missed or regressed generation cannot prove its duration. */
export function generationPublication(previous: GenerationRead, current: GenerationRead): GenerationPublication | undefined {
  const delta = validate(current) - validate(previous);
  if (current.beforeMs <= previous.afterMs) throw new Error('generation request intervals overlap or touch');
  if (delta < 0n || delta > 1n) throw new Error('regressed or unobserved generation publication');
  if (delta === 0n) return undefined;
  return { generation: current.generation, beforeMs: previous.beforeMs, afterMs: current.afterMs };
}

/** Bound a complete generation, excluding the first partial generation in a window. */
export function generationInterval(previous: GenerationPublication, current: GenerationPublication): {
  minimumSeconds: number; maximumSeconds: number;
} {
  if (validate(current) - validate(previous) !== 1n ||
      current.beforeMs < previous.beforeMs || current.afterMs <= previous.afterMs) {
    throw new Error('unordered or unobserved generation interval');
  }
  return { minimumSeconds: Math.max(0, (current.beforeMs - previous.afterMs) / 1000),
    maximumSeconds: (current.afterMs - previous.beforeMs) / 1000 };
}
