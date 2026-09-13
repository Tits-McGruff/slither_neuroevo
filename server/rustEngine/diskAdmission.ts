import { lstat, opendir, realpath, statfs } from 'node:fs/promises';
import { join } from 'node:path';
import { isRecognizedArchiveArtifact } from './archiveScavenger.ts';

/** Current Stage 6 allowance for one small metadata/WAL transaction. */
export const SQLITE_WAL_ALLOWANCE_BYTES = 64n * 1024n * 1024n;
/** Space left unused for the operating system and filesystem operations. */
export const OPERATING_DISK_RESERVE_BYTES = 1024n * 1024n * 1024n;
/** Aggregate private transfer/work-file quota selected by the approved plan. */
export const ARCHIVE_TEMP_QUOTA_BYTES = 9n * 1024n * 1024n * 1024n;
/** Rust checkpoint-v3 limit plus one winner-object publication allowance. */
export const CHECKPOINT_PUBLICATION_BYTES = 528n * 1024n * 1024n;
/** Maximum decoded/checkpoint/inventory work retained while an import is staged. */
export const IMPORT_CANDIDATE_BYTES = 5n * 1024n * 1024n * 1024n;
/** Maximum checkpoint plus content objects published by one admitted archive. */
export const IMPORT_FINAL_MANAGED_BYTES = 5n * 1024n * 1024n * 1024n;

/** Exact additional files one operation may need before old data can be pruned. */
export interface DiskAdmissionRequest {
  /** Plain operation name included in rejection diagnostics. */
  operation: 'checkpoint' | 'export' | 'import' | 'pin';
  /** New upload/source bytes retained in the private temporary subtree. */
  sourceSpoolBytes: bigint;
  /** New candidate/output work bytes retained in the private temporary subtree. */
  candidateSpoolBytes: bigint;
  /** New immutable managed bytes that must commit before pruning may run. */
  finalManagedBytes: bigint;
}

/** Complete scalar terms used to decide one disk admission. */
export interface DiskAdmissionDecision extends DiskAdmissionRequest {
  /** Recognized temporary bytes already present and unavailable for reuse. */
  existingTempBytes: bigint;
  /** Current filesystem blocks available to the process. */
  freeBytes: bigint;
  /** Existing plus newly planned private temporary bytes. */
  totalTempBytes: bigint;
  /** Minimum free bytes required before the operation may begin. */
  requiredFreeBytes: bigint;
}

/** Current managed-directory space counters used by health and admission. */
export interface ManagedDiskDiagnostics {
  /** Recognized private work files currently charged to the temporary quota. */
  tempByteCount: bigint;
  /** Current filesystem blocks available to this process. */
  freeByteCount: bigint;
  /** Configured aggregate private-work-file quota. */
  tempQuotaByteCount: bigint;
  /** Space deliberately withheld from managed operations. */
  operatingReserveByteCount: bigint;
}

/** Structured rejection retained across HTTP and background-generation callers. */
export class DiskAdmissionError extends Error {
  /** Stable reason for tests and future health projection. */
  public readonly code: 'TEMP_QUOTA' | 'FREE_DISK';
  /** Exact rejected calculation. */
  public readonly decision: DiskAdmissionDecision;

  /** Construct one bounded, plain-language disk rejection. */
  public constructor(
    code: DiskAdmissionError['code'],
    decision: DiskAdmissionDecision
  ) {
    const detail = `free=${decision.freeBytes}, existingTemp=${decision.existingTempBytes}, ` +
      `sourceSpool=${decision.sourceSpoolBytes}, candidateSpool=${decision.candidateSpoolBytes}, ` +
      `finalManaged=${decision.finalManagedBytes}, sqliteWal=${SQLITE_WAL_ALLOWANCE_BYTES}, ` +
      `operatingReserve=${OPERATING_DISK_RESERVE_BYTES}`;
    super(code === 'TEMP_QUOTA'
      ? `${decision.operation} needs ${decision.totalTempBytes} temporary bytes, above the ${ARCHIVE_TEMP_QUOTA_BYTES}-byte quota (${detail})`
      : `${decision.operation} needs ${decision.requiredFreeBytes} free bytes but only ${decision.freeBytes} are available (${detail})`);
    this.name = 'DiskAdmissionError';
    this.code = code;
    this.decision = decision;
  }
}

/** Reject negative or unrepresentable caller arithmetic before inspecting disk. */
function validateRequest(request: DiskAdmissionRequest): void {
  for (const [label, value] of [
    ['source spool', request.sourceSpoolBytes],
    ['candidate spool', request.candidateSpoolBytes],
    ['final managed', request.finalManagedBytes]
  ] as const) {
    if (value < 0n || value > 0xffff_ffff_ffff_ffffn) {
      throw new RangeError(`${request.operation} ${label} bytes must fit an unsigned 64-bit value`);
    }
  }
}

/** Build and enforce the approved arithmetic from already measured scalar terms. */
export function evaluateDiskAdmission(
  request: DiskAdmissionRequest,
  existingTempBytes: bigint,
  freeBytes: bigint
): DiskAdmissionDecision {
  validateRequest(request);
  if (existingTempBytes < 0n || freeBytes < 0n) {
    throw new RangeError('filesystem byte counts cannot be negative');
  }
  const totalTempBytes = existingTempBytes + request.sourceSpoolBytes +
    request.candidateSpoolBytes;
  const requiredFreeBytes = totalTempBytes + request.finalManagedBytes +
    SQLITE_WAL_ALLOWANCE_BYTES + OPERATING_DISK_RESERVE_BYTES;
  const decision = { ...request, existingTempBytes, freeBytes, totalTempBytes,
    requiredFreeBytes };
  if (totalTempBytes > ARCHIVE_TEMP_QUOTA_BYTES) {
    throw new DiskAdmissionError('TEMP_QUOTA', decision);
  }
  if (freeBytes < requiredFreeBytes) {
    throw new DiskAdmissionError('FREE_DISK', decision);
  }
  return decision;
}

/** Count only regular, recognized private work files in the controlled root. */
export async function inspectArchiveTempBytes(managedDirectory: string): Promise<bigint> {
  const directory = await realpath(managedDirectory);
  let bytes = 0n;
  const entries = await opendir(directory);
  for await (const entry of entries) {
    if (!entry.isFile() || !isRecognizedArchiveArtifact(entry.name)) continue;
    let metadata: Awaited<ReturnType<typeof lstat>>;
    try {
      metadata = await lstat(join(directory, entry.name));
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === 'ENOENT') continue;
      throw error;
    }
    if (metadata.isSymbolicLink() || !metadata.isFile()) continue;
    bytes += BigInt(metadata.size);
  }
  return bytes;
}

/** Inspect current managed temporary use and available filesystem space. */
export async function inspectManagedDisk(managedDirectory: string): Promise<ManagedDiskDiagnostics> {
  const [tempByteCount, space] = await Promise.all([
    inspectArchiveTempBytes(managedDirectory),
    statfs(managedDirectory, { bigint: true })
  ]);
  return {
    tempByteCount,
    freeByteCount: space.bavail * space.bsize,
    tempQuotaByteCount: ARCHIVE_TEMP_QUOTA_BYTES,
    operatingReserveByteCount: OPERATING_DISK_RESERVE_BYTES
  };
}

/** Inspect current filesystem state and enforce one operation's full formula. */
export async function admitDiskOperation(
  managedDirectory: string,
  request: DiskAdmissionRequest
): Promise<DiskAdmissionDecision> {
  const diagnostics = await inspectManagedDisk(managedDirectory);
  return evaluateDiskAdmission(
    request,
    diagnostics.tempByteCount,
    diagnostics.freeByteCount
  );
}
