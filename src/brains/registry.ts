import type { Brain } from './types.ts';
import type { GraphSpec } from './graph/schema.ts';
import { compileGraph, graphKey } from './graph/compiler.ts';
import type { CompiledGraph } from './graph/compiler.ts';
import { GraphBrain } from './graph/runtime.ts';

/** Brain spec alias for registry compatibility. */
export type BrainSpec = GraphSpec;
/** Factory signature for constructing brains. */
export type BrainFactory = (spec: BrainSpec, weights: Float32Array) => Brain;

/** Registry of brain factories keyed by spec type. */
const registry = new Map<string, BrainFactory>();
/** Cache of compiled graph specs keyed by graph key. */
const compiledCache = new Map<string, CompiledGraph>();

/**
 * Compile a brain spec or return cached compilation.
 * @param spec - Brain spec to compile.
 * @returns Compiled graph spec.
 */
function getCompiled(spec: BrainSpec): CompiledGraph {
  const key = graphKey(spec);
  const cached = compiledCache.get(key);
  if (cached) return cached;
  const compiled = compileGraph(spec);
  compiledCache.set(key, compiled);
  return compiled;
}

/**
 * Register a brain factory for a given type.
 * @param type - Brain type identifier.
 * @param factory - Factory function for that type.
 */
export function registerBrain(type: string, factory: BrainFactory): void {
  registry.set(type, factory);
}

/**
 * Build a brain instance from a spec and weights.
 * @param spec - Brain spec to use.
 * @param weights - Weight buffer for the brain.
 * @returns Brain instance.
 */
export function buildBrain(spec: BrainSpec, weights: Float32Array): Brain {
  const factory = registry.get(spec.type);
  if (!factory) throw new Error(`Unknown brain type: ${spec.type}`);
  return factory(spec, weights);
}

/**
 * Compute the parameter length for a brain spec.
 * @param spec - Brain spec to compile.
 * @returns Total parameter count.
 */
export function brainParamLength(spec: BrainSpec): number {
  return getCompiled(spec).totalParams;
}

/**
 * Compile a brain spec to a compiled graph.
 * @param spec - Brain spec to compile.
 * @returns Compiled graph spec.
 */
export function compileBrainSpec(spec: BrainSpec): CompiledGraph {
  return getCompiled(spec);
}

/** Register the graph brain implementation. */
registerBrain('graph', (spec, weights) => {
  const compiled = getCompiled(spec);
  return new GraphBrain(compiled, weights);
});
