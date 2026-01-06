import { spawnSync } from 'node:child_process';
import { existsSync, readdirSync } from 'node:fs';
import { join, resolve } from 'node:path';

const categories = {
  unit: ['unit.test.ts'],
  integration: ['integration.test.ts'],
  system: ['system.test.ts'],
  acceptance: ['acceptance.test.ts'],
  regression: ['regression.test.ts'],
  performance: ['performance.test.ts'],
  security: ['security.test.ts']
} as const;

type Category = keyof typeof categories;

const category = process.argv[2] as Category | undefined;
if (!category || !(category in categories)) {
  const allowed = Object.keys(categories).join(', ');
  console.error(`Usage: tsx scripts/run-tests.ts <category>\nCategories: ${allowed}`);
  process.exit(1);
}

const suffixes = categories[category];
const roots = ['src', 'server'];
const files: string[] = [];

function walk(dir: string): void {
  if (!existsSync(dir)) return;
  const entries = readdirSync(dir, { withFileTypes: true });
  for (const entry of entries) {
    const fullPath = join(dir, entry.name);
    if (entry.isDirectory()) {
      walk(fullPath);
      continue;
    }
    if (!entry.isFile()) continue;
    for (const suffix of suffixes) {
      if (entry.name.endsWith(suffix)) {
        files.push(fullPath);
        break;
      }
    }
  }
}

for (const root of roots) walk(resolve(root));

if (!files.length) {
  console.error(`No test files found for category "${category}".`);
  process.exit(1);
}

const vitestBin = resolve(
  'node_modules',
  '.bin',
  process.platform === 'win32' ? 'vitest.cmd' : 'vitest'
);

const result = spawnSync(vitestBin, ['run', ...files], { stdio: 'inherit' });
if (result.error) {
  console.error(result.error.message);
  process.exit(1);
}
process.exit(result.status ?? 1);
