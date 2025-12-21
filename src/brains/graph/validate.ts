import type { GraphSpec } from './schema.ts';
import { compileGraph } from './compiler.ts';

export function validateGraph(spec: GraphSpec): { ok: true } | { ok: false; reason: string } {
  try {
    compileGraph(spec);
    return { ok: true };
  } catch (err) {
    const message = err instanceof Error ? err.message : 'invalid graph';
    return { ok: false, reason: message };
  }
}
