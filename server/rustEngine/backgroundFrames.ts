import { Buffer } from 'node:buffer';
import type { RustBackgroundDisplay } from '../../src/protocol/rustBackground.ts';
import type { ExperimentalRunningAuthorityNativeHandle } from './backgroundRuntime.ts';
import type { U64Hex } from './checkpointPersistenceProtocol.ts';

/** One immutable send lease; release only after all sends using these bytes finish. */
export interface BackgroundFrameLease {
  /** Complete frame-v1 bytes; consumers must not write into this view. */
  readonly bytes: Buffer;
  /** Exact stats/chronology for these bytes, even if Rust has since advanced. */
  readonly display: RustBackgroundDisplay;
  /** Return storage after send completion or cancellation; safe to call twice. */
  release(): void;
}

/** A reusable destination whose ownership is held by at most one lease. */
interface FrameSlot {
  /** Fixed admitted allocation, never grown on the display path. */
  bytes: Buffer;
  /** True until the current lease's send callbacks finish. */
  leased: boolean;
}

/** Two bounded Node send buffers backed by Rust's single latest-frame cache. */
export class BackgroundFramePool {
  /** No unsent frame queue: acquisition always asks Rust for its latest frame. */
  private readonly slots: FrameSlot[];
  /** Coarse native copy operation; never receives a World or simulation callback. */
  private readonly runtime: Pick<ExperimentalRunningAuthorityNativeHandle, 'copyLatestFrame'>;

  /** Allocate the explicitly budgeted send storage before attaching clients. */
  constructor(runtime: Pick<ExperimentalRunningAuthorityNativeHandle, 'copyLatestFrame'>, maximumFrameBytes: number) {
    if (!Number.isSafeInteger(maximumFrameBytes) || maximumFrameBytes <= 0) {
      throw new RangeError('maximumFrameBytes must be a positive safe integer');
    }
    this.runtime = runtime;
    this.slots = Array.from({ length: 2 }, () => ({ bytes: Buffer.allocUnsafe(maximumFrameBytes), leased: false }));
  }

  /** Call after draining reliable output and checking socket backpressure. */
  tryAcquireLatest(afterSequence: U64Hex): BackgroundFrameLease | null {
    const slot = this.slots.find(candidate => !candidate.leased);
    if (!slot) return null;
    const copied = this.runtime.copyLatestFrame(slot.bytes, afterSequence);
    if (copied.status !== 'copied') return null;
    const length = copied.display.frameByteLength;
    if (!Number.isSafeInteger(length) || length <= 0 || length > slot.bytes.length) {
      throw new RangeError('native frame copy returned an invalid byte length');
    }
    slot.leased = true;
    let released = false;
    return {
      bytes: slot.bytes.subarray(0, length),
      display: copied.display,
      release(): void {
        if (!released) {
          released = true;
          slot.leased = false;
        }
      }
    };
  }
}
