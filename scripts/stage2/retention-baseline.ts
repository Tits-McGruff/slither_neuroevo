/**
 * Stage 2 accelerated checkpoint/history/Hall-of-Fame retention fixture.
 *
 * This is a size-matched managed-file and SQLite-metadata measurement, not the
 * production checkpoint-v3 writer and not a valid save archive.
 */

import { createHash } from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { performance } from 'node:perf_hooks';
import { spawnSync } from 'node:child_process';
import { pathToFileURL } from 'node:url';
import Database from 'better-sqlite3';
import {
  OWNER_RETENTION_DEFAULTS,
  selectRetainedCheckpoints,
  type RetainedCheckpoint,
  type RetentionCandidate,
  type RetentionSettings
} from './retentionPolicy.ts';

/** Scenarios with retained evolved-like codec evidence. */
type ProjectionScenario = 'P0' | 'P1' | 'P2' | 'P3';

/** Default number of 60-second generations in eight hours. */
const DEFAULT_GENERATIONS = 480;
/** Exact approved compact history record size. */
export const HISTORY_RECORD_BYTES = 56;
/** Deterministic observation points for the isolated history-row measurement. */
const HISTORY_GROWTH_SAMPLES = [0, 1, 8, 64, 480] as const;
/** Largest isolated history fixture accepted by the command-line runner. */
const MAX_HISTORY_GENERATIONS = 1_000_000;
/** Owner-selected initial Hall-of-Fame unique-genome count. */
const DEFAULT_HOF_UNIQUE_LIMIT = 50;
/** Safety ceiling for modeled checkpoint plus Hall-of-Fame payload bytes. */
const DEFAULT_MAX_MODELED_PAYLOAD_BYTES = 2 * 1024 * 1024 * 1024;
/** Stable current-run identity used by the fixture. */
const CURRENT_RUN_ID = 'stage2-retention-current';
/** Retained codec evidence directory. */
const CODEC_EVIDENCE_DIRECTORY = path.resolve(
  'docs',
  'todo',
  'evidence',
  'stage2',
  'windows-5800x'
);

/** Command-line options. */
interface RetentionBaselineOptions {
  /** Generation boundaries represented by the accelerated fixture. */
  generations: number;
  /** Scenario whose bytes are physically materialized. */
  materializeScenario: ProjectionScenario;
  /** Hall-of-Fame unique-genome limit. */
  hofUniqueLimit: number;
  /** Maximum modeled checkpoint plus Hall-of-Fame payload bytes. */
  maxModeledPayloadBytes: number;
  /** Retention policy, including the automatic cap. */
  retention: RetentionSettings;
  /** Optional JSON output destination. */
  outputPath: string | null;
  /** Run only the isolated compact-history SQLite measurement. */
  historyOnly: boolean;
  /** Number of records written by the isolated compact-history measurement. */
  historyGenerations: number;
}

/** One SQLite dbstat allocation summary, when the virtual table is available. */
interface DbstatObject {
  /** SQLite object name. */
  name: string;
  /** Number of pages owned by the object. */
  pages: number;
  /** Bytes allocated to the object pages. */
  pageBytes: number;
  /** Bytes used for SQLite payloads. */
  payloadBytes: number;
  /** Unused bytes within the object's pages. */
  unusedBytes: number;
}

/** dbstat details for the compact history table and primary-key index. */
interface HistoryDbstat {
  /** Whether the linked SQLite build exposes dbstat. */
  available: boolean;
  /** Failure reason when dbstat is not compiled into SQLite. */
  reason?: string;
  /** Allocation summary for generation_history. */
  table: DbstatObject | null;
  /** Allocation summary for generation_history's PRIMARY KEY index. */
  primaryKeyIndex: DbstatObject | null;
}

/** File and page state at one isolated compact-history measurement point. */
interface HistoryStorageState {
  /** Main SQLite database bytes. */
  databaseBytes: number;
  /** Write-ahead log bytes. */
  walBytes: number;
  /** Shared-memory sidecar bytes. */
  shmBytes: number;
  /** Main database page count. */
  pageCount: number;
  /** Reusable main-database page count. */
  freelistCount: number;
  /** Table/index allocation, if supported by this SQLite build. */
  dbstat: HistoryDbstat;
}

/** One growth observation from the compact-history-only SQLite fixture. */
interface HistoryGrowthSample {
  /** Number of committed compact history records at the observation point. */
  records: number;
  /** Exact fixed-width logical history bytes. */
  logicalBytes: number;
  /** Physical SQLite state. */
  storage: HistoryStorageState;
}

/** Isolated compact-history measurement options exposed for focused tests. */
export interface HistoryStorageMeasurementOptions {
  /** Final committed record count. */
  generations: number;
}

/** Exact shared DDL for the target compact history row shape. */
const HISTORY_TABLE_SQL = `
CREATE TABLE generation_history (
  run_id TEXT NOT NULL,
  generation INTEGER NOT NULL,
  record BLOB NOT NULL,
  PRIMARY KEY (run_id, generation)
);`;

/** Minimal retained codec artifact fields consumed by this fixture. */
interface CodecArtifact {
  /** Evidence schema. */
  schema: string;
  /** Evidence schema version. */
  version: number;
  /** Source identity of the codec measurement. */
  source: { commit: string; dirty: boolean };
  /** Population description. */
  fixture: {
    scenario: { name: ProjectionScenario; description: string };
    populationCount: number;
    weightsPerGenome: number;
    rawWeightBytes: number;
  };
  /** Whole-population codec measurements. */
  wholePopulation: {
    selectedArchiveV1: { encoding: string; bytes: number };
  };
  /** Per-genome comparison used only for Hall-of-Fame size estimates. */
  perGenome: {
    approvedRawOrShuffledBytes: number;
  };
}

/** Codec artifact plus its retained file identity. */
interface LoadedCodecArtifact {
  /** Parsed evidence. */
  data: CodecArtifact;
  /** Repository-relative source path. */
  relativePath: string;
  /** Exact retained bytes. */
  fileBytes: number;
  /** Exact retained SHA-256. */
  sha256: string;
}

/** Simple numeric distribution. */
interface Distribution {
  /** Number of samples. */
  count: number;
  /** Minimum value. */
  min: number;
  /** Median value. */
  p50: number;
  /** 95th percentile. */
  p95: number;
  /** 99th percentile. */
  p99: number;
  /** Maximum value. */
  max: number;
  /** Arithmetic mean. */
  mean: number;
}

/** Flat directory totals. */
interface DirectoryTotals {
  /** Regular-file count. */
  files: number;
  /** Sum of regular-file lengths. */
  bytes: number;
}

/**
 * Parse one bounded positive safe integer.
 * @param value - Raw CLI value.
 * @param option - Option name.
 * @param maximum - Inclusive upper bound.
 * @returns Parsed integer.
 */
function parseInteger(value: string | undefined, option: string, maximum: number): number {
  const parsed = Number.parseInt(value ?? '', 10);
  if (!Number.isSafeInteger(parsed) || parsed < 1 || parsed > maximum) {
    throw new RangeError(`${option} must be an integer from 1 to ${maximum}`);
  }
  return parsed;
}

/**
 * Parse command-line options.
 * @param argv - Arguments after the script path.
 * @returns Validated options.
 */
function parseOptions(argv: readonly string[]): RetentionBaselineOptions {
  const options: RetentionBaselineOptions = {
    generations: DEFAULT_GENERATIONS,
    materializeScenario: 'P0',
    hofUniqueLimit: DEFAULT_HOF_UNIQUE_LIMIT,
    maxModeledPayloadBytes: DEFAULT_MAX_MODELED_PAYLOAD_BYTES,
    retention: { ...OWNER_RETENTION_DEFAULTS },
    outputPath: null,
    historyOnly: false,
    historyGenerations: DEFAULT_GENERATIONS
  };
  for (let index = 0; index < argv.length; index++) {
    const option = argv[index];
    const value = argv[index + 1];
    switch (option) {
      case '--generations':
        options.generations = parseInteger(value, option, 1_000_000);
        index++;
        break;
      case '--scenario':
        if (value !== 'P0' && value !== 'P1' && value !== 'P2' && value !== 'P3') {
          throw new Error('--scenario must be P0, P1, P2, or P3');
        }
        options.materializeScenario = value;
        index++;
        break;
      case '--hof-unique':
        options.hofUniqueLimit = parseInteger(value, option, 10_000);
        index++;
        break;
      case '--automatic-cap-bytes':
        options.retention.automaticByteCap = parseInteger(
          value,
          option,
          Number.MAX_SAFE_INTEGER
        );
        index++;
        break;
      case '--max-modeled-payload-bytes':
        options.maxModeledPayloadBytes = parseInteger(value, option, Number.MAX_SAFE_INTEGER);
        index++;
        break;
      case '--output':
        if (!value) throw new Error('--output requires a path');
        options.outputPath = path.resolve(value);
        index++;
        break;
      case '--history-only':
        options.historyOnly = true;
        break;
      case '--history-generations':
        options.historyGenerations = parseInteger(value, option, MAX_HISTORY_GENERATIONS);
        index++;
        break;
      default:
        throw new Error(`Unknown option ${option ?? '<missing>'}`);
    }
  }
  return options;
}

/**
 * Return current Git identity without changing repository state.
 * @returns Commit and dirty flag.
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
 * Read one file size, returning zero for an absent WAL/SHM path.
 * @param filePath - Candidate file.
 * @returns File bytes or zero.
 */
function fileSize(filePath: string): number {
  try {
    return fs.statSync(filePath).size;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') return 0;
    throw error;
  }
}

/**
 * Summarize a numeric sample.
 * @param values - Finite numbers.
 * @returns Rounded distribution.
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
    const lowerValue = sorted[lower]!;
    return lowerValue + (sorted[upper]! - lowerValue) * (position - lower);
  };
  const round = (value: number): number => Number(value.toFixed(6));
  return {
    count: sorted.length,
    min: round(sorted[0]!),
    p50: round(percentile(0.5)),
    p95: round(percentile(0.95)),
    p99: round(percentile(0.99)),
    max: round(sorted.at(-1)!),
    mean: round(sorted.reduce((sum, value) => sum + value, 0) / sorted.length)
  };
}

/**
 * Sum regular files directly inside one fixture directory.
 * @param directory - Existing flat directory.
 * @returns File count and bytes.
 */
function directoryTotals(directory: string): DirectoryTotals {
  let files = 0;
  let bytes = 0;
  for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
    if (!entry.isFile()) continue;
    files++;
    bytes += fs.statSync(path.join(directory, entry.name)).size;
  }
  return { files, bytes };
}

/**
 * Load and validate one evolved-like codec artifact.
 * @param scenario - Scenario name.
 * @returns Parsed data and retained artifact identity.
 */
function loadCodecArtifact(scenario: ProjectionScenario): LoadedCodecArtifact {
  const relativePath = path.join(
    'docs',
    'todo',
    'evidence',
    'stage2',
    'windows-5800x',
    `codec-${scenario.toLowerCase()}-evolved25.json`
  );
  const absolutePath = path.resolve(relativePath);
  if (path.dirname(absolutePath) !== CODEC_EVIDENCE_DIRECTORY) {
    throw new Error(`unexpected codec evidence path ${absolutePath}`);
  }
  const bytes = fs.readFileSync(absolutePath);
  const data = JSON.parse(bytes.toString('utf8')) as CodecArtifact;
  if (data.schema !== 'slither-stage2-codec-baseline' || data.version !== 2) {
    throw new Error(`${relativePath} is not codec evidence version 2`);
  }
  if (data.fixture.scenario.name !== scenario) {
    throw new Error(`${relativePath} contains scenario ${data.fixture.scenario.name}`);
  }
  const selected = data.wholePopulation.selectedArchiveV1;
  if (
    selected.encoding !== 'raw-f32le-v1' &&
    selected.encoding !== 'f32le-shuffle4-zstd-v1'
  ) {
    throw new Error(`${relativePath} selected unsupported encoding ${selected.encoding}`);
  }
  if (!Number.isSafeInteger(selected.bytes) || selected.bytes < 1) {
    throw new Error(`${relativePath} contains invalid selected bytes`);
  }
  return {
    data,
    relativePath: relativePath.replaceAll('\\', '/'),
    fileBytes: bytes.length,
    sha256: createHash('sha256').update(bytes).digest('hex')
  };
}

/**
 * Construct count/byte-policy candidates for one scenario.
 * @param generations - Current-run generation count.
 * @param bytes - Stored bytes represented by each checkpoint.
 * @returns Current-run automatic checkpoints plus two prior anchors.
 */
function projectionCandidates(generations: number, bytes: number): RetentionCandidate[] {
  const candidates: RetentionCandidate[] = [
    {
      key: 'prior-anchor-1',
      runId: 'prior-run-1',
      generation: 91,
      bytes,
      createdOrdinal: 1,
      pinned: false,
      priorRunAnchor: true
    },
    {
      key: 'prior-anchor-2',
      runId: 'prior-run-2',
      generation: 47,
      bytes,
      createdOrdinal: 2,
      pinned: false,
      priorRunAnchor: true
    }
  ];
  for (let generation = 1; generation <= generations; generation++) {
    candidates.push({
      key: `current-${generation}`,
      runId: CURRENT_RUN_ID,
      generation,
      bytes,
      createdOrdinal: 1_000 + generation,
      pinned: false,
      priorRunAnchor: false
    });
  }
  return candidates;
}

/**
 * Count retained checkpoints by selected class.
 * @param kept - Retained checkpoints.
 * @returns Class counts.
 */
function countRetentionClasses(
  kept: readonly RetainedCheckpoint[]
): Record<string, number> {
  const counts: Record<string, number> = {};
  for (const item of kept) {
    counts[item.retentionClass] = (counts[item.retentionClass] ?? 0) + 1;
  }
  return counts;
}

/**
 * Derive one overnight retention projection from retained codec evidence.
 * @param artifact - Scenario codec artifact.
 * @param generations - Current-run generations.
 * @param hofUniqueLimit - Retained unique Hall-of-Fame genomes.
 * @param retention - Retention settings.
 * @returns Size projection labelled as derived arithmetic.
 */
function projectScenario(
  artifact: LoadedCodecArtifact,
  generations: number,
  hofUniqueLimit: number,
  retention: RetentionSettings
): Record<string, unknown> {
  const fixture = artifact.data.fixture;
  const checkpointBytes = artifact.data.wholePopulation.selectedArchiveV1.bytes;
  const decision = selectRetainedCheckpoints(
    projectionCandidates(generations, checkpointBytes),
    CURRENT_RUN_ID,
    retention
  );
  const averageHofGenomeBytes =
    artifact.data.perGenome.approvedRawOrShuffledBytes / fixture.populationCount;
  return {
    evidenceClass: 'derived arithmetic from retained measured codec bytes',
    scenario: fixture.scenario,
    codecArtifact: {
      path: artifact.relativePath,
      sha256: artifact.sha256,
      bytes: artifact.fileBytes,
      source: artifact.data.source
    },
    checkpointWeightPayload: {
      encoding: artifact.data.wholePopulation.selectedArchiveV1.encoding,
      rawBytes: fixture.rawWeightBytes,
      storedBytes: checkpointBytes
    },
    currentRunWithoutRetention: {
      checkpoints: generations,
      storedWeightPayloadBytes: generations * checkpointBytes
    },
    retainedAutomatic: {
      checkpoints: decision.kept.filter(item => item.retentionClass !== 'pinned').length,
      storedWeightPayloadBytes: decision.automaticBytes,
      protectedMinimumBytes: decision.protectedAutomaticBytes,
      classCounts: countRetentionClasses(decision.kept)
    },
    compactHistory: {
      records: generations,
      logicalBytes: generations * HISTORY_RECORD_BYTES
    },
    hallOfFame: {
      uniqueGenomes: hofUniqueLimit,
      rawWeightBytes: hofUniqueLimit * fixture.weightsPerGenome * Float32Array.BYTES_PER_ELEMENT,
      derivedAverageApprovedStoredBytes: Number(averageHofGenomeBytes.toFixed(3)),
      derivedStoredBytes: Math.ceil(averageHofGenomeBytes * hofUniqueLimit)
    }
  };
}

/**
 * Build deterministic high-entropy bytes of an exact size.
 * @param length - Required length.
 * @param seed - Xorshift seed.
 * @returns Reproducible bytes.
 */
function deterministicPayload(length: number, seed: number): Buffer {
  const output = Buffer.allocUnsafe(length);
  let state = seed >>> 0;
  for (let index = 0; index < output.length; index++) {
    state ^= state << 13;
    state ^= state >>> 17;
    state ^= state << 5;
    output[index] = state & 0xff;
  }
  return output;
}

/**
 * Encode one deterministic approved 56-byte history record.
 * @param generation - Generation number.
 * @returns Fixed-width little-endian record.
 */
export function encodeHistoryRecord(generation: number): Buffer {
  const record = Buffer.alloc(HISTORY_RECORD_BYTES);
  record.writeBigUInt64LE(BigInt(generation), 0);
  record.writeDoubleLE(generation * 1.25, 8);
  record.writeDoubleLE(generation * 0.75, 16);
  record.writeDoubleLE(generation * 0.25, 24);
  record.writeUInt32LE(1 + (generation % 17), 32);
  record.writeUInt32LE(1 + (generation % 55), 36);
  record.writeDoubleLE(Math.sin(generation * 0.03125), 40);
  record.writeDoubleLE(Math.abs(Math.cos(generation * 0.015625)), 48);
  return record;
}

/**
 * Create one same-size unique file from a retained-byte template.
 * @param templatePath - Existing size template.
 * @param destination - New fixture file.
 * @param ordinal - Value written into the first eight bytes.
 */
function copyUniqueSizedFile(templatePath: string, destination: string, ordinal: number): void {
  fs.copyFileSync(templatePath, destination, fs.constants.COPYFILE_EXCL);
  const descriptor = fs.openSync(destination, 'r+');
  try {
    const marker = Buffer.allocUnsafe(8);
    marker.writeBigUInt64LE(BigInt(ordinal));
    fs.writeSync(descriptor, marker, 0, marker.length, 0);
  } finally {
    fs.closeSync(descriptor);
  }
}

/**
 * Return an approved per-genome size whose sequence sums to the retained
 * artifact's aggregate every population-length block.
 * @param aggregateBytes - Aggregate per-genome selected bytes.
 * @param populationCount - Genomes represented by the aggregate.
 * @param ordinal - One-based synthetic Hall-of-Fame genome ordinal.
 * @returns Size-matched genome bytes.
 */
function perGenomeFixtureBytes(
  aggregateBytes: number,
  populationCount: number,
  ordinal: number
): number {
  const base = Math.floor(aggregateBytes / populationCount);
  const remainder = aggregateBytes % populationCount;
  return base + ((ordinal - 1) % populationCount < remainder ? 1 : 0);
}

/**
 * Create fixture-only metadata schema.
 * @param database - Disposable database.
 */
function createFixtureSchema(database: Database.Database): void {
  database.exec(`
    PRAGMA foreign_keys = ON;
    PRAGMA journal_mode = WAL;
    PRAGMA synchronous = FULL;
    PRAGMA wal_autocheckpoint = 1000;
    CREATE TABLE definitions (
      kind TEXT NOT NULL,
      content_key TEXT NOT NULL,
      payload BLOB NOT NULL,
      PRIMARY KEY (kind, content_key)
    );
    CREATE TABLE checkpoints (
      checkpoint_key TEXT PRIMARY KEY,
      run_id TEXT NOT NULL,
      generation INTEGER NOT NULL,
      retention_class TEXT NOT NULL,
      file_name TEXT NOT NULL UNIQUE,
      file_bytes INTEGER NOT NULL,
      created_ordinal INTEGER NOT NULL
    );
    CREATE TABLE current_checkpoint (
      singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
      checkpoint_key TEXT NOT NULL REFERENCES checkpoints(checkpoint_key)
    );
    ${HISTORY_TABLE_SQL}
    CREATE TABLE hof_entries (
      run_id TEXT NOT NULL,
      generation INTEGER NOT NULL,
      score REAL NOT NULL,
      genome_key TEXT,
      weights_retained INTEGER NOT NULL,
      metadata BLOB NOT NULL,
      PRIMARY KEY (run_id, generation)
    );
    CREATE TABLE hof_genomes (
      genome_key TEXT PRIMARY KEY,
      file_name TEXT NOT NULL UNIQUE,
      file_bytes INTEGER NOT NULL
    );
  `);
  const insertDefinition = database.prepare(
    'INSERT INTO definitions(kind, content_key, payload) VALUES (?, ?, ?)'
  );
  insertDefinition.run('graph', 'stage2-graph-v1', Buffer.from('one stable graph definition'));
  insertDefinition.run('config', 'stage2-config-v1', Buffer.from('one stable config definition'));
}

/**
 * Capture dbstat allocation for the exact compact-history table and its
 * PRIMARY KEY index without treating dbstat support as a fixture prerequisite.
 * @param database - Open disposable SQLite database.
 * @returns Available dbstat detail or an explicit unsupported result.
 */
function captureHistoryDbstat(database: Database.Database): HistoryDbstat {
  try {
    const indexes = database.prepare(`PRAGMA index_list('generation_history')`).all() as Array<{
      name: string;
      origin: string;
    }>;
    const primaryKeyIndex = indexes.find(index => index.origin === 'pk')?.name;
    if (!primaryKeyIndex) throw new Error('generation_history PRIMARY KEY index is absent');
    const objects = database.prepare(`
      SELECT name,
             COUNT(*) AS pages,
             SUM(pgsize) AS pageBytes,
             SUM(payload) AS payloadBytes,
             SUM(unused) AS unusedBytes
        FROM dbstat
       WHERE name IN (?, ?)
       GROUP BY name
       ORDER BY name
    `).all('generation_history', primaryKeyIndex) as DbstatObject[];
    return {
      available: true,
      table: objects.find(object => object.name === 'generation_history') ?? null,
      primaryKeyIndex: objects.find(object => object.name === primaryKeyIndex) ?? null
    };
  } catch (error) {
    return {
      available: false,
      reason: error instanceof Error ? error.message : String(error),
      table: null,
      primaryKeyIndex: null
    };
  }
}

/**
 * Capture physical SQLite state for the isolated compact-history fixture.
 * @param database - Open disposable SQLite database.
 * @param databasePath - Main database path.
 * @returns File, page and optional dbstat accounting.
 */
function captureHistoryStorageState(
  database: Database.Database,
  databasePath: string
): HistoryStorageState {
  return {
    databaseBytes: fileSize(databasePath),
    walBytes: fileSize(`${databasePath}-wal`),
    shmBytes: fileSize(`${databasePath}-shm`),
    pageCount: database.pragma('page_count', { simple: true }) as number,
    freelistCount: database.pragma('freelist_count', { simple: true }) as number,
    dbstat: captureHistoryDbstat(database)
  };
}

/**
 * Run the bounded, history-only SQLite measurement without creating a
 * checkpoint archive, managed file, segment store, or production database.
 * @param options - Final record count for the disposable fixture.
 * @returns Machine-readable row-growth and WAL evidence.
 */
export function runHistoryStorageMeasurement(
  options: HistoryStorageMeasurementOptions
): Record<string, unknown> {
  if (
    !Number.isSafeInteger(options.generations) ||
    options.generations < 1 ||
    options.generations > MAX_HISTORY_GENERATIONS
  ) {
    throw new RangeError(
      `history generations must be an integer from 1 to ${MAX_HISTORY_GENERATIONS}`
    );
  }
  const temporaryRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'slither-stage2-history-'));
  const resolvedTemporaryRoot = path.resolve(temporaryRoot);
  const resolvedSystemTemp = path.resolve(os.tmpdir());
  let database: Database.Database | null = null;
  try {
    if (
      path.dirname(resolvedTemporaryRoot) !== resolvedSystemTemp ||
      !path.basename(resolvedTemporaryRoot).startsWith('slither-stage2-history-')
    ) {
      throw new Error(`unexpected temporary history fixture path ${resolvedTemporaryRoot}`);
    }
    const databasePath = path.join(resolvedTemporaryRoot, 'history.db');
    const openedDatabase = new Database(databasePath);
    database = openedDatabase;
    openedDatabase.pragma('journal_mode = WAL');
    openedDatabase.pragma('synchronous = FULL');
    openedDatabase.pragma('wal_autocheckpoint = 1000');
    openedDatabase.exec(HISTORY_TABLE_SQL);
    openedDatabase.pragma('wal_checkpoint(TRUNCATE)');

    const selectedSamples = new Set<number>([
      ...HISTORY_GROWTH_SAMPLES.filter(sample => sample <= options.generations),
      options.generations
    ]);
    const growthSamples: HistoryGrowthSample[] = [];
    const captureSample = (records: number): void => {
      growthSamples.push({
        records,
        logicalBytes: records * HISTORY_RECORD_BYTES,
        storage: captureHistoryStorageState(openedDatabase, databasePath)
      });
    };
    const appendTransactionsMs: number[] = [];
    const insertHistory = openedDatabase.prepare(`
      INSERT INTO generation_history(run_id, generation, record)
      VALUES (?, ?, ?)
    `);
    const appendHistory = openedDatabase.transaction((generation: number) => {
      insertHistory.run(CURRENT_RUN_ID, generation, encodeHistoryRecord(generation));
    });

    captureSample(0);
    let peakWalBytes = growthSamples[0]!.storage.walBytes;
    for (let generation = 1; generation <= options.generations; generation++) {
      const started = performance.now();
      appendHistory(generation);
      appendTransactionsMs.push(performance.now() - started);
      peakWalBytes = Math.max(peakWalBytes, fileSize(`${databasePath}-wal`));
      if (selectedSamples.has(generation)) captureSample(generation);
    }
    const counts = openedDatabase.prepare(`
      SELECT COUNT(*) AS records, COALESCE(SUM(LENGTH(record)), 0) AS logical_bytes
        FROM generation_history
    `).get() as { records: number; logical_bytes: number };
    if (
      counts.records !== options.generations ||
      counts.logical_bytes !== options.generations * HISTORY_RECORD_BYTES
    ) {
      throw new Error('isolated compact history row/byte accounting mismatch');
    }
    const beforePassiveCheckpoint = captureHistoryStorageState(openedDatabase, databasePath);
    const checkpointStarted = performance.now();
    const passiveCheckpointResult = openedDatabase.pragma('wal_checkpoint(PASSIVE)');
    const passiveCheckpointMs = performance.now() - checkpointStarted;
    const afterPassiveCheckpoint = captureHistoryStorageState(openedDatabase, databasePath);

    return {
      fixtureKind: 'isolated compact-history SQLite row measurement; not a checkpoint, archive, segment, or production schema',
      fixture: {
        generations: options.generations,
        recordBytes: HISTORY_RECORD_BYTES,
        table: 'generation_history',
        primaryKey: ['run_id', 'generation'],
        encoding: 'eight-field little-endian binary record'
      },
      growthSamples,
      appendTransactionMs: distribution(appendTransactionsMs),
      wal: {
        peakObservedBytes: peakWalBytes,
        beforePassiveCheckpoint,
        passiveCheckpointMs: Number(passiveCheckpointMs.toFixed(6)),
        passiveCheckpointResult,
        afterPassiveCheckpoint
      },
      accountingAssertions: {
        exactRecordCount: true,
        exactFixedWidthLogicalBytes: true,
        requiredSamplesPresent: [0, 1, 8, 64, 480]
          .filter(sample => sample <= options.generations)
          .every(sample => growthSamples.some(observation => observation.records === sample))
      },
      limitations: {
        scope:
          'no checkpoint payload, managed file, Hall-of-Fame, deletion, segment, restore, or production persistence path is exercised',
        durability:
          'SQLite FULL transaction timing and passive checkpoint timing are not managed-checkpoint publication or directory-durability evidence',
        target:
          'this disposable local measurement is not target-VM evidence and does not select a production storage layout'
      }
    };
  } finally {
    try {
      database?.close();
    } finally {
      fs.rmSync(resolvedTemporaryRoot, { recursive: true, force: true });
    }
  }
}

/**
 * Read the linked SQLite library version without retaining a database handle.
 * @returns SQLite version record.
 */
function sqliteVersion(): unknown {
  const database = new Database(':memory:');
  try {
    return database.prepare('SELECT sqlite_version() AS version').get();
  } finally {
    database.close();
  }
}

/**
 * Run the physical P8 retention/history/Hall-of-Fame fixture.
 * @param artifact - Size source for the selected scenario.
 * @param options - Validated fixture settings.
 * @returns Measured result.
 */
function runMaterializedFixture(
  artifact: LoadedCodecArtifact,
  options: RetentionBaselineOptions
): Record<string, unknown> {
  const checkpointBytes = artifact.data.wholePopulation.selectedArchiveV1.bytes;
  const aggregateHofBytes = artifact.data.perGenome.approvedRawOrShuffledBytes;
  const populationCount = artifact.data.fixture.populationCount;
  let cumulativeHofBytes = 0;
  for (let generation = 1; generation <= options.generations; generation++) {
    cumulativeHofBytes += perGenomeFixtureBytes(
      aggregateHofBytes,
      populationCount,
      generation
    );
  }
  const cumulativeCheckpointBytes =
    checkpointBytes * (options.generations + options.retention.priorRunAnchorCount);
  const cumulativeMaterializedBytes = cumulativeCheckpointBytes + cumulativeHofBytes;
  if (
    !Number.isSafeInteger(cumulativeMaterializedBytes) ||
    cumulativeMaterializedBytes > options.maxModeledPayloadBytes
  ) {
    throw new RangeError(
      `fixture models ${cumulativeMaterializedBytes} checkpoint/Hall-of-Fame payload bytes, ` +
      `above the ${options.maxModeledPayloadBytes}-byte safety limit`
    );
  }

  const temporaryRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'slither-stage2-retention-'));
  const resolvedTemporaryRoot = path.resolve(temporaryRoot);
  const resolvedSystemTemp = path.resolve(os.tmpdir());
  if (
    path.dirname(resolvedTemporaryRoot) !== resolvedSystemTemp ||
    !path.basename(resolvedTemporaryRoot).startsWith('slither-stage2-retention-')
  ) {
    throw new Error(`unexpected temporary fixture path ${resolvedTemporaryRoot}`);
  }
  const checkpointDirectory = path.join(resolvedTemporaryRoot, 'checkpoints');
  const hofDirectory = path.join(resolvedTemporaryRoot, 'hof');
  const templateDirectory = path.join(resolvedTemporaryRoot, 'templates');
  let database: Database.Database | null = null;
  try {
    fs.mkdirSync(checkpointDirectory);
    fs.mkdirSync(hofDirectory);
    fs.mkdirSync(templateDirectory);
    const checkpointTemplatePath = path.join(templateDirectory, 'checkpoint-payload.bin');
    fs.writeFileSync(
      checkpointTemplatePath,
      deterministicPayload(checkpointBytes, 0x51e7c0de)
    );
    const maximumHofBytes = Math.ceil(aggregateHofBytes / populationCount);
    const hofTemplatePath = path.join(templateDirectory, 'hof-genome.bin');
    fs.writeFileSync(
      hofTemplatePath,
      deterministicPayload(maximumHofBytes, 0x70f0face)
    );

    const databasePath = path.join(resolvedTemporaryRoot, 'metadata.db');
    const openedDatabase = new Database(databasePath);
    database = openedDatabase;
    const transactionTimesMs: number[] = [];
    const checkpointDeletionTimesMs: number[] = [];
    const hofDeletionTimesMs: number[] = [];
    let activeCandidates: RetentionCandidate[] = [];
    let currentCheckpointBytes = 0;
    let peakCheckpointBytes = 0;
    let currentHofBytes = 0;
    let peakHofBytes = 0;
    let prunedCheckpointCount = 0;
    let prunedCheckpointBytes = 0;
    let prunedHofCount = 0;
    let prunedHofBytes = 0;
    const memoryBefore = process.memoryUsage();
    const maxRssBeforeKiB = process.resourceUsage().maxRSS;
    const statfsBefore = fs.statfsSync(resolvedTemporaryRoot);
    const freeDiskBefore = statfsBefore.bavail * statfsBefore.bsize;
    const fixtureStarted = performance.now();

    createFixtureSchema(openedDatabase);
    const insertCheckpoint = openedDatabase.prepare(`
      INSERT INTO checkpoints(
        checkpoint_key, run_id, generation, retention_class,
        file_name, file_bytes, created_ordinal
      ) VALUES (?, ?, ?, ?, ?, ?, ?)
    `);
    const updateCheckpointClass = openedDatabase.prepare(
      'UPDATE checkpoints SET retention_class = ? WHERE checkpoint_key = ?'
    );
    const deleteCheckpoint = openedDatabase.prepare(
      'DELETE FROM checkpoints WHERE checkpoint_key = ?'
    );
    const upsertCurrent = openedDatabase.prepare(`
      INSERT INTO current_checkpoint(singleton, checkpoint_key)
      VALUES (1, ?)
      ON CONFLICT(singleton) DO UPDATE SET checkpoint_key = excluded.checkpoint_key
    `);
    const insertHistory = openedDatabase.prepare(`
      INSERT INTO generation_history(run_id, generation, record)
      VALUES (?, ?, ?)
    `);
    const insertHofEntry = openedDatabase.prepare(`
      INSERT INTO hof_entries(
        run_id, generation, score, genome_key, weights_retained, metadata
      ) VALUES (?, ?, ?, ?, 1, ?)
    `);
    const insertHofGenome = openedDatabase.prepare(`
      INSERT INTO hof_genomes(genome_key, file_name, file_bytes)
      VALUES (?, ?, ?)
    `);
    const releaseHofWeights = openedDatabase.prepare(`
      UPDATE hof_entries
         SET genome_key = NULL, weights_retained = 0
       WHERE run_id = ? AND generation = ?
    `);
    const deleteHofGenome = openedDatabase.prepare(
      'DELETE FROM hof_genomes WHERE genome_key = ?'
    );

    const initialTransaction = openedDatabase.transaction(() => {
      for (let anchor = 1; anchor <= options.retention.priorRunAnchorCount; anchor++) {
        const key = `prior-anchor-${anchor}`;
        const fileName = `${key}.payload-fixture`;
        copyUniqueSizedFile(
          checkpointTemplatePath,
          path.join(checkpointDirectory, fileName),
          anchor
        );
        const candidate: RetentionCandidate = {
          key,
          runId: `prior-run-${anchor}`,
          generation: 10 + anchor,
          bytes: checkpointBytes,
          createdOrdinal: anchor,
          pinned: false,
          priorRunAnchor: true
        };
        activeCandidates.push(candidate);
        currentCheckpointBytes += checkpointBytes;
        insertCheckpoint.run(
          key,
          candidate.runId,
          candidate.generation,
          'prior-anchor',
          fileName,
          checkpointBytes,
          candidate.createdOrdinal
        );
      }
    });
    initialTransaction();
    peakCheckpointBytes = currentCheckpointBytes;

    for (let generation = 1; generation <= options.generations; generation++) {
      const checkpointKey = `current-${generation}`;
      const checkpointFileName = `${checkpointKey}.payload-fixture`;
      copyUniqueSizedFile(
        checkpointTemplatePath,
        path.join(checkpointDirectory, checkpointFileName),
        1_000 + generation
      );
      currentCheckpointBytes += checkpointBytes;
      peakCheckpointBytes = Math.max(peakCheckpointBytes, currentCheckpointBytes);
      const newCandidate: RetentionCandidate = {
        key: checkpointKey,
        runId: CURRENT_RUN_ID,
        generation,
        bytes: checkpointBytes,
        createdOrdinal: 1_000 + generation,
        pinned: false,
        priorRunAnchor: false
      };
      const decision = selectRetainedCheckpoints(
        [...activeCandidates, newCandidate],
        CURRENT_RUN_ID,
        options.retention
      );

      const hofBytes = perGenomeFixtureBytes(aggregateHofBytes, populationCount, generation);
      const hofBuffer = Buffer.from(
        fs.readFileSync(hofTemplatePath).subarray(0, hofBytes)
      );
      hofBuffer.writeBigUInt64LE(BigInt(generation), 0);
      const genomeKey = createHash('sha256').update(hofBuffer).digest('hex');
      const hofFileName = `${genomeKey}.weights-fixture`;
      fs.writeFileSync(path.join(hofDirectory, hofFileName), hofBuffer, { flag: 'wx' });
      currentHofBytes += hofBytes;
      peakHofBytes = Math.max(peakHofBytes, currentHofBytes);
      const outgoingHofGeneration = generation - options.hofUniqueLimit;
      let outgoingHof: { genome_key: string; file_name: string; file_bytes: number } | null = null;
      if (outgoingHofGeneration > 0) {
        outgoingHof = openedDatabase.prepare(`
          SELECT g.genome_key, g.file_name, g.file_bytes
            FROM hof_entries AS e
            JOIN hof_genomes AS g ON g.genome_key = e.genome_key
           WHERE e.run_id = ? AND e.generation = ?
        `).get(CURRENT_RUN_ID, outgoingHofGeneration) as typeof outgoingHof;
      }

      const transactionStarted = performance.now();
      const commit = openedDatabase.transaction(() => {
        insertCheckpoint.run(
          newCandidate.key,
          newCandidate.runId,
          newCandidate.generation,
          'recent',
          checkpointFileName,
          checkpointBytes,
          newCandidate.createdOrdinal
        );
        insertHistory.run(CURRENT_RUN_ID, generation, encodeHistoryRecord(generation));
        const hofMetadata = Buffer.alloc(32);
        hofMetadata.writeBigUInt64LE(BigInt(generation), 0);
        hofMetadata.writeDoubleLE(generation * 1.25, 8);
        insertHofGenome.run(genomeKey, hofFileName, hofBytes);
        insertHofEntry.run(
          CURRENT_RUN_ID,
          generation,
          generation * 1.25,
          genomeKey,
          hofMetadata
        );
        if (outgoingHof) {
          releaseHofWeights.run(CURRENT_RUN_ID, outgoingHofGeneration);
          deleteHofGenome.run(outgoingHof.genome_key);
        }
        for (const item of decision.kept) {
          updateCheckpointClass.run(item.retentionClass, item.key);
        }
        for (const item of decision.pruned) {
          deleteCheckpoint.run(item.key);
        }
        upsertCurrent.run(checkpointKey);
      });
      commit();
      transactionTimesMs.push(performance.now() - transactionStarted);

      for (const item of decision.pruned) {
        const deletionStarted = performance.now();
        fs.unlinkSync(path.join(checkpointDirectory, `${item.key}.payload-fixture`));
        checkpointDeletionTimesMs.push(performance.now() - deletionStarted);
        currentCheckpointBytes -= item.bytes;
        prunedCheckpointCount++;
        prunedCheckpointBytes += item.bytes;
      }
      if (outgoingHof) {
        const deletionStarted = performance.now();
        fs.unlinkSync(path.join(hofDirectory, outgoingHof.file_name));
        hofDeletionTimesMs.push(performance.now() - deletionStarted);
        currentHofBytes -= outgoingHof.file_bytes;
        prunedHofCount++;
        prunedHofBytes += outgoingHof.file_bytes;
      }
      activeCandidates = decision.kept.map(item => ({
        key: item.key,
        runId: item.runId,
        generation: item.generation,
        bytes: item.bytes,
        createdOrdinal: item.createdOrdinal,
        pinned: item.pinned,
        priorRunAnchor: item.priorRunAnchor
      }));
    }

    const beforeWalCheckpoint = {
      databaseBytes: fileSize(databasePath),
      walBytes: fileSize(`${databasePath}-wal`),
      shmBytes: fileSize(`${databasePath}-shm`),
      pageCount: openedDatabase.pragma('page_count', { simple: true }) as number,
      freelistCount: openedDatabase.pragma('freelist_count', { simple: true }) as number
    };
    const walCheckpointStarted = performance.now();
    const walCheckpointResult = openedDatabase.pragma('wal_checkpoint(PASSIVE)');
    const walCheckpointMs = performance.now() - walCheckpointStarted;
    const afterWalCheckpoint = {
      databaseBytes: fileSize(databasePath),
      walBytes: fileSize(`${databasePath}-wal`),
      shmBytes: fileSize(`${databasePath}-shm`),
      pageCount: openedDatabase.pragma('page_count', { simple: true }) as number,
      freelistCount: openedDatabase.pragma('freelist_count', { simple: true }) as number
    };
    const checkpointTotals = directoryTotals(checkpointDirectory);
    const hofTotals = directoryTotals(hofDirectory);
    const checkpointClasses = openedDatabase.prepare(`
      SELECT retention_class, COUNT(*) AS count, SUM(file_bytes) AS bytes
        FROM checkpoints
       GROUP BY retention_class
       ORDER BY retention_class
    `).all();
    const databaseCounts = {
      checkpoints: openedDatabase.prepare(`
        SELECT COUNT(*) AS records, COALESCE(SUM(file_bytes), 0) AS bytes
          FROM checkpoints
      `).get() as { records: number; bytes: number },
      history: openedDatabase.prepare(`
        SELECT COUNT(*) AS records, SUM(LENGTH(record)) AS logical_bytes
          FROM generation_history
      `).get() as { records: number; logical_bytes: number },
      hofEntries: openedDatabase.prepare(
        'SELECT COUNT(*) AS value FROM hof_entries'
      ).get() as { value: number },
      hofGenomes: openedDatabase.prepare(`
        SELECT COUNT(*) AS records, COALESCE(SUM(file_bytes), 0) AS bytes
          FROM hof_genomes
      `).get() as { records: number; bytes: number },
      definitions: openedDatabase.prepare(
        'SELECT COUNT(*) AS value FROM definitions'
      ).get() as { value: number },
      current: openedDatabase.prepare(`
        SELECT c.checkpoint_key, p.generation
          FROM current_checkpoint AS c
          JOIN checkpoints AS p ON p.checkpoint_key = c.checkpoint_key
      `).get() as { checkpoint_key: string; generation: number }
    };
    if (
      checkpointTotals.files !== databaseCounts.checkpoints.records ||
      checkpointTotals.bytes !== databaseCounts.checkpoints.bytes ||
      checkpointTotals.bytes !== currentCheckpointBytes
    ) {
      throw new Error('managed checkpoint file/metadata accounting mismatch');
    }
    if (
      hofTotals.files !== databaseCounts.hofGenomes.records ||
      hofTotals.bytes !== databaseCounts.hofGenomes.bytes ||
      hofTotals.bytes !== currentHofBytes
    ) {
      throw new Error('Hall-of-Fame file/metadata accounting mismatch');
    }
    if (
      databaseCounts.history.records !== options.generations ||
      databaseCounts.history.logical_bytes !== options.generations * HISTORY_RECORD_BYTES
    ) {
      throw new Error('compact history row/byte accounting mismatch');
    }
    if (
      databaseCounts.hofEntries.value !== options.generations ||
      databaseCounts.hofGenomes.records !== Math.min(options.generations, options.hofUniqueLimit)
    ) {
      throw new Error('Hall-of-Fame metadata/retained-unique accounting mismatch');
    }
    if (
      databaseCounts.current.checkpoint_key !== `current-${options.generations}` ||
      databaseCounts.current.generation !== options.generations
    ) {
      throw new Error('current pointer does not identify the latest retained checkpoint');
    }
    const statfsAfter = fs.statfsSync(resolvedTemporaryRoot);
    const memoryAfter = process.memoryUsage();
    return {
      fixtureKind: 'size-matched physical managed files plus SQLite metadata; not valid archives',
      scenario: artifact.data.fixture.scenario,
      generations: options.generations,
      representedRoundSeconds: 60,
      representedWallHours: options.generations / 60,
      selectedWeightEncoding: artifact.data.wholePopulation.selectedArchiveV1,
      retainedCodecArtifact: {
        path: artifact.relativePath,
        sha256: artifact.sha256,
        bytes: artifact.fileBytes
      },
      policy: options.retention,
      hallOfFameUniqueLimit: options.hofUniqueLimit,
      modeledPayloadBytes: {
        checkpointPayloads: cumulativeCheckpointBytes,
        hallOfFamePayloads: cumulativeHofBytes,
        historyRecords: options.generations * HISTORY_RECORD_BYTES,
        checkpointAndHallOfFameTotal: cumulativeMaterializedBytes,
        safetyLimit: options.maxModeledPayloadBytes,
        excludedFromSafetyLimit:
          'template files, SQLite/SHM/WAL bytes, filesystem metadata and allocation overhead'
      },
      managedCheckpointFiles: {
        final: checkpointTotals,
        peakBytesBeforePrune: peakCheckpointBytes,
        prunedFiles: prunedCheckpointCount,
        prunedBytes: prunedCheckpointBytes,
        deletionMs: distribution(checkpointDeletionTimesMs),
        classes: checkpointClasses
      },
      hallOfFameFiles: {
        final: hofTotals,
        peakBytesBeforePrune: peakHofBytes,
        prunedFiles: prunedHofCount,
        prunedBytes: prunedHofBytes,
        deletionMs: distribution(hofDeletionTimesMs)
      },
      sqlite: {
        metadataTransactionMs: distribution(transactionTimesMs),
        beforeWalCheckpoint,
        walCheckpointMs: Number(walCheckpointMs.toFixed(6)),
        walCheckpointResult,
        afterWalCheckpoint,
        peakWalMeasured: false,
        counts: databaseCounts
      },
      accountingAssertions: {
        checkpointFilesMatchMetadata: true,
        hallOfFameFilesMatchMetadata: true,
        hallOfFameMetadataAndUniqueLimitMatch: true,
        historyIsExactFixedWidth: true,
        currentPointerIsLatestRetained: true
      },
      limitations: {
        durability:
          'fixture payload copies are not fsynced or directory-synced; metadata transaction timing is not durable checkpoint publication latency',
        hallOfFame:
          'every generation is modeled as a new qualifying unique genome; duplicate hashes, non-qualifiers, pins and multiple-run scope are not exercised',
        wal:
          'only final WAL state and one final passive checkpoint are measured; peak WAL is not sampled'
      },
      resource: {
        fixtureWallMs: Number((performance.now() - fixtureStarted).toFixed(6)),
        memoryBefore,
        memoryAfter,
        processMaxRssBeforeBytes: maxRssBeforeKiB * 1024,
        processMaxRssAfterBytes: process.resourceUsage().maxRSS * 1024,
        freeDiskBeforeBytes: freeDiskBefore,
        freeDiskAfterBytes: statfsAfter.bavail * statfsAfter.bsize
      }
    };
  } finally {
    try {
      database?.close();
    } finally {
      fs.rmSync(resolvedTemporaryRoot, { recursive: true, force: true });
    }
  }
}

/**
 * Run all projections and one physical accelerated fixture.
 * @param options - Validated command options.
 * @returns Machine-readable evidence.
 */
function runBaseline(options: RetentionBaselineOptions): Record<string, unknown> {
  const artifacts = new Map<ProjectionScenario, LoadedCodecArtifact>();
  for (const scenario of ['P0', 'P1', 'P2', 'P3'] as const) {
    artifacts.set(scenario, loadCodecArtifact(scenario));
  }
  const projections = [...artifacts.values()].map(artifact => (
    projectScenario(artifact, options.generations, options.hofUniqueLimit, options.retention)
  ));
  const materializedArtifact = artifacts.get(options.materializeScenario);
  if (!materializedArtifact) throw new Error('materialized codec artifact is missing');
  return {
    schema: 'slither-stage2-retention-baseline',
    version: 1,
    evidenceClass: 'new measured result plus separately labelled derived arithmetic',
    caveat:
      'The physical fixture writes size-matched payload files and real SQLite metadata/history/Hall-of-Fame rows. It is not checkpoint-v3, not USTAR, does not fsync payload files, does not prove restore or durability, and is not target-VM evidence.',
    source: sourceIdentity(),
    environment: {
      capturedAt: new Date().toISOString(),
      platform: process.platform,
      architecture: process.arch,
      osType: os.type(),
      osRelease: os.release(),
      osVersion: os.version(),
      hostname: os.hostname(),
      node: process.version,
      sqlite: sqliteVersion()
    },
    approvedDefaults: {
      generations: options.generations,
      representedEightHoursAtSixtySecondGenerations: options.generations === 480,
      historyRecordBytes: HISTORY_RECORD_BYTES,
      hallOfFameUniqueLimit: options.hofUniqueLimit,
      retention: options.retention
    },
    projections,
    materialized: runMaterializedFixture(materializedArtifact, options)
  };
}

/**
 * Wrap the isolated history-row fixture in a retained-evidence envelope.
 * @param generations - Final compact-history record count.
 * @returns Machine-readable Stage 2 measurement artifact.
 */
function runHistoryOnlyBaseline(generations: number): Record<string, unknown> {
  return {
    schema: 'slither-stage2-history-sqlite-overhead',
    version: 1,
    evidenceClass: 'new measured result',
    caveat:
      'Disposable compact-history SQLite measurement only. It is not checkpoint-v3, an archive, a segment implementation, restore evidence, durability evidence, or target-VM evidence.',
    source: sourceIdentity(),
    environment: {
      capturedAt: new Date().toISOString(),
      platform: process.platform,
      architecture: process.arch,
      osType: os.type(),
      osRelease: os.release(),
      osVersion: os.version(),
      hostname: os.hostname(),
      node: process.version,
      sqlite: sqliteVersion()
    },
    measurement: runHistoryStorageMeasurement({ generations })
  };
}

/**
 * Publish one completed history-only JSON artifact without overwriting an
 * existing destination. The final hard-link creation is the no-overwrite
 * publication point; this is artifact safety, not a durability claim.
 * @param outputPath - Final artifact path that must not already exist.
 * @param json - Fully serialized artifact content.
 */
function writeHistoryArtifactNoOverwrite(outputPath: string, json: string): void {
  const parent = path.dirname(outputPath);
  const name = path.basename(outputPath);
  fs.mkdirSync(parent, { recursive: true });
  const temporaryDirectory = fs.mkdtempSync(path.join(parent, `.${name}.history-`));
  const resolvedTemporaryDirectory = path.resolve(temporaryDirectory);
  const resolvedParent = path.resolve(parent);
  const temporaryFile = path.join(resolvedTemporaryDirectory, 'artifact.json');
  try {
    if (
      path.dirname(resolvedTemporaryDirectory) !== resolvedParent ||
      !path.basename(resolvedTemporaryDirectory).startsWith(`.${name}.history-`)
    ) {
      throw new Error(`unexpected history artifact temporary path ${resolvedTemporaryDirectory}`);
    }
    const descriptor = fs.openSync(temporaryFile, 'wx');
    try {
      fs.writeFileSync(descriptor, json, 'utf8');
      fs.fsyncSync(descriptor);
    } finally {
      fs.closeSync(descriptor);
    }
    fs.linkSync(temporaryFile, outputPath);
  } finally {
    fs.rmSync(resolvedTemporaryDirectory, { recursive: true, force: true });
  }
}

/** Execute the CLI. */
function main(): void {
  const options = parseOptions(process.argv.slice(2));
  const result = options.historyOnly
    ? runHistoryOnlyBaseline(options.historyGenerations)
    : runBaseline(options);
  const json = `${JSON.stringify(result, null, 2)}\n`;
  if (options.outputPath) {
    if (options.historyOnly) {
      writeHistoryArtifactNoOverwrite(options.outputPath, json);
      console.info(`[stage2.history] wrote ${options.outputPath}`);
    } else {
      fs.mkdirSync(path.dirname(options.outputPath), { recursive: true });
      fs.writeFileSync(options.outputPath, json, 'utf8');
      console.info(`[stage2.retention] wrote ${options.outputPath}`);
    }
  } else {
    process.stdout.write(json);
  }
}

const invokedPath = process.argv[1];
if (invokedPath && import.meta.url === pathToFileURL(path.resolve(invokedPath)).href) {
  try {
    main();
  } catch (error) {
    console.error(error);
    process.exitCode = 1;
  }
}
