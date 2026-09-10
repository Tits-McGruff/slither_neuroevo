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
  if (branchRunId === sourceRunId || recoveredDescriptor.runId !== sourceRunId || recoveredDescriptor.generation === '0000000000000000') {
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
