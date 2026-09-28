import type { RecoveryBranchResult, RecoveryScanCursor } from './recoveryProtocol.ts';
import { randomBytes, randomUUID } from 'node:crypto';
import { mkdir, open, unlink } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import type { RustStartupMetadata } from '../../src/protocol/rustBackground.ts';
import type { ExperimentalRunningAuthorityNativeHandle } from './backgroundRuntime.ts';
import { CheckpointPersistenceClient, type ManagedCheckpointCommitResult } from './checkpointPersistenceClient.ts';
import { EXPERIMENTAL_ENGINE_CONTRACT_VERSION, type ExperimentalEngineInit } from './experimentalNativeBridge.ts';
import { createExperimentalFreshRunSession, validateExperimentalFreshRunBinding, type ExperimentalFreshRunSession } from './experimentalFreshRunSession.ts';
import { computeNativeSourceIdentity } from './nativeSourceIdentity.ts';
import type {
  ManagedCheckpointSelection,
  ManagedCheckpointDescriptor,
  ManagedImportBranchResult,
  ManagedLegacyConversion,
  ManagedStorageDiagnostics
} from './checkpointPersistenceProtocol.ts';
import { scavengeStaleArchiveArtifacts } from './archiveScavenger.ts';
import {
  admitDiskOperation,
  CHECKPOINT_DISK_ADMISSION_REQUEST,
  SQLITE_WAL_ALLOWANCE_BYTES
} from './diskAdmission.ts';
import { OWNER_CHECKPOINT_RETENTION_DEFAULTS, type CheckpointRetentionInventory } from './checkpointRetention.ts';

/** Bounded background queues for the experimental Rust server. */
const BACKGROUND_INIT: ExperimentalEngineInit = {
  contractVersion: EXPERIMENTAL_ENGINE_CONTRACT_VERSION, maxInboundBatches: 64, maxInboundCommands: 64,
  maxInboundOwnedBytes: 4 * 1024 * 1024, maxBatchCommands: 1, maxBatchOwnedBytes: 1024 * 1024,
  maxOutputReliable: 32, maxOutputReliableOwnedBytes: 16 * 1024 * 1024,
  maxOutputDiscrete: 4, maxOutputDiscreteOwnedBytes: 1024 * 1024,
  maxOutputTotalOwnedBytes: 32 * 1024 * 1024, maxOutputEventOwnedBytes: 1024 * 1024,
  maxOutputFrameConnections: 4
};
/** Native source/loader directory, independent of the shell's working directory. */
const NATIVE_DIRECTORY = fileURLToPath(new URL('../../native/', import.meta.url));
/** CommonJS loader for the generated native addon. */
const require = createRequire(import.meta.url);

/** Explicit storage and identity inputs for experimental startup composition. */
export interface ExperimentalStartupOptions {
  /** Bounded persistent Rust calculation threads, independent of old Node MT. */
  calculationWorkers?: number;
  /** Owner-selected physical checkpoint budget in MiB. */
  checkpointBudgetMiB?: number;
  /** Dedicated managed-metadata database; fresh startup requires a new path. */
  databasePath: string;
  /** Internal exact-current restart prerequisite; automatic latest recovery is separate. */
  restoreCurrent?: boolean;
  /** Validate current first, then recover from the newest valid retained boundary. */
  restoreLatest?: boolean;
  /** Exact retained root; validation failure never substitutes another checkpoint. */
  restoreCheckpointId?: string;
  /** Controlled immutable checkpoint directory. */
  managedDirectory: string;
  /** Optional normalized Uint32 seed; omission uses OS entropy. */
  seed?: number;
  /** Coalesced notification; the caller attaches its router before calling start. */
  onWake(): void;
}

/** One durable authority ready to attach to HTTP/WebSocket routing. */
export interface ExperimentalServerRuntime {
  /** Sole Rust owner, intentionally unstarted until transport setup succeeds. */
  runtime: ExperimentalRunningAuthorityNativeHandle;
  /** Identifier reported by the source-validated production addon. */
  nativeBuildIdentifier: string;
  /** Small immutable facts captured from Rust before authority transfer. */
  metadata: RustStartupMetadata;
  /** Durable recovery provenance for health/welcome reporting. */
  recovery: RecoveryBranchResult | null;
  /** Durable older-checkpoint import provenance for restart and status surfaces. */
  importBranch: ManagedImportBranchResult | null;
  /** Durable notice that this run began from an old population-only checkpoint. */
  legacyConversion: ManagedLegacyConversion | null;
  /** Dedicated metadata worker reused for generation commits. */
  persistence: CheckpointPersistenceClient;
  /** Exact committed startup checkpoint, retained unchanged on restore. */
  runStart: ManagedCheckpointCommitResult;
  /** Controlled root for subsequent immutable generation files. */
  managedDirectory: string;
  /** Recheck free disk before each generation publication; retention never deletes files. */
  admitCheckpoint(): Promise<void>;
  /** Stop and join Rust before closing its persistence worker. */
  close(): Promise<void>;
}

/** Admit bounded publication against current free disk without deleting retained saves. */
async function admitCheckpoint(directory: string): Promise<void> {
  await admitDiskOperation(directory, CHECKPOINT_DISK_ADMISSION_REQUEST);
}

/** Reject a budget that cannot protect the minimum retained set through one publication. */
export function assertStartupCheckpointBudget(
  retention: Pick<CheckpointRetentionInventory, 'protectedAutomaticStoredByteCount' | 'automaticByteCap'>,
  storage: Pick<ManagedStorageDiagnostics, 'databaseByteCount' | 'walByteCount' | 'shmByteCount'>
): void {
  const protectedBytes = BigInt(`0x${retention.protectedAutomaticStoredByteCount}`);
  const sqliteBytes = BigInt(`0x${storage.databaseByteCount}`) +
    BigInt(`0x${storage.walByteCount}`) + BigInt(`0x${storage.shmByteCount}`);
  const publicationReserve = CHECKPOINT_DISK_ADMISSION_REQUEST.candidateSpoolBytes +
    CHECKPOINT_DISK_ADMISSION_REQUEST.finalManagedBytes + SQLITE_WAL_ALLOWANCE_BYTES;
  const required = protectedBytes + sqliteBytes + publicationReserve;
  const cap = BigInt(`0x${retention.automaticByteCap}`);
  if (required > cap) {
    throw new RangeError(`checkpoint budget ${cap} bytes cannot preserve the protected checkpoints and one publication; requires at least ${required} bytes`);
  }
}

/** Include the proposed replacement boundary before its SQLite current-pointer swap. */
export function assertReplacementCheckpointBudget(
  descriptor: Pick<ManagedCheckpointDescriptor, 'storedByteCount'>,
  retention: Pick<CheckpointRetentionInventory, 'protectedAutomaticStoredByteCount' | 'automaticByteCap'>,
  storage: Pick<ManagedStorageDiagnostics, 'databaseByteCount' | 'walByteCount' | 'shmByteCount'>
): void {
  const requiredAnchors = BigInt(`0x${retention.protectedAutomaticStoredByteCount}`) +
    BigInt(`0x${descriptor.storedByteCount}`);
  assertStartupCheckpointBudget({
    automaticByteCap: retention.automaticByteCap,
    protectedAutomaticStoredByteCount: requiredAnchors.toString(16).padStart(16, '0')
  }, storage);
}

/** Admit one already-published fresh boundary before SQLite commits its current pointer. */
export async function admitPendingRunStartCheckpoint(
  descriptor: Pick<ManagedCheckpointDescriptor, 'storedByteCount' | 'relativeFilename'>,
  inspectStorage: () => Promise<ManagedStorageDiagnostics>,
  managedDirectory: string,
  automaticCapBytes: bigint
): Promise<void> {
  if (!/^[0-9a-f]{64}\.checkpoint-v3$/u.test(descriptor.relativeFilename)) {
    throw new TypeError('pending run-start checkpoint filename must be digest-derived');
  }
  const storage = await inspectStorage();
  try {
    assertStartupCheckpointBudget({
      protectedAutomaticStoredByteCount: descriptor.storedByteCount,
      automaticByteCap: automaticCapBytes.toString(16).padStart(16, '0')
    }, storage);
  } catch (error) {
    if (error instanceof RangeError) await unlink(join(managedDirectory, descriptor.relativeFilename));
    throw error;
  }
}

/** Construct or restore a durable Rust boundary and transfer its sole running authority. */
export async function createExperimentalServerRuntime(options: ExperimentalStartupOptions): Promise<ExperimentalServerRuntime> {
  if ([options.restoreCurrent === true, options.restoreLatest === true, options.restoreCheckpointId !== undefined].filter(Boolean).length > 1) throw new Error('choose one checkpoint startup selector');
  if (options.restoreCheckpointId !== undefined && !/^[0-9a-f]{64}$/u.test(options.restoreCheckpointId)) throw new Error('invalid exact managed checkpoint ID');
  const restoring = options.restoreCurrent === true || options.restoreLatest === true || options.restoreCheckpointId !== undefined;
  if (restoring && options.seed !== undefined) throw new Error('checkpoint restore cannot override its retained seed');
  // The constructor seed is unused by native restore; welcome comes only from its metadata.
  const seed = restoring ? 0 : (options.seed ?? randomBytes(4).readUInt32LE());
  if (!Number.isInteger(seed) || seed < 0 || seed > 0xffff_ffff) throw new RangeError('experimental seed must be a Uint32');
  const calculationWorkers = options.calculationWorkers ?? 1;
  if (!Number.isInteger(calculationWorkers) || calculationWorkers < 1 || calculationWorkers > 7) {
    throw new RangeError('Rust calculation workers must be from 1 to 7');
  }
  const checkpointBudgetMiB = options.checkpointBudgetMiB ??
    Number(OWNER_CHECKPOINT_RETENTION_DEFAULTS.automaticByteCap / (1024n * 1024n));
  if (!Number.isSafeInteger(checkpointBudgetMiB) || checkpointBudgetMiB < 1_280 || checkpointBudgetMiB > 65_536) {
    throw new RangeError('checkpoint budget must be from 1280 to 65536 MiB');
  }
  const databasePath = resolve(options.databasePath);
  const managedDirectory = resolve(options.managedDirectory);
  const sourceIdentity = computeNativeSourceIdentity(NATIVE_DIRECTORY);
  const binding = validateExperimentalFreshRunBinding(require(resolve(NATIVE_DIRECTORY, 'index.js')) as unknown, sourceIdentity);
  const nativeBuildIdentifier = binding.nativeAddonBuildIdentifier();
  await mkdir(managedDirectory, { recursive: true });
  const scavenged = await scavengeStaleArchiveArtifacts(managedDirectory);
  if (scavenged.removed > 0) {
    console.warn(`[rust.startup] removed ${scavenged.removed} stale temporary file(s) (${scavenged.removedBytes} bytes)`);
  }
  if (!restoring) {
    await mkdir(dirname(databasePath), { recursive: true });
    await admitCheckpoint(managedDirectory);
    // Exclusive creation keeps existing reference/legacy databases out of fresh startup.
    const reservation = await open(databasePath, 'wx');
    await reservation.close();
  }
  const persistence = new CheckpointPersistenceClient({ databasePath,
    managedRootPath: managedDirectory, existingOnly: restoring,
    automaticByteCapBytes: BigInt(checkpointBudgetMiB) * 1024n * 1024n });
  let runtime: ExperimentalRunningAuthorityNativeHandle | undefined;
  try {
    /** Construct only a scalar native handle; initialization owns all population allocation. */
    const makeSession = (runId: string): ExperimentalFreshRunSession => createExperimentalFreshRunSession({
      binding, sourceIdentity, runId, seed, memoryCeilingBytes: 4n * 1024n * 1024n * 1024n,
      calculationWorkers,
      persistence, managedDirectory,
      beforeRunStartCommit: descriptor => admitPendingRunStartCheckpoint(descriptor,
        () => persistence.inspectStorage(), managedDirectory, BigInt(checkpointBudgetMiB) * 1024n * 1024n)
    });
    let selection: ManagedCheckpointSelection | null = null;
    let startupSelectionCompleted = false;
    let legacyConversion: ManagedLegacyConversion | null = null;
    let session: ExperimentalFreshRunSession | undefined;
    if (restoring) {
      try {
        selection = await persistence.selectStartup();
        startupSelectionCompleted = true;
        legacyConversion = selection.legacyConversion;
        if (!selection.descriptor || !selection.runId) {
          if (options.restoreCheckpointId) throw new Error('requested exact checkpoint is not current');
          const legacySnapshot = await persistence.selectLegacySnapshot();
          if (legacySnapshot === null) throw new Error('no current managed or compatible legacy checkpoint to restore');
          session = makeSession(randomUUID());
          await session.initializeFromLegacySqlite(databasePath, legacySnapshot.snapshotId);
          legacyConversion = { ...legacySnapshot, completeness: 'population-only' };
        } else {
          if (options.restoreCheckpointId && selection.descriptor.logicalRootSha256 !== options.restoreCheckpointId) throw new Error('requested exact checkpoint is not current');
          session = makeSession(selection.runId);
          await session.initializeFromCheckpoint(selection.descriptor,
            selection.descriptor.runId !== selection.runId
              ? selection.recovery ?? selection.importBranch ?? undefined : undefined);
        }
      } catch (currentError) {
        // A selected legacy-only database has no managed lineage to scan.
        // Preserve its conversion error instead of replacing it with a
        // misleading "no active lineage available for recovery" rejection.
        if (startupSelectionCompleted && !selection?.descriptor) throw currentError;
        if (!options.restoreLatest && !options.restoreCheckpointId) throw currentError;
        if (options.restoreCheckpointId && selection?.descriptor?.logicalRootSha256 === options.restoreCheckpointId) throw currentError;
        let compatibleRestored = false;
        if (options.restoreLatest && selection?.descriptor && selection.runId) {
          const descriptor = selection.descriptor;
          const restored = makeSession(selection.runId);
          let admitted = false;
          try {
            await restored.initializeFromCheckpoint(
              descriptor,
              descriptor.runId !== selection.runId
                ? selection.recovery ?? selection.importBranch ?? undefined : undefined,
              true
            );
            admitted = true;
          } catch {
            // Corrupt or version/target/math-incompatible current files continue
            // through ordinary newest-valid retained-boundary recovery below.
          }
          if (admitted) {
            const recovery = await persistence.commitRecoveryBranch({
              operationId: randomBytes(16).toString('hex'), branchRunId: randomUUID(),
              sourceRunId: selection.runId, failedCheckpointId: descriptor.logicalRootSha256,
              recoveredDescriptor: descriptor, compatibleBuild: true
            });
            await restored.adoptRecoveryBranch(recovery);
            session = restored;
            selection = { descriptor, runId: recovery.branchRunId, recovery, importBranch: null,
              legacyConversion: null };
            legacyConversion = null;
            compatibleRestored = true;
          }
        }
        if (!compatibleRestored) {
          const failedRoot = selection?.descriptor?.logicalRootSha256;
          let cursor: RecoveryScanCursor | null = null;
          for (;;) {
            const candidate = await persistence.scanRecoveryCandidate(cursor);
            cursor = candidate.cursor;
            if (candidate.exhausted) {
              throw new Error(options.restoreCheckpointId ? 'requested exact managed checkpoint is not valid in the retained active lineage' : 'no valid retained managed checkpoint; startup remains faulted', { cause: currentError });
            }
            if (options.restoreCheckpointId && candidate.cursor.checkpointId !== options.restoreCheckpointId) continue;
            const descriptor = candidate.descriptor;
            if (options.restoreCheckpointId && !descriptor) throw new Error('requested exact managed checkpoint has invalid metadata');
            if (!descriptor || (!options.restoreCheckpointId && descriptor.logicalRootSha256 === failedRoot)) continue;
            let restored = makeSession(descriptor.runId);
            let compatibleBuild = false;
            try { await restored.initializeFromCheckpoint(descriptor); }
            catch (error) {
              if (options.restoreCheckpointId) throw error;
              // A different application build may have produced an older valid
              // boundary too. The failed private session cannot be reused.
              restored = makeSession(descriptor.runId);
              try {
                await restored.initializeFromCheckpoint(descriptor, undefined, true);
                compatibleBuild = true;
              } catch {
                continue; // Neither validator admitted this candidate.
              }
            }
            const recovery = await persistence.commitRecoveryBranch({
              operationId: randomBytes(16).toString('hex'), branchRunId: randomUUID(),
              sourceRunId: cursor.sourceRunId, failedCheckpointId: cursor.failedCheckpointId,
              recoveredDescriptor: descriptor, ...(compatibleBuild ? { compatibleBuild: true } : {})
            });
            // Commit failures escape; an older candidate must never hide a durability failure.
            await restored.adoptRecoveryBranch(recovery);
            session = restored;
            selection = {
              descriptor, runId: recovery.branchRunId, recovery, importBranch: null,
              legacyConversion: null
            };
            legacyConversion = null;
            break;
          }
        }
      }
    } else {
      session = makeSession(randomUUID());
      await session.initialize();
    }
    if (!session) throw new Error('startup completed without an admitted Rust session');
    const selected = selection?.descriptor ?? null;
    const metadata = session.startupMetadata();
    if (selected && metadata.runId !== selection?.runId) throw new Error('restored startup identity differs from selected checkpoint');
    const runStart: ManagedCheckpointCommitResult = selected ? {
      operationId: selected.operationId, transitionEpoch: selected.transitionEpoch,
      runId: selected.runId, checkpointId: selected.logicalRootSha256, descriptor: selected
    } : await session.commitPendingRunStart(
      randomBytes(16).toString('hex'), legacyConversion
    );
    assertStartupCheckpointBudget(await persistence.inspectRetention(), await persistence.inspectStorage());
    await session.activateRunningAuthority();
    runtime = await session.createBackgroundRuntime(BACKGROUND_INIT, options.onWake);
    const owner = runtime;
    let closing: Promise<void> | undefined;
    return {
      runtime: owner, nativeBuildIdentifier, metadata, recovery: selection?.recovery ?? null,
      importBranch: selection?.importBranch ?? null, legacyConversion,
      persistence, runStart, managedDirectory,
      admitCheckpoint: async () => {
        await persistence.applyRetention(CHECKPOINT_DISK_ADMISSION_REQUEST.candidateSpoolBytes +
          CHECKPOINT_DISK_ADMISSION_REQUEST.finalManagedBytes + SQLITE_WAL_ALLOWANCE_BYTES);
        await admitCheckpoint(managedDirectory);
      },
      close(): Promise<void> {
        closing ??= (async () => {
          try { owner.requestStop(); await owner.join(); }
          finally { await persistence.close(); }
        })();
        return closing;
      }
    };
  } catch (error) {
    try { if (runtime) { runtime.requestStop(); await runtime.join(); } }
    finally { await persistence.close(); }
    throw error;
  }
}
