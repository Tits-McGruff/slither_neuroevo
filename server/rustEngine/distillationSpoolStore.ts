import Database from 'better-sqlite3';
import { createHash, randomBytes } from 'node:crypto';
import { closeSync, existsSync, fsyncSync, lstatSync, mkdirSync, openSync, readFileSync,
  opendirSync, realpathSync, renameSync, statfsSync, unlinkSync, writeSync } from 'node:fs';
import { join, resolve } from 'node:path';
import type { TeacherCaptureDrain, TeacherChunkDescriptor } from '../../src/protocol/distillation.ts';
import { DISTILLATION_SPOOL_DEFAULTS, DISTILLATION_ZERO, distillationHex, distillationInteger,
  parseSpoolLimits, parseTeacherDrain, type DistillationChunkPage, type DistillationDataset,
  type DistillationLease, type DistillationSpoolLimits, type DistillationDatasetSummary } from './distillationProtocol.ts';

/** SQLite metadata retained separately from the owner's game database. */
interface DatasetRow {
  /** Native run seed; old diagnostic stores retain an explicit unknown value. */
  run_seed: number | null;
  /** Immutable native identity. */
  epoch: string;
  /** Bounded immutable native contract JSON. */
  contract: string;
  /** Exact canonical sensor bytes represented as UTF-8. */
  sensors: string;
  /** Durable closure semantics. */
  state: DistillationDataset['state'];
  /** Highest durably observed native assignment. */
  production: string;
  /** Highest historically published complete object. */
  storage: string;
  /** Native disposition settled by completed spool work. */
  resolved: string;
  /** Last absolute native drop counters. */
  dropped_chunks: string;
  /** Last absolute native sample drops. */
  dropped_samples: string;
  /** Last absolute native byte drops. */
  dropped_bytes: string;
  /** Count of transferred chunks rejected by physical/retention admission. */
  spool_drops: number;
  /** Bounded native diagnostic. */
  error: string | null;
}

/** One complete opaque object and its bounded descriptor. */
interface ChunkRow {
  /** Global oldest-first retention order. */
  ordinal: number;
  /** Capture dataset. */
  epoch: string;
  /** Exact assigned sequence. */
  sequence: string;
  /** Digest/size/stream descriptor JSON. */
  descriptor: string;
  /** Complete encoded file size. */
  bytes: number;
  /** Safe immutable relative path. */
  filename: string;
}

/** Test-only interruption locations in the real publication path. */
export type DistillationSpoolFailpoint = 'after-write' | 'after-rename' | 'after-metadata';

/** Synchronous worker-owned durable spool; the main server uses its async client. */
export class DistillationSpoolStore {
  /** Canonical controlled directory containing only this spool's objects/database. */
  readonly root: string;
  /** Explicit storage and lease ceilings. */
  readonly limits: Readonly<DistillationSpoolLimits>;
  /** Single worker-owned synchronous connection. */
  private readonly db: Database.Database;
  /** Inject one real failure without changing production behavior. */
  private failpoint: DistillationSpoolFailpoint | undefined;

  /** Create/open only the dedicated spool and reconcile incomplete publications. */
  constructor(root: string, limits: DistillationSpoolLimits = { ...DISTILLATION_SPOOL_DEFAULTS },
    failpointForTesting?: DistillationSpoolFailpoint) {
    this.limits = Object.freeze(parseSpoolLimits(limits));
    if (!root || typeof root !== 'string') throw new TypeError('spool root must be nonempty');
    const requested = resolve(root);
    mkdirSync(requested, { recursive: true });
    if (lstatSync(requested).isSymbolicLink()) throw new Error('spool root cannot be a symlink');
    this.root = realpathSync(requested);
    const databasePath = join(this.root, 'metadata.sqlite');
    for (const suffix of ['', '-wal', '-shm']) {
      const path = `${databasePath}${suffix}`;
      if (existsSync(path) && !lstatSync(path).isFile()) throw new Error('spool database must be a regular file');
    }
    this.db = new Database(databasePath);
    this.failpoint = failpointForTesting;
    try {
      const applicationId = this.db.pragma('application_id', { simple: true });
      const version = this.db.pragma('user_version', { simple: true });
      const tables = this.db.prepare("SELECT COUNT(*) AS count FROM sqlite_master WHERE type = 'table'").get() as { count: number };
      if ((applicationId !== 0x534c5448 || (version !== 1 && version !== 2)) && (applicationId !== 0 || version !== 0 || tables.count !== 0)) {
        throw new Error('unsupported distillation spool database');
      }
      this.db.pragma('application_id = 1397511240');
      this.db.pragma('journal_mode = WAL');
      this.db.pragma('synchronous = FULL');
      this.db.pragma('foreign_keys = ON');
      this.db.exec(`
        CREATE TABLE IF NOT EXISTS datasets (
          epoch TEXT PRIMARY KEY, contract TEXT NOT NULL, sensors TEXT NOT NULL, state TEXT NOT NULL,
          production TEXT NOT NULL, storage TEXT NOT NULL, resolved TEXT NOT NULL,
          dropped_chunks TEXT NOT NULL, dropped_samples TEXT NOT NULL, dropped_bytes TEXT NOT NULL,
          spool_drops INTEGER NOT NULL DEFAULT 0, error TEXT, run_seed INTEGER
        );
        CREATE TABLE IF NOT EXISTS chunks (
          ordinal INTEGER PRIMARY KEY AUTOINCREMENT, epoch TEXT NOT NULL REFERENCES datasets(epoch),
          sequence TEXT NOT NULL, descriptor TEXT NOT NULL, bytes INTEGER NOT NULL, filename TEXT NOT NULL UNIQUE,
          UNIQUE(epoch, sequence)
        );
        CREATE INDEX IF NOT EXISTS chunks_epoch_sequence ON chunks(epoch, sequence);
        CREATE TABLE IF NOT EXISTS unavailable (epoch TEXT NOT NULL REFERENCES datasets(epoch), sequence TEXT NOT NULL,
          PRIMARY KEY(epoch, sequence));
        CREATE TABLE IF NOT EXISTS leases (id TEXT PRIMARY KEY, epoch TEXT NOT NULL REFERENCES datasets(epoch), expires INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS lease_chunks (lease_id TEXT NOT NULL REFERENCES leases(id) ON DELETE CASCADE,
          ordinal INTEGER NOT NULL REFERENCES chunks(ordinal), PRIMARY KEY(lease_id, ordinal));
      `);
      this.db.transaction(() => {
        if (version === 1) this.db.exec('ALTER TABLE datasets ADD COLUMN run_seed INTEGER');
        this.db.pragma('user_version = 2');
      })();
      this.db.prepare("UPDATE datasets SET state = 'interrupted', resolved = production WHERE state = 'active'").run();
      this.db.prepare('DELETE FROM unavailable WHERE sequence <= (SELECT resolved FROM datasets WHERE epoch = unavailable.epoch)').run();
      this.expireLeases();
      this.reconcile();
    } catch (error) { this.db.close(); throw error; }
  }

  /** Validate all metadata/digests first, then publish whole immutable objects. */
  ingest(input: TeacherCaptureDrain, runSeed: number): DistillationDataset {
    distillationInteger(runSeed, 0, 0xffff_ffff, 'native run seed');
    const drain = parseTeacherDrain(input);
    const epoch = drain.contract.captureEpoch;
    let dataset = this.datasetRow(epoch);
    const contract = JSON.stringify(drain.contract);
    if (dataset && (dataset.contract !== contract || dataset.sensors !== drain.sensorContractJson)) {
      throw new Error('immutable dataset contract mismatch');
    }
    if (dataset && dataset.run_seed !== null && dataset.run_seed !== runSeed) throw new Error('immutable dataset run seed mismatch');
    if (!dataset) {
      const count = this.db.prepare('SELECT COUNT(*) AS count FROM datasets').get() as { count: number };
      if (count.count >= this.limits.maxDatasets) throw new Error('dataset manifest quota exhausted');
      this.db.prepare(`INSERT INTO datasets(epoch, contract, sensors, state, production, storage, resolved,
        dropped_chunks, dropped_samples, dropped_bytes, run_seed) VALUES(?, ?, ?, 'active', ?, ?, ?, ?, ?, ?, ?)`).run(
        epoch, contract, drain.sensorContractJson, DISTILLATION_ZERO, DISTILLATION_ZERO, DISTILLATION_ZERO,
        DISTILLATION_ZERO, DISTILLATION_ZERO, DISTILLATION_ZERO, runSeed);
      dataset = this.datasetRow(epoch)!;
    }
    if (dataset.state === 'interrupted' && (drain.productionWatermark > dataset.production ||
        drain.chunks.some(chunk => chunk.descriptor.sequence > dataset.resolved))) {
      throw new Error('interrupted capture epoch cannot resume');
    }
    this.expireLeases();
    for (const chunk of drain.chunks) {
      const existing = this.chunkRow(epoch, chunk.descriptor.sequence);
      if (existing) {
        if (existing.descriptor !== JSON.stringify(chunk.descriptor)) throw new Error('immutable chunk retry mismatch');
        this.verifyFile(existing);
        continue;
      }
      if (chunk.descriptor.sequence <= dataset.resolved) continue; // Settled loss/eviction never resurrects on retry.
      if (!this.diskAdmits(chunk.bytes.byteLength) || !this.makeRoom(chunk.bytes.byteLength)) {
        this.db.transaction(() => {
          const inserted = this.db.prepare('INSERT OR IGNORE INTO unavailable VALUES(?, ?)').run(epoch, chunk.descriptor.sequence);
          this.db.prepare('UPDATE datasets SET spool_drops = spool_drops + ?, production = MAX(production, ?) WHERE epoch = ?')
            .run(inserted.changes, chunk.descriptor.sequence, epoch);
          // One explicit unavailable disposition can settle only its immediate contiguous successor.
          const current = this.datasetRow(epoch)!;
          if (BigInt(`0x${chunk.descriptor.sequence}`) === BigInt(`0x${current.resolved}`) + 1n) {
            this.db.prepare('UPDATE datasets SET resolved = ? WHERE epoch = ?').run(chunk.descriptor.sequence, epoch);
          }
        })();
        continue;
      }
      this.publish(chunk.descriptor, chunk.bytes);
    }
    this.db.transaction(() => {
      this.db.prepare(`UPDATE datasets SET production = MAX(production, ?), resolved = MAX(resolved, ?),
      dropped_chunks = MAX(dropped_chunks, ?), dropped_samples = MAX(dropped_samples, ?), dropped_bytes = MAX(dropped_bytes, ?),
      state = CASE WHEN state = 'interrupted' THEN state WHEN ? THEN 'stopped' ELSE state END,
      error = COALESCE(?, error) WHERE epoch = ?`).run(drain.productionWatermark, drain.releasedThrough,
      drain.droppedChunks, drain.droppedSamples, drain.droppedBytes, drain.stopped ? 1 : 0, drain.error ?? null, epoch);
      this.db.prepare('DELETE FROM unavailable WHERE epoch = ? AND sequence <= (SELECT resolved FROM datasets WHERE epoch = ?)').run(epoch, epoch);
    })();
    return this.dataset(epoch);
  }

  /** Return one coherent bounded metadata snapshot without reading any sample payload. */
  dataset(epoch: string): DistillationDataset {
    distillationHex(epoch, 32, 'capture epoch', true);
    const row = this.datasetRow(epoch);
    if (!row) throw new Error('dataset not found');
    const retained = this.db.prepare('SELECT COUNT(*) AS count, COALESCE(SUM(bytes), 0) AS bytes, MIN(sequence) AS floor FROM chunks WHERE epoch = ?')
      .get(epoch) as { count: number; bytes: number; floor: string | null };
    const floor = retained.floor ?? this.nextWord(row.resolved);
    return { schemaVersion: 1, runSeed: row.run_seed, contract: JSON.parse(row.contract) as DistillationDataset['contract'],
      sensorContractJson: row.sensors, state: row.state, productionWatermark: row.production,
      storageWatermark: row.storage, resolvedThrough: row.resolved, retentionFloor: floor,
      nativeDroppedChunks: row.dropped_chunks, nativeDroppedSamples: row.dropped_samples,
      nativeDroppedBytes: row.dropped_bytes, spoolDroppedChunks: this.word(row.spool_drops),
      retainedChunks: retained.count, retainedBytes: this.word(retained.bytes), error: row.error };
  }

  /** Bound pagination and include its watermarks from the same serialized worker turn. */
  datasets(after = '0'.repeat(32), limit = 100): { datasets: DistillationDatasetSummary[]; nextCursor: string; hasMore: boolean } {
    distillationHex(after, 32, 'dataset cursor'); distillationInteger(limit, 1, 256, 'dataset page limit');
    const rows = this.db.prepare('SELECT * FROM datasets WHERE epoch > ? ORDER BY epoch LIMIT ?').all(after, limit + 1) as DatasetRow[];
    const selected = rows.slice(0, limit);
    return { datasets: selected.map(row => ({ captureEpoch: row.epoch, runId: (JSON.parse(row.contract) as { runId: string }).runId,
      state: row.state, productionWatermark: row.production, storageWatermark: row.storage, resolvedThrough: row.resolved })),
    nextCursor: selected.at(-1)?.epoch ?? after, hasMore: rows.length > limit };
  }

  /** Bound pagination and include its watermarks from the same serialized worker turn. */
  list(epoch: string, after = DISTILLATION_ZERO, limit = 100): DistillationChunkPage {
    distillationHex(after, 16, 'chunk cursor');
    distillationInteger(limit, 1, 256, 'page limit');
    const dataset = this.dataset(epoch);
    const rows = this.db.prepare('SELECT * FROM chunks WHERE epoch = ? AND sequence > ? ORDER BY sequence LIMIT ?')
      .all(epoch, after, limit + 1) as ChunkRow[];
    const selected = rows.slice(0, limit);
    return { dataset, chunks: selected.map(row => JSON.parse(row.descriptor) as TeacherChunkDescriptor),
      nextCursor: selected.at(-1)?.sequence ?? after, hasMore: rows.length > limit };
  }

  /** Pin a finite available object set under independent global count/byte budgets. */
  lease(epoch: string, sequences: string[], durationMs: number): DistillationLease {
    this.dataset(epoch);
    distillationInteger(durationMs, 1, this.limits.maxLeaseMs, 'lease duration');
    if (!Array.isArray(sequences) || sequences.length < 1 || sequences.length > 256 || new Set(sequences).size !== sequences.length) {
      throw new TypeError('invalid lease sequence selection');
    }
    sequences.forEach(sequence => distillationHex(sequence, 16, 'lease chunk', true));
    this.expireLeases();
    return this.db.transaction(() => {
      const rows = sequences.map(sequence => this.chunkRow(epoch, sequence));
      if (rows.some(row => !row)) throw new Error('lease requested unavailable chunk');
      const count = this.db.prepare('SELECT COUNT(*) AS count FROM leases').get() as { count: number };
      if (count.count >= this.limits.maxLeases) throw new Error('lease count quota exhausted');
      const pinned = this.db.prepare('SELECT DISTINCT chunks.ordinal, chunks.bytes FROM chunks JOIN lease_chunks USING(ordinal)')
        .all() as Array<{ ordinal: number; bytes: number }>;
      const all = new Map(pinned.map(row => [row.ordinal, row.bytes]));
      rows.forEach(row => all.set(row!.ordinal, row!.bytes));
      if (all.size > this.limits.maxPinnedChunks || [...all.values()].reduce((sum, bytes) => sum + bytes, 0) > this.limits.maxPinnedBytes) {
        throw new Error('lease pin quota exhausted');
      }
      const id = randomBytes(16).toString('hex');
      const expiresAt = Date.now() + durationMs;
      this.db.prepare('INSERT INTO leases VALUES(?, ?, ?)').run(id, epoch, expiresAt);
      const insert = this.db.prepare('INSERT INTO lease_chunks VALUES(?, ?)');
      rows.forEach(row => insert.run(id, row!.ordinal));
      return { id, captureEpoch: epoch, expiresAt, chunks: rows.map(row => JSON.parse(row!.descriptor) as TeacherChunkDescriptor) };
    })();
  }

  /** Renew only a still-live storage lease for a bounded new interval. */
  renewLease(id: string, durationMs: number): number {
    distillationHex(id, 32, 'lease identity', true);
    distillationInteger(durationMs, 1, this.limits.maxLeaseMs, 'lease duration');
    this.expireLeases();
    const expires = Date.now() + durationMs;
    if (this.db.prepare('UPDATE leases SET expires = ? WHERE id = ?').run(expires, id).changes !== 1) throw new Error('lease not found/expired');
    return expires;
  }

  /** Explicit release is idempotent and never touches gameplay ownership. */
  releaseLease(id: string): void {
    distillationHex(id, 32, 'lease identity', true);
    this.db.prepare('DELETE FROM leases WHERE id = ?').run(id);
  }

  /** Copy one complete object while the serialized worker pins its filesystem read. */
  download(epoch: string, sequence: string, leaseId: string): { descriptor: TeacherChunkDescriptor; bytes: Uint8Array } {
    this.dataset(epoch);
    distillationHex(sequence, 16, 'chunk sequence', true);
    distillationHex(leaseId, 32, 'lease identity', true);
    this.expireLeases();
    const row = this.db.prepare(`SELECT chunks.* FROM chunks JOIN lease_chunks USING(ordinal)
      JOIN leases ON leases.id = lease_chunks.lease_id WHERE chunks.epoch = ? AND chunks.sequence = ? AND leases.id = ?`)
      .get(epoch, sequence, leaseId) as ChunkRow | undefined;
    if (!row) throw new Error('chunk unavailable or lease expired');
    const bytes = this.verifyFile(row);
    return { descriptor: JSON.parse(row.descriptor) as TeacherChunkDescriptor, bytes };
  }

  /** Permanently close a failed epoch using only its last durably observed tail. */
  interrupt(epoch: string): void {
    this.dataset(epoch);
    this.db.prepare("UPDATE datasets SET state = 'interrupted', resolved = production WHERE epoch = ?").run(epoch);
  }

  /** Flush/close only this worker-owned diagnostic database. */
  close(): void { if (this.db.open) this.db.close(); }

  /** Select bounded immutable epoch metadata. */
  private datasetRow(epoch: string): DatasetRow | undefined {
    return this.db.prepare('SELECT * FROM datasets WHERE epoch = ?').get(epoch) as DatasetRow | undefined;
  }

  /** Select the complete immutable object by exact epoch/sequence. */
  private chunkRow(epoch: string, sequence: string): ChunkRow | undefined {
    return this.db.prepare('SELECT * FROM chunks WHERE epoch = ? AND sequence = ?').get(epoch, sequence) as ChunkRow | undefined;
  }

  /** Rotate oldest complete unpinned objects; pinned saturation refuses new data. */
  private makeRoom(bytes: number): boolean {
    if (bytes > this.limits.maxBytes) return false;
    for (;;) {
      const total = this.db.prepare('SELECT COUNT(*) AS count, COALESCE(SUM(bytes), 0) AS bytes FROM chunks').get() as { count: number; bytes: number };
      if (total.count < this.limits.maxChunks && total.bytes + bytes <= this.limits.maxBytes) return true;
      const victim = this.db.prepare('SELECT * FROM chunks WHERE NOT EXISTS (SELECT 1 FROM lease_chunks WHERE lease_chunks.ordinal = chunks.ordinal) ORDER BY ordinal LIMIT 1')
        .get() as ChunkRow | undefined;
      if (!victim) return false;
      this.db.prepare('DELETE FROM chunks WHERE ordinal = ?').run(victim.ordinal);
      this.unlinkKnown(victim.filename);
    }
  }

  /** Preserve physical reserve plus the existing bounded SQLite/WAL allowance. */
  private diskAdmits(bytes: number): boolean {
    const filesystem = statfsSync(this.root, { bigint: true });
    return filesystem.bavail * filesystem.bsize >= BigInt(this.limits.diskReserveBytes) + BigInt(bytes) + 64n * 1024n ** 2n;
  }

  /** Fsync complete bytes, publish the final name, then commit descriptor visibility. */
  private publish(descriptor: TeacherChunkDescriptor, bytes: Uint8Array): void {
    const filename = `${descriptor.captureEpoch}-${descriptor.sequence}-${descriptor.sha256}.teacher`;
    const finalPath = join(this.root, filename);
    const temporary = `${filename}.${randomBytes(8).toString('hex')}.pending`;
    const partialPath = join(this.root, temporary);
    let file: number | undefined;
    let ownsFinal = false;
    let committed = false;
    try {
      file = openSync(partialPath, 'wx');
      let offset = 0;
      while (offset < bytes.byteLength) {
        const written = writeSync(file, bytes, offset, bytes.byteLength - offset);
        if (written === 0) throw new Error('spool write made no progress');
        offset += written;
      }
      fsyncSync(file); closeSync(file); file = undefined;
      this.inject('after-write');
      file = openSync(finalPath, 'wx'); ownsFinal = true; closeSync(file); file = undefined;
      renameSync(partialPath, finalPath);
      file = openSync(finalPath, 'r+'); fsyncSync(file); closeSync(file); file = undefined;
      this.syncDirectory();
      this.inject('after-rename');
      this.db.transaction(() => {
        this.db.prepare('INSERT INTO chunks(epoch, sequence, descriptor, bytes, filename) VALUES(?, ?, ?, ?, ?)')
          .run(descriptor.captureEpoch, descriptor.sequence, JSON.stringify(descriptor), bytes.byteLength, filename);
        this.db.prepare('UPDATE datasets SET production = MAX(production, ?), storage = MAX(storage, ?) WHERE epoch = ?')
          .run(descriptor.sequence, descriptor.sequence, descriptor.captureEpoch);
      })();
      committed = true;
      this.inject('after-metadata');
    } finally {
      if (file !== undefined) closeSync(file);
      if (existsSync(partialPath)) this.unlinkKnown(temporary);
      if (ownsFinal && !committed) this.unlinkKnown(filename);
    }
  }

  /** Flush POSIX rename metadata; Windows flushes the final regular file above. */
  private syncDirectory(): void {
    if (process.platform === 'win32') return;
    const file = openSync(this.root, 'r');
    try { fsyncSync(file); } finally { closeSync(file); }
  }

  /** Require a regular bounded file and verify exact digest before download/advertising. */
  private verifyFile(row: ChunkRow): Uint8Array {
    if (!/^[0-9a-f]{32}-[0-9a-f]{16}-[0-9a-f]{64}\.teacher$/u.test(row.filename)) throw new Error('unsafe spool object filename');
    const path = join(this.root, row.filename);
    const metadata = lstatSync(path);
    if (!metadata.isFile() || metadata.isSymbolicLink() || metadata.size !== row.bytes || row.bytes > 1024 * 1024) {
      throw new Error('spool object file shape mismatch');
    }
    const bytes = readFileSync(path);
    const descriptor = JSON.parse(row.descriptor) as TeacherChunkDescriptor;
    if (createHash('sha256').update(bytes).digest('hex') !== descriptor.sha256) throw new Error('spool object digest mismatch');
    return bytes;
  }

  /** Remove only exact safe object names under the verified dedicated directory. */
  private unlinkKnown(filename: string): void {
    if (!/^[0-9a-f]{32}-[0-9a-f]{16}-[0-9a-f]{64}\.teacher(?:\.[0-9a-f]{16}\.pending)?$/u.test(filename)) {
      throw new Error('unsafe spool filename');
    }
    const path = join(this.root, filename);
    if (existsSync(path)) unlinkSync(path);
  }

  /** Bound recovery scans and remove only task-owned unpublished artifacts. */
  private reconcile(): void {
    const files: string[] = [];
    const directory = opendirSync(this.root);
    try {
      for (let entry = directory.readSync(); entry; entry = directory.readSync()) {
        if (files.length >= 2 * this.limits.maxChunks + 16) throw new Error('spool recovery inventory limit');
        files.push(entry.name);
      }
    } finally { directory.closeSync(); }
    const rows = this.db.prepare('SELECT * FROM chunks ORDER BY ordinal LIMIT ?').all(this.limits.maxChunks + 1) as ChunkRow[];
    if (rows.length > this.limits.maxChunks) throw new Error('spool metadata exceeds selected count quota');
    const known = new Set(rows.map(row => row.filename));
    for (const filename of files) {
      if (/^[0-9a-f]{32}-[0-9a-f]{16}-[0-9a-f]{64}\.teacher(?:\.[0-9a-f]{16}\.pending)?$/u.test(filename) && !known.has(filename)) {
        this.unlinkKnown(filename);
      }
    }
    for (const row of rows) {
      try { this.verifyFile(row); } catch {
        this.db.transaction(() => {
          this.db.prepare('DELETE FROM lease_chunks WHERE ordinal = ?').run(row.ordinal);
          this.db.prepare('DELETE FROM chunks WHERE ordinal = ?').run(row.ordinal);
        })();
        this.unlinkKnown(row.filename);
      }
    }
  }

  /** Expiry removes only storage pins and never consumes a game/controller identity. */
  private expireLeases(): void { this.db.prepare('DELETE FROM leases WHERE expires <= ?').run(Date.now()); }

  /** One-shot real publication interruption for durability/retry verification. */
  private inject(point: DistillationSpoolFailpoint): void {
    if (this.failpoint !== point) return;
    this.failpoint = undefined;
    throw new Error(`injected spool failure ${point}`);
  }

  /** Exact small-count/byte conversion to an unsigned wire word. */
  private word(value: number): string { return BigInt(value).toString(16).padStart(16, '0'); }

  /** Saturate the empty-retention floor at u64::MAX. */
  private nextWord(value: string): string {
    const next = BigInt(`0x${value}`) + 1n;
    return (next > 0xffff_ffff_ffff_ffffn ? 0xffff_ffff_ffff_ffffn : next).toString(16).padStart(16, '0');
  }
}
