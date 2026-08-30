/** Regression guard for the retained TypeScript random-genome fixture. */

import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

/** Repository root derived without depending on the caller's working directory. */
const REPOSITORY_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
/** Executable fixture generator. */
const GENERATOR_PATH = join(
  REPOSITORY_ROOT,
  'scripts',
  'stage5',
  'generate-genome-init-fixture.ts'
);
/** Retained fixture covered by the native build identity. */
const FIXTURE_PATH = join(
  REPOSITORY_ROOT,
  'native',
  'fixtures',
  'genome-init-reference.json'
);
/** Repository-local TypeScript execution shim. */
const TSX_CLI_PATH = join(REPOSITORY_ROOT, 'node_modules', 'tsx', 'dist', 'cli.mjs');

describe('Stage 5 TypeScript random-genome fixture', () => {
  it('reproduces every retained Float32 bit and RNG continuation', () => {
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
