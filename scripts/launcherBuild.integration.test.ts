import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
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

it.runIf(process.platform === 'win32')('passes server and Vite arguments through the detached Windows launcher', () => {
  const root = mkdtempSync(join(tmpdir(), 'slither-windows-launcher-'));
  const workspace = join(root, 'workspace with spaces');
  mkdirSync(workspace);
  try {
    // The local npm substitute captures argv and stays alive until startup has observed it.
    writeFileSync(join(workspace, 'npm.cmd'), '@echo off\r\necho %*\r\nping -n 3 127.0.0.1 >nul\r\n');
    const result = spawnSync('powershell.exe', ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-File',
      resolve('scripts/test/windows-launcher-arguments.ps1'), '-LauncherPath', resolve('scripts/slither.ps1'),
      '-FixtureDirectory', workspace], { encoding: 'utf8', timeout: 15_000 });
    expect(result.error, `${result.stdout}\n${result.stderr}`).toBeUndefined();
    expect(result.status, `${result.stdout}\n${result.stderr}`).toBe(0);
    expect(readFileSync(join(workspace, 'server.log'), 'utf8').trim()).toBe('run server');
    expect(readFileSync(join(workspace, 'dev.log'), 'utf8').trim()).toBe('run dev -- --force');
  } finally { rmSync(root, { recursive: true, force: true }); }
}, 20_000);
