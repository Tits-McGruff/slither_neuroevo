// ops.ts
// Low-level neural network primitives and parameter layouts used by brains.

import { clamp } from '../utils.ts';

export function sigmoid(x: number): number {
  return 1 / (1 + Math.exp(-x));
}

export function mlpParamCount(layerSizes: number[]): number {
  let n = 0;
  for (let l = 0; l < layerSizes.length - 1; l++) {
    const ins = layerSizes[l]!;
    const outs = layerSizes[l + 1]!;
    n += outs * ins + outs;
  }
  return n;
}

export function gruParamCount(inSize: number, hiddenSize: number): number {
  return 3 * hiddenSize * (inSize + hiddenSize + 1);
}

export function lstmParamCount(inSize: number, hiddenSize: number): number {
  return 4 * hiddenSize * (inSize + hiddenSize + 1);
}

export function rruParamCount(inSize: number, hiddenSize: number): number {
  return 2 * hiddenSize * (inSize + hiddenSize + 1);
}

export function headParamCount(hiddenSize: number, outSize: number): number {
  return outSize * hiddenSize + outSize;
}

export class MLP {
  layerSizes: number[];
  key: string;
  paramCount: number;
  w: Float32Array;
  _bufs: Float32Array[];

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
}

export class GRU {
  inSize: number;
  hiddenSize: number;
  paramCount: number;
  w: Float32Array;
  h: Float32Array;
  _z: Float32Array;
  _r: Float32Array;

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
  }

  reset(): void {
    this.h.fill(0);
  }

  step(x: Float32Array): Float32Array {
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
        const hk = this.h[k] ?? 0;
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
        const hVal = this.h[k] ?? 0;
        sumH += (this.w[uhRow + k] ?? 0) * (rVal * hVal);
      }
      sumH += this.w[bh + j] ?? 0;
      const hTilde = Math.tanh(sumH);
      const z = this._z[j] ?? 0;
      const prevH = this.h[j] ?? 0;
      this.h[j] = (1 - z) * prevH + z * hTilde;
    }
    return this.h;
  }
}

export class LSTM {
  inSize: number;
  hiddenSize: number;
  paramCount: number;
  w: Float32Array;
  h: Float32Array;
  c: Float32Array;

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
  }

  reset(): void {
    this.h.fill(0);
    this.c.fill(0);
  }

  step(x: Float32Array): Float32Array {
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
        const hk = this.h[k] ?? 0;
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
      const nextC = fGate * (this.c[j] ?? 0) + iGate * gGate;
      this.c[j] = nextC;
      this.h[j] = oGate * Math.tanh(nextC);
    }
    return this.h;
  }
}

export class RRU {
  inSize: number;
  hiddenSize: number;
  paramCount: number;
  w: Float32Array;
  h: Float32Array;

  constructor(inSize: number, hiddenSize: number, weights: Float32Array | null = null) {
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
      for (let j = 0; j < H; j++) this.w[idx++] = (Math.random() * 2 - 1) * 0.10; // br
    }
    this.h = new Float32Array(hiddenSize);
  }

  reset(): void {
    this.h.fill(0);
  }

  step(x: Float32Array): Float32Array {
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
        const hk = this.h[k] ?? 0;
        sumC += (this.w[ucRow + k] ?? 0) * hk;
        sumR += (this.w[urRow + k] ?? 0) * hk;
      }
      sumC += this.w[bc + j] ?? 0;
      sumR += this.w[br + j] ?? 0;
      const cand = Math.tanh(sumC);
      const gate = sigmoid(sumR);
      const prev = this.h[j] ?? 0;
      this.h[j] = (1 - gate) * prev + gate * cand;
    }
    return this.h;
  }
}

export class DenseHead {
  inSize: number;
  outSize: number;
  paramCount: number;
  w: Float32Array;
  _out: Float32Array;

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
}
