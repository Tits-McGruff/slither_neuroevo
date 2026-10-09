/** Explicit test-only composition; normal startup never imports this module. */
import { randomBytes, randomUUID } from 'node:crypto';
import { mkdir } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { createRequire } from 'node:module';
import { resolve } from 'node:path';
import assert from 'node:assert/strict';
import { CheckpointPersistenceClient } from '../rustEngine/checkpointPersistenceClient.ts';
import { ExperimentalFreshRunSession, type ExperimentalFreshRunNativeBinding } from '../rustEngine/experimentalFreshRunSession.ts';
import type { ExperimentalStartupOptions, ExperimentalServerRuntime } from '../rustEngine/experimentalStartup.ts';
import type { ExperimentalRunningAuthorityNativeHandle } from '../rustEngine/backgroundRuntime.ts';
import type { ExperimentalEngineInit } from '../rustEngine/experimentalNativeBridge.ts';
import { computeNativeSourceIdentity } from '../rustEngine/nativeSourceIdentity.ts';
import type { U64Hex } from '../rustEngine/checkpointPersistenceProtocol.ts';

/** Isolated release addon produced by CI's Rust feature build. */
const ADDON_PATH = resolve(process.env['SLITHER_ENGINE_PANIC_TEST_ADDON'] ?? 'native/target/ci-panic-hooks.node');
/** CommonJS loader scoped to the test module. */
const require = createRequire(import.meta.url);
/** Coarse queues exercising the same transport contract with a small fixture. */
const INIT: ExperimentalEngineInit = {
  contractVersion: 1, maxInboundBatches: 16, maxInboundCommands: 16,
  maxInboundOwnedBytes: 2 * 1024 * 1024, maxBatchCommands: 1, maxBatchOwnedBytes: 1024 * 1024,
  maxOutputReliable: 32, maxOutputReliableOwnedBytes: 16 * 1024 * 1024,
  maxOutputDiscrete: 4, maxOutputDiscreteOwnedBytes: 1024 * 1024,
  maxOutputTotalOwnedBytes: 32 * 1024 * 1024, maxOutputEventOwnedBytes: 1024 * 1024,
  maxOutputFrameConnections: 4
};
/** Whether the next unstarted authority must fail its first parallel calculation. */
let injectPanic = true;
/** Only the calculation-panic cases need to retain copied frame chronology. */
let retainFrameChronology = true;
/** Real handle retained only until this fixture's transport is ready. */
let preparedRuntime: PanicRuntime | undefined;
/** Chronology of real native frame copies requested by the production output router. */
const copiedFrameSteps: string[] = [];

/** Additional method emitted exclusively by the test-hooks addon. */
export interface PanicRuntime extends ExperimentalRunningAuthorityNativeHandle {
  /** Arm only before coordinator start; never mutate the live world from JavaScript. */
  armCalculationPanicForTest(): void;
  /** Fail only the next export's end-block write, length check or stored-role validation. */
  armExportFailureForTest(mode: number): void;
}

/** Load without falsifying provenance, and independently check exact source and release class. */
export function loadPanicBinding(): ExperimentalFreshRunNativeBinding {
  if (!existsSync(ADDON_PATH)) throw new Error(
    `Missing isolated panic-test addon ${ADDON_PATH}. Build cargo --release --features engine-test-hooks and copy its native library to this .node path; keep native/*.node as the production build.`
  );
  const binding = require(ADDON_PATH) as ExperimentalFreshRunNativeBinding;
  assert.equal(binding.nativeAddonBuildClass(), 'test-hooks');
  assert.equal(binding.nativeAddonBuildProfile(), 'release');
  assert.equal(binding.nativeAddonSourceSha256(), computeNativeSourceIdentity(resolve('native')).sha256);
  assert.equal(binding.nativeAddonBuildTarget(), process.platform === 'win32'
    ? 'x86_64-pc-windows-msvc' : 'x86_64-unknown-linux-gnu');
  return binding;
}

/** Select the next disposable run's failure mode before starting its HTTP transport. */
export function configurePanicFixture(armed: boolean, retainFrames = true): void {
  injectPanic = armed;
  retainFrameChronology = retainFrames;
  preparedRuntime = undefined;
  copiedFrameSteps.length = 0;
}

/** Return only scalar chronology, preserving the real frame bytes and copy behavior. */
export function panicFixtureFrameSteps(): readonly string[] {
  return copiedFrameSteps;
}

/** Begin the actual coordinator only after the test attaches its real WebSocket listener. */
export function startPreparedPanicRuntime(): PanicRuntime {
  assert(preparedRuntime, 'transport must have prepared one real Rust runtime');
  preparedRuntime.start();
  return preparedRuntime;
}

/** Build a real durable owner for the isolated mock of only the startup dependency. */
export async function createPanicTestRuntime(options: ExperimentalStartupOptions): Promise<ExperimentalServerRuntime> {
  const binding = loadPanicBinding();
  await mkdir(options.managedDirectory, { recursive: true });
  const restoring = options.restoreLatest === true;
  const persistence = new CheckpointPersistenceClient({ databasePath: options.databasePath,
    managedRootPath: options.managedDirectory, existingOnly: restoring });
  let runtime: PanicRuntime | undefined;
  try {
    const selected = restoring ? await persistence.selectStartup() : null;
    const session = new ExperimentalFreshRunSession(binding, {
      runId: selected?.runId ?? randomUUID(), seed: options.seed ?? 42,
      memoryCeilingBytes: 4n * 1024n * 1024n * 1024n,
      calculationWorkers: options.calculationWorkers ?? 2,
      persistence, managedDirectory: options.managedDirectory
    });
    if (restoring) {
      assert(selected?.descriptor, 'restart must use its real committed checkpoint');
      await session.initializeFromCheckpoint(selected.descriptor);
    } else await session.initialize();
    const metadata = session.startupMetadata();
    const descriptor = selected?.descriptor;
    const runStart = descriptor ? {
      operationId: descriptor.operationId, transitionEpoch: descriptor.transitionEpoch,
      runId: descriptor.runId, checkpointId: descriptor.logicalRootSha256, descriptor
    } : await session.commitPendingRunStart(randomBytes(16).toString('hex'));
    await session.activateRunningAuthority();
    runtime = await session.createBackgroundRuntime(INIT, options.onWake) as PanicRuntime;
    assert.equal(typeof runtime.armCalculationPanicForTest, 'function');
    if (injectPanic) runtime.armCalculationPanicForTest();
    preparedRuntime = runtime;
    const actual = runtime;
    // Hold start only until the client listener exists. Every other call reaches
    // the real native receiver; a Proxy must bind its N-API methods to that receiver.
    const transportRuntime = new Proxy(actual, {
      get(target, key) {
        if (key === 'start') return (): void => {};
        if (key === 'copyLatestFrame') return (destination: Uint8Array, afterSequence: U64Hex) => {
          const copied = target.copyLatestFrame(destination, afterSequence);
          if (retainFrameChronology && copied.status === 'copied' && copied.display) {
            assert(copiedFrameSteps.length < 128, 'short fixture must retain bounded frame chronology');
            copiedFrameSteps.push(copied.display.completedStep);
          }
          return copied;
        };
        const value: unknown = Reflect.get(target, key, target);
        return typeof value === 'function' ? value.bind(target) : value;
      }
    });
    return {
      runtime: transportRuntime, nativeBuildIdentifier: binding.nativeAddonBuildIdentifier(),
      metadata, recovery: null, importBranch: null, legacyConversion: null,
      persistence, runStart, managedDirectory: options.managedDirectory,
      admitCheckpoint: async (): Promise<void> => {},
      async close(): Promise<void> {
        try { actual.requestStop(); await actual.join(); }
        finally { await persistence.close(); }
      }
    };
  } catch (error) {
    try { if (runtime) { runtime.requestStop(); await runtime.join(); } }
    finally { await persistence.close(); }
    throw error;
  }
}
