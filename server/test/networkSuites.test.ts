import { describe, expect, it } from 'vitest';
import { isNetworkSuiteOptOut, NETWORK_TESTS_OPT_OUT_ENV } from './networkSuites.ts';

/** Network-suite opt-out contract. */
const SUITE = 'network suite registration';

describe(SUITE, () => {
  it('requires the explicit documented value instead of inferring bind restrictions', () => {
    expect(isNetworkSuiteOptOut({})).toBe(false);
    expect(isNetworkSuiteOptOut({ [NETWORK_TESTS_OPT_OUT_ENV]: '0' })).toBe(false);
    expect(isNetworkSuiteOptOut({ [NETWORK_TESTS_OPT_OUT_ENV]: 'true' })).toBe(false);
    expect(isNetworkSuiteOptOut({ [NETWORK_TESTS_OPT_OUT_ENV]: '1' })).toBe(true);
  });
});
