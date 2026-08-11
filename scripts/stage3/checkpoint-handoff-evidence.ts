/**
 * Build and run the real Stage 3 Rust-to-Node checkpoint handoff tests.
 *
 * The feature-gated addon is compiled under a unique temporary Cargo target.
 * It never replaces the normal production addon, starts no server, opens no
 * listener, and writes checkpoint fixtures only beneath operating-system temp.
 */

import { spawnSync } from 'node:child_process';
import { copyFileSync, existsSync, mkdtempSync, rmSync } from 'node:fs';
import { homedir, tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

/** Repository root used as the fixed command working directory. */
const REPOSITORY_ROOT = fileURLToPath(new URL('../..', import.meta.url));

/** Native manifest built with the test-only feature. */
const NATIVE_MANIFEST = join(REPOSITORY_ROOT, 'native', 'Cargo.toml');

/** Exact integration test that refuses stale test-hook addons. */
const HANDOFF_TEST = join(
  REPOSITORY_ROOT,
  'server',
  'rustEngine',
  'checkpointPersistence.native.test.ts'
);

/** Repository-local Vitest entry point; no npm shim is required. */
const VITEST_ENTRY = join(REPOSITORY_ROOT, 'node_modules', 'vitest', 'vitest.mjs');

/** Hard ceiling for an isolated release build, including a cold Cargo cache. */
const BUILD_TIMEOUT_MS = 15 * 60 * 1_000;

/** Hard ceiling for four local integration assertions with no network listeners. */
const TEST_TIMEOUT_MS = 2 * 60 * 1_000;

/** Return the platform cdylib Cargo emits before it receives a `.node` suffix. */
function nativeLibraryFilename(): string {
  if (process.platform === 'win32' && process.arch === 'x64') return 'slither_native.dll';
  if (process.platform === 'linux' && process.arch === 'x64') return 'libslither_native.so';
  throw new Error(
    `Stage 3 checkpoint handoff evidence supports only x64 Windows and Linux, not ` +
    `${process.platform}/${process.arch}`
  );
}

/** Resolve Cargo in ordinary shells and rustup's standard non-login-shell location. */
function cargoCommand(): string {
  const explicit = process.env['CARGO']?.trim();
  if (explicit) return explicit;
  const rustupCargo = join(homedir(), '.cargo', 'bin', process.platform === 'win32'
    ? 'cargo.exe'
    : 'cargo');
  return existsSync(rustupCargo) ? rustupCargo : 'cargo';
}

/** Run one bounded child command and convert every abnormal exit into a failure. */
function runChecked(
  command: string,
  args: readonly string[],
  options: { environment: NodeJS.ProcessEnv; timeoutMs: number; label: string }
): void {
  const result = spawnSync(command, [...args], {
    cwd: REPOSITORY_ROOT,
    env: options.environment,
    stdio: 'inherit',
    timeout: options.timeoutMs,
    windowsHide: true
  });
  if (result.error) {
    throw new Error(`${options.label} failed to execute: ${result.error.message}`);
  }
  if (result.status !== 0) {
    throw new Error(
      `${options.label} exited with ${String(result.status)}${result.signal
        ? ` after signal ${result.signal}`
        : ''}`
    );
  }
}

/** Build one isolated hook addon, run every real assertion, and remove its build root. */
export function runCheckpointHandoffEvidence(): void {
  const temporaryRoot = mkdtempSync(join(tmpdir(), 'slither-stage3-checkpoint-handoff-build-'));
  const cargoTarget = join(temporaryRoot, 'cargo-target');
  const hookAddon = join(temporaryRoot, 'slither-native-stage3.node');
  try {
    const buildEnvironment: NodeJS.ProcessEnv = {
      ...process.env,
      CARGO_TARGET_DIR: cargoTarget
    };
    runChecked(
      cargoCommand(),
      [
        'build',
        '--manifest-path',
        NATIVE_MANIFEST,
        '--release',
        '--locked',
        '--features',
        'engine-test-hooks'
      ],
      {
        environment: buildEnvironment,
        timeoutMs: BUILD_TIMEOUT_MS,
        label: 'isolated engine-test-hooks Cargo build'
      }
    );
    copyFileSync(join(cargoTarget, 'release', nativeLibraryFilename()), hookAddon);

    const testEnvironment: NodeJS.ProcessEnv = {
      ...process.env,
      SLITHER_STAGE3_CHECKPOINT_TEST_ADDON: hookAddon
    };
    delete testEnvironment['NAPI_RS_NATIVE_LIBRARY_PATH'];
    runChecked(
      process.execPath,
      [VITEST_ENTRY, 'run', HANDOFF_TEST, '--reporter=verbose'],
      {
        environment: testEnvironment,
        timeoutMs: TEST_TIMEOUT_MS,
        label: 'Stage 3 checkpoint handoff integration test'
      }
    );
  } finally {
    rmSync(temporaryRoot, { recursive: true, force: true });
  }
}

/** Execute the tracked evidence path only when invoked as the program entry point. */
function main(): void {
  runCheckpointHandoffEvidence();
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  try {
    main();
  } catch (error) {
    console.error(error);
    process.exitCode = 1;
  }
}
