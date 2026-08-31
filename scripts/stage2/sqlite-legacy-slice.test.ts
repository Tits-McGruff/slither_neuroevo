/** Focused contracts for the disposable Stage 2 SQLite legacy slice probe. */

import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import Database from 'better-sqlite3';
import { afterEach, describe, expect, it } from 'vitest';
import {
  assertBoundedSliceQueryShape,
  BLOB_KIND,
  createLegacySliceFixture,
  deterministicText,
  readLegacyColumnSlices,
  TEXT_KIND,
  verifyTemporaryRoot
} from './sqlite-legacy-slice.ts';

/** Temporary roots owned solely by this test suite. */
const temporaryRoots: string[] = [];

afterEach(() => {
  for (const root of temporaryRoots.splice(0)) {
    fs.rmSync(verifyTemporaryRoot(root), { recursive: true, force: true });
  }
});

/** Create a safely named test-owned root. */
function createTemporaryRoot(): string {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'slither-stage2-sqlite-slice-'));
  temporaryRoots.push(root);
  return root;
}

/** Run the public disposable CLI exactly as a user-facing evidence capture would. */
function runProbeCli(argumentsAfterScript: readonly string[]): ReturnType<typeof spawnSync> {
  const scriptPath = fileURLToPath(new URL('./sqlite-legacy-slice.ts', import.meta.url));
  const tsxCli = path.resolve('node_modules', 'tsx', 'dist', 'cli.mjs');
  return spawnSync(process.execPath, [tsxCli, scriptPath, ...argumentsAfterScript], {
    encoding: 'utf8',
    timeout: 30_000,
    maxBuffer: 1024 * 1024
  });
}

describe('Stage 2 SQLite legacy slice probe', () => {
  it('uses bounded SQLite substr(CAST(... AS BLOB)) query shapes', () => {
    expect(assertBoundedSliceQueryShape()).toEqual({
      chunkQueriesUseBoundedSubstrCast: true,
      chunkPathContainsNoFullColumnSelect: true
    });
  });

  it('builds exact-length valid JSON UTF-8 text with a chunk boundary inside an emoji', () => {
    const text = deterministicText(1024);
    const encoded = Buffer.from(text, 'utf8');
    const emoji = Buffer.from('😀', 'utf8');
    const emojiOffset = encoded.indexOf(emoji);
    expect(encoded).toHaveLength(1024);
    expect(JSON.parse(text)).toMatchObject({ format: 'stage2' });
    expect(emojiOffset).toBeGreaterThanOrEqual(0);
    expect(emojiOffset % 5 + emoji.length).toBeGreaterThan(5);
  });

  it('reconstructs UTF-8 JSON TEXT and BLOB values bit exactly across multibyte boundaries', () => {
    const root = createTemporaryRoot();
    const fixture = createLegacySliceFixture(path.join(root, 'fixture.db'), 16 * 1024 + 13);
    const database = new Database(fixture.databasePath, { readonly: true, fileMustExist: true });
    try {
      const textChunkBytes = 17;
      const textFixture = deterministicText(fixture.payloadBytes);
      const textBytes = Buffer.from(textFixture, 'utf8');
      const emoji = Buffer.from('😀', 'utf8');
      const emojiOffsets: number[] = [];
      for (let offset = textBytes.indexOf(emoji); offset >= 0; offset = textBytes.indexOf(emoji, offset + 1)) {
        emojiOffsets.push(offset);
      }
      const blobChunkBytes = 4096;
      const blob = readLegacyColumnSlices(database, BLOB_KIND, blobChunkBytes);
      const text = readLegacyColumnSlices(database, TEXT_KIND, textChunkBytes);
      expect(blob.logicalBytes).toBe(fixture.payloadBytes);
      expect(text.logicalBytes).toBe(fixture.payloadBytes);
      expect(blob.sha256).toBe(fixture.blobSha256);
      expect(text.sha256).toBe(fixture.textSha256);
      expect(fixture.sqliteEncoding.toUpperCase()).toBe('UTF-8');
      expect(blob.largestReturnedChunkBytes).toBeLessThanOrEqual(blobChunkBytes);
      expect(text.largestReturnedChunkBytes).toBeLessThanOrEqual(textChunkBytes);
      expect(blob.chunks).toBeGreaterThan(1);
      expect(text.chunks).toBeGreaterThan(1);
      expect(emojiOffsets.some(offset => offset % textChunkBytes + emoji.length > textChunkBytes)).toBe(true);
    } finally {
      database.close();
    }
  });

  it('detects a missing legacy row instead of truncating its reconstruction', () => {
    const root = createTemporaryRoot();
    const fixture = createLegacySliceFixture(path.join(root, 'fixture.db'), 1024);
    const database = new Database(fixture.databasePath);
    try {
      database.prepare('DELETE FROM legacy_values WHERE kind = ?').run(BLOB_KIND);
      expect(() => readLegacyColumnSlices(database, BLOB_KIND, 64)).toThrow(
        'missing or invalid blob logical byte length'
      );
      const text = readLegacyColumnSlices(database, TEXT_KIND, 64);
      expect(text.sha256).toBe(fixture.textSha256);
      expect(createHash('sha256').update(Buffer.from('')).digest('hex')).not.toBe(text.sha256);
    } finally {
      database.close();
    }
  });

  it('spawns the isolated reader, records its conservative conclusion, and atomically refuses overwrite', () => {
    const root = createTemporaryRoot();
    const output = path.join(root, 'artifact.json');
    const first = runProbeCli([
      '--bytes', '16384',
      '--chunk-bytes', '65536',
      '--child-timeout-ms', '30000',
      '--output', output
    ]);
    expect(first.status).toBe(0);
    expect(first.error).toBeUndefined();
    const artifact = JSON.parse(fs.readFileSync(output, 'utf8')) as {
      source: { commit: string; dirty: boolean };
      fixture: { sqliteEncoding: string };
      readerProcess: { timeoutMs: number };
      result: {
        assertions: { sqliteEncodingIsUtf8: boolean; exactBlob: boolean; exactText: boolean };
        conclusion: { boundedNativeAllocationProved: boolean; productionQueryPathAuthorized: boolean };
      };
    };
    expect(artifact.source.commit).not.toBe('');
    expect(typeof artifact.source.dirty).toBe('boolean');
    expect(artifact.fixture.sqliteEncoding.toUpperCase()).toBe('UTF-8');
    expect(artifact.readerProcess.timeoutMs).toBe(30_000);
    expect(artifact.result.assertions).toMatchObject({
      sqliteEncodingIsUtf8: true,
      exactBlob: true,
      exactText: true
    });
    expect(artifact.result.conclusion).toEqual({
      boundedNativeAllocationProved: false,
      productionQueryPathAuthorized: false,
      requiredProductionDecision: expect.stringContaining('separately reviewed native incremental-BLOB')
    });

    const second = runProbeCli([
      '--bytes', '16384',
      '--chunk-bytes', '65536',
      '--output', output
    ]);
    expect(second.status).toBe(1);
    expect(second.stderr).toContain('Refusing to overwrite artifact');
  }, 45_000);
});
