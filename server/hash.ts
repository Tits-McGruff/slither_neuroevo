/** Version of the canonical configuration hash envelope. */
export const CONFIG_HASH_VERSION = 1 as const;

/**
 * Recursively copy a JSON-compatible value with lexically sorted object keys.
 * @param value - Serializable value to canonicalize.
 * @param ancestors - Objects on the active traversal stack.
 * @returns Canonical JSON-compatible value, or undefined for omitted fields.
 */
function canonicalizeValue(value: unknown, ancestors: WeakSet<object>): unknown {
  if (value === null || typeof value === 'string' || typeof value === 'boolean') return value;
  if (typeof value === 'number') return Number.isFinite(value) ? value : null;
  if (typeof value === 'undefined' || typeof value === 'function' || typeof value === 'symbol') {
    return undefined;
  }
  if (typeof value === 'bigint') throw new TypeError('config hash does not support bigint values');
  if (typeof value !== 'object') return undefined;
  if (ancestors.has(value)) throw new TypeError('config hash does not support cyclic values');
  ancestors.add(value);
  try {
    if (Array.isArray(value)) {
      return value.map((entry) => canonicalizeValue(entry, ancestors) ?? null);
    }
    const output: Record<string, unknown> = {};
    for (const key of Object.keys(value).sort()) {
      const normalized = canonicalizeValue((value as Record<string, unknown>)[key], ancestors);
      if (normalized !== undefined) output[key] = normalized;
    }
    return output;
  } finally {
    ancestors.delete(value);
  }
}

/**
 * Serialize a config-like value with a version envelope and sorted keys.
 * @param value - Serializable configuration content.
 * @returns Stable canonical JSON representation.
 */
export function canonicalizeConfig(value: unknown): string {
  const normalized = canonicalizeValue(value, new WeakSet<object>());
  return JSON.stringify({ version: CONFIG_HASH_VERSION, value: normalized ?? null });
}

/**
 * Hash a config-like value to a stable versioned hex string using FNV-1a.
 * @param value - Serializable value to hash.
 * @returns Version-prefixed 8-character content hash.
 */
export function hashConfig(value: unknown): string {
  const json = canonicalizeConfig(value);
  // FNV-1a 32-bit hash for quick config comparisons.
  let hash = 2166136261;
  for (let i = 0; i < json.length; i++) {
    hash ^= json.charCodeAt(i);
    hash = Math.imul(hash, 16777619);
  }
  return `v${CONFIG_HASH_VERSION}-${(hash >>> 0).toString(16).padStart(8, '0')}`;
}
