/** Measure the current per-brain graph paths on Stage 4 heterogeneous fixtures. */

import fs from 'node:fs';
import { createRequire } from 'node:module';
import os from 'node:os';
import path from 'node:path';
import { performance } from 'node:perf_hooks';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { spawnSync } from 'node:child_process';
import { computeNativeSourceIdentity } from '../../server/rustEngine/nativeSourceIdentity.ts';
import {
  getNativeAddonBuildIdentifier,
  prepareInferenceBackend
} from '../../src/brains/nativeBridge.ts';
import type { InferenceBackend } from '../../src/brains/types.ts';
import {
  buildStage4CurrentBrains,
  buildStage4InferenceFixture,
  initializeStage4BrainState,
  STAGE4_INFERENCE_FIXTURE_VERSION,
  stage4OneStepComparisonData,
  stage4ResultDigests,
  type Stage4InferenceScenarioName
} from './inferenceFixture.ts';

/** Version of the retained current-runtime inference evidence document. */
const RESULT_VERSION = 1;
/** Native crate used to reproduce the addon's selected-source identity. */
const NATIVE_DIRECTORY = fileURLToPath(new URL('../../native', import.meta.url));
/** Generated napi-rs loader for the same addon used by the current graph path. */
const NATIVE_LOADER = fileURLToPath(new URL('../../native/index.js', import.meta.url));
/** CommonJS loader scoped to this evidence module. */
const require = createRequire(import.meta.url);
/** Provenance declaration supplied by the benchmark operator. */
type EvidenceEnvironment = 'development' | 'owner-target-vm';
/** Current runtime path being measured. */
type CurrentInferencePath = 'js' | 'native';

/** Parsed command-line options. */
interface BenchmarkOptions {
  /** P0, P1, P2, or P3 workload. */
  scenario: Stage4InferenceScenarioName;
  /** Current TypeScript graph path or count-one native-kernel graph path. */
  path: CurrentInferencePath;
  /** Untimed complete population passes. */
  warmupPasses: number;
  /** Individually timed complete population passes. */
  measuredPasses: number;
  /** Required JSON output destination. */
  outputPath: string;
  /** Explicit environment provenance. */
  evidenceEnvironment: EvidenceEnvironment;
}

/** Quantile summary for complete heterogeneous-population pass times. */
interface Distribution {
  /** Number of observations. */
  count: number;
  /** Minimum milliseconds. */
  min: number;
  /** Median milliseconds. */
  p50: number;
  /** 95th percentile milliseconds. */
  p95: number;
  /** 99th percentile milliseconds. */
  p99: number;
  /** Maximum milliseconds. */
  max: number;
  /** Arithmetic mean milliseconds. */
  mean: number;
}

/** Production native identity retained beside count-one evidence. */
interface NativeEvidenceIdentity {
  /** Source-derived build identifier. */
  nativeAddonBuildIdentifier: string;
  /** Embedded selected-source SHA-256. */
  nativeAddonSourceSha256: string;
  /** Independently reproduced selected-source SHA-256. */
  currentSourceSha256: string;
  /** Exact Cargo target triple. */
  nativeAddonBuildTarget: string;
  /** Cargo profile. */
  nativeAddonBuildProfile: string;
  /** Production/test-hook build class. */
  nativeAddonBuildClass: string;
  /** Compiler identity. */
  nativeAddonRustcVersion: string;
  /** Effective correctness-build contract. */
  nativeAddonBuildContractSha256: string;
}

/** Required native identity exports loaded only for evidence validation. */
interface NativeIdentityExports {
  nativeAddonBuildIdentifier(): string;
  nativeAddonSourceSha256(): string;
  nativeAddonBuildTarget(): string;
  nativeAddonBuildProfile(): string;
  nativeAddonBuildClass(): string;
  nativeAddonRustcVersion(): string;
  nativeAddonBuildContractSha256(): string;
}

/**
 * Parse one bounded integer option.
 * @param value - Raw CLI token.
 * @param name - Option label.
 * @param allowZero - Whether zero is valid.
 * @returns Validated integer.
 */
function parseCount(value: string | undefined, name: string, allowZero: boolean): number {
  const parsed = Number.parseInt(value ?? '', 10);
  const minimum = allowZero ? 0 : 1;
  if (!Number.isSafeInteger(parsed) || parsed < minimum || parsed > 100_000) {
    throw new Error(`${name} must be an integer from ${minimum} to 100000.`);
  }
  return parsed;
}

/**
 * Parse the explicit evidence-runner command line.
 * @param argv - Tokens after the script filename.
 * @returns Complete benchmark options.
 */
export function parseStage4InferenceBenchmarkOptions(argv: readonly string[]): BenchmarkOptions {
  const options: Partial<BenchmarkOptions> = {
    warmupPasses: 10,
    measuredPasses: 60,
    evidenceEnvironment: 'development'
  };
  for (let index = 0; index < argv.length; index++) {
    const option = argv[index];
    const value = argv[index + 1];
    switch (option) {
      case '--scenario':
        if (value !== 'P0' && value !== 'P1' && value !== 'P2' && value !== 'P3') {
          throw new Error('--scenario must be P0, P1, P2, or P3.');
        }
        options.scenario = value;
        index++;
        break;
      case '--path':
        if (value !== 'js' && value !== 'native') {
          throw new Error('--path must be js or native.');
        }
        options.path = value;
        index++;
        break;
      case '--warmup-passes':
        options.warmupPasses = parseCount(value, option, true);
        index++;
        break;
      case '--passes':
        options.measuredPasses = parseCount(value, option, false);
        index++;
        break;
      case '--output':
        if (!value) throw new Error('--output requires a path.');
        options.outputPath = path.resolve(value);
        index++;
        break;
      case '--environment':
        if (value !== 'development' && value !== 'owner-target-vm') {
          throw new Error('--environment must be development or owner-target-vm.');
        }
        options.evidenceEnvironment = value;
        index++;
        break;
      default:
        throw new Error(`Unknown option ${option ?? '<missing>'}.`);
    }
  }
  if (!options.scenario || !options.path || !options.outputPath) {
    throw new Error('--scenario, --path, and --output are required.');
  }
  return options as BenchmarkOptions;
}

/**
 * Summarize finite timing observations with interpolated percentiles.
 * @param values - Millisecond samples.
 * @returns Rounded distribution.
 */
function distribution(values: readonly number[]): Distribution {
  if (values.length === 0 || values.some(value => !Number.isFinite(value) || value < 0)) {
    throw new Error('Timing distribution requires finite nonnegative samples.');
  }
  const sorted = [...values].sort((left, right) => left - right);
  const percentile = (fraction: number): number => {
    const position = (sorted.length - 1) * fraction;
    const lower = Math.floor(position);
    const upper = Math.ceil(position);
    const lowerValue = sorted[lower]!;
    const upperValue = sorted[upper]!;
    return lowerValue + (upperValue - lowerValue) * (position - lower);
  };
  const round = (value: number): number => Number(value.toFixed(6));
  return {
    count: sorted.length,
    min: round(sorted[0]!),
    p50: round(percentile(0.5)),
    p95: round(percentile(0.95)),
    p99: round(percentile(0.99)),
    max: round(sorted[sorted.length - 1]!),
    mean: round(sorted.reduce((sum, value) => sum + value, 0) / sorted.length)
  };
}

/**
 * Read commit and dirtiness without taking optional Git locks.
 * @returns Exact source identity visible to the runner.
 */
function sourceIdentity(): { commit: string; dirty: boolean } {
  const environment = { ...process.env, GIT_OPTIONAL_LOCKS: '0' };
  const commit = spawnSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8', env: environment });
  const status = spawnSync('git', ['status', '--porcelain'], { encoding: 'utf8', env: environment });
  if (commit.status !== 0 || status.status !== 0) {
    throw new Error('Unable to resolve Git source identity for Stage 4 evidence.');
  }
  return { commit: commit.stdout.trim(), dirty: status.stdout.trim().length !== 0 };
}

/**
 * Fail closed when the count-one benchmark addon is stale or not a release production build.
 * @returns Exact addon and independently reproduced source identity.
 */
function validateNativeEvidenceIdentity(): NativeEvidenceIdentity {
  const loaded = require(NATIVE_LOADER) as unknown;
  if (typeof loaded !== 'object' || loaded === null) {
    throw new TypeError('Native evidence loader did not export an object.');
  }
  const exports = loaded as Record<string, unknown>;
  const names = [
    'nativeAddonBuildIdentifier',
    'nativeAddonSourceSha256',
    'nativeAddonBuildTarget',
    'nativeAddonBuildProfile',
    'nativeAddonBuildClass',
    'nativeAddonRustcVersion',
    'nativeAddonBuildContractSha256'
  ] as const;
  for (const name of names) {
    if (typeof exports[name] !== 'function') {
      throw new TypeError(`Native evidence addon is missing ${name}().`);
    }
  }
  const native = exports as unknown as NativeIdentityExports;
  const currentSourceSha256 = computeNativeSourceIdentity(NATIVE_DIRECTORY).sha256;
  const nativeAddonSourceSha256 = native.nativeAddonSourceSha256();
  const nativeAddonBuildIdentifier = native.nativeAddonBuildIdentifier();
  const nativeAddonBuildTarget = native.nativeAddonBuildTarget();
  const nativeAddonBuildProfile = native.nativeAddonBuildProfile();
  const nativeAddonBuildClass = native.nativeAddonBuildClass();
  const nativeAddonRustcVersion = native.nativeAddonRustcVersion();
  const nativeAddonBuildContractSha256 = native.nativeAddonBuildContractSha256();
  if (nativeAddonSourceSha256 !== currentSourceSha256) {
    throw new Error(
      `Native evidence addon is stale: addon=${nativeAddonSourceSha256}, tree=${currentSourceSha256}.`
    );
  }
  if (nativeAddonBuildIdentifier !== getNativeAddonBuildIdentifier()) {
    throw new Error('The evidence identity loader and current GraphBrain loaded different addons.');
  }
  if (nativeAddonBuildProfile !== 'release' || nativeAddonBuildClass !== 'production') {
    throw new Error(
      `Native evidence requires a release production addon; got ${nativeAddonBuildProfile}/${nativeAddonBuildClass}.`
    );
  }
  if (!/^x86_64-(?:pc-windows-msvc|unknown-linux-gnu)$/u.test(nativeAddonBuildTarget)) {
    throw new Error(`Native evidence addon has unsupported target ${nativeAddonBuildTarget}.`);
  }
  if (!/^rustc\s+\S+/u.test(nativeAddonRustcVersion)) {
    throw new Error('Native evidence addon returned an invalid Rust compiler identity.');
  }
  if (!/^sha256:[0-9a-f]{64}$/u.test(nativeAddonBuildContractSha256)) {
    throw new Error('Native evidence addon returned an invalid build-contract identity.');
  }
  return {
    nativeAddonBuildIdentifier,
    nativeAddonSourceSha256,
    currentSourceSha256,
    nativeAddonBuildTarget,
    nativeAddonBuildProfile,
    nativeAddonBuildClass,
    nativeAddonRustcVersion,
    nativeAddonBuildContractSha256
  };
}

/**
 * Read Debian identity where available.
 * @returns Parsed operating-system release fields.
 */
function linuxRelease(): Record<string, string> {
  if (process.platform !== 'linux') return {};
  try {
    return Object.fromEntries(
      fs.readFileSync('/etc/os-release', 'utf8')
        .split(/\r?\n/u)
        .filter(line => line.includes('='))
        .map(line => {
          const separator = line.indexOf('=');
          return [line.slice(0, separator), line.slice(separator + 1).replace(/^"|"$/gu, '')];
        })
    );
  } catch {
    return {};
  }
}

/**
 * Capture enough machine data to distinguish development from Oxygen evidence.
 * @param declaration - Operator-declared provenance.
 * @returns Environment facts and target-VM checks.
 */
function environmentEvidence(declaration: EvidenceEnvironment): Record<string, unknown> {
  const cpus = os.cpus();
  const cpuModel = cpus[0]?.model.trim() ?? 'unknown';
  const release = linuxRelease();
  const hostname = os.hostname();
  const ownerTargetVmValidated = declaration === 'owner-target-vm'
    && process.platform === 'linux'
    && release.ID === 'debian'
    && hostname === 'oxygen'
    && cpuModel.includes('AMD Ryzen 7 2700')
    && cpus.length === 8
    && os.totalmem() >= 15 * 1024 ** 3
    && os.totalmem() <= 17 * 1024 ** 3;
  return {
    capturedAt: new Date().toISOString(),
    platform: process.platform,
    architecture: process.arch,
    osType: os.type(),
    osRelease: os.release(),
    osVersion: os.version(),
    distributionId: release.ID ?? null,
    hostname,
    node: process.version,
    v8: process.versions.v8,
    cpuModel,
    logicalCpuCount: cpus.length,
    totalMemoryBytes: os.totalmem(),
    ownerTargetVmValidated,
    provenance: {
      declaration,
      ownerTargetVmValidated
    }
  };
}

/**
 * Execute one complete current graph pass and consume every output.
 * @param brains - Differently weighted current GraphBrains.
 * @param observations - One distinct observation per brain.
 * @returns Finite output accumulator preventing dead work.
 */
function executeCurrentPass(
  brains: ReturnType<typeof buildStage4CurrentBrains>,
  observations: readonly Float32Array[]
): number {
  let accumulator = 0;
  for (let index = 0; index < brains.length; index++) {
    const output = brains[index]!.forward(observations[index]!);
    for (const value of output) accumulator += value;
  }
  if (!Number.isFinite(accumulator)) {
    throw new Error('Current graph path produced a non-finite population output.');
  }
  return accumulator;
}

/**
 * Run one complete current-runtime comparison path.
 * @param options - Validated options.
 * @returns Machine-readable evidence object.
 */
export async function runStage4CurrentInferenceBenchmark(
  options: BenchmarkOptions
): Promise<Record<string, unknown>> {
  const backend: InferenceBackend = options.path;
  await prepareInferenceBackend(backend);
  const nativeIdentity = backend === 'native' ? validateNativeEvidenceIdentity() : null;
  const environment = environmentEvidence(options.evidenceEnvironment);
  if (
    options.evidenceEnvironment === 'owner-target-vm'
    && environment.ownerTargetVmValidated !== true
  ) {
    throw new Error(
      'owner-target-vm was declared, but the Debian/Oxygen/Ryzen-2700/8-thread/16-GiB identity checks did not all pass.'
    );
  }
  const memoryBeforeFixture = process.memoryUsage();
  const fixture = buildStage4InferenceFixture(options.scenario);
  const brains = buildStage4CurrentBrains(fixture, backend);
  const memoryAfterFixture = process.memoryUsage();
  const nativeNodesPerBrain = fixture.compiled.nodes.filter(node => node.paramLength > 0).length;
  const oneStepConsumedOutput = executeCurrentPass(brains, fixture.observations);
  const oneStepComparison = stage4OneStepComparisonData(brains);
  for (let slot = 0; slot < brains.length; slot++) {
    initializeStage4BrainState(brains[slot]!, fixture.scenario, slot);
  }
  let consumedOutput = 0;

  for (let pass = 0; pass < options.warmupPasses; pass++) {
    consumedOutput += executeCurrentPass(brains, fixture.observations);
  }
  const memoryAfterWarmup = process.memoryUsage();
  const cpuBefore = process.cpuUsage();
  const samples: number[] = [];
  let peakRssBytes = memoryAfterWarmup.rss;
  let peakHeapUsedBytes = memoryAfterWarmup.heapUsed;
  let peakExternalBytes = memoryAfterWarmup.external;
  for (let pass = 0; pass < options.measuredPasses; pass++) {
    const started = performance.now();
    consumedOutput += executeCurrentPass(brains, fixture.observations);
    samples.push(performance.now() - started);
    const memory = process.memoryUsage();
    peakRssBytes = Math.max(peakRssBytes, memory.rss);
    peakHeapUsedBytes = Math.max(peakHeapUsedBytes, memory.heapUsed);
    peakExternalBytes = Math.max(peakExternalBytes, memory.external);
  }
  const cpu = process.cpuUsage(cpuBefore);
  const memoryAfterMeasure = process.memoryUsage();
  const digests = stage4ResultDigests(brains);
  if (digests.distinctOutputPairs < 2) {
    throw new Error('Heterogeneous fixture failed to produce at least two distinct outputs.');
  }
  if (digests.recurrentSha256 === fixture.initialRecurrentSha256) {
    throw new Error('Recurrent state did not advance during the benchmark.');
  }

  return {
    schema: 'slither-stage4-current-inference-benchmark',
    version: RESULT_VERSION,
    evidenceClass: options.evidenceEnvironment === 'owner-target-vm'
      ? 'new measured target-VM current-runtime result'
      : 'new measured development-machine current-runtime result',
    caveat: 'Source-shaped synthetic inference-only microbenchmark. It excludes actual fresh/evolved genomes and sensor observations, sensing, physics, frames, Node workers, the Rust-authoritative coordinator, and end-to-end server latency; it cannot by itself satisfy the Stage 4 production-workload gate.',
    source: sourceIdentity(),
    environment,
    workload: {
      fixtureVersion: STAGE4_INFERENCE_FIXTURE_VERSION,
      fixtureClass: 'source-shaped deterministic synthetic numeric data',
      scenario: fixture.scenario,
      graphKey: fixture.compiled.key,
      totalParametersPerBrain: fixture.compiled.totalParams,
      recurrentFloatsPerBrain: fixture.compiled.totalStateSize,
      outputFloatsPerBrain: fixture.compiled.outputSize,
      weightsSha256: fixture.weightsSha256,
      observationsSha256: fixture.observationsSha256,
      initialRecurrentSha256: fixture.initialRecurrentSha256,
      differentlyWeightedBrains: true,
      distinctObservations: true,
      nonzeroRecurrentState: fixture.compiled.totalStateSize > 0,
      actualFreshOrEvolvedGenomes: false,
      actualDeliveredSensorObservations: false,
      warmupPasses: options.warmupPasses,
      measuredPasses: options.measuredPasses
    },
    path: {
      name: backend === 'native'
        ? 'current-typescript-count-one-native'
        : 'current-typescript-js-graph',
      graphOwner: 'TypeScript',
      nativeIdentity,
      nativeKernelNodesPerBrain: backend === 'native' ? nativeNodesPerBrain : 0,
      nativeCallsPerWholePass: backend === 'native'
        ? nativeNodesPerBrain * fixture.scenario.populationCount
        : 0,
      sharedWeightBatch: false
    },
    result: {
      wholePopulationPassMs: distribution(samples),
      oneStepComparisonProbe: {
        ...oneStepComparison,
        consumedOutput: Number(oneStepConsumedOutput.toFixed(9)),
        scope: 'raw complete-population Float32 result from the shared initial state; excluded from timed samples and reset before warmup; compare element by element rather than comparing rounded hashes'
      },
      consumedOutput: Number(consumedOutput.toFixed(9)),
      outputsSha256: digests.outputsSha256,
      finalRecurrentSha256: digests.recurrentSha256,
      distinctOutputPairs: digests.distinctOutputPairs,
      cpu: {
        userMicros: cpu.user,
        systemMicros: cpu.system,
        totalMicros: cpu.user + cpu.system
      },
      memory: {
        beforeFixture: memoryBeforeFixture,
        afterFixture: memoryAfterFixture,
        afterWarmup: memoryAfterWarmup,
        afterMeasure: memoryAfterMeasure,
        sampledPeakRssBytes: peakRssBytes,
        sampledPeakHeapUsedBytes: peakHeapUsedBytes,
        sampledPeakExternalBytes: peakExternalBytes,
        processResourceUsageMaxRssKiB: process.resourceUsage().maxRSS
      }
    },
    command: process.argv
  };
}

/**
 * Write one complete evidence file after successful measurement.
 * @param outputPath - Destination path.
 * @param report - Serializable evidence.
 */
function writeReport(outputPath: string, report: Record<string, unknown>): void {
  fs.mkdirSync(path.dirname(outputPath), { recursive: true });
  fs.writeFileSync(outputPath, `${JSON.stringify(report, null, 2)}\n`, 'utf8');
}

/** Execute the CLI only when this module is the process entry point. */
if (process.argv[1] && pathToFileURL(path.resolve(process.argv[1])).href === import.meta.url) {
  const options = parseStage4InferenceBenchmarkOptions(process.argv.slice(2));
  const report = await runStage4CurrentInferenceBenchmark(options);
  writeReport(options.outputPath, report);
  process.stdout.write(`wrote ${options.scenario}/${options.path} evidence to ${options.outputPath}\n`);
}
