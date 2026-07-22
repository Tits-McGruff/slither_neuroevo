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

const result = spawnSync(process.execPath, [vitestBin, 'run', ...files, ...forwardedArgs], {
  stdio: 'inherit'
});
if (result.error) {
  console.error(result.error.message);
  process.exit(1);
}
process.exit(result.status ?? 1);
