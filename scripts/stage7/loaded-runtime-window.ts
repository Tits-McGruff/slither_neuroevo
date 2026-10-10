/** Sample an already running production workload while an independent real trainer supplies load. */
import { createHash } from 'node:crypto';
import { readFile, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import WebSocket from 'ws';
import type { RustBackgroundHealth, RustQueueDiagnostics } from '../../src/protocol/rustBackground.ts';
import type { ExperimentalRuntimeTelemetrySnapshot } from '../../server/rustEngine/runtimeTelemetry.ts';
import type { WsOutboundDiagnostics } from '../../server/wsHub.ts';
import { summarizeRssSoak } from './rss-soak-summary.ts';
import { summarizeQueueSoak } from './queue-soak-summary.ts';
import { counterWindow } from './archive-overlap-summary.ts';
import { generationInterval, generationPublication } from './generation-window-summary.ts';
import type { GenerationPublication } from './generation-window-summary.ts';
import { summarizeStepWindow } from './step-window-summary.ts';

/** Mandatory real-time workloads in the approved migration plan. */
type Scenario = 'P0' | 'P1' | 'P2';

/** Production health identity and scalar measurements, without world arrays. */
interface Health extends RustBackgroundHealth {
  /** Whether the interface has retained a terminal fault. */
  ok: boolean;
  /** Current durable lineage. */
  runId: string;
  /** Current admitted configuration. */
  configHash: string;
  /** Source-derived native addon identity. */
  nativeBuildIdentifier: string;
  /** Process-lifetime distributions and controller receipts. */
  telemetry: ExperimentalRuntimeTelemetrySnapshot;
  /** Actual native occupancy, limits and peaks retained after output drains. */
  nativeQueues: RustQueueDiagnostics;
  /** Bounded WebSocket output occupancy, independent from replaceable frames. */
  outbound: WsOutboundDiagnostics;
  /** Cached scalar durable-file and SQLite diagnostics. */
  storage: { sqlite: { databaseBytes: string; walBytes: string; shmBytes: string };
    managed: { temporaryBytes: string; freeBytes: string; operatingReserveBytes: string } };
  /** Current automatic pruning envelope. */
  retention: { automaticByteCap: string; automaticStoredByteCount: string };
}

/** Decode an exact native counter without silently truncating large values. */
function counter(value: string): bigint {
  if (!/^[0-9a-f]{16}$/u.test(value)) throw new Error(`invalid native counter: ${value}`);
  return BigInt(`0x${value}`);
}

/** Read a bounded response and keep observation failures distinct from authority faults. */
async function readHealth(url: URL): Promise<{ health: Health; latencyMs: number; beforeMs: number; afterMs: number }> {
  const beforeMs = performance.now();
  const signal = AbortSignal.timeout(15_000);
  for (let attempt = 0; attempt < 5; attempt++) {
    const response = await fetch(new URL('/api/health', url), { signal });
    const health = await response.json() as Health;
    const afterMs = performance.now();
    if (!response.ok || health.ok !== true) throw new Error(`authority fault: ${JSON.stringify(health)}`);
    if (health.stepTimingHistogramConsistent === true) {
      return { health, latencyMs: afterMs - beforeMs, beforeMs, afterMs };
    }
    if (health.stepTimingHistogramConsistent !== false) throw new Error('native step histogram is missing');
    await new Promise<void>(done => setTimeout(done, 1));
  }
  throw new Error('native step histogram remained inconsistent after five bounded reads');
}

/** Read actual workload settings without joining a display stream or changing authority. */
async function welcome(url: URL): Promise<Record<string, unknown>> {
  const socketUrl = new URL(url);
  socketUrl.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
  const socket = new WebSocket(socketUrl);
  return await new Promise((resolveWelcome, reject) => {
    const timer = setTimeout(() => finish(new Error('welcome timeout')), 15_000);
    /** Settle the read-only exchange and release the socket. */
    function finish(error?: Error, message?: Record<string, unknown>): void {
      clearTimeout(timer);
      socket.terminate();
      if (error) reject(error);
      else resolveWelcome(message!);
    }
    socket.on('open', () => socket.send(JSON.stringify({ type: 'hello', version: 2, clientType: 'ui' })));
    socket.on('error', error => finish(error));
    socket.on('message', (data, binary) => {
      if (binary) return;
      try {
        const message = JSON.parse(data.toString()) as Record<string, unknown>;
        if (message['type'] === 'welcome') finish(undefined, message);
        else if (message['type'] === 'error') finish(new Error(JSON.stringify(message)));
      } catch (error) { finish(error instanceof Error ? error : new Error(String(error))); }
    });
  });
}

/** Require the named workload and real trainer activity before starting the clock. */
function assertWorkload(message: Record<string, unknown>, health: Health, scenario: Scenario, workers: number): void {
  const settings = message['settings'] as { core: { snakeCount: number; simSpeed: number };
    updates: Array<{ path: string; value: unknown }> };
  const inference = message['inferenceMode'] as { activeBackend: string; activeWorkerCount: number; parameterCount: number };
  const sensor = message['sensorSpec'] as { sensorCount: number };
  const value = (path: string): unknown => settings.updates.find(item => item.path === path)?.value;
  if (settings.core.snakeCount !== (scenario === 'P1' ? 300 : 55) || settings.core.simSpeed !== 1 ||
      value('generationSeconds') !== 60 || value('baselineBots.count') !== 10 ||
      value('pelletCountTarget') !== 3500 || value('sense.bubbleBins') !== (scenario === 'P2' ? 32 : 16) ||
      sensor.sensorCount !== (scenario === 'P2' ? 147 : 83) || inference.activeBackend !== 'native' ||
      !Number.isSafeInteger(inference.parameterCount) ||
      (scenario === 'P2' ? inference.parameterCount < 400_000 : inference.parameterCount !== 13_458) ||
      inference.activeWorkerCount !== workers || health.calculationWorkers !== workers ||
      counter(health.generation) < 2n || health.telemetry.controllerActivity.trainer.freshAssignments < 2 ||
      health.telemetry.controllerActivity.trainer.appliedActions === 0) {
    throw new Error(`${scenario} configuration, evolved population, requested workers, or two active trainer actors missing`);
  }
}

/** Compute a conservative empirical percentile for the sampler's request latencies. */
function percentile(values: readonly number[], fraction: number): number {
  const sorted = [...values].sort((left, right) => left - right);
  return sorted[Math.ceil(sorted.length * fraction) - 1]!;
}

/** Identify the exact sampler and summary bytes independently of the server revision. */
async function measurementSourceDigests(): Promise<Record<string, string>> {
  const paths = ['./loaded-runtime-window.ts', './archive-overlap-summary.ts', './generation-window-summary.ts',
    './rss-soak-summary.ts', './queue-soak-summary.ts', './step-window-summary.ts'];
  return Object.fromEntries(await Promise.all(paths.map(async path =>
    [path, createHash('sha256').update(await readFile(new URL(path, import.meta.url))).digest('hex')])));
}

/** Measure one uninterrupted authority and preserve a compact report even if sampling fails. */
async function run(): Promise<void> {
  const [base, output, secondsText, sourceRevision, scenarioText = 'P1', workersText = '6'] = process.argv.slice(2);
  const seconds = Number(secondsText);
  const workers = Number(workersText);
  if (!base || !output || !sourceRevision || !/^[0-9a-f]{40}$/u.test(sourceRevision) ||
      !Number.isInteger(seconds) || seconds < 600 || seconds > 7200 ||
      !['P0', 'P1', 'P2'].includes(scenarioText) || !Number.isSafeInteger(workers) || workers < 1) {
    throw new Error('usage: loaded-runtime-window.ts URL NEW_REPORT_PATH SECONDS>=600 SOURCE_COMMIT [P0|P1|P2] [WORKERS]');
  }
  // This sampler may target another host; assertWorkload checks the server's admitted count.
  const scenario = scenarioText as Scenario;
  const measurementSources = await measurementSourceDigests();
  const url = new URL(base);
  const metadata = await welcome(url);
  const initialRead = await readHealth(url);
  const initial = initialRead.health;
  if (!initial.nativeBuildIdentifier.includes(`+${sourceRevision.slice(0, 12)}.`)) {
    throw new Error('running addon does not identify the requested source revision');
  }
  assertWorkload(metadata, initial, scenario, workers);
  const started = initialRead.afterMs;
  const startedAtUtc = new Date().toISOString();
  const transitions: Array<GenerationPublication & { wallSeconds: number }> = [];
  const observationFailures: Array<{ wallSeconds: number; error: string }> = [];
  const latencies: number[] = [];
  const resourceSamples: Array<{ wallSeconds: number; rssBytes: number; heapUsedBytes: number;
    beforeMs: number; afterMs: number;
    externalBytes: number; trainerAppliedActions: number;
    generation: string; nativeQueues: RustQueueDiagnostics; outbound: Health['outbound']; storage: Health['storage'];
    automaticStoredBytes: string; automaticByteCap: string }> = [];
  /** Retain the resource diagnostics and clock brackets from one health observation. */
  const recordResources = (read: Awaited<ReturnType<typeof readHealth>>, wallSeconds: number): void => {
    const health = read.health;
    resourceSamples.push({ wallSeconds, rssBytes: health.telemetry.process.rssBytes,
      beforeMs: read.beforeMs, afterMs: read.afterMs,
      heapUsedBytes: health.telemetry.process.heapUsedBytes, externalBytes: health.telemetry.process.externalBytes,
      trainerAppliedActions: health.telemetry.controllerActivity.trainer.appliedActions,
      generation: health.generation, nativeQueues: health.nativeQueues, outbound: health.outbound, storage: health.storage,
      automaticStoredBytes: counter(health.retention.automaticStoredByteCount).toString(),
      automaticByteCap: counter(health.retention.automaticByteCap).toString() });
  };
  recordResources(initialRead, 0);
  let finalRead = initialRead;
  let final = initial;
  let previous = initial;
  let previousRead = initialRead;
  let overloaded = initial.schedulerOverloaded;
  let failure: string | undefined;
  let nextProgress = 30;
  try {
    do {
      await new Promise<void>(done => setTimeout(done, 500));
      let sample: Awaited<ReturnType<typeof readHealth>>;
      try { sample = await readHealth(url); }
      catch (error) {
        if (String(error).includes('authority fault:')) throw error;
        observationFailures.push({ wallSeconds: (performance.now() - started) / 1000, error: String(error) });
        if (observationFailures.length >= 3) throw new Error('three health observation failures');
        continue;
      }
      finalRead = sample;
      final = sample.health;
      latencies.push(sample.latencyMs);
      if (final.runId !== initial.runId ||
          counter(final.worldEpoch) - counter(previous.worldEpoch) !== counter(final.generation) - counter(previous.generation) ||
          final.configHash !== initial.configHash || final.nativeBuildIdentifier !== initial.nativeBuildIdentifier ||
          final.calculationWorkers !== workers || counter(final.completedStep) < counter(previous.completedStep) ||
          counter(final.schedulerDroppedWallMicros) < counter(previous.schedulerDroppedWallMicros) ||
          counter(final.generation) < counter(previous.generation) ||
          final.telemetry.controllerActivity.trainer.appliedActions < previous.telemetry.controllerActivity.trainer.appliedActions) {
        throw new Error('authority identity changed or a monotonic counter regressed');
      }
      overloaded ||= final.schedulerOverloaded;
      const elapsed = (sample.afterMs - started) / 1000;
      const publication = generationPublication(
        { ...previousRead, generation: previous.generation },
        { ...sample, generation: final.generation });
      if (publication) transitions.push({ ...publication, wallSeconds: elapsed });
      previous = final;
      previousRead = sample;
      if (elapsed >= nextProgress) {
        recordResources(sample, elapsed);
        process.stderr.write(`elapsed=${elapsed.toFixed(1)}s generation=${counter(final.generation)} p99=${final.telemetry.step.p99Ms}ms dropped=${counter(final.schedulerDroppedWallMicros)} rssMiB=${(final.telemetry.process.rssBytes / 1024 ** 2).toFixed(1)} trainerActions=${final.telemetry.controllerActivity.trainer.appliedActions}\n`);
        nextProgress += 30;
      }
    } while ((finalRead.beforeMs - started) / 1000 < seconds);
  } catch (error) { failure = String(error); }
  let progressWindow: ReturnType<typeof counterWindow> | undefined;
  try { progressWindow = counterWindow(
    { ...initialRead, completedStep: initial.completedStep },
    { ...finalRead, completedStep: final.completedStep }); }
  catch (error) { failure ??= String(error); }
  const wallSeconds = progressWindow?.maximumWallSeconds ?? 0;
  if (JSON.stringify(await measurementSourceDigests()) !== JSON.stringify(measurementSources)) {
    failure ??= 'measurement source changed during the window';
  }
  const finalObservationSeconds = (finalRead.afterMs - started) / 1000;
  if (finalObservationSeconds > resourceSamples.at(-1)!.wallSeconds) recordResources(finalRead, finalObservationSeconds);
  let memorySoak: ReturnType<typeof summarizeRssSoak> | undefined;
  try { if (seconds >= 1800 && progressWindow && progressWindow.minimumWallSeconds >= 1800) {
    memorySoak = summarizeRssSoak(resourceSamples);
  } } catch (error) { failure ??= String(error); }
  let queueSoak: ReturnType<typeof summarizeQueueSoak> | undefined;
  try { queueSoak = summarizeQueueSoak(resourceSamples); }
  catch (error) { failure ??= String(error); }
  const deltaSteps = counter(final.completedStep) - counter(initial.completedStep);
  const dropped = counter(final.schedulerDroppedWallMicros) - counter(initial.schedulerDroppedWallMicros);
  const ratio = progressWindow?.minimumSimulatedWallRatio ?? 0;
  const intervals = transitions.slice(1).map((item, index) => generationInterval(transitions[index]!, item));
  const trainerActions = final.telemetry.controllerActivity.trainer.appliedActions - initial.telemetry.controllerActivity.trainer.appliedActions;
  const timing = final.telemetry;
  let stepWindow: ReturnType<typeof summarizeStepWindow> | undefined;
  try { stepWindow = summarizeStepWindow(initial, final); }
  catch (error) { failure ??= String(error); }
  const healthLatencyP95Ms = latencies.length ? percentile(latencies, 0.95) : null;
  const meetsMeasuredGates = !failure && progressWindow !== undefined && progressWindow.minimumWallSeconds >= seconds && ratio >= 0.98 && dropped === 0n &&
    !overloaded && stepWindow !== undefined && (scenario === 'P2' || stepWindow.meetsP0P1StepGate) && intervals.length > 0 && intervals.every(value => value.maximumSeconds <= 62) &&
    timing.checkpointBarrier.samples > 0 && timing.checkpointBarrier.p95Ms <= 1000 && timing.checkpointBarrier.maxMs <= 2000 &&
    timing.process.eventLoopDelayP95Ms <= 20 && timing.process.eventLoopDelayP99Ms <= 50 &&
    healthLatencyP95Ms !== null && healthLatencyP95Ms <= 100 &&
    timing.process.maxRssBytes < 12 * 1024 ** 3 &&
    trainerActions > 0 && timing.trainerAction.p95Ms <= 100 &&
    (seconds < 1800 || memorySoak?.meetsMemoryGate === true) && queueSoak?.meetsQueueGate === true;
  const report = { scenario, rustWorkers: workers, sourceRevision, measurementSources,
    startedAtUtc, requestedSeconds: seconds, wallSeconds,
    measuredScope: 'Production server with two independent real PyRL actors; connected-player timings are server receipt-to-application measurements. Browser rendering requires separate evidence.',
    histogramScope: 'stepWindow subtracts exact consistent step-computation bucket prefixes inside the same initial/final HTTP brackets, including generation-ending evolution/preparation computations. Persistence waits have a separate barrier clock; terminal phase attribution is also reported separately. Other native/interface histogram fields remain process-lifetime diagnostics, including pre-window trainer warm-up.',
    clockScope: 'Counter clocks bracket the complete initial/final HTTP reads. Acceptance uses the longest possible duration and lowest possible progress; minimum duration must cover the request. Generation intervals use last-old/first-new request brackets. Resource timestamps are response-receipt offsets from the initial reply; request brackets bound the health observations, not internal diagnostic sampling times. No sample is retimed after a failure or sampler shutdown.',
    counterClockBrackets: { initial: { beforeMs: initialRead.beforeMs, afterMs: initialRead.afterMs },
      final: { beforeMs: finalRead.beforeMs, afterMs: finalRead.afterMs } }, progressWindow,
    healthLatencyScope: 'Sampler-to-server route; loopback only if URL is loopback.',
    workloadWelcome: metadata, initialHealth: initial, finalHealth: final,
    deltaSteps: deltaSteps.toString(), simulatedWallRatio: ratio, droppedWallMicros: dropped.toString(),
    overloadedDuringSamples: overloaded, trainerAppliedActionsDelta: trainerActions,
    transitions, generationIntervalBounds: intervals,
    generationIntervalsSeconds: intervals.map(interval => interval.maximumSeconds),
    observationFailures, resourceSamples, memorySoak, queueSoak, stepWindow,
    healthLatencyP95Ms,
    healthLatencyMaxMs: latencies.length ? Math.max(...latencies) : null,
    failure, meetsMeasuredGates };
  await writeFile(resolve(output), `${JSON.stringify(report, null, 2)}\n`, { flag: 'wx' });
  process.stdout.write(`${JSON.stringify({ wallSeconds, simulatedWallRatio: ratio,
    droppedWallMicros: dropped.toString(), trainerActions, memorySoak, queueSoak, meetsMeasuredGates, failure })}\n`);
  if (!meetsMeasuredGates) process.exitCode = 1;
}

void run().catch(error => { process.stderr.write(`${String(error)}\n`); process.exitCode = 1; });
