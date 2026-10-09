import { mkdtempSync, rmSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { join, resolve } from 'node:path';
import { tmpdir } from 'node:os';
import { expect, it } from 'vitest';

/** Real POSIX shell, with Git for Windows providing the same fixture locally. */
const POSIX_SHELL = process.platform === 'win32'
  ? resolve(process.env['ProgramFiles'] ?? 'C:/Program Files', 'Git/bin/bash.exe') : '/bin/sh';

it('builds the browser for the runtime port and invalidates cached routing inputs', () => {
  const root = mkdtempSync(join(tmpdir(), 'slither-launcher-build-'));
  try {
    const result = spawnSync(POSIX_SHELL, [resolve('scripts/test/launcher-build-cache.sh').replaceAll('\\', '/'),
      resolve('play.sh').replaceAll('\\', '/'), root.replaceAll('\\', '/')], { encoding: 'utf8', timeout: 5000 });
    expect(result.error, `${result.stdout}\n${result.stderr}`).toBeUndefined();
    expect(result.status, result.stderr).toBe(0);
    expect(result.stdout.split(/\r?\n/u).filter(line => line.startsWith('build='))).toEqual([
      'build=6200|', 'build=6201|', 'build=6201|ws://split-host:6201'
    ]);
  } finally { rmSync(root, { recursive: true, force: true }); }
});
