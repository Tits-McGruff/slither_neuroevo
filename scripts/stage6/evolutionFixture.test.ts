/** Regression guard for the retained current-TypeScript evolution fixture. */

import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

/** Repository root derived independently of the caller's working directory. */
const REPOSITORY_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
/** Executable current-source fixture generator. */
const GENERATOR_PATH = join(
  REPOSITORY_ROOT,
  'scripts',
  'stage6',
  'generate-evolution-fixture.ts'
);
/** Retained fixture covered by the native build identity. */
const FIXTURE_PATH = join(
  REPOSITORY_ROOT,
  'native',
  'fixtures',
  'evolution-reference.json'
);
/** Repository-local TypeScript execution shim. */
const TSX_CLI_PATH = join(REPOSITORY_ROOT, 'node_modules', 'tsx', 'dist', 'cli.mjs');

describe('Stage 6 current-TypeScript evolution fixture', () => {
  it('reproduces fitness, history, Hall of Fame, weights and RNG continuation', () => {
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
