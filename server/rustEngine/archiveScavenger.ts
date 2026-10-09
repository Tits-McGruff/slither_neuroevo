import { lstat, opendir, realpath, rmdir, unlink } from 'node:fs/promises';
import { join } from 'node:path';

/** Archive/checkpoint scratch files become startup-cleanup candidates after one day. */
export const ARCHIVE_ARTIFACT_GRACE_MS = 24 * 60 * 60 * 1000;
/** Exact private direct-child names produced by bounded checkpoint/archive work. */
const ARCHIVE_ARTIFACT_NAME = /^(?:checkpoint-v3-[0-9a-f]{32}\.partial|\.[0-9a-f]{32}\.(?:hof-weights\.partial|(?:weights|recurrent|hof-weights)\.codec\.partial|export-hof-weights\.partial|(?:export|import)-inventory-v1(?:\.partial)?|import-hof-weights\.(?:encoded|raw)\.partial|slither-save\.(?:partial|ready)|upload\.(?:partial|ready)))$/u;
/** Exact operation-owned directory used before an imported checkpoint is accepted. */
const IMPORT_VALIDATION_DIRECTORY = /^\.([0-9a-f]{32})\.import-validation$/u;
/** Immutable-looking objects inside private staging have no managed-root references. */
const STAGED_IMMUTABLE_NAME = /^[0-9a-f]{64}\.(?:checkpoint-v3|hof-weights-v1)$/u;

/** Bounded result suitable for one startup log or health projection. */
export interface ArchiveScavengeResult {
  /** Recognized root or private-stage files old enough to inspect. */
  examined: number;
  /** Regular files successfully removed or concurrently absent. */
  removed: number;
  /** Bytes represented by removed regular files. */
  removedBytes: bigint;
}

/** Exact private filenames emitted by the current Rust archive/checkpoint writers. */
export function isRecognizedArchiveArtifact(name: string): boolean {
  return ARCHIVE_ARTIFACT_NAME.test(name);
}

/** Remove a stale private stage only if every child is an old, known regular file. */
async function scavengeImportStage(
  path: string, operation: string, cutoff: number, result: ArchiveScavengeResult
): Promise<void> {
  const directory = await lstat(path);
  if (!directory.isDirectory() || directory.isSymbolicLink() || directory.mtimeMs > cutoff ||
      await realpath(path) !== path) return;
  /** Known scratch must also belong to this exact operation's private directory. */
  const recognized = (name: string): boolean => STAGED_IMMUTABLE_NAME.test(name) ||
    (name.startsWith(`.${operation}.`) && isRecognizedArchiveArtifact(name));
  for await (const entry of await opendir(path)) {
    if (!entry.isFile() || !recognized(entry.name)) return;
    const child = await lstat(join(path, entry.name));
    if (!child.isFile() || child.isSymbolicLink() || child.mtimeMs > cutoff) return;
  }
  for await (const entry of await opendir(path)) {
    const childPath = join(path, entry.name);
    const child = await lstat(childPath);
    if (!recognized(entry.name) || !child.isFile() || child.isSymbolicLink() || child.mtimeMs > cutoff) continue;
    result.examined++;
    await unlink(childPath);
    result.removed++;
    result.removedBytes += BigInt(child.size);
  }
  await rmdir(path);
}

/**
 * Remove only stale, recognized, unreferenced scratch files from the managed root.
 *
 * Managed-root immutable files are preserved. Exact stale private import stages
 * may be removed after checking every child; fresh stages, links, nested directories
 * and stages containing unknown names are preserved in full.
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
    const staged = IMPORT_VALIDATION_DIRECTORY.exec(entry.name);
    if (entry.isDirectory() && staged) {
      await scavengeImportStage(join(directory, entry.name), staged[1]!, cutoff, result);
      continue;
    }
    if (!entry.isFile() || !isRecognizedArchiveArtifact(entry.name)) continue;
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
