/** Regression guard for the compact selected-TypeScript fresh-run oracle. */

import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

/** Repository root derived without depending on the caller's working directory. */
const REPOSITORY_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
/** Executable selected-source fixture generator. */
const GENERATOR_PATH = join(
  REPOSITORY_ROOT,
  'scripts',
  'stage6',
  'generate-fresh-run-reference.ts'
);
/** Retained compact oracle covered by the native source identity. */
const FIXTURE_PATH = join(
  REPOSITORY_ROOT,
  'native',
  'fixtures',
  'fresh-run-reference.json'
);
/** Repository-local TypeScript execution shim. */
const TSX_CLI_PATH = join(REPOSITORY_ROOT, 'node_modules', 'tsx', 'dist', 'cli.mjs');

describe('Stage 6 selected TypeScript fresh-run fixture', () => {
  it('reproduces all default graph, population-digest, and RNG evidence', () => {
    const generated = JSON.parse(
      execFileSync(process.execPath, [TSX_CLI_PATH, GENERATOR_PATH], {
        cwd: REPOSITORY_ROOT,
        encoding: 'utf8'
      })
    ) as unknown;
    const retained = JSON.parse(readFileSync(FIXTURE_PATH, 'utf8')) as unknown;
    expect(generated).toEqual(retained);
  });
});
