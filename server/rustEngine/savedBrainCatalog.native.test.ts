import Database from 'better-sqlite3';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
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
