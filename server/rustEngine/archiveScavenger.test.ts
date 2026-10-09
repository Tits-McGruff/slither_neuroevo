import {
  mkdirSync,
  mkdtempSync,
  readdirSync,
  rmSync,
  symlinkSync,
  utimesSync,
  writeFileSync
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import {
  ARCHIVE_ARTIFACT_GRACE_MS,
  scavengeStaleArchiveArtifacts
} from './archiveScavenger.ts';

/** Disposable roots removed after each focused cleanup test. */
const roots: string[] = [];

/** Create one isolated managed root. */
function fixtureDirectory(): string {
  const root = mkdtempSync(join(tmpdir(), 'slither-archive-scavenger-'));
  roots.push(root);
  return root;
}

/** Write one file and place its modification time at an exact millisecond. */
function writeAt(directory: string, name: string, mtimeMs: number): void {
  const path = join(directory, name);
  writeFileSync(path, name);
  const timestamp = new Date(mtimeMs);
  utimesSync(path, timestamp, timestamp);
}

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe('archive startup scavenging', () => {
  it('reclaims an abandoned private import stage while preserving shared finals and uncertain stages', async () => {
    const directory = fixtureDirectory();
    const now = Date.UTC(2026, 8, 13, 12);
    const old = now - ARCHIVE_ARTIFACT_GRACE_MS - 1;
    const checkpoint = `${'ab'.repeat(32)}.checkpoint-v3`;
    const elite = `${'cd'.repeat(32)}.hof-weights-v1`;
    writeAt(directory, checkpoint, old);
    writeAt(directory, elite, old);
    for (const [index, condition] of ['stale', 'fresh-directory', 'fresh-child', 'unknown', 'nested'].entries()) {
      const operation = String(index + 1).repeat(32);
      const path = join(directory, `.${operation}.import-validation`);
      mkdirSync(path);
      writeAt(path, checkpoint, old);
      writeAt(path, elite, condition === 'fresh-child' ? now : old);
      writeAt(path, `.${operation}.import-inventory-v1`, old);
      if (condition === 'unknown') writeAt(path, 'owner-notes.txt', old);
      if (condition === 'nested') mkdirSync(join(path, 'nested'));
      const timestamp = new Date(condition === 'fresh-directory' ? now : old);
      utimesSync(path, timestamp, timestamp);
    }
    const outside = join(directory, 'owner-directory');
    mkdirSync(outside);
    writeAt(outside, checkpoint, old);
    symlinkSync(outside, join(directory, `.${'6'.repeat(32)}.import-validation`),
      process.platform === 'win32' ? 'junction' : 'dir');
    const before = readdirSync(directory).sort();
    await expect(scavengeStaleArchiveArtifacts(directory, now)).resolves.toMatchObject({ examined: 3, removed: 3 });
    expect(readdirSync(directory).sort()).toEqual(before.filter(name => name !== `.${'1'.repeat(32)}.import-validation`));
    for (let index = 2; index <= 5; index++) {
      const operation = String(index).repeat(32);
      expect(readdirSync(join(directory, `.${operation}.import-validation`)))
        .toEqual(expect.arrayContaining([checkpoint, elite, `.${operation}.import-inventory-v1`]));
    }
    expect(readdirSync(outside)).toEqual([checkpoint]);
  });

  it('removes only exact old writer scratch names', async () => {
    const directory = fixtureDirectory();
    const operation = '12'.repeat(16);
    const now = Date.UTC(2026, 8, 13, 12);
    const old = now - ARCHIVE_ARTIFACT_GRACE_MS - 1;
    const stale = [
      `checkpoint-v3-${operation}.partial`,
      `.${operation}.weights.codec.partial`,
      `.${operation}.recurrent.codec.partial`,
      `.${operation}.hof-weights.codec.partial`,
      `.${operation}.hof-weights.partial`,
      `.${operation}.export-hof-weights.partial`,
      `.${operation}.export-inventory-v1`,
      `.${operation}.export-inventory-v1.partial`,
      `.${operation}.import-inventory-v1`,
      `.${operation}.import-inventory-v1.partial`,
      `.${operation}.import-hof-weights.encoded.partial`,
      `.${operation}.import-hof-weights.raw.partial`,
      `.${operation}.slither-save.partial`,
      `.${operation}.slither-save.ready`,
      `.${operation}.upload.partial`,
      `.${operation}.upload.ready`
    ];
    for (const name of stale) writeAt(directory, name, old);

    const fresh = `.${'34'.repeat(16)}.upload.partial`;
    const completed = `${'ab'.repeat(32)}.checkpoint-v3`;
    const unknown = `.${operation}.owner-notes.ready`;
    writeAt(directory, fresh, now);
    writeAt(directory, completed, old);
    writeAt(directory, unknown, old);
    mkdirSync(join(directory, `.${'56'.repeat(16)}.upload.ready`));

    const removedBytes = stale.reduce((total, name) => total + BigInt(name.length), 0n);
    await expect(scavengeStaleArchiveArtifacts(directory, now)).resolves.toEqual({
      examined: stale.length,
      removed: stale.length,
      removedBytes
    });
    expect(readdirSync(directory).sort()).toEqual([completed, fresh, unknown,
      `.${'56'.repeat(16)}.upload.ready`].sort());
  });

  it('keeps an exact artifact through the complete 24-hour grace', async () => {
    const directory = fixtureDirectory();
    const now = Date.UTC(2026, 8, 13, 12);
    const name = `.${'78'.repeat(16)}.slither-save.ready`;
    writeAt(directory, name, now - ARCHIVE_ARTIFACT_GRACE_MS + 1);

    await expect(scavengeStaleArchiveArtifacts(directory, now)).resolves.toEqual({
      examined: 0,
      removed: 0,
      removedBytes: 0n
    });
    expect(readdirSync(directory)).toEqual([name]);
  });
});
