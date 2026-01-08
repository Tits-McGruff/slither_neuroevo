import { CFG } from '../config.ts';
import type { VizData } from '../protocol/messages.ts';
import type { Brain } from './types.ts';

/** Brain implementation that returns zero outputs and no params. */
export class NullBrain implements Brain {
  /** Cached zero-output buffer. */
  private output: Float32Array;

  /**
   * Create a NullBrain with a cached output buffer sized to CFG.brain.outSize.
   */
  constructor() {
    this.output = new Float32Array(CFG.brain.outSize);
  }

  /**
   * No-op reset for stateless brain.
   */
  reset(): void {
    // No-op: NullBrain has no internal state.
  }

  /**
   * Return a stable zero buffer for the outputs.
   * @param _input - Input buffer (unused).
   * @returns Cached zero buffer.
   */
  forward(_input: Float32Array): Float32Array {
    return this.output;
  }

  /**
   * Return the number of parameters for this brain.
   * @returns Zero for NullBrain.
   */
  paramLength(): number {
    return 0;
  }

  /**
   * Return visualization data for UI rendering.
   * @returns Empty visualization payload.
   */
  getVizData(): VizData {
    return { kind: 'null', layers: [] };
  }
}
