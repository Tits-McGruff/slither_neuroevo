import type { IncomingMessage } from 'node:http';

/**
 * Read one small JSON request with a strict encoded-byte limit.
 * @param request - Incoming request stream.
 * @param limitBytes - Maximum admitted encoded body bytes.
 * @returns Parsed JSON value, or an empty object for an empty body.
 */
export async function readJsonBody(request: IncomingMessage, limitBytes: number): Promise<unknown> {
  const chunks: Buffer[] = [];
  let total = 0;
  for await (const chunk of request) {
    const bytes = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk as ArrayBuffer);
    total += bytes.length;
    if (total > limitBytes) throw new Error('payload too large');
    chunks.push(bytes);
  }
  const text = Buffer.concat(chunks).toString('utf8');
  return text ? JSON.parse(text) as unknown : {};
}
