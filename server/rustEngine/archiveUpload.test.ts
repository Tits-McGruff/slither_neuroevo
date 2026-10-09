import { readFileSync, mkdtempSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { realpath } from 'node:fs/promises';
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
  it.each(['partial', 'ready'] as const)('preserves a pre-existing upload %s file when rejecting its operation ID', async suffix => {
    const directory = fixtureDirectory();
    const operationId = 'ab'.repeat(16) as CheckpointOperationId;
    const filename = `.${operationId}.upload.${suffix}`;
    writeFileSync(join(directory, filename), 'retained upload bytes');
    await expect(spoolArchiveUpload({ source: chunks('different bytes'), contentLength: '15',
      scratchDirectory: directory, operationId, maximumBytes: 64n })).rejects.toThrow();
    expect(readdirSync(directory)).toEqual([filename]);
    expect(readFileSync(join(directory, filename)).toString()).toBe('retained upload bytes');
  });

  it('rejects a concurrent duplicate operation without removing the first writer spool', async () => {
    const directory = fixtureDirectory();
    const operationId = 'cd'.repeat(16) as CheckpointOperationId;
    const paused = Promise.withResolvers<void>();
    const proceed = Promise.withResolvers<void>();
    /** Hold a real writer after its first bytes reached disk. */
    async function* firstBody(): AsyncIterable<Uint8Array> {
      yield Buffer.from('first-');
      paused.resolve();
      await proceed.promise;
      yield Buffer.from('body');
    }
    const first = spoolArchiveUpload({ source: firstBody(), contentLength: '10',
      scratchDirectory: directory, operationId, maximumBytes: 64n });
    try {
      await paused.promise;
      const partialName = `.${operationId}.upload.partial`;
      expect(readFileSync(join(directory, partialName)).toString()).toBe('first-');
      await expect(spoolArchiveUpload({ source: chunks('second body'), contentLength: '11',
        scratchDirectory: directory, operationId, maximumBytes: 64n })).rejects.toMatchObject({ code: 'EEXIST' });
      expect(readdirSync(directory)).toEqual([partialName]);
      expect(readFileSync(join(directory, partialName)).toString()).toBe('first-');
      proceed.resolve();
      const result = await first;
      expect(readFileSync(result.readyPath).toString()).toBe('first-body');
      expect(readdirSync(directory)).toEqual([result.relativeFilename]);
    } finally {
      proceed.resolve();
      await first.catch(() => {});
    }
  });

  it('syncs one exact streamed body to an operation-local ready file', async () => {
    const directory = fixtureDirectory();
    const operationId = '12'.repeat(16) as CheckpointOperationId;
    const canonicalDirectory = await realpath(directory);
    const result = await spoolArchiveUpload({ source: chunks('save-', 'bytes'), contentLength: '10',
      scratchDirectory: directory, operationId, maximumBytes: 64n });
    expect(result).toEqual({ operationId, relativeFilename: `.${operationId}.upload.ready`,
      readyPath: join(canonicalDirectory, `.${operationId}.upload.ready`), storedByteCount: '000000000000000a' });
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

  it('terminates and cleans an upload that stops producing chunks', async () => {
    const directory = fixtureDirectory();
    /** Produce one chunk, then model a connected peer that never progresses. */
    async function* stalled(): AsyncIterable<Uint8Array> {
      yield Buffer.from('started');
      await new Promise<never>(() => {});
    }
    await expect(spoolArchiveUpload({ source: stalled(), contentLength: undefined,
      scratchDirectory: directory, operationId: '78'.repeat(16) as CheckpointOperationId,
      maximumBytes: 64n, noProgressTimeoutMs: 10 })).rejects.toMatchObject({ code: 'NO_PROGRESS' });
    expect(readdirSync(directory)).toEqual([]);
  });

  it('does not count empty chunks as upload progress', async () => {
    const directory = fixtureDirectory();
    /** A connected source may keep yielding without delivering another byte. */
    async function* emptyChunks(): AsyncIterable<Uint8Array> {
      yield Buffer.from('started');
      for (let index = 0; index < 100; index++) {
        await new Promise<void>(done => setTimeout(done, 2));
        yield Buffer.alloc(0);
      }
    }
    await expect(spoolArchiveUpload({ source: emptyChunks(), contentLength: undefined,
      scratchDirectory: directory, operationId: '9a'.repeat(16) as CheckpointOperationId,
      maximumBytes: 64n, noProgressTimeoutMs: 20 })).rejects.toMatchObject({ code: 'NO_PROGRESS' });
    expect(readdirSync(directory)).toEqual([]);
  });
});
