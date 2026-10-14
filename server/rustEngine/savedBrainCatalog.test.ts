import { parseSavedBrainOperation, parseSavedBrainResult } from './savedBrainProtocol.ts';
import Database from 'better-sqlite3';
import { createHash } from 'node:crypto';
import { existsSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { basename, join } from 'node:path';
import { afterEach, expect, it } from 'vitest';
import { SavedBrainCatalog, parseSavedBrainRecord, type SavedBrainNativeCodec } from './savedBrainCatalog.ts';
import type { BrainArtifactDescriptor } from '../../src/protocol/brainArtifacts.ts';

/** Disposable metadata fixtures; they make no neural codec/execution claim. */
const fixtures: Array<{root: string; db: Database.Database}> = [];
/** Close every isolated SQLite connection before deleting its task-owned fixture. */
afterEach(() => { for (const fixture of fixtures.splice(0)) {fixture.db.close(); rmSync(fixture.root, {recursive: true, force: true});} });

/** Native-shaped metadata placeholders exercise only catalog transactions and leases. */
function descriptor(number: number, weight = 1): BrainArtifactDescriptor {
  return { schemaVersion: 1, artifactSha256: number.toString(16).padStart(64, '0'),
    weightsSha256: weight.toString(16).padStart(64, '0'), architectureKey: `metadata-graph-${number}`,
    layoutSha256: 'a'.repeat(64), sensorSha256: createHash('sha256').update('slither.sensor-contract.v1\0').update('{}').digest('hex'),
    inputSize: number % 2 + 1, parameterCount: 1, stateSize: 0, controlIntervalSeconds: 0.06,
    lifeStateRule: 'reset', sourceKind: number % 2 ? 'distilled' : 'imported' };
}

/** Construct real SQLite/opaque-file storage with an explicit metadata-only codec double. */
function fixture(quota = 2n * 1024n * 1024n * 1024n): {root: string; db: Database.Database; catalog: SavedBrainCatalog; published: BrainArtifactDescriptor[]; allowances: bigint[]} {
  const root = mkdtempSync(join(tmpdir(), 'slither-saved-brain-metadata-'));
  const db = new Database(join(root, 'metadata.sqlite'));
  db.pragma('foreign_keys=ON');
  db.exec(`CREATE TABLE rust_hall_of_fame_weights_v1 (logical_sha256 TEXT PRIMARY KEY,
    relative_filename TEXT,encoding TEXT,stored_byte_count_hex TEXT,decoded_byte_count_hex TEXT,
    weight_count_hex TEXT,created_at_ms INTEGER)`);
  const published: BrainArtifactDescriptor[] = [];
  const allowances: bigint[] = [];
  const codec: SavedBrainNativeCodec = {
    inspect: path => descriptor(path.endsWith('.slither-brain') ? Number(BigInt('0x' + basename(path).split('.')[0]!)) : Number(path)),
    publish(path, directory, _operation, expected, allowance, artifactCounted, weightsCounted) {
      const artifact = descriptor(Number(path));
      allowances.push(BigInt(`0x${allowance}`));
      expect(expected).toBe(artifact.artifactSha256);
      expect(artifactCounted).toBe(false);
      const needed = 100n + (weightsCounted ? 0n : 4n);
      if (BigInt(`0x${allowance}`) < needed) throw new Error('native stored-byte allowance');
      published.push(artifact);
      const artifactRelativeFilename = `${artifact.artifactSha256}.slither-brain`;
      const relativeFilename = `${artifact.weightsSha256}.hof-weights-v1`;
      writeFileSync(join(directory, artifactRelativeFilename), Buffer.alloc(100));
      if (!existsSync(join(directory, relativeFilename))) writeFileSync(join(directory, relativeFilename), Buffer.alloc(4));
      return {schemaVersion: 1, artifact, weights: {version: 1, logicalSha256: artifact.weightsSha256,
        relativeFilename, encoding: 'raw-f32le-v1', storedByteCount: '0000000000000004',
        decodedByteCount: '0000000000000004', weightCount: '0000000000000001'},
        artifactRelativeFilename, artifactByteCount: '0000000000000064',
        graphManifest: {schemaVersion: 1, architectureKey: artifact.architectureKey, layoutSha256: artifact.layoutSha256,
          sensorSha256: artifact.sensorSha256, sensorCanonicalJson: '{}', inputSize: artifact.inputSize,
          totalParameters: 1, totalStateSize: 0, deployment: {controlIntervalSeconds: 0.06, lifeStateRule: 'reset'}},
        provenance: {schemaVersion: 1, sourceKind: artifact.sourceKind, details: {teacher: 'baseline-v2', seed: '42', samples: 256}}};
    }
  };
  const catalog = new SavedBrainCatalog(db, root, codec, quota);
  fixtures.push({root, db});
  return {root, db, catalog, published, allowances};
}

it('retains heterogeneous records separately from HoF and shares one immutable weight row', () => {
  const {db, catalog, published} = fixture();
  const first = catalog.save('1', 'distilled example', '11'.repeat(16));
  const second = catalog.save('2', 'imported example', '22'.repeat(16));
  expect(first.artifact.architectureKey).not.toBe(second.artifact.architectureKey);
  expect(db.prepare('SELECT COUNT(*) AS n FROM rust_hall_of_fame_weights_v1').get()).toEqual({n: 1});
  expect(catalog.save('1', 'retry label', '33'.repeat(16))).toEqual(first);
  expect(published).toHaveLength(2);
  const page = catalog.list(null, 1);
  expect(page.entries[0]?.brainId).toBe(first.brainId);
  expect(page.entries[0]).not.toHaveProperty('graphManifest');
  expect(page.entries[0]?.artifact).not.toHaveProperty('architectureKey');
  expect(catalog.list(page.next, 1)).toMatchObject({entries: [{brainId: second.brainId}], next: null});
  expect(() => parseSavedBrainRecord({...first, artifact: {...first.artifact, sensorSha256: '0'.repeat(64)}})).toThrow(/mismatch/);
});

it('deleted catalog rows keep leased originals and weights until the exact lease is released', () => {
  const {root, db, catalog, allowances} = fixture();
  const record = catalog.save('1', 'leased', '11'.repeat(16));
  const lease = catalog.acquire(record.brainId, '22'.repeat(16));
  expect(catalog.acquire(record.brainId, '22'.repeat(16))).toBe(lease);
  expect(catalog.hasLeases).toBe(true);
  expect(catalog.delete(record.brainId)).toBe(true);
  expect(catalog.get(record.brainId)).toBeNull();
  expect(existsSync(join(root, record.artifactRelativeFilename))).toBe(true);
  expect(existsSync(join(root, record.weights.relativeFilename))).toBe(true);
  catalog.save('2', 'another record', '33'.repeat(16));
  expect(allowances.at(-1)).toBe(2n * 1024n * 1024n * 1024n - 104n);
  catalog.release('22'.repeat(16));
  expect(catalog.hasLeases).toBe(false);
  expect(existsSync(join(root, record.artifactRelativeFilename))).toBe(false);
  expect(db.prepare('SELECT COUNT(*) AS n FROM rust_hall_of_fame_weights_v1').get()).toEqual({n: 1});
  expect(() => catalog.release('22'.repeat(16))).toThrow(/not active/);
});

it('enforces the record quota before publication and admits exact retries at the quota', () => {
  const {catalog, published, db} = fixture();
  // This quota fixture exercises every catalog admission, not 100 disk syncs.
  // Nested catalog transactions still expose all preceding rows to the same connection.
  db.transaction(() => {
    for (let id = 1; id <= 100; id++) catalog.save(String(id), `brain ${id}`, id.toString(16).padStart(32, '0'));
  })();
  expect(() => catalog.save('101', 'overflow', 'ff'.repeat(16))).toThrow(/record quota/);
  expect(published).toHaveLength(100);
  expect(catalog.save('1', 'retry', 'ab'.repeat(16)).brainId).toBe(descriptor(1).artifactSha256);
});

it('charges shared physical bytes exactly at the bounded quota before further publication', () => {
  const {catalog, published} = fixture(204n);
  catalog.save('1', 'first', '11'.repeat(16));
  catalog.save('2', 'second shared weights', '22'.repeat(16));
  expect(() => catalog.save('3', 'overflow', '33'.repeat(16))).toThrow(/allowance/);
  expect(published).toHaveLength(2);
});

it('rejects inconsistent catalog references before quota admission or orphan traversal', () => {
  const {catalog, db} = fixture();
  const record = catalog.save('1', 'record', '11'.repeat(16));
  db.prepare('UPDATE rust_saved_brains_v1 SET artifact_byte_count_hex=? WHERE brain_id=?')
    .run('0000000000000065', record.brainId);
  expect(() => catalog.referencedOriginals()).toThrow(/binding mismatch/);
  expect(() => catalog.save('2', 'another', '22'.repeat(16))).toThrow(/binding mismatch/);
});

it('bounds metadata protocol pages and rejects mutated deployment or lease identities', () => {
  const {catalog} = fixture();
  const record = catalog.save('1', 'record', '11'.repeat(16));
  expect(parseSavedBrainResult({action: 'list', page: catalog.list(null, 1)})).toMatchObject({action: 'list'});
  expect(parseSavedBrainResult({action: 'read', record})).toEqual({action: 'read', record});
  expect(() => parseSavedBrainResult({action: 'acquire', record: null})).toThrow();
  expect(() => parseSavedBrainResult({action: 'read', record: {...record, artifact: {...record.artifact, controlIntervalSeconds: 0.04}}})).toThrow(/cadence/);
  expect(() => parseSavedBrainOperation({action: 'release', leaseId: '11'.repeat(16), extra: true})).toThrow();
  expect(() => parseSavedBrainRecord(new Uint8Array(1024))).toThrow();
  expect(() => parseSavedBrainOperation({action: 'save', path: 'bad\0path', name: 'brain'})).toThrow();
  expect(() => parseSavedBrainOperation({action: 'list', after: null, limit: 26})).toThrow();
  expect(() => parseSavedBrainResult({action: 'list', page: {entries: [], next: record.brainId}})).toThrow(/chronology/);
});

it('protects the complete retained graph and provenance metadata against silent SQLite edits', () => {
  const {catalog, db} = fixture();
  const record = catalog.save('1', 'record', '11'.repeat(16));
  db.prepare('UPDATE rust_saved_brains_v1 SET record_json=? WHERE brain_id=?')
    .run(JSON.stringify({...record, provenance: {...record.provenance, details: {seed: 'changed'}}}), record.brainId);
  expect(() => catalog.get(record.brainId)).toThrow(/metadata digest/);
  expect(() => catalog.referencedOriginals()).toThrow(/metadata digest/);
});
