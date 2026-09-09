import { randomBytes, randomUUID } from 'node:crypto';
import { mkdir, open, statfs } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import type { RustStartupMetadata } from '../../src/protocol/rustBackground.ts';
import type { ExperimentalRunningAuthorityNativeHandle } from './backgroundRuntime.ts';
import { CheckpointPersistenceClient, type ManagedCheckpointCommitResult } from './checkpointPersistenceClient.ts';
import type { ExperimentalEngineInit } from './experimentalNativeBridge.ts';
import { loadExperimentalFreshRunSession } from './experimentalFreshRunSession.ts';

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

/** Explicit fresh-only storage and identity inputs for experimental startup. */
export interface ExperimentalStartupOptions {
  /** A new dedicated database; an existing path is never opened or overwritten. */
  databasePath: string;
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
  /** Dedicated metadata worker reused for generation commits. */
  persistence: CheckpointPersistenceClient;
  /** Exact committed generation-one checkpoint. */
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

/** Construct, durably checkpoint, and transfer one explicit fresh Rust authority. */
export async function createExperimentalServerRuntime(options: ExperimentalStartupOptions): Promise<ExperimentalServerRuntime> {
  const seed = options.seed ?? randomBytes(4).readUInt32LE();
  if (!Number.isInteger(seed) || seed < 0 || seed > 0xffff_ffff) throw new RangeError('experimental seed must be a Uint32');
  const databasePath = resolve(options.databasePath);
  const managedDirectory = resolve(options.managedDirectory);
  await mkdir(dirname(databasePath), { recursive: true });
  await mkdir(managedDirectory, { recursive: true });
  await admitCheckpoint(managedDirectory);
  // Exclusive creation keeps the owner's reference/legacy databases out of this
  // fresh-only route. Failed startup leaves its new files available for diagnosis.
  const reservation = await open(databasePath, 'wx');
  await reservation.close();
  const persistence = new CheckpointPersistenceClient({ databasePath, managedRootPath: managedDirectory });
  let runtime: ExperimentalRunningAuthorityNativeHandle | undefined;
  try {
    const session = await loadExperimentalFreshRunSession({
      nativeManifestDirectory: NATIVE_DIRECTORY, loadBinding: () => require(resolve(NATIVE_DIRECTORY, 'index.js')) as unknown,
      runId: randomUUID(), seed, memoryCeilingBytes: 4n * 1024n * 1024n * 1024n,
      persistence, managedDirectory
    });
    await session.initialize();
    const metadata = session.startupMetadata();
    const runStart = await session.commitPendingRunStart(randomBytes(16).toString('hex'));
    await session.activateRunningAuthority();
    runtime = await session.createBackgroundRuntime(BACKGROUND_INIT, options.onWake);
    const owner = runtime;
    let closing: Promise<void> | undefined;
    return {
      runtime: owner, metadata, persistence, runStart, managedDirectory,
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
