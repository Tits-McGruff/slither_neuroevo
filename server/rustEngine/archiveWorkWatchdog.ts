import type { ExperimentalRunningAuthorityNativeHandle, RustArchiveWorkProgress } from './backgroundRuntime.ts';

/** Normal archive preparation no-progress deadline from the approved plan. */
export const ARCHIVE_WORK_NO_PROGRESS_MS = 60_000;
/** Cheap cross-thread sampling interval; this is not a total job deadline. */
const ARCHIVE_WORK_POLL_MS = 1_000;

/** Track one exact native archive job without treating a long active job as hung. */
export class ArchiveWorkDeadline {
  private lastBytes = 0n;
  private lastProgressAtMs: number;

  /** Construct a deadline for the operation just submitted to libuv. */
  public constructor(
    private readonly operationId: string,
    private readonly kind: RustArchiveWorkProgress['kind'],
    nowMs: number,
    private readonly noProgressMs = ARCHIVE_WORK_NO_PROGRESS_MS
  ) {
    if (!Number.isFinite(nowMs) || !Number.isFinite(noProgressMs) || noProgressMs <= 0) {
      throw new RangeError('archive watchdog timing must be finite and positive');
    }
    this.lastProgressAtMs = nowMs;
  }

  /** Return a fault only for mismatched, regressing, or idle native work. */
  public observe(status: RustArchiveWorkProgress | null, nowMs: number): Error | null {
    if (!status || status.operationId !== this.operationId || status.kind !== this.kind ||
        !/^[0-9a-f]{16}$/u.test(status.completedBytes) ||
        typeof status.started !== 'boolean' || typeof status.finished !== 'boolean' ||
        !Number.isFinite(nowMs)) {
      return new Error('native archive progress identity or counter is invalid');
    }
    const bytes = BigInt(`0x${status.completedBytes}`);
    if (bytes < this.lastBytes) return new Error('native archive progress moved backwards');
    if (status.finished) return null;
    if (bytes > this.lastBytes) {
      this.lastBytes = bytes;
      this.lastProgressAtMs = nowMs;
      return null;
    }
    if (nowMs - this.lastProgressAtMs >= this.noProgressMs) {
      return new Error(status.started
        ? `Rust archive ${this.kind} made no progress for ${this.noProgressMs} ms`
        : `Rust archive ${this.kind} did not start within ${this.noProgressMs} ms`);
    }
    return null;
  }
}

/** Poll a worker-owned counter until the caller stops this watch or it faults. */
export function watchArchiveWork(
  runtime: Pick<ExperimentalRunningAuthorityNativeHandle, 'archiveWorkProgress'>,
  operationId: string,
  kind: RustArchiveWorkProgress['kind'],
  onFault: (error: Error) => void
): () => void {
  const deadline = new ArchiveWorkDeadline(operationId, kind, performance.now());
  let stopped = false;
  const timer = setInterval(() => {
    if (stopped) return;
    let failure: Error | null;
    try { failure = deadline.observe(runtime.archiveWorkProgress(), performance.now()); }
    catch (error) { failure = error instanceof Error ? error : new Error(String(error)); }
    if (failure) {
      stop();
      onFault(failure);
    }
  }, ARCHIVE_WORK_POLL_MS);
  timer.unref();
  /** Stop polling after the promise resolves or the process begins stopping. */
  const stop = (): void => {
    if (stopped) return;
    stopped = true;
    clearInterval(timer);
  };
  return stop;
}
