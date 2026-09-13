import { open, realpath, rename, unlink } from 'node:fs/promises';
import { join } from 'node:path';
import type { CheckpointOperationId, U64Hex } from './checkpointPersistenceProtocol.ts';

/** Four-GiB fixed ceiling for the current experimental save profile. */
export const P0_ARCHIVE_UPLOAD_LIMIT = 4n * 1024n * 1024n * 1024n;
/** Default maximum silence between successive upload chunks. */
export const ARCHIVE_UPLOAD_NO_PROGRESS_MS = 60_000;

/** Inputs for one opaque raw-body upload into the controlled scratch directory. */
export interface ArchiveUploadOptions {
  /** Request body read incrementally without buffering or text conversion. */
  source: AsyncIterable<Uint8Array>;
  /** Optional canonical decimal Content-Length supplied by the HTTP peer. */
  contentLength: string | undefined;
  /** Existing server-controlled scratch directory. */
  scratchDirectory: string;
  /** Unpredictable operation token used only for controlled child filenames. */
  operationId: CheckpointOperationId;
  /** Maximum accepted wire bytes. */
  maximumBytes?: bigint;
  /** Maximum silence between chunks; exposed only for focused timing tests. */
  noProgressTimeoutMs?: number;
}

/** Exact ready-file facts; the upload body itself never enters this object. */
export interface SpooledArchiveUpload {
  /** Exact operation that owns this ready file. */
  operationId: CheckpointOperationId;
  /** Controlled direct-child basename. */
  relativeFilename: string;
  /** Absolute ready path below the canonical scratch directory. */
  readyPath: string;
  /** Exact received bytes as canonical fixed-width hexadecimal. */
  storedByteCount: U64Hex;
}

/** Bounded upload rejection with a stable machine-readable category. */
export class ArchiveUploadError extends Error {
  /** Stable category suitable for a small HTTP error response. */
  public readonly code: 'INVALID_LENGTH' | 'ARCHIVE_TOO_LARGE' | 'LENGTH_MISMATCH' | 'EMPTY_ARCHIVE' | 'NO_PROGRESS';

  /** Construct one bounded upload rejection. */
  public constructor(
    code: ArchiveUploadError['code'],
    message: string
  ) {
    super(message);
    this.name = 'ArchiveUploadError';
    this.code = code;
  }
}

/** Await one input chunk with an idle deadline that resets for every successful read. */
async function nextChunk(
  iterator: AsyncIterator<Uint8Array>,
  timeoutMs: number
): Promise<IteratorResult<Uint8Array>> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new ArchiveUploadError(
      'NO_PROGRESS',
      `archive upload made no progress for ${timeoutMs} ms`
    )), timeoutMs);
    timer.unref();
    iterator.next().then(value => {
      clearTimeout(timer);
      resolve(value);
    }, error => {
      clearTimeout(timer);
      reject(error);
    });
  });
}

/** Parse an optional HTTP length before opening an upload spool. */
function parseContentLength(value: string | undefined, maximumBytes: bigint): bigint | undefined {
  if (value === undefined) return undefined;
  if (!/^(?:0|[1-9][0-9]*)$/u.test(value)) {
    throw new ArchiveUploadError('INVALID_LENGTH', 'archive Content-Length must be one canonical decimal integer');
  }
  const bytes = BigInt(value);
  if (bytes === 0n) throw new ArchiveUploadError('EMPTY_ARCHIVE', 'archive upload is empty');
  if (bytes > maximumBytes) {
    throw new ArchiveUploadError(
      'ARCHIVE_TOO_LARGE',
      `archive declares ${bytes} bytes; the current limit is ${maximumBytes} bytes`
    );
  }
  return bytes;
}

/** Convert one admitted byte count to the shared unsigned-64-bit wire form. */
function u64Hex(value: bigint): U64Hex {
  return value.toString(16).padStart(16, '0') as U64Hex;
}

/**
 * Stream one raw request body to a synced operation-local file.
 *
 * The caller owns deletion of the returned ready file. Any unsuccessful path
 * removes both partial and ready names before rejecting.
 */
export async function spoolArchiveUpload(options: ArchiveUploadOptions): Promise<SpooledArchiveUpload> {
  const maximumBytes = options.maximumBytes ?? P0_ARCHIVE_UPLOAD_LIMIT;
  const noProgressTimeoutMs = options.noProgressTimeoutMs ?? ARCHIVE_UPLOAD_NO_PROGRESS_MS;
  if (maximumBytes <= 0n || maximumBytes > P0_ARCHIVE_UPLOAD_LIMIT) {
    throw new RangeError('archive upload maximum must be within the fixed P0 limit');
  }
  if (!Number.isSafeInteger(noProgressTimeoutMs) || noProgressTimeoutMs < 1) {
    throw new RangeError('archive upload no-progress timeout must be a positive safe integer');
  }
  if (!/^[0-9a-f]{32}$/u.test(options.operationId)) {
    throw new TypeError('archive upload operation ID must be 32 lowercase hexadecimal digits');
  }
  const declaredBytes = parseContentLength(options.contentLength, maximumBytes);
  const directory = await realpath(options.scratchDirectory);
  const partialName = `.${options.operationId}.upload.partial`;
  const readyName = `.${options.operationId}.upload.ready`;
  const partialPath = join(directory, partialName);
  const readyPath = join(directory, readyName);
  let file: Awaited<ReturnType<typeof open>> | undefined;
  let receivedBytes = 0n;
  const iterator = options.source[Symbol.asyncIterator]();
  try {
    file = await open(partialPath, 'wx');
    for (;;) {
      const next = await nextChunk(iterator, noProgressTimeoutMs);
      if (next.done) break;
      const chunk = next.value;
      if (!(chunk instanceof Uint8Array)) throw new TypeError('archive upload emitted a non-binary chunk');
      receivedBytes += BigInt(chunk.byteLength);
      if (receivedBytes > maximumBytes) {
        throw new ArchiveUploadError(
          'ARCHIVE_TOO_LARGE',
          `archive upload exceeded the ${maximumBytes}-byte limit`
        );
      }
      let written = 0;
      while (written < chunk.byteLength) {
        const result = await file.write(chunk, written, chunk.byteLength - written);
        if (result.bytesWritten === 0) throw new Error('archive upload spool stopped making write progress');
        written += result.bytesWritten;
      }
    }
    if (receivedBytes === 0n) throw new ArchiveUploadError('EMPTY_ARCHIVE', 'archive upload is empty');
    if (declaredBytes !== undefined && receivedBytes !== declaredBytes) {
      throw new ArchiveUploadError(
        'LENGTH_MISMATCH',
        `archive received ${receivedBytes} bytes but Content-Length declared ${declaredBytes}`
      );
    }
    await file.sync();
    await file.close();
    file = undefined;
    await rename(partialPath, readyPath);
    return { operationId: options.operationId, relativeFilename: readyName,
      readyPath, storedByteCount: u64Hex(receivedBytes) };
  } catch (error) {
    void iterator.return?.();
    await file?.close().catch(() => {});
    await unlink(partialPath).catch(() => {});
    await unlink(readyPath).catch(() => {});
    throw error;
  }
}
