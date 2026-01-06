/**
 * Hash a config-like value to a stable hex string using FNV-1a.
 * @param value - Serializable value to hash.
 * @returns 8-char hex hash string.
 */
export function hashConfig(value: unknown): string {
  const json = JSON.stringify(value);
  // FNV-1a 32-bit hash for quick config comparisons.
  let hash = 2166136261;
  for (let i = 0; i < json.length; i++) {
    hash ^= json.charCodeAt(i);
    hash = Math.imul(hash, 16777619);
  }
  return (hash >>> 0).toString(16).padStart(8, '0');
}
