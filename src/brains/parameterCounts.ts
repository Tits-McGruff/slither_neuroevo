/** Pure neural parameter-layout counts shared by browser graph tools and runtime brains. */

/**
 * Compute parameter count for an MLP with the given layer sizes.
 * @param layerSizes - Layer sizes including input and output.
 * @returns Total parameter count.
 */
export function mlpParamCount(layerSizes: number[]): number {
  let count = 0;
  for (let layer = 0; layer < layerSizes.length - 1; layer++) {
    const inputSize = layerSizes[layer]!;
    const outputSize = layerSizes[layer + 1]!;
    count += outputSize * inputSize + outputSize;
  }
  return count;
}

/**
 * Compute parameter count for a GRU layer.
 * @param inputSize - Input size.
 * @param hiddenSize - Hidden size.
 * @returns Total parameter count.
 */
export function gruParamCount(inputSize: number, hiddenSize: number): number {
  return 3 * hiddenSize * (inputSize + hiddenSize + 1);
}

/**
 * Compute parameter count for an LSTM layer.
 * @param inputSize - Input size.
 * @param hiddenSize - Hidden size.
 * @returns Total parameter count.
 */
export function lstmParamCount(inputSize: number, hiddenSize: number): number {
  return 4 * hiddenSize * (inputSize + hiddenSize + 1);
}

/**
 * Compute parameter count for an RRU layer.
 * @param inputSize - Input size.
 * @param hiddenSize - Hidden size.
 * @returns Total parameter count.
 */
export function rruParamCount(inputSize: number, hiddenSize: number): number {
  return 2 * hiddenSize * (inputSize + hiddenSize + 1);
}

/**
 * Compute parameter count for a dense head.
 * @param inputSize - Input size.
 * @param outputSize - Output size.
 * @returns Total parameter count.
 */
export function headParamCount(inputSize: number, outputSize: number): number {
  return outputSize * inputSize + outputSize;
}
