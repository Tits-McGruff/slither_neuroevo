import { createHash } from 'node:crypto';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { expect, it, vi } from 'vitest';
import { BrainLibraryClient } from '../../src/net/brainLibrary.ts';
import { largeBrainFixture } from '../test/largeBrainFixture.ts';
import { CheckpointPersistenceClient } from './checkpointPersistenceClient.ts';
import { DistillationSpoolClient } from './distillationSpoolClient.ts';

it('admits, saves, lists, exports, imports and reopens the production large brain with real native workers', async () => {
  const root = mkdtempSync(join(tmpdir(), 'slither-large-brain-'));
  const managed = join(root, 'managed'); mkdirSync(managed);
  const databasePath = join(root, 'metadata.sqlite');
  let persistence = new CheckpointPersistenceClient({databasePath, managedRootPath: managed});
  let spool = new DistillationSpoolClient(join(root, 'diagnostic'));
  try {
    const bytes = largeBrainFixture();
    const id = createHash('sha256').update(bytes).digest('hex');
    const candidate = await spool.ingestCandidate(id, bytes);
    expect(candidate.descriptor).toMatchObject({parameterCount: 1_746_104, inputSize: 264, stateSize: 792,
      sensorSha256: 'fa525c20998440a3eec3a3f1ed944760ed1c26650d8819608efa0cd44f83893e', lifeStateRule: 'reset'});
    expect(await persistence.savedBrainCatalog({action: 'list', after: null, limit: 25})).toMatchObject({page: {entries: []}});
    const download = await spool.downloadCandidate(id);
    const input = join(root, 'candidate.slither-brain'); writeFileSync(input, download.bytes);
    const saved = await persistence.savedBrainCatalog({action: 'save', path: input, name: 'production large'});
    if (saved.action !== 'save') throw new Error('wrong save response');
    const record = saved.record;
    const exported = readFileSync(join(managed, record.artifactRelativeFilename));
    expect(exported).toEqual(bytes);
    expect(BigInt('0x' + record.weights.decodedByteCount)).toBe(6_984_416n);
    const page = await persistence.savedBrainCatalog({action: 'list', after: null, limit: 25});
    if (page.action !== 'list') throw new Error('wrong list response');
    vi.stubGlobal('fetch', vi.fn(async () => new Response(JSON.stringify({schemaVersion: 1,
      processEpoch: '12'.repeat(16), deploymentSequence: '0000000000000000',
      scope: {expectedRunId: 'large', expectedConfigRevision: '0000000000000001', expectedWorldEpoch: '0000000000000001'}, page: page.page}))));
    expect((await new BrainLibraryClient(() => 'http://localhost').list()).page.entries[0]?.artifact.parameterCount).toBe(1_746_104);
    await persistence.close(); await spool.close();
    persistence = new CheckpointPersistenceClient({databasePath, managedRootPath: managed, existingOnly: true});
    spool = new DistillationSpoolClient(join(root, 'diagnostic'));
    expect(await spool.candidate(id)).toEqual(candidate);
    expect(await persistence.savedBrainCatalog({action: 'read', brainId: id})).toEqual({action: 'read', record});
    // Original portable bytes go through native admission again after explicit deletion/import.
    await persistence.savedBrainCatalog({action: 'delete', brainId: id});
    writeFileSync(input, exported);
    const imported = await persistence.savedBrainCatalog({action: 'save', path: input, name: 'reimported'});
    expect(imported).toMatchObject({action: 'save', record: {brainId: id, artifact: record.artifact, weights: record.weights}});
    const damaged = Buffer.from(bytes); damaged[damaged.length - 1] = damaged[damaged.length - 1]! ^ 1;
    await expect(spool.ingestCandidate(createHash('sha256').update(damaged).digest('hex'), damaged)).rejects.toThrow(/digest/u);
  } finally {
    vi.unstubAllGlobals(); await spool.close(); await persistence.close();
    rmSync(root, {recursive: true, force: true});
  }
}, 30000);
