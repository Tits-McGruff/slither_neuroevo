import Database from 'better-sqlite3';
import { createHash } from 'node:crypto';
import { mkdirSync,mkdtempSync,rmSync,symlinkSync,writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach,type TestContext } from 'vitest';
import { CheckpointPersistenceClient } from './checkpointPersistenceClient.ts';
import type {
ManagedCheckpointDescriptor,
ManagedGenerationCommit,
ManagedGenerationSummary,
ManagedHallOfFameReference,
ManagedHallOfFameWeightsDescriptor
} from './checkpointPersistenceProtocol.ts';

/**
 * Test-suite label used in runner output.
 *
 * These fixtures cover descriptor validation and metadata transaction mechanics only. Real
 * checkpoint-file compatibility and corruption are owned by the integrated Rust-to-Node path,
 * where Rust validates the logical root during restore/startup.
 */
export const SUITE = 'minimal checkpoint metadata and compact-history persistence worker';
/** Disposable fixture roots removed after their clients stop. */
export const fixtureRoots: string[] = [];
/** Active clients closed before their temporary roots are removed. */
export const clients: CheckpointPersistenceClient[] = [];

/**
 * Format one small unsigned integer as the canonical descriptor u64 wire form.
 * @param value - Nonnegative value represented exactly in a test fixture.
 * @returns Fixed-width lowercase hexadecimal value.
 */
export function u64(value: bigint): string {
  return value.toString(16).padStart(16, '0');
}

/**
 * Encode one JavaScript fixture number as its exact big-endian IEEE-754 Float64 bits.
 * @param value - Finite fixture value.
 * @returns Fixed-width lowercase hexadecimal bits matching Rust `f64::to_bits()`.
 */
export function f64(value: number): string {
  const bytes = Buffer.allocUnsafe(8);
  bytes.writeDoubleBE(value);
  return bytes.toString('hex');
}

/**
 * Build one exact compact eight-field generation result.
 * @param completedGeneration - Generation whose round completed.
 * @param overrides - Optional field changes for rejection/idempotency tests.
 * @returns Strict small generation-history record.
 */
export function createGenerationSummary(
  completedGeneration: bigint,
  overrides: Partial<ManagedGenerationSummary> = {}
): ManagedGenerationSummary {
  return {
    completedGeneration: u64(completedGeneration),
    bestF64Hex: f64(12.5),
    averageF64Hex: f64(7.25),
    minimumF64Hex: f64(-1.5),
    speciesCount: u64(2n),
    topSpeciesSize: u64(1n),
    averageWeightF64Hex: f64(0.125),
    weightVarianceF64Hex: f64(0.03125),
    ...overrides
  };
}

/**
 * Build one compact reference to the elite already stored in a generation checkpoint.
 * @param completedGeneration - Generation whose elite was selected.
 * @param fitnessF64Hex - Exact best-fitness bits shared with compact history.
 * @param overrides - Optional field changes for validation and replay tests.
 * @returns Strict scalar Hall-of-Fame reference without duplicated genome weights.
 */
export function createHallOfFameReference(
  completedGeneration: bigint,
  fitnessF64Hex: string,
  overrides: Partial<ManagedHallOfFameReference> = {}
): ManagedHallOfFameReference {
  return {
    completedGeneration: u64(completedGeneration),
    sourcePopulationSlot: u64(1n),
    sourceSnakeId: u64(17n),
    fitnessF64Hex,
    pointsF64Hex: f64(6.25),
    length: u64(9n),
    successorPopulationSlot: u64(0n),
    successorGenomeId: u64(1_001n),
    ...overrides
  };
}

/** Write one small immutable winner-weight object into the current disposable managed root. */
export function createHallOfFameWeights(
  completedGeneration: bigint,
  overrides: Partial<ManagedHallOfFameWeightsDescriptor> = {}
): ManagedHallOfFameWeightsDescriptor {
  const root = fixtureRoots.at(-1);
  if (!root) throw new Error('Hall-of-Fame weights require an active fixture');
  const bytes = Buffer.alloc(16, Number(completedGeneration & 0xffn));
  const logicalSha256 = createHash('sha256').update(bytes).digest('hex');
  const descriptor: ManagedHallOfFameWeightsDescriptor = {
    version: 1,
    logicalSha256,
    relativeFilename: `${logicalSha256}.hof-weights-v1`,
    encoding: 'raw-f32le-v1',
    storedByteCount: u64(BigInt(bytes.length)),
    decodedByteCount: u64(BigInt(bytes.length)),
    weightCount: u64(BigInt(bytes.length / 4)),
    ...overrides
  };
  writeFileSync(join(root, 'managed-checkpoints', descriptor.relativeFilename), bytes);
  return descriptor;
}

/**
 * Build the complete small metadata transaction payload for one finished generation.
 * @param completedGeneration - Generation whose round completed.
 * @param summaryOverrides - Optional compact-history changes.
 * @param hallOfFameOverrides - Optional Hall-of-Fame reference changes.
 * @returns Atomic history/reference commit fixture.
 */
export function createGenerationCommit(
  completedGeneration: bigint,
  summaryOverrides: Partial<ManagedGenerationSummary> = {},
  hallOfFameOverrides: Partial<ManagedHallOfFameReference> = {}
): ManagedGenerationCommit {
  const summary = createGenerationSummary(completedGeneration, summaryOverrides);
  return {
    summary,
    hallOfFame: createHallOfFameReference(
      completedGeneration,
      summary.bestF64Hex,
      hallOfFameOverrides
    ),
    hallOfFameWeights: createHallOfFameWeights(completedGeneration)
  };
}

/**
 * Decode one stored 56-byte compact record back to its exact protocol fields.
 * @param record - SQLite BLOB returned by the disposable test database.
 * @returns Exact generation-summary wire values.
 */
export function decodeGenerationSummary(record: Buffer): ManagedGenerationSummary {
  if (record.length !== 56) throw new RangeError('history fixture record must contain 56 bytes');
  const hex = (offset: number): string => record.readBigUInt64LE(offset).toString(16).padStart(16, '0');
  return {
    completedGeneration: hex(0),
    bestF64Hex: hex(8),
    averageF64Hex: hex(16),
    minimumF64Hex: hex(24),
    speciesCount: u64(BigInt(record.readUInt32LE(32))),
    topSpeciesSize: u64(BigInt(record.readUInt32LE(36))),
    averageWeightF64Hex: hex(40),
    weightVarianceF64Hex: hex(48)
  };
}

/**
 * Decode one stored 56-byte Hall-of-Fame reference without numeric coercion.
 * @param record - SQLite BLOB returned by the disposable test database.
 * @returns Exact Hall-of-Fame wire values.
 */
export function decodeHallOfFameReference(record: Buffer): ManagedHallOfFameReference {
  if (record.length !== 56) throw new RangeError('Hall-of-Fame fixture record must contain 56 bytes');
  const hex = (offset: number): string => record.readBigUInt64LE(offset).toString(16).padStart(16, '0');
  return {
    completedGeneration: hex(0),
    sourcePopulationSlot: u64(BigInt(record.readUInt32LE(8))),
    successorPopulationSlot: u64(BigInt(record.readUInt32LE(12))),
    sourceSnakeId: hex(16),
    successorGenomeId: hex(24),
    fitnessF64Hex: hex(32),
    pointsF64Hex: hex(40),
    length: hex(48)
  };
}

/**
 * Build one isolated root, managed directory, and client-owned worker.
 * @returns Paths and client for a disposable real SQLite checkpoint publication test.
 */
export function createFixture(
  workerUrlForTesting?: URL,
  workerResponseModeForTesting?: 'invalid' | 'mismatched' | 'exit' | 'exit-clean' | 'stall' | 'stall-after-progress' | 'progressing',
  noProgressTimeoutMs?: number,
  checkpointCommitFailpointForTesting?: 'before-commit' | 'after-commit-before-reply',
  importCommitFailpointForTesting?: 'before-commit' | 'after-commit-before-reply'
): {
  root: string;
  managedRoot: string;
  databasePath: string;
  client: CheckpointPersistenceClient;
} {
  const root = mkdtempSync(join(tmpdir(), 'slither-checkpoint-worker-'));
  fixtureRoots.push(root);
  const managedRoot = join(root, 'managed-checkpoints');
  mkdirSync(managedRoot);
  const databasePath = join(root, 'metadata.sqlite');
  const client = new CheckpointPersistenceClient({
    databasePath,
    managedRootPath: managedRoot,
    ...(workerUrlForTesting ? { workerUrlForTesting } : {}),
    ...(workerResponseModeForTesting ? { workerResponseModeForTesting } : {}),
    ...(noProgressTimeoutMs ? { noProgressTimeoutMs } : {}),
    ...(checkpointCommitFailpointForTesting ? { checkpointCommitFailpointForTesting } : {}),
    ...(importCommitFailpointForTesting ? { importCommitFailpointForTesting } : {})
  });
  clients.push(client);
  return { root, managedRoot, databasePath, client };
}

/**
 * Create one final digest-derived file and matching scalar descriptor.
 * @param managedRoot - Controlled directory receiving the final immutable file.
 * @param options - Optional exact descriptor overrides.
 * @returns Strict descriptor ready for client submission.
 */
export function createDescriptor(
  managedRoot: string,
  options: Partial<ManagedCheckpointDescriptor> = {}
): ManagedCheckpointDescriptor {
  const bytes = Buffer.from(options.operationId ?? 'checkpoint-v3-fixture', 'utf8');
  const logicalRootSha256 = options.logicalRootSha256 ?? createHash('sha256').update(bytes).digest('hex');
  const relativeFilename = options.relativeFilename ?? `${logicalRootSha256}.checkpoint-v3`;
  if (!options.relativeFilename) writeFileSync(join(managedRoot, relativeFilename), bytes);
  return {
    protocolVersion: 1,
    operationId: '0123456789abcdef0123456789abcdef',
    transitionEpoch: u64(1n),
    runId: '8c50f3b8-b27a-4d88-b0bc-7a8557c0e456',
    generation: u64(1n),
    completedStep: u64(0n),
    boundaryKind: 'run-start',
    checkpointFormatVersion: u64(3n),
    stateVersion: u64(1n),
    graphLayoutVersion: u64(1n),
    managedRoot: 'checkpoint-v3',
    relativeFilename,
    logicalRootSha256,
    storedByteCount: u64(BigInt(bytes.length)),
    decodedByteCount: u64(BigInt(bytes.length)),
    roleCount: u64(3n),
    populationCount: u64(2n),
    weightCount: u64(8n),
    recurrentStateCount: u64(0n),
    weightsEncoding: 'raw-f32le-v1',
    recurrentStateEncoding: 'raw-f32le-v1',
    graphLayoutSha256: createHash('sha256').update('graph-layout').digest('hex'),
    writeValidationPolicy: 'write-hash-count-fsync-rename-v1',
    ...options
  };
}

/**
 * Read the current pointer after closing the worker that exclusively writes it.
 * @param databasePath - Disposable SQLite database path.
 * @param runId - Run whose pointer is inspected.
 * @returns Current checkpoint/root tuple, or undefined before first commit.
 */
export function readCurrentPointer(databasePath: string, runId: string): {
  checkpoint_id: string;
  transition_epoch: string;
  operation_id: string;
} | undefined {
  const db = new Database(databasePath, { readonly: true });
  try {
    return db.prepare(
      'SELECT checkpoint_id, transition_epoch, operation_id FROM rust_checkpoint_v3_current WHERE run_id = ?'
    ).get(runId) as { checkpoint_id: string; transition_epoch: string; operation_id: string } | undefined;
  } finally {
    db.close();
  }
}

/**
 * Create a symlink or skip only when the current platform disallows this test fixture.
 * @param context - Current Vitest test context.
 * @param target - Existing target file.
 * @param linkPath - New link path beneath the controlled root.
 * @returns True when the symlink exists and can be tested.
 */
export function createSymlinkOrSkip(context: TestContext, target: string, linkPath: string): boolean {
  try {
    symlinkSync(target, linkPath, 'file');
    return true;
  } catch (error) {
    const code = (error as NodeJS.ErrnoException).code;
    if (code === 'EPERM' || code === 'EACCES' || code === 'ENOSYS') {
      context.skip();
    }
    throw error;
  }
}

afterEach(async () => {
  for (const client of clients.splice(0)) await client.close().catch(() => {});
  for (const root of fixtureRoots.splice(0)) rmSync(root, { recursive: true, force: true });
});

// Allow worker startup, durable I/O and joined shutdown on shared runners.
/** Construct one strict fixed native metadata fixture without running simulation code. */
export function createExperimentPayload(base: ManagedGenerationCommit): Buffer {
  const bytes = Buffer.alloc(768);
  /** Set exact native record words for a synthetic metadata-only publication. */
  const word = (index: number, value: bigint): void => { bytes.writeBigUInt64LE(value, index * 8); };
  for (const [index, value] of [[0, 1n], [2, 1n], [3, 10n], [5, BigInt('0x' + base.completion!.actualElapsedF64Hex)],
    [6, BigInt('0x' + base.completion!.configuredDurationF64Hex)], [7, 2n], [8, 2n], [57, 1n], [58, BigInt('0x' + base.hallOfFame.pointsF64Hex)], [59, 1n], [61, 1n]] as const) word(index, value);
  for (let index = 20; index < 34; index++) word(index, BigInt('0x' + f64(6.25)));
  word(34, BigInt('0x' + f64(12.5))); word(36, BigInt('0x' + f64(12.5)));
  for (const [index, value] of [[0, 2n], [1, 1n], [2, 1n], [3, 17n], [5, 2n], [7, 1n], [8, BigInt('0x' + f64(6.25))],
    [13, 9n], [14, BigInt('0x' + f64(6.25))], [15, BigInt('0x' + f64(6.25))], [17, BigInt('0x' + f64(6.25))],
    [18, BigInt('0x' + f64(6.25))], [20, BigInt('0x' + f64(6.25))], [21, 1n], [24, 9n], [25, 9n], [26, 9n], [27, 9n], [28, 9n], [30, 1n]] as const) word(64 + index, value);
  return bytes;
}
