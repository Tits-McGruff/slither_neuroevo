/** Sample an already running production workload while an independent real trainer supplies load. */
import { writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import WebSocket from 'ws';
import type { RustBackgroundHealth } from '../../src/protocol/rustBackground.ts';
import type { ExperimentalRuntimeTelemetrySnapshot } from '../../server/rustEngine/runtimeTelemetry.ts';

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
}

/** Decode an exact native counter without silently truncating large values. */
function counter(value: string): bigint {
  if (!/^[0-9a-f]{16}$/u.test(value)) throw new Error(`invalid native counter: ${value}`);
  return BigInt(`0x${value}`);
}

/** Read a bounded response and keep observation failures distinct from authority faults. */
async function readHealth(url: URL): Promise<{ health: Health; latencyMs: number }> {
  const started = performance.now();
  const response = await fetch(new URL('/api/health', url), { signal: AbortSignal.timeout(15_000) });
  const health = await response.json() as Health;
  if (!response.ok || health.ok !== true) throw new Error(`authority fault: ${JSON.stringify(health)}`);
  return { health, latencyMs: performance.now() - started };
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

/** Require the approved P1 configuration and real trainer activity before starting the clock. */
function assertP1(message: Record<string, unknown>, health: Health): void {
  const settings = message['settings'] as { core: { snakeCount: number; simSpeed: number };
    updates: Array<{ path: string; value: unknown }> };
  const inference = message['inferenceMode'] as { activeBackend: string; activeWorkerCount: number };
  const sensor = message['sensorSpec'] as { sensorCount: number };
  const value = (path: string): unknown => settings.updates.find(item => item.path === path)?.value;
  if (settings.core.snakeCount !== 300 || settings.core.simSpeed !== 1 ||
      value('generationSeconds') !== 60 || value('baselineBots.count') !== 10 ||
      value('pelletCountTarget') !== 3500 || value('sense.bubbleBins') !== 16 ||
      sensor.sensorCount !== 83 || inference.activeBackend !== 'native' ||
      inference.activeWorkerCount !== 6 || health.calculationWorkers !== 6 ||
      counter(health.generation) < 2n || health.telemetry.controllerActivity.trainer.freshAssignments < 2 ||
      health.telemetry.controllerActivity.trainer.appliedActions === 0) {
    throw new Error('P1 configuration, evolved population, six workers, or two active trainer actors missing');
  }
}

/** Compute a conservative empirical percentile for the sampler's request latencies. */
function percentile(values: readonly number[], fraction: number): number {
  const sorted = [...values].sort((left, right) => left - right);
  return sorted[Math.ceil(sorted.length * fraction) - 1]!;
}

/** Measure one uninterrupted authority and preserve a compact report even if sampling fails. */
async function run(): Promise<void> {
  const [base, output, secondsText, sourceRevision] = process.argv.slice(2);
  const seconds = Number(secondsText);
  if (!base || !output || !sourceRevision || !/^[0-9a-f]{40}$/u.test(sourceRevision) ||
      !Number.isInteger(seconds) || seconds < 600 || seconds > 7200) {
    throw new Error('usage: loaded-runtime-window.ts URL NEW_REPORT_PATH SECONDS>=600 SOURCE_COMMIT');
  }
  const url = new URL(base);
  const metadata = await welcome(url);
  const initial = (await readHealth(url)).health;
  if (!initial.nativeBuildIdentifier.includes(`+${sourceRevision.slice(0, 12)}.`)) {
    throw new Error('running addon does not identify the requested source revision');
  }
  assertP1(metadata, initial);
  const started = performance.now();
  const startedAtUtc = new Date().toISOString();
  const transitions: Array<{ generation: string; wallSeconds: number }> = [];
  const observationFailures: Array<{ wallSeconds: number; error: string }> = [];
  const latencies: number[] = [];
  let final = initial;
  let previous = initial;
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
      final = sample.health;
      latencies.push(sample.latencyMs);
      if (final.runId !== initial.runId ||
          counter(final.worldEpoch) - counter(previous.worldEpoch) !== counter(final.generation) - counter(previous.generation) ||
          final.configHash !== initial.configHash || final.nativeBuildIdentifier !== initial.nativeBuildIdentifier ||
          final.calculationWorkers !== 6 || counter(final.completedStep) < counter(previous.completedStep) ||
          counter(final.schedulerDroppedWallMicros) < counter(previous.schedulerDroppedWallMicros) ||
          counter(final.generation) < counter(previous.generation) ||
          final.telemetry.controllerActivity.trainer.appliedActions < previous.telemetry.controllerActivity.trainer.appliedActions) {
        throw new Error('authority identity changed or a monotonic counter regressed');
      }
      overloaded ||= final.schedulerOverloaded;
      const elapsed = (performance.now() - started) / 1000;
      if (final.generation !== previous.generation) transitions.push({ generation: final.generation, wallSeconds: elapsed });
      previous = final;
      if (elapsed >= nextProgress) {
        process.stderr.write(`elapsed=${elapsed.toFixed(1)}s generation=${counter(final.generation)} p99=${final.telemetry.step.p99Ms}ms dropped=${counter(final.schedulerDroppedWallMicros)} trainerActions=${final.telemetry.controllerActivity.trainer.appliedActions}\n`);
        nextProgress += 30;
      }
    } while ((performance.now() - started) / 1000 < seconds);
  } catch (error) { failure = String(error); }
  const wallSeconds = (performance.now() - started) / 1000;
  const deltaSteps = counter(final.completedStep) - counter(initial.completedStep);
  const dropped = counter(final.schedulerDroppedWallMicros) - counter(initial.schedulerDroppedWallMicros);
  const ratio = Number(deltaSteps) / 60 / wallSeconds;
  const intervals = transitions.slice(1).map((item, index) => item.wallSeconds - transitions[index]!.wallSeconds);
  const trainerActions = final.telemetry.controllerActivity.trainer.appliedActions - initial.telemetry.controllerActivity.trainer.appliedActions;
  const timing = final.telemetry;
  const meetsMeasuredGates = !failure && wallSeconds >= seconds && ratio >= 0.98 && dropped === 0n &&
    !overloaded && timing.step.p99Ms <= 16.667 && intervals.length > 0 && intervals.every(value => value <= 62) &&
    timing.checkpointBarrier.samples > 0 && timing.checkpointBarrier.p95Ms <= 1000 && timing.checkpointBarrier.maxMs <= 2000 &&
    timing.process.eventLoopDelayP95Ms <= 20 && timing.process.eventLoopDelayP99Ms <= 50 &&
    trainerActions > 0 && timing.trainerAction.p95Ms <= 100;
  const report = { scenario: 'P1', sourceRevision, startedAtUtc, requestedSeconds: seconds, wallSeconds,
    measuredScope: 'Production server with two independent real PyRL actors; browser/player budgets unmeasured.',
    histogramScope: 'Native and interface histograms cover this server process lifetime, including pre-window trainer warm-up.',
    healthLatencyScope: 'Sampler-to-server route; loopback only if URL is loopback.',
    workloadWelcome: metadata, initialHealth: initial, finalHealth: final,
    deltaSteps: deltaSteps.toString(), simulatedWallRatio: ratio, droppedWallMicros: dropped.toString(),
    overloadedDuringSamples: overloaded, trainerAppliedActionsDelta: trainerActions,
    transitions, generationIntervalsSeconds: intervals, observationFailures,
    healthLatencyP95Ms: latencies.length ? percentile(latencies, 0.95) : null,
    healthLatencyMaxMs: latencies.length ? Math.max(...latencies) : null,
    failure, meetsMeasuredGates };
  await writeFile(resolve(output), `${JSON.stringify(report, null, 2)}\n`, { flag: 'wx' });
  process.stdout.write(`${JSON.stringify({ wallSeconds, simulatedWallRatio: ratio,
    droppedWallMicros: dropped.toString(), trainerActions, meetsMeasuredGates, failure })}\n`);
  if (!meetsMeasuredGates) process.exitCode = 1;
}

void run().catch(error => { process.stderr.write(`${String(error)}\n`); process.exitCode = 1; });
