/** Compare equivalent Stage 4 one-step inference results element by element. */

import { createHash } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

/** Stable comparison artifact version. */
const COMPARISON_VERSION = 1;

/** Parsed comparator command line. */
export interface ComparatorOptions {
  /** Rust scalar report. */
  rustPath: string;
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
    /** Cargo profile for the standalone Rust runner. */
    buildProfile?: string;
    /** Test-hook build class required by the standalone runner. */
    buildClass?: string;
    /** Exact supported compilation target. */
    targetTriple?: string;
    /** Other source fields differ across report families. */
    [key: string]: unknown;
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
  };
  /** Execution-path identity that must match the report's assigned role. */
  path: {
    /** Stable current or Rust path label. */
    name: string;
    /** Number of N-API calls inside one complete population pass. */
    nativeCallsPerWholePass: number;
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
type ReportRole = 'rust' | 'js' | 'native';

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
      case '--js': values.jsPath = path.resolve(value); break;
      case '--native': values.nativePath = path.resolve(value); break;
      case '--output': values.outputPath = path.resolve(value); break;
      default: throw new Error(`Unknown option ${option ?? '<missing>'}.`);
    }
  }
  if (!values.rustPath || !values.jsPath || !values.nativePath || !values.outputPath) {
    throw new Error('--rust, --js, --native, and --output are required.');
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
  if (role === 'rust') {
    if (
      report.schema !== 'slither-stage4-rust-inference-benchmark'
      || report.path?.name !== 'rust-scalar-coarse-heterogeneous'
      || nativeCalls !== 0
      || nativeIdentityPresent
      || report.source?.buildProfile !== 'release'
      || report.source?.buildClass !== 'test-hooks'
      || !isSupportedTarget(report.source?.targetTriple)
    ) {
      throw new Error(`${reportPath} does not prove the Rust scalar coarse heterogeneous path.`);
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
    || report.path.nativeIdentity?.nativeAddonBuildProfile !== 'release'
    || report.path.nativeIdentity?.nativeAddonBuildClass !== 'production'
    || !isSupportedTarget(report.path.nativeIdentity?.nativeAddonBuildTarget)
  ) {
    throw new Error(`${reportPath} does not prove the current count-one native path.`);
  }
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

/** Verify fixture and tolerance identity across the three input reports. */
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

/** Build, validate, and optionally persist one three-path comparison report. */
export function runComparison(options: ComparatorOptions): Record<string, unknown> {
  const rust = loadReport(options.rustPath, 'rust');
  const js = loadReport(options.jsPath, 'js');
  const native = loadReport(options.nativePath, 'native');
  const reports = [rust, js, native] as const;
  const tolerance = validateSharedIdentity(reports);
  const compare = (left: LoadedReport, right: LoadedReport) => ({
    outputs: compareFloat32Buffers(left.outputs, right.outputs, tolerance),
    recurrent: compareFloat32Buffers(left.recurrent, right.recurrent, tolerance)
  });
  const comparisons = {
    rustVsJs: compare(rust, js),
    rustVsCountOneNative: compare(rust, native),
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
    sources: Object.fromEntries(reports.map(report => [
      path.basename(report.path),
      report.reportSha256
    ])),
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
