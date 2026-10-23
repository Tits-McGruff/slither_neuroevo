/** Reject malformed or changed external capacity fixtures before any native upload. */
import { expect, it } from 'vitest';
import { createHash } from 'node:crypto';
import { mkdtemp, mkdir, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import type { SavedBrainSummaryV1 } from '../../server/protocol.ts';
import { loadLiveLibraryFixture, readLiveLibraryArtifact, readLiveCapacityCatalog } from './liveCapacityLibrary.ts';

/** One explicit synthetic metadata index; portable bytes remain opaque to these boundary tests. */
function index(): {schemaVersion: number; records: {brainId: string; weightsSha256: string; relativeFilename: string;
  name: string; artifactByteCount: string; parameterCount: number}[]} {
  return {schemaVersion: 1, records: Array.from({length: 101}, (_, ordinal) => ({
    brainId: createHash('sha256').update(Buffer.alloc(88, ordinal)).digest('hex'),
    weightsSha256: ordinal.toString(16).padStart(64, '0'),
    relativeFilename: 'library-inputs/' + String(ordinal).padStart(3, '0') + '.slither-brain',
    name: 'Opaque boundary fixture ' + ordinal, artifactByteCount: '0000000000000058', parameterCount: 494306,
  }))};
}

/** Confine temporary cleanup to this test's exclusive direct child of the OS scratch root. */
async function cleanup(root: string): Promise<void> {
  if (dirname(resolve(root)) !== resolve(tmpdir()) || !root.startsWith(join(tmpdir(), 'slither-live-library-'))) throw new Error('unsafe library fixture cleanup');
  await rm(root, {recursive: true, force: true});
}

it('rejects missing/aliased/duplicate fixture ownership and unbounded original lengths', async () => {
  const root = await mkdtemp(join(tmpdir(), 'slither-live-library-'));
  try {
    await mkdir(join(root, 'library-inputs')); const file = join(root, 'library-inputs/index.json');
    for (const mutate of [
      (v: ReturnType<typeof index>) => {v.records.pop();},
      (v: ReturnType<typeof index>) => {v.records[0]!.relativeFilename = '../other.slither-brain';},
      (v: ReturnType<typeof index>) => {v.records[1]!.brainId = v.records[0]!.brainId;},
      (v: ReturnType<typeof index>) => {v.records[1]!.weightsSha256 = v.records[0]!.weightsSha256;},
      (v: ReturnType<typeof index>) => {v.records[0]!.artifactByteCount = '0000000000800001';},
      (v: ReturnType<typeof index>) => {v.records[0]!.parameterCount = 25042;},
    ]) {
      const value = index(); mutate(value); await writeFile(file, JSON.stringify(value));
      await expect(loadLiveLibraryFixture(file)).rejects.toThrow();
    }
  } finally {await cleanup(root);}
});

it('rechecks exact opaque file length and hash immediately before upload', async () => {
  const root = await mkdtemp(join(tmpdir(), 'slither-live-library-'));
  try {
    await mkdir(join(root, 'library-inputs'));
    const file = join(root, 'library-inputs/index.json'); await writeFile(file, JSON.stringify(index()));
    const original = join(root, 'library-inputs/000.slither-brain');
    await writeFile(original, Buffer.alloc(88));
    const fixture = await loadLiveLibraryFixture(file);
    expect(await readLiveLibraryArtifact(fixture, 0)).toEqual(Buffer.alloc(88));
    await writeFile(original, Buffer.alloc(88, 1));
    await expect(readLiveLibraryArtifact(fixture, 0)).rejects.toThrow(/original changed/u);
    await writeFile(original, Buffer.alloc(89));
    await expect(readLiveLibraryArtifact(fixture, 0)).rejects.toThrow();
  } finally {await cleanup(root);}
});

it('requires four distinct ordered full metadata pages with an exact terminal cursor', async () => {
  const records = index().records.slice(0, 100).sort((a, b) => a.brainId.localeCompare(b.brainId))
    .map(record => ({...record, artifact: {parameterCount: 494306, inputSize: 504, sourceKind: 'manual'}})) as unknown as SavedBrainSummaryV1[];
  /** Present opaque metadata only; native wire decoding is exercised by the real live gate. */
  const page = async (after: string | null) => {
    const start = after ? records.findIndex(record => record.brainId === after) + 1 : 0;
    return {entries: records.slice(start, start + 25), next: start < 75 ? records[start + 24]!.brainId : null};
  };
  const receipt = await readLiveCapacityCatalog(page);
  expect(receipt).toMatchObject({pages: 4, recordCount: 100, originalBytes: '8800'});
  await expect(readLiveCapacityCatalog(async after => ({...await page(after), next: null}))).rejects.toThrow();
  await expect(readLiveCapacityCatalog(async after => ({...await page(after), entries: records.slice(0, 25)}))).rejects.toThrow();
});
