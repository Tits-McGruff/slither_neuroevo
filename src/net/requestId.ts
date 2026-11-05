/**
 * Generate a cryptographically random 128-bit wire request identity.
 * getRandomValues is available on trusted-LAN HTTP pages, where the
 * secure-context-only randomUUID API is unavailable. Never substitute a
 * timestamp or Math.random: retries depend on collision-resistant identities.
 */
export function createRequestId(): string {
  const bytes = new Uint8Array(16);
  globalThis.crypto.getRandomValues(bytes);
  return Array.from(bytes, byte => byte.toString(16).padStart(2, '0')).join('');
}
