/** Bounded archive preparation shared by real-server correctness fixtures. */
import { get } from 'node:http';
import { Readable } from 'node:stream';

/**
 * Source leasing, disk admission, archive encoding, validation and publication precede the HTTP response.
 * Correctness fixtures use this as a deadlock guard rather than a performance requirement.
 */
export const ARCHIVE_PREPARATION_TIMEOUT_MS = 30_000;

/**
 * Request a real production archive within its preparation deadline.
 * @param port - Task-owned loopback server port.
 * @param phase - Diagnostic identity of this export within its test workflow.
 * @returns The real response, retaining its bounded signal while the caller consumes the body.
 */
export async function fixtureArchiveDownload(port: number, phase: string): Promise<Response> {
  try {
    // Ephemeral loopback ports can appear on Fetch's browser-oriented blocked list.
    // Native HTTP accepts every OS-assigned port while preserving streaming and cancellation.
    return await new Promise<Response>((resolve, reject) => {
      const request = get({ hostname: '127.0.0.1', port, path: '/api/export/latest',
        signal: AbortSignal.timeout(ARCHIVE_PREPARATION_TIMEOUT_MS) }, response => {
        const headers = new Headers();
        for (let index = 0; index < response.rawHeaders.length; index += 2) {
          headers.append(response.rawHeaders[index]!, response.rawHeaders[index + 1]!);
        }
        const body = Readable.toWeb(response) as ReadableStream<Uint8Array>;
        resolve(new Response(body, { status: response.statusCode ?? 500, headers }));
      });
      request.once('error', reject);
    });
  } catch (cause) {
    throw new Error(`${phase} export did not complete archive preparation`, { cause });
  }
}