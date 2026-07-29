/** Stage 2 packed-weight, legacy-JSON, and archive-v1 codec measurements. */

import { createHash } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { performance } from 'node:perf_hooks';
import { spawnSync } from 'node:child_process';
import zlib from 'node:zlib';
import { resetCFGToDefaults } from '../../src/config.ts';
import { World } from '../../src/world.ts';
import {
  describePopulation,
  evolvePopulationFixture,
  installStage2Scenario,
  packPopulationWeights,
  shuffleFloat32Bytes,
  STAGE2_WORLD_SEED,
  unshuffleFloat32Bytes,
  type Stage2ScenarioName
} from './fixtures.ts';

/** Maximum legacy decompressed population accepted by current production code. */
const CURRENT_LEGACY_DECOMPRESSED_LIMIT = 512 * 1024 * 1024;
/** Zstandard compression level selected by the approved provisional design. */
const ZSTD_LEVEL = 3;

/** Codec runner options. */
interface CodecOptions {
  /** Standard population/brain workload. */
  scenario: Extract<Stage2ScenarioName, 'P0' | 'P1' | 'P2' | 'P3'>;
  /** Fresh or evolved-like fixture. */
  fixture: 'fresh' | 'evolved';
  /** Number of deterministic operator generations for evolved fixtures. */
  evolutionGenerations: number;
  /** Optional artifact destination. */
  outputPath: string | null;
}

/** One encode/decode measurement. */
interface CodecMeasurement {
  /** Stored encoding label. */
  encoding: string;
  /** Input packed bytes. */
  rawBytes: number;
  /** Encoded bytes. */
  encodedBytes: number;
  /** Encoded/raw ratio. */
  encodedToRawRatio: number;
  /** Percent smaller than raw; negative means expansion. */
  reductionPercent: number;
  /** Synchronous encoding time. */
  encodeMs: number;
  /** Synchronous decoding time. */
  decodeMs: number;
  /** Whether decoded bytes match bit-for-bit. */
  bitExact: boolean;
  /** SHA-256 of decoded packed bytes. */
  decodedSha256: string;
}

/**
 * Parse a bounded integer.
 * @param value - CLI value.
 * @param option - Option name.
 * @returns Parsed integer.
 */
function parseInteger(value: string | undefined, option: string): number {
  const parsed = Number.parseInt(value ?? '', 10);
  if (!Number.isSafeInteger(parsed) || parsed < 1 || parsed > 10_000) {
    throw new Error(`${option} must be an integer from 1 to 10000`);
  }
  return parsed;
}

/**
 * Parse command-line options.
 * @param argv - Arguments after script path.
 * @returns Validated options.
 */
function parseOptions(argv: readonly string[]): CodecOptions {
  const result: CodecOptions = {
    scenario: 'P0',
    fixture: 'fresh',
    evolutionGenerations: 25,
    outputPath: null
  };
  for (let index = 0; index < argv.length; index++) {
    const option = argv[index];
    const value = argv[index + 1];
    switch (option) {
      case '--scenario':
        if (value !== 'P0' && value !== 'P1' && value !== 'P2' && value !== 'P3') {
          throw new Error('--scenario must be P0, P1, P2, or P3');
        }
        result.scenario = value;
        index++;
        break;
      case '--fixture':
        if (value !== 'fresh' && value !== 'evolved') {
          throw new Error('--fixture must be fresh or evolved');
        }
        result.fixture = value;
        index++;
        break;
      case '--evolution-generations':
        result.evolutionGenerations = parseInteger(value, option);
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
 * Read source identity without changing repository state.
 * @returns Commit and dirty-worktree flag.
 */
function sourceIdentity(): { commit: string; dirty: boolean } {
  const commit = spawnSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' });
  const status = spawnSync('git', ['status', '--porcelain'], { encoding: 'utf8' });
  return {
    commit: commit.status === 0 ? commit.stdout.trim() : 'unavailable',
    dirty: status.status !== 0 || status.stdout.trim().length > 0
  };
}

/**
 * Return exact UTF-8 bytes that current JSON number arrays would occupy,
 * without constructing a population-sized string.
 * @param population - Population whose weights would be serialized.
 * @returns Exact weight-array bytes and Float32 sample count.
 */
function countDecimalJsonWeights(
  population: readonly { weights: Float32Array }[]
): { bytes: number; samples: number; bytesPerFloat: number } {
  let bytes = 0;
  let samples = 0;
  for (const genome of population) {
    bytes += 2;
    for (let index = 0; index < genome.weights.length; index++) {
      if (index > 0) bytes += 1;
      const encoded = JSON.stringify(genome.weights[index]!);
      bytes += Buffer.byteLength(encoded, 'utf8');
      samples++;
    }
  }
  return {
    bytes,
    samples,
    bytesPerFloat: samples > 0 ? bytes / samples : 0
  };
}

/**
 * Compress with explicit level and optional frame checksum.
 * @param input - Source bytes.
 * @param checksum - Whether to add the Zstandard frame checksum.
 * @returns Encoded frame.
 */
function compressZstd(input: Buffer, checksum: boolean): Buffer {
  return zlib.zstdCompressSync(input, {
    params: {
      [zlib.constants.ZSTD_c_compressionLevel]: ZSTD_LEVEL,
      [zlib.constants.ZSTD_c_checksumFlag]: checksum ? 1 : 0,
      [zlib.constants.ZSTD_c_contentSizeFlag]: 1
    }
  });
}

/**
 * Measure one raw or shuffled Zstandard round trip.
 * @param raw - Canonical packed Float32 bytes.
 * @param shuffled - Whether to byte-shuffle before compression.
 * @param checksum - Whether the frame contains a codec checksum.
 * @returns Size, timing, and bit-exact result.
 */
function measureZstd(raw: Buffer, shuffled: boolean, checksum: boolean): CodecMeasurement {
  const source = shuffled ? shuffleFloat32Bytes(raw) : raw;
  const encodeStarted = performance.now();
  const encoded = compressZstd(source, checksum);
  const encodeMs = performance.now() - encodeStarted;
  const decodeStarted = performance.now();
  const decodedPayload = zlib.zstdDecompressSync(encoded);
  const decoded = shuffled ? unshuffleFloat32Bytes(decodedPayload) : decodedPayload;
  const decodeMs = performance.now() - decodeStarted;
  const bitExact = decoded.equals(raw);
  const rawBytes = raw.length;
  const ratio = rawBytes > 0 ? encoded.length / rawBytes : 0;
  return {
    encoding: `${shuffled ? 'shuffle-' : ''}zstd-level-${ZSTD_LEVEL}${checksum ? '-checksum' : ''}`,
    rawBytes,
    encodedBytes: encoded.length,
    encodedToRawRatio: Number(ratio.toFixed(8)),
    reductionPercent: Number(((1 - ratio) * 100).toFixed(6)),
    encodeMs: Number(encodeMs.toFixed(6)),
    decodeMs: Number(decodeMs.toFixed(6)),
    bitExact,
    decodedSha256: createHash('sha256').update(decoded).digest('hex')
  };
}

/**
 * Compare per-genome storage without retaining all encoded buffers.
 * Plain Zstandard remains comparison evidence; the approved archive-v1
 * numeric-entry choice is raw packed versus byte-shuffled Zstandard.
 * @param population - Dense population.
 * @returns Aggregate comparison and approved raw-or-shuffled sizes and timing.
 */
function measurePerGenome(
  population: readonly { weights: Float32Array }[]
): Record<string, number> {
  let rawBytes = 0;
  let plainZstdBytes = 0;
  let shuffledZstdBytes = 0;
  let minimumAcrossMeasuredBytes = 0;
  let approvedRawOrShuffledBytes = 0;
  let minimumRawSelections = 0;
  let minimumPlainSelections = 0;
  let minimumShuffledSelections = 0;
  let approvedRawSelections = 0;
  let approvedShuffledSelections = 0;
  const started = performance.now();
  for (const genome of population) {
    const raw = Buffer.from(
      genome.weights.buffer,
      genome.weights.byteOffset,
      genome.weights.byteLength
    );
    const plain = compressZstd(raw, false);
    const shuffled = compressZstd(shuffleFloat32Bytes(raw), false);
    rawBytes += raw.length;
    plainZstdBytes += plain.length;
    shuffledZstdBytes += shuffled.length;
    const smallest = Math.min(raw.length, plain.length, shuffled.length);
    minimumAcrossMeasuredBytes += smallest;
    if (smallest === raw.length) minimumRawSelections++;
    else if (smallest === shuffled.length) minimumShuffledSelections++;
    else minimumPlainSelections++;
    const approvedSmallest = Math.min(raw.length, shuffled.length);
    approvedRawOrShuffledBytes += approvedSmallest;
    if (approvedSmallest === raw.length) approvedRawSelections++;
    else approvedShuffledSelections++;
  }
  return {
    rawBytes,
    plainZstdBytes,
    shuffledZstdBytes,
    minimumAcrossMeasuredBytes,
    minimumAcrossMeasuredToRawRatio:
      rawBytes > 0 ? Number((minimumAcrossMeasuredBytes / rawBytes).toFixed(8)) : 0,
    minimumRawSelections,
    minimumPlainSelections,
    minimumShuffledSelections,
    approvedRawOrShuffledBytes,
    approvedRawOrShuffledToRawRatio:
      rawBytes > 0 ? Number((approvedRawOrShuffledBytes / rawBytes).toFixed(8)) : 0,
    approvedRawSelections,
    approvedShuffledSelections,
    encodeMs: Number((performance.now() - started).toFixed(6))
  };
}

/**
 * Build and gzip the exact old length-prefixed JSON representation when it
 * falls within the current decoder's declared decompressed limit.
 * @param population - Population to encode.
 * @returns Measurement or explicit bounded skip.
 */
function measureLegacyGzip(
  population: readonly {
    archKey: string;
    brainType: string;
    fitness: number;
    weights: Float32Array;
  }[]
): Record<string, unknown> {
  const records: Buffer[] = [];
  let totalBytes = 0;
  for (const genome of population) {
    const json = Buffer.from(JSON.stringify({
      archKey: genome.archKey,
      brainType: genome.brainType,
      fitness: genome.fitness,
      weights: Array.from(genome.weights)
    }), 'utf8');
    totalBytes += 4 + json.length;
    if (totalBytes > CURRENT_LEGACY_DECOMPRESSED_LIMIT) {
      return {
        measured: false,
        reason: `exact legacy bytes exceed current ${CURRENT_LEGACY_DECOMPRESSED_LIMIT}-byte decompressed limit`,
        bytesBeforeStop: totalBytes
      };
    }
    const prefix = Buffer.allocUnsafe(4);
    prefix.writeUInt32LE(json.length, 0);
    records.push(prefix, json);
  }
  const legacy = Buffer.concat(records, totalBytes);
  const encodeStarted = performance.now();
  const compressed = zlib.gzipSync(legacy);
  const encodeMs = performance.now() - encodeStarted;
  const decodeStarted = performance.now();
  const decoded = zlib.gunzipSync(compressed);
  const decodeMs = performance.now() - decodeStarted;
  return {
    measured: true,
    decompressedBytes: legacy.length,
    compressedBytes: compressed.length,
    compressedToDecompressedRatio: Number((compressed.length / legacy.length).toFixed(8)),
    encodeMs: Number(encodeMs.toFixed(6)),
    decodeMs: Number(decodeMs.toFixed(6)),
    bitExact: decoded.equals(legacy),
    sha256: createHash('sha256').update(legacy).digest('hex')
  };
}

/**
 * Run the requested codec fixture.
 * @param options - Validated command options.
 * @returns Machine-readable evidence.
 */
function runCodecBaseline(options: CodecOptions): Record<string, unknown> {
  const scenario = installStage2Scenario(options.scenario);
  const world = new World(scenario.settings, {
    seed: STAGE2_WORLD_SEED,
    runId: `stage2-codec-${options.scenario.toLowerCase()}`
  });
  const population = options.fixture === 'fresh'
    ? world.population.map(genome => genome.clone())
    : evolvePopulationFixture(world.population, world.arch, options.evolutionGenerations);
  const description = describePopulation(population, world.arch);
  const raw = packPopulationWeights(population);
  const decimalJson = countDecimalJsonWeights(population);
  const plain = measureZstd(raw, false, false);
  const shuffled = measureZstd(raw, true, false);
  const plainChecksum = measureZstd(raw, false, true);
  const shuffledChecksum = measureZstd(raw, true, true);
  const measuredCandidates = [
    { encoding: 'raw-packed', bytes: raw.length },
    { encoding: plain.encoding, bytes: plain.encodedBytes },
    { encoding: shuffled.encoding, bytes: shuffled.encodedBytes }
  ].sort((left, right) => left.bytes - right.bytes);
  const archiveV1Candidates = [
    { encoding: 'raw-f32le-v1', bytes: raw.length },
    { encoding: 'f32le-shuffle4-zstd-v1', bytes: shuffled.encodedBytes }
  ].sort((left, right) => left.bytes - right.bytes);
  return {
    schema: 'slither-stage2-codec-baseline',
    version: 2,
    evidenceClass: 'new measured result',
    caveat: 'Offline population-weight codec fixture; managed checkpoint container and full server memory are measured separately.',
    source: sourceIdentity(),
    environment: {
      capturedAt: new Date().toISOString(),
      platform: process.platform,
      architecture: process.arch,
      node: process.version,
      zlib: process.versions.zlib,
      zstd: process.versions.zstd ?? null
    },
    fixture: {
      scenario,
      kind: options.fixture,
      evolutionGenerations: options.fixture === 'evolved' ? options.evolutionGenerations : 0,
      seed: STAGE2_WORLD_SEED,
      ...description
    },
    decimalJsonWeights: {
      ...decimalJson,
      bytesPerRawByte: Number((decimalJson.bytes / raw.length).toFixed(8)),
      reductionUsingRawPackedPercent: Number(((1 - raw.length / decimalJson.bytes) * 100).toFixed(6))
    },
    wholePopulation: {
      rawPacked: {
        bytes: raw.length,
        sha256: createHash('sha256').update(raw).digest('hex')
      },
      plainZstd: plain,
      shuffledZstd: shuffled,
      plainZstdWithFrameChecksum: plainChecksum,
      shuffledZstdWithFrameChecksum: shuffledChecksum,
      smallestMeasuredComparison: measuredCandidates[0],
      selectedArchiveV1: archiveV1Candidates[0]
    },
    perGenome: measurePerGenome(population),
    currentLegacyGzip: measureLegacyGzip(population)
  };
}

/** Execute the CLI. */
function main(): void {
  const options = parseOptions(process.argv.slice(2));
  try {
    const result = runCodecBaseline(options);
    const json = `${JSON.stringify(result, null, 2)}\n`;
    if (options.outputPath) {
      fs.mkdirSync(path.dirname(options.outputPath), { recursive: true });
      fs.writeFileSync(options.outputPath, json, 'utf8');
      console.info(`[stage2.codec] wrote ${options.outputPath}`);
    } else {
      process.stdout.write(json);
    }
  } finally {
    resetCFGToDefaults();
  }
}

try {
  main();
} catch (error) {
  console.error(error);
  process.exitCode = 1;
}
