import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import { NETWORK_TESTS_OPT_OUT_ENV } from '../server/test/networkSuites.ts';

/** Phase 8 CI workflow contract. */
const SUITE = 'CI native and test-layer contract';

/** Authoritative workflow text inspected without adding a YAML dependency. */
const WORKFLOW = readFileSync(resolve('.github/workflows/CI.yml'), 'utf8');

/**
 * Count literal occurrences in the workflow.
 * @param value - Literal text to count.
 * @returns Number of non-overlapping matches.
 */
function countOccurrences(value: string): number {
  return WORKFLOW.split(value).length - 1;
}

describe(SUITE, () => {
  it('keeps Ubuntu and Windows on the Node 22/24 native matrix', () => {
    expect(WORKFLOW).toContain('os: [ubuntu-latest, windows-latest]');
    expect(WORKFLOW).toContain('node-version: [22.x, 24.x]');
  });

  it('builds the addon once and executes native MT in that same matrix job', () => {
    expect(countOccurrences('npm --prefix native run build')).toBe(1);
    expect(WORKFLOW).toContain('npm run test:native-required');
    expect(WORKFLOW).toContain('nativeAddonBuildIdentifier');
    expect(WORKFLOW).not.toContain('npm run test:native ');
    expect(WORKFLOW).not.toContain('npm run build\n');
  });

  it('runs every explicit JavaScript layer without hiding bind failures', () => {
    for (const command of [
      'test:unit',
      'test:component',
      'test:integration',
      'test:system',
      'test:acceptance',
      'test:regression',
      'test:performance',
      'test:security'
    ]) {
      expect(WORKFLOW).toContain(`npm run ${command}`);
    }
    expect(WORKFLOW).not.toContain(NETWORK_TESTS_OPT_OUT_ENV);
  });

  it('runs Rust tests, formatting, clippy, TypeScript, ESLint, and the client build', () => {
    expect(WORKFLOW).toContain('cargo test --manifest-path native/Cargo.toml --release');
    expect(WORKFLOW).toContain('cargo fmt -- --check');
    expect(WORKFLOW).toContain('cargo clippy -- -D warnings');
    expect(WORKFLOW).toContain('npm run build:client');
    expect(WORKFLOW).toContain('npm run typecheck');
    expect(WORKFLOW).toContain('npm run lint');
  });
});
