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
