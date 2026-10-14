import Database from 'better-sqlite3';
import { createHash } from 'node:crypto';
import { closeSync, existsSync, fsyncSync, lstatSync, mkdirSync, openSync, readFileSync, readdirSync,
  renameSync, statfsSync, unlinkSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { MAX_BRAIN_ARTIFACT_BYTES, type BrainArtifactDescriptor, type BrainCandidateRecord } from '../../src/protocol/brainArtifacts.ts';
import { distillationHex, distillationInteger } from './distillationProtocol.ts';
import { parseShadowReceipt } from './shadowProtocol.ts';
import type { ShadowReceipt, ShadowEventPage } from '../../src/protocol/shadows.ts';

/** Separate bounded transient candidate retention, never automatic saved-brain catalog entries. */
const MAX_CANDIDATES = 8;
/** Candidate bytes cannot silently consume the teacher spool's retention quota. */
const MAX_CANDIDATE_BYTES = 64 * 1024 * 1024;
/** Fixed controlled candidate object names. */
const FILE = /^[0-9a-f]{64}\.slither-brain$/u;

/** Parse only independently native-built metadata, without parsing portable neural bytes. */
export function parseBrainDescriptor(value: unknown): BrainArtifactDescriptor {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new TypeError('invalid native brain descriptor');
  const data = value as Record<string, unknown>;
  const keys = ['schemaVersion', 'artifactSha256', 'weightsSha256', 'architectureKey', 'layoutSha256', 'sensorSha256',
    'inputSize', 'parameterCount', 'stateSize', 'controlIntervalSeconds', 'lifeStateRule', 'sourceKind'];
  if (Object.keys(data).length !== keys.length || keys.some(key => !Object.hasOwn(data, key))) throw new TypeError('invalid native brain descriptor fields');
  distillationInteger(data['schemaVersion'], 1, 1, 'brain descriptor version');
  for (const key of ['artifactSha256', 'weightsSha256', 'layoutSha256', 'sensorSha256']) distillationHex(data[key], 64, key);
  distillationInteger(data['inputSize'], 1, 504, 'brain inputs');
  distillationInteger(data['parameterCount'], 0, 1_000_000, 'brain parameters');
  distillationInteger(data['stateSize'], 0, 16384, 'brain state');
  const cadence = data['controlIntervalSeconds'];
  if (typeof cadence !== 'number' || !Number.isFinite(cadence) || cadence < 0.008 || cadence > 0.06 ||
      typeof data['architectureKey'] !== 'string' || !data['architectureKey'] || Buffer.byteLength(data['architectureKey']) > 512 * 1024 ||
      (data['lifeStateRule'] !== 'reset' && data['lifeStateRule'] !== 'preserve') ||
      !['distilled', 'imported', 'manual'].includes(data['sourceKind'] as string)) throw new TypeError('invalid native brain deployment metadata');
  return data as unknown as BrainArtifactDescriptor;
}

/** One durable transient candidate metadata row. */
interface Row {
  /** Content identity/filename prefix. */
  id: string;
  /** Exact independently verified metadata. */
  descriptor: string;
  /** Complete opaque file length. */
  bytes: number;
  /** FIFO publication order. */
  ordinal: number;
}

/** Worker-owned candidate files/database, isolated from game state and teacher retention. */
export class BrainCandidateStore {
  /** Controlled directory holding complete candidate artifacts. */
  readonly root: string;
  /** Dedicated durable metadata connection. */
  private readonly db: Database.Database;

  /** Source-identified coarse native file validator. */
  private readonly validate: (path: string, registerShadow?: boolean) => BrainArtifactDescriptor;
  /** Shared configured physical disk reserve. */
  private readonly reserveBytes: number;

  /** The supplied validator must load source-identified Rust; it never instantiates a world. */
  constructor(root: string, validate: (path: string, registerShadow?: boolean) => BrainArtifactDescriptor, reserveBytes: number) {
    this.validate = validate;
    this.reserveBytes = reserveBytes;
    this.root = resolve(root);
    mkdirSync(this.root, { recursive: true });
    if (lstatSync(this.root).isSymbolicLink()) throw new Error('candidate directory cannot be a symlink');
    const path = join(this.root, 'metadata.sqlite');
    for (const suffix of ['', '-wal', '-shm']) {
      const file = path + suffix;
      if (existsSync(file) && (!lstatSync(file).isFile() || lstatSync(file).size > 64 * 1024 * 1024)) throw new Error('candidate metadata file admission');
    }
    this.db = new Database(path);
    try {
      const application = this.db.pragma('application_id', { simple: true });
      const version = this.db.pragma('user_version', { simple: true });
      const tables = (this.db.prepare("SELECT COUNT(*) AS count FROM sqlite_master WHERE type='table'").get() as { count: number }).count;
      if ((application !== 0x534c4341 || (version !== 1 && version !== 2)) && (application !== 0 || version !== 0 || tables !== 0)) throw new Error('unsupported candidate metadata database');
      this.db.pragma('journal_mode=WAL'); this.db.pragma('synchronous=FULL');
      this.db.transaction(() => {
        this.db.exec('CREATE TABLE IF NOT EXISTS candidates (ordinal INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT UNIQUE NOT NULL, descriptor TEXT NOT NULL, bytes INTEGER NOT NULL)');
        this.db.exec('CREATE TABLE IF NOT EXISTS shadow_events (ordinal INTEGER PRIMARY KEY AUTOINCREMENT, process_epoch TEXT NOT NULL, command_sequence TEXT NOT NULL, request_id TEXT NOT NULL, payload TEXT NOT NULL, UNIQUE(process_epoch,command_sequence), UNIQUE(process_epoch,request_id))');
        this.db.pragma('application_id=1397506881'); this.db.pragma('user_version=2');
      })();
      const quota = this.db.prepare('SELECT COUNT(*) AS count, COALESCE(SUM(bytes),0) AS bytes, COALESCE(MAX(length(CAST(descriptor AS BLOB))),0) AS descriptorBytes FROM candidates').get() as { count: number; bytes: number; descriptorBytes: number };
      if (quota.count > MAX_CANDIDATES || quota.bytes > MAX_CANDIDATE_BYTES || quota.descriptorBytes > 768 * 1024) throw new Error('candidate metadata quota exceeded');
      const events = this.db.prepare('SELECT COUNT(*) AS count, COALESCE(MAX(length(CAST(payload AS BLOB))),0) AS bytes FROM shadow_events').get() as { count: number; bytes: number };
      if (events.count > 1024 || events.bytes > 16 * 1024) throw new Error('shadow chronology quota exceeded');
      for (const row of this.db.prepare('SELECT process_epoch,command_sequence,request_id,payload FROM shadow_events').all() as { process_epoch: string; command_sequence: string; request_id: string; payload: string }[]) {
        const receipt = parseShadowReceipt(JSON.parse(row.payload));
        if (receipt.processEpoch !== row.process_epoch || receipt.commandSequence !== row.command_sequence || receipt.requestId !== row.request_id) throw new Error('shadow chronology identity mismatch');
      }
      const rows = this.db.prepare('SELECT * FROM candidates').all() as Row[];
      for (const row of rows) {
        try { this.bytes(row); this.record(row); const admitted = parseBrainDescriptor(this.validate(join(this.root, `${row.id}.slither-brain`))); if (JSON.stringify(admitted) !== row.descriptor) throw new Error('candidate native metadata mismatch'); } catch {
          this.db.prepare('DELETE FROM candidates WHERE id=?').run(row.id);
          this.remove(row.id);
        }
      }
      for (const name of readdirSync(this.root)) {
        if (FILE.test(name) && !this.row(name.slice(0, 64))) unlinkSync(join(this.root, name));
        if (/^\.candidate-[0-9a-f]{64}\.tmp$/u.test(name)) unlinkSync(join(this.root, name));
      }
    } catch (error) { this.db.close(); throw error; }
  }

  /** Validate native layout before publishing bytes/metadata; content-ID retries are idempotent. */
  ingest(id: string, bytes: Uint8Array): BrainCandidateRecord {
    distillationHex(id, 64, 'candidate identity');
    if (!(bytes instanceof Uint8Array) || bytes.byteLength < 88 || bytes.byteLength > MAX_BRAIN_ARTIFACT_BYTES ||
        createHash('sha256').update(bytes).digest('hex') !== id) throw new TypeError('candidate byte/digest admission');
    const existing = this.row(id);
    if (existing) { this.bytes(existing); return this.record(existing); }
    const disk = statfsSync(this.root, { bigint: true });
    if (disk.bavail * disk.bsize < BigInt(this.reserveBytes) + BigInt(bytes.byteLength) + 64n * 1024n ** 2n) throw new Error('candidate disk reserve exhausted');
    const temporary = join(this.root, `.candidate-${id}.tmp`);
    const final = join(this.root, `${id}.slither-brain`);
    try {
      writeFileSync(temporary, bytes, { flag: 'wx' });
      const descriptor = parseBrainDescriptor(this.validate(temporary));
      if (descriptor.artifactSha256 !== id) throw new Error('native candidate identity mismatch');
      const descriptorJson = JSON.stringify(descriptor);
      if (Buffer.byteLength(descriptorJson) > 768 * 1024) throw new Error('candidate metadata admission');
      const total = this.db.prepare('SELECT COUNT(*) AS count, COALESCE(SUM(bytes),0) AS bytes FROM candidates').get() as { count: number; bytes: number };
      while (total.count >= MAX_CANDIDATES || total.bytes + bytes.byteLength > MAX_CANDIDATE_BYTES) {
        const old = this.db.prepare('SELECT * FROM candidates ORDER BY ordinal LIMIT 1').get() as Row | undefined;
        if (!old) throw new Error('candidate retention quota exhausted');
        this.db.prepare('DELETE FROM candidates WHERE id=?').run(old.id); this.remove(old.id);
        total.count -= 1; total.bytes -= old.bytes;
      }
      const file = openSync(temporary, 'r+'); try { fsyncSync(file); } finally { closeSync(file); }
      if (existsSync(final)) {
        const info = lstatSync(final);
        if (!info.isFile() || info.size !== bytes.byteLength || createHash('sha256').update(readFileSync(final)).digest('hex') !== id) throw new Error('candidate file publication collision');
      } else renameSync(temporary, final);
      const published = openSync(final, 'r+'); try { fsyncSync(published); } finally { closeSync(published); }
      this.syncDirectory();
      this.db.prepare('INSERT INTO candidates(id,descriptor,bytes) VALUES(?,?,?)').run(id, descriptorJson, bytes.byteLength);
      return { descriptor, byteCount: bytes.byteLength };
    } finally { if (existsSync(temporary)) unlinkSync(temporary); }
  }

  /** Exact available metadata; eviction returns an explicit unavailable identity. */
  get(id: string): BrainCandidateRecord {
    distillationHex(id, 64, 'candidate identity');
    const row = this.row(id); if (!row) throw new Error('candidate not found');
    this.bytes(row); return this.record(row);
  }

  /** Explicit cold registration cannot make cache capacity a stored-artifact corruption signal. */
  prepareShadow(id: string): BrainCandidateRecord {
    const record = this.get(id);
    const descriptor = parseBrainDescriptor(this.validate(join(this.root, `${id}.slither-brain`), true));
    if (JSON.stringify(descriptor) !== JSON.stringify(record.descriptor)) throw new Error('shadow candidate native metadata mismatch');
    return record;
  }

  /** Persist only a confirmed native result; exact duplicate replies are immutable retries. */
  recordShadow(value: unknown): ShadowReceipt {
    const receipt = parseShadowReceipt(value);
    const payload = JSON.stringify(receipt);
    if (Buffer.byteLength(payload) > 16 * 1024) throw new Error('shadow receipt byte admission');
    return this.db.transaction(() => {
      const old = this.db.prepare('SELECT payload FROM shadow_events WHERE process_epoch=? AND command_sequence=?').get(receipt.processEpoch, receipt.commandSequence) as { payload: string } | undefined;
      if (old) {
        if (old.payload !== payload) throw new Error('shadow receipt immutable retry mismatch');
        return receipt;
      }
      const disk = statfsSync(this.root, { bigint: true });
      if (disk.bavail * disk.bsize < BigInt(this.reserveBytes) + 64n * 1024n) throw new Error('shadow chronology disk reserve exhausted');
      this.db.prepare('INSERT INTO shadow_events(process_epoch,command_sequence,request_id,payload) VALUES(?,?,?,?)').run(receipt.processEpoch, receipt.commandSequence, receipt.requestId, payload);
      this.db.exec('DELETE FROM shadow_events WHERE ordinal NOT IN (SELECT ordinal FROM shadow_events ORDER BY ordinal DESC LIMIT 1024)');
      return receipt;
    })();
  }

  /** Find a retained request result before admitting an idempotent retry. */
  shadowReceipt(processEpoch: string, requestId: string): ShadowReceipt | null {
    distillationHex(processEpoch, 32, 'shadow process', true);
    distillationHex(requestId, 32, 'shadow request', true);
    const row = this.db.prepare('SELECT payload FROM shadow_events WHERE process_epoch=? AND request_id=?').get(processEpoch, requestId) as { payload: string } | undefined;
    return row ? parseShadowReceipt(JSON.parse(row.payload)) : null;
  }

  /** Read bounded chronology pages with exact SQLite u64 cursors, including rotated gaps. */
  shadowEvents(after: string, limit: number): ShadowEventPage {
    distillationHex(after, 16, 'shadow event cursor');
    distillationInteger(limit, 1, 256, 'shadow event page limit');
    if (BigInt('0x' + after) > 0x7fff_ffff_ffff_ffffn) throw new TypeError('shadow event cursor exceeds SQLite chronology');
    const rows = this.db.prepare('SELECT ordinal,payload FROM shadow_events WHERE ordinal>? ORDER BY ordinal LIMIT ?').safeIntegers(true).all(BigInt('0x' + after), limit + 1) as { ordinal: bigint; payload: string }[];
    const events = rows.slice(0, limit).map(row => ({ ordinal: row.ordinal.toString(16).padStart(16, '0'), receipt: parseShadowReceipt(JSON.parse(row.payload)) }));
    return { events, nextCursor: events.at(-1)?.ordinal ?? after, hasMore: rows.length > limit };
  }

  /** Return one complete immutable object; the serialized worker owns its entire read. */
  download(id: string): { record: BrainCandidateRecord; bytes: Uint8Array } {
    distillationHex(id, 64, 'candidate identity'); const row = this.row(id); if (!row) throw new Error('candidate not found'); return { record: this.record(row), bytes: this.bytes(row) };
  }

  /** Flush/close this diagnostic catalog only. */
  close(): void { if (this.db.open) this.db.close(); }

  /** Select one controlled content identity. */
  private row(id: string): Row | undefined { return this.db.prepare('SELECT * FROM candidates WHERE id=?').get(id) as Row | undefined; }
  /** Parse bounded native metadata retained by the publication transaction. */
  private record(row: Row): BrainCandidateRecord { return { descriptor: parseBrainDescriptor(JSON.parse(row.descriptor)), byteCount: row.bytes }; }
  /** Verify immutable opaque files independently of metadata before returning bytes. */
  private bytes(row: Row): Buffer {
    distillationHex(row.id, 64, 'stored candidate identity');
    distillationInteger(row.bytes, 88, MAX_BRAIN_ARTIFACT_BYTES, 'stored candidate bytes');
    const path = join(this.root, `${row.id}.slither-brain`);
    const info = lstatSync(path); if (!info.isFile() || info.size !== row.bytes) throw new Error('candidate file unavailable');
    const bytes = readFileSync(path); if (bytes.byteLength !== row.bytes || createHash('sha256').update(bytes).digest('hex') !== row.id) throw new Error('candidate file digest mismatch');
    return bytes;
  }
  /** Delete only a fixed content-addressed candidate basename. */
  private remove(id: string): void { distillationHex(id, 64, 'removed candidate'); const path = join(this.root, `${id}.slither-brain`); if (existsSync(path)) unlinkSync(path); }
  /** Durable POSIX directory publication; Windows file handles were flushed writable. */
  private syncDirectory(): void {
    if (process.platform !== 'win32') { const descriptor = openSync(this.root, 'r'); try { fsyncSync(descriptor); } finally { closeSync(descriptor); } }
  }
}
