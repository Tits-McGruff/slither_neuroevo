import { availableParallelism } from 'node:os';

/** Process-available logical CPUs detected once during server module startup. */
export const RUST_CALCULATION_WORKER_MAX = availableParallelism();

/**
 * Validate a manual Rust worker count without changing the separate Node MT pool.
 * @param count - Requested integer count; zero has no automatic meaning.
 * @param maximum - Detected CPU count, supplied explicitly by deterministic tests.
 * @returns The admitted count.
 */
export function validateRustCalculationWorkers(count: number, maximum = RUST_CALCULATION_WORKER_MAX): number {
  if (!Number.isSafeInteger(maximum) || maximum < 1) throw new RangeError('Rust worker maximum must be a positive safe integer');
  if (!Number.isSafeInteger(count) || count < 1 || count > maximum) {
    throw new RangeError(`Rust calculation workers must be an integer from 1 to ${maximum} (available logical CPUs)`);
  }
  return count;
}
