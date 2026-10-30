import { createRequire } from 'node:module';
import { mkdtempSync, rmSync, readFileSync } from 'node:fs';
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

it('passes an explicit native memory budget through normal server startup and preserves checkpoint arithmetic on resume', async () => {
  const backend = 'rust-i16-matrix-fma128-v1';
  if (!admitted(backend)) return;
  const root = mkdtempSync(join(tmpdir(), 'slither-native-memory-'));
  const config = { ...RUST_TEST_CONFIG, port: 0, dbPath: join(root, 'owner.db'),
    resume: 'fresh' as const, seed: 42, rustCalculationWorkers: 1, mathBackend: backend as InferenceMathBackend };
  try {
    // A rejected fresh initialization has no committed lineage. Keep that
    // disposable store separate from the successful startup/resume fixture.
    const rejected = await startRustServer({ ...config, dbPath: join(root, 'rejected.db'), nativeMemoryMiB: 256 });
    try { expect(rejected.startupFault).toContain('268435456-byte ceiling'); } finally { await rejected.close(); }
    const started = await startRustServer({ ...config, nativeMemoryMiB: 6144 });
    let checkpoint: unknown;
    try {
      expect(started.startupFault).toBeUndefined();
      const response = await fetch(`http://127.0.0.1:${started.port}/api/health`);
      const health = await response.json() as Record<string, unknown>;
      expect(health['mathBackend']).toBe(backend);
      checkpoint = health['startupCheckpointId'];
    } finally { await started.close(); }
    const resumed = await createExperimentalServerRuntime({ databasePath: config.dbPath,
      managedDirectory: `${config.dbPath}.checkpoints`, nativeMemoryMiB: 6144,
      restoreLatest: true, mathBackend: backend, calculationWorkers: 1, onWake() {} });
    try {
      expect(resumed.metadata.mathBackend).toBe(backend);
      expect(resumed.runStart.checkpointId).toBe(checkpoint);
    } finally { await resumed.close(); }
  } finally { rmSync(root, { recursive: true, force: true }); }
}, 30_000);

it('starts a population-only i16 lineage without rewriting its scalar source and resumes it normally', async () => {
  const backend = 'rust-i16-matrix-fma128-v1';
  if (!admitted(backend)) return;
  const root = mkdtempSync(join(tmpdir(), 'slither-population-fork-'));
  const options = { databasePath: join(root, 'owner.db'), managedDirectory: join(root, 'checkpoints'),
    calculationWorkers: 1, onWake() {} };
  try {
    const source = await createExperimentalServerRuntime({ ...options, seed: 42, mathBackend: 'rust-scalar-v1' });
    const descriptor = source.runStart.descriptor;
    const metadata = source.metadata;
    await source.close();
    const sourceBytes = readFileSync(join(options.managedDirectory, descriptor.relativeFilename));
    const fork = await createExperimentalServerRuntime({ ...options, populationFromCheckpointId: descriptor.logicalRootSha256,
      mathBackend: backend });
    const forkCheckpoint = fork.runStart.checkpointId;
    try {
      expect(fork.metadata).toMatchObject({ mathBackend: backend, seed: metadata.seed,
        settings: metadata.settings, graphSpec: metadata.graphSpec, parameterCount: metadata.parameterCount });
      expect(fork.metadata.runId).toContain(`population-fork-${descriptor.logicalRootSha256}-`);
      expect(fork.runStart.descriptor).toMatchObject({ boundaryKind: 'run-start', generation: '0000000000000001',
        completedStep: '0000000000000000', weightCount: descriptor.weightCount, populationCount: descriptor.populationCount });
      expect(forkCheckpoint).not.toBe(descriptor.logicalRootSha256);
      expect(readFileSync(join(options.managedDirectory, descriptor.relativeFilename)).equals(sourceBytes)).toBe(true);
    } finally { await fork.close(); }
    const resumed = await createExperimentalServerRuntime({ ...options, restoreLatest: true, mathBackend: backend });
    try {
      expect(resumed.metadata.mathBackend).toBe(backend);
      expect(resumed.runStart.checkpointId).toBe(forkCheckpoint);
    } finally { await resumed.close(); }
    await expect(createExperimentalServerRuntime({ ...options, restoreLatest: true,
      populationFromCheckpointId: descriptor.logicalRootSha256, mathBackend: backend })).rejects.toThrow(/fresh lineage/u);
    await expect(createExperimentalServerRuntime({ ...options, populationFromCheckpointId: '0'.repeat(64),
      mathBackend: backend })).rejects.toThrow(/checkpoint|retained/u);
    const afterFailure = await createExperimentalServerRuntime({ ...options, restoreLatest: true, mathBackend: backend });
    try { expect(afterFailure.runStart.checkpointId).toBe(forkCheckpoint); } finally { await afterFailure.close(); }
  } finally { rmSync(root, { recursive: true, force: true }); }
}, 30_000);
