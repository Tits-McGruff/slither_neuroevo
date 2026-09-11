/** Production checkpoint-retention selection and bounded inventory accounting. */

/** Largest unsigned integer representable by the persistence wire contract. */
const MAX_U64 = 0xffff_ffff_ffff_ffffn;

/** Stored numeric encodings reported by managed checkpoint descriptors. */
export type RetentionNumericEncoding = 'raw-f32le-v1' | 'f32le-shuffle4-zstd-v1';

/** One immutable managed file considered by the retention selector. */
export interface CheckpointRetentionCandidate {
  /** Content-addressed managed checkpoint identity. */
  checkpointId: string;
  /** Effective run used for current-lineage selection. */
  runId: string;
  /** Exact generation boundary. */
  generation: bigint;
  /** Stored managed-file bytes. */
  storedBytes: bigint;
  /** Encoding-independent decoded role bytes. */
  decodedBytes: bigint;
  /** Stable database creation order. */
  createdOrdinal: bigint;
  /** Whether the owner explicitly protected this checkpoint. */
  pinned: boolean;
  /** Whether this is the newest boundary of an eligible prior run. */
  priorRunAnchor: boolean;
  /** Population-weight encoding selected by Rust. */
  weightsEncoding: RetentionNumericEncoding;
  /** Recurrent-state encoding selected by Rust. */
  recurrentStateEncoding: RetentionNumericEncoding;
}

/** Owner-configurable automatic retention values. */
export interface CheckpointRetentionSettings {
  /** Current-lineage boundaries nearest the latest boundary. */
  recentCount: number;
  /** Maximum older milestone boundaries. */
  milestoneCount: number;
  /** Generation interval qualifying as a milestone. */
  milestoneInterval: bigint;
  /** Maximum newest prior-run anchors. */
  priorRunAnchorCount: number;
  /** Maximum stored bytes across unpinned automatic checkpoints. */
  automaticByteCap: bigint;
}

/** Retention role assigned to a kept managed checkpoint. */
export type CheckpointRetentionClass = 'latest' | 'recent' | 'milestone' | 'prior-anchor' | 'pinned';

/** Kept candidate with its exclusive reporting class. */
export interface RetainedManagedCheckpoint extends CheckpointRetentionCandidate {
  /** Highest-priority reason the file remains protected. */
  retentionClass: CheckpointRetentionClass;
}

/** Deterministic keep/prune decision made without filesystem mutation. */
export interface CheckpointRetentionDecision {
  /** Managed files retained by count, byte, anchor, or pin rules. */
  kept: RetainedManagedCheckpoint[];
  /** Superseded unpinned files eligible for later verified deletion. */
  pruned: CheckpointRetentionCandidate[];
  /** Stored bytes charged to the automatic cap. */
  automaticBytes: bigint;
  /** Pinned bytes outside the automatic cap. */
  pinnedBytes: bigint;
  /** Non-negotiable current/predecessor/prior-anchor bytes. */
  protectedAutomaticBytes: bigint;
}

/** Compact encoding counts for one retention class. */
export interface CheckpointRetentionEncodingCounts {
  /** Checkpoints whose population weights are raw Float32 bytes. */
  rawWeights: number;
  /** Checkpoints whose population weights use shuffled Zstandard blocks. */
  shuffledZstdWeights: number;
  /** Checkpoints whose recurrent state is raw Float32 bytes. */
  rawRecurrent: number;
  /** Checkpoints whose recurrent state uses shuffled Zstandard blocks. */
  shuffledZstdRecurrent: number;
}

/** Bounded aggregate for one exclusive retention class. */
export interface CheckpointRetentionBucket {
  /** Number of immutable managed files. */
  checkpointCount: number;
  /** Exact stored file bytes as a canonical u64. */
  storedByteCount: string;
  /** Exact decoded logical role bytes as a canonical u64. */
  decodedByteCount: string;
  /** Selected numeric encoding counts. */
  encodings: CheckpointRetentionEncodingCounts;
}

/** Small retention inventory suitable for health and worker boundaries. */
export interface CheckpointRetentionInventory {
  /** Inventory protocol version. */
  schemaVersion: 1;
  /** Effective active run used by the selector. */
  activeRunId: string;
  /** Configured cap for unpinned automatic files. */
  automaticByteCap: string;
  /** Automatic stored bytes after planned pruning. */
  automaticStoredByteCount: string;
  /** Minimum non-prunable automatic bytes. */
  protectedAutomaticStoredByteCount: string;
  /** Pinned stored bytes outside the automatic cap. */
  pinnedStoredByteCount: string;
  /** Exclusive retained-file classes. */
  retained: {
    /** Current checkpoint. */
    latest: CheckpointRetentionBucket;
    /** Other newest current-lineage checkpoints. */
    recent: CheckpointRetentionBucket;
    /** Older interval checkpoints. */
    milestone: CheckpointRetentionBucket;
    /** Newest protected prior-run checkpoints. */
    priorRunAnchor: CheckpointRetentionBucket;
    /** Owner-pinned checkpoints. */
    pinned: CheckpointRetentionBucket;
  };
  /** Superseded files a later verified prune transaction may remove. */
  plannedPrune: CheckpointRetentionBucket;
}

/** Result of one verified automatic pruning pass. */
export interface CheckpointPruneResult {
  /** Number of immutable files removed or confirmed already absent. */
  deletedCheckpointCount: number;
  /** Exact descriptor bytes represented by those files. */
  deletedStoredByteCount: string;
  /** Retention state after every deletion was durably classified. */
  inventory: CheckpointRetentionInventory;
}

/** Owner-approved initial production defaults. */
export const OWNER_CHECKPOINT_RETENTION_DEFAULTS: CheckpointRetentionSettings = {
  recentCount: 8,
  milestoneCount: 12,
  milestoneInterval: 25n,
  priorRunAnchorCount: 2,
  automaticByteCap: 4n * 1024n * 1024n * 1024n
};

/** Validate one candidate before ordering or byte arithmetic. */
function validateCandidate(candidate: CheckpointRetentionCandidate): void {
  if (!/^[0-9a-f]{64}$/u.test(candidate.checkpointId)) throw new TypeError('invalid retention checkpoint ID');
  if (!candidate.runId || candidate.runId.includes('\0') || Buffer.byteLength(candidate.runId) > 256) {
    throw new TypeError(`checkpoint ${candidate.checkpointId} has an invalid run ID`);
  }
  for (const [label, value] of [
    ['generation', candidate.generation], ['stored bytes', candidate.storedBytes],
    ['decoded bytes', candidate.decodedBytes], ['creation order', candidate.createdOrdinal]
  ] as const) {
    if (value < 0n || value > MAX_U64 || (label === 'generation' && value === 0n)) {
      throw new RangeError(`checkpoint ${candidate.checkpointId} has invalid ${label}`);
    }
  }
}

/** Validate settings before selecting any managed file. */
function validateSettings(settings: CheckpointRetentionSettings): void {
  for (const [label, value] of [
    ['recentCount', settings.recentCount], ['milestoneCount', settings.milestoneCount],
    ['priorRunAnchorCount', settings.priorRunAnchorCount]
  ] as const) {
    if (!Number.isSafeInteger(value) || value < 1) throw new RangeError(`${label} must be a positive safe integer`);
  }
  if (settings.recentCount < 2) throw new RangeError('recentCount must preserve the latest checkpoint and one predecessor');
  if (settings.milestoneInterval < 1n || settings.milestoneInterval > MAX_U64) {
    throw new RangeError('milestoneInterval must be a positive u64');
  }
  if (settings.automaticByteCap < 1n || settings.automaticByteCap > MAX_U64) {
    throw new RangeError('automaticByteCap must be a positive u64');
  }
}

/** Sum exact bytes without permitting the reporting contract to overflow. */
function sumBytes(candidates: readonly CheckpointRetentionCandidate[]): bigint {
  let total = 0n;
  for (const candidate of candidates) {
    total += candidate.storedBytes;
    if (total > MAX_U64) throw new RangeError('retention byte sum exceeds u64');
  }
  return total;
}

/** Encode one admitted exact byte value for the scalar worker/health boundary. */
function u64(value: bigint): string {
  if (value < 0n || value > MAX_U64) throw new RangeError('retention value exceeds u64');
  return value.toString(16).padStart(16, '0');
}

/** Aggregate one exclusive candidate set without retaining checkpoint identities. */
function bucket(candidates: readonly CheckpointRetentionCandidate[]): CheckpointRetentionBucket {
  let decodedBytes = 0n;
  const encodings: CheckpointRetentionEncodingCounts = {
    rawWeights: 0, shuffledZstdWeights: 0, rawRecurrent: 0, shuffledZstdRecurrent: 0
  };
  for (const candidate of candidates) {
    decodedBytes += candidate.decodedBytes;
    if (decodedBytes > MAX_U64) throw new RangeError('retention decoded-byte sum exceeds u64');
    if (candidate.weightsEncoding === 'raw-f32le-v1') encodings.rawWeights++;
    else encodings.shuffledZstdWeights++;
    if (candidate.recurrentStateEncoding === 'raw-f32le-v1') encodings.rawRecurrent++;
    else encodings.shuffledZstdRecurrent++;
  }
  return {
    checkpointCount: candidates.length,
    storedByteCount: u64(sumBytes(candidates)),
    decodedByteCount: u64(decodedBytes),
    encodings
  };
}

/** Compare newest boundaries first without narrowing bigint values. */
function newestFirst(left: CheckpointRetentionCandidate, right: CheckpointRetentionCandidate): number {
  if (left.generation !== right.generation) return left.generation > right.generation ? -1 : 1;
  if (left.createdOrdinal !== right.createdOrdinal) return left.createdOrdinal > right.createdOrdinal ? -1 : 1;
  return left.checkpointId.localeCompare(right.checkpointId);
}

/** Compare stable creation order from oldest to newest. */
function oldestFirst(left: CheckpointRetentionCandidate, right: CheckpointRetentionCandidate): number {
  if (left.createdOrdinal !== right.createdOrdinal) return left.createdOrdinal < right.createdOrdinal ? -1 : 1;
  return left.checkpointId.localeCompare(right.checkpointId);
}

/**
 * Apply the approved recent/milestone/prior-anchor/pin rule and automatic byte cap.
 *
 * This function only decides eligibility. The persistence worker separately rechecks all
 * metadata and file references before a later pruning slice unlinks anything.
 */
export function selectManagedCheckpointRetention(
  candidates: readonly CheckpointRetentionCandidate[],
  currentRunId: string,
  settings: CheckpointRetentionSettings = OWNER_CHECKPOINT_RETENTION_DEFAULTS
): CheckpointRetentionDecision {
  if (!currentRunId || currentRunId.includes('\0') || Buffer.byteLength(currentRunId) > 256) {
    throw new TypeError('currentRunId must be a bounded nonempty string');
  }
  validateSettings(settings);
  const ids = new Set<string>();
  for (const candidate of candidates) {
    validateCandidate(candidate);
    if (ids.has(candidate.checkpointId)) throw new Error(`duplicate checkpoint ${candidate.checkpointId}`);
    ids.add(candidate.checkpointId);
  }

  const pinned = candidates.filter(candidate => candidate.pinned).sort(oldestFirst);
  const latestCurrent = candidates.filter(candidate => !candidate.priorRunAnchor &&
    candidate.runId === currentRunId).sort(newestFirst)[0];
  const current = candidates.filter(candidate => !candidate.pinned && !candidate.priorRunAnchor &&
    candidate.runId === currentRunId).sort(newestFirst);
  const latestAnchorByRun = new Map<string, CheckpointRetentionCandidate>();
  for (const candidate of candidates) {
    if (candidate.pinned || !candidate.priorRunAnchor || candidate.runId === currentRunId) continue;
    const previous = latestAnchorByRun.get(candidate.runId);
    if (!previous || newestFirst(candidate, previous) < 0) latestAnchorByRun.set(candidate.runId, candidate);
  }
  const anchors = [...latestAnchorByRun.values()].sort((left, right) => {
    if (left.createdOrdinal !== right.createdOrdinal) return left.createdOrdinal > right.createdOrdinal ? -1 : 1;
    return left.checkpointId.localeCompare(right.checkpointId);
  }).slice(0, settings.priorRunAnchorCount);
  const recent = current.slice(0, settings.recentCount);
  const recentIds = new Set(recent.map(candidate => candidate.checkpointId));
  const milestones = current.filter(candidate => !recentIds.has(candidate.checkpointId) &&
    candidate.generation % settings.milestoneInterval === 0n).slice(0, settings.milestoneCount);

  const protectedRecent = recent.slice(0, Math.min(2, recent.length));
  const protectedIds = new Set([...protectedRecent, ...anchors].map(candidate => candidate.checkpointId));
  const protectedAutomaticBytes = sumBytes([...protectedRecent, ...anchors]);
  if (protectedAutomaticBytes > settings.automaticByteCap) {
    throw new RangeError(`protected automatic checkpoints require ${protectedAutomaticBytes} bytes, above the ${settings.automaticByteCap}-byte cap`);
  }

  const automatic = new Map<string, RetainedManagedCheckpoint>();
  recent.forEach(candidate => automatic.set(candidate.checkpointId, {
    ...candidate, retentionClass: candidate.checkpointId === latestCurrent?.checkpointId ? 'latest' : 'recent'
  }));
  milestones.forEach(candidate => automatic.set(candidate.checkpointId, { ...candidate, retentionClass: 'milestone' }));
  anchors.forEach(candidate => automatic.set(candidate.checkpointId, { ...candidate, retentionClass: 'prior-anchor' }));
  let automaticBytes = sumBytes([...automatic.values()]);

  for (const candidate of milestones.slice().sort((left, right) => newestFirst(right, left))) {
    if (automaticBytes <= settings.automaticByteCap) break;
    automatic.delete(candidate.checkpointId);
    automaticBytes -= candidate.storedBytes;
  }
  for (const candidate of recent.filter(candidate => !protectedIds.has(candidate.checkpointId))
    .sort((left, right) => newestFirst(right, left))) {
    if (automaticBytes <= settings.automaticByteCap) break;
    automatic.delete(candidate.checkpointId);
    automaticBytes -= candidate.storedBytes;
  }
  if (automaticBytes > settings.automaticByteCap) {
    throw new RangeError(`automatic checkpoints still require ${automaticBytes} bytes after allowed pruning`);
  }

  const kept = [...automatic.values(), ...pinned.map(candidate => ({ ...candidate,
    retentionClass: candidate.checkpointId === latestCurrent?.checkpointId ? 'latest' as const : 'pinned' as const }))]
    .sort(oldestFirst);
  const keptIds = new Set(kept.map(candidate => candidate.checkpointId));
  return {
    kept,
    pruned: candidates.filter(candidate => !keptIds.has(candidate.checkpointId)).sort(oldestFirst),
    automaticBytes,
    pinnedBytes: sumBytes(pinned),
    protectedAutomaticBytes
  };
}

/** Build the bounded health/worker inventory from one already-validated decision. */
export function buildCheckpointRetentionInventory(
  decision: CheckpointRetentionDecision,
  activeRunId: string,
  settings: CheckpointRetentionSettings = OWNER_CHECKPOINT_RETENTION_DEFAULTS
): CheckpointRetentionInventory {
  validateSettings(settings);
  const retained = (retentionClass: CheckpointRetentionClass): CheckpointRetentionBucket => bucket(
    retentionClass === 'pinned'
      ? decision.kept.filter(item => item.pinned)
      : decision.kept.filter(item => item.retentionClass === retentionClass)
  );
  return {
    schemaVersion: 1,
    activeRunId,
    automaticByteCap: u64(settings.automaticByteCap),
    automaticStoredByteCount: u64(decision.automaticBytes),
    protectedAutomaticStoredByteCount: u64(decision.protectedAutomaticBytes),
    pinnedStoredByteCount: u64(decision.pinnedBytes),
    retained: {
      latest: retained('latest'),
      recent: retained('recent'),
      milestone: retained('milestone'),
      priorRunAnchor: retained('prior-anchor'),
      pinned: retained('pinned')
    },
    plannedPrune: bucket(decision.pruned)
  };
}

/** Require one plain object with exactly the expected keys. */
function exactRecord(value: unknown, keys: readonly string[], label: string): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new TypeError(`invalid ${label}`);
  const record = value as Record<string, unknown>;
  if (Object.keys(record).length !== keys.length || keys.some(key => !Object.hasOwn(record, key))) {
    throw new TypeError(`invalid ${label} fields`);
  }
  return record;
}

/** Parse one nonnegative bounded JavaScript count. */
function count(value: unknown, label: string): number {
  if (!Number.isSafeInteger(value) || (value as number) < 0) throw new TypeError(`invalid ${label}`);
  return value as number;
}

/** Parse one canonical u64 wire value. */
function u64Wire(value: unknown, label: string): string {
  if (typeof value !== 'string' || !/^[0-9a-f]{16}$/u.test(value)) throw new TypeError(`invalid ${label}`);
  return value;
}

/** Parse one worker-returned aggregate without trusting nested fields. */
function parseBucket(value: unknown, label: string): CheckpointRetentionBucket {
  const record = exactRecord(value, ['checkpointCount', 'storedByteCount', 'decodedByteCount', 'encodings'], label);
  const encodings = exactRecord(record['encodings'], [
    'rawWeights', 'shuffledZstdWeights', 'rawRecurrent', 'shuffledZstdRecurrent'
  ], `${label} encodings`);
  const parsedEncodings: CheckpointRetentionEncodingCounts = {
    rawWeights: count(encodings['rawWeights'], `${label} raw weight count`),
    shuffledZstdWeights: count(encodings['shuffledZstdWeights'], `${label} shuffled weight count`),
    rawRecurrent: count(encodings['rawRecurrent'], `${label} raw recurrent count`),
    shuffledZstdRecurrent: count(encodings['shuffledZstdRecurrent'], `${label} shuffled recurrent count`)
  };
  const checkpointCount = count(record['checkpointCount'], `${label} checkpoint count`);
  if (parsedEncodings.rawWeights + parsedEncodings.shuffledZstdWeights !== checkpointCount ||
      parsedEncodings.rawRecurrent + parsedEncodings.shuffledZstdRecurrent !== checkpointCount) {
    throw new TypeError(`invalid ${label} encoding totals`);
  }
  return {
    checkpointCount,
    storedByteCount: u64Wire(record['storedByteCount'], `${label} stored bytes`),
    decodedByteCount: u64Wire(record['decodedByteCount'], `${label} decoded bytes`),
    encodings: parsedEncodings
  };
}

/** Validate a complete retention inventory returned by the isolated worker. */
export function parseCheckpointRetentionInventory(value: unknown): CheckpointRetentionInventory {
  const record = exactRecord(value, [
    'schemaVersion', 'activeRunId', 'automaticByteCap', 'automaticStoredByteCount',
    'protectedAutomaticStoredByteCount', 'pinnedStoredByteCount', 'retained', 'plannedPrune'
  ], 'checkpoint retention inventory');
  if (record['schemaVersion'] !== 1 || typeof record['activeRunId'] !== 'string' ||
      !record['activeRunId'] || record['activeRunId'].includes('\0') || Buffer.byteLength(record['activeRunId']) > 256) {
    throw new TypeError('invalid checkpoint retention inventory identity');
  }
  const retained = exactRecord(record['retained'], [
    'latest', 'recent', 'milestone', 'priorRunAnchor', 'pinned'
  ], 'checkpoint retention classes');
  return {
    schemaVersion: 1,
    activeRunId: record['activeRunId'],
    automaticByteCap: u64Wire(record['automaticByteCap'], 'automatic byte cap'),
    automaticStoredByteCount: u64Wire(record['automaticStoredByteCount'], 'automatic stored bytes'),
    protectedAutomaticStoredByteCount: u64Wire(record['protectedAutomaticStoredByteCount'], 'protected automatic bytes'),
    pinnedStoredByteCount: u64Wire(record['pinnedStoredByteCount'], 'pinned stored bytes'),
    retained: {
      latest: parseBucket(retained['latest'], 'latest retention'),
      recent: parseBucket(retained['recent'], 'recent retention'),
      milestone: parseBucket(retained['milestone'], 'milestone retention'),
      priorRunAnchor: parseBucket(retained['priorRunAnchor'], 'prior-run retention'),
      pinned: parseBucket(retained['pinned'], 'pinned retention')
    },
    plannedPrune: parseBucket(record['plannedPrune'], 'planned prune')
  };
}

/** Validate a complete automatic-prune result returned by the isolated worker. */
export function parseCheckpointPruneResult(value: unknown): CheckpointPruneResult {
  const record = exactRecord(value, [
    'deletedCheckpointCount', 'deletedStoredByteCount', 'inventory'
  ], 'checkpoint prune result');
  return {
    deletedCheckpointCount: count(record['deletedCheckpointCount'], 'deleted checkpoint count'),
    deletedStoredByteCount: u64Wire(record['deletedStoredByteCount'], 'deleted stored bytes'),
    inventory: parseCheckpointRetentionInventory(record['inventory'])
  };
}
