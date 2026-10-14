import { randomBytes } from 'node:crypto';
import { existsSync, mkdirSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { expect, it } from 'vitest';
import type { RustBackgroundEvent } from '../../src/protocol/rustBackground.ts';
import { loadPanicBinding } from '../test/panicRuntime.ts';
import type { ExperimentalFreshRunNativeHandle } from './experimentalFreshRunSession.ts';
import type { ExperimentalRunningAuthorityNativeHandle } from './backgroundRuntime.ts';
import { CheckpointPersistenceClient } from './checkpointPersistenceClient.ts';
import { parseManagedCheckpointDescriptor } from './checkpointPersistenceProtocol.ts';
import type { ExperimentalEngineInit } from './experimentalNativeBridge.ts';

/** Explicit native-only v4 admission, absent from the production constructor. */
interface QualifiedSession extends ExperimentalFreshRunNativeHandle {
  /** Build the native qualification fixture's Dense Basic B8 graph. */
  initializeBaselineV2ForTest(request: {profile: string; bins: number; mode: number;
    cadence: number; rayMinimum: number; rayCap: number; rayBatch: number}): Promise<unknown>;
}

/** Production-sized bounded queues; test code never executes a neural graph. */
const INIT: ExperimentalEngineInit = {contractVersion: 1, maxInboundBatches: 16, maxInboundCommands: 16,
  maxInboundOwnedBytes: 2 * 1024 * 1024, maxBatchCommands: 1, maxBatchOwnedBytes: 1024 * 1024,
  maxOutputReliable: 32, maxOutputReliableOwnedBytes: 16 * 1024 * 1024, maxOutputDiscrete: 4,
  maxOutputDiscreteOwnedBytes: 1024 * 1024, maxOutputTotalOwnedBytes: 32 * 1024 * 1024,
  maxOutputEventOwnedBytes: 1024 * 1024, maxOutputFrameConnections: 4};

/** Remove only this test's verified direct child of the temporary directory. */
function cleanup(root: string): void {
  if (dirname(resolve(root)) !== resolve(tmpdir()) || !root.includes('slither-library-spawn-')) throw new Error('unsafe fixture cleanup');
  rmSync(root, {recursive: true, force: true});
}

for (const workers of [1, 3]) {
  it(`deploys a catalog lease through the native command and survives deletion (workers=${workers})`, async () => {
    const root = mkdtempSync(join(tmpdir(), 'slither-library-spawn-'));
    const managed = join(root, 'managed'); mkdirSync(managed);
    const persistence = new CheckpointPersistenceClient({databasePath: join(root, 'game.sqlite'), managedRootPath: managed});
    const Constructor = loadPanicBinding().ExperimentalStage6aFreshRunSession as new (
      run: string, seed: string, memory: string, workers: number) => QualifiedSession;
    const runId = 'saved-spawn-' + randomBytes(8).toString('hex');
    const session = new Constructor(runId, '0000002a', '0000000100000000', workers);
    let runtime: ExperimentalRunningAuthorityNativeHandle | undefined;
    try {
      await session.initializeBaselineV2ForTest({profile: 'basic', bins: 8, mode: 0, cadence: 0.06,
        rayMinimum: 1, rayCap: 256, rayBatch: 65536});
      const checkpoint = parseManagedCheckpointDescriptor(await session.publishRunStartCheckpoint({
        managedDirectory: managed, operationId: randomBytes(16).toString('hex')}));
      await persistence.commit(checkpoint); session.acknowledgeRunStartPersistence(checkpoint);
      await session.activateRunningAuthority();
      runtime = await session.createBackgroundRuntime(INIT, () => {}) as ExperimentalRunningAuthorityNativeHandle;
      const native = runtime;
      const saved = await persistence.savedBrainCatalog({action: 'save', path: resolve('native/fixtures/portable-dense-basic8-v1.slither-brain'), name: 'explicit native deployment'});
      if (saved.action !== 'save') throw new Error('save response mismatch');
      const leaseId = randomBytes(16).toString('hex');
      await persistence.savedBrainCatalog({action: 'acquire', brainId: saved.record.brainId, leaseId});
      native.start();
      /** Drain reliable receipts while the actual native fixed-step authority runs. */
      const receipt = async (sequence: string): Promise<RustBackgroundEvent> => {
        const deadline = performance.now() + 5000;
        while (performance.now() < deadline) {
          for (const event of native.drainOutputs(32, INIT.maxOutputEventOwnedBytes).events) {
            if (event.kind === 'fault') throw new Error(event.faultDetail);
            if (event.commandSequence === sequence) return event;
          }
          await new Promise<void>(done => setImmediate(done));
        }
        throw new Error('native saved deployment receipt deadline');
      };
      native.submitLiveSettings('0000000000000001', [{path: 'brain.controlDt', value: 0.06}]);
      expect((await receipt('0000000000000001')).kind).toBe('liveSettingsApplied');
      const request = {brainId: saved.record.brainId, expectedRunId: runId,
        expectedConfigRevision: '0000000000000002', expectedWorldEpoch: native.health().worldEpoch,
        weights: saved.record.weights};
      expect(() => native.submitSavedBrainSpawn('0000000000000002', managed, {...request, expectedWorldEpoch: '0'.repeat(16)})).toThrow();
      native.submitSavedBrainSpawn('0000000000000002', managed, {...request, expectedRunId: 'stale-run'});
      expect((await receipt('0000000000000002')).kind).toBe('commandRejected');
      native.submitSavedBrainSpawn('0000000000000003', managed, {...request, weights: {
        ...request.weights, weightCount: '0000000040000000', decodedByteCount: '0000000100000000'}});
      expect(await receipt('0000000000000003')).toMatchObject({kind: 'commandRejected',
        rejectionDetail: expect.stringContaining('shared weight descriptor')});
      native.submitSavedBrainSpawn('0000000000000004', managed, request);
      const spawned = await receipt('0000000000000004');
      expect(spawned).toMatchObject({kind: 'savedBrainSpawned', savedBrainSpawn: {
        snakeId: expect.any(Number), effectiveStep: expect.stringMatching(/^[0-9a-f]{16}$/u)}});
      expect(spawned.hallOfFameResurrection).toBeUndefined();
      const step = BigInt('0x' + spawned.savedBrainSpawn!.effectiveStep);
      await persistence.savedBrainCatalog({action: 'delete', brainId: saved.record.brainId});
      await persistence.savedBrainCatalog({action: 'release', leaseId});
      expect(existsSync(join(managed, saved.record.artifactRelativeFilename))).toBe(false);
      const deadline = performance.now() + 5000;
      while (BigInt('0x' + native.health().completedStep) <= step && performance.now() < deadline) {
        for (const event of native.drainOutputs(32, INIT.maxOutputEventOwnedBytes).events) {
          if (event.kind === 'fault') throw new Error(event.faultDetail);
        }
        await new Promise<void>(done => setImmediate(done));
      }
      expect(BigInt('0x' + native.health().completedStep)).toBeGreaterThan(step);
    } finally {
      if (runtime) { runtime.requestStop(); await runtime.join(); }
      await persistence.close();
      cleanup(root);
    }
  }, 15000);
}
