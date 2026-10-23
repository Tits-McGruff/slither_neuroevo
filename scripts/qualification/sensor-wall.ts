import { readObservationQuality, summarizeObservationQuality, type ObservationQualityNative } from './observationQuality.ts';
/** Measure real Rust scheduling with durable teacher storage and reused frame copies. */
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createHash, randomBytes } from 'node:crypto';
import { existsSync } from 'node:fs';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { availableParallelism, cpus, platform, release, tmpdir, totalmem } from 'node:os';
import { basename, dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { DEFAULT_CONFIG } from '../../server/config.ts';
import { validateRustCalculationWorkers } from '../../server/rustWorkers.ts';
import { loadPanicBinding } from '../../server/test/panicRuntime.ts';
import { computeNativeSourceIdentity } from '../../server/rustEngine/nativeSourceIdentity.ts';
import { parseRustStartupMetadata } from '../../server/rustEngine/startupMetadata.ts';
import type { ExperimentalFreshRunNativeHandle } from '../../server/rustEngine/experimentalFreshRunSession.ts';
import type { ExperimentalRunningAuthorityNativeHandle } from '../../server/rustEngine/backgroundRuntime.ts';
import type { ExperimentalEngineInit } from '../../server/rustEngine/experimentalNativeBridge.ts';
import { CheckpointPersistenceClient } from '../../server/rustEngine/checkpointPersistenceClient.ts';
import { parseManagedCheckpointDescriptor } from '../../server/rustEngine/checkpointPersistenceProtocol.ts';
import { BackgroundCommandAdmission } from '../../server/rustEngine/commandAdmission.ts';
import { BackgroundGenerationRouter } from '../../server/rustEngine/backgroundGeneration.ts';
import { DistillationSpoolClient } from '../../server/rustEngine/distillationSpoolClient.ts';
import { DistillationCapture } from '../../server/rustEngine/distillationCapture.ts';
import { admitDiskOperation, CHECKPOINT_DISK_ADMISSION_REQUEST } from '../../server/rustEngine/diskAdmission.ts';
import { summarizeSensorWall, type SensorWallRead } from './sensorWallSummary.ts';

/** Complete candidate request; all graph construction and execution stays native. */
interface QualifiedSession extends ExperimentalFreshRunNativeHandle {
  /** Select the same modest P0/P1 graph/settings builder as the controlled-step host. */
  initializeBaselineV2ForTest(request: {profile: string; bins: number; mode: number; cadence: number;
    rayMinimum: number; rayCap: number; rayBatch: number; workload: string}): Promise<unknown>;
}
/** Production-sized bounded queues without any external controller. */
const INIT: ExperimentalEngineInit = {contractVersion: 1, maxInboundBatches: 16, maxInboundCommands: 16,
  maxInboundOwnedBytes: 2 * 1024 ** 2, maxBatchCommands: 1, maxBatchOwnedBytes: 1024 ** 2,
  maxOutputReliable: 32, maxOutputReliableOwnedBytes: 16 * 1024 ** 2, maxOutputDiscrete: 4,
  maxOutputDiscreteOwnedBytes: 1024 ** 2, maxOutputTotalOwnedBytes: 32 * 1024 ** 2,
  maxOutputEventOwnedBytes: 1024 ** 2, maxOutputFrameConnections: 4};
/** Bounded timer yield; it never advances the native simulation clock. */
const delay = async (milliseconds: number): Promise<void> => new Promise(done => setTimeout(done, milliseconds));
/** Identity of exact host sources retained alongside the native build identity. */
async function digest(path: string): Promise<string> {
  return createHash('sha256').update(await readFile(path)).digest('hex');
}
/** Bracket a coherent native health cache without treating a partial histogram as evidence. */
async function observe(native: ExperimentalRunningAuthorityNativeHandle): Promise<SensorWallRead> {
  for (let attempt = 0; attempt < 10; attempt++) {
    const beforeMs = performance.now(); const health = native.health(true); const afterMs = performance.now();
    assert.equal(health.lifecycle, 'running', 'qualification authority stopped');
    assert.equal(health.faultCode, undefined, 'qualification authority faulted');
    if (health.stepTimingHistogramConsistent) {
      const {worldEpoch, generation, completedStep, calculationWorkers, schedulerCompletedSteps,
        schedulerDroppedWallMicros, schedulerOverloaded, lifecycle, stepTimingSamples,
        stepTimingHistogramConsistent, stepTimingBucketUpperMicros, stepTimingBucketCounts} = health;
      assert(stepTimingBucketUpperMicros && stepTimingBucketCounts, 'native histogram layout missing');
      return {beforeMs, afterMs, health: {worldEpoch, generation, completedStep, calculationWorkers,
        schedulerCompletedSteps, schedulerDroppedWallMicros, schedulerOverloaded, lifecycle,
        stepTimingSamples, stepTimingHistogramConsistent, stepTimingBucketUpperMicros, stepTimingBucketCounts}};
    }
    await delay(5);
  }
  throw new Error('qualification histogram remained incoherent');
}
/** Existing coarse timing counters, sampled beside the exact acceptance window.
 * These independent atomics are diagnostic phase attribution, not a simultaneous
 * completed-step snapshot or a substitute for the consistent endpoint histogram.
 */
function costs(native: ExperimentalRunningAuthorityNativeHandle) {
  const beforeMs = performance.now(); const health = native.health(); const afterMs = performance.now();
  const counters: Record<string, string> = {};
  for (const key of ['stepTimingSamples', 'stepTimingTotalMicros', 'slowStepSamples', 'slowStepControlMicros',
    'slowStepControlIndexMicros', 'slowStepControlNeuralMicros', 'slowStepControlSensingMicros',
    'slowStepControlInferenceMicros', 'slowStepWorldMicros', 'slowStepOtherMicros'] as const) {
    const value = health[key]; assert(/^[0-9a-f]{16}$/u.test(value) && value !== 'f'.repeat(16), 'invalid phase counter');
    counters[key] = value;
  }
  return {beforeMs, afterMs, worldEpoch: health.worldEpoch, generation: health.generation,
    completedStep: health.completedStep, counters};
}
/** Clean only this process-created temporary tree after native and worker shutdown. */
async function removeRoot(root: string): Promise<void> {
  assert.equal(dirname(resolve(root)), resolve(tmpdir()));
  assert(basename(root).startsWith('slither-sensor-wall-'));
  await rm(root, {recursive: true, force: true});
}
/** Run one bounded real-clock window; failed measured gates remain visible report data. */
async function run(): Promise<void> {
  const args = new Map<string, string>();
  for (let index = 2; index < process.argv.length; index += 2) {
    const key = process.argv[index]!; const value = process.argv[index + 1];
    assert(['--scenario', '--workers', '--seconds', '--warmup-seconds', '--report', '--ray-cap', '--ray-batch'].includes(key) &&
      value !== undefined && !args.has(key), 'invalid or duplicate qualification argument');
    args.set(key, value);
  }
  const scenario = args.get('--scenario'); assert(scenario === 'P0' || scenario === 'P1', '--scenario P0|P1 required');
  const outputArgument = args.get('--report'); assert(outputArgument, '--report required');
  const output = resolve(outputArgument); assert(!existsSync(output), 'qualification report already exists');
  const workers = validateRustCalculationWorkers(Number(args.get('--workers') ?? Math.min(DEFAULT_CONFIG.rustCalculationWorkers, availableParallelism())));
  const seconds = Number(args.get('--seconds') ?? 60); const warmupSeconds = Number(args.get('--warmup-seconds') ?? 10);
  assert(Number.isSafeInteger(seconds) && seconds >= 60 && seconds <= 600, 'seconds must be 60..600');
  assert(Number.isSafeInteger(warmupSeconds) && warmupSeconds >= 5 && warmupSeconds <= 60, 'warmup seconds must be 5..60');
  const rayCap = Number(args.get('--ray-cap') ?? 256); const rayBatch = Number(args.get('--ray-batch') ?? 65536);
  assert(Number.isSafeInteger(rayCap) && rayCap >= 1 && rayCap <= 1_000_000, 'ray cap must be 1..1000000');
  assert(Number.isSafeInteger(rayBatch) && rayBatch >= 512 && rayBatch <= 10_000_000, 'ray batch must be 512..10000000');
  const hostPath = fileURLToPath(import.meta.url); const summaryPath = fileURLToPath(new URL('./sensorWallSummary.ts', import.meta.url));
  const hostSha256 = await digest(hostPath); const summarySha256 = await digest(summaryPath);
  const observationHelperPath = fileURLToPath(new URL('./observationQuality.ts', import.meta.url));
  const observationContractPath = fileURLToPath(new URL('../../src/protocol/observationQuality.ts', import.meta.url));
  const observationHelperSha256 = await digest(observationHelperPath);
  const observationContractSha256 = await digest(observationContractPath);
  const nativeSourceSha256 = computeNativeSourceIdentity(resolve('native')).sha256;
  const binding = loadPanicBinding();
  const Constructor = binding.ExperimentalStage6aFreshRunSession as new (run: string, seed: string, memory: string, workers: number) => QualifiedSession;
  const runId = 'sensor-wall-' + randomBytes(8).toString('hex');
  const session = new Constructor(runId, '0000002a', '0000000100000000', workers);
  const root = await mkdtemp(join(tmpdir(), 'slither-sensor-wall-')); const managed = join(root, 'managed');
  let runtime: ExperimentalRunningAuthorityNativeHandle | undefined; let persistence: CheckpointPersistenceClient | undefined;
  let spool: DistillationSpoolClient | undefined; let capture: DistillationCapture | undefined;
  let timer: NodeJS.Timeout | undefined; let activePump: Promise<void> | undefined; let pumpError: unknown;
  let report: unknown;
  try {
    await mkdir(managed); await admitDiskOperation(managed, CHECKPOINT_DISK_ADMISSION_REQUEST);
    const request = {profile: 'motion', bins: 16, mode: 1, cadence: 1 / 60, rayMinimum: 1, rayCap, rayBatch, workload: scenario};
    await session.initializeBaselineV2ForTest(request);
    const metadata = parseRustStartupMetadata(session.startupMetadata());
    const settings = new Map(metadata.settings.map(entry => [entry.path, entry.value]));
    for (const [path, value] of [['snakeCount', scenario === 'P0' ? 55 : 300], ['baselineBots.count', 10],
      ['evolvedRespawn.enabled', true], ['generationSeconds', 604800], ['simSpeed', 1],
      ['brain.controlDt', 1 / 60], ['baselineBots.respawnDelay', 20], ['observer.earlyEndMode', 2], ['world.pelletOverflow', 1]] as const) {
      assert.equal(settings.get(path), value, 'qualification setting ' + path);
    }
    assert.equal(metadata.parameterCount, 25042); assert.equal(metadata.sensorVersion, 4);
    assert.equal(metadata.mathBackend, 'rust-sse2-v1'); assert.equal(metadata.fixedStepSeconds, 1 / 60); assert(metadata.sensorContract);
    assert(metadata.maximumFrameBytes <= 64 * 1024 ** 2, 'bounded qualification frame admission');
    persistence = new CheckpointPersistenceClient({databasePath: join(root, 'game.sqlite'), managedRootPath: managed});
    const checkpoint = parseManagedCheckpointDescriptor(await session.publishRunStartCheckpoint({managedDirectory: managed, operationId: randomBytes(16).toString('hex')}));
    await persistence.commit(checkpoint); session.acknowledgeRunStartPersistence(checkpoint);
    await session.activateRunningAuthority(); runtime = await session.createBackgroundRuntime(INIT, () => {}) as ExperimentalRunningAuthorityNativeHandle;
    const native = runtime; const admission = new BackgroundCommandAdmission(native);
    spool = new DistillationSpoolClient(join(root, 'diagnostics')); const storage = spool;
    capture = new DistillationCapture(() => native, admission, storage, () => ({runId, seed: 42})); const diagnostics = capture;
    const generation = new BackgroundGenerationRouter({native, admission, persistence, managedDirectory: managed, maxAssignments: 1,
      send: () => {throw new Error('qualification must not assign controllers');},
      admitCheckpoint: async () => {await admitDiskOperation(managed, CHECKPOINT_DISK_ADMISSION_REQUEST);}});
    timer = setInterval(() => {
      if (activePump || pumpError) return;
      activePump = (async () => {
        generation.flush();
        for (const event of native.drainOutputs(32, INIT.maxOutputEventOwnedBytes).events) {
          if (event.kind === 'fault') throw new Error(event.faultDetail ?? 'native fault');
          if (event.kind.startsWith('controller') || event.kind === 'externalObservations') throw new Error('unexpected controller traffic');
          diagnostics.event(event); await generation.handle(event);
        }
      })().catch(error => {pumpError = error;}).finally(() => {activePump = undefined;});
    }, 5);
    native.start();
    const captureLimits = {expectedRunId: runId, expectedConfigRevision: metadata.configRevision,
      baselineSlots: Array.from({length: 10}, (_, slot) => slot), pendingBytes: 16 * 1024 ** 2, queueChunks: 1024, queueBytes: 16 * 1024 ** 2};
    const dataset = await capture.start(captureLimits); const storagePolicy = await storage.storagePolicy();
    const frame = new Uint8Array(metadata.maximumFrameBytes); let frameSequence = '0'.repeat(16);
    let frameCopies = 0; let frameBytes = 0; let frameMaximumBytes = 0; let nextFrame = 0;
    let peakHostRssBytes = process.memoryUsage.rss();
    /** Consume only latest Rust-packed bytes in one reusable buffer, at most 30 Hz. */
    const consume = (): void => {
      if (pumpError) throw pumpError;
      peakHostRssBytes = Math.max(peakHostRssBytes, process.memoryUsage.rss());
      const now = performance.now(); if (now < nextFrame) return; nextFrame = now + 1000 / 30;
      const copied = native.copyLatestFrame(frame, frameSequence);
      assert.notEqual(copied.status, 'tooSmall', 'native frame exceeded declared maximum');
      if (copied.status === 'copied') {
        assert(copied.display.sequence > frameSequence && copied.display.frameByteLength <= frame.byteLength);
        frameSequence = copied.display.sequence; frameCopies++; frameBytes += copied.display.frameByteLength;
        frameMaximumBytes = Math.max(frameMaximumBytes, copied.display.frameByteLength);
      }
    };
    console.info('[sensor-wall] warmup', scenario, 'workers=' + workers);
    const warmupEnd = performance.now() + warmupSeconds * 1000;
    while (performance.now() < warmupEnd) {consume(); await delay(20);}
    const initialDataset = await storage.dataset(dataset.contract.captureEpoch);
    const initial = await observe(native); const reads = [initial]; const initialCosts = costs(native);
    const observationBefore = readObservationQuality(native as typeof native & ObservationQualityNative);
    const initialCopies = frameCopies; const initialBytes = frameBytes; const cpuBefore = process.cpuUsage();
    const systemCpuBefore = cpus().map(cpu => cpu.times);
    let nextRead = initial.afterMs + 200;
    console.info('[sensor-wall] measuring', seconds, 'seconds');
    while (reads.at(-1)!.beforeMs - initial.afterMs < seconds * 1000) {
      consume();
      if (performance.now() >= nextRead) {
        assert(reads.length < 4096, 'health sample admission'); reads.push(await observe(native)); nextRead = performance.now() + 200;
      }
      assert(performance.now() - initial.afterMs < (seconds + 30) * 1000, 'measurement deadline');
      await delay(20);
    }
    const cpu = process.cpuUsage(cpuBefore); const systemCpuAfter = cpus().map(entry => entry.times); const summary = summarizeSensorWall(reads, seconds);
    const finalCosts = costs(native);
    const observationAfter = readObservationQuality(native as typeof native & ObservationQualityNative);
    assert.equal(finalCosts.worldEpoch, initialCosts.worldEpoch); assert.equal(finalCosts.generation, initialCosts.generation);
    const queueDiagnostics = native.queueDiagnostics(); const display = native.latestDisplay();
    let finalDataset = await capture.stop(dataset.contract.captureEpoch);
    const settleDeadline = performance.now() + 10_000;
    while (finalDataset.state !== 'stopped' || finalDataset.resolvedThrough !== finalDataset.productionWatermark) {
      assert(performance.now() < settleDeadline, 'final capture settlement deadline');
      await capture.poll(); await delay(10); finalDataset = await storage.dataset(dataset.contract.captureEpoch);
    }
    let cursor = '0'.repeat(16); let retainedChunks = 0; let retainedBytes = 0n;
    const retainedSamplesByBaselineSlot = Array<number>(10).fill(0);
    for (let pageNumber = 0; ; pageNumber++) {
      assert(pageNumber < 256, 'bounded descriptor pagination');
      const page = await storage.list(dataset.contract.captureEpoch, cursor, 256);
      assert.equal(page.dataset.productionWatermark, finalDataset.productionWatermark);
      for (const chunk of page.chunks) {
        assert(chunk.sequence > cursor && chunk.baselineSlot < 10, 'stored teacher chronology or slot');
        cursor = chunk.sequence; retainedChunks++; retainedBytes += BigInt(chunk.byteCount);
        retainedSamplesByBaselineSlot[chunk.baselineSlot]! += chunk.count;
      }
      assert.equal(page.nextCursor, cursor);
      if (!page.hasMore) break;
      assert(page.chunks.length > 0, 'descriptor pagination must make progress');
    }
    assert.equal(retainedChunks, finalDataset.retainedChunks);
    assert.equal(retainedBytes, BigInt('0x' + finalDataset.retainedBytes));
    assert(frameCopies > initialCopies && finalDataset.retainedChunks > 0, 'frame and teacher consumers must do actual work');
    if (pumpError) throw pumpError;
    assert.equal(finalDataset.error, null, 'capture diagnostic failure');
    assert.equal(hostSha256, await digest(hostPath)); assert.equal(summarySha256, await digest(summaryPath));
    assert.equal(observationHelperSha256, await digest(observationHelperPath));
    assert.equal(observationContractSha256, await digest(observationContractPath));
    assert.equal(observationBefore.snapshot.runId, runId);
    assert.equal(observationAfter.snapshot.runId, runId);
    assert.equal(nativeSourceSha256, computeNativeSourceIdentity(resolve('native')).sha256, 'native sources changed during measurement');
    report = {schemaVersion: 1, recordedAt: new Date().toISOString(), scenario, workers, requestedSeconds: seconds, warmupSeconds,
      scope: 'Real Rust background clock, durable run-start SQLite checkpoint, ten v2 stochastic teachers, diagnostic spool and latest-frame copying. Seeded random modest population with unlimited evolved respawn and normal teacher death gaps. Excludes browser/WebSocket delivery, trainer, shadows, full storage capacity, long soak and production-default selection.',
      source: {revision: execFileSync('git', ['rev-parse', 'HEAD'], {encoding: 'utf8'}).trim(), hostSha256, summarySha256, observationHelperSha256, observationContractSha256,
        nativeSourceSha256, nativeBuildIdentifier: binding.nativeAddonBuildIdentifier(), nativeTarget: binding.nativeAddonBuildTarget(), nativeClass: binding.nativeAddonBuildClass()},
      environment: {platform: platform(), release: release(), cpu: cpus()[0]?.model, availableCpus: availableParallelism(), totalMemoryBytes: totalmem()},
      workload: {request, metadata, captureLimits, storagePolicy, queues: INIT, runStartCheckpoint: checkpoint},
      observationQuality: {before: observationBefore, after: observationAfter,
        summary: summarizeObservationQuality(observationBefore, observationAfter)},
      summary, nativePhaseCosts: {initial: initialCosts, final: finalCosts,
        caveat: 'Coarse atomic timing counters sampled beside the window; nested control/neural/sensing/inference totals overlap. Slow-step attribution covers only steps above 16.667 ms.'}, diagnosticDelivery: {nativeDroppedChunks: finalDataset.nativeDroppedChunks,
        nativeDroppedSamples: finalDataset.nativeDroppedSamples, spoolDroppedChunks: finalDataset.spoolDroppedChunks,
        captureHasNoDrops: [finalDataset.nativeDroppedChunks, finalDataset.nativeDroppedSamples, finalDataset.spoolDroppedChunks].every(value => value === '0'.repeat(16))},
      reads: reads.map((read, index) => {
        const health = {...read.health};
        // Endpoint histograms establish the exact window; intermediate reads retain
        // scalar chronology and the observed coherence flag without repeating buckets.
        if (index > 0 && index < reads.length - 1) {delete health.stepTimingBucketUpperMicros; delete health.stepTimingBucketCounts;}
        return {...read, health};
      }), frames: {copies: frameCopies - initialCopies, bytesCopied: frameBytes - initialBytes,
        reusableBufferBytes: frame.byteLength, maximumObservedBytesIncludingWarmup: frameMaximumBytes, lastDisplay: display},
      host: {peakRssBytes: peakHostRssBytes, cpuUserMicros: cpu.user, cpuSystemMicros: cpu.system, systemCpuBefore, systemCpuAfter},
      capture: {initial: initialDataset, final: finalDataset, retainedSamplesByBaselineSlot,
        retainedSamples: retainedSamplesByBaselineSlot.reduce((sum, count) => sum + count, 0), caveat: 'Dataset totals include startup, warmup and final seal; they are not exact measurement-window row counts.'}, queueDiagnostics};
  } finally {
    clearInterval(timer); await activePump;
    try {if (runtime) {runtime.requestStop(); await runtime.join();}}
    finally {
      try {if (capture) await capture.close(); else await spool?.close();}
      finally {try {await persistence?.close();} finally {await removeRoot(root);}}
    }
  }
  await mkdir(dirname(output), {recursive: true}); await writeFile(output, JSON.stringify(report, null, 2) + '\n', {flag: 'wx'});
  console.info('[sensor-wall] measured report', output);
}

await run();
