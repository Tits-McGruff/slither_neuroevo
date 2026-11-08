/** Opaque archive transfer and progress UI; connection and admission stay with main. */
export interface ArchiveTransportPort {
  readonly serverWsUrl: string;
  readonly serverArchiveExport: boolean;
  readonly serverArchiveImport: boolean;
  pendingExport: boolean;
  readonly isConnected: () => boolean;
}


/**
 * Resolve the HTTP base URL for server API requests from a WS URL.
 * @param wsUrl - WebSocket URL used for the server connection.
 * @returns HTTP base URL or null when parsing fails.
 */
export function resolveServerHttpBase(wsUrl: string): string | null {
  try {
    const url = new URL(wsUrl);
    const protocol = url.protocol === 'wss:' ? 'https:' : 'http:';
    return `${protocol}//${url.host}`;
  } catch {
    return null;
  }
}


/**
 * Start an opaque direct archive download when the connected authority supports it.
 */
export async function exportServerSnapshot(ports: ArchiveTransportPort): Promise<void> {
  const base = resolveServerHttpBase(ports.serverWsUrl);
  if (!base) {
    ports.pendingExport = false;
    alert('Export failed: invalid server URL.');
    return;
  }
  if (ports.serverArchiveExport) {
    const link = document.createElement('a');
    link.href = `${base}/api/export/latest`;
    link.download = '';
    // Error JSON is not an attachment, so keep its navigation outside the game tab.
    link.target = '_blank';
    link.rel = 'noopener';
    link.hidden = true;
    document.body.append(link);
    link.click();
    link.remove();
    ports.pendingExport = false;
    return;
  }
  ports.pendingExport = false;
  alert('Export unavailable: this server does not support native archive downloads.');
}


/** Small success response returned after an atomic Rust archive replacement. */
interface ServerArchiveImportResult {
  /** True only after SQLite and the running Rust authority both switched. */
  ok: boolean;
  /** Imported run identity. */
  runId: string;
  /** Exact fixed-width imported generation. */
  generation: string;
  /** Exact imported checkpoint identity. */
  checkpointId: string;
  /** True when an older source was resumed under a fresh run identity. */
  branched?: boolean;
  /** Original archive run retained when `branched` is true. */
  sourceRunId?: string;
  /** Bounded rejection detail when the upload fails. */
  message?: string;
}


/** Small structured failure used to offer the explicit safe branch operation. */
class ServerArchiveImportError extends Error {
  /** Stable server rejection code, when supplied. */
  public readonly code: string | undefined;

  public constructor(message: string, code?: string) {
    super(message);
    this.name = 'ServerArchiveImportError';
    this.code = code;
  }
}


/**
 * Upload one selected save unchanged; JavaScript never reads its population bytes.
 * @param file - Browser-owned file selected by the user.
 * @param onProgress - Optional visible upload-byte progress callback.
 * @returns Small committed replacement identity from the server.
 */
function uploadServerArchive(
  file: File,
  ports: ArchiveTransportPort,
  onProgress?: (sentBytes: number, totalBytes: number) => void,
  resumeAsBranch = false
): Promise<ServerArchiveImportResult> {
  const base = resolveServerHttpBase(ports.serverWsUrl);
  if (!base) return Promise.reject(new Error('invalid server URL.'));
  return new Promise((resolve, reject) => {
    const request = new XMLHttpRequest();
    request.open('POST', `${base}/api/import/archive${resumeAsBranch ? '?mode=branch' : ''}`);
    request.setRequestHeader('Content-Type', 'application/vnd.slither-neuroevo.save');
    request.timeout = 0;
    request.upload.onprogress = event => {
      if (event.lengthComputable) onProgress?.(event.loaded, event.total);
    };
    request.onerror = () => reject(new Error('archive upload connection failed'));
    request.onabort = () => reject(new Error('archive upload was cancelled'));
    request.onload = () => {
      let result: (Partial<ServerArchiveImportResult> & { code?: string }) | undefined;
      try { result = JSON.parse(request.responseText) as Partial<ServerArchiveImportResult> & { code?: string }; }
      catch { /* The status below supplies the bounded failure. */ }
      if (request.status < 200 || request.status >= 300 || result?.ok !== true ||
          typeof result.runId !== 'string' || !/^[0-9a-f]{16}$/u.test(result.generation ?? '') ||
          !/^[0-9a-f]{64}$/u.test(result.checkpointId ?? '')) {
        reject(new ServerArchiveImportError(
          result?.message ?? `server archive import failed (${request.status})`, result?.code
        ));
        return;
      }
      resolve(result as ServerArchiveImportResult);
    };
    request.send(file);
  });
}
/** Import the original selected File and preserve the existing progress/branch UI sequence. */
export async function importServerArchive(e: Event, btnImport: HTMLButtonElement, ports: ArchiveTransportPort): Promise<void> {
    const target = e.target as HTMLInputElement | null;
    if (!target?.files?.length) return;
    const file = target.files.item(0);
    if (!file) return;
    const importLabel = btnImport.textContent;
    btnImport.disabled = true;
    try {
      if (ports.serverArchiveImport) {
        const progress = (sentBytes: number, totalBytes: number): void => {
          btnImport.textContent = sentBytes >= totalBytes
            ? 'Activating...'
            : `Importing ${Math.min(99, Math.floor(sentBytes * 100 / totalBytes))}%`;
        };
        let result: ServerArchiveImportResult;
        try {
          result = await uploadServerArchive(file, ports, progress);
        } catch (error) {
          if (!(error instanceof ServerArchiveImportError) || error.code !== 'IMPORT_REQUIRES_BRANCH' ||
              !confirm('This save is older than later history already stored for that run. Resume it as a new run while keeping the later history?')) {
            throw error;
          }
          result = await uploadServerArchive(file, ports, progress, true);
        }
        const source = result.branched && result.sourceRunId
          ? ` as a new run (original run ${result.sourceRunId})` : ` from run ${result.runId}`;
        alert(`Imported generation ${BigInt(`0x${result.generation}`).toString()}${source}.`);
        return;
      }
      throw new Error('Import unavailable: this server does not support native archive uploads.');
    } catch (err) {
      console.error("Import failed", err);
      const error = err as Error;
      if (ports.isConnected() && error.message.includes('no compatible genomes')) {
        alert('Import failed: the file does not match the server brain. Re-export with the latest build (includes graph/settings metadata) or reset the server to the matching graph before importing.');
      } else {
        alert("Failed to import file: " + error.message);
      }
    } finally {
      if (target) target.value = '';
      btnImport.disabled = false;
      btnImport.textContent = importLabel;
    }
  }
