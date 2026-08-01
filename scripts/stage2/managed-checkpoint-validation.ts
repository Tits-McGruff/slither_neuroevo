/**
 * Stage 2 managed-checkpoint write-validation trade-off benchmark.
 *
 * This disposable Node harness measures the selected container policy without
 * becoming the production checkpoint writer. The production codec belongs to
 * the Rust-authoritative engine and must be remeasured on the Debian VM.
 */

import { createHash, randomUUID } from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { performance } from 'node:perf_hooks';
import { spawnSync } from 'node:child_process';
import { resetCFGToDefaults } from '../../src/config.ts';
import type { Genome } from '../../src/mlp.ts';
import { World } from '../../src/world.ts';
import {
  createUstarHeader,
  decodeShuffledZstdBlocks,
  encodeShuffledZstdBlocks,
  logicalSha256,
  ManagedCheckpointFormatError,
  parseUstarHeader,
  ustarPaddingBytes,
  USTAR_BLOCK_BYTES,
  USTAR_TRAILER_BYTES,
  verifyLogicalRoot,
  computeLogicalRoot,
  type LogicalRoleDigest,
  type Stage2NumericEncoding
} from './managedCheckpointFormat.ts';
import {
  evolvePopulationFixture,
  installStage2Scenario,
  packPopulationWeights,
  STAGE2_WORLD_SEED
} from './fixtures.ts';

/** Workloads required by the write-validation comparison. */
type ValidationScenario = 'P0' | 'P2';
/** Deterministic population fixture state. */
type FixtureKind = 'fresh' | 'evolved';
/** Post-write work attributed to one policy variant. */
type ValidationPolicy = 'none' | 'lightweight-scan' | 'full-decode';
/** Named write-validation choices from Draft 4. */
type VariantName = 'single-pass' | 'frame-checksum' | 'lightweight-scan' | 'full-decode';
/** Provenance declaration supplied by the benchmark operator. */
type EvidenceEnvironment = 'development' | 'owner-target-vm';

/** Default independently decoded Float32 block size. */
const DEFAULT_BLOCK_BYTES = 1024 * 1024;
/** Default repeated trials per validation choice. */
const DEFAULT_TRIALS = 3;
/** Default deterministic operator generations for an evolved fixture. */
const DEFAULT_EVOLUTION_GENERATIONS = 25;
/** Maximum admitted raw population bytes for this disposable runner. */
const MAX_RAW_BYTES = 1024 * 1024 * 1024;
/** Fixed prototype checkpoint role ordering. */
const LOGICAL_ROLE_ORDER = [
  'checkpoint',
  'population-index',
  'population-weights',
  'population-recurrent'
] as const;
/** Fixed small-role entry names. */
const CHECKPOINT_ENTRY = 'checkpoint.json';
/** Fixed population-index entry name. */
const POPULATION_INDEX_ENTRY = 'population/index.bin';
/** Fixed reset-state entry name. */
const RECURRENT_ENTRY = 'population/recurrent.f32le';
/** Final manifest entry name. */
const MANIFEST_ENTRY = 'manifest.json';
/** Weight entry for raw packed selection. */
const RAW_WEIGHTS_ENTRY = 'population/weights.f32le';
/** Weight entry for shuffled-Zstandard selection. */
const COMPRESSED_WEIGHTS_ENTRY = 'population/weights.f32le.shuf4.zst';
/** Largest manifest accepted by the disposable scanner. */
const MAX_MANIFEST_BYTES = 1024 * 1024;
/** Bounded allowance above decoded bytes for one stored Zstandard frame. */
const MAX_ZSTD_FRAME_OVERHEAD_BYTES = 256 * 1024;
/** Exact shuffled-block marker validated before frame allocation. */
const SHUFFLED_BLOCK_MAGIC = Buffer.from('SFZ1', 'ascii');

/** Command-line options. */
interface RunnerOptions {
  /** P0 or P2 named workload. */
  scenario: ValidationScenario;
  /** Fresh or evolved-like weights. */
  fixture: FixtureKind;
  /** Operator generations used for an evolved fixture. */
  evolutionGenerations: number;
  /** Repetitions of every policy. */
  trials: number;
  /** Independently decoded numeric block bytes. */
  blockBytes: number;
  /** Optional retained JSON destination. */
  outputPath: string | null;
  /** Explicit provenance declaration; hardware checks alone never infer owner-host identity. */
  evidenceEnvironment: EvidenceEnvironment;
}

/** One validation choice. */
interface VariantDefinition {
  /** Artifact label. */
  name: VariantName;
  /** Whether generated Zstandard frames carry their own checksum. */
  frameChecksum: boolean;
  /** Work performed after closing/fsyncing the partial file. */
  validation: ValidationPolicy;
}

/** Exact prototype manifest role. */
interface PrototypeManifestRole extends LogicalRoleDigest {
  /** Actual USTAR entry name. */
  entry: string;
  /** Versioned stored encoding. */
  encoding: string;
  /** Stored entry bytes. */
  storedLength: number;
  /** Logical record count where meaningful. */
  logicalCount: number;
  /** Logical record bytes where meaningful. */
  recordSize: number;
}

/** Final manifest used only by this Stage 2 prototype. */
interface PrototypeManifest {
  /** Stable small-document marker. */
  magic: 'slither-neuroevo-save';
  /** Container contract revision exercised by the probe. */
  archiveVersion: 1;
  /** Internal checkpoint kind. */
  kind: 'checkpoint-v3-prototype';
  /** Prevents accidental treatment as a production save. */
  benchmarkOnly: true;
  /** Ordered logical roles preceding the manifest. */
  roles: PrototypeManifestRole[];
  /** One encoding-independent checkpoint identity. */
  logicalRoot: string;
  /** Sum of preceding stored role bytes. */
  totalStoredRoleBytes: number;
  /** Sum of preceding decoded role bytes. */
  totalDecodedRoleBytes: number;
  /** Population slots represented by the index. */
  populationCount: number;
  /** Float32 weights represented by the population. */
  totalWeightCount: number;
  /** Explicit generation-boundary reset-state declaration. */
  recurrentBoundary: 'zero-reset';
}

/** Strict file scan entry location. */
interface FileEntryLocation {
  /** Entry name. */
  name: string;
  /** Header byte offset. */
  headerOffset: number;
  /** First stored-data byte. */
  dataOffset: number;
  /** Stored entry bytes. */
  size: number;
}

/** Strict lightweight file-scan result. */
interface FileScanResult {
  /** Ordered entry locations. */
  entries: FileEntryLocation[];
  /** Validated final manifest. */
  manifest: PrototypeManifest;
  /** Bytes actually read rather than seek-skipped. */
  bytesRead: number;
  /** Complete archive bytes. */
  archiveBytes: number;
}

/** One process-memory snapshot. */
interface MemorySnapshot {
  /** Resident bytes. */
  rss: number;
  /** JavaScript heap bytes in use. */
  heapUsed: number;
  /** Native/external bytes reported by Node. */
  external: number;
  /** ArrayBuffer backing bytes. */
  arrayBuffers: number;
}

/** Highest sampled process-memory values during one trial. */
class MemoryPeak {
  /** Highest sampled resident bytes. */
  private rss = 0;
  /** Highest sampled heap bytes. */
  private heapUsed = 0;
  /** Highest sampled external bytes. */
  private external = 0;
  /** Highest sampled ArrayBuffer bytes. */
  private arrayBuffers = 0;

  /** Record one current process-memory sample. */
  sample(): void {
    const current = process.memoryUsage();
    this.rss = Math.max(this.rss, current.rss);
    this.heapUsed = Math.max(this.heapUsed, current.heapUsed);
    this.external = Math.max(this.external, current.external);
    this.arrayBuffers = Math.max(this.arrayBuffers, current.arrayBuffers);
  }

  /**
   * Return the sampled peak.
   * @returns Independent maxima by reported category.
   */
  result(): MemorySnapshot {
    return {
      rss: this.rss,
      heapUsed: this.heapUsed,
      external: this.external,
      arrayBuffers: this.arrayBuffers
    };
  }
}

/** Archive write accounting. */
class ArchiveWriter {
  /** Open output descriptor. */
  private readonly descriptor: number;
  /** Bytes written to the partial file. */
  private written = 0;

  /**
   * Open one new partial file without replacement.
   * @param filePath - Same-directory partial destination.
   */
  constructor(filePath: string) {
    this.descriptor = fs.openSync(filePath, 'wx');
  }

  /**
   * Append every byte and update evidence accounting.
   * @param data - Bytes to append.
   */
  write(data: Buffer): void {
    let offset = 0;
    while (offset < data.length) {
      const count = fs.writeSync(this.descriptor, data, offset, data.length - offset, null);
      if (count <= 0) throw new Error('archive write made no progress');
      offset += count;
    }
    this.written += data.length;
  }

  /**
   * Flush file data and metadata.
   * @returns Wall milliseconds spent in `fsync`.
   */
  sync(): number {
    const started = performance.now();
    fs.fsyncSync(this.descriptor);
    return performance.now() - started;
  }

  /** Close the partial file. */
  close(): void {
    fs.closeSync(this.descriptor);
  }

  /**
   * Return exact bytes written.
   * @returns Complete partial-file bytes.
   */
  finishBytes(): number {
    return this.written;
  }
}

/** Candidate-generation result. */
interface CandidateResult {
  /** Selected payload encoding. */
  encoding: Stage2NumericEncoding;
  /** Stored entry bytes. */
  selectedBytes: number;
  /** Shuffled candidate file bytes. */
  candidateBytes: number;
  /** Independent block count. */
  blocks: number;
  /** Candidate-generation wall time. */
  encodeMs: number;
  /** Candidate path when selected, otherwise still available until cleanup. */
  candidatePath: string;
  /** Logical SHA-256 calculated while streaming source blocks. */
  logicalSha256: string;
}

/** One full-decode result. */
interface FullDecodeResult {
  /** Total bytes read, including the lightweight scan. */
  bytesRead: number;
  /** Decoded logical role bytes. */
  decodedBytes: number;
  /** Reconstructed logical root. */
  logicalRoot: string;
  /** Decoded population hash. */
  weightSha256: string;
}

/** Directory synchronization result. */
interface DirectorySyncResult {
  /** Whether the runtime attempted to open and sync the directory. */
  attempted: boolean;
  /** Whether the operation completed successfully. */
  supported: boolean;
  /** Wall milliseconds. */
  elapsedMs: number;
  /** Platform error code when unsupported. */
  errorCode: string | null;
}

/** Numeric sample summary. */
interface Distribution {
  /** Number of samples. */
  count: number;
  /** Minimum. */
  min: number;
  /** Median. */
  p50: number;
  /** 95th percentile. */
  p95: number;
  /** Maximum. */
  max: number;
  /** Arithmetic mean. */
  mean: number;
}

/** One measured validation trial. */
interface TrialResult {
  /** Policy choice. */
  variant: VariantName;
  /** One-based trial ordinal. */
  trial: number;
  /** Whether compressed frames contain a codec checksum. */
  frameChecksum: boolean;
  /** Post-write validation performed. */
  validationPolicy: ValidationPolicy;
  /** Selected large numeric encoding. */
  selectedEncoding: Stage2NumericEncoding;
  /** Exact block count. */
  compressedBlocks: number;
  /** Logical identity shared by encoding variants. */
  logicalRoot: string;
  /** Exact byte accounting. */
  bytes: {
    rawWeights: number;
    shuffledCandidate: number;
    selectedWeightEntry: number;
    archive: number;
    applicationWritten: number;
    validationRead: number;
    decodedDuringValidation: number;
  };
  /** Split wall timings. */
  timingsMs: {
    candidateEncodeAndLogicalHash: number;
    archiveAssembly: number;
    fileFsync: number;
    validation: number;
    rename: number;
    directorySync: number;
    totalBarrier: number;
  };
  /** Highest manually sampled memory values. */
  sampledMemoryPeak: MemorySnapshot;
  /** Directory-sync platform result. */
  directorySync: DirectorySyncResult;
  /** Whether all applicable checks succeeded. */
  accepted: boolean;
}

/** One fault-policy observation. */
interface FaultObservation {
  /** Injected damage. */
  mutation: string;
  /** Whether lightweight scanning rejected it. */
  lightweight: { accepted: boolean; code: string | null };
  /** Whether full decode rejected it. */
  fullDecode: { accepted: boolean; code: string | null };
}

/** Variant definitions in stable report order. */
const VARIANTS: readonly VariantDefinition[] = [
  { name: 'single-pass', frameChecksum: false, validation: 'none' },
  { name: 'frame-checksum', frameChecksum: true, validation: 'none' },
  { name: 'lightweight-scan', frameChecksum: false, validation: 'lightweight-scan' },
  { name: 'full-decode', frameChecksum: false, validation: 'full-decode' }
];

/**
 * Parse one bounded integer option.
 * @param value - Raw CLI value.
 * @param option - Option label.
 * @param maximum - Inclusive maximum.
 * @returns Validated integer.
 */
function parseInteger(value: string | undefined, option: string, maximum: number): number {
  const parsed = Number.parseInt(value ?? '', 10);
  if (!Number.isSafeInteger(parsed) || parsed < 1 || parsed > maximum) {
    throw new RangeError(`${option} must be an integer from 1 through ${maximum}`);
  }
  return parsed;
}

/**
 * Parse command-line arguments.
 * @param argv - Arguments following the script path.
 * @returns Validated runner options.
 */
function parseOptions(argv: readonly string[]): RunnerOptions {
  const options: RunnerOptions = {
    scenario: 'P0',
    fixture: 'evolved',
    evolutionGenerations: DEFAULT_EVOLUTION_GENERATIONS,
    trials: DEFAULT_TRIALS,
    blockBytes: DEFAULT_BLOCK_BYTES,
    outputPath: null,
    evidenceEnvironment: 'development'
  };
  for (let index = 0; index < argv.length; index++) {
    const option = argv[index];
    const value = argv[index + 1];
    switch (option) {
      case '--scenario':
        if (value !== 'P0' && value !== 'P2') throw new Error('--scenario must be P0 or P2');
        options.scenario = value;
        index++;
        break;
      case '--fixture':
        if (value !== 'fresh' && value !== 'evolved') {
          throw new Error('--fixture must be fresh or evolved');
        }
        options.fixture = value;
        index++;
        break;
      case '--evolution-generations':
        options.evolutionGenerations = parseInteger(value, option, 10_000);
        index++;
        break;
      case '--trials':
        options.trials = parseInteger(value, option, 20);
        index++;
        break;
      case '--block-bytes':
        options.blockBytes = parseInteger(value, option, 64 * 1024 * 1024);
        if (options.blockBytes % 4 !== 0) {
          throw new RangeError('--block-bytes must be a multiple of four');
        }
        index++;
        break;
      case '--output':
        if (!value) throw new Error('--output requires a path');
        options.outputPath = path.resolve(value);
        index++;
        break;
      case '--environment':
        if (value !== 'development' && value !== 'owner-target-vm') {
          throw new Error('--environment must be development or owner-target-vm');
        }
        options.evidenceEnvironment = value;
        index++;
        break;
      default:
        throw new Error(`unknown option ${option ?? '<missing>'}`);
    }
  }
  return options;
}

/**
 * Return current source identity without modifying repository state.
 * @returns Commit and dirty flag.
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
 * Hardware similarity never establishes that a run occurred on the owner's VM.
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
 * Round one measurement for stable JSON.
 * @param value - Finite measurement.
 * @returns Six-decimal value.
 */
function rounded(value: number): number {
  return Number(value.toFixed(6));
}

/**
 * Summarize numeric samples.
 * @param values - Nonempty finite samples.
 * @returns Basic distribution.
 */
function distribution(values: readonly number[]): Distribution {
  if (values.length === 0) return { count: 0, min: 0, p50: 0, p95: 0, max: 0, mean: 0 };
  const sorted = [...values].sort((left, right) => left - right);
  const percentile = (fraction: number): number => {
    const position = (sorted.length - 1) * fraction;
    const lower = Math.floor(position);
    const upper = Math.ceil(position);
    return sorted[lower]! + (sorted[upper]! - sorted[lower]!) * (position - lower);
  };
  return {
    count: sorted.length,
    min: rounded(sorted[0]!),
    p50: rounded(percentile(0.5)),
    p95: rounded(percentile(0.95)),
    max: rounded(sorted.at(-1)!),
    mean: rounded(sorted.reduce((sum, value) => sum + value, 0) / sorted.length)
  };
}

/**
 * Read exactly the requested bytes at one file position.
 * @param descriptor - Open readable file.
 * @param bytes - Requested byte count.
 * @param position - Absolute byte offset.
 * @returns Exact buffer.
 */
function readExact(descriptor: number, bytes: number, position: number): Buffer {
  const buffer = Buffer.allocUnsafe(bytes);
  let offset = 0;
  while (offset < bytes) {
    const count = fs.readSync(descriptor, buffer, offset, bytes - offset, position + offset);
    if (count <= 0) throw new Error(`unexpected EOF at byte ${position + offset}`);
    offset += count;
  }
  return buffer;
}

/**
 * Append one small in-memory USTAR entry.
 * @param writer - Open archive writer.
 * @param name - Fixed entry name.
 * @param data - Stored entry bytes.
 */
function writeBufferEntry(writer: ArchiveWriter, name: string, data: Buffer): void {
  writer.write(createUstarHeader(name, data.length));
  writer.write(data);
  const padding = ustarPaddingBytes(data.length);
  if (padding > 0) writer.write(Buffer.alloc(padding));
}

/**
 * Append one file-backed USTAR entry with bounded copy memory.
 * @param writer - Open archive writer.
 * @param name - Fixed entry name.
 * @param filePath - Source file.
 * @param storedBytes - Expected source bytes.
 * @param chunkBytes - Copy buffer bound.
 */
function writeFileEntry(
  writer: ArchiveWriter,
  name: string,
  filePath: string,
  storedBytes: number,
  chunkBytes: number
): void {
  writer.write(createUstarHeader(name, storedBytes));
  const descriptor = fs.openSync(filePath, 'r');
  try {
    const buffer = Buffer.allocUnsafe(Math.min(chunkBytes, Math.max(1, storedBytes)));
    let position = 0;
    while (position < storedBytes) {
      const wanted = Math.min(buffer.length, storedBytes - position);
      const count = fs.readSync(descriptor, buffer, 0, wanted, position);
      if (count <= 0) throw new Error(`candidate file ended at ${position} of ${storedBytes}`);
      writer.write(buffer.subarray(0, count));
      position += count;
    }
    if (fs.fstatSync(descriptor).size !== storedBytes) {
      throw new Error('candidate file length changed during archive construction');
    }
  } finally {
    fs.closeSync(descriptor);
  }
  const padding = ustarPaddingBytes(storedBytes);
  if (padding > 0) writer.write(Buffer.alloc(padding));
}

/**
 * Append one raw population buffer in bounded slices.
 * @param writer - Open archive writer.
 * @param name - Raw weight entry name.
 * @param raw - Packed population bytes.
 * @param chunkBytes - Maximum appended slice.
 */
function writeRawEntry(
  writer: ArchiveWriter,
  name: string,
  raw: Buffer,
  chunkBytes: number
): void {
  writer.write(createUstarHeader(name, raw.length));
  for (let offset = 0; offset < raw.length; offset += chunkBytes) {
    writer.write(raw.subarray(offset, Math.min(raw.length, offset + chunkBytes)));
  }
  const padding = ustarPaddingBytes(raw.length);
  if (padding > 0) writer.write(Buffer.alloc(padding));
}

/**
 * Build a fixed-size population index for the prototype container.
 * @param population - Ordered dense genomes.
 * @returns Versioned 48-byte records.
 */
function buildPopulationIndex(population: readonly Genome[]): Buffer {
  const headerBytes = 16;
  const recordBytes = 48;
  const index = Buffer.alloc(headerBytes + population.length * recordBytes);
  index.write('SNPI', 0, 4, 'ascii');
  index.writeUInt16LE(1, 4);
  index.writeUInt16LE(recordBytes, 6);
  index.writeUInt32LE(population.length, 8);
  let weightOffset = 0;
  for (let slot = 0; slot < population.length; slot++) {
    const genome = population[slot]!;
    const offset = headerBytes + slot * recordBytes;
    index.writeUInt32LE(slot, offset);
    index.writeUInt32LE(0, offset + 4);
    index.writeBigUInt64LE(BigInt(weightOffset), offset + 8);
    index.writeUInt32LE(genome.weights.length, offset + 16);
    index.writeUInt32LE(0, offset + 20);
    index.writeBigUInt64LE(0n, offset + 24);
    index.writeDoubleLE(0, offset + 32);
    index.writeUInt32LE(1, offset + 40);
    weightOffset += genome.weights.length;
  }
  return index;
}

/**
 * Generate a bounded shuffled-Zstandard candidate file.
 * @param raw - Packed population bytes.
 * @param candidatePath - Exclusive candidate destination.
 * @param blockBytes - Decoded block bound.
 * @param frameChecksum - Whether frames contain a checksum.
 * @param memory - Trial memory sampler.
 * @returns Candidate and adaptive-selection accounting.
 */
function createCandidate(
  raw: Buffer,
  candidatePath: string,
  blockBytes: number,
  frameChecksum: boolean,
  memory: MemoryPeak
): CandidateResult {
  const started = performance.now();
  const descriptor = fs.openSync(candidatePath, 'wx');
  const logicalHash = createHash('sha256');
  let candidateBytes = 0;
  let blocks = 0;
  try {
    for (let offset = 0; offset < raw.length; offset += blockBytes) {
      const block = raw.subarray(offset, Math.min(raw.length, offset + blockBytes));
      logicalHash.update(block);
      const encoded = encodeShuffledZstdBlocks(block, {
        blockBytes: block.length,
        checksum: frameChecksum
      });
      let written = 0;
      while (written < encoded.length) {
        const count = fs.writeSync(
          descriptor,
          encoded,
          written,
          encoded.length - written,
          null
        );
        if (count <= 0) throw new Error('candidate write made no progress');
        written += count;
      }
      candidateBytes += encoded.length;
      blocks++;
      memory.sample();
    }
  } finally {
    fs.closeSync(descriptor);
  }
  if (fs.statSync(candidatePath).size !== candidateBytes) {
    throw new Error('candidate length does not match encoded accounting');
  }
  return {
    encoding: candidateBytes < raw.length
      ? 'f32le-shuffle4-zstd-v1'
      : 'raw-f32le-v1',
    selectedBytes: Math.min(candidateBytes, raw.length),
    candidateBytes,
    blocks,
    encodeMs: performance.now() - started,
    candidatePath,
    logicalSha256: logicalHash.digest('hex')
  };
}

/**
 * Build the small benchmark-only checkpoint document.
 * @param options - Scenario and fixture identity.
 * @param scenario - Resolved scenario structure.
 * @param architectureKey - Current compiled architecture key.
 * @returns Deterministic small JSON bytes.
 */
function buildCheckpointJson(
  options: RunnerOptions,
  scenario: ReturnType<typeof installStage2Scenario>,
  architectureKey: string
): Buffer {
  const graphDefinition = scenario.graphSpec ?? { type: 'current-default-stack-reference' };
  const graphLayoutDigest = createHash('sha256')
    .update(JSON.stringify(graphDefinition))
    .digest('hex');
  return Buffer.from(`${JSON.stringify({
    magic: 'slither-neuroevo-checkpoint',
    version: 3,
    benchmarkOnly: true,
    boundary: 'generation',
    recurrentBoundary: 'zero-reset',
    runId: `stage2-${options.scenario}-${options.fixture}`,
    seed: STAGE2_WORLD_SEED,
    generation: options.fixture === 'evolved' ? options.evolutionGenerations + 1 : 1,
    completedFixedSteps: 0,
    settings: scenario.settings,
    graphDefinition,
    architectureKey,
    graphLayoutDigest,
    rngState: { benchmarkPlaceholder: true },
    allocatorState: { benchmarkPlaceholder: true }
  })}\n`, 'utf8');
}

/**
 * Build the final manifest from already measured role bytes.
 * @param roles - Ordered role declarations.
 * @param populationCount - Dense slots.
 * @param totalWeightCount - Packed Float32 count.
 * @returns Manifest object and bytes.
 */
function buildManifest(
  roles: PrototypeManifestRole[],
  populationCount: number,
  totalWeightCount: number
): { manifest: PrototypeManifest; bytes: Buffer } {
  const logicalRoles = roles.map(({ role, logicalLength, logicalSha256 }) => ({
    role,
    logicalLength,
    logicalSha256
  }));
  const manifest: PrototypeManifest = {
    magic: 'slither-neuroevo-save',
    archiveVersion: 1,
    kind: 'checkpoint-v3-prototype',
    benchmarkOnly: true,
    roles,
    logicalRoot: computeLogicalRoot(logicalRoles),
    totalStoredRoleBytes: roles.reduce((sum, role) => sum + role.storedLength, 0),
    totalDecodedRoleBytes: roles.reduce((sum, role) => sum + role.logicalLength, 0),
    populationCount,
    totalWeightCount,
    recurrentBoundary: 'zero-reset'
  };
  return { manifest, bytes: Buffer.from(`${JSON.stringify(manifest)}\n`, 'utf8') };
}

/**
 * Parse and minimally type-check the prototype manifest.
 * @param bytes - UTF-8 manifest bytes.
 * @returns Checked manifest.
 */
function parseManifest(bytes: Buffer): PrototypeManifest {
  let value: unknown;
  try {
    value = JSON.parse(bytes.toString('utf8')) as unknown;
  } catch {
    throw new ManagedCheckpointFormatError('MANIFEST_JSON', 'manifest is not valid JSON');
  }
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    throw new ManagedCheckpointFormatError('MANIFEST_SHAPE', 'manifest must be an object');
  }
  const record = value as Record<string, unknown>;
  if (
    record.magic !== 'slither-neuroevo-save' ||
    record.archiveVersion !== 1 ||
    record.kind !== 'checkpoint-v3-prototype' ||
    record.benchmarkOnly !== true ||
    !Array.isArray(record.roles) ||
    typeof record.logicalRoot !== 'string' ||
    typeof record.totalStoredRoleBytes !== 'number' ||
    typeof record.totalDecodedRoleBytes !== 'number' ||
    typeof record.populationCount !== 'number' ||
    typeof record.totalWeightCount !== 'number' ||
    record.recurrentBoundary !== 'zero-reset'
  ) {
    throw new ManagedCheckpointFormatError('MANIFEST_SHAPE', 'manifest fields are invalid');
  }
  const roles: PrototypeManifestRole[] = record.roles.map((item, index) => {
    if (!item || typeof item !== 'object' || Array.isArray(item)) {
      throw new ManagedCheckpointFormatError('MANIFEST_ROLE', `role ${index} is invalid`);
    }
    const role = item as Record<string, unknown>;
    if (
      typeof role.role !== 'string' ||
      typeof role.entry !== 'string' ||
      typeof role.encoding !== 'string' ||
      typeof role.storedLength !== 'number' ||
      typeof role.logicalLength !== 'number' ||
      typeof role.logicalCount !== 'number' ||
      typeof role.recordSize !== 'number' ||
      typeof role.logicalSha256 !== 'string'
    ) {
      throw new ManagedCheckpointFormatError('MANIFEST_ROLE', `role ${index} is invalid`);
    }
    for (const [label, numeric] of [
      ['storedLength', role.storedLength],
      ['logicalLength', role.logicalLength],
      ['logicalCount', role.logicalCount],
      ['recordSize', role.recordSize]
    ] as const) {
      if (!Number.isSafeInteger(numeric) || numeric < 0) {
        throw new ManagedCheckpointFormatError(
          'MANIFEST_ROLE',
          `role ${index} ${label} is invalid`
        );
      }
    }
    if (!/^[0-9a-f]{64}$/u.test(role.logicalSha256)) {
      throw new ManagedCheckpointFormatError('MANIFEST_ROLE', `role ${index} hash is invalid`);
    }
    return {
      role: role.role,
      entry: role.entry,
      encoding: role.encoding,
      storedLength: role.storedLength,
      logicalLength: role.logicalLength,
      logicalCount: role.logicalCount,
      recordSize: role.recordSize,
      logicalSha256: role.logicalSha256
    };
  });
  return {
    magic: 'slither-neuroevo-save',
    archiveVersion: 1,
    kind: 'checkpoint-v3-prototype',
    benchmarkOnly: true,
    roles,
    logicalRoot: record.logicalRoot,
    totalStoredRoleBytes: record.totalStoredRoleBytes,
    totalDecodedRoleBytes: record.totalDecodedRoleBytes,
    populationCount: record.populationCount,
    totalWeightCount: record.totalWeightCount,
    recurrentBoundary: 'zero-reset'
  };
}

/**
 * Strictly scan USTAR structure and manifest without reading numeric payloads.
 * @param filePath - Closed partial or ready file.
 * @returns Entry locations, manifest, and actual bytes read.
 */
function lightweightScan(filePath: string): FileScanResult {
  const descriptor = fs.openSync(filePath, 'r');
  let bytesRead = 0;
  try {
    const fileBytes = fs.fstatSync(descriptor).size;
    const entries: FileEntryLocation[] = [];
    const seen = new Set<string>();
    let offset = 0;
    while (true) {
      if (offset + USTAR_BLOCK_BYTES > fileBytes) {
        throw new ManagedCheckpointFormatError(
          'USTAR_TRUNCATED_HEADER',
          'archive ends inside a USTAR header',
          offset
        );
      }
      const header = readExact(descriptor, USTAR_BLOCK_BYTES, offset);
      bytesRead += header.length;
      if (header.equals(Buffer.alloc(USTAR_BLOCK_BYTES))) {
        if (offset + USTAR_TRAILER_BYTES !== fileBytes) {
          throw new ManagedCheckpointFormatError(
            'USTAR_TRAILER',
            'archive must end with exactly two zero blocks',
            offset
          );
        }
        const second = readExact(descriptor, USTAR_BLOCK_BYTES, offset + USTAR_BLOCK_BYTES);
        bytesRead += second.length;
        if (!second.equals(Buffer.alloc(USTAR_BLOCK_BYTES))) {
          throw new ManagedCheckpointFormatError(
            'USTAR_TRAILER',
            'second USTAR trailer block is not zero',
            offset + USTAR_BLOCK_BYTES
          );
        }
        break;
      }
      const parsed = parseUstarHeader(header, offset);
      if (seen.has(parsed.name)) {
        throw new ManagedCheckpointFormatError(
          'USTAR_DUPLICATE_ENTRY',
          `duplicate entry ${parsed.name}`,
          offset
        );
      }
      seen.add(parsed.name);
      const dataOffset = offset + USTAR_BLOCK_BYTES;
      const dataEnd = dataOffset + parsed.size;
      const padding = ustarPaddingBytes(parsed.size);
      const nextOffset = dataEnd + padding;
      if (
        !Number.isSafeInteger(nextOffset) ||
        dataEnd > fileBytes ||
        nextOffset > fileBytes
      ) {
        throw new ManagedCheckpointFormatError(
          'USTAR_TRUNCATED_ENTRY',
          `entry ${parsed.name} exceeds the archive`,
          offset
        );
      }
      if (padding > 0) {
        const paddingBytes = readExact(descriptor, padding, dataEnd);
        bytesRead += paddingBytes.length;
        if (!paddingBytes.equals(Buffer.alloc(padding))) {
          throw new ManagedCheckpointFormatError(
            'USTAR_NONZERO_PADDING',
            `entry ${parsed.name} has nonzero padding`,
            dataEnd
          );
        }
      }
      entries.push({
        name: parsed.name,
        headerOffset: offset,
        dataOffset,
        size: parsed.size
      });
      offset = nextOffset;
    }
    if (entries.at(-1)?.name !== MANIFEST_ENTRY) {
      throw new ManagedCheckpointFormatError(
        'USTAR_MANIFEST_ORDER',
        'manifest.json must be the final regular entry'
      );
    }
    const manifestLocation = entries.at(-1)!;
    if (manifestLocation.size > MAX_MANIFEST_BYTES) {
      throw new ManagedCheckpointFormatError(
        'MANIFEST_LIMIT',
        `manifest exceeds ${MAX_MANIFEST_BYTES} bytes`
      );
    }
    const manifestBytes = readExact(
      descriptor,
      manifestLocation.size,
      manifestLocation.dataOffset
    );
    bytesRead += manifestBytes.length;
    const manifest = parseManifest(manifestBytes);
    const expectedRoleNames = [...LOGICAL_ROLE_ORDER];
    if (
      manifest.roles.length !== expectedRoleNames.length ||
      manifest.roles.some((role, index) => role.role !== expectedRoleNames[index])
    ) {
      throw new ManagedCheckpointFormatError(
        'MANIFEST_ROLE_ORDER',
        'manifest logical roles are missing or out of order'
      );
    }
    const expectedEntries = [...manifest.roles.map(role => role.entry), MANIFEST_ENTRY];
    if (
      entries.length !== expectedEntries.length ||
      entries.some((entry, index) => entry.name !== expectedEntries[index])
    ) {
      throw new ManagedCheckpointFormatError(
        'MANIFEST_ENTRY_SET',
        'USTAR entries do not match the manifest role order'
      );
    }
    for (let index = 0; index < manifest.roles.length; index++) {
      if (entries[index]!.size !== manifest.roles[index]!.storedLength) {
        throw new ManagedCheckpointFormatError(
          'MANIFEST_STORED_LENGTH',
          `stored length differs for ${manifest.roles[index]!.role}`
        );
      }
    }
    if (
      manifest.totalStoredRoleBytes !==
        manifest.roles.reduce((sum, role) => sum + role.storedLength, 0) ||
      manifest.totalDecodedRoleBytes !==
        manifest.roles.reduce((sum, role) => sum + role.logicalLength, 0)
    ) {
      throw new ManagedCheckpointFormatError(
        'MANIFEST_TOTALS',
        'manifest aggregate lengths are inconsistent'
      );
    }
    verifyLogicalRoot(manifest.roles, manifest.logicalRoot);
    return { entries, manifest, bytesRead, archiveBytes: fileBytes };
  } finally {
    fs.closeSync(descriptor);
  }
}

/**
 * Stream one raw entry through SHA-256 without retaining another full copy.
 * @param descriptor - Open archive.
 * @param location - Raw entry location.
 * @param chunkBytes - Read buffer bound.
 * @returns Hash and bytes read.
 */
function hashRawEntry(
  descriptor: number,
  location: FileEntryLocation,
  chunkBytes: number
): { sha256: string; bytesRead: number } {
  const hash = createHash('sha256');
  const buffer = Buffer.allocUnsafe(Math.min(chunkBytes, Math.max(1, location.size)));
  let position = 0;
  while (position < location.size) {
    const wanted = Math.min(buffer.length, location.size - position);
    const count = fs.readSync(
      descriptor,
      buffer,
      0,
      wanted,
      location.dataOffset + position
    );
    if (count <= 0) throw new Error(`entry ${location.name} ended early`);
    hash.update(buffer.subarray(0, count));
    position += count;
  }
  return { sha256: hash.digest('hex'), bytesRead: location.size };
}

/**
 * Decode the bounded shuffled-Zstandard weight entry and hash logical bytes.
 * @param descriptor - Open archive.
 * @param location - Compressed entry.
 * @param maxBlockBytes - Decoded block bound.
 * @param maxDecodedBytes - Aggregate logical bound.
 * @param memory - Trial memory sampler.
 * @returns Decoded hash and byte accounting.
 */
function decodeCompressedEntry(
  descriptor: number,
  location: FileEntryLocation,
  maxBlockBytes: number,
  maxDecodedBytes: number,
  memory: MemoryPeak
): { sha256: string; storedRead: number; decodedBytes: number } {
  const hash = createHash('sha256');
  let storedRead = 0;
  let decodedBytes = 0;
  while (storedRead < location.size) {
    if (storedRead + 12 > location.size) {
      throw new ManagedCheckpointFormatError(
        'SHUFFLED_BLOCK_HEADER',
        'compressed entry ends inside a block header',
        location.dataOffset + storedRead
      );
    }
    const header = readExact(descriptor, 12, location.dataOffset + storedRead);
    if (!header.subarray(0, 4).equals(SHUFFLED_BLOCK_MAGIC)) {
      throw new ManagedCheckpointFormatError(
        'SHUFFLED_BLOCK_MAGIC',
        'compressed entry contains an unsupported block marker',
        location.dataOffset + storedRead
      );
    }
    const floatCount = header.readUInt32LE(4);
    const frameBytes = header.readUInt32LE(8);
    const declaredDecodedBytes = floatCount * 4;
    const remainingDecodedBytes = maxDecodedBytes - decodedBytes;
    const recordBytes = 12 + frameBytes;
    if (
      floatCount === 0 ||
      !Number.isSafeInteger(declaredDecodedBytes) ||
      declaredDecodedBytes > maxBlockBytes ||
      declaredDecodedBytes > remainingDecodedBytes
    ) {
      throw new ManagedCheckpointFormatError(
        'SHUFFLED_BLOCK_LIMIT',
        'compressed block exceeds its decoded byte limit',
        location.dataOffset + storedRead
      );
    }
    if (
      frameBytes === 0 ||
      frameBytes > maxBlockBytes + MAX_ZSTD_FRAME_OVERHEAD_BYTES ||
      !Number.isSafeInteger(recordBytes) ||
      storedRead + recordBytes > location.size
    ) {
      throw new ManagedCheckpointFormatError(
        'SHUFFLED_BLOCK_FRAME',
        'compressed block exceeds its USTAR entry',
        location.dataOffset + storedRead
      );
    }
    const frame = readExact(descriptor, frameBytes, location.dataOffset + storedRead + 12);
    const decoded = decodeShuffledZstdBlocks(Buffer.concat([header, frame]), {
      maxBlockBytes,
      maxTotalDecodedBytes: maxDecodedBytes - decodedBytes
    });
    hash.update(decoded);
    decodedBytes += decoded.length;
    storedRead += recordBytes;
    memory.sample();
  }
  return { sha256: hash.digest('hex'), storedRead, decodedBytes };
}

/**
 * Fully decode every role and verify one logical digest/root layer.
 * @param filePath - Closed archive path.
 * @param scan - Existing strict lightweight scan.
 * @param blockBytes - Decode scratch bound.
 * @param expectedWeightSha256 - Fixture population hash.
 * @param memory - Trial memory sampler.
 * @returns Full validation accounting.
 */
function fullDecode(
  filePath: string,
  scan: FileScanResult,
  blockBytes: number,
  expectedWeightSha256: string,
  memory: MemoryPeak
): FullDecodeResult {
  const descriptor = fs.openSync(filePath, 'r');
  let bytesRead = scan.bytesRead;
  let decodedBytes = 0;
  let weightSha256 = '';
  try {
    for (let index = 0; index < scan.manifest.roles.length; index++) {
      const role = scan.manifest.roles[index]!;
      const location = scan.entries[index]!;
      let result: { sha256: string; bytesRead: number; decodedBytes: number };
      if (role.encoding === 'f32le-shuffle4-zstd-v1') {
        const decoded = decodeCompressedEntry(
          descriptor,
          location,
          blockBytes,
          role.logicalLength,
          memory
        );
        result = {
          sha256: decoded.sha256,
          bytesRead: decoded.storedRead,
          decodedBytes: decoded.decodedBytes
        };
      } else {
        const raw = hashRawEntry(descriptor, location, blockBytes);
        result = {
          sha256: raw.sha256,
          bytesRead: raw.bytesRead,
          decodedBytes: location.size
        };
      }
      if (
        result.decodedBytes !== role.logicalLength ||
        result.sha256 !== role.logicalSha256
      ) {
        throw new ManagedCheckpointFormatError(
          'LOGICAL_ROLE_MISMATCH',
          `logical role ${role.role} failed length or SHA-256`
        );
      }
      if (role.role === 'population-weights') weightSha256 = result.sha256;
      bytesRead += result.bytesRead;
      decodedBytes += result.decodedBytes;
    }
  } finally {
    fs.closeSync(descriptor);
  }
  verifyLogicalRoot(scan.manifest.roles, scan.manifest.logicalRoot);
  if (weightSha256 !== expectedWeightSha256) {
    throw new ManagedCheckpointFormatError(
      'WEIGHT_FIXTURE_MISMATCH',
      'decoded population does not match the named fixture'
    );
  }
  return {
    bytesRead,
    decodedBytes,
    logicalRoot: scan.manifest.logicalRoot,
    weightSha256
  };
}

/**
 * Attempt the platform's directory synchronization operation.
 * Linux failure is fatal; Windows records Node's expected EPERM limitation.
 * @param directory - Managed directory containing the renamed file.
 * @returns Exact platform result.
 */
function syncDirectory(directory: string): DirectorySyncResult {
  const started = performance.now();
  let descriptor: number | null = null;
  try {
    descriptor = fs.openSync(directory, 'r');
    fs.fsyncSync(descriptor);
    return {
      attempted: true,
      supported: true,
      elapsedMs: rounded(performance.now() - started),
      errorCode: null
    };
  } catch (error) {
    const code = (error as NodeJS.ErrnoException).code ?? 'UNKNOWN';
    if (process.platform !== 'win32' || code !== 'EPERM') throw error;
    return {
      attempted: true,
      supported: false,
      elapsedMs: rounded(performance.now() - started),
      errorCode: code
    };
  } finally {
    if (descriptor !== null) fs.closeSync(descriptor);
  }
}

/**
 * Expected complete USTAR bytes for known entry payload lengths.
 * @param sizes - Ordered stored entry sizes.
 * @returns Header, payload, padding and trailer bytes.
 */
function expectedArchiveBytes(sizes: readonly number[]): number {
  return sizes.reduce(
    (sum, size) => sum + USTAR_BLOCK_BYTES + size + ustarPaddingBytes(size),
    USTAR_TRAILER_BYTES
  );
}

/**
 * Execute one policy trial in its own managed directory.
 * @param temporaryRoot - Validated disposable root.
 * @param options - Runner settings.
 * @param variant - Validation choice.
 * @param trial - One-based trial number.
 * @param rawWeights - Named fixture's packed bytes.
 * @param population - Named fixture's dense genomes.
 * @param checkpointJson - Small prototype state.
 * @param populationIndex - Small fixed-record index.
 * @param expectedWeightSha256 - Fixture population hash.
 * @returns Complete trial evidence and final archive path for optional faults.
 */
function runTrial(
  temporaryRoot: string,
  options: RunnerOptions,
  variant: VariantDefinition,
  trial: number,
  rawWeights: Buffer,
  population: readonly Genome[],
  checkpointJson: Buffer,
  populationIndex: Buffer,
  expectedWeightSha256: string
): { result: TrialResult; finalPath: string } {
  const trialRoot = path.join(temporaryRoot, `${trial}-${variant.name}`);
  fs.mkdirSync(trialRoot);
  const candidatePath = path.join(trialRoot, 'weights-candidate.partial');
  const partialPath = path.join(trialRoot, `${randomUUID()}.checkpoint-v3.partial`);
  const memory = new MemoryPeak();
  memory.sample();
  const barrierStarted = performance.now();
  const candidate = createCandidate(
    rawWeights,
    candidatePath,
    options.blockBytes,
    variant.frameChecksum,
    memory
  );
  if (candidate.logicalSha256 !== expectedWeightSha256) {
    throw new Error('write-path logical population hash differs from the named fixture');
  }
  const weightsEntry = candidate.encoding === 'f32le-shuffle4-zstd-v1'
    ? COMPRESSED_WEIGHTS_ENTRY
    : RAW_WEIGHTS_ENTRY;
  const recurrent = Buffer.alloc(0);
  const roles: PrototypeManifestRole[] = [
    {
      role: 'checkpoint',
      entry: CHECKPOINT_ENTRY,
      encoding: 'json-utf8-v1',
      storedLength: checkpointJson.length,
      logicalLength: checkpointJson.length,
      logicalCount: 1,
      recordSize: 0,
      logicalSha256: logicalSha256(checkpointJson)
    },
    {
      role: 'population-index',
      entry: POPULATION_INDEX_ENTRY,
      encoding: 'population-index-le-v1',
      storedLength: populationIndex.length,
      logicalLength: populationIndex.length,
      logicalCount: population.length,
      recordSize: 48,
      logicalSha256: logicalSha256(populationIndex)
    },
    {
      role: 'population-weights',
      entry: weightsEntry,
      encoding: candidate.encoding,
      storedLength: candidate.selectedBytes,
      logicalLength: rawWeights.length,
      logicalCount: rawWeights.length / 4,
      recordSize: 4,
      logicalSha256: expectedWeightSha256
    },
    {
      role: 'population-recurrent',
      entry: RECURRENT_ENTRY,
      encoding: 'raw-f32le-v1',
      storedLength: 0,
      logicalLength: 0,
      logicalCount: 0,
      recordSize: 4,
      logicalSha256: logicalSha256(recurrent)
    }
  ];
  const { manifest, bytes: manifestBytes } = buildManifest(
    roles,
    population.length,
    rawWeights.length / 4
  );
  const archiveStarted = performance.now();
  const writer = new ArchiveWriter(partialPath);
  let fileFsyncMs = 0;
  let archiveBytes = 0;
  try {
    writeBufferEntry(writer, CHECKPOINT_ENTRY, checkpointJson);
    writeBufferEntry(writer, POPULATION_INDEX_ENTRY, populationIndex);
    if (candidate.encoding === 'f32le-shuffle4-zstd-v1') {
      writeFileEntry(
        writer,
        weightsEntry,
        candidate.candidatePath,
        candidate.selectedBytes,
        options.blockBytes
      );
    } else {
      writeRawEntry(writer, weightsEntry, rawWeights, options.blockBytes);
    }
    writeBufferEntry(writer, RECURRENT_ENTRY, recurrent);
    writeBufferEntry(writer, MANIFEST_ENTRY, manifestBytes);
    writer.write(Buffer.alloc(USTAR_TRAILER_BYTES));
    fileFsyncMs = writer.sync();
    archiveBytes = writer.finishBytes();
  } finally {
    writer.close();
  }
  const archiveAssemblyMs = performance.now() - archiveStarted - fileFsyncMs;
  const expectedBytes = expectedArchiveBytes([
    checkpointJson.length,
    populationIndex.length,
    candidate.selectedBytes,
    recurrent.length,
    manifestBytes.length
  ]);
  if (
    archiveBytes !== expectedBytes ||
    fs.statSync(partialPath).size !== expectedBytes
  ) {
    throw new Error('partial archive did not reach its expected complete length');
  }
  fs.rmSync(candidatePath);
  memory.sample();

  const validationStarted = performance.now();
  let validationRead = 0;
  let decodedDuringValidation = 0;
  if (variant.validation === 'lightweight-scan') {
    const scan = lightweightScan(partialPath);
    validationRead = scan.bytesRead;
    if (scan.manifest.logicalRoot !== manifest.logicalRoot) {
      throw new Error('lightweight scan returned the wrong logical root');
    }
  } else if (variant.validation === 'full-decode') {
    const scan = lightweightScan(partialPath);
    const decoded = fullDecode(
      partialPath,
      scan,
      options.blockBytes,
      expectedWeightSha256,
      memory
    );
    validationRead = decoded.bytesRead;
    decodedDuringValidation = decoded.decodedBytes;
  }
  const validationMs = performance.now() - validationStarted;
  const finalPath = path.join(trialRoot, `${manifest.logicalRoot}.checkpoint-v3.ready`);
  const renameStarted = performance.now();
  fs.renameSync(partialPath, finalPath);
  const renameMs = performance.now() - renameStarted;
  const directorySync = syncDirectory(trialRoot);
  memory.sample();
  const totalBarrierMs = performance.now() - barrierStarted;
  return {
    result: {
      variant: variant.name,
      trial,
      frameChecksum: variant.frameChecksum,
      validationPolicy: variant.validation,
      selectedEncoding: candidate.encoding,
      compressedBlocks: candidate.blocks,
      logicalRoot: manifest.logicalRoot,
      bytes: {
        rawWeights: rawWeights.length,
        shuffledCandidate: candidate.candidateBytes,
        selectedWeightEntry: candidate.selectedBytes,
        archive: archiveBytes,
        applicationWritten: candidate.candidateBytes + archiveBytes,
        validationRead,
        decodedDuringValidation
      },
      timingsMs: {
        candidateEncodeAndLogicalHash: rounded(candidate.encodeMs),
        archiveAssembly: rounded(Math.max(0, archiveAssemblyMs)),
        fileFsync: rounded(fileFsyncMs),
        validation: rounded(validationMs),
        rename: rounded(renameMs),
        directorySync: directorySync.elapsedMs,
        totalBarrier: rounded(totalBarrierMs)
      },
      sampledMemoryPeak: memory.result(),
      directorySync,
      accepted: true
    },
    finalPath
  };
}

/**
 * Capture whether one operation accepts or rejects a mutated file.
 * @param operation - Validation operation.
 * @returns Acceptance and stable error code.
 */
function captureValidation(operation: () => unknown): { accepted: boolean; code: string | null } {
  try {
    operation();
    return { accepted: true, code: null };
  } catch (error) {
    return {
      accepted: false,
      code: error instanceof ManagedCheckpointFormatError
        ? error.code
        : ((error as NodeJS.ErrnoException).code ?? error.constructor.name)
    };
  }
}

/**
 * Flip one file byte in place.
 * @param filePath - Mutable disposable copy.
 * @param offset - Absolute byte offset.
 */
function flipFileByte(filePath: string, offset: number): void {
  const descriptor = fs.openSync(filePath, 'r+');
  try {
    const byte = readExact(descriptor, 1, offset);
    byte[0] = byte[0]! ^ 0x01;
    fs.writeSync(descriptor, byte, 0, 1, offset);
  } finally {
    fs.closeSync(descriptor);
  }
}

/**
 * Exercise the diagnostic distinction between scan and full decode.
 * @param sourcePath - Valid ready archive.
 * @param temporaryRoot - Disposable root.
 * @param blockBytes - Decode block bound.
 * @param weightSha256 - Named fixture hash.
 * @returns Actual mutation matrix.
 */
function runFaultMatrix(
  sourcePath: string,
  temporaryRoot: string,
  blockBytes: number,
  weightSha256: string
): FaultObservation[] {
  const baselineScan = lightweightScan(sourcePath);
  const weightEntry = baselineScan.entries.find(entry => (
    entry.name === RAW_WEIGHTS_ENTRY || entry.name === COMPRESSED_WEIGHTS_ENTRY
  ));
  const manifestEntry = baselineScan.entries.find(entry => entry.name === MANIFEST_ENTRY);
  if (!weightEntry || !manifestEntry) throw new Error('fault baseline lacks required entries');
  const mutations: Array<{
    name: string;
    mutate: (filePath: string) => void;
  }> = [
    {
      name: 'truncate-one-terminal-block',
      mutate: filePath => fs.truncateSync(filePath, fs.statSync(filePath).size - USTAR_BLOCK_BYTES)
    },
    {
      name: 'corrupt-first-header',
      mutate: filePath => flipFileByte(filePath, 12)
    },
    {
      name: 'corrupt-weight-payload',
      mutate: filePath => flipFileByte(
        filePath,
        weightEntry.dataOffset + Math.min(32, Math.max(0, weightEntry.size - 1))
      )
    },
    {
      name: 'corrupt-manifest-root-text',
      mutate: filePath => {
        const descriptor = fs.openSync(filePath, 'r+');
        try {
          const bytes = readExact(descriptor, manifestEntry.size, manifestEntry.dataOffset);
          const marker = Buffer.from(baselineScan.manifest.logicalRoot, 'ascii');
          const relative = bytes.indexOf(marker);
          if (relative < 0) throw new Error('logical root is absent from manifest bytes');
          const replacement = bytes[relative] === 0x30 ? 0x31 : 0x30;
          fs.writeSync(descriptor, Buffer.from([replacement]), 0, 1, manifestEntry.dataOffset + relative);
        } finally {
          fs.closeSync(descriptor);
        }
      }
    }
  ];
  return mutations.map((mutation, index) => {
    const copy = path.join(temporaryRoot, `fault-${index}.slither-save`);
    fs.copyFileSync(sourcePath, copy, fs.constants.COPYFILE_EXCL);
    try {
      mutation.mutate(copy);
      const lightweight = captureValidation(() => lightweightScan(copy));
      const full = captureValidation(() => {
        const scan = lightweightScan(copy);
        fullDecode(copy, scan, blockBytes, weightSha256, new MemoryPeak());
      });
      return { mutation: mutation.name, lightweight, fullDecode: full };
    } finally {
      fs.rmSync(copy, { force: true });
    }
  });
}

/**
 * Corrupt one compressed weight payload and compare scan versus full decode.
 * @param sourcePath - Valid ready archive.
 * @param temporaryRoot - Disposable root.
 * @param blockBytes - Decode block bound.
 * @param weightSha256 - Named fixture hash.
 * @param mutation - Artifact label identifying checksum policy.
 * @returns One actual diagnosis observation.
 */
function runPayloadFault(
  sourcePath: string,
  temporaryRoot: string,
  blockBytes: number,
  weightSha256: string,
  mutation: string
): FaultObservation {
  const baseline = lightweightScan(sourcePath);
  const weights = baseline.entries.find(entry => entry.name === COMPRESSED_WEIGHTS_ENTRY);
  if (!weights) throw new Error('checksummed fault source did not select compressed weights');
  const copy = path.join(temporaryRoot, `${mutation}.slither-save`);
  fs.copyFileSync(sourcePath, copy, fs.constants.COPYFILE_EXCL);
  try {
    flipFileByte(copy, weights.dataOffset + Math.min(32, Math.max(0, weights.size - 1)));
    const lightweight = captureValidation(() => lightweightScan(copy));
    const full = captureValidation(() => {
      const scan = lightweightScan(copy);
      fullDecode(copy, scan, blockBytes, weightSha256, new MemoryPeak());
    });
    return { mutation, lightweight, fullDecode: full };
  } finally {
    fs.rmSync(copy, { force: true });
  }
}

/**
 * Summarize results for each validation variant.
 * @param trials - Raw repeated trials.
 * @returns Summary distributions without discarding raw trials.
 */
function summarizeVariants(trials: readonly TrialResult[]): Record<string, unknown> {
  return Object.fromEntries(VARIANTS.map(variant => {
    const selected = trials.filter(trial => trial.variant === variant.name);
    return [variant.name, {
      frameChecksum: variant.frameChecksum,
      validationPolicy: variant.validation,
      archiveBytes: distribution(selected.map(trial => trial.bytes.archive)),
      candidateBytes: distribution(selected.map(trial => trial.bytes.shuffledCandidate)),
      candidateEncodeAndLogicalHashMs: distribution(
        selected.map(trial => trial.timingsMs.candidateEncodeAndLogicalHash)
      ),
      archiveAssemblyMs: distribution(selected.map(trial => trial.timingsMs.archiveAssembly)),
      fileFsyncMs: distribution(selected.map(trial => trial.timingsMs.fileFsync)),
      validationMs: distribution(selected.map(trial => trial.timingsMs.validation)),
      renameMs: distribution(selected.map(trial => trial.timingsMs.rename)),
      totalBarrierMs: distribution(selected.map(trial => trial.timingsMs.totalBarrier)),
      validationReadBytes: distribution(selected.map(trial => trial.bytes.validationRead)),
      sampledPeakRssBytes: distribution(selected.map(trial => trial.sampledMemoryPeak.rss))
    }];
  }));
}

/**
 * Require one named fault observation.
 * @param matrix - Actual fault observations.
 * @param mutation - Exact mutation label.
 * @returns Matching observation.
 */
function requireFault(
  matrix: readonly FaultObservation[],
  mutation: string
): FaultObservation {
  const result = matrix.find(item => item.mutation === mutation);
  if (!result) throw new Error(`required fault observation ${mutation} is missing`);
  return result;
}

/**
 * Abort rather than retaining an artifact with a failed or vacuous assertion.
 * @param assertions - Named evidence invariants.
 */
function enforceAssertions(assertions: Readonly<Record<string, boolean>>): void {
  const failed = Object.entries(assertions)
    .filter(([, passed]) => !passed)
    .map(([name]) => name);
  if (failed.length > 0) {
    throw new Error(`managed-checkpoint evidence assertions failed: ${failed.join(', ')}`);
  }
}

/**
 * Execute the complete named-fixture comparison in one disposable root.
 * @param options - Validated CLI options.
 * @param source - Captured source identity before output creation.
 * @returns Machine-readable evidence.
 */
function runBenchmark(
  options: RunnerOptions,
  source: { commit: string; dirty: boolean }
): Record<string, unknown> {
  const provenance = captureEnvironmentProvenance(options.evidenceEnvironment);
  const prefix = 'slither-stage2-checkpoint-validation-';
  const temporaryRoot = fs.mkdtempSync(path.join(os.tmpdir(), prefix));
  const resolvedRoot = path.resolve(temporaryRoot);
  const expectedParent = path.resolve(os.tmpdir());
  let safeTemporaryRoot = false;
  try {
    if (
      path.dirname(resolvedRoot) !== expectedParent ||
      !path.basename(resolvedRoot).startsWith(prefix)
    ) {
      throw new Error(`refusing unsafe temporary root ${resolvedRoot}`);
    }
    safeTemporaryRoot = true;
    const freeDiskBefore = fs.statfsSync(resolvedRoot);
    const resourceBeforeKiB = process.resourceUsage().maxRSS;
    const scenario = installStage2Scenario(options.scenario);
    const world = new World(scenario.settings, { seed: STAGE2_WORLD_SEED });
    const population = options.fixture === 'evolved'
      ? evolvePopulationFixture(world.population, world.arch, options.evolutionGenerations)
      : world.population;
    const rawWeights = packPopulationWeights(population);
    if (rawWeights.length > MAX_RAW_BYTES) {
      throw new RangeError(`fixture exceeds the ${MAX_RAW_BYTES}-byte runner limit`);
    }
    const requiredTemporaryHeadroom = rawWeights.length * 4 + 64 * 1024 * 1024;
    const freeBytes = freeDiskBefore.bavail * freeDiskBefore.bsize;
    if (requiredTemporaryHeadroom > freeBytes) {
      throw new Error(
        `fixture requires ${requiredTemporaryHeadroom} temporary bytes but only ${freeBytes} are free`
      );
    }
    const weightSha256 = logicalSha256(rawWeights);
    const populationIndex = buildPopulationIndex(population);
    const checkpointJson = buildCheckpointJson(options, scenario, world.arch.key);
    const trialResults: TrialResult[] = [];
    let faultSource: string | null = null;
    let checksumFaultSource: string | null = null;
    for (let trial = 1; trial <= options.trials; trial++) {
      const rotated = [...VARIANTS.slice((trial - 1) % VARIANTS.length), ...VARIANTS.slice(
        0,
        (trial - 1) % VARIANTS.length
      )];
      for (const variant of rotated) {
        const measured = runTrial(
          resolvedRoot,
          options,
          variant,
          trial,
          rawWeights,
          population,
          checkpointJson,
          populationIndex,
          weightSha256
        );
        trialResults.push(measured.result);
        let retainForFaults = false;
        if (faultSource === null && variant.name === 'single-pass') {
          faultSource = measured.finalPath;
          retainForFaults = true;
        } else if (checksumFaultSource === null && variant.name === 'frame-checksum') {
          checksumFaultSource = measured.finalPath;
          retainForFaults = true;
        }
        if (!retainForFaults) {
          fs.rmSync(path.dirname(measured.finalPath), { recursive: true, force: true });
        }
      }
    }
    if (!faultSource || !checksumFaultSource) {
      throw new Error('required fault sources were not produced');
    }
    const faultMatrix = runFaultMatrix(
      faultSource,
      resolvedRoot,
      options.blockBytes,
      weightSha256
    );
    faultMatrix.push(runPayloadFault(
      checksumFaultSource,
      resolvedRoot,
      options.blockBytes,
      weightSha256,
      'corrupt-checksummed-weight-payload'
    ));
    const roots = new Set(trialResults.map(result => result.logicalRoot));
    const encodings = new Set(trialResults.map(result => result.selectedEncoding));
    const fullDecodeTrials = trialResults.filter(
      result => result.validationPolicy === 'full-decode'
    );
    const truncatedFault = requireFault(faultMatrix, 'truncate-one-terminal-block');
    const headerFault = requireFault(faultMatrix, 'corrupt-first-header');
    const payloadFault = requireFault(faultMatrix, 'corrupt-weight-payload');
    const manifestFault = requireFault(faultMatrix, 'corrupt-manifest-root-text');
    const checksumFault = requireFault(faultMatrix, 'corrupt-checksummed-weight-payload');
    for (const entry of fs.readdirSync(resolvedRoot)) {
      fs.rmSync(path.join(resolvedRoot, entry), { recursive: true, force: true });
    }
    const temporaryDirectoryEmpty = fs.readdirSync(resolvedRoot).length === 0;
    const freeDiskAfterCleanup = fs.statfsSync(resolvedRoot);
    const everyDirectorySyncSucceeded = trialResults.every(
      result =>
        result.directorySync.attempted &&
        result.directorySync.supported &&
        result.directorySync.errorCode === null
    );
    const assertions = {
      everyTrialAccepted:
        trialResults.length === options.trials * VARIANTS.length &&
        trialResults.every(result => result.accepted),
      oneLogicalRootAcrossVariants: roots.size === 1,
      selectedEncodingConsistentAcrossVariants: encodings.size === 1,
      everyFullDecodeReadLogicalPopulation:
        fullDecodeTrials.length === options.trials &&
        fullDecodeTrials.every(
          result => result.bytes.decodedDuringValidation >= rawWeights.length
        ),
      truncationAndHeaderCaughtByLightweightScan:
        !truncatedFault.lightweight.accepted &&
        !headerFault.lightweight.accepted,
      payloadCorruptionRequiresDecode:
        payloadFault.lightweight.accepted &&
        !payloadFault.fullDecode.accepted &&
        payloadFault.fullDecode.code === 'LOGICAL_ROLE_MISMATCH',
      manifestRootCorruptionCaughtWithoutPayloadDecode:
        !manifestFault.lightweight.accepted,
      frameChecksumProducesCodecLevelCorruptionDiagnosis:
        checksumFault.lightweight.accepted &&
        !checksumFault.fullDecode.accepted &&
        checksumFault.fullDecode.code === 'SHUFFLED_BLOCK_DECODE',
      temporaryDirectoryEmptyBeforeRootRemoval: temporaryDirectoryEmpty
    };
    enforceAssertions(assertions);
    return {
      schema: 'slither-stage2-managed-checkpoint-validation',
      version: 2,
      evidenceClass: provenance.ownerTargetVmValidated
        ? 'new measured target-VM prototype result'
        : 'new measured development-machine result',
      caveat: provenance.ownerTargetVmValidated
        ? 'Disposable Node built-in USTAR/Zstandard measurement captured on the Ryzen 7 2700 Debian target VM. It is not the production Rust checkpoint-v3 writer, is not wired to SQLite or authority, and does not construct restored Rust state.'
        : 'Disposable Node built-in USTAR/Zstandard measurement only. It is not a production checkpoint-v3 writer, not wired to SQLite or authority, does not construct restored Rust state, and is not Ryzen 7 2700 Debian evidence.',
      source,
      command: process.argv,
      environment: {
        capturedAt: new Date().toISOString(),
        platform: process.platform,
        architecture: process.arch,
        osType: os.type(),
        osRelease: os.release(),
        osVersion: os.version(),
        provenance,
        hostname: provenance.hostname,
        cpuModel: provenance.cpuModel,
        logicalCpuCount: provenance.logicalCpuCount,
        totalMemoryBytes: provenance.totalMemoryBytes,
        node: process.version,
        zstd: process.versions.zstd ?? null,
        tempFilesystem: {
          root: path.parse(resolvedRoot).root,
          type: freeDiskBefore.type,
          blockSize: freeDiskBefore.bsize,
          freeBytesBefore: freeDiskBefore.bavail * freeDiskBefore.bsize,
          freeBytesAfterFixtureCleanup:
            freeDiskAfterCleanup.bavail * freeDiskAfterCleanup.bsize
        }
      },
      fixture: {
        scenario,
        fixtureKind: options.fixture,
        evolutionGenerations: options.fixture === 'evolved'
          ? options.evolutionGenerations
          : 0,
        seed: STAGE2_WORLD_SEED,
        populationCount: population.length,
        weightsPerGenome: population[0]?.weights.length ?? 0,
        totalWeightCount: rawWeights.length / 4,
        rawWeightBytes: rawWeights.length,
        rawWeightSha256: weightSha256,
        architectureKey: world.arch.key,
        populationIndexBytes: populationIndex.length,
        checkpointJsonBytes: checkpointJson.length
      },
      prototype: {
        format: 'strict-ustar-v1-prototype',
        benchmarkOnly: true,
        logicalRoles: LOGICAL_ROLE_ORDER,
        logicalRootExcludesManifestToAvoidSelfReference: true,
        blockBytes: options.blockBytes,
        frameCount: trialResults[0]?.compressedBlocks ?? 0,
        adaptiveEncodings: ['raw-f32le-v1', 'f32le-shuffle4-zstd-v1'],
        selectedEncodings: [...encodings],
        oneLogicalRootAcrossVariants: roots.size === 1
      },
      variants: summarizeVariants(trialResults),
      rawTrials: trialResults,
      faultMatrix,
      assertions,
      resource: {
        processMaxRssBeforeBytes: resourceBeforeKiB * 1024,
        processMaxRssAfterBytes: process.resourceUsage().maxRSS * 1024,
        requiredTemporaryHeadroomBytes: requiredTemporaryHeadroom
      },
      diagnosticValue: {
        singlePass:
          'Detects writer/codec completion errors, file fsync failure and final-length mismatch; latent stored corruption remains discoverable only on a later decode.',
        frameChecksum:
          'For this injected bit flip, the checksummed frame failed at codec decode while the unchecked frame decoded and then failed its logical SHA-256. It performs no extra write-time read and is not claimed to improve every corruption diagnosis.',
        lightweightScan:
          'Reopens headers, padding, role set, final manifest, declared stored lengths, aggregate counts, logical-root declaration and exact trailer without reading numeric payloads.',
        fullDecode:
          'Reads and bounded-decodes every role, recomputes the single per-role logical hashes and root, and verifies population bits by SHA-256.'
      },
      limitations: {
        production:
          'The real writer/reader remains Rust Stage 3 work; Node never becomes the production archive codec.',
        memory:
          'The named TypeScript reference fixture already owns per-genome weights and this harness packs one contiguous raw population buffer. Codec scratch is block-bounded, but this is not a Rust-state RSS measurement.',
        zstdAdmission:
          'The prototype checks its decoded block envelope before Node decompression but cannot preflight the Zstandard frame window. The Rust importer must cap declared content/window before allocation.',
        durability:
          everyDirectorySyncSucceeded
            ? `File fsync, same-filesystem rename and parent-directory fsync succeeded in all ${trialResults.length} measured trials. This prototype timing is not a power-loss durability test.`
            : process.platform === 'win32'
              ? 'File fsync and same-filesystem rename succeeded, while Windows Node returned EPERM for parent-directory fsync. That limitation is retained rather than described as a power-loss guarantee; every other sync error is fatal, and Debian parent-directory fsync remains mandatory.'
              : 'At least one measured parent-directory fsync was unsupported or failed. The production writer cannot claim durable publication until that failure is resolved.',
        scope:
          'No SQLite pointer, persistence worker, restored world, retention, import/export HTTP, legacy compatibility, recovery branch, exhaustive malformed-input corpus or crash-state matrix is implemented here.',
        target:
          provenance.ownerTargetVmValidated
            ? 'This artifact supplies Ryzen 7 2700 Debian prototype timing. Final policy selection and the archive throughput/RSS gate still require the production Rust implementation.'
            : 'Final policy selection and the archive throughput/RSS gate still require the Rust implementation and Ryzen 7 2700 Debian measurements.'
      }
    };
  } finally {
    try {
      resetCFGToDefaults();
    } finally {
      if (safeTemporaryRoot) {
        fs.rmSync(resolvedRoot, { recursive: true, force: true });
      }
    }
  }
}

/** Execute the Stage 2 CLI. */
function main(): void {
  const options = parseOptions(process.argv.slice(2));
  const source = sourceIdentity();
  const result = runBenchmark(options, source);
  const json = `${JSON.stringify(result, null, 2)}\n`;
  if (options.outputPath) {
    fs.mkdirSync(path.dirname(options.outputPath), { recursive: true });
    fs.writeFileSync(options.outputPath, json, 'utf8');
    console.info(`[stage2.managed-checkpoint] wrote ${options.outputPath}`);
  } else {
    process.stdout.write(json);
  }
}

try {
  main();
} catch (error) {
  console.error(error);
  process.exitCode = 1;
}
