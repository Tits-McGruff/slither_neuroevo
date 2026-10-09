/** Pure Stage 2 model of the owner-selected automatic checkpoint retention rule. */

/** Checkpoint supplied to the retention selector. */
export interface RetentionCandidate {
  /** Stable fixture key. */
  key: string;
  /** Run identity. */
  runId: string;
  /** Generation boundary represented by this checkpoint. */
  generation: number;
  /** Stored managed-file bytes. */
  bytes: number;
  /** Monotonic creation order across runs. */
  createdOrdinal: number;
  /** Whether the owner explicitly pinned this checkpoint. */
  pinned: boolean;
  /** Whether this is the latest resumable anchor of a prior run. */
  priorRunAnchor: boolean;
}

/** Configurable owner-selected retention values. */
export interface RetentionSettings {
  /** Automatic checkpoints retained nearest the current boundary. */
  recentCount: number;
  /** Maximum older milestone slots. */
  milestoneCount: number;
  /** Generation interval qualifying an older checkpoint as a milestone. */
  milestoneInterval: number;
  /** Maximum prior-run latest anchors. */
  priorRunAnchorCount: number;
  /** Maximum bytes across unpinned automatic checkpoints and anchors. */
  automaticByteCap: number;
}

/** Retention class assigned to one kept checkpoint. */
export type RetentionClass = 'latest' | 'recent' | 'milestone' | 'prior-anchor' | 'pinned';

/** Kept checkpoint plus its selected class. */
export interface RetainedCheckpoint extends RetentionCandidate {
  /** Selected retention class. */
  retentionClass: RetentionClass;
}

/** Complete deterministic retention decision. */
export interface RetentionDecision {
  /** Checkpoints retained after count and byte limits. */
  kept: RetainedCheckpoint[];
  /** Superseded checkpoints eligible for deletion. */
  pruned: RetentionCandidate[];
  /** Bytes governed by the automatic cap. */
  automaticBytes: number;
  /** Pinned bytes outside the automatic cap. */
  pinnedBytes: number;
  /** Minimum protected automatic bytes. */
  protectedAutomaticBytes: number;
}

/** Owner-selected initial defaults approved in Draft 4. */
export const OWNER_RETENTION_DEFAULTS: RetentionSettings = {
  recentCount: 8,
  milestoneCount: 12,
  milestoneInterval: 25,
  priorRunAnchorCount: 2,
  automaticByteCap: 4 * 1024 * 1024 * 1024
};

/**
 * Validate one retention candidate before arithmetic or ordering.
 * @param candidate - Candidate to validate.
 */
function validateCandidate(candidate: RetentionCandidate): void {
  if (!candidate.key) throw new TypeError('checkpoint key must not be empty');
  if (!candidate.runId) throw new TypeError(`checkpoint ${candidate.key} has no run id`);
  if (!Number.isSafeInteger(candidate.generation) || candidate.generation < 1) {
    throw new RangeError(`checkpoint ${candidate.key} has an invalid generation`);
  }
  if (!Number.isSafeInteger(candidate.bytes) || candidate.bytes < 0) {
    throw new RangeError(`checkpoint ${candidate.key} has invalid stored bytes`);
  }
  if (!Number.isSafeInteger(candidate.createdOrdinal) || candidate.createdOrdinal < 0) {
    throw new RangeError(`checkpoint ${candidate.key} has invalid creation order`);
  }
}

/**
 * Validate configurable retention values.
 * @param settings - Settings to validate.
 */
function validateSettings(settings: RetentionSettings): void {
  const integerFields = [
    ['recentCount', settings.recentCount],
    ['milestoneCount', settings.milestoneCount],
    ['milestoneInterval', settings.milestoneInterval],
    ['priorRunAnchorCount', settings.priorRunAnchorCount],
    ['automaticByteCap', settings.automaticByteCap]
  ] as const;
  for (const [name, value] of integerFields) {
    if (!Number.isSafeInteger(value) || value < 1) {
      throw new RangeError(`${name} must be a positive safe integer`);
    }
  }
  if (settings.recentCount < 2) {
    throw new RangeError('recentCount must preserve the latest checkpoint and one predecessor');
  }
}

/**
 * Sum stored bytes with checked safe-integer arithmetic.
 * @param candidates - Checkpoints whose bytes are summed.
 * @returns Exact byte sum.
 */
function sumBytes(candidates: readonly RetentionCandidate[]): number {
  let total = 0;
  for (const candidate of candidates) {
    total += candidate.bytes;
    if (!Number.isSafeInteger(total)) throw new RangeError('retention byte sum exceeds safe integer');
  }
  return total;
}

/**
 * Apply the approved recent/milestone/anchor/pin policy and automatic byte cap.
 * Pinned checkpoints are never selected for automatic pruning and do not
 * consume the automatic cap. The cap removes oldest milestones first and then
 * oldest non-protected recent checkpoints.
 * @param candidates - All current fixture checkpoints.
 * @param currentRunId - Run whose recent and milestone generations are selected.
 * @param settings - Configurable retention values.
 * @returns Kept/pruned sets and cap accounting.
 */
export function selectRetainedCheckpoints(
  candidates: readonly RetentionCandidate[],
  currentRunId: string,
  settings: RetentionSettings = OWNER_RETENTION_DEFAULTS
): RetentionDecision {
  if (!currentRunId) throw new TypeError('currentRunId must not be empty');
  validateSettings(settings);
  const keys = new Set<string>();
  for (const candidate of candidates) {
    validateCandidate(candidate);
    if (keys.has(candidate.key)) throw new Error(`duplicate checkpoint key ${candidate.key}`);
    keys.add(candidate.key);
  }

  const pinned = candidates
    .filter(candidate => candidate.pinned)
    .sort((left, right) => left.createdOrdinal - right.createdOrdinal);
  const currentAutomatic = candidates
    .filter(candidate => (
      !candidate.pinned &&
      !candidate.priorRunAnchor &&
      candidate.runId === currentRunId
    ))
    .sort((left, right) => (
      right.generation - left.generation ||
      right.createdOrdinal - left.createdOrdinal
    ));
  const latestAnchorByRun = new Map<string, RetentionCandidate>();
  for (const candidate of candidates) {
    if (
      candidate.pinned ||
      !candidate.priorRunAnchor ||
      candidate.runId === currentRunId
    ) {
      continue;
    }
    const previous = latestAnchorByRun.get(candidate.runId);
    if (!previous || candidate.createdOrdinal > previous.createdOrdinal) {
      latestAnchorByRun.set(candidate.runId, candidate);
    }
  }
  const anchors = [...latestAnchorByRun.values()]
    .sort((left, right) => right.createdOrdinal - left.createdOrdinal)
    .slice(0, settings.priorRunAnchorCount);
  const recent = currentAutomatic.slice(0, settings.recentCount);
  const recentKeys = new Set(recent.map(candidate => candidate.key));
  const milestones = currentAutomatic
    .filter(candidate => (
      !recentKeys.has(candidate.key) &&
      candidate.generation % settings.milestoneInterval === 0
    ))
    .slice(0, settings.milestoneCount);

  const protectedRecent = recent.slice(0, Math.min(2, recent.length));
  const protectedKeys = new Set([
    ...protectedRecent.map(candidate => candidate.key),
    ...anchors.map(candidate => candidate.key)
  ]);
  const protectedAutomaticBytes = sumBytes([...protectedRecent, ...anchors]);
  if (protectedAutomaticBytes > settings.automaticByteCap) {
    throw new RangeError(
      `protected automatic checkpoints require ${protectedAutomaticBytes} bytes, ` +
      `above the ${settings.automaticByteCap}-byte cap`
    );
  }

  const keptAutomatic = new Map<string, RetainedCheckpoint>();
  for (let index = 0; index < recent.length; index++) {
    const candidate = recent[index]!;
    keptAutomatic.set(candidate.key, {
      ...candidate,
      retentionClass: index === 0 ? 'latest' : 'recent'
    });
  }
  for (const candidate of milestones) {
    keptAutomatic.set(candidate.key, { ...candidate, retentionClass: 'milestone' });
  }
  for (const candidate of anchors) {
    keptAutomatic.set(candidate.key, { ...candidate, retentionClass: 'prior-anchor' });
  }

  let automaticBytes = sumBytes([...keptAutomatic.values()]);
  const removableMilestones = milestones
    .slice()
    .sort((left, right) => left.generation - right.generation);
  for (const candidate of removableMilestones) {
    if (automaticBytes <= settings.automaticByteCap) break;
    keptAutomatic.delete(candidate.key);
    automaticBytes -= candidate.bytes;
  }

  const removableRecent = recent
    .filter(candidate => !protectedKeys.has(candidate.key))
    .sort((left, right) => left.generation - right.generation);
  for (const candidate of removableRecent) {
    if (automaticBytes <= settings.automaticByteCap) break;
    keptAutomatic.delete(candidate.key);
    automaticBytes -= candidate.bytes;
  }
  if (automaticBytes > settings.automaticByteCap) {
    throw new RangeError(
      `automatic checkpoints still require ${automaticBytes} bytes after allowed pruning`
    );
  }

  const kept: RetainedCheckpoint[] = [
    ...keptAutomatic.values(),
    ...pinned.map(candidate => ({ ...candidate, retentionClass: 'pinned' as const }))
  ].sort((left, right) => left.createdOrdinal - right.createdOrdinal);
  const keptKeys = new Set(kept.map(candidate => candidate.key));
  const pruned = candidates
    .filter(candidate => !keptKeys.has(candidate.key))
    .sort((left, right) => left.createdOrdinal - right.createdOrdinal);
  return {
    kept,
    pruned,
    automaticBytes,
    pinnedBytes: sumBytes(pinned),
    protectedAutomaticBytes
  };
}
