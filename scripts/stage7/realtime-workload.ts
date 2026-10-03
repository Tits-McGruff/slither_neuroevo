/** Measure P0/P1/P2 through the running Rust server at the requested 1× rate. */

import { existsSync } from 'node:fs';
import { mkdir } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import WebSocket from 'ws';
import { DEFAULT_CONFIG } from '../../server/config.ts';
import { startRustServer } from '../../server/rustServer.ts';
import { buildLargeBrainGraph } from '../stage2/fixtures.ts';

/** Mandatory real-time workload names from the approved plan. */
type Scenario = 'P0' | 'P1' | 'P2';

/** Validated request for a new disposable measurement database. */
interface Options {
  /** Approved workload to configure through Protocol 2. */
  scenario: Scenario;
  /** New database path whose managed directory also must be absent. */
  databasePath: string;
  /** Wall seconds sampled after generation-one warm-up. */
  measureSeconds: number;
  /** Native calculation worker count to measure. */
  rustWorkers: number;
}

/** Small subset of production health used to evaluate one measured interval. */
interface Health {
  /** False when any native or interface fault has stopped authority. */
  ok: boolean;
  /** Current exact generation as sixteen hexadecimal digits. */
  generation: string;
  /** Committed complete steps as sixteen hexadecimal digits. */
  completedStep: string;
  /** Requested time discarded by the bounded scheduler. */
  schedulerDroppedWallMicros: string;
  /** Whether the scheduler currently reports overload. */
  schedulerOverloaded: boolean;
  /** Public fault if the authority stopped. */
  interfaceFault?: string;
  /** Aggregated runtime timings and process memory from production health. */
  telemetry: { step: { samples: number; p95Ms: number; p99Ms: number; maxMs: number }; process: {
    rssBytes: number; maxRssBytes: number; eventLoopDelayP95Ms: number;
    eventLoopDelayP99Ms: number; eventLoopDelayMaxMs: number
  }; checkpointBarrier: { samples: number; p95Ms: number; maxMs: number };
  frame: { latestBytes: number; maximumObservedBytes: number } };
}

/** Parse one bounded positive integer without accepting a partial string. */
function positive(value: string | undefined, label: string, maximum: number): number {
  if (!value || !/^[1-9][0-9]*$/u.test(value)) throw new Error(`${label} must be a positive integer`);
  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed) || parsed > maximum) {
    throw new RangeError(`${label} must be at most ${maximum}`);
  }
  return parsed;
}

/** Require an explicit scenario, absent database, duration, and worker count. */
function options(argv: readonly string[]): Options {
  const values = new Map<string, string>();
  for (let index = 0; index < argv.length; index += 2) {
    const key = argv[index];
    const value = argv[index + 1];
    if (!key || !value || !['--scenario', '--db-path', '--measure-seconds', '--rust-workers'].includes(key) ||
        values.has(key)) throw new Error(`invalid or duplicate option: ${key ?? '<missing>'}`);
    values.set(key, value);
  }
  const scenario = values.get('--scenario');
  if (scenario !== 'P0' && scenario !== 'P1' && scenario !== 'P2') {
    throw new Error('--scenario must be P0, P1, or P2');
  }
  const path = values.get('--db-path');
  if (!path) throw new Error('--db-path is required');
  const databasePath = resolve(path);
  if (existsSync(databasePath) || existsSync(`${databasePath}.checkpoints`)) {
    throw new Error(`measurement destination already exists: ${databasePath}`);
  }
  return { scenario, databasePath,
    measureSeconds: positive(values.get('--measure-seconds') ?? '600', '--measure-seconds', 7200),
    rustWorkers: positive(values.get('--rust-workers') ?? '5', '--rust-workers', 7) };
}

/** Decode one small Protocol 2 JSON message without retaining display frames. */
function packet(data: WebSocket.RawData): Record<string, unknown> {
  const value: unknown = JSON.parse(data.toString());
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('invalid server packet');
  return value as Record<string, unknown>;
}

/** Check the actual native workload advertised by the server after setup. */
function assertWorkload(welcome: Record<string, unknown>, scenario: Scenario, workers: number): void {
  const settings = welcome['settings'] as { core?: { snakeCount?: number; simSpeed?: number };
    updates?: Array<{ path?: string; value?: unknown }> } | undefined;
  const sensor = welcome['sensorSpec'] as { sensorCount?: number } | undefined;
  const inference = welcome['inferenceMode'] as { activeBackend?: string; activeWorkerCount?: number;
    parameterCount?: number } | undefined;
  const value = (path: string): unknown => settings?.updates?.find(update => update.path === path)?.value;
  if (settings?.core?.snakeCount !== (scenario === 'P1' ? 300 : 55) ||
      settings.core.simSpeed !== 1 || value('generationSeconds') !== 60 ||
      value('baselineBots.count') !== 10 || value('pelletCountTarget') !== 3500 ||
      value('sense.bubbleBins') !== (scenario === 'P2' ? 32 : 16) ||
      sensor?.sensorCount !== (scenario === 'P2' ? 147 : 83) ||
      inference?.activeBackend !== 'native' || inference.activeWorkerCount !== workers ||
      (scenario === 'P2' && (inference.parameterCount ?? 0) < 400_000)) {
    throw new Error(`Rust welcome did not select the approved ${scenario} workload`);
  }
}

/** Configure the approved 60-second workload by Reset and verify its welcome. */
export async function configure(port: number, scenario: Scenario, workers: number): Promise<void> {
  const socket = new WebSocket(`ws://127.0.0.1:${port}`);
  await new Promise<void>((resolveReady, reject) => {
    let done = false;
    const timer = setTimeout(() => finish(new Error('workload setup timed out')), 120_000);
    /** Settle the one-command exchange and release its spectator socket. */
    const finish = (error?: Error): void => {
      if (done) return;
      done = true;
      clearTimeout(timer);
      socket.terminate();
      if (error) reject(error);
      else resolveReady();
    };
    socket.on('open', () => socket.send(JSON.stringify({ type: 'hello', version: 2, clientType: 'ui' })));
    socket.on('error', error => finish(error));
    socket.on('close', () => finish(new Error('workload socket closed before setup')));
    socket.on('message', (data, binary) => {
      if (binary || done) return;
      try {
        const message = packet(data);
        if (message['type'] === 'welcome') {
          socket.send(JSON.stringify({ type: 'join', mode: 'spectator' }));
          socket.send(JSON.stringify({ type: 'reset',
            settings: { snakeCount: scenario === 'P1' ? 300 : 55, simSpeed: 1 },
            updates: [
              { path: 'generationSeconds', value: 60 },
              { path: 'sense.bubbleBins', value: scenario === 'P2' ? 32 : 16 },
              { path: 'baselineBots.count', value: 10 },
              { path: 'pelletCountTarget', value: 3500 }
            ],
            ...(scenario === 'P2' ? { graphSpec: buildLargeBrainGraph(147) } : {})
          }));
        } else if (message['type'] === 'stateReplaced' && message['reason'] === 'reset') {
          assertWorkload(message['welcome'] as Record<string, unknown>, scenario, workers);
          finish();
        } else if (message['type'] === 'error') {
          finish(new Error(`workload setup failed: ${String(message['message'])}`));
        }
      } catch (error) { finish(error instanceof Error ? error : new Error(String(error))); }
    });
  });
}

/** Read one bounded health response and its local request latency. */
async function readHealth(port: number): Promise<{ health: Health; latencyMs: number }> {
  const started = performance.now();
  const response = await fetch(`http://127.0.0.1:${port}/api/health`, {
    signal: AbortSignal.timeout(5_000)
  });
  const health = await response.json() as Health;
  const latencyMs = performance.now() - started;
  if (!response.ok || !health.ok) throw new Error(`Rust authority faulted: ${health.interfaceFault ?? response.status}`);
  return { health, latencyMs };
}

/** Return the upper sampled percentile without assuming a normal distribution. */
function percentile(values: number[], fraction: number): number {
  if (values.length === 0) throw new Error('cannot summarize empty latency samples');
  const sorted = [...values].sort((left, right) => left - right);
  return sorted[Math.ceil(fraction * sorted.length) - 1]!;
}

/** Wait through generation one so the measured population has evolved once. */
async function warmToGenerationTwo(port: number): Promise<Health> {
  const deadline = performance.now() + 600_000;
  for (;;) {
    const { health } = await readHealth(port);
    if (BigInt(`0x${health.generation}`) >= 2n) return health;
    if (performance.now() >= deadline) throw new Error('first 60-second generation did not complete within ten minutes');
    await new Promise<void>(done => setTimeout(done, 500));
  }
}

/** Measure one evolved 1× Rust run through real HTTP health and generation commits. */
export async function run(request: Options): Promise<Record<string, unknown>> {
  await mkdir(dirname(request.databasePath), { recursive: true });
  const warmup = await startRustServer({ ...DEFAULT_CONFIG, port: 0,
    dbPath: request.databasePath, resume: 'fresh', seed: 1511506142,
    rustCalculationWorkers: request.rustWorkers, logLevel: 'error' });
  if (warmup.startupFault) {
    await warmup.close();
    throw new Error(`Rust startup failed: ${warmup.startupFault}`);
  }
  try {
    await configure(warmup.port, request.scenario, request.rustWorkers);
    await warmToGenerationTwo(warmup.port);
  } finally { await warmup.close(); }
  const server = await startRustServer({ ...DEFAULT_CONFIG, port: 0,
    dbPath: request.databasePath, resume: 'latest',
    rustCalculationWorkers: request.rustWorkers, logLevel: 'error' });
  if (server.startupFault) {
    await server.close();
    throw new Error(`Rust evolved-checkpoint restart failed: ${server.startupFault}`);
  }
  try {
    const initial = await readHealth(server.port);
    if (BigInt(`0x${initial.health.generation}`) !== 2n) {
      throw new Error('evolved-checkpoint restart did not begin at generation two');
    }
    const startedAt = performance.now();
    const startedCpu = process.cpuUsage();
    const endAt = startedAt + request.measureSeconds * 1000;
    const latencies: number[] = [initial.latencyMs];
    const transitions: Array<{ generation: string; wallSeconds: number }> = [];
    let previousGeneration = initial.health.generation;
    let final = initial.health;
    let nextProgress = startedAt + 30_000;
    while (performance.now() < endAt) {
      const sample = await readHealth(server.port);
      final = sample.health;
      latencies.push(sample.latencyMs);
      if (final.generation !== previousGeneration) {
        transitions.push({ generation: final.generation,
          wallSeconds: (performance.now() - startedAt) / 1000 });
        previousGeneration = final.generation;
      }
      if (performance.now() >= nextProgress) {
        process.stderr.write(`scenario=${request.scenario} generation=${BigInt(`0x${final.generation}`)} step=${BigInt(`0x${final.completedStep}`)} dropped=${BigInt(`0x${final.schedulerDroppedWallMicros}`)}\n`);
        nextProgress += 30_000;
      }
      await new Promise<void>(done => setTimeout(done, 250));
    }
    const wallSeconds = (performance.now() - startedAt) / 1000;
    const deltaSteps = BigInt(`0x${final.completedStep}`) - BigInt(`0x${initial.health.completedStep}`);
    const droppedWallMicros = BigInt(`0x${final.schedulerDroppedWallMicros}`) -
      BigInt(`0x${initial.health.schedulerDroppedWallMicros}`);
    const cpu = process.cpuUsage(startedCpu);
    const transitionIntervals = transitions.slice(1).map((transition, index) =>
      transition.wallSeconds - transitions[index]!.wallSeconds);
    const simulatedWallRatio = Number(deltaSteps) / 60 / wallSeconds;
    return { scenario: request.scenario, rustWorkers: request.rustWorkers,
      measuredEvolvedPopulation: true, measurementStartedFromCheckpoint: true,
      requestedMeasureSeconds: request.measureSeconds,
      wallSeconds, deltaSteps: deltaSteps.toString(), simulatedWallRatio,
      droppedWallMicros: droppedWallMicros.toString(), schedulerOverloadedAtEnd: final.schedulerOverloaded,
      startGeneration: initial.health.generation, endGeneration: final.generation,
      transitions, transitionIntervals,
      healthLatencyP95Ms: percentile(latencies, 0.95), healthLatencyMaxMs: Math.max(...latencies),
      stepSamples: final.telemetry.step.samples,
      stepP95Ms: final.telemetry.step.p95Ms, stepP99Ms: final.telemetry.step.p99Ms,
      stepMaxMs: final.telemetry.step.maxMs,
      eventLoopDelayP95Ms: final.telemetry.process.eventLoopDelayP95Ms,
      eventLoopDelayP99Ms: final.telemetry.process.eventLoopDelayP99Ms,
      eventLoopDelayMaxMs: final.telemetry.process.eventLoopDelayMaxMs,
      checkpointBarrier: final.telemetry.checkpointBarrier,
      frame: final.telemetry.frame,
      maxRssBytes: final.telemetry.process.maxRssBytes,
      cpuUserSeconds: cpu.user / 1_000_000, cpuSystemSeconds: cpu.system / 1_000_000,
      meetsMeasuredRatioAndDebtGate: simulatedWallRatio >= 0.98 && droppedWallMicros === 0n };
  } finally { await server.close(); }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const request = options(process.argv.slice(2));
  void run(request).then(result => process.stdout.write(`${JSON.stringify(result)}\n`)).catch(error => {
    process.stderr.write(`${String(error)}\n`);
    process.exitCode = 1;
  });
}
