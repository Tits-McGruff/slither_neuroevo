import { existsSync, readFileSync } from 'node:fs';
import { dirname, relative, resolve } from 'node:path';
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
  scripts?: Record<string, string>;
};

/** Repository root used to normalize production dependency paths. */
const ROOT = resolve('.');

/**
 * Follow static relative TypeScript imports from one production entry point.
 * @param entry - Repository-relative TypeScript entry point.
 * @returns Normalized repository-relative dependency paths, including entry.
 */
function staticTypeScriptDependencies(entry: string): Set<string> {
  const pending = [resolve(entry)];
  const visited = new Set<string>();
  const importPattern = /(?:import|export)\s+(?:type\s+)?(?:[^'";]*?\s+from\s+)?['"](?<path>[^'"]+)['"]/gu;
  while (pending.length > 0) {
    const file = pending.pop()!;
    if (visited.has(file)) continue;
    visited.add(file);
    const source = readFileSync(file, 'utf8');
    for (const match of source.matchAll(importPattern)) {
      const specifier = match.groups?.['path'];
      if (!specifier?.startsWith('.')) continue;
      let candidate = resolve(dirname(file), specifier);
      if (!existsSync(candidate) && existsSync(`${candidate}.ts`)) candidate = `${candidate}.ts`;
      if (existsSync(candidate)) pending.push(candidate);
    }
  }
  return new Set([...visited].map(file => relative(ROOT, file).replaceAll('\\', '/')));
}

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
  it('makes Rust the normal server and isolates the TypeScript reference entry point', () => {
    expect(PACKAGE.scripts?.['server']).toBe('tsx server/rustServer.ts');
    expect(PACKAGE.scripts?.['server:reference']).toBe('tsx server/index.ts');
    const production = staticTypeScriptDependencies('server/rustServer.ts');
    for (const forbidden of [
      'server/index.ts',
      'server/simServer.ts',
      'server/brainPool.ts',
      'server/worker/inferWorker.ts',
      'src/sim/SimCore.ts',
      'src/world.ts'
    ]) {
      expect(production.has(forbidden), `production dependency reached ${forbidden}`).toBe(false);
    }
  });

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
