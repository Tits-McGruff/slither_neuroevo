import { existsSync, readFileSync } from 'node:fs';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, relative, resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import ts from 'typescript';
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
 * Follow value imports, exports and literal dynamic loads from one entry point.
 * Explicit type declarations are erased; source-checked native loaders are
 * the only permitted computed loads in the inspected dependency graph.
 * @param entry - Repository-relative TypeScript entry point.
 * @returns Normalized repository-relative dependency paths, including entry.
 */
function runtimeTypeScriptDependencies(entry: string): Set<string> {
  const pending = [resolve(entry)];
  const visited = new Set<string>();
  while (pending.length > 0) {
    const file = pending.pop()!;
    if (visited.has(file)) continue;
    visited.add(file);
    const source = ts.createSourceFile(file, readFileSync(file, 'utf8'), ts.ScriptTarget.Latest, true);
    /** Resolve a literal relative load without executing the module. */
    const include = (specifier: string): void => {
      if (!specifier.startsWith('.')) return;
      let candidate = resolve(dirname(file), specifier);
      if (!existsSync(candidate) && existsSync(`${candidate}.ts`)) candidate = `${candidate}.ts`;
      if (!existsSync(candidate)) throw new Error(`unresolved relative runtime import: ${file} -> ${specifier}`);
      pending.push(candidate);
    };
    /** Visit syntax nodes so comments and multiline imports cannot hide a load. */
    const visit = (node: ts.Node): void => {
      if ((ts.isImportDeclaration(node) && !node.importClause?.isTypeOnly ||
          ts.isExportDeclaration(node) && !node.isTypeOnly) &&
          node.moduleSpecifier && ts.isStringLiteralLike(node.moduleSpecifier)) include(node.moduleSpecifier.text);
      if (ts.isImportEqualsDeclaration(node) && !node.isTypeOnly &&
          ts.isExternalModuleReference(node.moduleReference) &&
          node.moduleReference.expression && ts.isStringLiteralLike(node.moduleReference.expression)) {
        include(node.moduleReference.expression.text);
      }
      if (ts.isCallExpression(node) && (node.expression.kind === ts.SyntaxKind.ImportKeyword ||
          ts.isIdentifier(node.expression) && node.expression.text === 'require')) {
        const argument = node.arguments[0];
        if (argument && ts.isStringLiteralLike(argument)) include(argument.text);
        else if (!(file === resolve('server/rustEngine/experimentalStartup.ts') &&
            node.getText(source) === "require(resolve(NATIVE_DIRECTORY, 'index.js'))")) {
          throw new Error(`uninspectable runtime load: ${relative(ROOT, file)}: ${node.getText(source)}`);
        }
      }
      ts.forEachChild(node, visit);
    };
    visit(source);
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
  it('contains only native authority and keeps surviving entry points independent of retired execution', () => {
    expect(PACKAGE.scripts?.['server']).toBe('tsx server/rustServer.ts');
    expect(PACKAGE.scripts).not.toHaveProperty('server:reference');
    expect(PACKAGE.scripts).not.toHaveProperty('server:reference:dev');
    const retired = [
      'server/index.ts', 'server/simServer.ts', 'server/brainPool.ts', 'server/brainPoolProtocol.ts',
      'server/worker/inferWorker.ts', 'server/inferenceMode.ts', 'server/controllerRegistry.ts',
      'server/httpApi.ts', 'server/persistence.ts', 'server/checkpoint.ts', 'server/startupResume.ts',
      'src/sim/SimCore.ts', 'src/world.ts', 'src/snake.ts', 'src/mlp.ts', 'src/sensors.ts',
      'src/spatialHash.ts', 'src/serializer.ts', 'src/rng.ts', 'src/bots/baselineBots.ts',
      'src/brains/ops.ts', 'src/brains/graph/runtime.ts', 'src/brains/nativeBridge.ts',
      'src/brains/types.ts', 'src/brains/registry.ts', 'src/brains/nullBrain.ts'
    ];
    for (const file of retired) expect(existsSync(resolve(file)), `retired implementation remains: ${file}`).toBe(false);
    for (const entry of ['src/main.ts', 'server/rustServer.ts',
      'server/rustEngine/checkpointPersistenceWorker.ts', 'scripts/resolve-launcher-config.ts',
      'scripts/backup-production.ts', 'scripts/restore-production.ts',
      'scripts/stage7/realtime-workload.ts', 'scripts/stage7/codec-archive-fixture.ts',
      'scripts/stage7/compact-legacy-database.ts']) {
      const dependencies = runtimeTypeScriptDependencies(entry);
      for (const file of retired) expect(dependencies.has(file), `${entry} reached ${file}`).toBe(false);
    }
  });

  it('detects dynamic and CommonJS loads while excluding erased type-only declarations', async () => {
    const root = await mkdtemp(resolve(tmpdir(), 'slither-runtime-imports-'));
    if (dirname(root) !== resolve(tmpdir())) throw new Error('unexpected import fixture cleanup path');
    try {
      for (const name of ['value', 'dynamic', 'required', 'legacy', 'types']) {
        await writeFile(resolve(root, `${name}.ts`), 'export const marker = 1;\n');
      }
      await writeFile(resolve(root, 'entry.ts'), `
        import type { marker } from './types.ts';
        export { marker } from './value.ts';
        void import('./dynamic.ts');
        require('./required.ts');
        import legacy = require('./legacy.ts');
      `);
      const dependencies = runtimeTypeScriptDependencies(resolve(root, 'entry.ts'));
      /** Normalize a temporary fixture path like the production graph paths. */
      const key = (name: string): string => relative(ROOT, resolve(root, `${name}.ts`)).replaceAll('\\', '/');
      expect(dependencies.has(key('types'))).toBe(false);
      for (const name of ['value', 'dynamic', 'required', 'legacy']) expect(dependencies.has(key(name))).toBe(true);
      for (const load of ['void import(selectedFallback);', 'require(selectedFallback);']) {
        await writeFile(resolve(root, 'entry.ts'), `${load}\n`);
        expect(() => runtimeTypeScriptDependencies(resolve(root, 'entry.ts'))).toThrow(/uninspectable runtime load/);
      }
    } finally { await rm(root, { recursive: true, force: true }); }
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
