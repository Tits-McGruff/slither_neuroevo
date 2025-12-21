import type { Brain } from './types.ts';
import type { GraphSpec } from './graph/schema.ts';
import { compileGraph, graphKey } from './graph/compiler.ts';
import type { CompiledGraph } from './graph/compiler.ts';
import { GraphBrain } from './graph/runtime.ts';

export type BrainSpec = GraphSpec;
export type BrainFactory = (spec: BrainSpec, weights: Float32Array) => Brain;

const registry = new Map<string, BrainFactory>();
const compiledCache = new Map<string, CompiledGraph>();

function getCompiled(spec: BrainSpec): CompiledGraph {
  const key = graphKey(spec);
  const cached = compiledCache.get(key);
  if (cached) return cached;
  const compiled = compileGraph(spec);
  compiledCache.set(key, compiled);
  return compiled;
}

export function registerBrain(type: string, factory: BrainFactory): void {
  registry.set(type, factory);
}

export function buildBrain(spec: BrainSpec, weights: Float32Array): Brain {
  const factory = registry.get(spec.type);
  if (!factory) throw new Error(`Unknown brain type: ${spec.type}`);
  return factory(spec, weights);
}

export function brainParamLength(spec: BrainSpec): number {
  return getCompiled(spec).totalParams;
}

export function compileBrainSpec(spec: BrainSpec): CompiledGraph {
  return getCompiled(spec);
}

registerBrain('graph', (spec, weights) => {
  const compiled = getCompiled(spec);
  return new GraphBrain(compiled, weights);
});
