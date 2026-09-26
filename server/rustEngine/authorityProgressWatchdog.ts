import type { RustBackgroundHealth } from '../../src/protocol/rustBackground.ts';
import type { ExperimentalRunningAuthorityNativeHandle } from './backgroundRuntime.ts';

/** A ready authority should service another scheduler boundary well within this limit. */
export const AUTHORITY_NO_PROGRESS_MS = 5_000;
/** Sampling is independent of the Node output-drain cadence. */
const AUTHORITY_POLL_MS = 500;

/** Detect a stuck Rust coordinator without confusing durable or external barriers for work. */
export class AuthorityProgressDeadline {
  private boundary: bigint | undefined;
  private worldEpoch: string | undefined;
  private progressAtMs = 0;
  private sampledAtMs = 0;

  /** Observe only small, lock-free native health fields. */
  public observe(health: Pick<RustBackgroundHealth,
    'lifecycle' | 'loopState' | 'worldEpoch' | 'commandServiceBoundaries'>,
  nowMs: number, noProgressMs = AUTHORITY_NO_PROGRESS_MS): Error | null {
    if (!Number.isFinite(nowMs) || !Number.isFinite(noProgressMs) || noProgressMs <= 0 ||
        !/^[0-9a-f]{16}$/u.test(health.commandServiceBoundaries)) {
      return new Error('Rust authority watchdog received invalid progress');
    }
    if (health.lifecycle !== 'running' || health.loopState !== 'ready') {
      this.boundary = undefined;
      this.worldEpoch = undefined;
      return null;
    }
    const boundary = BigInt(`0x${health.commandServiceBoundaries}`);
    if (this.boundary === undefined || health.worldEpoch !== this.worldEpoch ||
        boundary !== this.boundary || nowMs - this.sampledAtMs > noProgressMs) {
      // A long pause in Node's own event loop cannot establish a Rust-only stall.
      this.boundary = boundary;
      this.worldEpoch = health.worldEpoch;
      this.progressAtMs = nowMs;
      this.sampledAtMs = nowMs;
      return null;
    }
    this.sampledAtMs = nowMs;
    return nowMs - this.progressAtMs >= noProgressMs
      ? new Error(`Rust authority made no scheduler progress for ${noProgressMs} ms`)
      : null;
  }
}

/** Watch a running coordinator from the Node event loop until shutdown or failure. */
export function watchRunningAuthority(
  runtime: Pick<ExperimentalRunningAuthorityNativeHandle, 'health'>,
  onFault: (error: Error) => void
): () => void {
  const deadline = new AuthorityProgressDeadline();
  const timer = setInterval(() => {
    let failure: Error | null;
    try { failure = deadline.observe(runtime.health(), performance.now()); }
    catch (error) { failure = error instanceof Error ? error : new Error(String(error)); }
    if (failure) {
      clearInterval(timer);
      onFault(failure);
    }
  }, AUTHORITY_POLL_MS);
  timer.unref();
  return () => clearInterval(timer);
}
