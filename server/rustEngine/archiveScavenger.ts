import { lstat, opendir, realpath, unlink } from 'node:fs/promises';
import { join } from 'node:path';

/** Archive/checkpoint scratch files become startup-cleanup candidates after one day. */
export const ARCHIVE_ARTIFACT_GRACE_MS = 24 * 60 * 60 * 1000;
/** Exact private direct-child names produced by bounded checkpoint/archive work. */
const ARCHIVE_ARTIFACT_NAME = /^(?:checkpoint-v3-[0-9a-f]{32}\.partial|\.[0-9a-f]{32}\.(?:hof-weights\.partial|(?:weights|recurrent|hof-weights)\.codec\.partial|export-hof-weights\.partial|slither-save\.(?:partial|ready)|upload\.(?:partial|ready)))$/u;

/** Bounded result suitable for one startup log or health projection. */
export interface ArchiveScavengeResult {
  /** Recognized direct-child files old enough to inspect. */
  examined: number;
  /** Regular files successfully removed or concurrently absent. */
  removed: number;
  /** Bytes represented by removed regular files. */
  removedBytes: bigint;
}

/** Exact private filenames emitted by the current Rust archive/checkpoint writers. */
function isRecognizedArtifact(name: string): boolean {
  return ARCHIVE_ARTIFACT_NAME.test(name);
}

/**
 * Remove only stale, recognized, unreferenced scratch files from the managed root.
 *
 * Immutable checkpoints and content-addressed Hall-of-Fame objects cannot match
 * these names. Fresh files, directories, links and all unknown names are left alone.
 */
export async function scavengeStaleArchiveArtifacts(
  managedDirectory: string,
  nowMs: number = Date.now()
): Promise<ArchiveScavengeResult> {
  if (!Number.isFinite(nowMs) || nowMs < ARCHIVE_ARTIFACT_GRACE_MS) {
    throw new RangeError('archive scavenger clock must be a finite post-grace timestamp');
  }
  const directory = await realpath(managedDirectory);
  const cutoff = nowMs - ARCHIVE_ARTIFACT_GRACE_MS;
  const result: ArchiveScavengeResult = { examined: 0, removed: 0, removedBytes: 0n };
  const entries = await opendir(directory);
  for await (const entry of entries) {
    if (!entry.isFile() || !isRecognizedArtifact(entry.name)) continue;
    const path = join(directory, entry.name);
    let metadata: Awaited<ReturnType<typeof lstat>>;
    try {
      metadata = await lstat(path);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === 'ENOENT') continue;
      throw error;
    }
    if (!metadata.isFile() || metadata.isSymbolicLink() || metadata.mtimeMs > cutoff) continue;
    result.examined++;
    try {
      await unlink(path);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error;
    }
    result.removed++;
    result.removedBytes += BigInt(metadata.size);
  }
  return result;
}
