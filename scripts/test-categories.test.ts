import { existsSync, readdirSync } from 'node:fs';
import { join, relative, resolve, sep } from 'node:path';
import { describe, expect, it } from 'vitest';
import {
  NATIVE_REQUIRED_TEST_FILES,
  PRIMARY_TEST_CATEGORIES,
  PRIMARY_TEST_CATEGORY_ORDER,
  TEST_CATEGORIES
} from './test-categories.ts';

/** Roots containing repository test files governed by the category manifest. */
const TEST_ROOTS = ['src', 'server', 'scripts'] as const;

/** Test-category manifest contract. */
const SUITE = 'test category manifest';

/**
 * Discover test files recursively below one repository directory.
 * @param directory - Absolute directory to scan.
 * @returns Repository-relative paths with portable separators.
 */
function discoverTests(directory: string): string[] {
  if (!existsSync(directory)) return [];
  const files: string[] = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const fullPath = join(directory, entry.name);
    if (entry.isDirectory()) {
      files.push(...discoverTests(fullPath));
    } else if (entry.isFile() && entry.name.endsWith('.test.ts')) {
      files.push(relative(process.cwd(), fullPath).split(sep).join('/'));
    }
  }
  return files;
}

describe(SUITE, () => {
  it('assigns every test file to exactly one primary layer', () => {
    const discovered = TEST_ROOTS
      .flatMap(root => discoverTests(resolve(root)))
      .sort();
    const assigned = PRIMARY_TEST_CATEGORY_ORDER
      .flatMap(category => [...PRIMARY_TEST_CATEGORIES[category]])
      .sort();
    const duplicates = assigned.filter((file, index) => assigned.indexOf(file) !== index);

    expect(duplicates).toEqual([]);
    expect(assigned).toEqual(discovered);
    expect(TEST_CATEGORIES.all).toHaveLength(discovered.length);
  });

  it('keeps the required-native overlay explicit and inside the complete suite', () => {
    const allFiles = new Set(TEST_CATEGORIES.all);
    expect(NATIVE_REQUIRED_TEST_FILES).toContain('src/brains/nativeBridge.test.ts');
    expect(NATIVE_REQUIRED_TEST_FILES).toContain('server/recoveryPhase4.brainPool.test.ts');
    expect(NATIVE_REQUIRED_TEST_FILES).toContain('server/recoveryPhase4.simServer.test.ts');
    expect(NATIVE_REQUIRED_TEST_FILES.every(file => allFiles.has(file))).toBe(true);
  });
});
