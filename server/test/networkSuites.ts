import { describe, it } from 'vitest';

/** Explicit environment variable that disables tests requiring a TCP bind. */
export const NETWORK_TESTS_OPT_OUT_ENV = 'SLITHER_SKIP_NETWORK_TESTS';

/** Test body supported by the focused network test wrapper. */
type NetworkTestBody = () => void | Promise<void>;

/** Suite factory supported by the focused network suite wrapper. */
type NetworkSuiteFactory = () => void;

/** Whether the current process explicitly disabled network tests. */
const NETWORK_TESTS_DISABLED = isNetworkSuiteOptOut(process.env);

/** Whether the visible opt-out notice has already been emitted in this module instance. */
let didAnnounceOptOut = false;

/**
 * Check whether a process environment explicitly disables network suites.
 * @param env - Environment variables to inspect.
 * @returns True only for the documented value `1`.
 */
export function isNetworkSuiteOptOut(env: NodeJS.ProcessEnv): boolean {
  return env[NETWORK_TESTS_OPT_OUT_ENV] === '1';
}

/** Emit the required visible notice when network tests are explicitly disabled. */
function announceNetworkOptOut(): void {
  if (!NETWORK_TESTS_DISABLED || didAnnounceOptOut) return;
  didAnnounceOptOut = true;
  console.warn(
    `[tests.network.skip] ${NETWORK_TESTS_OPT_OUT_ENV}=1; TCP-bind suites are explicitly skipped`
  );
}

/**
 * Register a suite that requires local TCP binding.
 * @param name - User-visible suite name.
 * @param factory - Suite registration callback.
 */
export function describeNetworkSuite(name: string, factory: NetworkSuiteFactory): void {
  announceNetworkOptOut();
  describe.skipIf(NETWORK_TESTS_DISABLED)(name, factory);
}

/**
 * Register one test that requires local TCP binding inside a mixed suite.
 * @param name - User-visible test name.
 * @param body - Test body.
 * @param timeout - Optional Vitest timeout in milliseconds.
 */
export function networkTest(name: string, body: NetworkTestBody, timeout?: number): void {
  announceNetworkOptOut();
  it.skipIf(NETWORK_TESTS_DISABLED)(name, body, timeout);
}
