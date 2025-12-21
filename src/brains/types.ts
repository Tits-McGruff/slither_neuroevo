import type { VizData } from '../protocol/messages.ts';

export interface Brain {
  forward(input: Float32Array): Float32Array;
  reset(): void;
  paramLength(): number;
  getVizData(): VizData;
}
