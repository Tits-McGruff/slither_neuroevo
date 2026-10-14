import type { SavedBrainPageV1, SavedBrainRecordV1, SavedBrainSpawnReceiptV1,
  SavedBrainSpawnRequestV1, SavedBrainDeploymentScopeV1, SavedBrainSummaryV1 } from '../../server/protocol.ts';
import { MAX_BRAIN_ARTIFACT_BYTES } from '../protocol/brainArtifacts.ts';

/** Native scalar metadata needed for one new explicit deployment. */
export interface BrainLibraryPage {
  /** Frozen wire revision. */
  schemaVersion: 1;
  /** New server processes invalidate earlier pending request bodies. */
  processEpoch: string;
  /** Compare-and-swap cursor for explicit deployment. */
  deploymentSequence: string;
  /** Exact current native scope, kept as opaque strings. */
  scope: SavedBrainDeploymentScopeV1;
  /** Bounded small records in canonical identity order. */
  page: SavedBrainPageV1;
}

/** One uncertain response retains the original destination and complete request. */
interface PendingSpawn {
  /** Exact server destination, independent of later UI reconnects. */
  base: string;
  /** Exact saved artifact selected by the user. */
  brainId: string;
  /** Original immutable request reused after an uncertain response. */
  body: SavedBrainSpawnRequestV1;
}

/** Bound UTF-8 metadata without importing Node parsers or model execution. */
const MAX_METADATA_BYTES = 1200 * 1024;
/** Fixed complete portable-file digest. */
const SHA = /^[0-9a-f]{64}$/u;
/** Exact full-width unsigned scalar identity. */
const U64 = /^[0-9a-f]{16}$/u;

/** Admit a metadata object; this never decodes portable neural bytes. */
function object(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('Invalid brain library metadata.');
  return value as Record<string, unknown>;
}

/** Small scalar record validation protects DOM presentation and opaque request identities. */
function summary(value: unknown): SavedBrainSummaryV1 {
  const row = object(value); const artifact = object(row['artifact']);
  if (row['schemaVersion'] !== 1 || typeof row['brainId'] !== 'string' || !SHA.test(row['brainId']) ||
      row['brainId'] !== artifact['artifactSha256'] || typeof row['name'] !== 'string' ||
      !row['name'].trim() || new TextEncoder().encode(row['name']).length > 256 ||
      !Number.isInteger(artifact['inputSize']) || Number(artifact['inputSize']) < 1 || Number(artifact['inputSize']) > 504 ||
      !Number.isInteger(artifact['parameterCount']) || Number(artifact['parameterCount']) < 0 || Number(artifact['parameterCount']) > 1_000_000 ||
      typeof artifact['sensorSha256'] !== 'string' || !SHA.test(artifact['sensorSha256']) ||
      typeof artifact['controlIntervalSeconds'] !== 'number' || !Number.isFinite(artifact['controlIntervalSeconds']) ||
      artifact['controlIntervalSeconds'] < 0.008 || artifact['controlIntervalSeconds'] > 0.06 ||
      !['reset','preserve'].includes(String(artifact['lifeStateRule'])) ||
      !['distilled','imported','manual'].includes(String(artifact['sourceKind']))) throw new Error('Invalid saved brain record.');
  return row as unknown as SavedBrainSummaryV1;
}

/** Presentation only; Rust independently checks all graph, sensor and packing semantics. */
function record(value: unknown, expected?: string): SavedBrainRecordV1 {
  const row = summary(value) as SavedBrainRecordV1;
  if ((expected !== undefined && row.brainId !== expected) || typeof row.artifact.architectureKey !== 'string' ||
      !row.artifact.architectureKey || row.graphManifest?.['schemaVersion'] !== 1 || row.provenance?.schemaVersion !== 1 ||
      row.provenance.sourceKind !== row.artifact.sourceKind) throw new Error('Invalid saved brain details.');
  return row;
}

/** Pure byte routing/metadata client; pending deployment bodies survive uncertain HTTP replies. */
export class BrainLibraryClient {
  /** Prevent concurrent status preparation from allocating two new request identities. */
  private starting = false;
  /** At most one explicit deployment is awaiting confirmation. */
  private pending: PendingSpawn | undefined;

  /** Resolve only the currently connected server; no stored browser model or fallback exists. */
  constructor(private readonly base: () => string | null) {}

  /** The UI must offer retry instead of silently issuing a replacement spawn. */
  hasPendingSpawn(): boolean { return this.pending !== undefined; }

  /** Explicit dismissal only forgets the local retry body; it never cancels native work. */
  dismissPendingSpawn(): void { this.pending = undefined; }

  /** Read one bounded page and a fresh authoritative deployment scope. */
  async list(after: string | null = null): Promise<BrainLibraryPage> {
    if (after !== null && !SHA.test(after)) throw new Error('Invalid library page.');
    const raw = object(await this.json(this.currentBase(), '/library?limit=25' + (after ? '&after=' + after : '')));
    const scope = object(raw['scope']); const page = object(raw['page']);
    if (raw['schemaVersion'] !== 1 || typeof raw['processEpoch'] !== 'string' || !/^[0-9a-f]{32}$/u.test(raw['processEpoch']) ||
        typeof raw['deploymentSequence'] !== 'string' || !U64.test(raw['deploymentSequence']) ||
        typeof scope['expectedRunId'] !== 'string' || !scope['expectedRunId'] || scope['expectedRunId'].length > 256 ||
        typeof scope['expectedConfigRevision'] !== 'string' || !U64.test(scope['expectedConfigRevision']) ||
        typeof scope['expectedWorldEpoch'] !== 'string' || !U64.test(scope['expectedWorldEpoch']) ||
        !Array.isArray(page['entries']) || page['entries'].length > 25 ||
        (page['next'] !== null && (typeof page['next'] !== 'string' || !SHA.test(page['next'])))) throw new Error('Invalid brain library page.');
    const entries = page['entries'].map(summary);
    if (entries.some((entry, index) => (index > 0 && entry.brainId <= entries[index - 1]!.brainId) || (after !== null && entry.brainId <= after)) ||
        (page['next'] !== null && page['next'] !== entries.at(-1)?.brainId)) throw new Error('Invalid brain library pagination.');
    return {...raw, scope: {expectedRunId: scope['expectedRunId'], expectedConfigRevision: scope['expectedConfigRevision'],
      expectedWorldEpoch: scope['expectedWorldEpoch']}, page: {...page, entries}} as unknown as BrainLibraryPage;
  }

  /** Fetch one full native-built graph/provenance record for presentation. */
  async read(brainId: string): Promise<SavedBrainRecordV1> {
    this.id(brainId); return record(await this.json(this.currentBase(), '/library/' + brainId), brainId);
  }

  /** Save requires an explicit selected candidate and label; upload alone never saves it. */
  async saveCandidate(brainId: string, name: string): Promise<SavedBrainRecordV1> {
    this.id(brainId);
    return record(await this.json(this.currentBase(), `/library/${brainId}/save`, {method: 'POST',
      headers: {'Content-Type': 'application/json'}, body: JSON.stringify({name})}), brainId);
  }

  /** Send the original File; no arrayBuffer, JSON population or model reconstruction. */
  async importFile(file: File, name: string): Promise<SavedBrainRecordV1> {
    if (file.size < 88 || file.size > MAX_BRAIN_ARTIFACT_BYTES) throw new Error('Choose a .slither-brain file up to 64 MiB.');
    return record(await this.json(this.currentBase(), '/library/import?name=' + encodeURIComponent(name), {
      method: 'POST', headers: {'Content-Type': 'application/octet-stream'}, body: file}));
  }

  /** Delete membership only; running native weights remain valid. */
  async delete(brainId: string): Promise<void> {
    this.id(brainId); const result = object(await this.json(this.currentBase(), '/library/' + brainId, {method: 'DELETE'}));
    if (result['brainId'] !== brainId || typeof result['deleted'] !== 'boolean') throw new Error('Invalid library deletion response.');
  }

  /** Direct attachment URL, preserving browser streaming and the original artifact. */
  exportUrl(brainId: string): string { this.id(brainId); return this.currentBase() + '/api/distillation/v1/library/' + brainId + '/export'; }

  /** New explicit action first refreshes the exact native scope and deployment cursor. */
  async spawn(brainId: string): Promise<SavedBrainSpawnReceiptV1> {
    this.id(brainId);
    if (this.pending || this.starting) throw new Error('Retry the pending spawn before requesting another.');
    this.starting = true;
    try {
      const base = this.currentBase(); const status = await this.list();
      if (this.currentBase() !== base) throw new Error('Server connection changed. Refresh the library.');
      const body = {...status.scope, requestId: crypto.randomUUID().replaceAll('-', ''),
        expectedProcessEpoch: status.processEpoch, expectedDeploymentSequence: status.deploymentSequence};
      this.pending = {base, brainId, body};
      return await this.retrySpawn();
    } finally { this.starting = false; }
  }

  /** Uncertain replies always reuse the original server, artifact and immutable request ID. */
  async retrySpawn(): Promise<SavedBrainSpawnReceiptV1> {
    const pending = this.pending;
    if (!pending) throw new Error('There is no pending spawn.');
    if (this.currentBase() !== pending.base) throw new Error('Reconnect to the original server to retry this spawn.');
    try {
      const result = object(await this.json(pending.base, `/library/${pending.brainId}/spawn`, {
        method: 'POST', headers: {'Content-Type': 'application/json'}, body: JSON.stringify(pending.body)}));
      if (result['schemaVersion'] !== 1 || result['brainId'] !== pending.brainId ||
          Object.entries(pending.body).some(([key, value]) => result[key] !== value) ||
          !Number.isSafeInteger(result['snakeId']) || Number(result['snakeId']) < 1 || Number(result['snakeId']) > 0xffff_ffff ||
          typeof result['effectiveStep'] !== 'string' || !U64.test(result['effectiveStep']) ||
          typeof result['commandSequence'] !== 'string' || !U64.test(result['commandSequence'])) throw new Error('Invalid spawn confirmation; retain this request for retry.');
      if (this.pending === pending) this.pending = undefined;
      return result as unknown as SavedBrainSpawnReceiptV1;
    } catch (error) {
      const status = error instanceof Error && 'httpStatus' in error ? Number(error.httpStatus) : 0;
      // These responses confirm rejection; an absent, malformed or 503 reply remains uncertain.
      if ([400,404,409,410,429].includes(status) && this.pending === pending) this.pending = undefined;
      throw error;
    }
  }

  /** Avoid following an old cached receipt after an authoritative world replacement. */
  async isCurrent(receipt: SavedBrainSpawnReceiptV1): Promise<boolean> {
    const status = await this.list();
    return receipt.expectedProcessEpoch === status.processEpoch &&
      Object.entries(status.scope).every(([key, value]) => receipt[key as keyof SavedBrainSpawnReceiptV1] === value);
  }

  /** Require a current connection before resolving any operation destination. */
  private currentBase(): string {
    const base = this.base(); if (!base) throw new Error('Connect to a server to use the brain library.'); return base.replace(/\/$/u, '');
  }
  /** Fixed opaque identifiers cannot become paths or query fragments. */
  private id(value: string): void { if (!SHA.test(value)) throw new Error('Invalid brain identity.'); }

  /** Bound actual streamed JSON bytes and cancel oversized responses before DOM presentation. */
  private async json(base: string, path: string, init?: RequestInit): Promise<unknown> {
    const response = await fetch(base + '/api/distillation/v1' + path, {...init, signal: AbortSignal.timeout(35_000)});
    if (!response.body) throw new Error('Brain library response is empty.');
    const reader = response.body.getReader(); const decoder = new TextDecoder();
    let bytes = 0; let text = '';
    try {
      for (;;) {
        const part = await reader.read(); if (part.done) break;
        bytes += part.value.byteLength;
        if (bytes > MAX_METADATA_BYTES) { await reader.cancel(); throw new Error('Brain library metadata is too large.'); }
        text += decoder.decode(part.value, {stream: true});
      }
      text += decoder.decode();
    } finally { reader.releaseLock(); }
    const value = JSON.parse(text) as unknown;
    if (!response.ok) throw Object.assign(new Error(String(object(value)['error'] ?? 'Brain library request failed.')), {httpStatus: response.status});
    return value;
  }
}
