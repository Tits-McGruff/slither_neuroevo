import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import { NETWORK_TESTS_OPT_OUT_ENV } from '../server/test/networkSuites.ts';

/** Phase 8 CI workflow contract. */
const SUITE = 'CI native and test-layer contract';

/** Authoritative workflow text inspected without adding a YAML dependency. */
const WORKFLOW = readFileSync(resolve('.github/workflows/CI.yml'), 'utf8');

/** User-facing setup documentation. */
const README = readFileSync(resolve('README.md'), 'utf8');

/** Root package manifest. */
const PACKAGE = JSON.parse(readFileSync(resolve('package.json'), 'utf8')) as {
  engines?: { node?: string };
};

/** Native package manifest. */
const NATIVE_PACKAGE = JSON.parse(readFileSync(resolve('native/package.json'), 'utf8')) as {
  engines?: { node?: string };
};

/** Version-manager pin for local development. */
const NVMRC = readFileSync(resolve('.nvmrc'), 'utf8').trim();

/** Root npm policy. */
const NPMRC = readFileSync(resolve('.npmrc'), 'utf8');

/** Native-package npm policy. */
const NATIVE_NPMRC = readFileSync(resolve('native/.npmrc'), 'utf8');

/**
 * Count literal occurrences in the workflow.
 * @param value - Literal text to count.
 * @returns Number of non-overlapping matches.
 */
function countOccurrences(value: string): number {
  return WORKFLOW.split(value).length - 1;
}

describe(SUITE, () => {
  it('keeps Node 24 as the minimum and tests Node 24/26 on Ubuntu and Windows', () => {
    expect(PACKAGE.engines?.node).toBe('>=24');
    expect(NATIVE_PACKAGE.engines?.node).toBe('>=24');
    expect(NVMRC).toBe('24');
    expect(NPMRC).toContain('engine-strict=true');
    expect(NATIVE_NPMRC).toContain('engine-strict=true');
    expect(README).toContain('- **Node.js**: v24 or newer');
    expect(README).toContain('Use Node 24+');
    expect(WORKFLOW).toContain('os: [ubuntu-latest, windows-latest]');
    expect(WORKFLOW).toContain('node-version: [24.x, 26.x]');
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
