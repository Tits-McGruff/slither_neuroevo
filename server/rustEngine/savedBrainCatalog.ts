import type Database from 'better-sqlite3';
import { lstatSync, realpathSync, statfsSync, unlinkSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { createHash } from 'node:crypto';
import type { SavedBrainPageV1, SavedBrainRecordV1, SavedBrainSummaryV1 } from '../protocol.ts';
import { parseBrainDescriptor } from './brainCandidateStore.ts';
import { parseManagedNeuralWeightsDescriptor } from './checkpointPersistenceProtocol.ts';
import { OPERATING_DISK_RESERVE_BYTES, SQLITE_WAL_ALLOWANCE_BYTES } from './diskAdmission.ts';
import type { BrainArtifactDescriptor } from '../../src/protocol/brainArtifacts.ts';

/** Owner-approved initial retained-record quota. */
const MAX_RECORDS = 100;
/** Owner-approved initial quota counts unique physical originals and shared weights. */
const MAX_STORED_BYTES = 2n * 1024n * 1024n * 1024n;
/** Full canonical metadata is admitted separately from portable bytes. */
const MAX_RECORD_BYTES = 1200 * 1024;
/** Fixed immutable object identity. */
const SHA = /^[0-9a-f]{64}$/u;
/** Exact scalar byte count. */
const U64 = /^[0-9a-f]{16}$/u;

/** Cold native-only codec operations, injected after source/build validation. */
export interface SavedBrainNativeCodec {
  /** Inspect native-built metadata without interpreting graph/weight bytes in Node. */
  inspect(path: string): BrainArtifactDescriptor;
  /** Validate the exact selected snapshot and enforce the physical stored-byte allowance. */
  publish(path: string, directory: string, operationId: string, expectedArtifact: string,
    availableBytes: string, artifactCounted: boolean, weightsCounted: boolean): unknown;
}

/** Parse bounded native-built library metadata; this never decodes model arrays. */
export function parseSavedBrainRecord(value: unknown): SavedBrainRecordV1 {
  if (!value || typeof value !== 'object' || Array.isArray(value) || ArrayBuffer.isView(value) || value instanceof ArrayBuffer ||
      Buffer.byteLength(JSON.stringify(value)) > MAX_RECORD_BYTES) throw new TypeError('invalid saved brain record');
  const row = value as Record<string, unknown>;
  const keys = ['schemaVersion', 'brainId', 'name', 'createdAtMs', 'artifact', 'weights',
    'artifactRelativeFilename', 'artifactByteCount', 'graphManifest', 'provenance'];
  if (Object.keys(row).length !== keys.length || keys.some(key => !Object.hasOwn(row, key)) ||
      row['schemaVersion'] !== 1 || typeof row['brainId'] !== 'string' || !SHA.test(row['brainId']) ||
      typeof row['name'] !== 'string' || row['name'].trim() !== row['name'] || !row['name'] ||
      Buffer.byteLength(row['name']) > 256 || row['name'].includes('\0') ||
      !Number.isSafeInteger(row['createdAtMs']) || (row['createdAtMs'] as number) < 0 ||
      typeof row['artifactByteCount'] !== 'string' || !U64.test(row['artifactByteCount']) ||
      row['artifactRelativeFilename'] !== `${row['brainId']}.slither-brain`) throw new TypeError('invalid saved brain fields');
  const artifact = parseBrainDescriptor(row['artifact']);
  const weights = parseManagedNeuralWeightsDescriptor(row['weights'], BigInt(artifact.parameterCount));
  const bytes = BigInt(`0x${row['artifactByteCount']}`);
  if (artifact.artifactSha256 !== row['brainId'] || weights.logicalSha256 !== artifact.weightsSha256 ||
      bytes < 88n || bytes > 64n * 1024n * 1024n) throw new TypeError('saved brain object identity mismatch');
  const graph = row['graphManifest'] as Record<string, unknown> | null;
  const provenance = row['provenance'] as Record<string, unknown> | null;
  if (!graph || typeof graph !== 'object' || Array.isArray(graph) || graph['schemaVersion'] !== 1 ||
      graph['architectureKey'] !== artifact.architectureKey || graph['layoutSha256'] !== artifact.layoutSha256 ||
      graph['sensorSha256'] !== artifact.sensorSha256 || graph['inputSize'] !== artifact.inputSize ||
      graph['totalParameters'] !== artifact.parameterCount || graph['totalStateSize'] !== artifact.stateSize ||
      typeof graph['sensorCanonicalJson'] !== 'string' ||
      createHash('sha256').update('slither.sensor-contract.v1\0').update(graph['sensorCanonicalJson']).digest('hex') !== artifact.sensorSha256 ||
      !provenance || typeof provenance !== 'object' || Array.isArray(provenance) ||
      Object.keys(provenance).length !== 3 || provenance['schemaVersion'] !== 1 ||
      provenance['sourceKind'] !== artifact.sourceKind || !Object.hasOwn(provenance, 'details') ||
      Buffer.byteLength(JSON.stringify(provenance)) > 64 * 1024) throw new TypeError('saved brain contract/provenance mismatch');
  const deployment = graph['deployment'] as Record<string, unknown> | null;
  if (!deployment || deployment['controlIntervalSeconds'] !== artifact.controlIntervalSeconds ||
      deployment['lifeStateRule'] !== artifact.lifeStateRule) throw new TypeError('saved brain cadence mismatch');
  return { schemaVersion: 1, brainId: row['brainId'], name: row['name'], createdAtMs: row['createdAtMs'] as number,
    artifact, weights, artifactRelativeFilename: row['artifactRelativeFilename'] as string,
    artifactByteCount: row['artifactByteCount'], graphManifest: graph,
    provenance: provenance as SavedBrainRecordV1['provenance'] };
}

/** Explicit retained library sharing the metadata worker's existing HoF object table. */
export class SavedBrainCatalog {
  /** Exact metadata connection; all catalog/HoF retention operations remain serialized. */
  private readonly db: Database.Database;
  /** Canonical controlled object directory. */
  private readonly root: string;
  /** Independently source-validated native codec, loaded lazily by its worker. */
  private readonly codec: SavedBrainNativeCodec;
  /** Production uses 2 GiB; tests may lower this bound, never raise it. */
  private readonly storedByteLimit: bigint;
  /** At most sixteen cold export/spawn leases; deleted rows retain their live objects. */
  private readonly leases = new Map<string, SavedBrainRecordV1>();

  /** One archive may pin up to 100 compact descriptors without retaining full graph metadata. */
  private readonly archiveLeases = new Map<string, SavedBrainSummaryV1[]>();

  /** Add the catalog without rewriting any existing HoF row or object. */
  constructor(db: Database.Database, directory: string, codec: SavedBrainNativeCodec, storedByteLimitForTesting = MAX_STORED_BYTES) {
    if (storedByteLimitForTesting < 1n || storedByteLimitForTesting > MAX_STORED_BYTES) throw new RangeError('invalid saved brain quota');
    this.storedByteLimit = storedByteLimitForTesting;
    this.db = db;
    this.root = realpathSync(directory);
    this.codec = codec;
    this.db.exec(`CREATE TABLE IF NOT EXISTS rust_saved_brains_v1 (
      brain_id TEXT PRIMARY KEY NOT NULL,
      weights_sha256 TEXT NOT NULL REFERENCES rust_hall_of_fame_weights_v1(logical_sha256),
      artifact_filename TEXT UNIQUE NOT NULL,
      artifact_byte_count_hex TEXT NOT NULL,
      record_json TEXT NOT NULL,
      record_sha256 TEXT NOT NULL,
      created_at_ms INTEGER NOT NULL
    )`);
  }

  /** Whether GC must defer deletion for a cold library export or spawn. */
  get hasLeases(): boolean { return this.leases.size !== 0 || this.archiveLeases.size !== 0; }

  /** Save only after native admission; exact retries preserve the original durable record. */
  save(path: string, name: string, operationId: string, createdAtMs = Date.now()): SavedBrainRecordV1 {
    if (!Number.isSafeInteger(createdAtMs) || createdAtMs < 0 || typeof path !== 'string' || !path || path.includes('\0') || path.length > 32768 ||
        typeof name !== 'string' || !name.trim() || name.includes('\0') || Buffer.byteLength(name.trim()) > 256 ||
        !/^[0-9a-f]{32}$/u.test(operationId)) throw new TypeError('invalid saved brain request');
    const selected = this.codec.inspect(path);
    const existing = this.get(selected.artifactSha256);
    if (existing) { this.verifyObjects(existing); this.verifyOriginal(existing); return existing; }
    const count = (this.db.prepare('SELECT COUNT(*) AS count FROM rust_saved_brains_v1').get() as {count: number}).count;
    if (count >= MAX_RECORDS) throw new Error('saved brain record quota exceeded');
    const filesystem = statfsSync(this.root, {bigint: true});
    // One bounded original, raw/fallback numeric candidates and the existing
    // SQLite/WAL/OS reserves must fit before any immutable publication starts.
    if (filesystem.bavail * filesystem.bsize < OPERATING_DISK_RESERVE_BYTES +
        SQLITE_WAL_ALLOWANCE_BYTES + 64n * 1024n * 1024n + 8_000_000n) {
      throw new Error('saved brain publication would consume the operating disk reserve');
    }
    const usage = this.usage();
    const raw = this.codec.publish(path, this.root, operationId, selected.artifactSha256,
      (this.storedByteLimit - usage.bytes).toString(16).padStart(16, '0'),
      usage.artifacts.has(selected.artifactSha256), usage.weights.has(selected.weightsSha256));
    if (!raw || typeof raw !== 'object' || Array.isArray(raw)) throw new TypeError('invalid native saved brain publication');
    const publication = raw as Record<string, unknown>;
    const record = parseSavedBrainRecord({ ...publication, brainId: selected.artifactSha256,
      name: name.trim(), createdAtMs });
    if (JSON.stringify(record.artifact) !== JSON.stringify(selected)) throw new Error('saved brain source changed during publication');
    const extra = (usage.artifacts.has(record.brainId) ? 0n : BigInt(`0x${record.artifactByteCount}`)) +
      (usage.weights.has(record.weights.logicalSha256) ? 0n : BigInt(`0x${record.weights.storedByteCount}`));
    if (usage.bytes + extra > this.storedByteLimit) throw new Error('saved brain stored-byte quota exceeded');
    this.verifyObjects(record);
    this.db.transaction(() => {
      const weights = record.weights;
      this.db.prepare(`INSERT OR IGNORE INTO rust_hall_of_fame_weights_v1
        (logical_sha256,relative_filename,encoding,stored_byte_count_hex,decoded_byte_count_hex,weight_count_hex,created_at_ms)
        VALUES(?,?,?,?,?,?,?)`).run(weights.logicalSha256, weights.relativeFilename, weights.encoding,
        weights.storedByteCount, weights.decodedByteCount, weights.weightCount, record.createdAtMs);
      const stored = this.db.prepare(`SELECT relative_filename,encoding,stored_byte_count_hex,decoded_byte_count_hex,
        weight_count_hex FROM rust_hall_of_fame_weights_v1 WHERE logical_sha256=?`).get(weights.logicalSha256) as Record<string, unknown>;
      if (stored['relative_filename'] !== weights.relativeFilename || stored['encoding'] !== weights.encoding ||
          stored['stored_byte_count_hex'] !== weights.storedByteCount || stored['decoded_byte_count_hex'] !== weights.decodedByteCount ||
          stored['weight_count_hex'] !== weights.weightCount) throw new Error('saved brain conflicts with immutable shared weights');
      this.db.prepare(`INSERT INTO rust_saved_brains_v1
        (brain_id,weights_sha256,artifact_filename,artifact_byte_count_hex,record_json,record_sha256,created_at_ms)
        VALUES(?,?,?,?,?,?,?)`).run(record.brainId, weights.logicalSha256, record.artifactRelativeFilename,
        record.artifactByteCount, JSON.stringify(record), createHash('sha256').update(JSON.stringify(record)).digest('hex'), record.createdAtMs);
    })();
    return record;
  }

  /** Retrieve one full bounded metadata record; heterogeneous graph storage is allowed. */
  get(brainId: string): SavedBrainRecordV1 | null {
    if (!SHA.test(brainId)) throw new TypeError('invalid saved brain identity');
    const row = this.db.prepare('SELECT * FROM rust_saved_brains_v1 WHERE brain_id=?').get(brainId) as Record<string, unknown> | undefined;
    if (!row) return null;
    const record = this.readRow(row);
    if (record.brainId !== brainId) throw new Error('saved brain catalog identity mismatch');
    return record;
  }

  /** Read bounded small summaries in stable identity order, never full graph pages. */
  list(after: string | null, limit: number): SavedBrainPageV1 {
    if ((after !== null && !SHA.test(after)) || !Number.isInteger(limit) || limit < 1 || limit > 25) throw new TypeError('invalid saved brain page');
    const rows = this.db.prepare('SELECT * FROM rust_saved_brains_v1 WHERE brain_id>? ORDER BY brain_id LIMIT ?')
      .all(after ?? '', limit + 1) as Array<Record<string, unknown>>;
    const records = rows.slice(0, limit).map(row => this.readRow(row));
    return { entries: records.map(record => this.summary(record)), next: rows.length > limit ? records.at(-1)!.brainId : null };
  }

  /** Pin immutable objects for an explicit cold export/spawn, independently of row deletion. */
  acquire(brainId: string, operationId: string): SavedBrainRecordV1 {
    if (!/^[0-9a-f]{32}$/u.test(operationId)) throw new TypeError('invalid saved brain lease');
    if (this.archiveLeases.has(operationId)) throw new Error('saved brain lease identity conflict');
    const previous = this.leases.get(operationId);
    if (previous) {
      if (previous.brainId !== brainId) throw new Error('saved brain lease identity conflict');
      return previous;
    }
    if (this.leases.size + this.archiveLeases.size >= 16) throw new Error('saved brain lease quota exceeded');
    const record = this.get(brainId);
    if (!record) throw new Error('saved brain does not exist');
    this.verifyObjects(record);
    this.verifyOriginal(record);
    this.leases.set(operationId, record);
    return record;
  }

  /** Atomically select an explicit archive subset; every portable source is cold-validated first. */
  acquireArchive(brainIds: readonly string[], operationId: string): SavedBrainSummaryV1[] {
    if (!/^[0-9a-f]{32}$/u.test(operationId) || !Array.isArray(brainIds) || brainIds.length < 1 || brainIds.length > MAX_RECORDS ||
        brainIds.some((id, index) => typeof id !== 'string' || !SHA.test(id) || (index > 0 && id <= brainIds[index - 1]!))) throw new TypeError('invalid saved brain archive selection');
    if (this.leases.has(operationId)) throw new Error('saved brain lease identity conflict');
    const previous = this.archiveLeases.get(operationId);
    if (previous) {
      if (previous.length !== brainIds.length || previous.some((entry, index) => entry.brainId !== brainIds[index])) throw new Error('saved brain archive lease identity conflict');
      return previous;
    }
    if (this.archiveLeases.size !== 0 || this.leases.size >= 16) throw new Error('saved brain archive lease quota exceeded');
    // The worker serializes catalog mutations. A failed final source cannot leave
    // an earlier partial lease or make a deleted row look unreferenced to GC.
    const entries = brainIds.map(brainId => {
      const record = this.get(brainId);
      if (!record) throw new Error('saved brain not found');
      this.verifyObjects(record); this.verifyOriginal(record);
      return this.summary(record);
    });
    if (Buffer.byteLength(JSON.stringify(entries)) > 256 * 1024) throw new Error('saved brain archive metadata quota exceeded');
    this.archiveLeases.set(operationId, entries);
    return entries;
  }

  /** Release one exact lease; a deleted portable original can then be reclaimed. */
  release(operationId: string): void {
    const archived = this.archiveLeases.get(operationId);
    if (archived) {
      this.archiveLeases.delete(operationId);
      for (const entry of archived) this.cleanupOriginal(entry);
      return;
    }
    const record = this.leases.get(operationId);
    if (!record) throw new Error('saved brain lease is not active');
    this.leases.delete(operationId);
    this.cleanupOriginal(record);
  }

  /** Delete only catalog membership; live native brains and leased files remain valid. */
  delete(brainId: string): boolean {
    const record = this.get(brainId);
    if (!record) return false;
    this.db.prepare('DELETE FROM rust_saved_brains_v1 WHERE brain_id=?').run(brainId);
    this.cleanupOriginal(record);
    return true;
  }

  /** Verify all small reference bindings before shared GC may delete anything. */
  referencedOriginals(): Array<{relativeFilename: string; byteCount: bigint}> {
    return this.referenceRecords().map(record => {
      this.verifyObjects(record);
      return {relativeFilename: record.artifactRelativeFilename, byteCount: BigInt(`0x${record.artifactByteCount}`)};
    });
  }

  /** Reject corrupt metadata bindings instead of treating their real objects as orphans. */
  private readRow(row: Record<string, unknown>): SavedBrainRecordV1 {
    if (typeof row['record_json'] !== 'string' || Buffer.byteLength(row['record_json']) > MAX_RECORD_BYTES) throw new Error('invalid saved brain metadata bytes');
    if (row['record_sha256'] !== createHash('sha256').update(row['record_json']).digest('hex')) throw new Error('saved brain metadata digest mismatch');
    const record = parseSavedBrainRecord(JSON.parse(row['record_json']));
    if (record.brainId !== row['brain_id'] || record.weights.logicalSha256 !== row['weights_sha256'] ||
        record.artifactRelativeFilename !== row['artifact_filename'] || record.artifactByteCount !== row['artifact_byte_count_hex'] ||
        record.createdAtMs !== row['created_at_ms']) throw new Error('saved brain catalog binding mismatch');
    return record;
  }

  /** A bounded complete reference walk is shared by quota admission and orphan validation. */
  private referenceRecords(): SavedBrainRecordV1[] {
    const rows = this.db.prepare('SELECT * FROM rust_saved_brains_v1 LIMIT 101').all() as Array<Record<string, unknown>>;
    if (rows.length > MAX_RECORDS) throw new Error('saved brain catalog quota is invalid');
    return rows.map(row => this.readRow(row));
  }

  /** Controlled direct files are verified without opening or interpreting neural bytes. */
  private verifyObjects(record: SavedBrainRecordV1): void {
    this.verifyFile(record.artifactRelativeFilename, BigInt(`0x${record.artifactByteCount}`));
    this.verifyFile(record.weights.relativeFilename, BigInt(`0x${record.weights.storedByteCount}`));
  }

  /** Reject links, escapes, file-kind changes and scalar-size mismatches. */
  private verifyFile(filename: string, bytes: bigint): void {
    const path = resolve(this.root, filename);
    const stats = lstatSync(path, {bigint: true});
    if (dirname(path) !== this.root || stats.isSymbolicLink() || !stats.isFile() || stats.size !== bytes ||
        dirname(realpathSync(path)) !== this.root) throw new Error('saved brain immutable file admission');
  }

  /** Cold export/deployment/retry admission independently validates the original portable file. */
  private verifyOriginal(record: SavedBrainRecordV1): void {
    const original = this.codec.inspect(join(this.root, record.artifactRelativeFilename));
    if (Object.entries(record.artifact).some(([key, value]) => original[key as keyof BrainArtifactDescriptor] !== value)) throw new Error('saved brain original descriptor mismatch');
  }

  /** Reclaim only unreferenced originals; the shared worker GC owns packed weight files. */
  private cleanupOriginal(record: Pick<SavedBrainSummaryV1, 'brainId' | 'artifactRelativeFilename' | 'artifactByteCount'>): void {
    if (this.get(record.brainId) || [...this.leases.values()].some(lease => lease.brainId === record.brainId) ||
        [...this.archiveLeases.values()].some(entries => entries.some(entry => entry.brainId === record.brainId))) return;
    try { this.verifyFile(record.artifactRelativeFilename, BigInt(`0x${record.artifactByteCount}`)); }
    catch (error) { if ((error as NodeJS.ErrnoException).code === 'ENOENT') return; throw error; }
    unlinkSync(join(this.root, record.artifactRelativeFilename));
  }

  /** Every unique library object, including deleted-but-leased originals, is charged once. */
  private usage(): {bytes: bigint; artifacts: Set<string>; weights: Set<string>} {
    const records = this.referenceRecords();
    const artifacts = new Map(records.map(record => [record.brainId, BigInt(`0x${record.artifactByteCount}`)]));
    const weights = new Map(records.map(record => [record.weights.logicalSha256, BigInt(`0x${record.weights.storedByteCount}`)]));
    for (const lease of this.leases.values()) {
      artifacts.set(lease.brainId, BigInt(`0x${lease.artifactByteCount}`));
      weights.set(lease.weights.logicalSha256, BigInt(`0x${lease.weights.storedByteCount}`));
    }
    for (const entries of this.archiveLeases.values()) for (const entry of entries) {
      artifacts.set(entry.brainId, BigInt(`0x${entry.artifactByteCount}`));
      weights.set(entry.weights.logicalSha256, BigInt(`0x${entry.weights.storedByteCount}`));
    }
    const bytes = [...artifacts.values(), ...weights.values()].reduce((sum, value) => sum + value, 0n);
    if (bytes > this.storedByteLimit) throw new Error('saved brain stored-byte quota is invalid');
    return { bytes, artifacts: new Set(artifacts.keys()), weights: new Set(weights.keys()) };
  }

  /** Omit full graph and trainer/source metadata from bounded list pages. */
  private summary(record: SavedBrainRecordV1): SavedBrainSummaryV1 {
    const {graphManifest: _graph, provenance: _provenance, artifact, ...summary} = record;
    const {architectureKey: _key, ...smallArtifact} = artifact;
    return {...summary, artifact: smallArtifact};
  }
}
