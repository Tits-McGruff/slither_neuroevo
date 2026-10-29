import { createRequire } from 'node:module';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { expect, it } from 'vitest';
import { INFERENCE_MATH_BACKENDS, type InferenceMathBackend } from '../config.ts';
import { RUST_TEST_CONFIG } from '../test/rustConfig.ts';
import { startRustServer } from '../rustServer.ts';
import { createExperimentalServerRuntime } from './experimentalStartup.ts';
import type { ExperimentalFreshRunNativeBinding } from './experimentalFreshRunSession.ts';
import { createRustWelcome } from './browserMetadata.ts';

/** Real production addon; missing/stale binaries must fail this native suite. */
const binding = createRequire(import.meta.url)(resolve('native/index.js')) as ExperimentalFreshRunNativeBinding;

/** Exercise actual CPU admission without constructing a population or calling SIMD. */
function admitted(backend: InferenceMathBackend): boolean {
  try {
    new binding.ExperimentalStage6aFreshRunSession('backend-probe', '0000002a', '0000000100000000', 1, backend);
    return true;
  } catch (error) {
    expect(error).toBeInstanceOf(Error);
    expect((error as Error).message).toContain(`inference backend ${backend} is unavailable on this CPU`);
    return false;
  }
}

it.each(INFERENCE_MATH_BACKENDS)('selects and restores explicit %s through ordinary production startup', async backend => {
  if (!admitted(backend)) return; // The clean unsupported-CPU rejection was asserted above.
  const root = mkdtempSync(join(tmpdir(), 'slither-math-startup-'));
  const options = { databasePath: join(root, 'owner.db'), managedDirectory: join(root, 'checkpoints'),
    calculationWorkers: 1, onWake() {} };
  try {
    const fresh = await createExperimentalServerRuntime({ ...options, seed: 42, mathBackend: backend });
    const checkpoint = fresh.runStart.checkpointId;
    try {
      expect(fresh.metadata.mathBackend).toBe(backend);
      expect(createRustWelcome(fresh.metadata, 'backend-test').inferenceMode.mathBackend).toBe(backend);
    } finally { await fresh.close(); }
    // No override means retain persisted arithmetic, never silently reselect it.
    const restored = await createExperimentalServerRuntime({ ...options, restoreCurrent: true });
    try {
      expect(restored.metadata.mathBackend).toBe(backend);
      expect(restored.runStart.checkpointId).toBe(checkpoint);
    } finally { await restored.close(); }
  } finally { rmSync(root, { recursive: true, force: true }); }
}, 30_000);

it('rejects explicit checkpoint arithmetic mismatch without recovery, pointer changes or reinterpretation', async () => {
  const backend = 'rust-i16-matrix-fma128-v1';
  if (!admitted(backend)) return;
  const root = mkdtempSync(join(tmpdir(), 'slither-math-mismatch-'));
  const options = { databasePath: join(root, 'owner.db'), managedDirectory: join(root, 'checkpoints'),
    calculationWorkers: 1, onWake() {} };
  try {
    const fresh = await createExperimentalServerRuntime({ ...options, seed: 42, mathBackend: backend });
    const checkpoint = fresh.runStart.checkpointId;
    await fresh.close();
    await expect(createExperimentalServerRuntime({ ...options, restoreLatest: true, mathBackend: 'rust-sse2-v1' }))
      .rejects.toThrow(/explicit inference backend mismatch:/u);
    const restored = await createExperimentalServerRuntime({ ...options, restoreCurrent: true, mathBackend: backend });
    try {
      expect(restored.runStart.checkpointId).toBe(checkpoint);
      expect(restored.recovery).toBeNull();
      expect(restored.metadata.mathBackend).toBe(backend);
    } finally { await restored.close(); }
  } finally { rmSync(root, { recursive: true, force: true }); }
}, 30_000);

it.each([undefined, 'rust-i16-matrix-fma128-v1'] as const)('passes normal server configuration %s into native authority and health', async backend => {
  if (backend && !admitted(backend)) return;
  const root = mkdtempSync(join(tmpdir(), 'slither-math-server-'));
  try {
    const server = await startRustServer({ ...RUST_TEST_CONFIG, port: 0, dbPath: join(root, 'owner.db'),
      resume: 'fresh', seed: 42, rustCalculationWorkers: 1, ...(backend ? { mathBackend: backend } : {}) });
    try {
      expect(server.startupFault).toBeUndefined();
      const response = await fetch(`http://127.0.0.1:${server.port}/api/health`);
      expect(response.status).toBe(200);
      expect(await response.json()).toMatchObject({ mathBackend: backend ?? 'rust-sse2-v1' });
    } finally { await server.close(); }
  } finally { rmSync(root, { recursive: true, force: true }); }
}, 30_000);
