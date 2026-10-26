import { createHash, randomBytes } from 'node:crypto';
import { open, unlink } from 'node:fs/promises';
import { join } from 'node:path';
import type { SavedBrainDeploymentScopeV1, SavedBrainPageV1, SavedBrainRecordV1,
  SavedBrainSpawnReceiptV1, SavedBrainSpawnRequestV1 } from '../protocol.ts';
import { MAX_BRAIN_ARTIFACT_BYTES } from '../../src/protocol/brainArtifacts.ts';
import type { RustBackgroundEvent } from '../../src/protocol/rustBackground.ts';
import type { ExperimentalRunningAuthorityNativeHandle } from './backgroundRuntime.ts';
import type { CheckpointPersistenceClient } from './checkpointPersistenceClient.ts';
import type { BackgroundCommandAdmission } from './commandAdmission.ts';
import type { DistillationSpoolClient } from './distillationSpoolClient.ts';
import { admitDiskOperation } from './diskAdmission.ts';
import { distillationHex } from './distillationProtocol.ts';

/** Bounded process-local retry history; restart changes the required process identity. */
const MAX_RECEIPTS = 64;
/** API timeout never releases a lease for a command whose native outcome is unknown. */
const COMMAND_WAIT_MS = 30_000;

/** One exact admitted deployment, retained beyond an HTTP timeout until native completion. */
interface PendingSpawn {
  /** Complete immutable request fingerprint. */
  key: string;
  /** Caller and scope fields used to construct the exact confirmed receipt. */
  request: SavedBrainSpawnRequestV1;
  /** Complete portable source identity. */
  brainId: string;
  /** Private metadata-worker lease retained while Rust may still read files. */
  leaseId: string;
  /** Admitted global command identity. */
  sequence: string;
  /** Bounded client wait independent of eventual native outcome. */
  timer: NodeJS.Timeout;
  /** Resolve only the corresponding confirmed publication. */
  resolve(receipt: SavedBrainSpawnReceiptV1): void;
  /** Timeouts/rejections do not imply cancellation of admitted native work. */
  reject(error: Error): void;
}

/** One small immutable retry outcome; it retains no portable or neural buffers. */
interface SpawnOutcome {
  /** Reject request-ID reuse with different immutable fields. */
  key: string;
  /** Native-confirmed publication when successful. */
  receipt?: SavedBrainSpawnReceiptV1;
  /** Known rejection, never an ambiguous timeout. */
  error?: string;
}

/** One bounded opaque export snapshot held until its HTTP response settles. */
export interface SavedBrainExport {
  /** Native-admitted metadata; the browser never reconstructs model parameters. */
  record: SavedBrainRecordV1;
  /** Exact self-contained portable source bytes. */
  bytes: Uint8Array;
  /** Release the cold object lease once, including cancelled responses. */
  release(): Promise<void>;
}

/** Explicit library controls; Rust validates every model and owns every live spawn. */
export class SavedBrainLibrary {
  /** Diagnostic identity for safe request retry across a server restart. */
  readonly processEpoch = randomBytes(16).toString('hex');
  /** One save/preparation operation across candidate and managed-object IO. */
  private preparation: Promise<unknown> | undefined;
  /** Exact pending native deployment; a timeout leaves this lease owned. */
  private pending: PendingSpawn | undefined;
  /** Shared promise for exact retries during preparation or the native wait. */
  private activeSpawn: {key: string; requestId: string; promise: Promise<SavedBrainSpawnReceiptV1>} | undefined;
  /** Compare-and-swap cursor prevents evicted request IDs from spawning again. */
  private deploymentSequence = '0000000000000000';
  /** Fixed-size confirmed/rejected command chronology. */
  private readonly outcomes = new Map<string, SpawnOutcome>();
  /** One response-owned bounded portable export at a time. */
  private exportLease: {leaseId: string; release(): Promise<void>} | undefined;
  /** Asynchronous lease release must finish before another deployment or shutdown. */
  private releasing: Promise<void> | undefined;
  /** A failed worker lease release blocks new cold work while gameplay continues. */
  private failure: Error | undefined;
  /** Reject new work after shutdown begins. */
  private closed = false;

  /** Reuse the private spool and existing serialized metadata worker and command sequence. */
  constructor(
    private readonly persistence: Pick<CheckpointPersistenceClient, 'savedBrainCatalog'>,
    private readonly spool: Pick<DistillationSpoolClient, 'downloadCandidate'>,
    private readonly directory: string,
    private readonly native: () => ExperimentalRunningAuthorityNativeHandle,
    private readonly admission: BackgroundCommandAdmission,
    private readonly scope: () => SavedBrainDeploymentScopeV1,
    private readonly available: () => boolean,
    private readonly wake: () => void
  ) {}

  /** Archive/replacement/GC owners must exclude live library file consumers. */
  get busy(): boolean { return !!(this.preparation || this.pending || this.exportLease || this.releasing); }

  /** Publish a process identity and an exact refreshable native deployment scope. */
  status(): {schemaVersion: 1; processEpoch: string; deploymentSequence: string; scope: SavedBrainDeploymentScopeV1} {
    this.requireOpen();
    return {schemaVersion: 1, processEpoch: this.processEpoch, deploymentSequence: this.deploymentSequence, scope: this.scope()};
  }

  /** Metadata pages remain available independently of capture and candidate FIFO rotation. */
  async list(after: string | null, limit: number): Promise<SavedBrainPageV1> {
    this.requireOpen();
    const result = await this.persistence.savedBrainCatalog({action: 'list', after, limit});
    if (result.action !== 'list') throw new Error('saved brain list response mismatch');
    return result.page;
  }

  /** Retrieve one complete bounded graph/provenance record without model buffers. */
  async read(brainId: string): Promise<SavedBrainRecordV1> {
    this.requireOpen(); this.identity(brainId);
    const result = await this.persistence.savedBrainCatalog({action: 'read', brainId});
    if (result.action !== 'read' || !result.record) throw new Error('saved brain not found');
    return result.record;
  }

  /** Remove catalog membership; native live values and cold export/spawn leases remain valid. */
  async delete(brainId: string): Promise<{brainId: string; deleted: boolean}> {
    this.requireOpen(); this.identity(brainId);
    const result = await this.persistence.savedBrainCatalog({action: 'delete', brainId});
    if (result.action !== 'delete') throw new Error('saved brain delete response mismatch');
    return {brainId, deleted: result.deleted};
  }

  /** Snapshot a selected candidate before FIFO eviction, then save through native publication. */
  saveCandidate(brainId: string, value: unknown): Promise<SavedBrainRecordV1> {
    this.identity(brainId);
    const name = this.name(value); this.requireExclusive();
    return this.publishSnapshot(async () => {
      const snapshot = await this.spool.downloadCandidate(brainId);
      if (!(snapshot.bytes instanceof Uint8Array) || snapshot.bytes.byteLength !== snapshot.record.byteCount ||
          snapshot.record.descriptor.artifactSha256 !== brainId) throw new Error('saved brain candidate snapshot mismatch');
      return snapshot.bytes;
    }, name, brainId);
  }

  /** Explicit standalone import preserves the original portable bytes and embedded provenance. */
  importArtifact(bytes: Uint8Array, value: unknown): Promise<SavedBrainRecordV1> {
    const name = this.name(value); this.requireExclusive();
    if (!(bytes instanceof Uint8Array) || bytes.byteLength < 88 || bytes.byteLength > MAX_BRAIN_ARTIFACT_BYTES) throw new TypeError('invalid portable brain byte count');
    return this.publishSnapshot(async () => bytes, name);
  }

  /** Admit one private source file plus worst-case immutable/codec publication before writing. */
  private publishSnapshot(load: () => Promise<Uint8Array>, name: string, expectedId?: string): Promise<SavedBrainRecordV1> {
    const operation = (async () => {
      await admitDiskOperation(this.directory, {operation: 'pin', sourceSpoolBytes: BigInt(MAX_BRAIN_ARTIFACT_BYTES),
        candidateSpoolBytes: 8_000_000n, finalManagedBytes: BigInt(MAX_BRAIN_ARTIFACT_BYTES) + 4_000_000n});
      const bytes = await load();
      if (bytes.byteLength < 88 || bytes.byteLength > MAX_BRAIN_ARTIFACT_BYTES) throw new Error('saved brain snapshot byte admission');
      const digest = createHash('sha256').update(bytes).digest('hex');
      if (expectedId !== undefined && digest !== expectedId) throw new Error('saved brain candidate snapshot mismatch');
      this.requireOpen();
      const path = join(this.directory, `.${randomBytes(16).toString('hex')}.brain-upload.partial`);
      const file = await open(path, 'wx');
      try {
        await file.writeFile(bytes); await file.sync(); await file.close();
        const result = await this.persistence.savedBrainCatalog({action: 'save', path, name});
        if (result.action !== 'save' || result.record.brainId !== digest) throw new Error('saved brain publication response mismatch');
        return result.record;
      } finally {
        await file.close();
        await unlink(path).catch(error => { if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error; });
      }
    })();
    this.preparation = operation;
    void operation.finally(() => { if (this.preparation === operation) this.preparation = undefined; }).catch(() => {});
    return operation;
  }

  /** Export only explicit saved membership, with native admission and an exact byte snapshot. */
  async export(brainId: string): Promise<SavedBrainExport> {
    this.identity(brainId); this.requireExclusive();
    const leaseId = randomBytes(16).toString('hex');
    const acquisition = this.persistence.savedBrainCatalog({action: 'acquire', brainId, leaseId});
    let releasePromise: Promise<void> | undefined;
    /** Shutdown and response cancellation share the same complete lease-release witness. */
    const release = (): Promise<void> => releasePromise ??= acquisition.then(async () => {
      try { await this.persistence.savedBrainCatalog({action: 'release', leaseId}); }
      catch (error) { this.failure = new Error('saved brain worker lease release failed: ' + String(error)); throw error; }
    }).finally(() => { if (this.exportLease?.leaseId === leaseId) this.exportLease = undefined; });
    this.exportLease = {leaseId, release};
    try {
      const result = await acquisition;
      if (result.action !== 'acquire') throw new Error('saved brain export lease mismatch');
      this.requireOpen();
      const record = result.record;
      const file = await open(join(this.directory, record.artifactRelativeFilename), 'r');
      try {
        const expected = Number(BigInt('0x' + record.artifactByteCount));
        const stats = await file.stat();
        if (!stats.isFile() || stats.size !== expected || expected > MAX_BRAIN_ARTIFACT_BYTES) throw new Error('saved brain export file admission');
        const bytes = Buffer.allocUnsafe(expected);
        let offset = 0;
        while (offset < expected) {
          const part = await file.read(bytes, offset, expected - offset, offset);
          if (!part.bytesRead) throw new Error('saved brain export truncated');
          offset += part.bytesRead;
        }
        if ((await file.stat()).size !== expected || createHash('sha256').update(bytes).digest('hex') !== brainId) throw new Error('saved brain export source changed');
        return {record, bytes, release};
      } finally { await file.close(); }
    } catch (error) { await release().catch(() => {}); throw error; }
  }

  /** Exact retries share one operation; a process restart requires a fresh explicit request. */
  spawn(brainId: string, value: unknown): Promise<SavedBrainSpawnReceiptV1> {
    this.identity(brainId); this.requireOpen();
    const request = this.spawnRequest(value);
    const key = JSON.stringify([brainId, request]);
    const prior = this.outcomes.get(request.requestId);
    if (prior) {
      if (prior.key !== key) throw new Error('saved brain request immutable retry mismatch');
      return prior.receipt ? Promise.resolve(prior.receipt) : Promise.reject(new Error(prior.error));
    }
    if (this.activeSpawn?.requestId === request.requestId) {
      if (this.activeSpawn.key !== key) throw new Error('saved brain request immutable retry mismatch');
      return this.activeSpawn.promise;
    }
    this.requireExclusive();
    if (request.expectedDeploymentSequence !== this.deploymentSequence) throw new Error('saved brain deployment sequence changed; refresh library metadata');
    const leaseId = randomBytes(16).toString('hex');
    const operation = (async () => {
      const result = await this.persistence.savedBrainCatalog({action: 'acquire', brainId, leaseId});
      let admitted = false;
      try {
        if (result.action !== 'acquire') throw new Error('saved brain deployment lease mismatch');
        this.requireOpen();
        const scope = this.scope();
        if (!this.available() || scope.expectedRunId !== request.expectedRunId ||
            scope.expectedConfigRevision !== request.expectedConfigRevision || scope.expectedWorldEpoch !== request.expectedWorldEpoch) throw new Error('saved brain deployment scope changed or authority busy');
        return await new Promise<SavedBrainSpawnReceiptV1>((resolve, reject) => {
          let sequence: string | undefined;
          if (!this.admission.trySubmitControl(current => {
            this.native().submitSavedBrainSpawn(current, this.directory, {brainId, ...scope, weights: result.record.weights});
            sequence = current;
          }) || !sequence) { reject(new Error('saved brain command queue busy')); return; }
          admitted = true;
          const timer = setTimeout(() => reject(new Error('saved brain deployment timed out; native outcome remains pending')), COMMAND_WAIT_MS);
          timer.unref();
          this.pending = {key, request, brainId, leaseId, sequence, timer, resolve, reject};
          this.wake();
        });
      } finally { if (!admitted) await this.persistence.savedBrainCatalog({action: 'release', leaseId}); }
    })();
    this.preparation = operation;
    this.activeSpawn = {key, requestId: request.requestId, promise: operation};
    void operation.finally(() => {
      if (this.preparation === operation) this.preparation = undefined;
      if (!this.pending && this.activeSpawn?.promise === operation) this.activeSpawn = undefined;
    }).catch(() => {});
    return operation;
  }

  /** Receive the actual command outcome; HTTP timeout never serves as lease cancellation. */
  event(event: RustBackgroundEvent): void {
    const pending = this.pending;
    if (!pending || event.commandSequence !== pending.sequence ||
        (event.kind !== 'savedBrainSpawned' && event.kind !== 'commandRejected')) return;
    clearTimeout(pending.timer); this.pending = undefined; this.activeSpawn = undefined;
    const publication = event.savedBrainSpawn;
    const receipt = event.kind === 'savedBrainSpawned' && publication && Number.isSafeInteger(publication.snakeId) &&
      publication.snakeId > 0 && publication.snakeId <= 0xffff_ffff && /^[0-9a-f]{16}$/u.test(publication.effectiveStep) && publication.effectiveStep !== '0000000000000000'
      ? {schemaVersion: 1 as const, ...pending.request, brainId: pending.brainId, commandSequence: pending.sequence,
        snakeId: publication.snakeId, effectiveStep: publication.effectiveStep} : undefined;
    this.deploymentSequence = pending.sequence;
    const error = event.kind === 'commandRejected' ? event.rejectionDetail ?? 'native saved brain command rejected'
      : 'worker native saved brain receipt mismatch; deployment outcome is uncertain';
    if (!receipt && event.kind === 'savedBrainSpawned') this.failure = new Error(error);
    this.outcomes.set(pending.request.requestId, {key: pending.key, ...(receipt ? {receipt} : {error})});
    while (this.outcomes.size > MAX_RECEIPTS) this.outcomes.delete(this.outcomes.keys().next().value!);
    if (receipt) pending.resolve(receipt); else pending.reject(new Error(error));
    const release = this.persistence.savedBrainCatalog({action: 'release', leaseId: pending.leaseId}).then(() => {}, error => {
      this.failure = new Error('saved brain worker lease release failed: ' + String(error)); throw error;
    });
    this.releasing = release;
    void release.finally(() => { if (this.releasing === release) this.releasing = undefined; }).catch(() => {});
  }

  /** Join native before releasing an unknown pending command's files, then close cold work. */
  async close(): Promise<void> {
    this.closed = true;
    if (this.pending) {
      this.native().requestStop(); await this.native().join();
      const pending = this.pending; this.pending = undefined;
      if (pending) {
        clearTimeout(pending.timer); pending.reject(new Error('saved brain library closing'));
        await this.persistence.savedBrainCatalog({action: 'release', leaseId: pending.leaseId});
      }
    }
    await this.preparation?.catch(() => {});
    await this.releasing?.catch(() => {});
    await this.exportLease?.release();
  }

  /** Reject late preparation before it can submit a command into a replaced authority. */
  private requireOpen(): void {
    if (this.closed) throw new Error('saved brain library closing');
    if (this.failure) throw this.failure;
  }
  /** One bounded cold transfer/deployment; server archive operations share the outer gate. */
  private requireExclusive(): void {
    this.requireOpen();
    if (this.busy || !this.available()) throw new Error('saved brain library busy or authority unavailable');
  }
  /** Fixed exact digest filenames never accept user filesystem paths. */
  private identity(brainId: string): void { distillationHex(brainId, 64, 'saved brain identity'); }
  /** Display names are bounded UTF-8 metadata, never paths or download filenames. */
  private name(value: unknown): string {
    const body = this.body(value, ['name']);
    if (typeof body['name'] !== 'string' || !body['name'].trim() || body['name'].includes('\0') ||
        Buffer.byteLength(body['name'].trim()) > 256) throw new TypeError('invalid saved brain name');
    return body['name'].trim();
  }

  /** Strict records reject hidden operation/path or deployment fields. */
  private body(value: unknown, keys: string[]): Record<string, unknown> {
    if (!value || typeof value !== 'object' || Array.isArray(value) || Object.keys(value).length !== keys.length ||
        keys.some(key => !Object.hasOwn(value, key))) throw new TypeError('invalid saved brain request fields');
    return value as Record<string, unknown>;
  }
  /** Normalize field order before immutable request fingerprints and exact scope comparison. */
  private spawnRequest(value: unknown): SavedBrainSpawnRequestV1 {
    const body = this.body(value, ['requestId','expectedProcessEpoch','expectedDeploymentSequence','expectedRunId','expectedConfigRevision','expectedWorldEpoch']);
    const requestId = distillationHex(body['requestId'], 32, 'saved brain request', true);
    const expectedProcessEpoch = distillationHex(body['expectedProcessEpoch'], 32, 'saved brain process', true);
    if (expectedProcessEpoch !== this.processEpoch) throw new Error('saved brain process changed; refresh library metadata');
    const run = body['expectedRunId'];
    if (typeof run !== 'string' || !run || run.includes('\0') || Buffer.byteLength(run) > 256) throw new TypeError('invalid saved brain run');
    return {requestId, expectedProcessEpoch, expectedRunId: run,
      expectedDeploymentSequence: distillationHex(body['expectedDeploymentSequence'], 16, 'saved brain deployment sequence'),
      expectedConfigRevision: distillationHex(body['expectedConfigRevision'], 16, 'saved brain config revision', true),
      expectedWorldEpoch: distillationHex(body['expectedWorldEpoch'], 16, 'saved brain world epoch', true)};
  }
}
