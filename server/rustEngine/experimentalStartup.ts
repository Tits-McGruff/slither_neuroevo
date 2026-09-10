import type { RecoveryBranchResult, RecoveryScanCursor } from './recoveryProtocol.ts';
import { randomBytes, randomUUID } from 'node:crypto';
import { mkdir, open, statfs } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import type { RustStartupMetadata } from '../../src/protocol/rustBackground.ts';
import type { ExperimentalRunningAuthorityNativeHandle } from './backgroundRuntime.ts';
import { CheckpointPersistenceClient, type ManagedCheckpointCommitResult } from './checkpointPersistenceClient.ts';
import type { ExperimentalEngineInit } from './experimentalNativeBridge.ts';
import { createExperimentalFreshRunSession, validateExperimentalFreshRunBinding, type ExperimentalFreshRunSession } from './experimentalFreshRunSession.ts';
import { computeNativeSourceIdentity } from './nativeSourceIdentity.ts';
import type { ManagedCheckpointSelection } from './checkpointPersistenceProtocol.ts';

/** Bounded production background queues for the first explicit P0 server. */
const BACKGROUND_INIT: ExperimentalEngineInit = {
  contractVersion: 1, maxInboundBatches: 64, maxInboundCommands: 64,
  maxInboundOwnedBytes: 4 * 1024 * 1024, maxBatchCommands: 1, maxBatchOwnedBytes: 1024 * 1024,
  maxOutputReliable: 32, maxOutputReliableOwnedBytes: 16 * 1024 * 1024,
  maxOutputDiscrete: 4, maxOutputDiscreteOwnedBytes: 1024 * 1024,
  maxOutputTotalOwnedBytes: 32 * 1024 * 1024, maxOutputEventOwnedBytes: 1024 * 1024,
  maxOutputFrameConnections: 4
};
/** Reserve enough disk for bounded publication before starting a fresh experiment. */
const MINIMUM_FREE_BYTES = 256n * 1024n * 1024n;
/** Native source/loader directory, independent of the shell's working directory. */
const NATIVE_DIRECTORY = fileURLToPath(new URL('../../native/', import.meta.url));
/** CommonJS loader for the generated native addon. */
const require = createRequire(import.meta.url);

/** Explicit storage and identity inputs for experimental startup composition. */
export interface ExperimentalStartupOptions {
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
  /** Small immutable facts captured from Rust before authority transfer. */
  metadata: RustStartupMetadata;
  /** Durable recovery provenance for health/welcome reporting. */
  recovery: RecoveryBranchResult | null;
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
  const space = await statfs(directory, { bigint: true });
  if (space.bavail * space.bsize < MINIMUM_FREE_BYTES) throw new Error('insufficient free disk for experimental checkpoints');
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
  const databasePath = resolve(options.databasePath);
  const managedDirectory = resolve(options.managedDirectory);
  const sourceIdentity = computeNativeSourceIdentity(NATIVE_DIRECTORY);
  const binding = validateExperimentalFreshRunBinding(require(resolve(NATIVE_DIRECTORY, 'index.js')) as unknown, sourceIdentity);
  if (!restoring) {
    await mkdir(dirname(databasePath), { recursive: true });
    await mkdir(managedDirectory, { recursive: true });
    await admitCheckpoint(managedDirectory);
    // Exclusive creation keeps existing reference/legacy databases out of fresh startup.
    const reservation = await open(databasePath, 'wx');
    await reservation.close();
  }
  const persistence = new CheckpointPersistenceClient({ databasePath,
    managedRootPath: managedDirectory, existingOnly: restoring });
  let runtime: ExperimentalRunningAuthorityNativeHandle | undefined;
  try {
    /** Construct only a scalar native handle; initialization owns all population allocation. */
    const makeSession = (runId: string): ExperimentalFreshRunSession => createExperimentalFreshRunSession({
      binding, sourceIdentity, runId, seed, memoryCeilingBytes: 4n * 1024n * 1024n * 1024n,
      persistence, managedDirectory
    });
    let selection: ManagedCheckpointSelection | null = null;
    let session: ExperimentalFreshRunSession;
    if (restoring) {
      try {
        selection = await persistence.selectStartup();
        if (!selection.descriptor || !selection.runId) throw new Error('no current managed checkpoint to restore');
        if (options.restoreCheckpointId && selection.descriptor.logicalRootSha256 !== options.restoreCheckpointId) throw new Error('requested exact checkpoint is not current');
        session = makeSession(selection.runId);
        await session.initializeFromCheckpoint(selection.descriptor,
          selection.descriptor.runId !== selection.runId ? selection.recovery ?? undefined : undefined);
      } catch (currentError) {
        if (!options.restoreLatest && !options.restoreCheckpointId) throw currentError;
        if (options.restoreCheckpointId && selection?.descriptor?.logicalRootSha256 === options.restoreCheckpointId) throw currentError;
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
          const restored = makeSession(descriptor.runId);
          try { await restored.initializeFromCheckpoint(descriptor); }
          catch (error) {
            if (options.restoreCheckpointId) throw error;
            continue; // Failed native admission retains no candidate population.
          }
          const recovery = await persistence.commitRecoveryBranch({
            operationId: randomBytes(16).toString('hex'), branchRunId: randomUUID(),
            sourceRunId: cursor.sourceRunId, failedCheckpointId: cursor.failedCheckpointId,
            recoveredDescriptor: descriptor
          });
          // Commit failures escape; an older candidate must never hide a durability failure.
          await restored.adoptRecoveryBranch(recovery);
          session = restored;
          selection = { descriptor, runId: recovery.branchRunId, recovery };
          break;
        }
      }
    } else {
      session = makeSession(randomUUID());
      await session.initialize();
    }
    const selected = selection?.descriptor ?? null;
    const metadata = session.startupMetadata();
    if (selected && metadata.runId !== selection?.runId) throw new Error('restored startup identity differs from selected checkpoint');
    const runStart: ManagedCheckpointCommitResult = selected ? {
      operationId: selected.operationId, transitionEpoch: selected.transitionEpoch,
      runId: selected.runId, checkpointId: selected.logicalRootSha256, descriptor: selected
    } : await session.commitPendingRunStart(randomBytes(16).toString('hex'));
    await session.activateRunningAuthority();
    runtime = await session.createBackgroundRuntime(BACKGROUND_INIT, options.onWake);
    const owner = runtime;
    let closing: Promise<void> | undefined;
    return {
      runtime: owner, metadata, recovery: selection?.recovery ?? null, persistence, runStart, managedDirectory,
      admitCheckpoint: () => admitCheckpoint(managedDirectory),
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
