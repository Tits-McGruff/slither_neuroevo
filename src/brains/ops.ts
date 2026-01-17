/** Low-level neural network primitives and parameter layouts used by brains. */

import { clamp } from '../utils.ts';
import { requireGruKernel, requireLstmKernel, requireRruKernel } from './wasmBridge.ts';

/**
 * Sigmoid activation function.
 * @param x - Input value.
 * @returns Sigmoid output in [0,1].
 */
export function sigmoid(x: number): number {
  return 1 / (1 + Math.exp(-x));
}

/**
 * Compute parameter count for an MLP with the given layer sizes.
 * @param layerSizes - Layer sizes including input and output.
 * @returns Total parameter count.
 */
export function mlpParamCount(layerSizes: number[]): number {
  let n = 0;
  for (let l = 0; l < layerSizes.length - 1; l++) {
    const ins = layerSizes[l]!;
    const outs = layerSizes[l + 1]!;
    n += outs * ins + outs;
  }
  return n;
}

/**
 * Compute parameter count for a GRU layer.
 * @param inSize - Input size.
 * @param hiddenSize - Hidden size.
 * @returns Total parameter count.
 */
export function gruParamCount(inSize: number, hiddenSize: number): number {
  return 3 * hiddenSize * (inSize + hiddenSize + 1);
}

/**
 * Compute parameter count for an LSTM layer.
 * @param inSize - Input size.
 * @param hiddenSize - Hidden size.
 * @returns Total parameter count.
 */
export function lstmParamCount(inSize: number, hiddenSize: number): number {
  return 4 * hiddenSize * (inSize + hiddenSize + 1);
}

/**
 * Compute parameter count for an RRU layer.
 * @param inSize - Input size.
 * @param hiddenSize - Hidden size.
 * @returns Total parameter count.
 */
export function rruParamCount(inSize: number, hiddenSize: number): number {
  return 2 * hiddenSize * (inSize + hiddenSize + 1);
}

/**
 * Compute parameter count for a dense head.
 * @param hiddenSize - Input size.
 * @param outSize - Output size.
 * @returns Total parameter count.
 */
export function headParamCount(hiddenSize: number, outSize: number): number {
  return outSize * hiddenSize + outSize;
}

/** Simple feed-forward MLP with tanh activations. */
export class MLP {
  /** Layer sizes including input and output. */
  layerSizes: number[];
  /** Architecture key derived from layer sizes. */
  key: string;
  /** Total parameter count. */
  paramCount: number;
  /** Packed weight and bias buffer. */
  w: Float32Array;
  /** Scratch buffers for hidden layer activations. */
  _bufs: Float32Array[];

  /**
   * Create an MLP instance with optional weights.
   * @param layerSizes - Layer sizes including input and output.
   * @param weights - Optional weight buffer.
   */
  constructor(layerSizes: number[], weights: Float32Array | null = null) {
    this.layerSizes = layerSizes.slice();
    this.key = this.layerSizes.join("x");
    this.paramCount = mlpParamCount(this.layerSizes);
    this.w = weights ? weights.slice() : new Float32Array(this.paramCount);
    if (!weights) {
      for (let i = 0; i < this.paramCount; i++) {
        this.w[i] = (Math.random() * 2 - 1) * 0.6;
      }
    }
    this._bufs = [];
    for (let l = 1; l < this.layerSizes.length; l++) {
      const size = this.layerSizes[l]!;
      this._bufs.push(new Float32Array(size));
    }
  }

  /**
   * Run a forward pass through the network.
   * @param input - Input activations.
   * @returns Output activations.
   */
  forward(input: Float32Array): Float32Array {
    let wi = 0;
    let cur = input;
    for (let l = 0; l < this.layerSizes.length - 1; l++) {
      const ins = this.layerSizes[l]!;
      const outs = this.layerSizes[l + 1]!;
      const next = this._bufs[l]!;
      for (let o = 0; o < outs; o++) {
        let sum = 0;
        for (let i = 0; i < ins; i++) sum += (this.w[wi++] ?? 0) * (cur[i] ?? 0);
        sum += this.w[wi++] ?? 0;
        next[o] = Math.tanh(sum);
      }
      cur = next;
    }
    return cur;
  }

  /**
   * Run a batched forward pass through the network.
   * @param inputs - Packed input buffer.
   * @param outputs - Packed output buffer.
   * @param count - Number of batch entries.
   * @param inputStride - Stride between batch inputs.
   * @param outputStride - Stride between batch outputs.
   */
  forwardBatch(
    inputs: Float32Array,
    outputs: Float32Array,
    count: number,
    inputStride: number,
    outputStride: number
  ): void {
    const layerCount = this.layerSizes.length;
    if (layerCount < 2) return;
    const outSize = this.layerSizes[layerCount - 1] ?? 0;
    const outLimit = Math.min(outSize, outputStride);
    for (let b = 0; b < count; b++) {
      let wi = 0;
      let cur: Float32Array | null = null;
      const inputBase = b * inputStride;
      for (let l = 0; l < layerCount - 1; l++) {
        const ins = this.layerSizes[l]!;
        const outs = this.layerSizes[l + 1]!;
        const next = this._bufs[l]!;
        for (let o = 0; o < outs; o++) {
          let sum = 0;
          if (l === 0) {
            for (let i = 0; i < ins; i++) {
              sum += (this.w[wi++] ?? 0) * (inputs[inputBase + i] ?? 0);
            }
          } else {
            const curBuf = cur!;
            for (let i = 0; i < ins; i++) {
              sum += (this.w[wi++] ?? 0) * (curBuf[i] ?? 0);
            }
          }
          sum += this.w[wi++] ?? 0;
          next[o] = Math.tanh(sum);
        }
        cur = next;
      }
      if (!cur) continue;
      const outBase = b * outputStride;
      for (let i = 0; i < outLimit; i++) outputs[outBase + i] = cur[i] ?? 0;
      for (let i = outLimit; i < outputStride; i++) outputs[outBase + i] = 0;
    }
  }
}

/** Gated recurrent unit implementation with configurable bias init. */
export class GRU {
  /** Input vector size. */
  inSize: number;
  /** Hidden state size. */
  hiddenSize: number;
  /** Total parameter count. */
  paramCount: number;
  /** Packed weight and bias buffer. */
  w: Float32Array;
  /** Current hidden state. */
  h: Float32Array;
  /** Scratch buffer for update gate. */
  _z: Float32Array;
  /** Scratch buffer for reset gate. */
  _r: Float32Array;
  /** Scratch buffer for the previous hidden state. */
  _hPrev: Float32Array;

  /**
   * Create a GRU instance with optional weights.
   * @param inSize - Input size.
   * @param hiddenSize - Hidden state size.
   * @param weights - Optional weight buffer.
   * @param initUpdateBias - Initial update gate bias.
   */
  constructor(
    inSize: number,
    hiddenSize: number,
    weights: Float32Array | null = null,
    initUpdateBias = -0.7
  ) {
    this.inSize = inSize;
    this.hiddenSize = hiddenSize;
    this.paramCount = gruParamCount(inSize, hiddenSize);
    this.w = weights ? weights.slice() : new Float32Array(this.paramCount);
    if (!weights) {
      const I = inSize;
      const H = hiddenSize;
      const Wsz = H * I;
      const Usz = H * H;
      let idx = 0;
      for (let i = 0; i < 3 * Wsz; i++) this.w[idx++] = (Math.random() * 2 - 1) * 0.35;
      for (let i = 0; i < 3 * Usz; i++) this.w[idx++] = (Math.random() * 2 - 1) * 0.18;
      for (let j = 0; j < H; j++) this.w[idx++] = initUpdateBias + (Math.random() * 2 - 1) * 0.10;
      for (let j = 0; j < H; j++) this.w[idx++] = (Math.random() * 2 - 1) * 0.10;
      for (let j = 0; j < H; j++) this.w[idx++] = (Math.random() * 2 - 1) * 0.10;
    }
    this.h = new Float32Array(hiddenSize);
    this._z = new Float32Array(hiddenSize);
    this._r = new Float32Array(hiddenSize);
    this._hPrev = new Float32Array(hiddenSize);
  }

  /** Reset the hidden state to zero. */
  reset(): void {
    this.h.fill(0);
  }

  /**
   * Advance the GRU by one timestep.
   * @param x - Input vector.
   * @returns Updated hidden state.
   */
  step(x: Float32Array): Float32Array {
    const kernel = requireGruKernel();
    kernel.stepBatch(
      this.w,
      x,
      this.h,
      this._z,
      this._r,
      this._hPrev,
      this.inSize,
      this.hiddenSize,
      1,
      this.inSize
    );
    return this.h;
  }

  /**
   * Advance the GRU by one timestep using the reference JS path.
   * @param x - Input vector.
   * @returns Updated hidden state.
   */
  stepReference(x: Float32Array): Float32Array {
    const I = this.inSize;
    const H = this.hiddenSize;
    const Wsz = H * I;
    const Usz = H * H;
    const Wz = 0;
    const Wr = Wz + Wsz;
    const Wh = Wr + Wsz;
    const Uz = Wh + Wsz;
    const Ur = Uz + Usz;
    const Uh = Ur + Usz;
    const bz = Uh + Usz;
    const br = bz + H;
    const bh = br + H;

    this._hPrev.set(this.h);
    const hPrev = this._hPrev;

    for (let j = 0; j < H; j++) {
      let sumZ = 0;
      let sumR = 0;
      const wzRow = Wz + j * I;
      const wrRow = Wr + j * I;
      for (let i = 0; i < I; i++) {
        const xi = x[i] ?? 0;
        sumZ += (this.w[wzRow + i] ?? 0) * xi;
        sumR += (this.w[wrRow + i] ?? 0) * xi;
      }
      const uzRow = Uz + j * H;
      const urRow = Ur + j * H;
      for (let k = 0; k < H; k++) {
        const hk = hPrev[k] ?? 0;
        sumZ += (this.w[uzRow + k] ?? 0) * hk;
        sumR += (this.w[urRow + k] ?? 0) * hk;
      }
      sumZ += this.w[bz + j] ?? 0;
      sumR += this.w[br + j] ?? 0;
      this._z[j] = sigmoid(sumZ);
      this._r[j] = sigmoid(sumR);
    }

    for (let j = 0; j < H; j++) {
      let sumH = 0;
      const whRow = Wh + j * I;
      for (let i = 0; i < I; i++) sumH += (this.w[whRow + i] ?? 0) * (x[i] ?? 0);
      const uhRow = Uh + j * H;
      for (let k = 0; k < H; k++) {
        const rVal = this._r[k] ?? 0;
        const hVal = hPrev[k] ?? 0;
        sumH += (this.w[uhRow + k] ?? 0) * (rVal * hVal);
      }
      sumH += this.w[bh + j] ?? 0;
      const hTilde = Math.tanh(sumH);
      const z = this._z[j] ?? 0;
      const prevH = hPrev[j] ?? 0;
      this.h[j] = (1 - z) * prevH + z * hTilde;
    }
    return this.h;
  }
}

/** Long short-term memory implementation. */
export class LSTM {
  /** Input vector size. */
  inSize: number;
  /** Hidden state size. */
  hiddenSize: number;
  /** Total parameter count. */
  paramCount: number;
  /** Packed weight and bias buffer. */
  w: Float32Array;
  /** Current hidden state. */
  h: Float32Array;
  /** Current cell state. */
  c: Float32Array;
  /** Scratch buffer for the previous hidden state. */
  _hPrev: Float32Array;
  /** Scratch buffer for the previous cell state. */
  _cPrev: Float32Array;

  /**
   * Create an LSTM instance with optional weights.
   * @param inSize - Input size.
   * @param hiddenSize - Hidden state size.
   * @param weights - Optional weight buffer.
   * @param initForgetBias - Initial forget gate bias.
   */
  constructor(inSize: number, hiddenSize: number, weights: Float32Array | null = null, initForgetBias = 0.6) {
    this.inSize = inSize;
    this.hiddenSize = hiddenSize;
    this.paramCount = lstmParamCount(inSize, hiddenSize);
    this.w = weights ? weights.slice() : new Float32Array(this.paramCount);
    if (!weights) {
      const I = inSize;
      const H = hiddenSize;
      const Wsz = H * I;
      const Usz = H * H;
      let idx = 0;
      for (let i = 0; i < 4 * Wsz; i++) this.w[idx++] = (Math.random() * 2 - 1) * 0.35;
      for (let i = 0; i < 4 * Usz; i++) this.w[idx++] = (Math.random() * 2 - 1) * 0.18;
      for (let j = 0; j < H; j++) this.w[idx++] = (Math.random() * 2 - 1) * 0.10; // bi
      for (let j = 0; j < H; j++) this.w[idx++] = initForgetBias + (Math.random() * 2 - 1) * 0.10; // bf
      for (let j = 0; j < H; j++) this.w[idx++] = (Math.random() * 2 - 1) * 0.10; // bo
      for (let j = 0; j < H; j++) this.w[idx++] = (Math.random() * 2 - 1) * 0.10; // bg
    }
    this.h = new Float32Array(hiddenSize);
    this.c = new Float32Array(hiddenSize);
    this._hPrev = new Float32Array(hiddenSize);
    this._cPrev = new Float32Array(hiddenSize);
  }

  /** Reset the hidden and cell state to zero. */
  reset(): void {
    this.h.fill(0);
    this.c.fill(0);
  }

  /**
   * Advance the LSTM by one timestep.
   * @param x - Input vector.
   * @returns Updated hidden state.
   */
  step(x: Float32Array): Float32Array {
    const kernel = requireLstmKernel();
    kernel.stepBatch(
      this.w,
      x,
      this.h,
      this.c,
      this._hPrev,
      this._cPrev,
      this.inSize,
      this.hiddenSize,
      1,
      this.inSize
    );
    return this.h;
  }

  /**
   * Advance the LSTM by one timestep using the reference JS path.
   * @param x - Input vector.
   * @returns Updated hidden state.
   */
  stepReference(x: Float32Array): Float32Array {
    const I = this.inSize;
    const H = this.hiddenSize;
    const Wsz = H * I;
    const Usz = H * H;
    const Wi = 0;
    const Wf = Wi + Wsz;
    const Wo = Wf + Wsz;
    const Wg = Wo + Wsz;
    const Ui = Wg + Wsz;
    const Uf = Ui + Usz;
    const Uo = Uf + Usz;
    const Ug = Uo + Usz;
    const bi = Ug + Usz;
    const bf = bi + H;
    const bo = bf + H;
    const bg = bo + H;

    this._hPrev.set(this.h);
    this._cPrev.set(this.c);
    const hPrev = this._hPrev;
    const cPrev = this._cPrev;

    for (let j = 0; j < H; j++) {
      let sumI = 0;
      let sumF = 0;
      let sumO = 0;
      let sumG = 0;
      const wiRow = Wi + j * I;
      const wfRow = Wf + j * I;
      const woRow = Wo + j * I;
      const wgRow = Wg + j * I;
      for (let i = 0; i < I; i++) {
        const xi = x[i] ?? 0;
        sumI += (this.w[wiRow + i] ?? 0) * xi;
        sumF += (this.w[wfRow + i] ?? 0) * xi;
        sumO += (this.w[woRow + i] ?? 0) * xi;
        sumG += (this.w[wgRow + i] ?? 0) * xi;
      }
      const uiRow = Ui + j * H;
      const ufRow = Uf + j * H;
      const uoRow = Uo + j * H;
      const ugRow = Ug + j * H;
      for (let k = 0; k < H; k++) {
        const hk = hPrev[k] ?? 0;
        sumI += (this.w[uiRow + k] ?? 0) * hk;
        sumF += (this.w[ufRow + k] ?? 0) * hk;
        sumO += (this.w[uoRow + k] ?? 0) * hk;
        sumG += (this.w[ugRow + k] ?? 0) * hk;
      }
      sumI += this.w[bi + j] ?? 0;
      sumF += this.w[bf + j] ?? 0;
      sumO += this.w[bo + j] ?? 0;
      sumG += this.w[bg + j] ?? 0;
      const iGate = sigmoid(sumI);
      const fGate = sigmoid(sumF);
      const oGate = sigmoid(sumO);
      const gGate = Math.tanh(sumG);
      const nextC = fGate * (cPrev[j] ?? 0) + iGate * gGate;
      this.c[j] = nextC;
      this.h[j] = oGate * Math.tanh(nextC);
    }
    return this.h;
  }
}

/** Minimal recurrent unit with reset gate. */
export class RRU {
  /** Input vector size. */
  inSize: number;
  /** Hidden state size. */
  hiddenSize: number;
  /** Total parameter count. */
  paramCount: number;
  /** Packed weight and bias buffer. */
  w: Float32Array;
  /** Current hidden state. */
  h: Float32Array;
  /** Scratch buffer for the previous hidden state. */
  _hPrev: Float32Array;

  /**
   * Create an RRU instance with optional weights.
   * @param inSize - Input size.
   * @param hiddenSize - Hidden state size.
   * @param weights - Optional weight buffer.
   * @param initGateBias - Initial reset gate bias.
   */
  constructor(inSize: number, hiddenSize: number, weights: Float32Array | null = null, initGateBias = 0.1) {
    this.inSize = inSize;
    this.hiddenSize = hiddenSize;
    this.paramCount = rruParamCount(inSize, hiddenSize);
    this.w = weights ? weights.slice() : new Float32Array(this.paramCount);
    if (!weights) {
      const I = inSize;
      const H = hiddenSize;
      const Wsz = H * I;
      const Usz = H * H;
      let idx = 0;
      for (let i = 0; i < 2 * Wsz; i++) this.w[idx++] = (Math.random() * 2 - 1) * 0.35;
      for (let i = 0; i < 2 * Usz; i++) this.w[idx++] = (Math.random() * 2 - 1) * 0.18;
      for (let j = 0; j < H; j++) this.w[idx++] = (Math.random() * 2 - 1) * 0.10; // bc
      for (let j = 0; j < H; j++) this.w[idx++] = initGateBias + (Math.random() * 2 - 1) * 0.10; // br
    }
    this.h = new Float32Array(hiddenSize);
    this._hPrev = new Float32Array(hiddenSize);
  }

  /** Reset the hidden state to zero. */
  reset(): void {
    this.h.fill(0);
  }

  /**
   * Advance the RRU by one timestep.
   * @param x - Input vector.
   * @returns Updated hidden state.
   */
  step(x: Float32Array): Float32Array {
    const kernel = requireRruKernel();
    kernel.stepBatch(
      this.w,
      x,
      this.h,
      this._hPrev,
      this.inSize,
      this.hiddenSize,
      1,
      this.inSize
    );
    return this.h;
  }

  /**
   * Advance the RRU by one timestep using the reference JS path.
   * @param x - Input vector.
   * @returns Updated hidden state.
   */
  stepReference(x: Float32Array): Float32Array {
    const I = this.inSize;
    const H = this.hiddenSize;
    const Wsz = H * I;
    const Usz = H * H;
    const Wc = 0;
    const Wr = Wc + Wsz;
    const Uc = Wr + Wsz;
    const Ur = Uc + Usz;
    const bc = Ur + Usz;
    const br = bc + H;

    this._hPrev.set(this.h);
    const hPrev = this._hPrev;

    for (let j = 0; j < H; j++) {
      let sumC = 0;
      let sumR = 0;
      const wcRow = Wc + j * I;
      const wrRow = Wr + j * I;
      for (let i = 0; i < I; i++) {
        const xi = x[i] ?? 0;
        sumC += (this.w[wcRow + i] ?? 0) * xi;
        sumR += (this.w[wrRow + i] ?? 0) * xi;
      }
      const ucRow = Uc + j * H;
      const urRow = Ur + j * H;
      for (let k = 0; k < H; k++) {
        const hk = hPrev[k] ?? 0;
        sumC += (this.w[ucRow + k] ?? 0) * hk;
        sumR += (this.w[urRow + k] ?? 0) * hk;
      }
      sumC += this.w[bc + j] ?? 0;
      sumR += this.w[br + j] ?? 0;
      const cand = Math.tanh(sumC);
      const gate = sigmoid(sumR);
      const prev = hPrev[j] ?? 0;
      this.h[j] = (1 - gate) * prev + gate * cand;
    }
    return this.h;
  }
}

/** Dense output head for mapping features to action outputs. */
export class DenseHead {
  /** Input vector size. */
  inSize: number;
  /** Output vector size. */
  outSize: number;
  /** Total parameter count. */
  paramCount: number;
  /** Packed weight and bias buffer. */
  w: Float32Array;
  /** Output buffer for the latest forward pass. */
  _out: Float32Array;

  /**
   * Create a dense head with optional weights.
   * @param inSize - Input size.
   * @param outSize - Output size.
   * @param weights - Optional weight buffer.
   */
  constructor(inSize: number, outSize: number, weights: Float32Array | null = null) {
    this.inSize = inSize;
    this.outSize = outSize;
    this.paramCount = headParamCount(inSize, outSize);
    this.w = weights ? weights.slice() : new Float32Array(this.paramCount);
    if (!weights) {
      for (let i = 0; i < this.w.length; i++) this.w[i] = clamp((Math.random() * 2 - 1) * 0.45, -5, 5);
    }
    this._out = new Float32Array(outSize);
  }

  /**
   * Run a forward pass through the dense head.
   * @param x - Input vector.
   * @returns Output vector.
   */
  forward(x: Float32Array): Float32Array {
    let idx = 0;
    for (let o = 0; o < this.outSize; o++) {
      let sum = 0;
      for (let i = 0; i < this.inSize; i++) sum += (this.w[idx++] ?? 0) * (x[i] ?? 0);
      sum += this.w[idx++] ?? 0;
      this._out[o] = Math.tanh(sum);
    }
    return this._out;
  }

  /**
   * Run a batched forward pass through the dense head.
   * @param inputs - Packed input buffer.
   * @param outputs - Packed output buffer.
   * @param count - Number of batch entries.
   * @param inputStride - Stride between batch inputs.
   * @param outputStride - Stride between batch outputs.
   */
  forwardBatch(
    inputs: Float32Array,
    outputs: Float32Array,
    count: number,
    inputStride: number,
    outputStride: number
  ): void {
    const outLimit = Math.min(this.outSize, outputStride);
    for (let b = 0; b < count; b++) {
      let idx = 0;
      const baseIn = b * inputStride;
      const baseOut = b * outputStride;
      for (let o = 0; o < this.outSize; o++) {
        let sum = 0;
        for (let i = 0; i < this.inSize; i++) {
          sum += (this.w[idx++] ?? 0) * (inputs[baseIn + i] ?? 0);
        }
        sum += this.w[idx++] ?? 0;
        if (o < outLimit) outputs[baseOut + o] = Math.tanh(sum);
      }
      for (let i = outLimit; i < outputStride; i++) outputs[baseOut + i] = 0;
    }
  }
}
