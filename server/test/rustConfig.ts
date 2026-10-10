import { DEFAULT_CONFIG, normalizeConfig } from '../config.ts';
import { RUST_CALCULATION_WORKER_MAX } from '../rustWorkers.ts';

/** Effective production defaults for real-server fixtures on the current CI process. */
export const RUST_TEST_CONFIG = normalizeConfig(DEFAULT_CONFIG);

/**
 * Keep a characterization worker count within the test process's CPU allowance.
 * @param requested - Positive comparison count from the fixture's scenario.
 * @returns The count this process can admit, without changing production defaults.
 */
export function rustWorkersForTest(requested: number): number {
  return Math.min(requested, RUST_CALCULATION_WORKER_MAX);
}
