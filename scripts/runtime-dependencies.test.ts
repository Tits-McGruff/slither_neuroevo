import {describe, expect, it} from 'vitest';
import {existsSync, readFileSync} from 'node:fs';
import {mkdtemp, rm, writeFile} from 'node:fs/promises';
import {dirname, relative, resolve} from 'node:path';
import {tmpdir} from 'node:os';
import ts from 'typescript';

const ROOT = resolve('.');
const PACKAGE = JSON.parse(readFileSync('package.json', 'utf8')) as {scripts?: Record<string,string>};
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


describe('retired runtime dependency boundaries', () => {
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

});
