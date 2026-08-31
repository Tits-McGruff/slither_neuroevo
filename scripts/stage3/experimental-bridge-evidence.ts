/**
 * Opt-in evidence capture for the isolated Stage 3 coarse Node-to-Rust bridge.
 *
 * This runner never starts the simulation server and is deliberately not a
 * production entry point.  It measures the real N-API addon only after an
 * operator invokes it explicitly.
 */

import { createRequire } from 'node:module';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { monitorEventLoopDelay, performance } from 'node:perf_hooks';
import { fileURLToPath } from 'node:url';
import {
  loadExperimentalNativeBridge,
  type ExperimentalEngineEvent,
  type ExperimentalEngineHealth,
  type ExperimentalEngineInit,
  type ExperimentalEngineNativeBinding
} from '../../server/rustEngine/experimentalNativeBridge.ts';
import { computeNativeSourceIdentity } from '../../server/rustEngine/nativeSourceIdentity.ts';

/** Evidence schema retained so later migration stages can reject incompatible records. */
const EVIDENCE_SCHEMA = 'slither-stage3-experimental-bridge-evidence';
/** Current evidence layout revision. */
const EVIDENCE_VERSION = 1;
/** The deliberate default requested by the Stage 3 foundation gate. */
const DEFAULT_IDLE_MS = 600_000;
/** A bounded post-idle probe period, long enough to collect stable latency samples. */
const DEFAULT_SUSTAINED_MS = 60_000;
/** Default cadence of coarse batches, not a production controller cadence. */
const DEFAULT_BATCHES_PER_SECOND = 30;
/** Default command count in one deliberately coarse test batch. */
const DEFAULT_BATCH_SIZE = 4;
/** Default opaque payload size per probe. */
const DEFAULT_PAYLOAD_BYTES = 256;
/** Default cadence for endpoint/peak process samples. */
const DEFAULT_SAMPLE_INTERVAL_MS = 1_000;
/** Prevent an evidence command from retaining an unbounded latency sample list. */
const MAX_SUBMITTED_COMMANDS = 65_536;
/** Prevent an accidental overnight evidence command from retaining excessive samples. */
const MAX_IDLE_MS = 24 * 60 * 60 * 1_000;
/** The sustained phase remains a bounded characterization, not a soak scheduler. */
const MAX_SUSTAINED_MS = 15 * 60 * 1_000;
/** Process samples are bounded independently of the nominal duration. */
const MAX_PROCESS_SAMPLES = 10_000;
/** Bounded operating-system identity file size accepted by the evidence runner. */
const MAX_OS_RELEASE_BYTES = 64 * 1024;
/** Binary-gigabyte unit used for the owner VM memory-allocation check. */
const GIBIBYTE = 1024 ** 3;

/** Explicit host class retained so target-VM evidence cannot be implied by hostname alone. */
export type ExperimentalBridgeEvidenceEnvironment = 'development-machine' | 'owner-target-vm';

/** Host facts used to classify evidence without coupling the checks to the live process. */
export interface ExperimentalBridgeEnvironmentFacts {
  /** Node platform identifier. */
  platform: string;
  /** Node architecture identifier. */
  architecture: string;
  /** Operating-system hostname, optionally qualified by a DNS suffix. */
  hostname: string;
  /** Normalized Linux distribution ID, or null outside a recognized Linux release. */
  distributionId: string | null;
  /** Processor model reported by Node. */
  cpuModel: string;
  /** Logical processors visible inside the VM. */
  logicalCpuCount: number;
  /** Physical memory visible inside the VM. */
  totalMemoryBytes: number;
}

/** CLI and programmatic options for one isolated bridge evidence run. */
export interface ExperimentalBridgeEvidenceOptions {
  /** Idle duration after the startup wake has fully drained. */
  idleMs: number;
  /** Duration of the bounded, real-addon probe phase following idle. */
  sustainedMs: number;
  /** Coarse batch cadence during the sustained phase. */
  batchesPerSecond: number;
  /** Commands in each all-or-nothing bridge batch. */
  batchSize: number;
  /** Opaque bytes in each command payload. */
  payloadBytes: number;
  /** Memory/health observation cadence. This timer belongs only to the evidence runner. */
  sampleIntervalMs: number;
  /** Optional evidence destination; existing files are never overwritten. */
  outputPath: string | null;
  /** Exact source commit supplied for a Git-less archive, or null for local Git discovery. */
  sourceCommit: string | null;
  /** Matching explicit dirty state; null selects local Git discovery. */
  sourceDirty: boolean | null;
  /** Explicit machine class for this measurement. */
  environmentClass: ExperimentalBridgeEvidenceEnvironment | null;
}

/** One timestamped process-memory observation. */
interface ProcessSample {
  /** Elapsed wall time since the real addon was started. */
  elapsedWallMs: number;
  /** Resident process bytes. */
  rssBytes: number;
  /** V8 heap bytes in use. */
  heapUsedBytes: number;
  /** Native/external bytes tracked by Node. */
  externalBytes: number;
  /** Array-buffer bytes tracked by Node. */
  arrayBuffersBytes: number;
}

/** Peak process observations, retained without retaining every allocation. */
interface MemoryPeaks {
  /** Maximum RSS observed. */
  rssBytes: number;
  /** Maximum V8 heap use observed. */
  heapUsedBytes: number;
  /** Maximum external-memory use observed. */
  externalBytes: number;
  /** Maximum array-buffer use observed. */
  arrayBuffersBytes: number;
}

/** One elapsed command/result measurement. */
interface CompletedProbe {
  /** Exact sequence received from Rust. */
  sequence: bigint;
  /** Round-trip elapsed time on this Node process. */
  latencyMs: number;
  /** Returned payload size. */
  bytes: number;
}

/** Conventional local addon location relative to this source file. */
const NATIVE_DIRECTORY = fileURLToPath(new URL('../../native', import.meta.url));
/** Generated napi-rs loader, intentionally loaded only by this opt-in runner. */
const NATIVE_LOADER = fileURLToPath(new URL('../../native/index.js', import.meta.url));
/** Repository root used for read-only source identity commands. */
const REPOSITORY_ROOT = fileURLToPath(new URL('../..', import.meta.url));
/** CommonJS loader used because napi-rs generates a CommonJS platform loader. */
const require = createRequire(import.meta.url);

/**
 * Parse a bounded positive integer from a CLI argument.
 * @param value - Raw option value.
 * @param name - Option name for the diagnostic.
 * @param maximum - Inclusive maximum.
 * @returns Validated integer.
 */
function parsePositiveInteger(value: string | undefined, name: string, maximum: number): number {
  if (!value || !/^\d+$/.test(value)) {
    throw new Error(`${name} must be a positive integer no greater than ${maximum}.`);
  }
  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed) || parsed <= 0 || parsed > maximum) {
    throw new Error(`${name} must be a positive integer no greater than ${maximum}.`);
  }
  return parsed;
}

/**
 * Reject an invalid programmatic numeric option using the same bounds as the CLI.
 * @param value - Candidate option value.
 * @param name - Option name for the diagnostic.
 * @param maximum - Inclusive maximum.
 */
function requireBoundedInteger(value: number, name: string, maximum: number): void {
  if (!Number.isSafeInteger(value) || value <= 0 || value > maximum) {
    throw new Error(`${name} must be a positive integer no greater than ${maximum}.`);
  }
}

/**
 * Parse one explicit command-line evidence request.
 * @param argv - Arguments after the runner path.
 * @returns A bounded evidence configuration.
 */
export function parseExperimentalBridgeEvidenceOptions(
  argv: readonly string[]
): ExperimentalBridgeEvidenceOptions {
  const result: ExperimentalBridgeEvidenceOptions = {
    idleMs: DEFAULT_IDLE_MS,
    sustainedMs: DEFAULT_SUSTAINED_MS,
    batchesPerSecond: DEFAULT_BATCHES_PER_SECOND,
    batchSize: DEFAULT_BATCH_SIZE,
    payloadBytes: DEFAULT_PAYLOAD_BYTES,
    sampleIntervalMs: DEFAULT_SAMPLE_INTERVAL_MS,
    outputPath: null,
    sourceCommit: null,
    sourceDirty: null,
    environmentClass: null
  };
  for (let index = 0; index < argv.length; index++) {
    const option = argv[index];
    const value = argv[index + 1];
    switch (option) {
      case '--idle-ms':
        result.idleMs = parsePositiveInteger(value, option, MAX_IDLE_MS);
        index++;
        break;
      case '--sustained-ms':
        result.sustainedMs = parsePositiveInteger(value, option, MAX_SUSTAINED_MS);
        index++;
        break;
      case '--batches-per-second':
        result.batchesPerSecond = parsePositiveInteger(value, option, 240);
        index++;
        break;
      case '--batch-size':
        result.batchSize = parsePositiveInteger(value, option, 64);
        index++;
        break;
      case '--payload-bytes':
        result.payloadBytes = parsePositiveInteger(value, option, 4 * 1024);
        index++;
        break;
      case '--sample-interval-ms':
        result.sampleIntervalMs = parsePositiveInteger(value, option, 60_000);
        index++;
        break;
      case '--output':
        if (!value) throw new Error('--output requires a path.');
        result.outputPath = path.resolve(value);
        index++;
        break;
      case '--source-commit':
        if (!value || !/^[0-9a-f]{40}$/.test(value)) {
          throw new Error('--source-commit must be an exact lowercase 40-character Git commit.');
        }
        result.sourceCommit = value;
        index++;
        break;
      case '--source-dirty':
        if (value !== 'true' && value !== 'false') {
          throw new Error('--source-dirty must be true or false.');
        }
        result.sourceDirty = value === 'true';
        index++;
        break;
      case '--environment':
        if (value !== 'development-machine' && value !== 'owner-target-vm') {
          throw new Error('--environment must be development-machine or owner-target-vm.');
        }
        result.environmentClass = value;
        index++;
        break;
      default:
        throw new Error(`Unknown option ${option ?? '<missing>'}.`);
    }
  }
  validateOptions(result);
  return result;
}

/**
 * Reject option combinations that would make the measurement itself unbounded.
 * @param options - Candidate evidence configuration.
 */
function validateOptions(options: ExperimentalBridgeEvidenceOptions): void {
  requireBoundedInteger(options.idleMs, 'idleMs', MAX_IDLE_MS);
  requireBoundedInteger(options.sustainedMs, 'sustainedMs', MAX_SUSTAINED_MS);
  requireBoundedInteger(options.batchesPerSecond, 'batchesPerSecond', 240);
  requireBoundedInteger(options.batchSize, 'batchSize', 64);
  requireBoundedInteger(options.payloadBytes, 'payloadBytes', 4 * 1024);
  requireBoundedInteger(options.sampleIntervalMs, 'sampleIntervalMs', 60_000);
  if (options.outputPath !== null && (typeof options.outputPath !== 'string' || options.outputPath.length === 0)) {
    throw new Error('outputPath must be null or a non-empty path.');
  }
  if ((options.sourceCommit === null) !== (options.sourceDirty === null)) {
    throw new Error('sourceCommit and sourceDirty must either both be supplied or both use Git discovery.');
  }
  if (options.sourceCommit !== null && !/^[0-9a-f]{40}$/.test(options.sourceCommit)) {
    throw new Error('sourceCommit must be an exact lowercase 40-character Git commit.');
  }
  if (options.sourceDirty !== null && typeof options.sourceDirty !== 'boolean') {
    throw new Error('sourceDirty must be null or boolean.');
  }
  if (options.environmentClass !== 'development-machine' && options.environmentClass !== 'owner-target-vm') {
    throw new Error('environmentClass must be development-machine or owner-target-vm.');
  }
  const estimatedBatches = Math.ceil((options.sustainedMs / 1_000) * options.batchesPerSecond);
  const estimatedCommands = estimatedBatches * options.batchSize;
  const estimatedSamples = Math.ceil((options.idleMs + options.sustainedMs) / options.sampleIntervalMs) + 4;
  if (estimatedCommands > MAX_SUBMITTED_COMMANDS) {
    throw new Error(
      `Sustained phase would submit ${estimatedCommands} commands; maximum is ${MAX_SUBMITTED_COMMANDS}.`
    );
  }
  if (estimatedSamples > MAX_PROCESS_SAMPLES) {
    throw new Error(`Measurement would retain ${estimatedSamples} process samples; maximum is ${MAX_PROCESS_SAMPLES}.`);
  }
}

/**
 * Capture the repository source that includes both the runner and Node bridge.
 * @param options - Validated explicit/archive identity or local-discovery request.
 * @returns Commit, dirty state, and the evidence method used.
 */
function repositorySourceIdentity(options: ExperimentalBridgeEvidenceOptions): {
  commit: string;
  dirty: boolean;
  method: 'explicit-archive' | 'git';
} {
  if (options.sourceCommit !== null && options.sourceDirty !== null) {
    return { commit: options.sourceCommit, dirty: options.sourceDirty, method: 'explicit-archive' };
  }
  const environment = { ...process.env, GIT_OPTIONAL_LOCKS: '0' };
  const commit = spawnSync('git', ['rev-parse', 'HEAD'], {
    cwd: REPOSITORY_ROOT,
    encoding: 'utf8',
    env: environment
  });
  const status = spawnSync('git', ['status', '--porcelain'], {
    cwd: REPOSITORY_ROOT,
    encoding: 'utf8',
    env: environment
  });
  const resolvedCommit = typeof commit.stdout === 'string' ? commit.stdout.trim() : '';
  const resolvedStatus = typeof status.stdout === 'string' ? status.stdout.trim() : '';
  if (commit.status !== 0 || !/^[0-9a-f]{40}$/.test(resolvedCommit) || status.status !== 0) {
    throw new Error(
      'Exact Git source identity is unavailable; a Git-less archive must supply both --source-commit and --source-dirty.'
    );
  }
  return {
    commit: resolvedCommit,
    dirty: resolvedStatus.length > 0,
    method: 'git'
  };
}

/** Read a bounded Linux distribution identifier without invoking another process. */
function linuxDistributionId(): string | null {
  if (process.platform !== 'linux') return null;
  const releasePath = '/etc/os-release';
  try {
    const metadata = fs.statSync(releasePath);
    if (!metadata.isFile() || metadata.size <= 0 || metadata.size > MAX_OS_RELEASE_BYTES) return null;
    const contents = fs.readFileSync(releasePath, 'utf8');
    const match = /^ID=(?:"([^"]+)"|([^\r\n]+))$/m.exec(contents);
    return (match?.[1] ?? match?.[2] ?? '').trim().toLowerCase() || null;
  } catch {
    return null;
  }
}

/**
 * Evaluate the recorded Oxygen allocation and host facts without trusting a caller-supplied label.
 * @param facts - Captured or synthetic environment facts.
 * @returns Individually visible checks used by the aggregate owner-target decision.
 */
export function evaluateOwnerTargetVmFacts(
  facts: ExperimentalBridgeEnvironmentFacts
): Record<string, boolean> {
  const shortHostname = facts.hostname.split('.', 1)[0]?.toLowerCase() ?? '';
  return {
    platformIsLinux: facts.platform === 'linux',
    architectureIsX64: facts.architecture === 'x64',
    distributionIsDebian: facts.distributionId === 'debian',
    hostnameIsOxygen: shortHostname === 'oxygen',
    cpuIsRyzen7_2700: /\bryzen\s+7\s+2700\b/i.test(facts.cpuModel),
    logicalCpuCountIsEight: facts.logicalCpuCount === 8,
    memoryMatches16GiBAllocation: facts.totalMemoryBytes >= 15 * GIBIBYTE &&
      facts.totalMemoryBytes <= 17 * GIBIBYTE
  };
}

/**
 * Capture and validate the declared machine class before starting a long run.
 * @param environmentClass - Explicit evidence-machine classification.
 * @returns Descriptive facts plus individually visible owner-target checks.
 */
function captureEnvironmentIdentity(
  environmentClass: ExperimentalBridgeEvidenceEnvironment
): Record<string, unknown> {
  const cpus = os.cpus();
  const hostname = os.hostname();
  const cpuModel = cpus[0]?.model ?? 'unknown';
  const totalMemoryBytes = os.totalmem();
  const distributionId = linuxDistributionId();
  const ownerTargetVmChecks = evaluateOwnerTargetVmFacts({
    platform: process.platform,
    architecture: process.arch,
    hostname,
    distributionId,
    cpuModel,
    logicalCpuCount: cpus.length,
    totalMemoryBytes
  });
  const ownerTargetVmValidated = Object.values(ownerTargetVmChecks).every(Boolean);
  if (environmentClass === 'owner-target-vm' && !ownerTargetVmValidated) {
    const failed = Object.entries(ownerTargetVmChecks)
      .filter(([, passed]) => !passed)
      .map(([name]) => name)
      .join(', ');
    throw new Error(`owner-target-vm evidence classification does not match Oxygen: ${failed}.`);
  }
  return {
    capturedAt: new Date().toISOString(),
    platform: process.platform,
    architecture: process.arch,
    hostname,
    distributionId,
    node: process.version,
    v8: process.versions.v8,
    cpuModel,
    logicalCpuCount: cpus.length,
    totalMemoryBytes,
    ownerTargetVmValidated,
    ownerTargetVmChecks
  };
}

/**
 * Load the generated real addon through its platform-aware napi-rs loader.
 * @returns Strict experimental bridge exports.
 */
function loadNativeBinding(): ExperimentalEngineNativeBinding {
  return require(NATIVE_LOADER) as ExperimentalEngineNativeBinding;
}

/**
 * Construct conservative evidence-only queue limits.
 * @param contractVersion - Exact native contract version read from the addon.
 * @returns A bounded native coordinator configuration.
 */
function evidenceInit(contractVersion: number): ExperimentalEngineInit {
  return {
    contractVersion,
    maxInboundBatches: 128,
    maxInboundCommands: 8_192,
    maxInboundOwnedBytes: 32 * 1024 * 1024,
    maxBatchCommands: 64,
    maxBatchOwnedBytes: 256 * 1024,
    maxOutputReliable: 8_192,
    maxOutputReliableOwnedBytes: 32 * 1024 * 1024,
    maxOutputDiscrete: 128,
    maxOutputDiscreteOwnedBytes: 512 * 1024,
    maxOutputTotalOwnedBytes: 32 * 1024 * 1024,
    maxOutputEventOwnedBytes: 256 * 1024,
    maxOutputFrameConnections: 1
  };
}

/**
 * Return the current process allocation endpoints in a JSON-safe shape.
 * @param startedAt - Monotonic start time.
 * @returns A single point-in-time allocation observation.
 */
function captureMemory(startedAt: number): ProcessSample {
  const memory = process.memoryUsage();
  return {
    elapsedWallMs: rounded(performance.now() - startedAt),
    rssBytes: memory.rss,
    heapUsedBytes: memory.heapUsed,
    externalBytes: memory.external,
    arrayBuffersBytes: memory.arrayBuffers
  };
}

/**
 * Fold a process sample into the bounded peak view.
 * @param peaks - Mutable measurement peak record.
 * @param sample - Newly observed process sample.
 */
function updatePeaks(peaks: MemoryPeaks, sample: ProcessSample): void {
  peaks.rssBytes = Math.max(peaks.rssBytes, sample.rssBytes);
  peaks.heapUsedBytes = Math.max(peaks.heapUsedBytes, sample.heapUsedBytes);
  peaks.externalBytes = Math.max(peaks.externalBytes, sample.externalBytes);
  peaks.arrayBuffersBytes = Math.max(peaks.arrayBuffersBytes, sample.arrayBuffersBytes);
}

/**
 * Yield a real event-loop turn without adding a bridge poll.
 * @returns Resolution on the next check phase.
 */
function nextTurn(): Promise<void> {
  return new Promise(resolve => setImmediate(resolve));
}

/**
 * Wait for a monotonic duration while allowing native wake continuations to run.
 * @param durationMs - Positive bounded wait duration.
 * @returns Resolution after at least the requested wall duration.
 */
async function waitWallDuration(durationMs: number): Promise<void> {
  const deadline = performance.now() + durationMs;
  while (true) {
    const remainingMs = deadline - performance.now();
    if (remainingMs <= 0) return;
    await new Promise(resolve => setTimeout(resolve, Math.max(1, Math.ceil(remainingMs))));
  }
}

/**
 * Wait for a condition without using a production bridge timer or retaining unbounded state.
 * @param condition - Predicate observed after event-loop turns.
 * @param description - Failure context.
 * @param timeoutMs - Hard local failure deadline.
 */
async function waitFor(condition: () => boolean, description: string, timeoutMs: number): Promise<void> {
  const deadline = performance.now() + timeoutMs;
  while (!condition()) {
    if (performance.now() >= deadline) throw new Error(`Timed out waiting for ${description}.`);
    await nextTurn();
  }
}

/**
 * Wait until startup publication and wake re-arm counters have stopped moving.
 *
 * These health reads occur before the measured idle interval. The interval
 * itself performs no bridge polling and compares only its two endpoints.
 *
 * @param readHealth - Small native health reader.
 * @param timeoutMs - Hard startup-settling deadline.
 * @returns The last of three identical, queue-empty health observations.
 */
async function waitForStableIdleHealth(
  readHealth: () => ExperimentalEngineHealth,
  timeoutMs: number
): Promise<ExperimentalEngineHealth> {
  const deadline = performance.now() + timeoutMs;
  let previousFingerprint: string | null = null;
  let consecutiveMatches = 0;
  while (performance.now() < deadline) {
    const current = readHealth();
    const empty = current.lifecycle === 'running' &&
      current.inboundBatches === 0n && current.inboundCommands === 0n &&
      current.inboundOwnedBytes === 0n && current.outputReliable === 0n &&
      current.outputDiscrete === 0n && current.outputFrames === 0n &&
      current.outputOwnedBytes === 0n && current.wakePending === false;
    const fingerprint = JSON.stringify(jsonHealth(current));
    consecutiveMatches = empty && fingerprint === previousFingerprint ? consecutiveMatches + 1 : 0;
    if (consecutiveMatches >= 2) return current;
    previousFingerprint = fingerprint;
    await waitWallDuration(10);
  }
  throw new Error('Timed out waiting for native startup health and wake counters to settle.');
}

/**
 * Calculate distribution percentiles from one bounded numeric sample list.
 * @param values - In-memory duration values.
 * @returns Rounded count and percentile summary.
 */
function latencyDistribution(values: readonly number[]): Record<string, number | null> {
  if (values.length === 0) {
    return { count: 0, minMs: null, meanMs: null, p50Ms: null, p95Ms: null, p99Ms: null, maxMs: null };
  }
  const ordered = [...values].sort((left, right) => left - right);
  const percentile = (percentage: number): number => {
    const index = Math.min(ordered.length - 1, Math.max(0, Math.ceil((percentage / 100) * ordered.length) - 1));
    return ordered[index] ?? 0;
  };
  const sum = values.reduce((total, value) => total + value, 0);
  return {
    count: values.length,
    minMs: rounded(ordered[0] ?? 0),
    meanMs: rounded(sum / values.length),
    p50Ms: rounded(percentile(50)),
    p95Ms: rounded(percentile(95)),
    p99Ms: rounded(percentile(99)),
    maxMs: rounded(ordered[ordered.length - 1] ?? 0)
  };
}

/**
 * Convert one phase-specific event-loop histogram into milliseconds.
 * @param histogram - Enabled monitor stopped at the phase boundary.
 * @returns Rounded delay distribution for only that phase.
 */
function eventLoopDelayDistribution(
  histogram: ReturnType<typeof monitorEventLoopDelay>
): Record<string, number> {
  return {
    min: rounded(histogram.min / 1e6),
    mean: rounded(histogram.mean / 1e6),
    p95: rounded(histogram.percentile(95) / 1e6),
    p99: rounded(histogram.percentile(99) / 1e6),
    max: rounded(histogram.max / 1e6)
  };
}

/**
 * Preserve exact native counters in JSON without silently narrowing BigInt values.
 * @param health - Native health surface.
 * @returns Counter fields represented as decimal strings and scalar fields unchanged.
 */
function jsonHealth(health: ExperimentalEngineHealth): Record<string, unknown> {
  return Object.fromEntries(Object.entries(health).map(([key, value]) => [
    key,
    typeof value === 'bigint' ? value.toString(10) : value ?? null
  ]));
}

/**
 * Round non-identity timing data without changing exact counts and byte totals.
 * @param value - Floating-point measurement.
 * @returns Six-decimal evidence value.
 */
function rounded(value: number): number {
  return Number(value.toFixed(6));
}

/**
 * Convert Node's exact microsecond CPU delta into a wall-normalized summary.
 * @param cpu - User and system CPU delta returned by process.cpuUsage.
 * @param wallMs - Matching monotonic wall duration.
 * @returns JSON-safe CPU counters and average occupied-core equivalent.
 */
function cpuSummary(cpu: NodeJS.CpuUsage, wallMs: number): Record<string, number> {
  return {
    userMicros: cpu.user,
    systemMicros: cpu.system,
    totalMicros: cpu.user + cpu.system,
    averageCpuCoreEquivalents: rounded(((cpu.user + cpu.system) / 1_000) / wallMs)
  };
}

/**
 * Create deterministic bounded opaque probe bytes without allocating a population representation.
 * @param sequence - Exact command identity.
 * @param byteLength - Bounded payload length.
 * @returns New opaque payload with its sequence encoded in the first bytes.
 */
function createPayload(sequence: bigint, byteLength: number): Uint8Array {
  const payload = new Uint8Array(byteLength);
  for (let index = 0; index < Math.min(8, byteLength); index++) {
    payload[index] = Number((sequence >> BigInt(index * 8)) & 0xffn);
  }
  return payload;
}

/**
 * Write an artifact beside its final destination, then atomically link its final non-replacing name.
 * @param outputPath - Requested final path.
 * @param result - JSON-compatible evidence object.
 */
export function writeEvidenceAtomically(outputPath: string, result: Record<string, unknown>): void {
  const directory = path.dirname(outputPath);
  fs.mkdirSync(directory, { recursive: true });
  if (fs.existsSync(outputPath)) {
    throw new Error(`Refusing to overwrite existing evidence file: ${outputPath}`);
  }
  const temporaryPath = path.join(
    directory,
    `.${path.basename(outputPath)}.${process.pid}.${Date.now()}.tmp`
  );
  let descriptor: number | null = null;
  try {
    descriptor = fs.openSync(temporaryPath, 'wx');
    fs.writeFileSync(descriptor, `${JSON.stringify(result, null, 2)}\n`, { encoding: 'utf8' });
    fs.fsyncSync(descriptor);
    fs.closeSync(descriptor);
    descriptor = null;
    // A same-directory hard link atomically creates the final name and fails
    // if another writer won the race. Unlike POSIX rename, it cannot replace
    // an evidence file created between the initial check and publication.
    fs.linkSync(temporaryPath, outputPath);
  } finally {
    if (descriptor !== null) fs.closeSync(descriptor);
    if (fs.existsSync(temporaryPath)) fs.rmSync(temporaryPath, { force: true });
  }
}

/**
 * Run the isolated real-addon bridge measurement.
 *
 * The sampling interval is evidence instrumentation. The bridge itself only
 * drains in response to payload-free native wakes and never receives a polling
 * timer from this runner.
 *
 * @param options - Validated bounded measurement options.
 * @returns JSON-safe evidence with exact counter fields encoded as decimal strings.
 */
export async function runExperimentalBridgeEvidence(
  options: ExperimentalBridgeEvidenceOptions
): Promise<Record<string, unknown>> {
  validateOptions(options);
  const repositorySource = repositorySourceIdentity(options);
  const environmentIdentity = captureEnvironmentIdentity(options.environmentClass);
  const binding = loadNativeBinding();
  const contractVersion = binding.experimentalEngineContractVersion();
  const sourceIdentity = computeNativeSourceIdentity(NATIVE_DIRECTORY);
  const received = new Map<bigint, CompletedProbe>();
  const submittedAt = new Map<bigint, number>();
  let submittedCommands = 0;
  let submittedBatches = 0;
  let submittedPayloadBytes = 0;
  let completedPayloadBytes = 0;
  let lifecycleEvents = 0;
  let unexpectedEvents = 0;
  let bridgeFault: Error | null = null;
  const bridge = await loadExperimentalNativeBridge({
    nativeManifestDirectory: NATIVE_DIRECTORY,
    loadBinding: () => binding,
    init: evidenceInit(contractVersion),
    handlers: {
      onEvent(event: ExperimentalEngineEvent): void {
        if (event.kind === 'started' || event.kind === 'stopped') {
          lifecycleEvents++;
          return;
        }
        if (event.kind !== 'probeResult') {
          unexpectedEvents++;
          return;
        }
        const startedAt = submittedAt.get(event.sequence);
        if (startedAt === undefined) {
          throw new Error(`Received an unknown probe sequence ${event.sequence.toString(10)}.`);
        }
        if (received.has(event.sequence)) {
          throw new Error(`Received duplicate probe sequence ${event.sequence.toString(10)}.`);
        }
        if (event.correlationId !== event.sequence) {
          throw new Error(`Probe ${event.sequence.toString(10)} returned a different correlation ID.`);
        }
        const expectedPayload = createPayload(event.sequence, options.payloadBytes);
        if (
          event.payload.byteLength !== expectedPayload.byteLength ||
          event.payload.some((value, index) => value !== expectedPayload[index])
        ) {
          throw new Error(`Probe ${event.sequence.toString(10)} returned a different payload.`);
        }
        received.set(event.sequence, {
          sequence: event.sequence,
          latencyMs: performance.now() - startedAt,
          bytes: event.payload.byteLength
        });
        completedPayloadBytes += event.payload.byteLength;
      },
      onFault(error: Error): void {
        bridgeFault = error;
      }
    },
    maxDrainEvents: 128,
    maxDrainOwnedBytes: 512 * 1024
  });
  const startedAt = performance.now();
  const memorySamples: ProcessSample[] = [];
  const initialMemory = captureMemory(startedAt);
  const peaks: MemoryPeaks = {
    rssBytes: initialMemory.rssBytes,
    heapUsedBytes: initialMemory.heapUsedBytes,
    externalBytes: initialMemory.externalBytes,
    arrayBuffersBytes: initialMemory.arrayBuffersBytes
  };
  memorySamples.push(initialMemory);
  const observeMemory = (): void => {
    const sample = captureMemory(startedAt);
    updatePeaks(peaks, sample);
    if (memorySamples.length < MAX_PROCESS_SAMPLES) memorySamples.push(sample);
  };
  const sampleTimer = setInterval(observeMemory, options.sampleIntervalMs);
  const idleEventLoop = monitorEventLoopDelay({ resolution: 10 });
  const sustainedEventLoop = monitorEventLoopDelay({ resolution: 10 });
  const cpuBefore = process.cpuUsage();
  try {
    bridge.start();
    await waitFor(() => bridge.health().lifecycle === 'running', 'native coordinator startup', 5_000);
    const beforeIdle = await waitForStableIdleHealth(() => bridge.health(), 5_000);
    const idleStartedAt = performance.now();
    const idleCpuBefore = process.cpuUsage();
    const idleMemoryBefore = captureMemory(startedAt);
    updatePeaks(peaks, idleMemoryBefore);
    if (memorySamples.length < MAX_PROCESS_SAMPLES) memorySamples.push(idleMemoryBefore);
    idleEventLoop.enable();
    await waitWallDuration(options.idleMs);
    idleEventLoop.disable();
    const idleWallMs = performance.now() - idleStartedAt;
    const idleCpu = process.cpuUsage(idleCpuBefore);
    const idleMemoryAfter = captureMemory(startedAt);
    updatePeaks(peaks, idleMemoryAfter);
    if (memorySamples.length < MAX_PROCESS_SAMPLES) memorySamples.push(idleMemoryAfter);
    const afterIdle = bridge.health();
    const idleHealthStable = JSON.stringify(jsonHealth(afterIdle)) === JSON.stringify(jsonHealth(beforeIdle));
    const idleWakeStable = idleHealthStable &&
      afterIdle.inboundBatches === 0n && afterIdle.inboundCommands === 0n &&
      afterIdle.inboundOwnedBytes === 0n && afterIdle.outputReliable === 0n &&
      afterIdle.outputDiscrete === 0n && afterIdle.outputFrames === 0n &&
      afterIdle.outputOwnedBytes === 0n && afterIdle.wakePending === false;
    if (!idleWakeStable) {
      throw new Error('Idle bridge wake/queue counters changed without a submitted command; polling or stale work is present.');
    }

    const phaseStartedAt = performance.now();
    const sustainedCpuBefore = process.cpuUsage();
    const sustainedMemoryBefore = captureMemory(startedAt);
    updatePeaks(peaks, sustainedMemoryBefore);
    if (memorySamples.length < MAX_PROCESS_SAMPLES) memorySamples.push(sustainedMemoryBefore);
    const sustainedHealthBefore = bridge.health();
    sustainedEventLoop.enable();
    const cadenceMs = 1_000 / options.batchesPerSecond;
    let nextBatchAt = phaseStartedAt;
    let sequence = 1n;
    while (performance.now() - phaseStartedAt < options.sustainedMs) {
      const commands = Array.from({ length: options.batchSize }, () => {
        const commandSequence = sequence;
        sequence += 1n;
        const payload = createPayload(commandSequence, options.payloadBytes);
        submittedAt.set(commandSequence, performance.now());
        submittedCommands++;
        submittedPayloadBytes += payload.byteLength;
        return { sequence: commandSequence, correlationId: commandSequence, payload };
      });
      bridge.submitProbeBatch({ contractVersion, commands });
      submittedBatches++;
      nextBatchAt += cadenceMs;
      const delayMs = nextBatchAt - performance.now();
      if (delayMs > 1) await waitWallDuration(delayMs);
      else await nextTurn();
    }
    const submissionWallMs = performance.now() - phaseStartedAt;
    await waitFor(
      () => received.size === submittedCommands,
      `${submittedCommands} probe results`,
      Math.max(10_000, Math.ceil(options.sustainedMs / 2))
    );
    if (bridgeFault) throw bridgeFault;
    sustainedEventLoop.disable();
    const completionWallMs = performance.now() - phaseStartedAt;
    const sustainedCpu = process.cpuUsage(sustainedCpuBefore);
    const sustainedMemoryAfter = captureMemory(startedAt);
    updatePeaks(peaks, sustainedMemoryAfter);
    if (memorySamples.length < MAX_PROCESS_SAMPLES) memorySamples.push(sustainedMemoryAfter);
    const finalHealth = bridge.health();
    const processedBatchesDelta = finalHealth.processedBatches - sustainedHealthBefore.processedBatches;
    const processedCommandsDelta = finalHealth.processedCommands - sustainedHealthBefore.processedCommands;
    const completions = [...received.values()];
    const latencyMs = completions.map(completion => completion.latencyMs);
    const assertions = {
      realAddonSourceMatchesCurrentTree: binding.nativeAddonSourceSha256() === sourceIdentity.sha256,
      idleHadNoWakeOrQueueGrowth: idleWakeStable,
      allSubmittedCommandsCompleted: received.size === submittedCommands,
      allReturnedPayloadBytesMatch: completedPayloadBytes === submittedPayloadBytes,
      noUnexpectedOutputEvents: unexpectedEvents === 0,
      noNativeWakeFailures: finalHealth.wakeFailures === 0n,
      noNativeTerminalFault: finalHealth.lifecycle === 'running' && finalHealth.faultCode === undefined,
      nativeProcessedAllBatches: processedBatchesDelta === BigInt(submittedBatches),
      nativeProcessedAllCommands: processedCommandsDelta === BigInt(submittedCommands),
      inboundQueueDrained: finalHealth.inboundBatches === 0n &&
        finalHealth.inboundCommands === 0n && finalHealth.inboundOwnedBytes === 0n,
      outputQueueDrained: finalHealth.outputReliable === 0n && finalHealth.outputDiscrete === 0n &&
        finalHealth.outputFrames === 0n && finalHealth.outputHasStats === false &&
        finalHealth.outputOwnedBytes === 0n && finalHealth.wakePending === false
    };
    if (!Object.values(assertions).every(Boolean)) {
      throw new Error(`Bridge evidence assertions failed: ${JSON.stringify(assertions)}`);
    }
    const finalMemory = captureMemory(startedAt);
    updatePeaks(peaks, finalMemory);
    if (memorySamples.length < MAX_PROCESS_SAMPLES) memorySamples.push(finalMemory);
    const cpu = process.cpuUsage(cpuBefore);
    const wallMs = performance.now() - startedAt;
    const idleDurationSatisfied = idleWallMs >= DEFAULT_IDLE_MS;
    const stage3IdleEvidenceSatisfied = idleDurationSatisfied && idleWakeStable;
    return {
      schema: EVIDENCE_SCHEMA,
      version: EVIDENCE_VERSION,
      evidenceClass: stage3IdleEvidenceSatisfied
        ? 'stage3-foundation-gate coarse-bridge result'
        : 'short-validation coarse-bridge result',
      environmentClass: options.environmentClass,
      caveat: 'This is an isolated real-addon Stage 3 bridge measurement. It does not start the normal server, make Rust authoritative, measure simulation throughput, establish production cutover, or validate the Debian VM unless run there and labelled separately.',
      command: [process.execPath, ...process.argv.slice(1)],
      source: repositorySource,
      environment: environmentIdentity,
      native: {
        sourceSha256: binding.nativeAddonSourceSha256(),
        independentlyComputedSourceSha256: sourceIdentity.sha256,
        buildTarget: binding.nativeAddonBuildTarget(),
        buildProfile: binding.nativeAddonBuildProfile(),
        buildClass: binding.nativeAddonBuildClass(),
        rustcVersion: binding.nativeAddonRustcVersion(),
        buildContractSha256: binding.nativeAddonBuildContractSha256(),
        experimentalEngineContractVersion: contractVersion
      },
      workload: {
        idleMs: options.idleMs,
        sustainedMs: options.sustainedMs,
        batchesPerSecond: options.batchesPerSecond,
        batchSize: options.batchSize,
        payloadBytes: options.payloadBytes,
        sampleIntervalMs: options.sampleIntervalMs,
        instrumentationNote: 'setInterval samples process state only; it does not invoke drainOutputs, submit commands, or wake the bridge.'
      },
      result: {
        wallMs: rounded(wallMs),
        submittedCommands,
        completedCommands: received.size,
        submittedPayloadBytes,
        completedPayloadBytes,
        lifecycleEvents,
        unexpectedEvents,
        commandLatencyMs: latencyDistribution(latencyMs),
        stage3IdleGate: {
          requiredIdleMs: DEFAULT_IDLE_MS,
          observedIdleWallMs: rounded(idleWallMs),
          idleDurationSatisfied,
          idleNoPollingSatisfied: idleWakeStable,
          stage3IdleEvidenceSatisfied
        },
        cpu: cpuSummary(cpu, wallMs),
        idle: {
          wallMs: rounded(idleWallMs),
          cpu: cpuSummary(idleCpu, idleWallMs),
          memory: {
            before: idleMemoryBefore,
            after: idleMemoryAfter
          },
          eventLoopDelayMs: eventLoopDelayDistribution(idleEventLoop),
          healthBefore: jsonHealth(beforeIdle),
          healthAfter: jsonHealth(afterIdle)
        },
        sustained: {
          requestedSubmissionWindowMs: options.sustainedMs,
          submissionWallMs: rounded(submissionWallMs),
          completionWallMs: rounded(completionWallMs),
          submittedBatches,
          processedBatches: processedBatchesDelta.toString(10),
          processedCommands: processedCommandsDelta.toString(10),
          achievedSubmittedBatchesPerSecond: rounded(submittedBatches / (submissionWallMs / 1_000)),
          achievedProcessedBatchesPerSecond: rounded(Number(processedBatchesDelta) / (completionWallMs / 1_000)),
          achievedSubmittedCommandsPerSecond: rounded(submittedCommands / (submissionWallMs / 1_000)),
          achievedCompletedCommandsPerSecond: rounded(received.size / (completionWallMs / 1_000)),
          cpu: cpuSummary(sustainedCpu, completionWallMs),
          eventLoopDelayMs: eventLoopDelayDistribution(sustainedEventLoop),
          memory: {
            before: sustainedMemoryBefore,
            after: sustainedMemoryAfter
          },
          healthBefore: jsonHealth(sustainedHealthBefore),
          healthAfter: jsonHealth(finalHealth)
        },
        memory: {
          before: initialMemory,
          after: finalMemory,
          peaks,
          samples: memorySamples
        },
        finalHealth: jsonHealth(finalHealth),
        assertions
      }
    };
  } finally {
    clearInterval(sampleTimer);
    idleEventLoop.disable();
    sustainedEventLoop.disable();
    await bridge.stop();
  }
}

/** Execute one explicit CLI evidence run. */
async function main(): Promise<void> {
  const options = parseExperimentalBridgeEvidenceOptions(process.argv.slice(2));
  const result = await runExperimentalBridgeEvidence(options);
  if (options.outputPath) {
    writeEvidenceAtomically(options.outputPath, result);
    console.info(`[stage3.experimental-bridge] wrote ${options.outputPath}`);
  } else {
    process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch(error => {
    console.error(error);
    process.exitCode = 1;
  });
}
