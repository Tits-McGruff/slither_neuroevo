import { readFileSync, mkdtempSync, readdirSync, realpathSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { spoolArchiveUpload } from './archiveUpload.ts';
import type { CheckpointOperationId } from './checkpointPersistenceProtocol.ts';

/** Disposable roots removed after each upload test. */
const roots: string[] = [];

/** Create one empty server-controlled upload directory. */
function fixtureDirectory(): string {
  const root = mkdtempSync(join(tmpdir(), 'slither-archive-upload-'));
  roots.push(root);
  return root;
}

/** Yield binary chunks without providing a second complete body buffer. */
async function* chunks(...values: string[]): AsyncIterable<Uint8Array> {
  for (const value of values) yield Buffer.from(value);
}

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe('raw archive upload spooling', () => {
  it('syncs one exact streamed body to an operation-local ready file', async () => {
    const directory = fixtureDirectory();
    const operationId = '12'.repeat(16) as CheckpointOperationId;
    const result = await spoolArchiveUpload({ source: chunks('save-', 'bytes'), contentLength: '10',
      scratchDirectory: directory, operationId, maximumBytes: 64n });
    expect(result).toEqual({ operationId, relativeFilename: `.${operationId}.upload.ready`,
      readyPath: join(realpathSync(directory), `.${operationId}.upload.ready`), storedByteCount: '000000000000000a' });
    expect(readFileSync(result.readyPath).toString()).toBe('save-bytes');
    expect(readdirSync(directory)).toEqual([`.${operationId}.upload.ready`]);
  });

  it('cleans partial files after length and streaming-limit failures', async () => {
    const directory = fixtureDirectory();
    await expect(spoolArchiveUpload({ source: chunks('short'), contentLength: '6',
      scratchDirectory: directory, operationId: '34'.repeat(16) as CheckpointOperationId,
      maximumBytes: 64n })).rejects.toMatchObject({ code: 'LENGTH_MISMATCH' });
    await expect(spoolArchiveUpload({ source: chunks('1234', '5678'), contentLength: undefined,
      scratchDirectory: directory, operationId: '56'.repeat(16) as CheckpointOperationId,
      maximumBytes: 7n })).rejects.toMatchObject({ code: 'ARCHIVE_TOO_LARGE' });
    expect(readdirSync(directory)).toEqual([]);
  });
});
