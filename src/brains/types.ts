import type { VizData } from '../protocol/messages.ts';

/** Brain runtime interface implemented by all controller types. */
export interface Brain {
  /** Run a forward pass and return the output buffer. */
  forward(input: Float32Array): Float32Array;
  /** Reset internal state for a new episode. */
  reset(): void;
  /** Return the number of parameters expected by this brain. */
  paramLength(): number;
  /** Return visualization data for UI rendering. */
  getVizData(): VizData;
}
