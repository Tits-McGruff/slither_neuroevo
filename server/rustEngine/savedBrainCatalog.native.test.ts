import Database from 'better-sqlite3';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { expect, it } from 'vitest';
import { CheckpointPersistenceClient } from './checkpointPersistenceClient.ts';

/** Actual production native publisher, metadata worker and shared reference GC. */
it('saves native-admitted originals across restart and preserves both HoF/library and lease references', async () => {
  const root = mkdtempSync(join(tmpdir(), 'slither-library-native-'));
  const managed = join(root, 'managed');
  mkdirSync(managed);
  const databasePath = join(root, 'metadata.sqlite');
  const source = resolve('native/fixtures/portable-brain-v1.slither-brain');
  let client = new CheckpointPersistenceClient({databasePath, managedRootPath: managed});
  let metadata: Database.Database | undefined;
  try {
    const saved = await client.savedBrainCatalog({action: 'save', path: source, name: 'native fixture'});
    if (saved.action !== 'save') throw new Error('wrong save response');
    const record = saved.record;
    const artifactPath = join(managed, record.artifactRelativeFilename);
    const weightsPath = join(managed, record.weights.relativeFilename);
    expect(readFileSync(artifactPath)).toEqual(readFileSync(source));
    expect(record.artifact.parameterCount).toBe(1962);
    expect(record.provenance.sourceKind).toBe('manual');
    expect(record.graphManifest['sensorSha256']).toBe(record.artifact.sensorSha256);
    await client.close();
    client = new CheckpointPersistenceClient({databasePath, managedRootPath: managed, existingOnly: true});
    expect(await client.savedBrainCatalog({action: 'read', brainId: record.brainId})).toEqual({action: 'read', record});
    expect(await client.savedBrainCatalog({action: 'save', path: source, name: 'retry'})).toEqual(saved);
    metadata = new Database(databasePath);
    /** Exercise a second real metadata consumer of the native-published object. */
    const insertHof = (): void => {
      const blob = Buffer.alloc(56); blob.writeBigUInt64LE(1n);
      metadata!.prepare(`INSERT INTO rust_hall_of_fame_v1
        (run_id,generation_hex,checkpoint_id,record_version,record_blob,weights_sha256,
          genome_sha256,fitness_value,pinned,weight_state,created_at_ms)
        VALUES(?,?,NULL,1,?,?,?,0,0,'selected',?)`).run('metadata-shared-reference', '0000000000000001',
          blob, record.weights.logicalSha256, 'b'.repeat(64), Date.now());
    };
    insertHof();
    metadata.prepare("DELETE FROM rust_hall_of_fame_v1 WHERE run_id='metadata-shared-reference'").run();
    expect((await client.reclaimManagedOrphans()).completed).toBe(true);
    expect(existsSync(artifactPath)).toBe(true);
    expect(existsSync(weightsPath)).toBe(true);
    insertHof();
    const leaseId = '13'.repeat(16);
    expect((await client.savedBrainCatalog({action: 'acquire', brainId: record.brainId, leaseId})).action).toBe('acquire');
    expect(await client.savedBrainCatalog({action: 'delete', brainId: record.brainId})).toEqual({action: 'delete', brainId: record.brainId, deleted: true});
    expect((await client.reclaimManagedOrphans()).completed).toBe(false);
    expect(existsSync(artifactPath)).toBe(true);
    await client.savedBrainCatalog({action: 'release', leaseId});
    expect(existsSync(artifactPath)).toBe(false);
    expect(existsSync(weightsPath)).toBe(true);
    metadata.prepare("DELETE FROM rust_hall_of_fame_v1 WHERE run_id='metadata-shared-reference'").run();
    writeFileSync(join(managed, 'owner-note.txt'), 'preserve');
    writeFileSync(artifactPath, readFileSync(source)); // Interrupted publication, no catalog reference.
    const cleaned = await client.reclaimManagedOrphans();
    expect(cleaned.completed).toBe(true);
    expect(cleaned.deletedSavedBrainCount).toBe('0000000000000001');
    expect(existsSync(artifactPath)).toBe(false);
    expect(existsSync(weightsPath)).toBe(false);
    expect(readFileSync(join(managed, 'owner-note.txt'), 'utf8')).toBe('preserve');
    expect(await client.savedBrainCatalog({action: 'list', after: null, limit: 25})).toEqual({action: 'list', page: {entries: [], next: null}});
  } finally {
    metadata?.close();
    await client.close();
    rmSync(root, {recursive: true, force: true});
  }
}, 15000);

/** Remove only the exact disposable archive-lease fixture root after its worker closes. */
function cleanupArchiveFixture(root: string): void {
  if (dirname(resolve(root)) !== resolve(tmpdir()) || !root.includes('slither-library-archive-native-')) throw new Error('unsafe archive fixture cleanup');
  rmSync(root, {recursive: true, force: true});
}

/** Real heterogeneous portable sources prove archive batch pins participate in shared GC. */
it('pins heterogeneous native originals in one archive lease through catalog deletion', async () => {
  const root = mkdtempSync(join(tmpdir(), 'slither-library-archive-native-'));
  const managed = join(root, 'managed'); mkdirSync(managed);
  const client = new CheckpointPersistenceClient({databasePath: join(root, 'metadata.sqlite'), managedRootPath: managed});
  try {
    const records = [];
    for (const filename of ['portable-brain-v1.slither-brain', 'portable-dense-basic8-v1.slither-brain']) {
      const saved = await client.savedBrainCatalog({action: 'save', path: resolve('native/fixtures', filename), name: filename});
      if (saved.action !== 'save') throw new Error('save response mismatch');
      records.push(saved.record);
    }
    records.sort((a, b) => a.brainId.localeCompare(b.brainId));
    const brainIds = records.map(record => record.brainId); const leaseId = 'ef'.repeat(16);
    const lease = await client.savedBrainCatalog({action: 'acquireArchive', brainIds, leaseId});
    expect(lease).toMatchObject({action: 'acquireArchive', entries: brainIds.map(brainId => ({brainId}))});
    expect(await client.savedBrainCatalog({action: 'acquireArchive', brainIds, leaseId})).toEqual(lease);
    const singleLease = 'de'.repeat(16);
    await client.savedBrainCatalog({action: 'acquire', brainId: brainIds[0]!, leaseId: singleLease});
    for (const brainId of brainIds) await client.savedBrainCatalog({action: 'delete', brainId});
    expect((await client.reclaimManagedOrphans()).completed).toBe(false);
    for (const record of records) {
      expect(existsSync(join(managed, record.artifactRelativeFilename))).toBe(true);
      expect(existsSync(join(managed, record.weights.relativeFilename))).toBe(true);
    }
    await client.savedBrainCatalog({action: 'release', leaseId});
    expect((await client.reclaimManagedOrphans()).completed).toBe(false);
    expect(existsSync(join(managed, records[0]!.artifactRelativeFilename))).toBe(true);
    expect(existsSync(join(managed, records[1]!.artifactRelativeFilename))).toBe(false);
    await client.savedBrainCatalog({action: 'release', leaseId: singleLease});
    expect((await client.reclaimManagedOrphans()).completed).toBe(true);
    for (const record of records) {
      expect(existsSync(join(managed, record.artifactRelativeFilename))).toBe(false);
      expect(existsSync(join(managed, record.weights.relativeFilename))).toBe(false);
    }
  } finally { await client.close(); cleanupArchiveFixture(root); }
}, 15000);
