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
import { BrainPool } from '../../server/brainPool.ts';
import {
  captureStage2WorldLoad,
  expectedStage2CollisionEntries,
  installDenseLongBodies,
  installStage2Scenario,
  P4_DENSE_COLLISION_ENTRY_THRESHOLD,
  STAGE2_WORLD_SEED,
  type Stage2WorldLoadSnapshot,
  type Stage2RecurrentKind,
  type Stage2ScenarioName
} from './fixtures.ts';

/** Schema version for machine-readable runtime evidence. */
const RESULT_VERSION = 2;
/** Default measured fixed steps. */
const DEFAULT_MEASURED_STEPS = 120;
/** Default warm-up fixed steps. */
const DEFAULT_WARMUP_STEPS = 30;
/** Provenance declaration supplied by the benchmark operator. */
type EvidenceEnvironment = 'development' | 'owner-target-vm';

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
  /** Retain a load/timing sample every N measured steps; zero disables it. */
  sampleEverySteps: number;
  /** Canonical Node inference workers; zero keeps the serial path. */
  workers: number;
  /** Optional JSON artifact destination. */
  outputPath: string | null;
  /** Explicit provenance declaration; a matching hardware profile is not host identity. */
  evidenceEnvironment: EvidenceEnvironment;
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

/** One bounded point in the optional dense-world load timeline. */
interface RuntimeTimelineSample {
  /** Completed measured-step count, with zero representing the measured start. */
  measuredStep: number;
  /** Wall milliseconds since the measured interval began. */
  elapsedWallMs: number;
  /** Fixed-step duration, or null for the measured-start sample. */
  fixedStepMs: number | null;
  /** Frame bytes packed during this step, or null when no frame was packed. */
  frameBytes: number | null;
  /** World and collision-index load at this completed boundary. */
  load: Stage2WorldLoadSnapshot;
  /** Resident bytes sampled at this boundary. */
  rssBytes: number;
  /** JavaScript heap bytes sampled at this boundary. */
  heapUsedBytes: number;
  /** External/backing-store bytes sampled at this boundary. */
  externalBytes: number;
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
  let warmupExplicit = false;
  const result: RuntimeOptions = {
    scenario: 'P0',
    backend: 'native',
    recurrentKind: 'GRU',
    warmupSteps: DEFAULT_WARMUP_STEPS,
    measuredSteps: DEFAULT_MEASURED_STEPS,
    frameEvery: 1,
    sampleEverySteps: 0,
    workers: 0,
    outputPath: null,
    evidenceEnvironment: 'development'
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
        warmupExplicit = true;
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
      case '--sample-every-steps':
        result.sampleEverySteps = parseCount(value, option, true);
        index++;
        break;
      case '--workers':
        result.workers = parseCount(value, option, true);
        if (result.workers > 8) throw new Error('--workers cannot exceed 8');
        index++;
        break;
      case '--output':
        if (!value) throw new Error('--output requires a path');
        result.outputPath = path.resolve(value);
        index++;
        break;
      case '--environment':
        if (value !== 'development' && value !== 'owner-target-vm') {
          throw new Error('--environment must be development or owner-target-vm');
        }
        result.evidenceEnvironment = value;
        index++;
        break;
      default:
        throw new Error(`Unknown option ${option ?? '<missing>'}`);
    }
  }
  if (result.scenario === 'P4' && !warmupExplicit) {
    result.warmupSteps = 0;
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
 * Return checked-out source identity without taking optional Git locks.
 * @returns Commit identity and dirty flag.
 */
function sourceIdentity(): { commit: string; dirty: boolean } {
  const gitEnvironment = { ...process.env, GIT_OPTIONAL_LOCKS: '0' };
  const commit = spawnSync('git', ['rev-parse', 'HEAD'], {
    encoding: 'utf8',
    env: gitEnvironment
  });
  const status = spawnSync('git', ['status', '--porcelain'], {
    encoding: 'utf8',
    env: gitEnvironment
  });
  return {
    commit: commit.status === 0 ? commit.stdout.trim() : 'unavailable',
    dirty: status.status !== 0 || status.stdout.trim().length > 0
  };
}

/**
 * Read the Linux distribution identifier without invoking another process.
 * @returns `/etc/os-release` ID, or null outside Linux or when unavailable.
 */
function linuxDistributionId(): string | null {
  if (process.platform !== 'linux') return null;
  try {
    const idLine = fs.readFileSync('/etc/os-release', 'utf8')
      .split(/\r?\n/u)
      .find(line => line.startsWith('ID='));
    if (!idLine) return null;
    return idLine.slice(3).trim().replace(/^['"]|['"]$/gu, '').toLowerCase();
  } catch {
    return null;
  }
}

/**
 * Capture and validate the explicit owner-target evidence declaration.
 * @param declaration - Operator-provided provenance class.
 * @returns Individual environment facts and their combined validation result.
 */
function captureEnvironmentProvenance(declaration: EvidenceEnvironment): {
  declaration: EvidenceEnvironment;
  platformIsLinux: boolean;
  distributionId: string | null;
  distributionIsDebian: boolean;
  hostname: string;
  hostnameIsOxygen: boolean;
  cpuModel: string;
  cpuModelMatches: boolean;
  logicalCpuCount: number;
  logicalCpuCountMatches: boolean;
  totalMemoryBytes: number;
  memoryAllocationMatches: boolean;
  ownerTargetVmValidated: boolean;
} {
  const platformIsLinux = process.platform === 'linux';
  const distributionId = linuxDistributionId();
  const hostname = os.hostname();
  const cpuModel = os.cpus()[0]?.model ?? 'unknown';
  const logicalCpuCount = os.cpus().length;
  const totalMemoryBytes = os.totalmem();
  const facts = {
    declaration,
    platformIsLinux,
    distributionId,
    distributionIsDebian: distributionId === 'debian',
    hostname,
    hostnameIsOxygen: hostname.toLowerCase().split('.')[0] === 'oxygen',
    cpuModel,
    cpuModelMatches: cpuModel.includes('AMD Ryzen 7 2700'),
    logicalCpuCount,
    logicalCpuCountMatches: logicalCpuCount === 8,
    totalMemoryBytes,
    memoryAllocationMatches: totalMemoryBytes >= 15 * 1024 * 1024 * 1024,
    ownerTargetVmValidated: false
  };
  facts.ownerTargetVmValidated =
    declaration === 'owner-target-vm' &&
    facts.platformIsLinux &&
    facts.distributionIsDebian &&
    facts.hostnameIsOxygen &&
    facts.cpuModelMatches &&
    facts.logicalCpuCountMatches &&
    facts.memoryAllocationMatches;
  if (declaration === 'owner-target-vm' && !facts.ownerTargetVmValidated) {
    throw new Error(
      `--environment owner-target-vm did not match oxygen Debian/Ryzen/8-vCPU/15-GiB facts: ${JSON.stringify(facts)}`
    );
  }
  return facts;
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
  const provenance = captureEnvironmentProvenance(options.evidenceEnvironment);
  const source = sourceIdentity();
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
  let pool: BrainPool | null = null;
  try {
    if (options.workers > 0) {
      const graph = enrichArchInfo(core.world.arch);
      const packedWeights = new Float32Array(core.world.population.length * graph.totalCount);
      for (let slot = 0; slot < core.world.population.length; slot++) {
        packedWeights.set(core.world.population[slot]!.weights, slot * graph.totalCount);
      }
      pool = new BrainPool(options.workers, options.backend);
      await pool.init({
        spec: core.world.arch.spec,
        specKey: core.world.archKey,
        populationCount: core.world.population.length,
        paramCount: graph.totalCount,
        inputStride: core.world.arch.spec.nodes.find(node => node.type === 'Input')?.outputSize ?? 0,
        outputStride: core.world.arch.spec.outputSize,
        maxBatch: core.world.population.length,
        weights: packedWeights
      });
      core.brainPool = pool;
    }
    const installedBodyPoints = scenario.denseLongBodies ? installDenseLongBodies(core.world) : null;
    const installedWorldLoad = scenario.denseLongBodies
      ? captureStage2WorldLoad(core.world)
      : null;
    const expectedInstalledCollisionEntries = scenario.denseLongBodies
      ? expectedStage2CollisionEntries(core.world)
      : null;
    if (scenario.denseLongBodies) {
      if (
        installedBodyPoints === null ||
        installedBodyPoints <= P4_DENSE_COLLISION_ENTRY_THRESHOLD
      ) {
        throw new Error(
          `P4 installed ${String(installedBodyPoints)} body points; expected more than ` +
          `${P4_DENSE_COLLISION_ENTRY_THRESHOLD}`
        );
      }
      if (
        installedWorldLoad === null ||
        installedWorldLoad.collisionGrid.currentEntries !==
          expectedInstalledCollisionEntries
      ) {
        throw new Error(
          `P4 collision index admitted ${String(installedWorldLoad?.collisionGrid.currentEntries)} ` +
          `entries; expected ${String(expectedInstalledCollisionEntries)}`
        );
      }
      if (installedWorldLoad.collisionGrid.outOfBoundsEntries !== 0) {
        throw new Error(
          `P4 collision index rejected ${installedWorldLoad.collisionGrid.outOfBoundsEntries} ` +
          'out-of-bounds entries'
        );
      }
      if (installedWorldLoad.collisionGrid.faultReason !== null) {
        throw new Error(`P4 collision index faulted: ${installedWorldLoad.collisionGrid.faultReason}`);
      }
    }
    for (let step = 0; step < options.warmupSteps; step++) {
      await core.update(core.fixedDt);
      await yieldEventLoop();
    }

    const profiler = new SimProfiler({ enabled: true, reportIntervalMs: 3_600_000 });
    core.world.profiler = profiler;
    const eventLoop = monitorEventLoopDelay({ resolution: 10 });
    const stepMs: number[] = [];
    const sensorMs: number[] = [];
    const brainMs: number[] = [];
    const physicsMs: number[] = [];
    const framePackMs: number[] = [];
    const frameBytes: number[] = [];
    let serialPopulationEvaluations = 0;
    let pooledPopulationEvaluations = 0;
    const timeline: RuntimeTimelineSample[] = [];
    let denseThresholdLastStep: number | null = null;
    let p4CollisionFaultFree = true;
    let p4FirstOutOfBoundsMeasuredStep: number | null = null;
    let committedSteps = 0;
    const measuredStartLoad = scenario.denseLongBodies || options.sampleEverySteps > 0
      ? captureStage2WorldLoad(core.world)
      : null;
    const p4OutOfBoundsAtMeasuredStart = scenario.denseLongBodies
      ? measuredStartLoad?.collisionGrid.outOfBoundsEntries ?? null
      : null;
    if (
      scenario.denseLongBodies &&
      (measuredStartLoad?.collisionGrid.currentEntries ?? 0) >
        P4_DENSE_COLLISION_ENTRY_THRESHOLD
    ) {
      denseThresholdLastStep = 0;
    }
    const memoryBefore = process.memoryUsage();
    let peakRssBytes = memoryBefore.rss;
    let peakHeapUsedBytes = memoryBefore.heapUsed;
    let peakExternalBytes = memoryBefore.external;
    const cpuBefore = process.cpuUsage();
    const wallStarted = performance.now();
    eventLoop.enable();
    if (options.sampleEverySteps > 0) {
      if (measuredStartLoad === null) {
        throw new Error('timeline sampling requires a measured-start load snapshot');
      }
      timeline.push({
        measuredStep: 0,
        elapsedWallMs: 0,
        fixedStepMs: null,
        frameBytes: null,
        load: measuredStartLoad,
        rssBytes: memoryBefore.rss,
        heapUsedBytes: memoryBefore.heapUsed,
        externalBytes: memoryBefore.external
      });
    }
    for (let step = 0; step < options.measuredSteps; step++) {
      const started = performance.now();
      const committed = await core.update(core.fixedDt);
      const duration = performance.now() - started;
      if (committed !== 1) throw new Error(`Expected one committed step, received ${committed}`);
      committedSteps += committed;
      stepMs.push(duration);
      sensorMs.push(profiler.tickSensorsMs);
      brainMs.push(profiler.tickBrainMs);
      physicsMs.push(Math.max(0, duration - profiler.tickSensorsMs - profiler.tickBrainMs));
      serialPopulationEvaluations += core.world._serialControlCount;
      pooledPopulationEvaluations += core.world._controlBatch.count;
      const measuredStep = step + 1;
      let packedFrameBytes: number | null = null;
      if (options.frameEvery > 0 && measuredStep % options.frameEvery === 0) {
        const frameStarted = performance.now();
        const frame = core.serialize();
        framePackMs.push(performance.now() - frameStarted);
        frameBytes.push(frame.byteLength);
        packedFrameBytes = frame.byteLength;
      }
      const memory = process.memoryUsage();
      peakRssBytes = Math.max(peakRssBytes, memory.rss);
      peakHeapUsedBytes = Math.max(peakHeapUsedBytes, memory.heapUsed);
      peakExternalBytes = Math.max(peakExternalBytes, memory.external);
      if (scenario.denseLongBodies) {
        const stepCollision = core.world.getCollisionGridDiagnostics();
        p4CollisionFaultFree &&= stepCollision.faultReason === null;
        if (
          p4FirstOutOfBoundsMeasuredStep === null &&
          p4OutOfBoundsAtMeasuredStart !== null &&
          stepCollision.outOfBoundsEntries > p4OutOfBoundsAtMeasuredStart
        ) {
          p4FirstOutOfBoundsMeasuredStep = measuredStep;
        }
        if (stepCollision.currentEntries > P4_DENSE_COLLISION_ENTRY_THRESHOLD) {
          denseThresholdLastStep = measuredStep;
        }
      }
      if (
        options.sampleEverySteps > 0 &&
        (
          measuredStep === 1 ||
          measuredStep % options.sampleEverySteps === 0 ||
          measuredStep === options.measuredSteps
        )
      ) {
        const load = captureStage2WorldLoad(core.world);
        timeline.push({
          measuredStep,
          elapsedWallMs: Number((performance.now() - wallStarted).toFixed(6)),
          fixedStepMs: Number(duration.toFixed(6)),
          frameBytes: packedFrameBytes,
          load,
          rssBytes: memory.rss,
          heapUsedBytes: memory.heapUsed,
          externalBytes: memory.external
        });
      }
      await yieldEventLoop();
    }
    const measuredWallMs = performance.now() - wallStarted;
    const cpu = process.cpuUsage(cpuBefore);
    const memoryAfter = process.memoryUsage();
    eventLoop.disable();
    const graph = enrichArchInfo(core.world.arch);
    const nativeKernelNodesPerBrain = graph.nodes.filter(node => node.length > 0).length;
    const brainCallCount = profiler.windowBrainCalls;
    const populationEvaluations = serialPopulationEvaluations + pooledPopulationEvaluations;
    const collision = core.world.getCollisionGridDiagnostics();
    const p4OutOfBoundsAtMeasuredEnd = scenario.denseLongBodies
      ? collision.outOfBoundsEntries
      : null;
    const p4OutOfBoundsAccumulatedDuringMeasuredSteps =
      p4OutOfBoundsAtMeasuredStart !== null && p4OutOfBoundsAtMeasuredEnd !== null
        ? p4OutOfBoundsAtMeasuredEnd - p4OutOfBoundsAtMeasuredStart
        : null;
    const assertions = {
      allMeasuredStepsCommitted: committedSteps === options.measuredSteps,
      p4InstalledBodyThresholdExceeded: !scenario.denseLongBodies ||
        (installedBodyPoints ?? 0) > P4_DENSE_COLLISION_ENTRY_THRESHOLD,
      p4InstalledCollisionThresholdExceeded: !scenario.denseLongBodies ||
        (installedWorldLoad?.collisionGrid.currentEntries ?? 0) >
          P4_DENSE_COLLISION_ENTRY_THRESHOLD,
      p4InstalledCollisionEntriesExact: !scenario.denseLongBodies ||
        installedWorldLoad?.collisionGrid.currentEntries ===
          expectedInstalledCollisionEntries,
      p4InstalledOutOfBoundsEntriesZero: !scenario.denseLongBodies ||
        installedWorldLoad?.collisionGrid.outOfBoundsEntries === 0,
      p4CollisionFaultFree: !scenario.denseLongBodies || p4CollisionFaultFree
    };
    if (!Object.values(assertions).every(Boolean)) {
      throw new Error(`Stage 2 runtime assertions failed: ${JSON.stringify(assertions)}`);
    }
    return {
    schema: 'slither-stage2-runtime-baseline',
    version: RESULT_VERSION,
    evidenceClass: provenance.ownerTargetVmValidated
      ? 'new measured target-VM current-TypeScript result'
      : 'new measured development-machine current-TypeScript result',
    caveat: provenance.ownerTargetVmValidated
      ? 'Direct production SimCore/World path measured on the oxygen Ryzen 7 2700 Debian VM, including the canonical BrainPool when selected; not an end-to-end server, LAN, browser, RL trainer, or Rust-authoritative result.'
      : 'Direct production SimCore/World path, including the canonical BrainPool when selected; not an end-to-end server, LAN, browser, RL trainer, target-VM, or Rust-authoritative result.',
    source,
    environment: {
      capturedAt: new Date().toISOString(),
      platform: process.platform,
      architecture: process.arch,
      osType: os.type(),
      osRelease: os.release(),
      osVersion: os.version(),
      hostname: os.hostname(),
      provenance,
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
      sampleEverySteps: options.sampleEverySteps,
      command: [process.execPath, ...process.argv.slice(1)],
      processMode: options.workers > 0 ? 'canonical-node-worker-pool' : 'serial',
      requestedWorkers: options.workers,
      activeWorkers: pool?.getActiveWorkerCount() ?? 0,
      workerStatuses: pool?.getWorkerStatuses() ?? [],
      installedBodyPoints,
      installedWorldLoad,
      expectedInstalledCollisionEntries,
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
      serialPopulationEvaluations,
      pooledPopulationEvaluations,
      populationEvaluations,
      derivedNativeCrossings: options.backend === 'native'
        ? populationEvaluations * nativeKernelNodesPerBrain
        : 0,
      nativeKernelNodesPerBrain,
      collisionGrid: collision,
      denseWorldTimeline: {
        thresholdCollisionEntries: P4_DENSE_COLLISION_ENTRY_THRESHOLD,
        denseThresholdLastStep,
        denseThresholdMeaning: !scenario.denseLongBodies
          ? 'Not a P4 dense-world fixture.'
          : denseThresholdLastStep === null
            ? 'The collision index was never above the threshold during the measured interval.'
            : denseThresholdLastStep === 0
              ? 'The collision index was above the threshold only at the measured start and fell below it during the first completed step.'
              : `The last completed measured step above the threshold was ${denseThresholdLastStep}.`,
        outOfBounds: {
          atMeasuredStart: p4OutOfBoundsAtMeasuredStart,
          atMeasuredEnd: p4OutOfBoundsAtMeasuredEnd,
          accumulatedDuringMeasuredSteps: p4OutOfBoundsAccumulatedDuringMeasuredSteps,
          firstObservedMeasuredStep: p4FirstOutOfBoundsMeasuredStep,
          meaning: scenario.denseLongBodies
            ? 'The counter is cumulative rejected segment-midpoint insertions since collision-grid construction, not the number currently missing. Any measured increase means segments were omitted from the current TypeScript collision index without a capacity fault.'
            : 'Not a P4 dense-world fixture.'
        },
        samples: timeline,
        interpretation: scenario.denseLongBodies
          ? 'The fixture is installed once. The timeline reports when ordinary deaths reduce load; it never reinflates bodies or disables collisions to manufacture sustained density.'
          : 'Not a P4 dense-world fixture.'
      },
      assertions,
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
  } finally {
    await pool?.shutdown();
  }
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
