import { MAX_BRAIN_ARTIFACT_BYTES, type BrainCandidateRecord } from '../../src/protocol/brainArtifacts.ts';
import { Worker } from 'node:worker_threads';
import type { TeacherCaptureDrain, TeacherChunkDescriptor } from '../../src/protocol/distillation.ts';
import { DISTILLATION_SPOOL_DEFAULTS, distillationHex, distillationInteger, parseSpoolLimits,
  parseTeacherDrain, type DistillationChunkPage, type DistillationDataset, type DistillationLease,
  type DistillationSpoolLimits, type DistillationDatasetSummary } from './distillationProtocol.ts';

/** Bounded request ownership while an isolated worker performs disk/SQLite work. */
interface Pending {
  /** Resolve only the exact matching request. */
  resolve(value: unknown): void;
  /** Reject worker failures independently of game authority. */
  reject(error: Error): void;
  /** Bound time spent without a complete worker reply. */
  timer: NodeJS.Timeout;
}

/** Async bounded diagnostic spool interface; it never owns a simulation/controller. */
export class DistillationSpoolClient {
  /** Initialization completes when the worker reconciles durable storage. */
  readonly ready: Promise<void>;
  /** Isolated synchronous filesystem/database owner. */
  private readonly worker: Worker;
  /** At most 32 bounded pending metadata/download requests. */
  private readonly pending = new Map<number, Pending>();
  /** Exact monotonically increasing request correlation. */
  private sequence = 0;
  /** One ingestion handoff may own bytes at a time. */
  private ingestActive = false;
  /** One complete candidate upload owns bytes at a time. */
  private candidateUploadActive = false;
  /** One complete candidate download owns bytes at a time. */
  private candidateDownloadActive = false;
  /** One bounded download pool. */
  private downloads = 0;
  /** Terminal diagnostic failure, independent of the game. */
  private failure: Error | undefined;
  /** Prevent new requests once close begins. */
  private closing = false;
  /** Initialization resolver retained until the worker is ready. */
  private resolveReady!: () => void;
  /** Initialization rejection on startup failure. */
  private rejectReady!: (error: Error) => void;
  /** One bounded startup deadline. */
  private startupTimer: NodeJS.Timeout;
  /** Exit is observed from construction so close cannot miss an early exit event. */
  private readonly exited: Promise<void>;
  /** Resolve the retained worker exit witness. */
  private resolveExited!: () => void;
  /** Repeated callers share the exact shutdown operation. */
  private closePromise: Promise<void> | undefined;

  /** Start a worker only after validating storage ceilings and the dedicated root. */
  constructor(root: string, limits: DistillationSpoolLimits = { ...DISTILLATION_SPOOL_DEFAULTS }) {
    const parsed = parseSpoolLimits(limits);
    if (typeof root !== 'string' || !root || root.length > 32_768) throw new TypeError('invalid spool root');
    this.ready = new Promise((resolve, reject) => { this.resolveReady = resolve; this.rejectReady = reject; });
    void this.ready.catch(() => {}); // A later API call still receives the original rejection.
    this.exited = new Promise(resolve => { this.resolveExited = resolve; });
    this.worker = new Worker(new URL('./distillationSpoolWorker.ts', import.meta.url), { workerData: { root, limits: parsed } });
    this.startupTimer = setTimeout(() => this.fail(new Error('distillation spool startup timed out')), 60_000);
    this.startupTimer.unref();
    this.worker.on('message', (message: unknown) => this.receive(message));
    this.worker.on('error', error => this.fail(error));
    this.worker.on('messageerror', error => this.fail(error));
    this.worker.on('exit', code => {
      this.resolveExited();
      if (!this.closing || code !== 0) this.fail(new Error(`distillation spool worker exited (${code})`));
    });
  }

  /** Transfer an exclusive bounded copy; caller/native bytes retain their ownership. */
  async ingest(value: TeacherCaptureDrain, runSeed: number): Promise<DistillationDataset> {
    distillationInteger(runSeed, 0, 0xffff_ffff, 'native run seed');
    if (this.ingestActive) throw new Error('distillation ingestion already active');
    this.ingestActive = true;
    try {
      const parsed = parseTeacherDrain(value);
      const chunks = parsed.chunks.map(chunk => ({ descriptor: chunk.descriptor, bytes: new Uint8Array(chunk.bytes) }));
      const transfer = chunks.map(chunk => chunk.bytes.buffer);
      return await this.request('ingest', [{ ...parsed, chunks }, runSeed], transfer) as DistillationDataset;
    } finally { this.ingestActive = false; }
  }

  /** Transfer one exclusive opaque candidate for independent native file admission. */
  async ingestCandidate(id: string, bytes: Uint8Array): Promise<BrainCandidateRecord> {
    distillationHex(id, 64, 'candidate identity');
    if (!(bytes instanceof Uint8Array) || bytes.byteLength < 88 || bytes.byteLength > MAX_BRAIN_ARTIFACT_BYTES) throw new TypeError('candidate byte admission');
    if (this.candidateUploadActive) throw new Error('candidate upload concurrency exhausted');
    this.candidateUploadActive = true;
    try {
      const owned = new Uint8Array(bytes);
      return await this.request('candidate-ingest', [id, owned], [owned.buffer]) as BrainCandidateRecord;
    } finally { this.candidateUploadActive = false; }
  }

  /** Read only native-admitted immutable metadata. */
  async candidate(id: string): Promise<BrainCandidateRecord> {
    distillationHex(id, 64, 'candidate identity');
    return await this.request('candidate-get', [id]) as BrainCandidateRecord;
  }

  /** Read one complete bounded object; response ownership is separately admitted by HTTP. */
  async downloadCandidate(id: string): Promise<{ record: BrainCandidateRecord; bytes: Uint8Array }> {
    distillationHex(id, 64, 'candidate identity');
    if (this.candidateDownloadActive) throw new Error('candidate download concurrency exhausted');
    this.candidateDownloadActive = true;
    try { return await this.request('candidate-download', [id]) as { record: BrainCandidateRecord; bytes: Uint8Array }; }
    finally { this.candidateDownloadActive = false; }
  }
  /** Read immutable dataset metadata and current durable watermarks. */
  async dataset(epoch: string): Promise<DistillationDataset> {
    distillationHex(epoch, 32, 'capture epoch', true);
    return await this.request('dataset', [epoch]) as DistillationDataset;
  }

  /** Fetch one bounded metadata page without reading any sample object. */
  async datasets(after: string, limit: number): Promise<{ datasets: DistillationDatasetSummary[]; nextCursor: string; hasMore: boolean }> {
    distillationHex(after, 32, 'dataset cursor'); distillationInteger(limit, 1, 256, 'dataset page limit');
    return await this.request('datasets', [after, limit]) as { datasets: DistillationDatasetSummary[]; nextCursor: string; hasMore: boolean };
  }

  /** Fetch one bounded metadata page without reading any sample object. */
  async list(epoch: string, after: string, limit: number): Promise<DistillationChunkPage> {
    distillationHex(epoch, 32, 'capture epoch', true); distillationHex(after, 16, 'chunk cursor');
    distillationInteger(limit, 1, 256, 'page limit');
    return await this.request('list', [epoch, after, limit]) as DistillationChunkPage;
  }

  /** Pin only a bounded immutable available set, with no actor/controller identity. */
  async lease(epoch: string, sequences: string[], durationMs: number): Promise<DistillationLease> {
    distillationHex(epoch, 32, 'capture epoch', true);
    if (!Array.isArray(sequences) || sequences.length < 1 || sequences.length > 256 || new Set(sequences).size !== sequences.length) {
      throw new TypeError('invalid bounded lease selection');
    }
    sequences.forEach(sequence => distillationHex(sequence, 16, 'lease chunk', true));
    distillationInteger(durationMs, 1, 300_000, 'lease duration');
    return await this.request('lease', [epoch, sequences, durationMs]) as DistillationLease;
  }

  /** Renew one storage-only lease for a finite interval. */
  async renewLease(id: string, durationMs: number): Promise<number> {
    distillationHex(id, 32, 'lease identity', true); distillationInteger(durationMs, 1, 300_000, 'lease duration');
    return await this.request('renew', [id, durationMs]) as number;
  }

  /** Release one pin idempotently. */
  async releaseLease(id: string): Promise<void> {
    distillationHex(id, 32, 'lease identity', true); await this.request('release', [id]);
  }

  /** Bound independent download buffers and require a live object pin. */
  async download(epoch: string, sequence: string, leaseId: string): Promise<{ descriptor: TeacherChunkDescriptor; bytes: Uint8Array }> {
    distillationHex(epoch, 32, 'capture epoch', true); distillationHex(sequence, 16, 'chunk sequence', true);
    distillationHex(leaseId, 32, 'lease identity', true);
    if (this.downloads >= 4) throw new Error('distillation download concurrency exhausted');
    this.downloads += 1;
    try { return await this.request('download', [epoch, sequence, leaseId]) as { descriptor: TeacherChunkDescriptor; bytes: Uint8Array }; }
    finally { this.downloads -= 1; }
  }

  /** Mark only the last durably observed epoch tail interrupted. */
  async interrupt(epoch: string): Promise<void> {
    distillationHex(epoch, 32, 'capture epoch', true); await this.request('interrupt', [epoch]);
  }

  /** Close after the worker finishes prior accepted operations. */
  async close(): Promise<void> {
    this.closePromise ??= this.closeInternal();
    return this.closePromise;
  }

  /** Seal shutdown once, retaining its exit witness across an early worker reply. */
  private async closeInternal(): Promise<void> {
    if (this.closing) return;
    if (this.failure) { this.closing = true; await this.worker.terminate(); return; }
    await this.ready;
    this.closing = true;
    await this.request('close', []);
    await this.exited;
  }

  /** Admit a bounded request and its exact correlation before transferring bytes. */
  private async request(operation: string, args: unknown[], transfer: ArrayBuffer[] = []): Promise<unknown> {
    await this.ready;
    if (this.failure) throw this.failure;
    if (this.closing && operation !== 'close') throw new Error('distillation spool closing');
    if (this.pending.size >= 32 || this.sequence >= Number.MAX_SAFE_INTEGER) throw new Error('distillation spool request admission exhausted');
    const id = ++this.sequence;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => this.fail(new Error('distillation spool request timed out')), 60_000);
      timer.unref();
      this.pending.set(id, { resolve, reject, timer });
      try { this.worker.postMessage({ id, operation, args }, transfer); }
      catch (error) { this.fail(error instanceof Error ? error : new Error(String(error))); }
    });
  }

  /** Require exact typed correlation; an unrelated worker response cannot settle a request. */
  private receive(message: unknown): void {
    if (!message || typeof message !== 'object' || Array.isArray(message)) { this.fail(new Error('invalid spool worker response')); return; }
    const raw = message as Record<string, unknown>;
    if (raw['kind'] === 'ready' && Object.keys(raw).length === 1) {
      clearTimeout(this.startupTimer); this.resolveReady(); return;
    }
    const id = raw['id'];
    if (raw['kind'] !== 'result' || typeof id !== 'number' || !Number.isSafeInteger(id) || typeof raw['ok'] !== 'boolean') {
      this.fail(new Error('invalid spool worker correlation')); return;
    }
    const pending = this.pending.get(id);
    if (!pending) { this.fail(new Error('unknown spool worker request identity')); return; }
    this.pending.delete(id); clearTimeout(pending.timer);
    if (raw['ok']) pending.resolve(raw['result']);
    else if (typeof raw['error'] === 'string' && raw['error'].length <= 1024) pending.reject(new Error(raw['error']));
    else { pending.reject(new Error('invalid spool worker failure')); this.fail(new Error('invalid spool worker failure')); }
  }

  /** Stop only diagnostic work; no authoritative engine reference exists here. */
  private fail(error: Error): void {
    if (this.failure) return;
    this.failure = error;
    clearTimeout(this.startupTimer); this.rejectReady(error);
    for (const pending of this.pending.values()) { clearTimeout(pending.timer); pending.reject(error); }
    this.pending.clear();
    void this.worker.terminate();
  }
}
