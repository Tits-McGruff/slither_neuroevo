/** Production-shaped Stage 2 TypeScript runtime benchmark. */

import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { monitorEventLoopDelay, performance } from 'node:perf_hooks';
import { spawnSync } from 'node:child_process';
import { prepareInferenceBackend, getNativeAddonBuildIdentifier } from '../../src/brains/nativeBridge.ts';
import type { InferenceBackend } from '../../src/brains/types.ts';
import { resetCFGToDefaults } from '../../src/config.ts';
import { enrichArchInfo } from '../../src/mlp.ts';
import { SimProfiler } from '../../src/profiling.ts';
import { SimCore } from '../../src/sim/SimCore.ts';
import {
  installDenseLongBodies,
  installStage2Scenario,
  STAGE2_WORLD_SEED,
  type Stage2RecurrentKind,
  type Stage2ScenarioName
} from './fixtures.ts';

/** Schema version for machine-readable runtime evidence. */
const RESULT_VERSION = 1;
/** Default measured fixed steps. */
const DEFAULT_MEASURED_STEPS = 120;
/** Default warm-up fixed steps. */
const DEFAULT_WARMUP_STEPS = 30;

/** Parsed runtime benchmark options. */
interface RuntimeOptions {
  /** Standard workload. */
  scenario: Stage2ScenarioName;
  /** Math backend. */
  backend: InferenceBackend;
  /** Recurrent family for P2/P3. */
  recurrentKind: Stage2RecurrentKind;
  /** Warm-up step count. */
  warmupSteps: number;
  /** Measured step count. */
  measuredSteps: number;
  /** Serialize every Nth measured step; zero disables frames. */
  frameEvery: number;
  /** Optional JSON artifact destination. */
  outputPath: string | null;
}

/** Quantile summary for one numeric sample. */
interface Distribution {
  /** Number of samples. */
  count: number;
  /** Minimum. */
  min: number;
  /** Median. */
  p50: number;
  /** 95th percentile. */
  p95: number;
  /** 99th percentile. */
  p99: number;
  /** Maximum. */
  max: number;
  /** Arithmetic mean. */
  mean: number;
}

/**
 * Parse a positive bounded integer.
 * @param value - CLI value.
 * @param name - Option name.
 * @param allowZero - Whether zero is valid.
 * @returns Parsed integer.
 */
function parseCount(value: string | undefined, name: string, allowZero = false): number {
  if (value == null) throw new Error(`${name} requires a value`);
  const parsed = Number.parseInt(value, 10);
  const minimum = allowZero ? 0 : 1;
  if (!Number.isSafeInteger(parsed) || parsed < minimum || parsed > 10_000_000) {
    throw new Error(`${name} must be an integer from ${minimum} to 10000000`);
  }
  return parsed;
}

/**
 * Parse command-line options.
 * @param argv - Arguments after the script path.
 * @returns Validated benchmark options.
 */
function parseOptions(argv: readonly string[]): RuntimeOptions {
  const result: RuntimeOptions = {
    scenario: 'P0',
    backend: 'native',
    recurrentKind: 'GRU',
    warmupSteps: DEFAULT_WARMUP_STEPS,
    measuredSteps: DEFAULT_MEASURED_STEPS,
    frameEvery: 1,
    outputPath: null
  };
  for (let index = 0; index < argv.length; index++) {
    const option = argv[index];
    const value = argv[index + 1];
    switch (option) {
      case '--scenario': {
        if (value !== 'P0' && value !== 'P1' && value !== 'P2' && value !== 'P3' && value !== 'P4') {
          throw new Error('--scenario must be P0, P1, P2, P3, or P4');
        }
        result.scenario = value;
        index++;
        break;
      }
      case '--backend': {
        if (value !== 'native' && value !== 'js') throw new Error('--backend must be native or js');
        result.backend = value;
        index++;
        break;
      }
      case '--recurrent': {
        const normalized = value?.toUpperCase();
        if (normalized !== 'GRU' && normalized !== 'LSTM' && normalized !== 'RRU') {
          throw new Error('--recurrent must be GRU, LSTM, or RRU');
        }
        result.recurrentKind = normalized;
        index++;
        break;
      }
      case '--warmup-steps':
        result.warmupSteps = parseCount(value, option, true);
        index++;
        break;
      case '--steps':
        result.measuredSteps = parseCount(value, option);
        index++;
        break;
      case '--frame-every':
        result.frameEvery = parseCount(value, option, true);
        index++;
        break;
      case '--output':
        if (!value) throw new Error('--output requires a path');
        result.outputPath = path.resolve(value);
        index++;
        break;
      default:
        throw new Error(`Unknown option ${option ?? '<missing>'}`);
    }
  }
  return result;
}

/**
 * Calculate a nearest-rank interpolated distribution.
 * @param values - Finite samples.
 * @returns Distribution summary.
 */
function distribution(values: readonly number[]): Distribution {
  if (values.length === 0) {
    return { count: 0, min: 0, p50: 0, p95: 0, p99: 0, max: 0, mean: 0 };
  }
  const sorted = [...values].sort((left, right) => left - right);
  const percentile = (fraction: number): number => {
    const position = (sorted.length - 1) * fraction;
    const lower = Math.floor(position);
    const upper = Math.ceil(position);
    const lowValue = sorted[lower]!;
    const highValue = sorted[upper]!;
    return lowValue + (highValue - lowValue) * (position - lower);
  };
  return {
    count: sorted.length,
    min: sorted[0]!,
    p50: percentile(0.5),
    p95: percentile(0.95),
    p99: percentile(0.99),
    max: sorted[sorted.length - 1]!,
    mean: sorted.reduce((sum, value) => sum + value, 0) / sorted.length
  };
}

/**
 * Round timing output without hiding meaningful microsecond differences.
 * @param summary - Raw distribution.
 * @returns Rounded distribution.
 */
function roundedDistribution(summary: Distribution): Distribution {
  const rounded = (value: number): number => Number(value.toFixed(6));
  return {
    count: summary.count,
    min: rounded(summary.min),
    p50: rounded(summary.p50),
    p95: rounded(summary.p95),
    p99: rounded(summary.p99),
    max: rounded(summary.max),
    mean: rounded(summary.mean)
  };
}

/**
 * Return the checked-out Git commit without mutating the repository.
 * @returns Commit identity or an explicit unavailable marker.
 */
function sourceCommit(): string {
  const result = spawnSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' });
  return result.status === 0 ? result.stdout.trim() : 'unavailable';
}

/**
 * Yield to the real Node event loop between fixed steps.
 * @returns Promise resolved on the next check phase.
 */
function yieldEventLoop(): Promise<void> {
  return new Promise(resolve => setImmediate(resolve));
}

/**
 * Run one scenario and return its complete evidence object.
 * @param options - Validated benchmark options.
 * @returns Machine-readable result.
 */
async function runRuntimeBaseline(options: RuntimeOptions): Promise<Record<string, unknown>> {
  const scenario = installStage2Scenario(options.scenario, options.recurrentKind);
  await prepareInferenceBackend(options.backend);
  const core = new SimCore({
    settings: scenario.settings,
    worldSeed: STAGE2_WORLD_SEED,
    runId: `stage2-${scenario.name.toLowerCase()}-${options.recurrentKind.toLowerCase()}`,
    inferenceBackend: options.backend,
    maxStepsPerPump: 1,
    tickRateHz: 60
  });
  const installedBodyPoints = scenario.denseLongBodies ? installDenseLongBodies(core.world) : null;
  for (let step = 0; step < options.warmupSteps; step++) {
    await core.update(core.fixedDt);
    await yieldEventLoop();
  }

  const profiler = new SimProfiler({ enabled: true, reportIntervalMs: 3_600_000 });
  core.world.profiler = profiler;
  const eventLoop = monitorEventLoopDelay({ resolution: 10 });
  eventLoop.enable();
  const stepMs: number[] = [];
  const sensorMs: number[] = [];
  const brainMs: number[] = [];
  const physicsMs: number[] = [];
  const framePackMs: number[] = [];
  const frameBytes: number[] = [];
  let peakRssBytes = process.memoryUsage().rss;
  let peakHeapUsedBytes = process.memoryUsage().heapUsed;
  let peakExternalBytes = process.memoryUsage().external;
  const cpuBefore = process.cpuUsage();
  const memoryBefore = process.memoryUsage();
  const wallStarted = performance.now();
  for (let step = 0; step < options.measuredSteps; step++) {
    const started = performance.now();
    const committed = await core.update(core.fixedDt);
    const duration = performance.now() - started;
    if (committed !== 1) throw new Error(`Expected one committed step, received ${committed}`);
    stepMs.push(duration);
    sensorMs.push(profiler.tickSensorsMs);
    brainMs.push(profiler.tickBrainMs);
    physicsMs.push(Math.max(0, duration - profiler.tickSensorsMs - profiler.tickBrainMs));
    if (options.frameEvery > 0 && step % options.frameEvery === 0) {
      const frameStarted = performance.now();
      const frame = core.serialize();
      framePackMs.push(performance.now() - frameStarted);
      frameBytes.push(frame.byteLength);
    }
    const memory = process.memoryUsage();
    peakRssBytes = Math.max(peakRssBytes, memory.rss);
    peakHeapUsedBytes = Math.max(peakHeapUsedBytes, memory.heapUsed);
    peakExternalBytes = Math.max(peakExternalBytes, memory.external);
    await yieldEventLoop();
  }
  const measuredWallMs = performance.now() - wallStarted;
  const cpu = process.cpuUsage(cpuBefore);
  const memoryAfter = process.memoryUsage();
  eventLoop.disable();
  const graph = enrichArchInfo(core.world.arch);
  const nativeKernelNodesPerBrain = graph.nodes.filter(node => node.length > 0).length;
  const brainCallCount = profiler.windowBrainCalls;
  const collision = core.world.getCollisionGridDiagnostics();
  return {
    schema: 'slither-stage2-runtime-baseline',
    version: RESULT_VERSION,
    evidenceClass: 'new measured result',
    caveat: 'Direct production SimCore/World path on the named host; not an end-to-end server, LAN, browser, or target-VM result.',
    source: {
      commit: sourceCommit(),
      dirty: spawnSync('git', ['status', '--porcelain'], { encoding: 'utf8' }).stdout.trim().length > 0
    },
    environment: {
      capturedAt: new Date().toISOString(),
      platform: process.platform,
      architecture: process.arch,
      osType: os.type(),
      osRelease: os.release(),
      osVersion: os.version(),
      hostname: os.hostname(),
      locale: Intl.DateTimeFormat().resolvedOptions().locale,
      node: process.version,
      v8: process.versions.v8,
      zlib: process.versions.zlib,
      zstd: process.versions.zstd ?? null,
      cpuModel: os.cpus()[0]?.model ?? 'unknown',
      logicalCpuCount: os.cpus().length,
      totalMemoryBytes: os.totalmem()
    },
    workload: {
      scenario,
      seed: STAGE2_WORLD_SEED,
      backend: options.backend,
      nativeAddonBuildIdentifier: getNativeAddonBuildIdentifier(),
      recurrentKind: options.recurrentKind,
      warmupSteps: options.warmupSteps,
      measuredSteps: options.measuredSteps,
      fixedDtSeconds: core.fixedDt,
      frameEvery: options.frameEvery,
      installedBodyPoints,
      graph: {
        key: graph.key,
        totalParams: graph.totalCount,
        totalStateFloats: graph.compiled.totalStateSize,
        order: graph.compiled.order,
        outputs: graph.compiled.outputs,
        nodes: graph.nodes
      }
    },
    result: {
      measuredWallMs: Number(measuredWallMs.toFixed(6)),
      simulatedSeconds: Number((options.measuredSteps * core.fixedDt).toFixed(9)),
      simulatedSecondsPerWallSecond: Number(
        ((options.measuredSteps * core.fixedDt) / (measuredWallMs / 1000)).toFixed(6)
      ),
      completedStepsPerWallSecond: Number((options.measuredSteps / (measuredWallMs / 1000)).toFixed(6)),
      fixedStepMs: roundedDistribution(distribution(stepMs)),
      sensorsPerStepMs: roundedDistribution(distribution(sensorMs)),
      brainPerStepMs: roundedDistribution(distribution(brainMs)),
      remainingPhysicsPerStepMs: roundedDistribution(distribution(physicsMs)),
      framePackMs: roundedDistribution(distribution(framePackMs)),
      frameBytes: roundedDistribution(distribution(frameBytes)),
      aliveAtEnd: core.world.snakes.filter(snake => snake.alive).length,
      totalSnakesAtEnd: core.world.snakes.length,
      generationAtEnd: core.world.generation,
      generationTimeAtEnd: Number(core.world.generationTime.toFixed(9)),
      profilerBrainCalls: brainCallCount,
      profilerSensorCalls: profiler.windowSensorCalls,
      derivedNativeCrossings: options.backend === 'native'
        ? brainCallCount * nativeKernelNodesPerBrain
        : 0,
      nativeKernelNodesPerBrain,
      collisionGrid: collision,
      eventLoopDelayMs: {
        min: Number((eventLoop.min / 1e6).toFixed(6)),
        mean: Number((eventLoop.mean / 1e6).toFixed(6)),
        p95: Number((eventLoop.percentile(95) / 1e6).toFixed(6)),
        p99: Number((eventLoop.percentile(99) / 1e6).toFixed(6)),
        max: Number((eventLoop.max / 1e6).toFixed(6))
      },
      cpu: {
        userMicros: cpu.user,
        systemMicros: cpu.system,
        totalMicros: cpu.user + cpu.system,
        averageLogicalCoreUtilization: Number(
          (((cpu.user + cpu.system) / 1000) / measuredWallMs).toFixed(6)
        )
      },
      memory: {
        before: memoryBefore,
        after: memoryAfter,
        peakRssBytes,
        peakHeapUsedBytes,
        peakExternalBytes
      }
    }
  };
}

/** Execute the command-line runner. */
async function main(): Promise<void> {
  const options = parseOptions(process.argv.slice(2));
  try {
    const result = await runRuntimeBaseline(options);
    const json = `${JSON.stringify(result, null, 2)}\n`;
    if (options.outputPath) {
      fs.mkdirSync(path.dirname(options.outputPath), { recursive: true });
      fs.writeFileSync(options.outputPath, json, 'utf8');
      console.info(`[stage2.runtime] wrote ${options.outputPath}`);
    } else {
      process.stdout.write(json);
    }
  } finally {
    resetCFGToDefaults();
  }
}

main().catch(error => {
  console.error(error);
  process.exitCode = 1;
});
