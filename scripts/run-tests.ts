import { spawnSync } from 'node:child_process';
import { resolve } from 'node:path';
import { isTestCategory, TEST_CATEGORIES } from './test-categories.ts';

/** Selected category from CLI args. */
const category = process.argv[2];
if (!category || !isTestCategory(category)) {
  const allowed = Object.keys(TEST_CATEGORIES).join(', ');
  console.error(`Usage: tsx scripts/run-tests.ts <category>\nCategories: ${allowed}`);
  process.exit(1);
}

/** Absolute files selected by the explicit category manifest. */
const files = TEST_CATEGORIES[category].map(file => resolve(file));

/** Direct Vitest ES module entry point, avoiding platform-specific command wrappers. */
const vitestBin = resolve('node_modules', 'vitest', 'vitest.mjs');

/** Additional Vitest arguments forwarded after the category name. */
const forwardedArgs = process.argv.slice(3);

/** Prepare the separate test-hooks addon only for layers that exercise real Rust panics. */
if (files.includes(resolve('server/rustServer.panic.native.test.ts'))) {
  const prepare = spawnSync(process.execPath, [resolve('node_modules/tsx/dist/cli.mjs'),
    resolve('scripts/prepare-panic-test-addon.ts')], { stdio: 'inherit' });
  if (prepare.error) throw prepare.error;
  if (prepare.status !== 0) process.exit(prepare.status ?? 1);
}

/** Keep real-server timing diagnostics isolated from other files' durable disk workloads.
 * Native MT remains exercised inside each file; only independent files run serially.
 */
const isolationArgs = category === 'integration' || category === 'native-required' ? ['--maxWorkers=1'] : [];

const result = spawnSync(process.execPath, [vitestBin, 'run', ...files, ...isolationArgs, ...forwardedArgs], {
  stdio: 'inherit'
});
if (result.error) {
  console.error(result.error.message);
  process.exit(1);
}
process.exit(result.status ?? 1);
