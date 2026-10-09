/** Bounded archive preparation shared by real-server correctness fixtures. */

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
    return await fetch(`http://127.0.0.1:${port}/api/export/latest`, {
      signal: AbortSignal.timeout(ARCHIVE_PREPARATION_TIMEOUT_MS)
    });
  } catch (cause) {
    throw new Error(`${phase} export did not complete archive preparation`, { cause });
  }
}