import { spawnSync } from 'node:child_process';
import { copyFileSync, existsSync, mkdirSync } from 'node:fs';
import { join, resolve } from 'node:path';

/** Exit code used when the build script fails. */
const EXIT_FAILURE = 1;
/** Rust target triple for wasm builds. */
const WASM_TARGET = 'wasm32-unknown-unknown';
/** Crate directory containing the SIMD kernel sources. */
const WASM_CRATE_DIR = resolve('wasm');
/** Name of the wasm output file produced by the crate. */
const WASM_ARTIFACT_NAME = 'slither_neuroevo_simd.wasm';
/** Name of the wasm asset bundled with the app. */
const WASM_OUTPUT_NAME = 'brains_simd.wasm';
/** Destination directory for bundled wasm assets. */
const WASM_OUTPUT_DIR = resolve('src', 'brains', 'wasm');

/**
 * Run a command and forward stdio to the current process.
 * @param {string} command - Command to execute.
 * @param {string[]} args - Arguments to pass to the command.
 * @param {string} cwd - Working directory to run in.
 */
function run(command, args, cwd) {
  const result = spawnSync(command, args, {
    cwd,
    stdio: 'inherit',
    env: {
      ...process.env,
      RUSTFLAGS: `${process.env.RUSTFLAGS ?? ''} -C target-feature=+simd128`.trim()
    }
  });
  if (result.error) {
    console.error(result.error.message);
    process.exit(EXIT_FAILURE);
  }
  if (result.status !== 0) {
    process.exit(result.status ?? EXIT_FAILURE);
  }
}

/**
 * Ensure the Rust toolchain is available.
 */
function ensureCargo() {
  const result = spawnSync('cargo', ['--version'], { stdio: 'ignore' });
  if (result.error) {
    console.error('cargo not found. Install Rust to build wasm kernels.');
    process.exit(EXIT_FAILURE);
  }
}

/**
 * Ensure the wasm target is installed via rustup.
 */
function ensureWasmTarget() {
  const rustup = spawnSync('rustup', ['--version'], { stdio: 'ignore' });
  if (rustup.error) {
    if (rustup.error.code === 'EPERM' || rustup.error.code === 'EACCES') {
      console.warn('rustup unavailable in this environment; skipping target validation.');
      return;
    }
    console.error('rustup not found. Install rustup to manage wasm targets.');
    process.exit(EXIT_FAILURE);
  }
  const list = spawnSync('rustup', ['target', 'list', '--installed'], {
    encoding: 'utf8'
  });
  if (list.error) {
    if (list.error.code === 'EPERM' || list.error.code === 'EACCES') {
      console.warn('rustup target query blocked; skipping target validation.');
      return;
    }
    console.error(list.error.message);
    process.exit(EXIT_FAILURE);
  }
  const installed = list.stdout
    .split('\n')
    .map((line) => line.trim())
    .filter(Boolean);
  if (installed.includes(WASM_TARGET)) return;
  const add = spawnSync('rustup', ['target', 'add', WASM_TARGET], { stdio: 'inherit' });
  if (add.error) {
    console.error(add.error.message);
    process.exit(EXIT_FAILURE);
  }
  if (add.status !== 0) {
    process.exit(add.status ?? EXIT_FAILURE);
  }
}

/**
 * Resolve the output wasm artifact path.
 * @returns {string} Absolute path to the wasm artifact.
 */
function resolveArtifactPath() {
  return join(WASM_CRATE_DIR, 'target', WASM_TARGET, 'release', WASM_ARTIFACT_NAME);
}

/**
 * Build the wasm kernels and copy the artifact into src/brains/wasm.
 */
function main() {
  if (!existsSync(WASM_CRATE_DIR)) {
    console.error(`Missing wasm crate directory: ${WASM_CRATE_DIR}`);
    process.exit(EXIT_FAILURE);
  }
  ensureCargo();
  ensureWasmTarget();
  run('cargo', ['build', '--release', '--target', WASM_TARGET], WASM_CRATE_DIR);
  const artifactPath = resolveArtifactPath();
  if (!existsSync(artifactPath)) {
    console.error(`Wasm artifact not found at ${artifactPath}`);
    process.exit(EXIT_FAILURE);
  }
  mkdirSync(WASM_OUTPUT_DIR, { recursive: true });
  const destPath = join(WASM_OUTPUT_DIR, WASM_OUTPUT_NAME);
  copyFileSync(artifactPath, destPath);
  console.log(`[wasm] wrote ${destPath}`);
}

main();
