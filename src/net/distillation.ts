import { distillationHex, distillationInteger, type DistillationDataset, type DistillationDatasetSummary, type DistillationDatasetPage } from '../protocol/distillation.ts';
import { parseShadowMetrics, parseShadowReceipt } from '../protocol/shadowMetadata.ts';
import type { ShadowPublication, ShadowReceipt } from '../protocol/shadows.ts';

/** Applied authority scope; editable settings and graph drafts never supply identities. */
export interface DistillationScope {
  /** Connected HTTP destination. */
  base: string;
  /** Current authoritative run. */
  runId: string;
  /** Exact applied configuration word. */
  configRevision: string;
}

/** An uncertain shadow response retains one exact original request for explicit retry. */
interface PendingShadow {
  /** Original connected scope, never retargeted after reconnect. */
  scope: DistillationScope;
  /** Original immutable route. */
  path: string;
  /** Original idempotent command body. */
  body: Record<string, unknown>;
  /** Exact echoed receipt fields required for confirmation. */
  expected: Partial<ShadowReceipt>;
}

/** Require a small metadata record without interpreting model/sample bytes. */
function object(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('Invalid distillation metadata.');
  return value as Record<string, unknown>;
}

/** Validate the scalar durable status used by the browser. */
function dataset(value: unknown, epoch?: string): DistillationDataset {
  const raw = object(value); const contract = object(raw['contract']);
  const id = distillationHex(contract['captureEpoch'], 32, 'capture epoch', true);
  if (epoch !== undefined && epoch !== id) throw new Error('Dataset identity changed.');
  if (raw['schemaVersion'] !== 1 || !['active', 'stopped', 'interrupted'].includes(String(raw['state'])) ||
      typeof contract['runId'] !== 'string' || !contract['runId'] || contract['runId'].length > 128 ||
      (raw['error'] !== null && (typeof raw['error'] !== 'string' || raw['error'].length > 1024))) throw new Error('Invalid dataset status.');
  for (const key of ['configSha256', 'sensorSha256', 'teacherSha256']) distillationHex(contract[key], 64, key);
  for (const key of ['productionWatermark', 'storageWatermark', 'retentionFloor', 'resolvedThrough',
    'nativeDroppedChunks', 'nativeDroppedSamples', 'nativeDroppedBytes', 'spoolDroppedChunks', 'retainedBytes']) distillationHex(raw[key], 16, key);
  distillationInteger(raw['retainedChunks'], 0, 65_536, 'retained chunks');
  distillationInteger(contract['teacherRevision'], 2, 2, 'teacher revision');
  distillationInteger(contract['teacherMode'], 0, 1, 'teacher mode');
  distillationInteger(contract['inputSize'], 1, 504, 'input width');
  const cadence = contract['controlIntervalSeconds'];
  if (typeof cadence !== 'number' || !Number.isFinite(cadence) || cadence < 0.008 || cadence > 0.06 ||
      typeof raw['sensorContractJson'] !== 'string' || new TextEncoder().encode(raw['sensorContractJson']).length > 65_536 ||
      (raw['runSeed'] !== null && (!Number.isInteger(raw['runSeed']) || Number(raw['runSeed']) < 0 || Number(raw['runSeed']) > 0xffff_ffff))) throw new Error('Invalid dataset contract.');
  return raw as unknown as DistillationDataset;
}

/** Dedicated metadata/command client; never joins a player, downloads samples or executes a graph. */
export class DistillationClient {
  /** At most one uncertain immutable shadow command. */
  private pending: PendingShadow | undefined;
  /** Serialize command preparation before a pending identity exists. */
  private busy = false;
  /** A server-assigned capture epoch cannot safely be recovered by replaying start. */
  private uncertainStart = false;

  /** Resolve only applied state from the currently connected authority. */
  constructor(private readonly scope: () => DistillationScope | null) {}

  /** Keep uncertain commands visible until confirmed or explicitly dismissed. */
  hasPendingShadow(): boolean { return this.pending !== undefined; }

  /** A lost capture-start response needs discovery or explicit local dismissal. */
  hasUncertainCapture(): boolean { return this.uncertainStart; }

  /** Explicit dismissal never cancels or replays an admitted capture start. */
  dismissUncertainCapture(): void { this.uncertainStart = false; }

  /** Forget a local retry body without cancelling any already admitted native command. */
  dismissPendingShadow(): void { this.pending = undefined; }

  /** Read one bounded page; callers must paginate before assuming an active epoch is absent. */
  async datasets(after = '0'.repeat(32)): Promise<DistillationDatasetPage> {
    distillationHex(after, 32, 'dataset cursor');
    const raw = object(await this.json(this.current(), '/datasets?limit=25&after=' + after));
    if (!Array.isArray(raw['datasets']) || raw['datasets'].length > 25 || typeof raw['hasMore'] !== 'boolean') throw new Error('Invalid dataset page.');
    const next = distillationHex(raw['nextCursor'], 32, 'dataset cursor');
    const rows = raw['datasets'].map(value => {
      const row = object(value);
      distillationHex(row['captureEpoch'], 32, 'capture epoch', true);
      for (const key of ['productionWatermark', 'storageWatermark', 'resolvedThrough']) distillationHex(row[key], 16, key);
      if (typeof row['runId'] !== 'string' || !row['runId'] || row['runId'].length > 128 ||
          !['active', 'stopped', 'interrupted'].includes(String(row['state']))) throw new Error('Invalid dataset summary.');
      return row as unknown as DistillationDatasetSummary;
    });
    if (rows.some((row, index) => row.captureEpoch <= (index ? rows[index - 1]!.captureEpoch : after)) ||
        next !== (rows.at(-1)?.captureEpoch ?? after) || (raw['hasMore'] && rows.length === 0)) throw new Error('Invalid dataset chronology.');
    return { datasets: rows, nextCursor: next, hasMore: raw['hasMore'] };
  }

  /** Read only a selected dataset manifest; no chunk lease or payload is acquired. */
  async manifest(epoch: string): Promise<DistillationDataset> {
    distillationHex(epoch, 32, 'capture epoch', true);
    return dataset(await this.json(this.current(), '/datasets/' + epoch + '/manifest'), epoch);
  }

  /** Start is explicit and never automatically retried because the server assigns its epoch. */
  async start(slots: number[]): Promise<DistillationDataset> {
    if (slots.length < 1 || slots.length > 120 || slots.some((slot, index) =>
      !Number.isInteger(slot) || slot < 0 || slot > 119 || (index > 0 && slot <= slots[index - 1]!))) throw new Error('Choose unique increasing teacher slots from 0 through 119.');
    if (this.uncertainStart) throw new Error('Discover the previous capture before starting another.');
    return this.command(async () => {
      const scope = this.current();
      try {
        const result = dataset(await this.json(scope, '/capture/start', this.post({
          expectedRunId: scope.runId, expectedConfigRevision: scope.configRevision, baselineSlots: slots,
          pendingBytes: 16 * 1024 ** 2, queueBytes: 16 * 1024 ** 2, queueChunks: 1024
        })));
        if (result.contract.runId !== scope.runId) throw new Error('Capture confirmation belongs to another run.');
        return result;
      } catch (error) {
        const status = error instanceof Error && 'httpStatus' in error ? Number(error.httpStatus) : 0;
        if (![400, 404, 409, 410, 429].includes(status)) this.uncertainStart = true;
        throw error;
      }
    });
  }

  /** Stop binds the selected exact epoch; a later capture cannot be stopped by this request. */
  async stop(epoch: string): Promise<DistillationDataset> {
    distillationHex(epoch, 32, 'capture epoch', true);
    return this.command(async () => dataset(await this.json(this.current(), '/capture/' + epoch + '/stop', this.post({})), epoch));
  }

  /** Four native summaries are fetched on demand, with the shared strict scalar parser. */
  async shadows(): Promise<ShadowPublication> {
    const raw = object(await this.json(this.current(), '/shadows'));
    const processEpoch = distillationHex(raw['processEpoch'], 32, 'shadow process', true);
    const { processEpoch: _epoch, ...metrics } = raw; void _epoch;
    return { ...parseShadowMetrics(JSON.stringify(metrics)), processEpoch };
  }

  /** Prepare a fresh segment from an explicitly refreshed compare-and-swap publication. */
  async activate(slot: number, teacher: number, artifact: string, epoch: string, status: ShadowPublication): Promise<ShadowReceipt> {
    distillationInteger(slot, 0, 3, 'shadow slot'); distillationInteger(teacher, 0, 119, 'teacher slot');
    distillationHex(artifact, 64, 'candidate digest'); distillationHex(epoch, 32, 'capture epoch', true);
    return this.command(async () => {
      const scope = this.current(); const requestId = crypto.randomUUID().replaceAll('-', '');
      const previousActivationSequence = status.slots[slot]?.activationSequence ?? '0'.repeat(16);
      this.pending = { scope, path: '/shadows/' + slot + '/activate',
        body: { requestId, expectedProcessEpoch: status.processEpoch, expectedActivationSequence: previousActivationSequence,
          expectedRunId: scope.runId, expectedConfigRevision: scope.configRevision, captureEpoch: epoch, baselineSlot: teacher, artifactSha256: artifact },
        expected: { requestId, processEpoch: status.processEpoch, previousActivationSequence, runId: scope.runId,
          configRevision: scope.configRevision, captureEpoch: epoch, baselineSlot: teacher, artifactSha256: artifact, shadowSlot: slot, enabled: true } };
      return this.retryShadow();
    });
  }

  /** A delayed stop is pinned to the observed activation and capture, never a replacement. */
  async stopShadow(slot: number, status: ShadowPublication): Promise<ShadowReceipt> {
    distillationInteger(slot, 0, 3, 'shadow slot');
    const active = status.slots[slot];
    if (!active?.enabled) throw new Error('Refresh and select an active shadow.');
    return this.command(async () => {
      const scope = this.current(); const requestId = crypto.randomUUID().replaceAll('-', '');
      this.pending = { scope, path: '/shadows/' + slot + '/stop',
        body: { requestId, captureEpoch: active.captureEpoch, activationSequence: active.activationSequence },
        expected: { requestId, processEpoch: status.processEpoch, runId: scope.runId, captureEpoch: active.captureEpoch,
          activationSequence: active.activationSequence, previousActivationSequence: active.activationSequence,
          artifactSha256: active.artifactSha256, baselineSlot: active.baselineSlot, shadowSlot: slot, enabled: false } };
      return this.retryShadow();
    });
  }

  /** Retry only the immutable original body and destination after an uncertain reply. */
  async retryShadow(): Promise<ShadowReceipt> {
    const pending = this.pending;
    if (!pending) throw new Error('There is no pending shadow command.');
    const scope = this.current();
    if (JSON.stringify(scope) !== JSON.stringify(pending.scope)) throw new Error('Authority changed. Dismiss this pending request and refresh.');
    try {
      const receipt = parseShadowReceipt(await this.json(pending.scope, pending.path, this.post(pending.body)));
      if (Object.entries(pending.expected).some(([key, value]) => receipt[key as keyof ShadowReceipt] !== value)) throw new Error('Shadow confirmation differs; retain the request for retry.');
      if (this.pending === pending) this.pending = undefined;
      return receipt;
    } catch (error) {
      const status = error instanceof Error && 'httpStatus' in error ? Number(error.httpStatus) : 0;
      if ([400, 404, 409, 410, 429].includes(status) && this.pending === pending) this.pending = undefined;
      throw error;
    }
  }

  /** Reject overlapping explicit mutations while preserving an uncertain shadow request. */
  private async command<T>(action: () => Promise<T>): Promise<T> {
    if (this.busy || this.pending) throw new Error('Resolve the pending diagnostic command first.');
    this.busy = true;
    try { return await action(); } finally { this.busy = false; }
  }

  /** Snapshot applied identities; never retain a mutable provider object. */
  private current(): DistillationScope {
    const current = this.scope();
    if (!current) throw new Error('Connect to a server supporting distillation controls.');
    distillationHex(current.configRevision, 16, 'configuration revision', true);
    return { base: current.base.replace(/\/$/u, ''), runId: current.runId, configRevision: current.configRevision };
  }

  /** Construct a small JSON command without model/sample bytes. */
  private post(body: object): RequestInit {
    return { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) };
  }

  /** Bound actual streamed bytes and release response ownership on every outcome. */
  private async json(scope: DistillationScope, path: string, init?: RequestInit): Promise<unknown> {
    const response = await fetch(scope.base + '/api/distillation/v1' + path, { ...init, signal: AbortSignal.timeout(35_000) });
    if (!response.body) throw new Error('Empty distillation response.');
    const reader = response.body.getReader(); const decoder = new TextDecoder();
    let bytes = 0; let text = '';
    try {
      for (;;) {
        const part = await reader.read(); if (part.done) break;
        bytes += part.value.byteLength;
        if (bytes > 1024 * 1024) { await reader.cancel(); throw new Error('Distillation metadata exceeds its limit.'); }
        text += decoder.decode(part.value, { stream: true });
      }
      text += decoder.decode();
    } finally { reader.releaseLock(); }
    const value = JSON.parse(text) as unknown;
    if (!response.ok) throw Object.assign(new Error(String(object(value)['error'] ?? 'Distillation request failed.')), { httpStatus: response.status });
    return value;
  }
}
