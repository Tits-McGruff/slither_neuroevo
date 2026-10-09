/** Compare equivalent Stage 4 one-step inference results element by element. */

import { createHash } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

/** Stable comparison artifact version. */
const COMPARISON_VERSION = 2;

/** Parsed comparator command line. */
export interface ComparatorOptions {
  /** Runtime-selected Rust SSE2 report. */
  rustPath: string;
  /** Explicit Rust scalar reference report. */
  scalarRustPath: string;
  /** Current TypeScript/JavaScript graph report. */
  jsPath: string;
  /** Current TypeScript/count-one-native report. */
  nativePath: string;
  /** Small comparison report destination. */
  outputPath: string;
}

/** Raw one-step values emitted outside timed samples. */
interface RawProbe {
  /** Existing absolute native-backend tolerance. */
  absoluteTolerance: number;
  /** Controller outputs as raw little-endian Float32 bytes in hexadecimal. */
  outputsF32LeHex: string;
  /** Recurrent state as raw little-endian Float32 bytes in hexadecimal. */
  recurrentF32LeHex: string;
  /** Number of controller-output floats. */
  outputFloats: number;
  /** Number of recurrent-state floats. */
  recurrentFloats: number;
  /** Exact raw controller-output digest. */
  outputsSha256: string;
  /** Exact raw recurrent-state digest. */
  recurrentSha256: string;
}

/** Native build fields required to prove a production count-one addon. */
interface NativeIdentityProof {
  /** Embedded native selected-source SHA-256. */
  nativeAddonSourceSha256: string;
  /** Independently reproduced native selected-source SHA-256. */
  currentSourceSha256: string;
  /** Cargo profile embedded by the addon. */
  nativeAddonBuildProfile: string;
  /** Production or test-hook build class. */
  nativeAddonBuildClass: string;
  /** Exact supported compilation target. */
  nativeAddonBuildTarget: string;
  /** Other validated identity fields retained by the runner. */
  [key: string]: unknown;
}

/** Minimal common view of Rust and current-runtime benchmark reports. */
interface InferenceReport {
  /** Evidence schema family. */
  schema: string;
  /** Rust build fields; current-runtime reports instead use Git identity here. */
  source?: {
    /** Git commit for current-runtime reports. */
    commit?: string;
    /** Whether the current-runtime worktree contained uncommitted changes. */
    dirty?: boolean;
    /** Selected native-source digest for standalone Rust reports. */
    nativeSourceSha256?: string;
    /** Cargo profile for the standalone Rust runner. */
    buildProfile?: string;
    /** Test-hook build class required by the standalone runner. */
    buildClass?: string;
    /** Exact supported compilation target. */
    targetTriple?: string;
    /** Other source fields differ across report families. */
    [key: string]: unknown;
  };
  /** Machine facts emitted in Rust or current-runtime naming. */
  environment?: {
    declaration?: string;
    provenance?: { declaration?: string };
    operatingSystem?: string;
    platform?: string;
    architecture?: string;
    hostname?: string | null;
    availableParallelism?: number | null;
    logicalCpuCount?: number;
    ownerTargetVmValidated?: boolean;
  };
  /** Workload identity shared by all paths. */
  workload: {
    /** Rust string label or TypeScript scenario object. */
    scenario: string | { name: string };
    /** Packed population-weight identity. */
    weightsSha256: string;
    /** Observation identity. */
    observationsSha256: string;
    /** Initial recurrent-state identity. */
    initialRecurrentSha256: string;
    /** Untimed complete-population passes. */
    warmupPasses: number;
    /** Timed complete-population passes. */
    measuredPasses: number;
  };
  /** Execution-path identity that must match the report's assigned role. */
  path: {
    /** Stable current or Rust path label. */
    name: string;
    /** Number of N-API calls inside one complete population pass. */
    nativeCallsPerWholePass: number;
    /** Stable Rust numeric-backend label. */
    mathBackend?: string;
    /** Runtime feature-admission result for a Rust numeric backend. */
    runtimeFeatureAvailable?: boolean;
    /** Production addon identity, present only for the count-one native path. */
    nativeIdentity?: NativeIdentityProof | null;
  };
  /** Results containing the raw comparison probe. */
  result: {
    /** One complete pass from the common initial state. */
    oneStepComparisonProbe: RawProbe;
  };
}

/** Required semantic role for one comparator input. */
type ReportRole = 'rust' | 'scalarRust' | 'js' | 'native';

/** Loaded report plus immutable file identity. */
interface LoadedReport {
  /** Source filename supplied to the comparator. */
  path: string;
  /** SHA-256 of the complete source report. */
  reportSha256: string;
  /** Parsed report. */
  report: InferenceReport;
  /** Validated controller-output bytes. */
  outputs: Buffer;
  /** Validated recurrent-state bytes. */
  recurrent: Buffer;
}

/** Numeric error summary for one pair of equally shaped vectors. */
export interface FloatComparison {
  /** Compared Float32 element count. */
  count: number;
  /** Selected absolute acceptance threshold. */
  absoluteTolerance: number;
  /** Largest observed absolute difference. */
  maxAbsoluteDifference: number;
  /** Index of the largest observed difference. */
  maxDifferenceIndex: number;
  /** Left value at the largest difference. */
  leftValueAtMax: number;
  /** Right value at the largest difference. */
  rightValueAtMax: number;
  /** Number of elements exceeding the threshold. */
  outsideTolerance: number;
  /** First failing index, or null when every value passes. */
  firstFailureIndex: number | null;
}

/** Parse required explicit report paths. */
function parseOptions(argv: readonly string[]): ComparatorOptions {
  const values: Partial<ComparatorOptions> = {};
  for (let index = 0; index < argv.length; index += 2) {
    const option = argv[index];
    const value = argv[index + 1];
    if (!value) throw new Error(`${option ?? '<missing option>'} requires a value.`);
    switch (option) {
      case '--rust': values.rustPath = path.resolve(value); break;
      case '--scalar-rust': values.scalarRustPath = path.resolve(value); break;
      case '--js': values.jsPath = path.resolve(value); break;
      case '--native': values.nativePath = path.resolve(value); break;
      case '--output': values.outputPath = path.resolve(value); break;
      default: throw new Error(`Unknown option ${option ?? '<missing>'}.`);
    }
  }
  if (
    !values.rustPath
    || !values.scalarRustPath
    || !values.jsPath
    || !values.nativePath
    || !values.outputPath
  ) {
    throw new Error('--rust, --scalar-rust, --js, --native, and --output are required.');
  }
  return values as ComparatorOptions;
}

/** Return the common P0-P3 label from either report representation. */
function scenarioName(report: InferenceReport): string {
  const scenario = report.workload.scenario;
  return typeof scenario === 'string' ? scenario : scenario.name;
}

/** Validate and decode one raw little-endian Float32 hexadecimal field. */
function decodeProbeHex(hex: string, count: number, digest: string, label: string): Buffer {
  if (!Number.isSafeInteger(count) || count < 0) throw new Error(`${label} count is invalid.`);
  if (!/^(?:[0-9a-f]{2})*$/.test(hex)) throw new Error(`${label} is not lowercase byte hexadecimal.`);
  const bytes = Buffer.from(hex, 'hex');
  if (bytes.length !== count * 4) {
    throw new Error(`${label} has ${bytes.length} bytes; expected ${count * 4}.`);
  }
  const actualDigest = createHash('sha256').update(bytes).digest('hex');
  if (actualDigest !== digest) throw new Error(`${label} SHA-256 does not match its report.`);
  return bytes;
}

/** Load and validate one benchmark report and its raw comparison vectors. */
function loadReport(reportPath: string, role: ReportRole): LoadedReport {
  const bytes = fs.readFileSync(reportPath);
  const report = JSON.parse(bytes.toString('utf8')) as InferenceReport;
  validateReportRole(report, role, reportPath);
  const probe = report.result?.oneStepComparisonProbe;
  if (!probe) throw new Error(`${reportPath} has no oneStepComparisonProbe.`);
  return {
    path: reportPath,
    reportSha256: createHash('sha256').update(bytes).digest('hex'),
    report,
    outputs: decodeProbeHex(
      probe.outputsF32LeHex,
      probe.outputFloats,
      probe.outputsSha256,
      `${reportPath} outputs`
    ),
    recurrent: decodeProbeHex(
      probe.recurrentF32LeHex,
      probe.recurrentFloats,
      probe.recurrentSha256,
      `${reportPath} recurrent state`
    )
  };
}

/** Fail closed when a report does not prove the execution path assigned by its CLI role. */
function validateReportRole(report: InferenceReport, role: ReportRole, reportPath: string): void {
  const nativeCalls = report.path?.nativeCallsPerWholePass;
  const nativeIdentityPresent = typeof report.path?.nativeIdentity === 'object'
    && report.path.nativeIdentity !== null;
  if (role === 'rust' || role === 'scalarRust') {
    const expectedPath = role === 'rust'
      ? 'rust-sse2-coarse-heterogeneous'
      : 'rust-scalar-coarse-heterogeneous';
    const expectedBackend = role === 'rust' ? 'rust-sse2-v1' : 'rust-scalar-v1';
    if (
      report.schema !== 'slither-stage4-rust-inference-benchmark'
      || report.path?.name !== expectedPath
      || report.path?.mathBackend !== expectedBackend
      || report.path?.runtimeFeatureAvailable !== true
      || nativeCalls !== 0
      || nativeIdentityPresent
      || report.source?.buildProfile !== 'release'
      || report.source?.buildClass !== 'test-hooks'
      || !isSupportedTarget(report.source?.targetTriple)
      || !isSha256(report.source?.nativeSourceSha256)
    ) {
      throw new Error(`${reportPath} does not prove the ${expectedPath} path.`);
    }
    return;
  }
  if (report.schema !== 'slither-stage4-current-inference-benchmark') {
    throw new Error(`${reportPath} is not a current-runtime inference report.`);
  }
  if (role === 'js') {
    if (
      report.path?.name !== 'current-typescript-js-graph'
      || nativeCalls !== 0
      || nativeIdentityPresent
      || !isCleanGitSource(report.source)
    ) {
      throw new Error(`${reportPath} does not prove the current TypeScript/JavaScript path.`);
    }
    return;
  }
  if (
    report.path?.name !== 'current-typescript-count-one-native'
    || !Number.isSafeInteger(nativeCalls)
    || nativeCalls <= 0
    || !nativeIdentityPresent
    || !isCleanGitSource(report.source)
    || !isSha256(report.path.nativeIdentity?.nativeAddonSourceSha256)
    || report.path.nativeIdentity?.currentSourceSha256
      !== report.path.nativeIdentity?.nativeAddonSourceSha256
    || report.path.nativeIdentity?.nativeAddonBuildProfile !== 'release'
    || report.path.nativeIdentity?.nativeAddonBuildClass !== 'production'
    || !isSupportedTarget(report.path.nativeIdentity?.nativeAddonBuildTarget)
  ) {
    throw new Error(`${reportPath} does not prove the current count-one native path.`);
  }
}

/** Return whether a report proves one exact clean Git revision. */
function isCleanGitSource(source: InferenceReport['source']): boolean {
  return typeof source?.commit === 'string'
    && /^[0-9a-f]{40}$/u.test(source.commit)
    && source.dirty === false;
}

/** Return whether a value is one lowercase SHA-256 digest. */
function isSha256(value: unknown): value is string {
  return typeof value === 'string' && /^[0-9a-f]{64}$/u.test(value);
}

/** Return whether one exact target is supported by the approved native runtime. */
function isSupportedTarget(target: string | undefined): boolean {
  return target === 'x86_64-pc-windows-msvc' || target === 'x86_64-unknown-linux-gnu';
}

/**
 * Compare equally shaped raw little-endian Float32 vectors with an absolute tolerance.
 * @param left - First raw vector.
 * @param right - Second raw vector.
 * @param absoluteTolerance - Maximum accepted absolute error.
 * @returns Detailed finite error summary.
 */
export function compareFloat32Buffers(
  left: Buffer,
  right: Buffer,
  absoluteTolerance: number
): FloatComparison {
  if (left.length !== right.length || left.length % 4 !== 0) {
    throw new Error('Float32 comparison buffers must have equal four-byte-aligned lengths.');
  }
  if (!Number.isFinite(absoluteTolerance) || absoluteTolerance < 0) {
    throw new Error('Absolute tolerance must be finite and nonnegative.');
  }
  let maxAbsoluteDifference = -1;
  let maxDifferenceIndex = 0;
  let leftValueAtMax = 0;
  let rightValueAtMax = 0;
  let outsideTolerance = 0;
  let firstFailureIndex: number | null = null;
  const count = left.length / 4;
  for (let index = 0; index < count; index++) {
    const leftValue = left.readFloatLE(index * 4);
    const rightValue = right.readFloatLE(index * 4);
    if (!Number.isFinite(leftValue) || !Number.isFinite(rightValue)) {
      throw new Error(`Non-finite comparison value at index ${index}.`);
    }
    const difference = Math.abs(leftValue - rightValue);
    if (difference > maxAbsoluteDifference) {
      maxAbsoluteDifference = difference;
      maxDifferenceIndex = index;
      leftValueAtMax = leftValue;
      rightValueAtMax = rightValue;
    }
    if (difference > absoluteTolerance) {
      outsideTolerance++;
      firstFailureIndex ??= index;
    }
  }
  return {
    count,
    absoluteTolerance,
    maxAbsoluteDifference: count === 0 ? 0 : maxAbsoluteDifference,
    maxDifferenceIndex,
    leftValueAtMax,
    rightValueAtMax,
    outsideTolerance,
    firstFailureIndex
  };
}

/** Verify fixture and tolerance identity across all input reports. */
function validateSharedIdentity(reports: readonly LoadedReport[]): number {
  const first = reports[0]!;
  const expected = first.report.workload;
  const tolerance = first.report.result.oneStepComparisonProbe.absoluteTolerance;
  for (const candidate of reports.slice(1)) {
    const workload = candidate.report.workload;
    if (
      scenarioName(candidate.report) !== scenarioName(first.report)
      || workload.weightsSha256 !== expected.weightsSha256
      || workload.observationsSha256 !== expected.observationsSha256
      || workload.initialRecurrentSha256 !== expected.initialRecurrentSha256
      || workload.warmupPasses !== expected.warmupPasses
      || workload.measuredPasses !== expected.measuredPasses
    ) {
      throw new Error('Inference reports do not describe the same deterministic input fixture.');
    }
    if (candidate.report.result.oneStepComparisonProbe.absoluteTolerance !== tolerance) {
      throw new Error('Inference reports do not declare the same absolute tolerance.');
    }
  }
  if (tolerance !== 1e-4) throw new Error(`Unexpected Stage 4 comparison tolerance ${tolerance}.`);
  return tolerance;
}

/** Normalize common machine facts across the Rust and current-runtime schemas. */
function normalizedEnvironment(report: InferenceReport): Record<string, unknown> {
  const environment = report.environment;
  const rawPlatform = environment?.operatingSystem ?? environment?.platform;
  const rawArchitecture = environment?.architecture;
  const hostname = environment?.hostname;
  return {
    declaration: environment?.declaration ?? environment?.provenance?.declaration,
    platform: rawPlatform === 'windows' ? 'win32' : rawPlatform,
    architecture: rawArchitecture === 'x86_64' ? 'x64' : rawArchitecture,
    hostname: typeof hostname === 'string' ? hostname.toLowerCase() : hostname,
    logicalCpuCount: environment?.availableParallelism ?? environment?.logicalCpuCount,
    ownerTargetVmValidated: environment?.ownerTargetVmValidated
  };
}

/** Require all four timing reports to describe the same measured host. */
function validateSharedEnvironment(reports: readonly LoadedReport[]): Record<string, unknown> {
  const expected = normalizedEnvironment(reports[0]!.report);
  const encoded = JSON.stringify(expected);
  if (Object.values(expected).some(value => value === undefined || value === null)) {
    throw new Error('Inference report is missing required common host identity fields.');
  }
  for (const candidate of reports.slice(1)) {
    if (JSON.stringify(normalizedEnvironment(candidate.report)) !== encoded) {
      throw new Error('Inference reports do not describe the same measured host environment.');
    }
  }
  return expected;
}

/** Require every compared implementation to belong to one exact clean source set. */
function validateSharedSourceIdentity(
  rust: LoadedReport,
  scalarRust: LoadedReport,
  js: LoadedReport,
  native: LoadedReport
): Record<string, string> {
  const rustSha = rust.report.source?.nativeSourceSha256;
  const scalarSha = scalarRust.report.source?.nativeSourceSha256;
  const nativeSha = native.report.path.nativeIdentity?.nativeAddonSourceSha256;
  if (!isSha256(rustSha) || rustSha !== scalarSha || rustSha !== nativeSha) {
    throw new Error('Rust SSE2, Rust scalar, and count-one native reports do not share one native source SHA-256.');
  }
  const targetTriple = rust.report.source?.targetTriple;
  if (!isSupportedTarget(targetTriple)
    || targetTriple !== scalarRust.report.source?.targetTriple
    || targetTriple !== native.report.path.nativeIdentity?.nativeAddonBuildTarget) {
    throw new Error('Rust SSE2, Rust scalar, and count-one native reports do not share one target triple.');
  }
  const jsCommit = js.report.source?.commit;
  const nativeCommit = native.report.source?.commit;
  if (!isCleanGitSource(js.report.source)
    || !isCleanGitSource(native.report.source)
    || jsCommit !== nativeCommit) {
    throw new Error('Current JavaScript and count-one native reports do not share one clean Git commit.');
  }
  return {
    nativeSourceSha256: rustSha,
    currentRuntimeCommit: jsCommit,
    targetTriple
  };
}

/** Build, validate, and optionally persist one four-path comparison report. */
export function runComparison(options: ComparatorOptions): Record<string, unknown> {
  const rust = loadReport(options.rustPath, 'rust');
  const scalarRust = loadReport(options.scalarRustPath, 'scalarRust');
  const js = loadReport(options.jsPath, 'js');
  const native = loadReport(options.nativePath, 'native');
  const reports = [rust, scalarRust, js, native] as const;
  const tolerance = validateSharedIdentity(reports);
  const sourceIdentity = validateSharedSourceIdentity(rust, scalarRust, js, native);
  const environmentIdentity = validateSharedEnvironment(reports);
  const compare = (left: LoadedReport, right: LoadedReport) => ({
    outputs: compareFloat32Buffers(left.outputs, right.outputs, tolerance),
    recurrent: compareFloat32Buffers(left.recurrent, right.recurrent, tolerance)
  });
  const comparisons = {
    rustSse2VsRustScalar: compare(rust, scalarRust),
    rustSse2VsJs: compare(rust, js),
    rustSse2VsCountOneNative: compare(rust, native),
    rustScalarVsJs: compare(scalarRust, js),
    rustScalarVsCountOneNative: compare(scalarRust, native),
    jsVsCountOneNative: compare(js, native)
  };
  const failureCount = Object.values(comparisons).reduce(
    (total, result) => total + result.outputs.outsideTolerance + result.recurrent.outsideTolerance,
    0
  );
  if (failureCount !== 0) {
    throw new Error(`${failureCount} one-step values exceeded the ${tolerance} absolute tolerance.`);
  }
  return {
    schema: 'slither-stage4-inference-cross-path-comparison',
    version: COMPARISON_VERSION,
    scenario: scenarioName(rust.report),
    fixture: rust.report.workload,
    sourceIdentity,
    environmentIdentity,
    sources: {
      rustSse2: { file: path.basename(rust.path), sha256: rust.reportSha256 },
      rustScalar: { file: path.basename(scalarRust.path), sha256: scalarRust.reportSha256 },
      currentJs: { file: path.basename(js.path), sha256: js.reportSha256 },
      currentCountOneNative: { file: path.basename(native.path), sha256: native.reportSha256 }
    },
    comparisons,
    command: process.argv
  };
}

/** Write one comparison artifact only after every numeric assertion passes. */
function writeComparison(outputPath: string, report: Record<string, unknown>): void {
  fs.mkdirSync(path.dirname(outputPath), { recursive: true });
  fs.writeFileSync(outputPath, `${JSON.stringify(report, null, 2)}\n`, 'utf8');
}

if (process.argv[1] && pathToFileURL(path.resolve(process.argv[1])).href === import.meta.url) {
  const options = parseOptions(process.argv.slice(2));
  const report = runComparison(options);
  writeComparison(options.outputPath, report);
  process.stdout.write(`wrote cross-path inference comparison to ${options.outputPath}\n`);
}
