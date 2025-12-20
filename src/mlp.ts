// mlp.ts
// Definition of a multi‑layer perceptron and genetic operators for
// neuroevolution.  We reuse the same architecture and mutation logic as
// the original monolithic implementation.

import { CFG } from './config.ts';
import { clamp, gaussian } from './utils.ts';

export type ArchKind = 'mlp' | 'mlp_gru';

export interface ArchDefinition {
  kind?: ArchKind;
  mlpSizes?: number[];
  mlp?: number[];
  gruHidden?: number;
  outSize?: number;
  key?: string;
  _info?: ArchInfo;
}

export interface ArchInfo {
  kind: ArchKind;
  mlpSizes: number[];
  gruHidden: number;
  outSize: number;
  mlpCount: number;
  gruCount: number;
  headCount: number;
  totalCount: number;
  featureSize: number;
  offs: { mlp: number; gru: number; head: number };
}

/**
 * Builds a neural network architecture array from UI settings.
 * Uses the global CFG to determine input/output sizes.
 */
export function buildArch(settings: {
  hiddenLayers: number;
  neurons1: number;
  neurons2: number;
  neurons3: number;
  neurons4: number;
  neurons5: number;
}): ArchDefinition {
  const layers = settings.hiddenLayers;
  const hidden = [];
  if (layers >= 1) hidden.push(settings.neurons1);
  if (layers >= 2) hidden.push(settings.neurons2);
  if (layers >= 3) hidden.push(settings.neurons3);
  if (layers >= 4) hidden.push(settings.neurons4);
  if (layers >= 5) hidden.push(settings.neurons5);

  // Ensure at least one hidden layer for feature extraction.
  if (hidden.length === 0) hidden.push(Math.max(4, settings.neurons1 || 8));

  const useGRU = !!(CFG.brain && CFG.brain.useGRU);
  if (!useGRU) {
    const mlpSizes = [CFG.brain.inSize, ...hidden, CFG.brain.outSize];
    return {
      kind: "mlp",
      mlpSizes,
      key: archKey({ kind: "mlp", mlpSizes })
    };
  }

  const gruHidden = Math.max(2, Math.floor(CFG.brain.gruHidden || 8));
  const mlpSizes = [CFG.brain.inSize, ...hidden];
  return {
    kind: "mlp_gru",
    mlpSizes,
    gruHidden,
    outSize: CFG.brain.outSize,
    key: archKey({ kind: "mlp_gru", mlpSizes, gruHidden, outSize: CFG.brain.outSize })
  };
}

/**
 * Produces a stable key for a given neural network architecture.
 */
export function archKey(arch: ArchDefinition | number[]): string {
  if (Array.isArray(arch)) return arch.join("x");
  const kind = arch.kind || "mlp";
  const mlp = (arch.mlpSizes || arch.mlp || []).join("x");
  if (kind === "mlp_gru") {
    const h = arch.gruHidden || 0;
    const o = arch.outSize || CFG.brain.outSize;
    return `mlp:${mlp}|gru:${h}|out:${o}`;
  }
  return `mlp:${mlp}`;
}

/**
 * Simple fully connected neural network with tanh activations.
 */
export class MLP {
  layerSizes: number[];
  key: string;
  paramCount: number;
  w: Float32Array;
  _bufs: Float32Array[];

  constructor(layerSizes: number[], weights: Float32Array | null = null) {
    // Copy sizes to avoid accidental mutation by caller.
    this.layerSizes = layerSizes.slice();
    // Unique key for caching neural architectures.
    this.key = archKey(this.layerSizes);
    // Count the number of parameters required by this network.
    this.paramCount = 0;
    for (let l = 0; l < this.layerSizes.length - 1; l++) {
      const ins = this.layerSizes[l];
      const outs = this.layerSizes[l + 1];
      // Each output has ins weights plus one bias term.
      this.paramCount += outs * ins + outs;
    }
    // If weights are provided, use them; otherwise initialise randomly.
    this.w = weights ? weights.slice() : new Float32Array(this.paramCount);
    if (!weights) {
      for (let i = 0; i < this.paramCount; i++) {
        this.w[i] = (Math.random() * 2 - 1) * 0.6;
      }
    }
    // Per-layer output buffers to avoid per-tick allocations.
    this._bufs = [];
    for (let l = 1; l < this.layerSizes.length; l++) this._bufs.push(new Float32Array(this.layerSizes[l]));
  }

  /**
   * Performs a forward pass through the network.  Uses tanh activations
   * everywhere.  Accepts and returns Float32Array instances.
   * @param {Float32Array} input
   * @returns {Float32Array}
   */
  forward(input: Float32Array): Float32Array {
    let wi = 0;
    let cur = input;
    for (let l = 0; l < this.layerSizes.length - 1; l++) {
      const ins = this.layerSizes[l];
      const outs = this.layerSizes[l + 1];
      const next = this._bufs[l];
      for (let o = 0; o < outs; o++) {
        let sum = 0;
        for (let i = 0; i < ins; i++) sum += this.w[wi++] * cur[i];
        sum += this.w[wi++]; // bias
        next[o] = Math.tanh(sum);
      }
      cur = next;
    }
    return cur;
  }
}

function mlpParamCount(layerSizes: number[]): number {
  let n = 0;
  for (let l = 0; l < layerSizes.length - 1; l++) {
    const ins = layerSizes[l];
    const outs = layerSizes[l + 1];
    n += outs * ins + outs;
  }
  return n;
}

function gruParamCount(inSize: number, hiddenSize: number): number {
  // z, r, h~ gates each have: W (H x I), U (H x H), b (H)
  return 3 * hiddenSize * (inSize + hiddenSize + 1);
}

function headParamCount(hiddenSize: number, outSize: number): number {
  return outSize * hiddenSize + outSize;
}

export function enrichArchInfo(arch: ArchDefinition): ArchInfo {
  if (arch && arch._info) return arch._info;
  const kind = arch.kind || "mlp";
  const mlpSizes = arch.mlpSizes || arch.mlp || [];
  const info: ArchInfo = {
    kind,
    mlpSizes,
    gruHidden: 0,
    outSize: CFG.brain.outSize,
    mlpCount: 0,
    gruCount: 0,
    headCount: 0,
    totalCount: 0,
    featureSize: 0,
    offs: { mlp: 0, gru: 0, head: 0 }
  };
  if (kind === "mlp") {
    info.mlpCount = mlpParamCount(mlpSizes);
    info.totalCount = info.mlpCount;
    info.featureSize = mlpSizes[mlpSizes.length - 1] || 0;
    info.offs = { mlp: 0, gru: info.mlpCount, head: info.mlpCount };
  } else {
    info.gruHidden = Math.max(2, Math.floor(arch.gruHidden || 8));
    info.outSize = Math.max(1, Math.floor(arch.outSize || CFG.brain.outSize));
    info.mlpCount = mlpParamCount(mlpSizes);
    info.featureSize = mlpSizes[mlpSizes.length - 1] || 0;
    info.gruCount = gruParamCount(info.featureSize, info.gruHidden);
    info.headCount = headParamCount(info.gruHidden, info.outSize);
    info.offs = {
      mlp: 0,
      gru: info.mlpCount,
      head: info.mlpCount + info.gruCount
    };
    info.totalCount = info.mlpCount + info.gruCount + info.headCount;
  }
  arch._info = info;
  return info;
}

export function sigmoid(x: number): number {
  // Stable enough for our weight ranges.
  return 1 / (1 + Math.exp(-x));
}

/**
 * Minimal GRU cell. Keeps internal hidden state and exposes step(x).
 */
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
      // Keep recurrent weights smaller for stability.
      const I = inSize;
      const H = hiddenSize;
      const Wsz = H * I; // per-gate input weights
      const Usz = H * H; // per-gate recurrent weights
      let idx = 0;
      // Wz, Wr, Wh
      for (let i = 0; i < 3 * Wsz; i++) this.w[idx++] = (Math.random() * 2 - 1) * 0.35;
      // Uz, Ur, Uh
      for (let i = 0; i < 3 * Usz; i++) this.w[idx++] = (Math.random() * 2 - 1) * 0.18;
      // bz, br, bh
      for (let j = 0; j < H; j++) this.w[idx++] = initUpdateBias + (Math.random() * 2 - 1) * 0.10; // bz
      for (let j = 0; j < H; j++) this.w[idx++] = (Math.random() * 2 - 1) * 0.10; // br
      for (let j = 0; j < H; j++) this.w[idx++] = (Math.random() * 2 - 1) * 0.10; // bh
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

    // Layout: Wz, Wr, Wh, Uz, Ur, Uh, bz, br, bh
    const Wz = 0;
    const Wr = Wz + Wsz;
    const Wh = Wr + Wsz;
    const Uz = Wh + Wsz;
    const Ur = Uz + Usz;
    const Uh = Ur + Usz;
    const bz = Uh + Usz;
    const br = bz + H;
    const bh = br + H;

    // z and r
    for (let j = 0; j < H; j++) {
      let sumZ = 0;
      let sumR = 0;
      const wzRow = Wz + j * I;
      const wrRow = Wr + j * I;
      for (let i = 0; i < I; i++) {
        const xi = x[i];
        sumZ += this.w[wzRow + i] * xi;
        sumR += this.w[wrRow + i] * xi;
      }
      const uzRow = Uz + j * H;
      const urRow = Ur + j * H;
      for (let k = 0; k < H; k++) {
        const hk = this.h[k];
        sumZ += this.w[uzRow + k] * hk;
        sumR += this.w[urRow + k] * hk;
      }
      sumZ += this.w[bz + j];
      sumR += this.w[br + j];
      this._z[j] = sigmoid(sumZ);
      this._r[j] = sigmoid(sumR);
    }

    // candidate and update hidden
    for (let j = 0; j < H; j++) {
      let sumH = 0;
      const whRow = Wh + j * I;
      for (let i = 0; i < I; i++) sumH += this.w[whRow + i] * x[i];
      const uhRow = Uh + j * H;
      for (let k = 0; k < H; k++) {
        sumH += this.w[uhRow + k] * (this._r[k] * this.h[k]);
      }
      sumH += this.w[bh + j];
      const hTilde = Math.tanh(sumH);
      const z = this._z[j];
      this.h[j] = (1 - z) * this.h[j] + z * hTilde;
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
      for (let i = 0; i < this.w.length; i++) this.w[i] = (Math.random() * 2 - 1) * 0.45;
    }
    this._out = new Float32Array(outSize);
  }
  forward(x: Float32Array): Float32Array {
    let idx = 0;
    for (let o = 0; o < this.outSize; o++) {
      let sum = 0;
      for (let i = 0; i < this.inSize; i++) sum += this.w[idx++] * x[i];
      sum += this.w[idx++];
      this._out[o] = Math.tanh(sum);
    }
    return this._out;
  }
}

/**
 * Unified controller wrapper.
 * - kind="mlp": forward(input) returns output.
 * - kind="mlp_gru": forward(input) updates hidden state and returns output.
 */
export class BrainController {
  arch: ArchDefinition;
  info: ArchInfo;
  kind: ArchKind;
  mlp: MLP;
  gru: GRU | null;
  head: DenseHead | null;

  constructor(arch: ArchDefinition, weights: Float32Array) {
    this.arch = arch;
    this.info = enrichArchInfo(arch);
    this.kind = this.info.kind;
    if (this.kind === "mlp") {
      this.mlp = new MLP(this.info.mlpSizes, weights);
      this.gru = null;
      this.head = null;
    } else {
      const w = weights;
      const mlpW = w.slice(0, this.info.mlpCount);
      const gruW = w.slice(this.info.offs.gru, this.info.offs.gru + this.info.gruCount);
      const headW = w.slice(this.info.offs.head, this.info.offs.head + this.info.headCount);
      this.mlp = new MLP(this.info.mlpSizes, mlpW);
      const initBias = CFG.brain && typeof CFG.brain.gruInitUpdateBias === "number" ? CFG.brain.gruInitUpdateBias : -0.7;
      this.gru = new GRU(this.info.featureSize, this.info.gruHidden, gruW, initBias);
      this.head = new DenseHead(this.info.gruHidden, this.info.outSize, headW);
    }
  }

  reset(): void {
    if (this.gru) this.gru.reset();
  }

  forward(input: Float32Array): Float32Array {
    if (this.kind === "mlp") return this.mlp.forward(input);
    const feat = this.mlp.forward(input);
    const h = this.gru!.step(feat);
    return this.head!.forward(h);
  }
}
/**
 * Represents an individual in the population.  Stores a neural
 * architecture key, the weight vector and the fitness score.
 */
export class Genome {
  archKey: string;
  weights: Float32Array;
  fitness: number;

  constructor(archKey: string, weights: Float32Array) {
    this.archKey = archKey;
    this.weights = weights ? weights.slice() : new Float32Array(0);
    this.fitness = 0;
  }
  /**
   * Creates a genome with a randomly initialised weight vector for the
   * given architecture.
   * @param {Array<number>} arch
   * @returns {Genome}
   */
  static random(arch: ArchDefinition): Genome {
    const info = enrichArchInfo(arch);
    if (info.kind === "mlp") {
      const net = new MLP(info.mlpSizes);
      return new Genome(arch.key || net.key, net.w);
    }
    const w = new Float32Array(info.totalCount);
    // MLP weights
    const mlp = new MLP(info.mlpSizes);
    w.set(mlp.w, 0);
    // GRU weights
    const initBias = CFG.brain && typeof CFG.brain.gruInitUpdateBias === "number" ? CFG.brain.gruInitUpdateBias : -0.7;
    const gru = new GRU(info.featureSize, info.gruHidden, null, initBias);
    w.set(gru.w, info.offs.gru);
    // Head weights
    const head = new DenseHead(info.gruHidden, info.outSize);
    w.set(head.w, info.offs.head);
    return new Genome(arch.key || archKey(arch), w);
  }
  /**
   * Builds an MLP instance from the stored weights.
   * @param {Array<number>} arch
   */
  buildBrain(arch: ArchDefinition): BrainController {
    const brain = new BrainController(arch, this.weights);
    brain.reset();
    return brain;
  }
  /**
   * Creates a deep copy of this genome, preserving fitness and weights.
   * @returns {Genome}
   */
  clone(): Genome {
    const g = new Genome(this.archKey, this.weights);
    g.fitness = this.fitness;
    return g;
  }
  /**
   * Serialises the genome to a plain object.
   * @returns {Object}
   */
  toJSON(): { archKey: string; weights: number[]; fitness: number } {
    return {
      archKey: this.archKey,
      weights: Array.from(this.weights),
      fitness: this.fitness
    };
  }
  /**
   * Reconstructs a genome from a plain object.
   * @param {Object} json
   * @returns {Genome}
   */
  static fromJSON(json: { archKey: string; weights: number[]; fitness?: number }): Genome {
    const g = new Genome(json.archKey, new Float32Array(json.weights));
    g.fitness = json.fitness || 0;
    return g;
  }
}

/**
 * Performs crossover between two parent genomes.  With probability
 * 1−crossoverRate the child is a copy of one parent; otherwise each
 * weight is chosen randomly from either parent.
 * @param {Genome} a
 * @param {Genome} b
 * @returns {Genome}
 */
export function crossover(a: Genome, b: Genome, arch: ArchDefinition): Genome {
  const info = enrichArchInfo(arch);
  const wa = a.weights;
  const wb = b.weights;
  const n = wa.length;
  const child = new Float32Array(n);

  // MLP-only uses the original uniform crossover.
  if (info.kind === "mlp") {
    if (Math.random() > CFG.crossoverRate) {
      child.set(Math.random() < 0.5 ? wa : wb);
      return new Genome(a.archKey, child);
    }
    for (let i = 0; i < n; i++) child[i] = Math.random() < 0.5 ? wa[i] : wb[i];
    return new Genome(a.archKey, child);
  }

  // For recurrent controllers we keep crossover structured.
  // MLP feature extractor + head: original uniform crossover.
  // GRU block: either inherit as a whole (mode 0) or unit-wise row crossover (mode 1).
  const mlpEnd = info.mlpCount;
  const gruStart = info.offs.gru;
  const gruEnd = info.offs.head;
  const headStart = info.offs.head;

  // MLP segment
  if (Math.random() > CFG.crossoverRate) {
    // Copy all weights from one parent as the baseline.
    child.set(Math.random() < 0.5 ? wa : wb);
  } else {
    for (let i = 0; i < mlpEnd; i++) child[i] = Math.random() < 0.5 ? wa[i] : wb[i];
    // Head segment
    for (let i = headStart; i < n; i++) child[i] = Math.random() < 0.5 ? wa[i] : wb[i];
  }

  const mode = Math.floor((CFG.brain && CFG.brain.gruCrossoverMode) || 0);
  if (mode === 0) {
    // Inherit the entire GRU block from one parent.
    const src = Math.random() < 0.5 ? wa : wb;
    child.set(src.subarray(gruStart, gruEnd), gruStart);
  } else {
    // Unit-wise row crossover: copy each unit's rows (all gates) from one parent.
    const I = info.featureSize;
    const H = info.gruHidden;
    const Wsz = H * I;
    const Usz = H * H;
    const local = gruStart;
    // Layout inside GRU block: Wz, Wr, Wh, Uz, Ur, Uh, bz, br, bh
    const Wz = local;
    const Wr = Wz + Wsz;
    const Wh = Wr + Wsz;
    const Uz = Wh + Wsz;
    const Ur = Uz + Usz;
    const Uh = Ur + Usz;
    const bz = Uh + Usz;
    const br = bz + H;
    const bh = br + H;
    for (let j = 0; j < H; j++) {
      const src = Math.random() < 0.5 ? wa : wb;
      // Input weights rows
      child.set(src.subarray(Wz + j * I, Wz + (j + 1) * I), Wz + j * I);
      child.set(src.subarray(Wr + j * I, Wr + (j + 1) * I), Wr + j * I);
      child.set(src.subarray(Wh + j * I, Wh + (j + 1) * I), Wh + j * I);
      // Recurrent rows
      child.set(src.subarray(Uz + j * H, Uz + (j + 1) * H), Uz + j * H);
      child.set(src.subarray(Ur + j * H, Ur + (j + 1) * H), Ur + j * H);
      child.set(src.subarray(Uh + j * H, Uh + (j + 1) * H), Uh + j * H);
      // Biases
      child[bz + j] = src[bz + j];
      child[br + j] = src[br + j];
      child[bh + j] = src[bh + j];
    }
  }

  return new Genome(a.archKey, child);
}

/**
 * Applies mutation to a genome in place.  Each weight has a chance to be
 * perturbed by a Gaussian noise scaled by mutationStd.
 * @param {Genome} genome
 */
export function mutate(genome: Genome, arch: ArchDefinition): void {
  const info = enrichArchInfo(arch);
  const w = genome.weights;
  if (info.kind === "mlp") {
    for (let i = 0; i < w.length; i++) {
      if (Math.random() < CFG.mutationRate) w[i] = clamp(w[i] + gaussian() * CFG.mutationStd, -5, 5);
    }
    return;
  }
  const gruStart = info.offs.gru;
  const gruEnd = info.offs.head;
  const mRateGRU = (CFG.brain && typeof CFG.brain.gruMutationRate === "number") ? CFG.brain.gruMutationRate : CFG.mutationRate;
  const mStdGRU = (CFG.brain && typeof CFG.brain.gruMutationStd === "number") ? CFG.brain.gruMutationStd : CFG.mutationStd;
  for (let i = 0; i < w.length; i++) {
    const inGRU = i >= gruStart && i < gruEnd;
    const rate = inGRU ? mRateGRU : CFG.mutationRate;
    const std = inGRU ? mStdGRU : CFG.mutationStd;
    if (Math.random() < rate) w[i] = clamp(w[i] + gaussian() * std, -5, 5);
  }
}
