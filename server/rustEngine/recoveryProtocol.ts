import { parseManagedCheckpointDescriptor, parseCheckpointOperationId,
  type ManagedCheckpointDescriptor, type U64Hex } from './checkpointPersistenceProtocol.ts';

/** Worker transaction input after Rust has validated the selected immutable content. */
export interface RecoveryBranchCommit {
  /** Idempotent transaction correlation. */
  operationId: string;
  /** Fresh lineage, never an existing source run. */
  branchRunId: string;
  /** Failed active lineage whose suffix remains preserved. */
  sourceRunId: string;
  /** Exact failed pointer identity observed before candidate validation. */
  failedCheckpointId: string;
  /** Original immutable descriptor, including its original lineage. */
  recoveredDescriptor: ManagedCheckpointDescriptor;
}

/** Durable recovery provenance; history references stop before the recovered round. */
export interface RecoveryBranchResult extends RecoveryBranchCommit {
  /** Highest abandoned retained boundary in the failed lineage. */
  abandonedThroughGeneration: U64Hex;
}

/** Require a bounded opaque run identity without numeric coercion. */
function runId(value: unknown): string {
  if (typeof value !== 'string' || !value || value.includes('\0') || Buffer.byteLength(value) > 256 ||
      Buffer.from(value, 'utf8').toString('utf8') !== value) {
    throw new TypeError('invalid recovery run identity');
  }
  return value;
}

/** Validate all bounded recovery fields before they cross a worker boundary. */
export function parseRecoveryBranchCommit(value: unknown): RecoveryBranchCommit {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new TypeError('invalid recovery commit');
  const raw = value as Record<string, unknown>;
  const keys = ['operationId', 'branchRunId', 'sourceRunId', 'failedCheckpointId', 'recoveredDescriptor'];
  if (Object.keys(raw).length !== keys.length || keys.some(key => !Object.hasOwn(raw, key))) {
    throw new TypeError('invalid recovery commit fields');
  }
  const branchRunId = runId(raw['branchRunId']);
  const sourceRunId = runId(raw['sourceRunId']);
  const recoveredDescriptor = parseManagedCheckpointDescriptor(raw['recoveredDescriptor']);
  if (branchRunId === sourceRunId || branchRunId === recoveredDescriptor.runId || recoveredDescriptor.generation === '0000000000000000') {
    throw new TypeError('recovery requires a distinct branch of the selected source run');
  }
  if (typeof raw['failedCheckpointId'] !== 'string' || !/^[0-9a-f]{64}$/u.test(raw['failedCheckpointId'])) {
    throw new TypeError('invalid failed checkpoint identity');
  }
  return { operationId: parseCheckpointOperationId(raw['operationId']), branchRunId, sourceRunId,
    failedCheckpointId: raw['failedCheckpointId'], recoveredDescriptor };
}

/** Validate a complete durable branch acknowledgement, preserving exact chronology. */
export function parseRecoveryBranchResult(value: unknown): RecoveryBranchResult {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new TypeError('invalid recovery result');
  const { abandonedThroughGeneration, ...commit } = value as Record<string, unknown>;
  const parsed = parseRecoveryBranchCommit(commit);
  if (typeof abandonedThroughGeneration !== 'string' || !/^[0-9a-f]{16}$/u.test(abandonedThroughGeneration) ||
      abandonedThroughGeneration < parsed.recoveredDescriptor.generation) {
    throw new TypeError('invalid abandoned recovery suffix');
  }
  return { ...parsed, abandonedThroughGeneration };
}

/** Stable descending scan position tied to one failed source pointer. */
export interface RecoveryScanCursor {
  /** Lineage selected at scan start. */
  sourceRunId: string;
  /** Failed pointer must still match before every next read. */
  failedCheckpointId: string;
  /** Last visited boundary, or null before the first candidate. */
  generation: U64Hex | null;
  /** Last visited immutable root, breaking any generation tie. */
  checkpointId: string | null;
}

/** One bounded metadata candidate; Rust still validates all immutable file content. */
export interface RecoveryScanResult {
  /** Position retained even when this candidate's metadata is corrupt. */
  cursor: RecoveryScanCursor;
  /** Strict descriptor when this row is eligible for native validation. */
  descriptor: ManagedCheckpointDescriptor | null;
  /** Bounded metadata rejection, never population data. */
  issue: string | null;
  /** No remaining candidates in this lineage. */
  exhausted: boolean;
}

/** Parse a scalar cursor without accepting arrays or unknown fields. */
export function parseRecoveryScanCursor(value: unknown): RecoveryScanCursor {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new TypeError('invalid recovery cursor');
  const raw = value as Record<string, unknown>;
  const keys = ['sourceRunId', 'failedCheckpointId', 'generation', 'checkpointId'];
  if (Object.keys(raw).length !== keys.length || keys.some(key => !Object.hasOwn(raw, key))) throw new TypeError('invalid recovery cursor fields');
  const sourceRunId = runId(raw['sourceRunId']);
  const failedCheckpointId = raw['failedCheckpointId'];
  const generation = raw['generation'];
  const checkpointId = raw['checkpointId'];
  if (typeof failedCheckpointId !== 'string' || !/^[0-9a-f]{64}$/u.test(failedCheckpointId) ||
      (generation !== null && (typeof generation !== 'string' || !/^[0-9a-f]{16}$/u.test(generation))) ||
      (checkpointId !== null && (typeof checkpointId !== 'string' || !/^[0-9a-f]{64}$/u.test(checkpointId))) ||
      (generation === null) !== (checkpointId === null)) throw new TypeError('invalid recovery cursor identity');
  return { sourceRunId, failedCheckpointId, generation: generation as U64Hex | null, checkpointId: checkpointId as string | null };
}

/** Parse one bounded scan response and bind its descriptor to the cursor. */
export function parseRecoveryScanResult(value: unknown): RecoveryScanResult {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new TypeError('invalid recovery scan result');
  const raw = value as Record<string, unknown>;
  const keys = ['cursor', 'descriptor', 'issue', 'exhausted'];
  if (Object.keys(raw).length !== keys.length || keys.some(key => !Object.hasOwn(raw, key))) throw new TypeError('invalid recovery scan fields');
  const cursor = parseRecoveryScanCursor(raw['cursor']);
  const descriptor = raw['descriptor'] === null ? null : parseManagedCheckpointDescriptor(raw['descriptor']);
  const issue = raw['issue'];
  if (typeof raw['exhausted'] !== 'boolean' || (issue !== null && (typeof issue !== 'string' || !issue || Buffer.byteLength(issue) > 1024)) ||
      (descriptor && (descriptor.generation !== cursor.generation || descriptor.logicalRootSha256 !== cursor.checkpointId)) ||
      (raw['exhausted'] && (descriptor !== null || issue !== null)) ||
      (!raw['exhausted'] && ((descriptor === null) === (issue === null) || cursor.generation === null))) {
    throw new TypeError('inconsistent recovery scan result');
  }
  return { cursor, descriptor, issue: issue as string | null, exhausted: raw['exhausted'] };
}
