import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import {
  ARCHIVE_TEMP_QUOTA_BYTES,
  DiskAdmissionError,
  OPERATING_DISK_RESERVE_BYTES,
  SQLITE_WAL_ALLOWANCE_BYTES,
  evaluateDiskAdmission,
  inspectArchiveTempBytes
} from './diskAdmission.ts';

/** Disposable directories removed after each disk-admission test. */
const roots: string[] = [];

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe('managed disk admission', () => {
  it('charges every formula term and admits an exact-fit operation', () => {
    const request = {
      operation: 'export' as const,
      sourceSpoolBytes: 11n,
      candidateSpoolBytes: 22n,
      finalManagedBytes: 33n
    };
    const required = 7n + 11n + 22n + 33n + SQLITE_WAL_ALLOWANCE_BYTES +
      OPERATING_DISK_RESERVE_BYTES;
    expect(evaluateDiskAdmission(request, 7n, required)).toEqual({
      ...request,
      existingTempBytes: 7n,
      freeBytes: required,
      totalTempBytes: 40n,
      requiredFreeBytes: required
    });
  });

  it('reports the complete calculation when free disk is short', () => {
    const request = {
      operation: 'checkpoint' as const,
      sourceSpoolBytes: 0n,
      candidateSpoolBytes: 0n,
      finalManagedBytes: 512n
    };
    expect(() => evaluateDiskAdmission(request, 9n, 10n)).toThrow(DiskAdmissionError);
    try {
      evaluateDiskAdmission(request, 9n, 10n);
    } catch (error) {
      expect(error).toMatchObject({ code: 'FREE_DISK' });
      expect((error as Error).message).toContain('free=10, existingTemp=9');
      expect((error as Error).message).toContain('finalManaged=512');
    }
  });

  it('rejects the temporary quota before free-space arithmetic', () => {
    expect(() => evaluateDiskAdmission({
      operation: 'import',
      sourceSpoolBytes: ARCHIVE_TEMP_QUOTA_BYTES,
      candidateSpoolBytes: 1n,
      finalManagedBytes: 0n
    }, 0n, ARCHIVE_TEMP_QUOTA_BYTES * 2n)).toThrow(/above the .*quota/);
  });

  it('counts only recognized regular private work files', async () => {
    const root = mkdtempSync(join(tmpdir(), 'slither-disk-admission-'));
    roots.push(root);
    const operation = '12'.repeat(16);
    writeFileSync(join(root, `.${operation}.upload.ready`), Buffer.alloc(7));
    writeFileSync(join(root, `.${operation}.slither-save.partial`), Buffer.alloc(11));
    writeFileSync(join(root, `${'ab'.repeat(32)}.checkpoint-v3`), Buffer.alloc(13));
    writeFileSync(join(root, 'owner-notes.txt'), Buffer.alloc(17));
    mkdirSync(join(root, `.${operation}.weights.codec.partial`));
    await expect(inspectArchiveTempBytes(root)).resolves.toBe(18n);
  });
});
