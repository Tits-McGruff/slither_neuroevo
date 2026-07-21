import type { VizData } from '../protocol/messages.ts';

/** Math backend bound to a constructed brain runtime. */
export type InferenceBackend = 'js' | 'native';

/** Brain runtime interface implemented by all controller types. */
export interface Brain {
  /** Math backend captured when this brain was constructed, when observable. */
  readonly inferenceBackend?: InferenceBackend;
  /** Run a forward pass and return the output buffer. */
  forward(input: Float32Array): Float32Array;
  /**
   * Run a batched forward pass into the provided output buffer.
   * @param inputs - Packed input buffer.
   * @param outputs - Packed output buffer.
   * @param count - Number of batch entries to process.
   * @param inputStride - Stride between batch inputs.
   * @param outputStride - Stride between batch outputs.
   */
  forwardBatch?(
    inputs: Float32Array,
    outputs: Float32Array,
    count: number,
    inputStride: number,
    outputStride: number
  ): void;
  /** Reset internal state for a new episode. */
  reset(): void;
  /** Return the number of parameters expected by this brain. */
  paramLength(): number;
  /** Return visualization data for UI rendering. */
  getVizData(): VizData;
}
