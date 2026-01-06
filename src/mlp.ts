/** Architecture selection, genome representation, and evolution operators. */

import { CFG } from './config.ts';
import { clamp, gaussian } from './utils.ts';
import { buildStackGraphSpec } from './brains/stackBuilder.ts';
import { graphKey } from './brains/graph/compiler.ts';
import type { CompiledGraph } from './brains/graph/compiler.ts';
import type { GraphNodeType, GraphSpec } from './brains/graph/schema.ts';
import { validateGraph } from './brains/graph/validate.ts';
import type { Brain } from './brains/types.ts';
import { buildBrain as buildBrainInstance, compileBrainSpec } from './brains/registry.ts';
import { DenseHead, GRU, LSTM, MLP, RRU } from './brains/ops.ts';

/** Re-export core brain ops and parameter helpers. */
export {
  DenseHead,
  GRU,
  LSTM,
  MLP,
  RRU,
  gruParamCount,
  headParamCount,
  lstmParamCount,
  mlpParamCount,
  rruParamCount,
  sigmoid
} from './brains/ops.ts';

/** Architecture definition bundling a graph spec and its key. */
export interface ArchDefinition {
  spec: GraphSpec;
  key: string;
  _info?: ArchInfo;
}

/** Parameter metadata for a single graph node. */
export interface NodeParamInfo {
  id: string;
  type: GraphNodeType;
  offset: number;
  length: number;
  inputSize: number;
  outputSize: number;
  hiddenSize?: number;
  hiddenSizes?: number[];
  isRecurrent: boolean;
}

/** Cached architecture info derived from a compiled graph. */
export interface ArchInfo {
  key: string;
  spec: GraphSpec;
  totalCount: number;
  nodes: NodeParamInfo[];
  compiled: CompiledGraph;
}

/**
 * Builds a stacked brain graph spec from UI settings + CFG toggles.
 * @param settings - Layer size and depth settings from the UI.
 * @returns Architecture definition derived from settings or graph spec.
 */
export function buildArch(settings: {
  hiddenLayers: number;
  neurons1: number;
  neurons2: number;
  neurons3: number;
  neurons4: number;
  neurons5: number;
}): ArchDefinition {
  const customSpec = CFG.brain?.graphSpec as GraphSpec | null | undefined;
  if (customSpec && typeof customSpec === 'object') {
    const result = validateGraph(customSpec);
    if (result.ok) {
      const inputNode = customSpec.nodes.find(node => node.type === 'Input');
      const inputSize = inputNode && 'outputSize' in inputNode ? inputNode.outputSize : null;
      if (inputSize === CFG.brain.inSize && customSpec.outputSize === CFG.brain.outSize) {
        return { spec: customSpec, key: graphKey(customSpec) };
      }
      console.warn('[Brain] Ignoring custom graph spec (input/output size mismatch).');
    } else {
      console.warn('[Brain] Ignoring custom graph spec:', result.reason);
    }
  }
  const spec = buildStackGraphSpec(settings, CFG);
  return { spec, key: graphKey(spec) };
}

/**
 * Compute a stable architecture key from a definition or raw spec.
 * @param arch - Architecture definition or raw graph spec.
 * @returns Graph key string.
 */
export function archKey(arch: ArchDefinition | GraphSpec): string {
  if ('spec' in arch) return graphKey(arch.spec);
  return graphKey(arch);
}

/**
 * Enrich an architecture with compiled info and parameter metadata.
 * @param arch - Architecture definition to enrich.
 * @returns Cached architecture info.
 */
export function enrichArchInfo(arch: ArchDefinition): ArchInfo {
  if (arch && arch._info) return arch._info;
  const compiled = compileBrainSpec(arch.spec);
  const nodes: NodeParamInfo[] = compiled.nodes.map(node => {
    const base: NodeParamInfo = {
      id: node.id,
      type: node.type,
      offset: node.paramOffset,
      length: node.paramLength,
      inputSize: node.inputSize,
      outputSize: node.outputSize,
      isRecurrent: node.type === 'GRU' || node.type === 'LSTM' || node.type === 'RRU'
    };
    if (node.hiddenSize != null) base.hiddenSize = node.hiddenSize;
    if (node.hiddenSizes && node.hiddenSizes.length) base.hiddenSizes = node.hiddenSizes.slice();
    return base;
  });
  const info: ArchInfo = {
    key: compiled.key,
    spec: compiled.spec,
    totalCount: compiled.totalParams,
    nodes,
    compiled
  };
  arch._info = info;
  return info;
}

/**
 * Represents an individual genome (weights + metadata).
 */
export class Genome {
  /** Architecture key used for compatibility checks. */
  archKey: string;
  /** Brain type identifier used for registry lookups. */
  brainType: string;
  /** Weight buffer for the genome. */
  weights: Float32Array;
  /** Cached fitness value assigned during evolution. */
  fitness: number;

  /**
   * Create a genome with an architecture key and weights.
   * @param archKey - Architecture key string.
   * @param weights - Weight array to store.
   * @param brainType - Brain type identifier.
   */
  constructor(archKey: string, weights: Float32Array, brainType = 'mlp') {
    this.archKey = archKey;
    this.brainType = brainType;
    this.weights = weights ? weights.slice() : new Float32Array(0);
    this.fitness = 0;
  }

  /**
   * Build a randomized genome for a specific architecture.
   * @param arch - Architecture definition to target.
   * @returns New randomized genome.
   */
  static random(arch: ArchDefinition): Genome {
    const info = enrichArchInfo(arch);
    const w = new Float32Array(info.totalCount);
    for (const node of info.nodes) {
      if (node.length <= 0) continue;
      const slice = w.subarray(node.offset, node.offset + node.length);
      switch (node.type) {
        case 'MLP': {
          const sizes = [node.inputSize, ...(node.hiddenSizes ?? []), node.outputSize];
          const mlp = new MLP(sizes);
          slice.set(mlp.w);
          break;
        }
        case 'Dense': {
          const head = new DenseHead(node.inputSize, node.outputSize);
          slice.set(head.w);
          break;
        }
        case 'GRU': {
          const initBias = CFG.brain && typeof CFG.brain.gruInitUpdateBias === 'number'
            ? CFG.brain.gruInitUpdateBias
            : -0.7;
          const gru = new GRU(node.inputSize, node.hiddenSize ?? node.outputSize, null, initBias);
          slice.set(gru.w);
          break;
        }
        case 'LSTM': {
          const initBias = CFG.brain && typeof CFG.brain.lstmInitForgetBias === 'number'
            ? CFG.brain.lstmInitForgetBias
            : 0.6;
          const lstm = new LSTM(node.inputSize, node.hiddenSize ?? node.outputSize, null, initBias);
          slice.set(lstm.w);
          break;
        }
        case 'RRU': {
          const initBias = CFG.brain && typeof CFG.brain.rruInitGateBias === 'number'
            ? CFG.brain.rruInitGateBias
            : 0.1;
          const rru = new RRU(node.inputSize, node.hiddenSize ?? node.outputSize, null, initBias);
          slice.set(rru.w);
          break;
        }
        default:
          break;
      }
    }
    return new Genome(arch.key, w, arch.spec.type);
  }

  /**
   * Build a brain instance from the genome weights and architecture.
   * @param arch - Architecture definition to compile.
   * @returns Initialized brain instance.
   */
  buildBrain(arch: ArchDefinition): Brain {
    const info = enrichArchInfo(arch);
    const brain = buildBrainInstance(info.compiled.spec, this.weights);
    brain.reset();
    return brain;
  }

  /**
   * Clone this genome including weights and fitness.
   * @returns New genome clone.
   */
  clone(): Genome {
    const g = new Genome(this.archKey, this.weights, this.brainType);
    g.fitness = this.fitness;
    return g;
  }

  /**
   * Serialize this genome into JSON-friendly data.
   * @returns JSON payload for persistence.
   */
  toJSON(): { archKey: string; brainType: string; weights: number[]; fitness: number } {
    return {
      archKey: this.archKey,
      brainType: this.brainType,
      weights: Array.from(this.weights),
      fitness: this.fitness
    };
  }

  /**
   * Deserialize a genome from a JSON payload.
   * @param json - JSON payload containing weights and metadata.
   * @returns New genome instance.
   */
  static fromJSON(json: { archKey: string; brainType?: string; weights: number[]; fitness?: number }): Genome {
    const g = new Genome(json.archKey, new Float32Array(json.weights), json.brainType || 'mlp');
    g.fitness = json.fitness || 0;
    return g;
  }
}

/**
 * Perform structured crossover for recurrent blocks.
 * @param out - Output weight buffer to write into.
 * @param wa - Parent A weights.
 * @param wb - Parent B weights.
 * @param node - Node parameter metadata.
 * @param mode - Crossover mode (0 block, 1 unit).
 */
function crossoverRecurrentBlock(
  out: Float32Array,
  wa: Float32Array,
  wb: Float32Array,
  node: NodeParamInfo,
  mode: number
): void {
  const offset = node.offset;
  const len = node.length;
  if (mode === 0) {
    const src = Math.random() < 0.5 ? wa : wb;
    out.set(src.subarray(offset, offset + len), offset);
    return;
  }
  const I = node.inputSize;
  const H = node.hiddenSize ?? node.outputSize;
  const Wsz = H * I;
  const Usz = H * H;
  if (node.type === 'GRU') {
    const Wz = offset;
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
      out.set(src.subarray(Wz + j * I, Wz + (j + 1) * I), Wz + j * I);
      out.set(src.subarray(Wr + j * I, Wr + (j + 1) * I), Wr + j * I);
      out.set(src.subarray(Wh + j * I, Wh + (j + 1) * I), Wh + j * I);
      out.set(src.subarray(Uz + j * H, Uz + (j + 1) * H), Uz + j * H);
      out.set(src.subarray(Ur + j * H, Ur + (j + 1) * H), Ur + j * H);
      out.set(src.subarray(Uh + j * H, Uh + (j + 1) * H), Uh + j * H);
      out[bz + j] = src[bz + j] ?? 0;
      out[br + j] = src[br + j] ?? 0;
      out[bh + j] = src[bh + j] ?? 0;
    }
    return;
  }
  if (node.type === 'LSTM') {
    const Wi = offset;
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
      const src = Math.random() < 0.5 ? wa : wb;
      out.set(src.subarray(Wi + j * I, Wi + (j + 1) * I), Wi + j * I);
      out.set(src.subarray(Wf + j * I, Wf + (j + 1) * I), Wf + j * I);
      out.set(src.subarray(Wo + j * I, Wo + (j + 1) * I), Wo + j * I);
      out.set(src.subarray(Wg + j * I, Wg + (j + 1) * I), Wg + j * I);
      out.set(src.subarray(Ui + j * H, Ui + (j + 1) * H), Ui + j * H);
      out.set(src.subarray(Uf + j * H, Uf + (j + 1) * H), Uf + j * H);
      out.set(src.subarray(Uo + j * H, Uo + (j + 1) * H), Uo + j * H);
      out.set(src.subarray(Ug + j * H, Ug + (j + 1) * H), Ug + j * H);
      out[bi + j] = src[bi + j] ?? 0;
      out[bf + j] = src[bf + j] ?? 0;
      out[bo + j] = src[bo + j] ?? 0;
      out[bg + j] = src[bg + j] ?? 0;
    }
    return;
  }
  if (node.type === 'RRU') {
    const Wc = offset;
    const Wr = Wc + Wsz;
    const Uc = Wr + Wsz;
    const Ur = Uc + Usz;
    const bc = Ur + Usz;
    const br = bc + H;
    for (let j = 0; j < H; j++) {
      const src = Math.random() < 0.5 ? wa : wb;
      out.set(src.subarray(Wc + j * I, Wc + (j + 1) * I), Wc + j * I);
      out.set(src.subarray(Wr + j * I, Wr + (j + 1) * I), Wr + j * I);
      out.set(src.subarray(Uc + j * H, Uc + (j + 1) * H), Uc + j * H);
      out.set(src.subarray(Ur + j * H, Ur + (j + 1) * H), Ur + j * H);
      out[bc + j] = src[bc + j] ?? 0;
      out[br + j] = src[br + j] ?? 0;
    }
  }
}

/**
 * Create a child genome by crossover of two parents.
 * @param a - Parent A genome.
 * @param b - Parent B genome.
 * @param arch - Architecture definition.
 * @returns Child genome.
 */
export function crossover(a: Genome, b: Genome, arch: ArchDefinition): Genome {
  const info = enrichArchInfo(arch);
  const wa = a.weights;
  const wb = b.weights;
  const n = wa.length;
  const child = new Float32Array(n);

  if (Math.random() > CFG.crossoverRate) {
    child.set(Math.random() < 0.5 ? wa : wb);
    return new Genome(a.archKey, child, arch.spec.type);
  }

  const mode = Math.floor((CFG.brain && CFG.brain.gruCrossoverMode) || 0);
  for (const node of info.nodes) {
    if (node.length <= 0) continue;
    if (node.isRecurrent) {
      crossoverRecurrentBlock(child, wa, wb, node, mode);
      continue;
    }
    for (let i = node.offset; i < node.offset + node.length; i++) {
      const aVal = wa[i] ?? 0;
      const bVal = wb[i] ?? 0;
      child[i] = Math.random() < 0.5 ? aVal : bVal;
    }
  }
  return new Genome(a.archKey, child, arch.spec.type);
}

/**
 * Mutate a genome in-place using configured mutation rates.
 * @param genome - Genome to mutate.
 * @param arch - Architecture definition for recurrent ranges.
 */
export function mutate(genome: Genome, arch: ArchDefinition): void {
  const info = enrichArchInfo(arch);
  const w = genome.weights;
  const recurrentRanges: Array<{ start: number; end: number }> = [];
  for (const node of info.nodes) {
    if (!node.isRecurrent) continue;
    recurrentRanges.push({ start: node.offset, end: node.offset + node.length });
  }
  const mRateGRU = (CFG.brain && typeof CFG.brain.gruMutationRate === 'number') ? CFG.brain.gruMutationRate : CFG.mutationRate;
  const mStdGRU = (CFG.brain && typeof CFG.brain.gruMutationStd === 'number') ? CFG.brain.gruMutationStd : CFG.mutationStd;
  for (let i = 0; i < w.length; i++) {
    const inRecurrent = recurrentRanges.some(r => i >= r.start && i < r.end);
    const rate = inRecurrent ? mRateGRU : CFG.mutationRate;
    const std = inRecurrent ? mStdGRU : CFG.mutationStd;
    if (Math.random() < rate) w[i] = clamp((w[i] ?? 0) + gaussian() * std, -5, 5);
  }
}
