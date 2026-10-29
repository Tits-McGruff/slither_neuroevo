import { distillationSpoolLimitsForGiB } from './rustEngine/distillationProtocol.ts';
import { DecisionInspectorRouting } from './rustEngine/decisionInspector.ts';
import type {
  RustBackgroundVisualization,
  RustImportBranchNotice,
  RustLegacyConversionNotice,
  RustRecoveryNotice
} from '../src/protocol/rustBackground.ts';
import type { GraphSpec } from '../src/brains/graph/schema.ts';
import type { ExperimentalServerRuntime } from './rustEngine/experimentalStartup.ts';
import { createServer } from 'node:http';
import { createReadStream, existsSync } from 'node:fs';
import { lstat, readdir, unlink } from 'node:fs/promises';
import { dirname, resolve, sep } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { networkInterfaces } from 'node:os';
import { isIP } from 'node:net';
import { randomBytes, randomUUID } from 'node:crypto';
import { type ServerConfig } from './config.ts';
import { WsHub } from './wsHub.ts';
import { parseProductionCli, PRODUCTION_CLI_HELP } from './productionCli.ts';
import { serveBrowserAsset } from './browserAssets.ts';
import { admitBrowserRequest, createBrowserOriginPolicy } from './browserOrigins.ts';
import { assertReplacementCheckpointBudget, createExperimentalServerRuntime } from './rustEngine/experimentalStartup.ts';
import { BackgroundOutputPump } from './rustEngine/backgroundOutput.ts';
import { DistillationSpoolClient } from './rustEngine/distillationSpoolClient.ts';
import { DistillationCapture } from './rustEngine/distillationCapture.ts';
import { DistillationHttpRouter } from './rustEngine/distillationHttp.ts';
import { SavedBrainLibrary } from './rustEngine/savedBrainLibrary.ts';
import type { BackgroundCommandAdmission } from './rustEngine/commandAdmission.ts';
import { ExternalControllerRouting } from './rustEngine/externalRouting.ts';
import { createRustSettings, createRustStats, createRustWelcome, wireInteger } from './rustEngine/browserMetadata.ts';
import { ExperimentalRuntimeTelemetry } from './rustEngine/runtimeTelemetry.ts';
import type { CheckpointRetentionInventory } from './rustEngine/checkpointRetention.ts';
import type {
  ManagedBrowserHallOfFameEntry,
  ManagedBrowserHistoryEntry,
  ManagedCheckpointExportLease,
  ManagedImportBranchResult,
  ManagedStorageDiagnostics,
  SavedBrainArchiveEntryV1
} from './rustEngine/checkpointPersistenceProtocol.ts';
import {
  parseManagedCheckpointDescriptor,
  parseManagedImportInventoryDescriptor,
  parseSavedBrainArchiveEntries
} from './rustEngine/checkpointPersistenceProtocol.ts';
import {
  parseArchiveContentLength,
  P0_ARCHIVE_UPLOAD_LIMIT,
  spoolArchiveUpload
} from './rustEngine/archiveUpload.ts';
import {
  admitDiskOperation,
  inspectManagedDisk,
  type ManagedDiskDiagnostics
} from './rustEngine/diskAdmission.ts';
import { parseRustStartupMetadata } from './rustEngine/startupMetadata.ts';
import { watchArchiveWork } from './rustEngine/archiveWorkWatchdog.ts';
import { watchRunningAuthority } from './rustEngine/authorityProgressWatchdog.ts';
import type { GodModeMsg, LiveSettingsMsg, NewRunMsg, ResetMsg } from './protocol.ts';
import { readJsonBody } from './readJsonBody.ts';
import { clearedStackGraph, normalizeStackSetting, STACK_SETTING_PATHS } from './rustEngine/stackGraph.ts';
import {
  getLiveSettingDefinition,
  normalizeLiveSettingsUpdates,
  type LiveSettingsUpdate
} from '../src/protocol/settings.ts';
import { normalizeSettingValue, EVOLVED_SETTING_PATHS } from '../src/protocol/settingDefinitions.ts';

/** Repository-owned built browser assets. */
const CLIENT_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../dist');
/** P0 external routing cap, within the native admitted controller capacity. */
const MAX_CONTROLLERS = 16;
/** Maximum discard interval after the complete rejection body is already sent. */
const ARCHIVE_REJECTION_DRAIN_MS = 1000;

/** Convert exact worker/filesystem counters to JSON-safe base-10 health fields. */
function storageHealthPayload(
  sqlite: ManagedStorageDiagnostics,
  managed: ManagedDiskDiagnostics
): Record<string, unknown> {
  const decimal = (value: string): string => BigInt(`0x${value}`).toString();
  return {
    schemaVersion: 1,
    sqlite: {
      databaseBytes: decimal(sqlite.databaseByteCount),
      walBytes: decimal(sqlite.walByteCount),
      shmBytes: decimal(sqlite.shmByteCount),
      pageSizeBytes: decimal(sqlite.pageSizeByteCount),
      pageCount: decimal(sqlite.pageCount),
      freelistPageCount: decimal(sqlite.freelistPageCount),
      usedPageBytes: decimal(sqlite.usedPageByteCount)
    },
    managed: {
      temporaryBytes: managed.tempByteCount.toString(),
      temporaryQuotaBytes: managed.tempQuotaByteCount.toString(),
      freeBytes: managed.freeByteCount.toString(),
      operatingReserveBytes: managed.operatingReserveByteCount.toString()
    }
  };
}

/** Reject an export before writing when its scalar worst-case files do not fit. */
async function admitExportSpace(directory: string, lease: ManagedCheckpointExportLease, selectedBytes = 0n): Promise<void> {
  const population = BigInt(`0x${lease.descriptor.populationCount}`);
  const weights = BigInt(`0x${lease.descriptor.weightCount}`);
  const hallOfFameCount = BigInt(`0x${lease.inventory.hallOfFameCount}`);
  if (population === 0n || weights % population !== 0n) {
    throw new Error('export checkpoint has an invalid population weight shape');
  }
  const hallOfFameRawBytes = hallOfFameCount * (weights / population) * 4n;
  const projectedAdditionalBytes = BigInt(`0x${lease.descriptor.storedByteCount}`) +
    BigInt(`0x${lease.inventory.storedByteCount}`) + hallOfFameRawBytes * 2n + selectedBytes + 16n * 1024n * 1024n;
  await admitDiskOperation(directory, {
    operation: 'export',
    sourceSpoolBytes: 0n,
    candidateSpoolBytes: projectedAdditionalBytes,
    finalManagedBytes: 0n
  });
}

/** Deliver a complete bounded error before closing a socket that may still carry upload bytes. */
async function rejectArchiveUpload(
  request: import('node:http').IncomingMessage,
  response: import('node:http').ServerResponse,
  status: number,
  result: { ok: false; message: string; code?: string }
): Promise<void> {
  const body = JSON.stringify(result);
  /** Finish on consumed input, peer closure, or a fixed deadline; empty chunks cannot extend it. */
  const discarded = new Promise<void>(done => {
    const finish = (): void => {
      clearTimeout(deadline);
      request.off('end', finish);
      request.off('close', finish);
      response.off('close', finish);
      done();
    };
    const deadline = setTimeout(finish, ARCHIVE_REJECTION_DRAIN_MS);
    deadline.unref();
    request.once('end', finish);
    request.once('close', finish);
    response.once('close', finish);
    if (request.readableEnded || request.destroyed || response.destroyed) finish();
  });
  request.resume();
  if (!response.destroyed) {
    response.writeHead(status, { 'Content-Type': 'application/json',
      'Content-Length': Buffer.byteLength(body), 'Connection': 'close' });
    // A known length lets the client finish reading the error before end() closes pending input.
    response.write(body);
  }
  await discarded;
  if (!response.destroyed) response.end();
}

/** Rust-authoritative process ownership returned to tests and the CLI. */
export interface RustServer {
  /** Actual bound port, including an OS-selected test port. */
  port: number;
  /** Explicit health-only startup failure, without an active simulation. */
  startupFault?: string;
  /** Stop sockets, join native execution, then close the metadata worker. */
  close(): Promise<void>;
}

/** Keep bounded diagnostics reachable after failed restore without starting any game or socket authority. */
async function startFaultedServer(config: ServerConfig, error: unknown): Promise<RustServer> {
  const reason = (error instanceof Error ? error.message : String(error)).slice(0, 512);
  const browserOrigins = createBrowserOriginPolicy(config);
  const server = createServer((request, response) => {
    if (!admitBrowserRequest(request, response, browserOrigins)) return;
    response.writeHead(503, { 'Content-Type': 'application/json' });
    response.end(JSON.stringify({ ok: false, authority: 'rust', lifecycle: 'startup-fault', interfaceFault: reason }));
  });
  server.on('upgrade', (_request, socket) => { socket.end('HTTP/1.1 503 Service Unavailable\r\nConnection: close\r\nContent-Length: 0\r\n\r\n'); });
  await new Promise<void>((done, reject) => { server.once('error', reject); server.listen(config.port, config.host, () => { server.off('error', reject); done(); }); });
  const address = server.address();
  if (!address || typeof address === 'string') throw new Error('faulted server has no TCP address');
  let closing: Promise<void> | undefined;
  console.error('[rust.startup-fault]', reason);
  return { port: address.port, startupFault: reason, close() {
    closing ??= new Promise<void>((done, reject) => server.close(error => error ? reject(error) : done()));
    return closing;
  } };
}

/** Project durable provenance without exposing native population or metadata-worker internals. */
function recoveryNotice(owner: ExperimentalServerRuntime): RustRecoveryNotice | undefined {
  const recovery = owner.recovery;
  if (!recovery) return undefined;
  const recovered = BigInt(`0x${recovery.recoveredDescriptor.generation}`);
  const through = BigInt(`0x${recovery.abandonedThroughGeneration}`) - 1n;
  return { failedRunId: recovery.sourceRunId, branchRunId: recovery.branchRunId,
    failedCheckpointId: recovery.failedCheckpointId, recoveredCheckpointId: recovery.recoveredDescriptor.logicalRootSha256,
    recoveredGeneration: recovery.recoveredDescriptor.generation,
    ...(recovery.compatibleBuild ? { compatibleBuild: true as const } : {}),
    ...(recovery.explicitResume ? { explicitResume: true as const } : {}),
    lostCompletedGenerations: through >= recovered ? { from: recovery.recoveredDescriptor.generation,
      through: through.toString(16).padStart(16, '0') } : null };
}

/** Small terminal archive-import response emitted only after all cleanup completes. */
interface ArchiveImportSuccess {
  /** Immutable partial-source classification when importing a converted legacy lineage. */
  legacyConversion?: RustLegacyConversionNotice;
  ok: true;
  runId: string;
  generation: string;
  completedStep: string;
  checkpointId: string;
  saveLogicalRootSha256: string;
  branched: boolean;
  sourceRunId?: string;
}

/** Maximum silence after an attachment body begins transferring. */
const ARCHIVE_DOWNLOAD_NO_PROGRESS_MS = 60_000;

/** Project durable import lineage for health and welcome messages. */
function importBranchNotice(value: ManagedImportBranchResult | null): RustImportBranchNotice | undefined {
  if (!value) return undefined;
  return { sourceRunId: value.sourceRunId, branchRunId: value.branchRunId,
    sourceGeneration: value.sourceGeneration, sourceCheckpointId: value.sourceCheckpointId };
}

/** Build one complete replacement setting request without trusting browser normalization. */
function replacementSettings(
  metadata: ExperimentalServerRuntime['metadata'],
  message?: ResetMsg
): Array<{ path: string; value: number }> {
  const current = createRustSettings(metadata);
  const core = current.core as unknown as Record<string, number>;
  const replacements = new Map<string, number>();
  const seenLifePaths = new Set<string>();
  for (const update of message?.updates ?? []) if (EVOLVED_SETTING_PATHS.has(update.path)) {
    if (seenLifePaths.has(update.path)) throw new Error('duplicate life setting: ' + update.path);
    seenLifePaths.add(update.path);
  }
  const selectedLives = message?.updates?.some(update => EVOLVED_SETTING_PATHS.has(update.path)) ?? false;
  const upgradingLives = selectedLives && !metadata.settings.some(setting => setting.path === 'evolvedRespawn.contractVersion');
  if (upgradingLives) {
    if (message?.updates?.find(update => update.path === 'evolvedRespawn.contractVersion')?.value !== 1 ||
        [...EVOLVED_SETTING_PATHS].some(path => !message.updates?.some(update => update.path === path))) {
      throw new Error('Selecting life experiments requires the complete version-one life settings.');
    }
  }
  for (const [key, value] of Object.entries(message?.settings ?? {})) {
    const definition = getLiveSettingDefinition(key);
    if (!definition || typeof value !== 'number') throw new Error(`reset setting ${key} is invalid`);
    const normalized = normalizeSettingValue(definition, value);
    if (!metadata.settings.some(setting => setting.path === key)) {
      if (message?.graphSpec === undefined) {
        if (!Object.is(core[key], normalized)) {
          throw new Error(`setting ${key} requires an explicit replacement graph`);
        }
      }
      continue;
    }
    replacements.set(key, normalized);
  }
  for (const update of message?.updates ?? []) {
    if (STACK_SETTING_PATHS.has(update.path)) {
      const value = normalizeStackSetting(update.path, update.value);
      if (message?.graphSpec === undefined && current.updates.find(setting => setting.path === update.path)?.value !== value) {
        throw new Error(`setting ${update.path} requires an explicit replacement graph`);
      }
      continue;
    }
    const definition = getLiveSettingDefinition(update.path);
    const sensorSelection = update.path === 'sense.layoutVersion' || update.path === 'sense.profile' || update.path.startsWith('sense.ray') ||
      update.path === 'baselineBots.algorithm' || update.path === 'baselineBots.behaviorMode' || update.path === 'baselineBots.controlDt';
    if (!definition || (!metadata.settings.some(setting => setting.path === update.path) &&
        !(upgradingLives && EVOLVED_SETTING_PATHS.has(update.path)) && !sensorSelection)) {
      throw new Error(`reset setting ${update.path} is not supported by the current Rust graph`);
    }
    const value = normalizeSettingValue(definition, update.value);
    if (sensorSelection && (replacements.has(update.path) || value !== update.value)) {
      throw new Error(`reset setting ${update.path} must be unique and within its declared bounds`);
    }
    replacements.set(update.path, value);
  }
  const result = metadata.settings.filter(setting => setting.path !== 'sense.contractSha256').map(setting => {
    const replacement = replacements.get(setting.path);
    const currentValue = typeof setting.value === 'boolean' ? Number(setting.value) : setting.value;
    if (typeof currentValue !== 'number' || !Number.isFinite(currentValue)) {
      throw new TypeError(`Rust fresh-run setting ${setting.path} is not numeric`);
    }
    return { path: setting.path, value: replacement ?? currentValue };
  });
  if (upgradingLives) for (const path of EVOLVED_SETTING_PATHS) result.push({ path, value: replacements.get(path)! });
  const values = new Map(result.map(setting => [setting.path, setting.value]));
  const version = replacements.get('sense.layoutVersion') ?? metadata.sensorVersion;
  const algorithm = replacements.get('baselineBots.algorithm') ?? values.get('baselineBots.controlVersion') ?? 1;
  if (algorithm === 2 && version !== 4) throw new Error('Baseline v2 requires v4 sensors.');
  if (version !== 4 && [...replacements.keys()].some(path => path === 'sense.profile' || path.startsWith('sense.ray'))) {
    throw new Error('V4 information and ray limits require v4 sensors.');
  }
  if (algorithm !== 2 && (replacements.has('baselineBots.behaviorMode') || replacements.has('baselineBots.controlDt'))) {
    throw new Error('Baseline variation and decision interval require baseline v2.');
  }
  if (version === 4) {
    for (const path of ['sense.profile', 'sense.rayMinimum', 'sense.rayCap', 'sense.rayBatch', 'sense.rayMaximumDue']) {
      const value = replacements.get(path) ?? values.get(path);
      if (value === undefined) throw new Error(`Selecting v4 requires explicit ${path}.`);
      values.set(path, value);
    }
    values.set('sense.contractVersion', 1);
  } else {
    for (const path of ['sense.contractVersion', 'sense.profile', 'sense.rayMinimum', 'sense.rayCap', 'sense.rayBatch', 'sense.rayMaximumDue']) values.delete(path);
  }
  values.set('brain.sensorVersion', version);
  if (algorithm === 2) {
    values.set('baselineBots.controlVersion', 2);
    const mode = replacements.get('baselineBots.behaviorMode') ?? values.get('baselineBots.mode');
    const cadence = replacements.get('baselineBots.controlDt') ?? values.get('baselineBots.controlDt');
    if (mode === undefined || cadence === undefined) throw new Error('Selecting baseline v2 requires explicit variation and decision interval.');
    values.set('baselineBots.mode', mode); values.set('baselineBots.controlDt', cadence);
  } else {
    for (const path of ['baselineBots.controlVersion', 'baselineBots.mode', 'baselineBots.controlDt']) values.delete(path);
  }
  return [...values].map(([path, value]) => ({path, value}));
}

/** Project the durable old-checkpoint conversion without exposing persistence internals. */
function legacyConversionNotice(
  value: ExperimentalServerRuntime['legacyConversion']
): RustLegacyConversionNotice | undefined {
  if (!value) return undefined;
  return {
    sourceSnapshotId: value.snapshotId,
    sourceFormat: value.sourceFormat,
    completeness: value.completeness,
    exactContinuation: false
  };
}

/** Apply Rust-confirmed numeric values to the small cached welcome configuration. */
function applyMetadataSettings(
  metadata: ExperimentalServerRuntime['metadata'],
  updates: readonly LiveSettingsUpdate[],
  configRevision: string,
  configHash: string
): ExperimentalServerRuntime['metadata'] {
  const values: ReadonlyMap<string, number> = new Map(
    updates.map(update => [update.path, update.value])
  );
  return {
    ...metadata,
    configRevision,
    configHash,
    settings: metadata.settings.map(setting => {
      const value = values.get(setting.path);
      if (value === undefined) return setting;
      return { ...setting, value: typeof setting.value === 'boolean' ? value === 1 : value };
    })
  };
}

/** Start native authority from fresh or retained managed state. */
export async function startRustServer(config: ServerConfig): Promise<RustServer> {
  /** Validate diagnostic resource admission before opening the owner database. */
  const distillationLimits = distillationSpoolLimitsForGiB(config.distillationSpoolGiB);
  if (config.controllerInputHoldMs !== 500 || config.controllerDisconnectGraceMs !== 30_000 || config.checkpointEveryGenerations !== 1) {
    throw new Error('Rust startup requires default controller timing and every-generation checkpoints');
  }
  const browserOrigins = createBrowserOriginPolicy(config);
  const sessionId = randomUUID();
  /** Ordered source hold shared by coarse archive population work. */
  let populationWork: { hold(): Promise<void>; release(): Promise<void> } | undefined;
  let schedule = (): void => {};
  let owner: ExperimentalServerRuntime;
  try {
    const databaseExists = existsSync(resolve(config.dbPath));
    if (!databaseExists && config.resume !== 'auto' && config.resume !== 'fresh') {
      throw new Error(`cannot resume: database does not exist at ${resolve(config.dbPath)}`);
    }
    owner = await createExperimentalServerRuntime({ databasePath: config.dbPath,
      managedDirectory: `${resolve(config.dbPath)}.checkpoints`,
      calculationWorkers: config.rustCalculationWorkers,
      ...(config.mathBackend === undefined ? {} : { mathBackend: config.mathBackend }),
      ...(config.populationFrom === undefined ? {} : { populationFromCheckpointId: config.populationFrom.slice(7) }),
      checkpointBudgetMiB: config.checkpointBudgetMiB,
      ...((config.resume === 'auto' || config.resume === 'latest') && databaseExists ? { restoreLatest: true } : {}),
      ...(config.resume.startsWith('sha256:') ? { restoreCheckpointId: config.resume.slice(7) } : {}),
      ...(config.seed === undefined ? {} : { seed: config.seed }), onWake: () => schedule() });
  } catch (error) { return startFaultedServer(config, error); }
  let recovery = recoveryNotice(owner);
  let importBranch = importBranchNotice(owner.importBranch);
  let legacyConversion = owner.metadata.legacyConversion ?? legacyConversionNotice(owner.legacyConversion);
  if (recovery) console.warn(recovery.explicitResume ? '[rust.resume]' : '[rust.recovery]', recovery);
  let activeMetadata = owner.metadata;
  let activeCheckpointId = owner.runStart.checkpointId;
  let retention: CheckpointRetentionInventory;
  let fitnessHistory: ManagedBrowserHistoryEntry[];
  /** Fixed experiment summaries retained only for bounded on-demand HTTP reads. */
  let experimentHistory: ManagedBrowserHistoryEntry[];
  /** Refresh compact stats once per history change, keeping sidecars out of the frame cadence. */
  const setHistory = (history: ManagedBrowserHistoryEntry[]): void => {
    experimentHistory = history;
    fitnessHistory = history.map(({ experiment: _experiment, ...entry }) => entry);
  };
  let hallOfFame: ManagedBrowserHallOfFameEntry[];
  let storage: ManagedStorageDiagnostics;
  let managedDisk: ManagedDiskDiagnostics;
  let retentionCleanup: { deletedCheckpointCount: number; deletedStoredByteCount: string };
  try {
    const initialCleanup = await owner.persistence.applyRetention();
    retention = initialCleanup.inventory;
    [experimentHistory, hallOfFame, storage, managedDisk] = await Promise.all([
      owner.persistence.readBrowserHistory(activeMetadata.runId),
      owner.persistence.readBrowserHallOfFame(activeMetadata.runId),
      owner.persistence.inspectStorage(),
      inspectManagedDisk(owner.managedDirectory)
    ]);
    setHistory(experimentHistory);
    retentionCleanup = { deletedCheckpointCount: initialCleanup.deletedCheckpointCount,
      deletedStoredByteCount: initialCleanup.deletedStoredByteCount };
  }
  catch (error) { await owner.close().catch(() => {}); return startFaultedServer(config, error); }
  const telemetry = new ExperimentalRuntimeTelemetry(owner.runtime.health(), owner.metadata.fixedStepSeconds);
  let fault: string | undefined;
  let stopping = false;
  let scheduled: NodeJS.Immediate | undefined;
  let timer: NodeJS.Timeout | undefined;
  let stopAuthorityWatch: (() => void) | undefined;
  let draining: Promise<boolean> | undefined;
  /** Lazily opened diagnostic store; ordinary capture-off startup creates no spool. */
  let distillation: DistillationCapture | undefined;
  /** Dedicated transport installed only when its namespace is requested. */
  let distillationHttp: DistillationHttpRouter | undefined;
  /** Explicit library commands share the current native scope and existing disk owners. */
  let savedBrains: SavedBrainLibrary | undefined;
  /** The existing shared command owner, available before the HTTP listener starts. */
  let distillationAdmission: BackgroundCommandAdmission | undefined;
  let lastFrame = 0;
  let lastStats = 0;
  let pumpStart = performance.now();
  let pumps = 0;
  let pumpsPerSecond = 0;
  const visualizationConnections = new Set<number>();
  let latestVisualization: RustBackgroundVisualization | undefined;
  let visualizationApplied = false;
  let visualizationCommand: { sequence: string; enabled: boolean } | undefined;
  let flushVisualization = (): void => {};
  /** One selected diagnostic owner shares the existing native command sequence. */
  let inspector: DecisionInspectorRouting | undefined;
  const pendingSettings = new Map<string, {
    connection: number;
    requestId: string;
    updates: LiveSettingsUpdate[];
  }>();
  /** One retained manual request; completion follows the native durable successor. */
  let pendingFinish: {
    connection: number; requestId: string; runId: string; generation: number;
    sequence: string; accepted: boolean; step?: number;
  } | undefined;
  /** At most one unresolved selection per connection, capped globally at 128. */
  const pendingSelections = new Map<string, { connection: number; requestId: string }>();
  const pendingGodModeMoves = new Map<string, {
    connection: number;
    requestId: string;
    snakeId: number;
    token: string;
  }>();
  const pendingGodModeKills = new Map<string, {
    connection: number;
    requestId: string;
    snakeId: number;
    token: string;
  }>();
  const pendingResurrections = new Map<string, {
    resolve(snakeId: number): void;
    reject(error: Error): void;
  }>();
  let pinning: Promise<void> | undefined;
  let retentionMaintenance: Promise<void> | undefined;
  let exportOperation: Promise<void> | undefined;
  let activeExportResponse: import('node:http').ServerResponse | undefined;
  let importOperation: Promise<void> | undefined;
  /** Failed permanent publication awaits a held native boundary before reclamation. */
  let orphanCleanupPending = false;
  /** Reclaim only while this caller holds native publication and excludes other archive jobs. */
  const reclaimFailedPublication = async (): Promise<void> => {
    const result = await owner.persistence.reclaimManagedOrphans();
    if (result.completed) orphanCleanupPending = false;
  };
  let resurrectionOperation: Promise<void> | undefined;
  let activeImportRequest: import('node:http').IncomingMessage | undefined;
  let activeImportResponse: import('node:http').ServerResponse | undefined;
  let importAuthorityPublished = false;
  let storageRefresh: Promise<void> | undefined;
  let storageInspectionFault: string | undefined;
  const disconnectedDuringImport = new Set<number>();
  let executeImport: ((request: import('node:http').IncomingMessage,
    resumeAsBranch: boolean, isCancelled: () => boolean) => Promise<ArchiveImportSuccess>) | undefined;
  let executeResurrection: ((entryId: string) => Promise<number>) | undefined;
  let reportArchiveStall = (error: Error): void => { console.error('[rust.archive-watchdog]', error.message); process.exit(1); };

  /** Refresh small storage counters without delaying or overlapping health responses. */
  const refreshStorage = (): void => {
    if (storageRefresh || stopping) return;
    storageRefresh = Promise.all([
      owner.persistence.inspectStorage(),
      inspectManagedDisk(owner.managedDirectory)
    ]).then(([nextStorage, nextManagedDisk]) => {
      storage = nextStorage;
      managedDisk = nextManagedDisk;
      storageInspectionFault = undefined;
    }).catch(error => {
      storageInspectionFault = error instanceof Error ? error.message : String(error);
      fault ??= `storage inspection failed: ${storageInspectionFault}`;
      owner.runtime.requestStop();
    }).finally(() => { storageRefresh = undefined; });
  };

  /** Keep population-sized bytes in Rust/filesystem/browser networking for one exact lease. */
  const serveExport = async (
    response: import('node:http').ServerResponse<import('node:http').IncomingMessage>,
    brainIds: string[]
  ): Promise<void> => {
    const lease = await owner.persistence.acquireCurrentExportLease();
    const readyPath = resolve(owner.managedDirectory, `.${lease.operationId}.slither-save.ready`);
    /** Successful native preparation transfers ownership of this exact ready file. */
    let readyOwned = false;
    /** Batch lease remains held until transfer/cancellation and checkpoint cleanup finish. */
    let brainLeaseOwned = false;
    try {
      let selected: SavedBrainArchiveEntryV1[] = [];
      if (brainIds.length) {
        const result = await owner.persistence.savedBrainCatalog({action: 'acquireArchive', brainIds, leaseId: lease.operationId});
        brainLeaseOwned = true;
        if (result.action !== 'acquireArchive') throw new Error('invalid saved brain archive lease');
        selected = parseSavedBrainArchiveEntries(result.entries.map(entry => ({brainId: entry.brainId,
          name: entry.name, createdAtMs: entry.createdAtMs, artifactByteCount: entry.artifactByteCount})));
      }
      // Source selection may outlive the client; release its lease without starting population work.
      if (response.destroyed) return;
      await admitExportSpace(owner.managedDirectory, lease, selected.reduce((sum, entry) => sum + BigInt(`0x${entry.artifactByteCount}`), 0n));
      if (response.destroyed) return;
      if (!populationWork) throw new Error('population work admission is not ready');
      await populationWork.hold();
      let ready: Awaited<ReturnType<typeof owner.runtime.prepareExportArchive>>;
      try {
        const preparation = owner.runtime.prepareExportArchive(
          owner.managedDirectory, lease.operationId, lease.descriptor, lease.inventory,
          ...(selected.length ? [JSON.stringify(selected)] : [])
        );
        const stopWatch = watchArchiveWork(owner.runtime, lease.operationId, 'export', reportArchiveStall);
        try { ready = await preparation; readyOwned = true; }
        finally { stopWatch(); }
      } finally { await populationWork.release(); }
      const downloadGeneration = BigInt(`0x${lease.descriptor.generation}`).toString();
      const downloadFilename = `slither-neuroevo-${lease.descriptor.logicalRootSha256.slice(0, 12)}-gen-${downloadGeneration}-v1.slither-save`;
      if (ready.operationId !== lease.operationId ||
          ready.checkpointId !== lease.descriptor.logicalRootSha256 ||
          ready.relativeFilename !== `.${lease.operationId}.slither-save.ready` ||
          !/^[0-9a-f]{64}$/u.test(ready.logicalRootSha256) ||
          ready.downloadFilename !== downloadFilename ||
          !/^[0-9a-f]{16}$/u.test(ready.storedByteCount)) {
        throw new Error('Rust returned an invalid export archive descriptor');
      }
      const expectedBytes = BigInt(`0x${ready.storedByteCount}`);
      if (!readyPath.startsWith(`${resolve(owner.managedDirectory)}${sep}`)) {
        throw new Error('Rust export archive escaped the managed directory');
      }
      const readyStat = await lstat(readyPath);
      if (readyStat.isSymbolicLink() || !readyStat.isFile() || BigInt(readyStat.size) !== expectedBytes) {
        throw new Error('Rust export archive is not the expected ready file');
      }
      if (response.destroyed) return;
      response.writeHead(200, {
        'Content-Type': 'application/vnd.slither-neuroevo.save',
        'Content-Length': expectedBytes.toString(),
        'Content-Disposition': `attachment; filename="${ready.downloadFilename}"`,
        'X-Slither-Checkpoint-Id': ready.checkpointId,
        'X-Slither-Save-Root': ready.logicalRootSha256
      });
      await new Promise<void>((done, reject) => {
        const stream = createReadStream(readyPath!);
        let settled = false;
        let noProgress: NodeJS.Timeout | undefined;
        /** Reset the idle deadline only when source/socket transfer advances. */
        const armNoProgress = (): void => {
          if (noProgress) clearTimeout(noProgress);
          noProgress = setTimeout(() => finish(new Error(
            `archive download made no progress for ${ARCHIVE_DOWNLOAD_NO_PROGRESS_MS} ms`
          )), ARCHIVE_DOWNLOAD_NO_PROGRESS_MS);
          noProgress.unref();
        };
        const finish = (error?: Error): void => {
          if (settled) return;
          settled = true;
          if (noProgress) clearTimeout(noProgress);
          response.off('drain', armNoProgress);
          stream.destroy();
          if (error) reject(error);
          else done();
        };
        stream.once('error', finish);
        stream.on('data', armNoProgress);
        response.on('drain', armNoProgress);
        response.once('finish', () => finish());
        response.once('close', () => finish());
        armNoProgress();
        stream.pipe(response);
      });
    } finally {
      try {
        if (readyOwned) {
          await unlink(readyPath).catch(error => {
            if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error;
          });
        }
      } finally {
        try {
          if (brainLeaseOwned) await owner.persistence.savedBrainCatalog({action: 'release', leaseId: lease.operationId});
        } finally { await owner.persistence.releaseExportLease(lease.operationId); }
      }
    }
  };
  const server = createServer((request, response) => {
    if (!admitBrowserRequest(request, response, browserOrigins)) return;
    const requestUrl = new URL(request.url ?? '/', 'http://localhost');
    const pathname = requestUrl.pathname;
    if (pathname.startsWith('/api/distillation/v1/')) {
      if (!distillationHttp) {
        if (!distillationAdmission || stopping) { response.writeHead(503); response.end(); return; }
        const spool = new DistillationSpoolClient(`${config.dbPath}.distillation`, distillationLimits);
        distillation = new DistillationCapture(() => owner.runtime, distillationAdmission, spool,
          () => ({ runId: activeMetadata.runId, seed: activeMetadata.seed }));
        savedBrains = new SavedBrainLibrary(owner.persistence, spool, owner.managedDirectory,
          () => owner.runtime, distillationAdmission,
          () => ({expectedRunId: activeMetadata.runId, expectedConfigRevision: activeMetadata.configRevision,
            expectedWorldEpoch: owner.runtime.health().worldEpoch}),
          () => !fault && !stopping && !importOperation && !exportOperation && !resurrectionOperation && !pinning && !retentionMaintenance,
          () => schedule());
        distillationHttp = new DistillationHttpRouter(spool, distillation, savedBrains);
      }
      distillationHttp.handle(request, response, requestUrl);
      return;
    }
    if (pathname === '/api/health' || pathname === '/health') {
      refreshStorage();
      const nativeHealth = owner.runtime.health(true);
      const archiveWork = owner.runtime.archiveWorkProgress();
      response.writeHead(fault ? 503 : 200, { 'Content-Type': 'application/json' });
      response.end(JSON.stringify({ ok: !fault, authority: 'rust', runId: activeMetadata.runId,
        seed: activeMetadata.seed, configRevision: wireInteger(activeMetadata.configRevision),
        configHash: activeMetadata.configHash, startupCheckpointId: activeCheckpointId, ...nativeHealth,
        nativeBuildIdentifier: owner.nativeBuildIdentifier, mathBackend: activeMetadata.mathBackend,
        telemetry: telemetry.snapshot(nativeHealth), nativeQueues: owner.runtime.queueDiagnostics(),
        outbound: hub?.getOutboundDiagnostics(),
        archiveWork,
        retention, retentionCleanup, storage: storageHealthPayload(storage, managedDisk),
        ...(storageInspectionFault ? { storageInspectionFault } : {}),
        ...(recovery ? { recovery } : {}), ...(importBranch ? { importBranch } : {}),
        ...(legacyConversion ? { legacyConversion } : {}),
        ...(fault ? { interfaceFault: fault } : {}) }));
      return;
    }
    if (request.method === 'POST' && pathname === '/api/import/archive') {
      const importMode = requestUrl.searchParams.get('mode');
      if (importMode !== null && importMode !== 'branch') {
        response.writeHead(400, { 'Content-Type': 'application/json' });
        response.end(JSON.stringify({ ok: false, message: 'unsupported archive import mode' }));
        return;
      }
      if (fault || stopping || !executeImport) {
        response.writeHead(503, { 'Content-Type': 'application/json' });
        response.end(JSON.stringify({ ok: false, message: fault ?? 'server is not ready' }));
        return;
      }
      if (importOperation || exportOperation || resurrectionOperation || pinning || retentionMaintenance || savedBrains?.busy) {
        response.writeHead(409, { 'Content-Type': 'application/json' });
        response.end(JSON.stringify({ ok: false, message: 'another archive operation is in progress' }));
        return;
      }
      activeImportRequest = request;
      activeImportResponse = response;
      /** Clear the busy gate before releasing either terminal HTTP response. */
      const finishImport = (): void => {
        activeImportRequest = undefined;
        activeImportResponse = undefined;
        importAuthorityPublished = false;
        importOperation = undefined;
      };
      importOperation = executeImport(request, importMode === 'branch',
        () => response.destroyed || stopping).then(result => {
        finishImport();
        if (response.destroyed) return;
        response.writeHead(200, { 'Content-Type': 'application/json' });
        response.end(JSON.stringify(result));
      }).catch(async error => {
        finishImport();
        if (response.destroyed) return;
        if (response.headersSent) {
          response.destroy();
          return;
        }
        const message = error instanceof Error ? error.message : String(error);
        const requiresBranch = message.includes('resume it as a branch');
        await rejectArchiveUpload(request, response, requiresBranch ? 409 : fault ? 503 : 400,
          { ok: false, message, ...(requiresBranch ? { code: 'IMPORT_REQUIRES_BRANCH' } : {}) });
      });
      return;
    }
    if (request.method === 'GET' && pathname === '/api/export/latest') {
      /** Explicit IDs are unique, sorted, bounded metadata; the default selects no library records. */
      const selection = requestUrl.searchParams.getAll('brainIds');
      const brainIds = selection.length === 1 ? selection[0]!.split(',') : [];
      if ([...requestUrl.searchParams.keys()].some(key => key !== 'brainIds') || selection.length > 1 ||
          (selection.length && (brainIds.length < 1 || brainIds.length > 100 || brainIds.some((id, index) =>
            !/^[0-9a-f]{64}$/u.test(id) || (index > 0 && brainIds[index - 1]! >= id))))) {
        response.writeHead(400, {'Content-Type': 'application/json'});
        response.end(JSON.stringify({ok: false, message: 'brainIds must contain 1..100 unique sorted saved brain IDs'}));
        return;
      }

      if (fault || stopping) {
        response.writeHead(503, { 'Content-Type': 'application/json' });
        response.end(JSON.stringify({ ok: false, message: fault ?? 'server is stopping' }));
        return;
      }
      if (exportOperation || importOperation || resurrectionOperation || pinning || retentionMaintenance || savedBrains?.busy) {
        response.writeHead(409, { 'Content-Type': 'application/json' });
        response.end(JSON.stringify({ ok: false, message: 'another persistence operation is in progress' }));
        return;
      }
      activeExportResponse = response;
      exportOperation = serveExport(response, brainIds).catch(error => {
        if (response.destroyed) return;
        if (response.headersSent) {
          response.destroy();
          return;
        }
        response.writeHead(500, { 'Content-Type': 'application/json' });
        response.end(JSON.stringify({ ok: false, message: error instanceof Error ? error.message : String(error) }));
      }).finally(() => { activeExportResponse = undefined; exportOperation = undefined; });
      return;
    }
    if (request.method === 'GET' && pathname === '/api/history') {
      response.writeHead(fault ? 503 : 200, { 'Content-Type': 'application/json' });
      response.end(JSON.stringify({ version: 1, runId: activeMetadata.runId, history: experimentHistory }));
      return;
    }
    if (request.method === 'GET' && pathname === '/api/hof') {
      response.writeHead(fault ? 503 : 200, { 'Content-Type': 'application/json' });
      response.end(JSON.stringify({
        hof: hallOfFame.map(entry => ({ ...entry, seed: activeMetadata.seed }))
      }));
      return;
    }
    if (request.method === 'GET' && pathname === '/api/graph-presets') {
      const rawLimit = requestUrl.searchParams.get('limit');
      const requestedLimit = rawLimit === null ? Number.NaN : Number(rawLimit);
      const limit = Number.isFinite(requestedLimit)
        ? Math.min(200, Math.max(1, Math.trunc(requestedLimit)))
        : 50;
      void owner.persistence.listGraphPresets(limit).then(presets => {
        if (response.destroyed) return;
        response.writeHead(200, { 'Content-Type': 'application/json' });
        response.end(JSON.stringify({ ok: true, presets }));
      }).catch(error => {
        if (response.destroyed) return;
        response.writeHead(500, { 'Content-Type': 'application/json' });
        response.end(JSON.stringify({ ok: false,
          message: error instanceof Error ? error.message : String(error) }));
      });
      return;
    }
    if (request.method === 'GET' && pathname.startsWith('/api/graph-presets/')) {
      const id = Number(pathname.slice('/api/graph-presets/'.length));
      if (!Number.isSafeInteger(id) || id < 1) {
        response.writeHead(400, { 'Content-Type': 'application/json' });
        response.end(JSON.stringify({ ok: false, message: 'preset id must be a positive integer' }));
        return;
      }
      void owner.persistence.loadGraphPreset(id).then(preset => {
        if (response.destroyed) return;
        response.writeHead(preset ? 200 : 404, { 'Content-Type': 'application/json' });
        response.end(JSON.stringify(preset
          ? { ok: true, preset }
          : { ok: false, message: 'preset not found' }));
      }).catch(error => {
        if (response.destroyed) return;
        response.writeHead(400, { 'Content-Type': 'application/json' });
        response.end(JSON.stringify({ ok: false,
          message: error instanceof Error ? error.message : String(error) }));
      });
      return;
    }
    if (request.method === 'POST' && pathname === '/api/graph-presets') {
      void readJsonBody(request, 512 * 1024).then(value => {
        if (!value || typeof value !== 'object' || Array.isArray(value)) {
          throw new TypeError('graph preset body must be an object');
        }
        const body = value as Record<string, unknown>;
        if (Object.keys(body).length !== 2 || typeof body['name'] !== 'string' ||
            !body['spec'] || typeof body['spec'] !== 'object' || Array.isArray(body['spec'])) {
          throw new TypeError('graph preset requires exactly name and spec');
        }
        return owner.persistence.saveGraphPreset(body['name'], body['spec'] as GraphSpec);
      }).then(presetId => {
        if (response.destroyed) return;
        response.writeHead(200, { 'Content-Type': 'application/json' });
        response.end(JSON.stringify({ ok: true, presetId }));
      }).catch(error => {
        if (response.destroyed) return;
        response.writeHead(400, { 'Content-Type': 'application/json' });
        response.end(JSON.stringify({ ok: false,
          message: error instanceof Error ? error.message : String(error) }));
      });
      return;
    }
    if (request.method === 'POST' && pathname === '/api/resurrect') {
      if (fault || stopping || !executeResurrection) {
        response.writeHead(503, { 'Content-Type': 'application/json' });
        response.end(JSON.stringify({ ok: false, message: fault ?? 'server is not ready' }));
        return;
      }
      if (resurrectionOperation || importOperation || exportOperation || pinning || retentionMaintenance || savedBrains?.busy) {
        response.writeHead(409, { 'Content-Type': 'application/json' });
        response.end(JSON.stringify({ ok: false, message: 'another authority operation is in progress' }));
        return;
      }
      resurrectionOperation = readJsonBody(request, 4096).then(value => {
        if (!value || typeof value !== 'object' || Array.isArray(value) ||
            Object.keys(value).length !== 1 ||
            typeof (value as Record<string, unknown>)['entryId'] !== 'string' ||
            !/^[0-9a-f]{16}$/u.test((value as Record<string, string>)['entryId']!)) {
          throw new TypeError('resurrection requires one exact Hall-of-Fame entryId');
        }
        return executeResurrection!((value as Record<string, string>)['entryId']!);
      }).then(snakeId => {
        if (response.destroyed) return;
        response.writeHead(200, { 'Content-Type': 'application/json' });
        response.end(JSON.stringify({ ok: true, snakeId }));
      }).catch(error => {
        if (response.destroyed) return;
        response.writeHead(400, { 'Content-Type': 'application/json' });
        response.end(JSON.stringify({ ok: false,
          message: error instanceof Error ? error.message : String(error) }));
      }).finally(() => { resurrectionOperation = undefined; });
      return;
    }
    if (request.method === 'POST' && pathname === '/api/checkpoints/current/pin') {
      if (fault || stopping) {
        response.writeHead(503, { 'Content-Type': 'application/json' });
        response.end(JSON.stringify({ ok: false, message: fault ?? 'server is stopping' }));
        return;
      }
      if (pinning || importOperation || exportOperation || resurrectionOperation || retentionMaintenance || savedBrains?.busy) {
        response.writeHead(409, { 'Content-Type': 'application/json' });
        response.end(JSON.stringify({ ok: false, message: 'another persistence operation is in progress' }));
        return;
      }
      pinning = admitDiskOperation(owner.managedDirectory, {
        operation: 'pin', sourceSpoolBytes: 0n, candidateSpoolBytes: 0n, finalManagedBytes: 0n
      }).then(() => owner.persistence.pinCurrentCheckpoint()).then(async pinned => {
        retention = await owner.persistence.inspectRetention();
        if (response.destroyed) return;
        response.writeHead(200, { 'Content-Type': 'application/json' });
        response.end(JSON.stringify({ ok: true, ...pinned }));
      }).catch(error => {
        if (response.destroyed) return;
        response.writeHead(500, { 'Content-Type': 'application/json' });
        response.end(JSON.stringify({ ok: false, message: error instanceof Error ? error.message : String(error) }));
      }).finally(() => { pinning = undefined; });
      return;
    }
    if (request.method !== 'GET' || pathname.startsWith('/api/')) {
      response.writeHead(501, { 'Content-Type': 'application/json' });
      response.end(JSON.stringify({ error: 'API route is not available in the Rust runtime' })); return;
    }
    void serveBrowserAsset(pathname, response, CLIENT_ROOT, config.publicWsUrl);
  });
  let hub: WsHub | undefined;
  let closePromise: Promise<void> | undefined;
  /** Join every owner exactly once, including startup failure. */
  const close = (): Promise<void> => {
    closePromise ??= (async () => {
      stopping = true;
      if (timer) clearInterval(timer);
      stopAuthorityWatch?.();
      activeImportRequest?.destroy();
      activeImportResponse?.destroy();
      activeExportResponse?.destroy();
      await importOperation?.catch(() => {});
      await resurrectionOperation?.catch(() => {});
      await pinning?.catch(() => {});
      await retentionMaintenance?.catch(() => {});
      await exportOperation?.catch(() => {});
      if (scheduled) clearImmediate(scheduled);
      hub?.closeAll();
      owner.runtime.requestStop();
      await draining?.catch(() => {});
      await savedBrains?.close().catch(error => { console.error('[saved-brains.shutdown]', error instanceof Error ? error.message : error); });
      try { await owner.close(); }
      finally {
        await distillation?.close().catch(error => { console.error('[distillation.shutdown]', error instanceof Error ? error.message : error); });
        telemetry.close();
        if (server.listening) await new Promise<void>((done, reject) => server.close(error => error ? reject(error) : done()));
      }
    })();
    return closePromise;
  };
  try {
    hub = new WsHub(server, { ...createRustWelcome(activeMetadata, sessionId, owner.nativeBuildIdentifier, config.rustCalculationWorkers), ...(recovery ? { recovery } : {}),
      ...(importBranch ? { importBranch } : {}),
      ...(legacyConversion ? { legacyConversion } : {}) }, { maxConnections: 64, browserOrigins });
    const sockets = hub;
    let routing!: ExternalControllerRouting;
    const output = new BackgroundOutputPump({
      owner, metadata: () => activeMetadata, maxControllers: MAX_CONTROLLERS,
      send: (id, message) => sockets.sendJsonTo(Number(BigInt(`0x${id}`)), message),
      event(event) {
        routing.event(event);
        inspector?.event(event);
        distillation?.event(event);
        savedBrains?.event(event);
        if (pendingFinish && pendingFinish.sequence === event.commandSequence &&
            (event.kind === 'finishGenerationAccepted' || event.kind === 'commandRejected')) {
          const pending = pendingFinish;
          if (event.kind === 'finishGenerationAccepted' && event.finishGenerationEffectiveStep) {
            pending.accepted = true;
            pending.step = wireInteger(event.finishGenerationEffectiveStep);
            sockets.sendJsonTo(pending.connection, { type: 'finishGenerationResult',
              requestId: pending.requestId, runId: pending.runId, generation: pending.generation,
              status: 'pending', step: pending.step });
          } else {
            sockets.sendJsonTo(pending.connection, { type: 'finishGenerationResult',
              requestId: pending.requestId, runId: pending.runId, generation: pending.generation,
              status: 'rejected', reason: event.rejectionDetail ?? 'Rust rejected manual completion' });
            pendingFinish = undefined;
          }
        }
        if (pendingFinish?.accepted && event.kind === 'generationStartPublished') {
          const pending = pendingFinish;
          sockets.sendJsonTo(pending.connection, { type: 'finishGenerationResult',
            requestId: pending.requestId, runId: pending.runId, generation: pending.generation,
            status: 'completed', ...(pending.step !== undefined ? { step: pending.step } : {}) });
          pendingFinish = undefined;
        }
        if (event.commandSequence && (event.kind === 'liveSettingsApplied' || event.kind === 'commandRejected')) {
          const pending = pendingSettings.get(event.commandSequence);
          if (pending) {
            pendingSettings.delete(event.commandSequence);
            if (event.kind === 'liveSettingsApplied') {
              if (!event.settingsConfigRevision || !event.settingsConfigHash || !event.settingsEffectiveStep) {
                fail(new Error('Rust live-settings result omitted its authoritative identity'));
                return;
              }
              inspector?.reset('inspector cleared by applied settings');
              activeMetadata = applyMetadataSettings(
                activeMetadata,
                pending.updates,
                event.settingsConfigRevision,
                event.settingsConfigHash
              );
              sockets.updateWelcome(createRustWelcome(activeMetadata, sessionId, owner.nativeBuildIdentifier, config.rustCalculationWorkers));
              sockets.broadcastJsonToUi({
                type: 'settingsApplied', requestId: pending.requestId, applied: true,
                updates: pending.updates,
                configRevision: wireInteger(event.settingsConfigRevision),
                configHash: event.settingsConfigHash,
                sequence: wireInteger(event.commandSequence),
                step: wireInteger(event.settingsEffectiveStep)
              });
            } else {
              sockets.sendJsonTo(pending.connection, {
                type: 'settingsApplied', requestId: pending.requestId, applied: false, updates: [],
                configRevision: wireInteger(activeMetadata.configRevision),
                configHash: activeMetadata.configHash,
                reason: event.rejectionDetail ?? event.rejectionCode ?? 'Rust rejected live settings'
              });
            }
          }
        }
        if (event.commandSequence && (event.kind === 'entitySelected' || event.kind === 'commandRejected')) {
          const pending = pendingSelections.get(event.commandSequence);
          if (pending) {
            pendingSelections.delete(event.commandSequence);
            sockets.sendJsonTo(pending.connection, { type: 'entitySelected', requestId: pending.requestId,
              found: event.kind === 'entitySelected' && !!event.entitySelection,
              ...(event.entitySelection ? { entity: event.entitySelection } : {}),
              ...(event.kind === 'commandRejected' ? { reason: event.rejectionDetail ?? 'Rust rejected selection' } : {}) });
          }
        }
        if (event.commandSequence && (event.kind === 'godModeMoved' || event.kind === 'commandRejected')) {
          const pending = pendingGodModeMoves.get(event.commandSequence);
          if (pending) {
            pendingGodModeMoves.delete(event.commandSequence);
            const moved = event.godModeMove;
            if (event.kind === 'godModeMoved' && moved) {
              sockets.sendJsonTo(pending.connection, {
                type: 'godModeResult', requestId: pending.requestId, action: 'move',
                snakeId: moved.snakeId, selectionToken: pending.token, applied: true,
                sequence: wireInteger(event.commandSequence), step: wireInteger(moved.effectiveStep),
                x: moved.x, y: moved.y
              });
            } else {
              sockets.sendJsonTo(pending.connection, {
                type: 'godModeResult', requestId: pending.requestId, action: 'move',
                snakeId: pending.snakeId, applied: false,
                reason: event.rejectionDetail ?? event.rejectionCode ?? 'Rust rejected God Mode move'
              });
            }
          }
        }
        if (event.commandSequence && (event.kind === 'godModeKilled' || event.kind === 'commandRejected')) {
          const pending = pendingGodModeKills.get(event.commandSequence);
          if (pending) {
            pendingGodModeKills.delete(event.commandSequence);
            const killed = event.godModeKill;
            if (event.kind === 'godModeKilled' && killed) {
              sockets.sendJsonTo(pending.connection, {
                type: 'godModeResult', requestId: pending.requestId, action: 'kill',
                snakeId: killed.snakeId, selectionToken: pending.token, applied: true,
                sequence: wireInteger(event.commandSequence), step: wireInteger(killed.effectiveStep),
                pelletsDropped: wireInteger(killed.pelletsDropped)
              });
            } else {
              sockets.sendJsonTo(pending.connection, {
                type: 'godModeResult', requestId: pending.requestId, action: 'kill',
                snakeId: pending.snakeId, applied: false,
                reason: event.rejectionDetail ?? event.rejectionCode ?? 'Rust rejected God Mode kill'
              });
            }
          }
        }
        if (event.commandSequence &&
            (event.kind === 'hallOfFameResurrected' || event.kind === 'commandRejected')) {
          const pending = pendingResurrections.get(event.commandSequence);
          if (pending) {
            pendingResurrections.delete(event.commandSequence);
            const resurrected = event.hallOfFameResurrection;
            if (event.kind === 'hallOfFameResurrected' && resurrected) {
              pending.resolve(resurrected.snakeId);
            } else {
              pending.reject(new Error(
                event.rejectionDetail ?? event.rejectionCode ?? 'Rust rejected resurrection'
              ));
            }
          }
        }
        const command = visualizationCommand;
        if (command && command.sequence === event.commandSequence &&
            (event.kind === 'visualizationChanged' || event.kind === 'commandRejected')) {
          visualizationCommand = undefined;
          if (event.kind !== 'visualizationChanged' ||
              event.visualizationEnabled !== command.enabled) {
            fail(new Error(event.rejectionDetail ?? 'Rust rejected visualization state'));
            return;
          }
          visualizationApplied = command.enabled;
          flushVisualization();
        }
        const now = performance.now();
        if (event.display) {
          telemetry.observeDisplay(event.display);
          sockets.updateWelcome({ frameByteLength: event.display.frameByteLength });
          if (now - lastStats >= 1000 / config.uiFrameRateHz) {
            lastStats = now;
            sockets.broadcastStats(createRustStats(
              event.display,
              activeMetadata,
              pumpsPerSecond,
              fitnessHistory,
              visualizationConnections.size > 0 ? latestVisualization : undefined
            ));
          }
        }
      },
      hasFrameRecipients: () => sockets.hasFrameRecipients() && performance.now() - lastFrame >= 1000 / config.uiFrameRateHz,
      diagnostics: () => inspector?.service(),
      wantsVisualization: () => visualizationConnections.size > 0,
      visualization(snapshot) { latestVisualization = snapshot; },
      frame(lease) { lastFrame = performance.now(); sockets.broadcastFrame(lease.bytes, lease.release); },
      observeCheckpointBarrier: durationMs => telemetry.observeCheckpointBarrier(durationMs),
      maintainCheckpointRetention: async () => {
        if (retentionMaintenance) {
          throw new Error('checkpoint retention maintenance overlapped a durable generation');
        }
        retentionMaintenance = (async () => {
          // SQLite has committed this generation; Rust still holds its durability
          // boundary. Do not scan while another native publisher is live.
          if (orphanCleanupPending && !importOperation && !exportOperation && !resurrectionOperation && !pinning && !savedBrains?.busy) {
            await reclaimFailedPublication();
          }
          return owner.persistence.applyRetention();
        })().then(async result => {
          retention = result.inventory;
          retentionCleanup = { deletedCheckpointCount: result.deletedCheckpointCount,
            deletedStoredByteCount: result.deletedStoredByteCount };
          setHistory(await owner.persistence.readBrowserHistory(activeMetadata.runId));
          hallOfFame = await owner.persistence.readBrowserHallOfFame(activeMetadata.runId);
        });
        try { await retentionMaintenance; }
        finally { retentionMaintenance = undefined; }
      }
    });
    distillationAdmission = output.admission;
    routing = new ExternalControllerRouting({ native: owner.runtime, admission: output.admission,
      maxControllers: MAX_CONTROLLERS, maxActionsPerSecond: config.maxActionsPerSecond, maxActionsPerTick: config.maxActionsPerTick,
      send: (connection, message) => sockets.sendJsonTo(connection, message),
      observeActionLatency: (kind, durationMs) => telemetry.observeAction(kind, durationMs),
      observeLifecycleLatency: (kind, operation, durationMs) =>
        telemetry.observeControllerLifecycle(kind, operation, durationMs),
      observeDisconnect: kind => telemetry.observeControllerDisconnect(kind) });
    populationWork = {
      async hold() {
        routing.pauseForReplacement();
        try { await output.stagePreparedImport(); }
        catch (error) { routing.resumeAfterReplacement(); throw error; }
      },
      async release() {
        try { await output.cancelPreparedImport(); routing.resumeAfterReplacement(); }
        catch (error) { fail(error); throw error; }
      }
    };
    /** Converge many UI subscriptions onto one ordered native capture toggle. */
    flushVisualization = (): void => {
      const enabled = visualizationConnections.size > 0;
      if (visualizationCommand || enabled === visualizationApplied || fault || stopping) return;
      let sequence: string | undefined;
      const admitted = output.admission.trySubmitControl(value => {
        owner.runtime.submitVisualization(value, enabled);
        sequence = value;
      });
      if (admitted && sequence) visualizationCommand = { sequence, enabled };
    };
    inspector = new DecisionInspectorRouting({runtime: () => owner.runtime, admission: output.admission,
      scope: () => ({sessionId, runId: activeMetadata.runId,
        available: !fault && !stopping && (!importOperation || importAuthorityPublished)}),
      reliable: (connection, message) => sockets.sendJsonTo(connection, message),
      replaceable: (connection, message) => sockets.sendInspectorTo(connection, message),
      clear: connection => sockets.clearInspectorTo(connection), schedule});
    /** Keep health available after a terminal native/interface failure. */
    const fail = (error: unknown): void => {
      const failure = error instanceof Error ? error : new Error(String(error));
      if (pendingFinish) {
        sockets.sendJsonTo(pendingFinish.connection, { type: 'finishGenerationResult',
          requestId: pendingFinish.requestId, runId: pendingFinish.runId, generation: pendingFinish.generation,
          status: 'rejected', reason: failure.message });
        pendingFinish = undefined;
      }
      for (const pending of pendingSelections.values()) {
        sockets.sendJsonTo(pending.connection, { type: 'entitySelected', requestId: pending.requestId,
          found: false, reason: failure.message });
      }
      pendingSelections.clear();
      for (const pending of pendingResurrections.values()) pending.reject(failure);
      pendingResurrections.clear();
      if (!fault) sockets.broadcastError(error instanceof Error ? error.message : String(error));
      fault ??= error instanceof Error ? error.message : String(error);
      owner.runtime.requestStop();
    };
    reportArchiveStall = (error: Error): void => {
      console.error('[rust.archive-watchdog]', error.message);
      fail(error);
      activeExportResponse?.destroy(error);
      activeImportRequest?.destroy(error);
      activeImportResponse?.destroy(error);
      // A hung in-process native worker cannot be stopped or safely reused.
      process.exit(1);
    };
    /** Hold the exact SQLite winner lease until Rust publishes or rejects the new snake. */
    executeResurrection = async (entryId): Promise<number> => {
      const selected = await owner.persistence.selectHallOfFameEntry(
        activeMetadata.runId,
        entryId as import('./rustEngine/checkpointPersistenceProtocol.ts').U64Hex
      );
      try {
        const snakeId = await new Promise<number>((resolve, reject) => {
          let admittedSequence: string | undefined;
          const admitted = output.admission.trySubmitControl(sequence => {
            owner.runtime.submitHallOfFameResurrection(
              sequence,
              owner.managedDirectory,
              selected.entryId,
              selected.weights
            );
            admittedSequence = sequence;
          });
          if (!admitted || !admittedSequence) {
            reject(new Error('authoritative command queue is busy'));
            return;
          }
          pendingResurrections.set(admittedSequence, { resolve, reject });
          schedule();
        });
        return snakeId;
      } finally {
        await owner.persistence.releaseHallOfFameEntry(selected.operationId);
      }
    };
    /** Keep upload bytes and the complete replacement outside JavaScript memory. */
    executeImport = async (request, resumeAsBranch, isCancelled): Promise<ArchiveImportSuccess> => {
      /** Reject a disconnected peer before committing; a committed replacement must finish publication. */
      const requireConnected = (): void => {
        if (isCancelled()) throw new Error('archive import cancelled before commit');
      };
      const operationId = randomBytes(16).toString('hex');
      const branchRunId = resumeAsBranch ? randomUUID() : null;
      const legacyRunId = randomUUID();
      const legacySeed = randomBytes(4).readUInt32LE(0);
      let uploadPath: string | undefined;
      let inventoryPath: string | undefined;
      let prepared = false;
      let staged = false;
      let commitAttempted = false;
      let committed = false;
      let nativePreparationStarted = false;
      let routingHeld = false;
      let previousCurrent: { runId: string; checkpointId: string } | undefined;
      try {
        requireConnected();
        const declaredUploadBytes = parseArchiveContentLength(
          request.headers['content-length'],
          P0_ARCHIVE_UPLOAD_LIMIT
        );
        await admitDiskOperation(owner.managedDirectory, {
          operation: 'import',
          sourceSpoolBytes: declaredUploadBytes ?? P0_ARCHIVE_UPLOAD_LIMIT,
          candidateSpoolBytes: 0n,
          finalManagedBytes: 0n
        });
        requireConnected();
        const upload = await spoolArchiveUpload({
          // Early spool rejection must leave the socket alive until its HTTP error is sent.
          source: request.iterator({ destroyOnReturn: false }),
          contentLength: request.headers['content-length'],
          scratchDirectory: owner.managedDirectory,
          operationId
        });
        uploadPath = upload.readyPath;
        requireConnected();
        const diskEstimate = await owner.runtime.estimateImportDisk(upload.readyPath);
        await admitDiskOperation(owner.managedDirectory, {
          operation: 'import', sourceSpoolBytes: 0n,
          candidateSpoolBytes: BigInt(`0x${diskEstimate.candidateSpoolBytes}`),
          finalManagedBytes: BigInt(`0x${diskEstimate.finalManagedBytes}`)
        });
        requireConnected();
        routing.pauseForReplacement();
        routingHeld = true;
        await output.stagePreparedImport();
        staged = true;
        nativePreparationStarted = true;
        const preparation = owner.runtime.prepareImportArchive(
          upload.readyPath,
          owner.managedDirectory,
          owner.managedDirectory,
          operationId,
          legacyRunId,
          legacySeed
        );
        const stopWatch = watchArchiveWork(owner.runtime, operationId, 'import', reportArchiveStall);
        let imported: Awaited<typeof preparation>;
        try { imported = await preparation; }
        finally { stopWatch(); }
        const descriptor = parseManagedCheckpointDescriptor(imported.descriptor);
        const inventory = parseManagedImportInventoryDescriptor(imported.inventory, operationId);
        const metadata = parseRustStartupMetadata(imported.startupMetadata);
        if (descriptor.operationId !== operationId || descriptor.runId !== imported.runId ||
            descriptor.generation !== imported.generation ||
            descriptor.completedStep !== imported.completedStep ||
            descriptor.logicalRootSha256 !== imported.checkpointId ||
            metadata.runId !== imported.runId) {
          throw new Error('prepared import identity is internally inconsistent');
        }
        prepared = true;
        inventoryPath = resolve(owner.managedDirectory, inventory.relativeFilename);
        // Hold the old world before cancelling so newly published, unreferenced files can be reclaimed safely.
        requireConnected();
        const selected = await owner.persistence.selectStartup();
        if (!selected.descriptor || selected.runId !== activeMetadata.runId) {
          throw new Error('staged import found no matching current checkpoint');
        }
        previousCurrent = { runId: selected.runId,
          checkpointId: selected.descriptor.logicalRootSha256 };
        requireConnected();
        commitAttempted = true;
        const durable = await owner.persistence.commitImport(descriptor, inventory, branchRunId);
        committed = true;
        if ((durable.importBranch?.branchRunId ?? null) !== branchRunId) {
          throw new Error('committed import branch identity is inconsistent');
        }
        const previousHealth = owner.runtime.health();
        await output.publishPreparedImport(durable.descriptor, branchRunId ?? undefined);
        telemetry.rebase(previousHealth, durable.descriptor.completedStep, metadata.fixedStepSeconds);
        activeMetadata = branchRunId === null ? metadata : { ...metadata, runId: branchRunId };
        setHistory(await owner.persistence.readBrowserHistory(activeMetadata.runId));
        hallOfFame = await owner.persistence.readBrowserHallOfFame(activeMetadata.runId);
        activeCheckpointId = durable.checkpointId;
        recovery = undefined;
        importBranch = importBranchNotice(durable.importBranch ?? null);
        legacyConversion = metadata.legacyConversion;
        routing.resetAfterImport();
        routingHeld = false;
        disconnectedDuringImport.clear();
        importAuthorityPublished = true;
        const welcome = { ...createRustWelcome(activeMetadata, sessionId, owner.nativeBuildIdentifier, config.rustCalculationWorkers), ...(importBranch ? { importBranch } : {}) };
        void distillation?.runReplaced().catch(error => { console.error('[distillation.replacement]', error instanceof Error ? error.message : error); });
        inspector?.reset('inspector cleared by authority replacement');
        sockets.replaceWelcome(welcome);
        sockets.enterAwaitingRejoin({
          type: 'stateReplaced', reason: 'import', checkpointId: durable.checkpointId, welcome
        });
        retention = await owner.persistence.inspectRetention();
        return {
          ok: true,
          runId: activeMetadata.runId,
          generation: descriptor.generation,
          completedStep: descriptor.completedStep,
          checkpointId: durable.checkpointId,
          saveLogicalRootSha256: imported.saveLogicalRootSha256,
          ...(legacyConversion ? { legacyConversion } : {}),
          branched: branchRunId !== null,
          ...(branchRunId === null ? {} : { sourceRunId: descriptor.runId })
        };
      } catch (error) {
        if (nativePreparationStarted && !committed) orphanCleanupPending = true;
        if (staged && !committed) {
          let oldPointerStillCurrent = !commitAttempted;
          if (commitAttempted) {
            try {
              const selected = await owner.persistence.selectStartup();
              oldPointerStillCurrent = selected.runId === previousCurrent?.runId &&
                selected.descriptor?.logicalRootSha256 === previousCurrent?.checkpointId;
            } catch { /* An unreadable commit outcome cannot release the old authority. */ }
          }
          if (oldPointerStillCurrent) {
            try {
              await reclaimFailedPublication();
              await output.cancelPreparedImport();
            } catch (cleanupError) { fail(cleanupError); }
          }
          else fail(error);
        }
        else if (prepared && !staged) {
          try { owner.runtime.discardPreparedImport(); } catch { /* Candidate may already be gone. */ }
        }
        if (!committed) {
          for (const connection of disconnectedDuringImport) routing.disconnect(connection);
          disconnectedDuringImport.clear();
          if (!fault) routing.flush();
        }
        if (committed) fail(error);
        throw error;
      } finally {
        if (routingHeld && !fault && !stopping) routing.resumeAfterReplacement();
        for (const path of [uploadPath, inventoryPath]) {
          if (path) await unlink(path).catch(error => {
            if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error;
          });
        }
      }
    };
    /** Build, persist, and publish one generation-one authority without exposing it early. */
    const executeFreshReplacement = async (
      reason: 'reset' | 'newRun',
      seed: number,
      settings: Array<{ path: string; value: number }>,
      graphSpec: NonNullable<ExperimentalServerRuntime['metadata']['graphSpec']>
    ): Promise<{ runId: string; seed: number; checkpointId: string }> => {
      const operationId = randomBytes(16).toString('hex');
      const runId = randomUUID();
      let prepared = false;
      let staged = false;
      let commitAttempted = false;
      let committed = false;
      let newlyPublishedBudgetRejectedFile: string | undefined;
      let nativePreparationStarted = false;
      let routingHeld = false;
      let previousCurrent: { runId: string; checkpointId: string } | undefined;
      try {
        await owner.admitCheckpoint();
        const preexistingManagedNames = new Set(await readdir(owner.managedDirectory));
        routing.pauseForReplacement();
        routingHeld = true;
        await output.stagePreparedImport();
        staged = true;
        nativePreparationStarted = true;
        const candidate = await owner.runtime.prepareFreshRun(
          owner.managedDirectory,
          operationId,
          runId,
          seed,
          settings,
          JSON.stringify(graphSpec)
        );
        prepared = true;
        const descriptor = parseManagedCheckpointDescriptor(candidate.descriptor);
        const metadata = parseRustStartupMetadata(candidate.startupMetadata);
        const returnedSettings = new Map(metadata.settings.map(setting => [setting.path,
          typeof setting.value === 'boolean' ? Number(setting.value) : setting.value]));
        // Rust derives the one textual v4 digest from the explicit numeric
        // selection. The metadata parser independently verifies its bytes and
        // its agreement with the graph/profile; every requested value still matches.
        const derivedDigestCount = metadata.sensorVersion === 4 ? 1 : 0;
        const settingsMatch = returnedSettings.size === settings.length + derivedDigestCount && settings.every(setting =>
          Object.is(returnedSettings.get(setting.path), setting.value)) &&
          (metadata.sensorVersion !== 4 || returnedSettings.get('sense.contractSha256') === metadata.sensorContract?.sha256);
        if (descriptor.operationId !== operationId || descriptor.runId !== runId ||
            descriptor.boundaryKind !== 'run-start' ||
            descriptor.generation !== '0000000000000001' ||
            descriptor.completedStep !== '0000000000000000' ||
            metadata.runId !== runId || metadata.seed !== seed ||
            !settingsMatch) {
          throw new Error('prepared fresh-run identity is internally inconsistent');
        }
        try {
          assertReplacementCheckpointBudget(descriptor,
            await owner.persistence.inspectRetention(), await owner.persistence.inspectStorage());
        } catch (error) {
          if (error instanceof RangeError && !preexistingManagedNames.has(descriptor.relativeFilename)) {
            newlyPublishedBudgetRejectedFile = resolve(owner.managedDirectory, descriptor.relativeFilename);
          }
          throw error;
        }
        const selected = await owner.persistence.selectStartup();
        if (!selected.descriptor || selected.runId !== activeMetadata.runId) {
          throw new Error('staged fresh run found no matching current checkpoint');
        }
        previousCurrent = { runId: selected.runId,
          checkpointId: selected.descriptor.logicalRootSha256 };
        commitAttempted = true;
        const durable = await owner.persistence.commit(descriptor, null, true);
        committed = true;
        const previousHealth = owner.runtime.health();
        await output.publishPreparedImport(durable.descriptor);
        telemetry.rebase(previousHealth, durable.descriptor.completedStep, metadata.fixedStepSeconds);
        activeMetadata = metadata;
        setHistory([]);
        hallOfFame = [];
        activeCheckpointId = durable.checkpointId;
        recovery = undefined;
        importBranch = undefined;
        legacyConversion = undefined;
        routing.resetAfterImport();
        routingHeld = false;
        disconnectedDuringImport.clear();
        importAuthorityPublished = true;
        const welcome = createRustWelcome(activeMetadata, sessionId, owner.nativeBuildIdentifier, config.rustCalculationWorkers);
        void distillation?.runReplaced().catch(error => { console.error('[distillation.replacement]', error instanceof Error ? error.message : error); });
        inspector?.reset('inspector cleared by authority replacement');
        sockets.replaceWelcome(welcome);
        sockets.enterAwaitingRejoin({
          type: 'stateReplaced', reason, checkpointId: durable.checkpointId, welcome
        });
        retention = await owner.persistence.inspectRetention();
        return { runId, seed, checkpointId: durable.checkpointId };
      } catch (error) {
        if (nativePreparationStarted && !committed) orphanCleanupPending = true;
        if (staged && !committed) {
          let oldPointerStillCurrent = !commitAttempted;
          if (commitAttempted) {
            try {
              const selected = await owner.persistence.selectStartup();
              oldPointerStillCurrent = selected.runId === previousCurrent?.runId &&
                selected.descriptor?.logicalRootSha256 === previousCurrent?.checkpointId;
            } catch { /* An unreadable commit outcome cannot release the old authority. */ }
          }
          if (oldPointerStillCurrent) {
            try {
              await reclaimFailedPublication();
              await output.cancelPreparedImport();
            } catch (cleanupError) { fail(cleanupError); }
          }
          else fail(new Error('fresh replacement checkpoint outcome is unknown; restart from a valid retained checkpoint'));
        }
        else if (prepared && !staged) {
          let discarded = false;
          try { owner.runtime.discardPreparedImport(); discarded = true; }
          catch { /* Candidate may already be gone. */ }
          if (discarded && newlyPublishedBudgetRejectedFile) {
            await unlink(newlyPublishedBudgetRejectedFile).catch(unlinkError => {
              if ((unlinkError as NodeJS.ErrnoException).code !== 'ENOENT') throw unlinkError;
            });
          }
        }
        if (!committed) {
          for (const connection of disconnectedDuringImport) routing.disconnect(connection);
          disconnectedDuringImport.clear();
          routing.flush();
        }
        if (committed) fail(error);
        throw error;
      } finally {
        if (routingHeld && !fault && !stopping) routing.resumeAfterReplacement();
      }
    };
    /** Serialize Reset/New Run with archive and retention work. */
    const startFreshReplacement = (
      connection: number,
      reason: 'reset' | 'newRun',
      seed: number,
      newRunMessage?: NewRunMsg,
      settings: Array<{ path: string; value: number }> = replacementSettings(activeMetadata),
      graphSpec = activeMetadata.graphSpec
    ): void => {
      if (fault || stopping || importOperation || exportOperation || resurrectionOperation || pinning || retentionMaintenance || savedBrains?.busy) {
        const detail = fault ?? (stopping ? 'server is stopping' : 'another persistence operation is in progress');
        if (newRunMessage) {
          sockets.sendJsonTo(connection, {
            type: 'newRunResult', requestId: newRunMessage.requestId, applied: false, reason: detail
          });
        } else {
          sockets.sendJsonTo(connection, { type: 'error', message: `reset failed: ${detail}` });
        }
        return;
      }
      importAuthorityPublished = false;
      importOperation = executeFreshReplacement(reason, seed, settings, graphSpec).then(result => {
        importOperation = undefined;
        importAuthorityPublished = false;
        if (newRunMessage) {
          sockets.sendJsonToAwaitingConnection(connection, {
            type: 'newRunResult', requestId: newRunMessage.requestId, applied: true,
            worldSeed: result.seed, runId: result.runId
          });
        }
      }).catch(error => {
        importOperation = undefined;
        importAuthorityPublished = false;
        if (fault) return;
        const detail = error instanceof Error ? error.message : String(error);
        if (newRunMessage) {
          sockets.sendJsonTo(connection, {
            type: 'newRunResult', requestId: newRunMessage.requestId, applied: false, reason: detail
          });
        } else {
          sockets.sendJsonTo(connection, { type: 'error', message: `reset failed: ${detail}` });
        }
      });
    };
    /** Run one bounded drain without overlapping asynchronous persistence. */
    schedule = (): void => {
      if (scheduled || draining || fault || (stopping && !importOperation && !resurrectionOperation)) return;
      scheduled = setImmediate(() => {
        scheduled = undefined;
        if (fault || (stopping && !importOperation && !resurrectionOperation)) return;
        pumps++;
        const now = performance.now();
        if (now - pumpStart >= 1000) { pumpsPerSecond = pumps * 1000 / (now - pumpStart); pumpStart = now; pumps = 0; }
        try { flushVisualization(); } catch (error) { fail(error); return; }
        draining = output.drain();
        void draining.then(() => routing.flush()).catch(fail).finally(() => { draining = undefined; });
      });
    };
    /** Reject commands while authority is unavailable. */
    const unsupported = (connection: number): void => { sockets.sendJsonTo(connection, { type: 'error', message: fault ?? 'command unavailable while Rust authority is paused' }); };
    /** Convert unexpected admission failures into a terminal interface fault. */
    const route = (action: () => void): void => { try { action(); } catch (error) { fail(error); } schedule(); };
    sockets.setHandlers({
      onJoin(connection, message, client) { route(() => {
        if (fault || stopping || (importOperation && !importAuthorityPublished)) unsupported(connection);
        else routing.join(connection, message, client);
      }); },
      onAction(connection, message) { route(() => {
        // Existing leases keep control during preparation; held input is coalesced until cancellation or swap.
        if (!fault && !stopping) routing.action(connection, message);
      }); },
      onDisconnect(connection) { route(() => {
        for (const [sequence, pending] of pendingSelections) {
          if (pending.connection === connection) pendingSelections.delete(sequence);
        }
        inspector?.disconnect(connection);
        visualizationConnections.delete(connection);
        if (visualizationConnections.size === 0) latestVisualization = undefined;
        flushVisualization();
        if (stopping || fault) return;
        if (importOperation && !importAuthorityPublished) disconnectedDuringImport.add(connection);
        else routing.disconnect(connection);
      }); },
      onReset(connection, message) {
        try {
          const settings = replacementSettings(activeMetadata, message);
          const graphSpec = message.graphSpec === null ? clearedStackGraph(activeMetadata, message, settings)
            : message.graphSpec === undefined ? activeMetadata.graphSpec : message.graphSpec;
          startFreshReplacement(connection, 'reset', activeMetadata.seed, undefined, settings, graphSpec);
        } catch (error) {
          sockets.sendJsonTo(connection, {
            type: 'error', message: `reset failed: ${error instanceof Error ? error.message : String(error)}`
          });
        }
      },
      onSettings(connection, message: LiveSettingsMsg) {
        const normalized = normalizeLiveSettingsUpdates(message.updates);
        if (!normalized.ok) {
          sockets.sendJsonTo(connection, {
            type: 'settingsApplied', requestId: message.requestId, applied: false, updates: [],
            configRevision: wireInteger(activeMetadata.configRevision),
            configHash: activeMetadata.configHash, reason: normalized.reason
          });
          return;
        }
        let admittedSequence: string | undefined;
        const admitted = !fault && !stopping && (!importOperation || importAuthorityPublished) &&
          output.admission.trySubmitControl(sequence => {
            owner.runtime.submitLiveSettings(sequence, normalized.updates);
            admittedSequence = sequence;
          });
        if (!admitted || !admittedSequence) {
          sockets.sendJsonTo(connection, {
            type: 'settingsApplied', requestId: message.requestId, applied: false, updates: [],
            configRevision: wireInteger(activeMetadata.configRevision),
            configHash: activeMetadata.configHash,
            reason: fault ?? (stopping ? 'server is stopping' : 'authoritative command queue is busy')
          });
          return;
        }
        pendingSettings.set(admittedSequence, {
          connection, requestId: message.requestId, updates: normalized.updates
        });
        schedule();
      },
      onFinishGeneration(connection, message) {
        /** Return a bounded rejection to the requesting peer. */
        const rejection = (reason: string): void => {
          sockets.sendJsonTo(connection, { type: 'finishGenerationResult', requestId: message.requestId,
            runId: message.expectedRunId, generation: message.expectedGeneration, status: 'rejected', reason });
        };
        if (pendingFinish) {
          if (pendingFinish.requestId === message.requestId && pendingFinish.runId === message.expectedRunId &&
              pendingFinish.generation === message.expectedGeneration) {
            pendingFinish.connection = connection;
            if (pendingFinish.accepted) sockets.sendJsonTo(connection, { type: 'finishGenerationResult',
              requestId: message.requestId, runId: pendingFinish.runId, generation: pendingFinish.generation,
              status: 'pending', ...(pendingFinish.step !== undefined ? { step: pendingFinish.step } : {}) });
          } else rejection('another finish generation request is already pending');
          return;
        }
        let admittedSequence: string | undefined;
        const admitted = !fault && !stopping && (!importOperation || importAuthorityPublished) &&
          output.admission.trySubmitControl(sequence => {
            owner.runtime.submitFinishGeneration(sequence, message.requestId, message.expectedRunId,
              BigInt(message.expectedGeneration).toString(16).padStart(16, '0'));
            admittedSequence = sequence;
          });
        if (!admitted || !admittedSequence) {
          rejection(fault ?? (stopping ? 'server is stopping' : 'authoritative command queue is busy'));
          return;
        }
        pendingFinish = { connection, requestId: message.requestId, runId: message.expectedRunId,
          generation: message.expectedGeneration, sequence: admittedSequence, accepted: false };
        schedule();
      },
      onSelectEntity(connection, message) {
        if (message.expectedSessionId !== sessionId || pendingSelections.size >= 128 ||
            [...pendingSelections.values()].some(pending => pending.connection === connection)) {
          sockets.sendJsonTo(connection, { type: 'entitySelected', requestId: message.requestId,
            found: false, reason: 'entity selection session is stale or another request is pending' });
          return;
        }
        let admittedSequence: string | undefined;
        const admitted = !fault && !stopping && (!importOperation || importAuthorityPublished) &&
          output.admission.trySubmitControl(sequence => {
            owner.runtime.submitEntitySelection(sequence, message.snakeId !== undefined ? 'frame' : 'slot',
              message.snakeId ?? message.populationSlot!, message.expectedRunId,
              BigInt(message.expectedGeneration).toString(16).padStart(16, '0'));
            admittedSequence = sequence;
          });
        if (!admitted || !admittedSequence) {
          sockets.sendJsonTo(connection, { type: 'entitySelected', requestId: message.requestId,
            found: false, reason: fault ?? 'authoritative command queue is busy' });
          return;
        }
        pendingSelections.set(admittedSequence, { connection, requestId: message.requestId });
        schedule();
      },
      onInspectEntity(connection, message) { route(() => { inspector?.request(connection, message); }); },
      onGodMode(connection, message: GodModeMsg) {
        if (!message.selectionToken) {
          sockets.sendJsonTo(connection, { type: 'godModeResult', requestId: message.requestId,
            action: message.action, snakeId: message.snakeId, applied: false,
            reason: 'resolve selectEntity before mutating a physical life' });
          return;
        }
        if (message.action === 'kill') {
          let admittedSequence: string | undefined;
          const admitted = !fault && !stopping && (!importOperation || importAuthorityPublished) &&
            output.admission.trySubmitControl(sequence => {
              owner.runtime.submitGodModeKill(sequence, message.snakeId, message.selectionToken!);
              admittedSequence = sequence;
            });
          if (!admitted || !admittedSequence) {
            sockets.sendJsonTo(connection, {
              type: 'godModeResult', requestId: message.requestId, action: 'kill',
              snakeId: message.snakeId, applied: false,
              reason: fault ?? (stopping ? 'server is stopping' : 'authoritative command queue is busy')
            });
            return;
          }
          pendingGodModeKills.set(admittedSequence, {
            connection, requestId: message.requestId, snakeId: message.snakeId, token: message.selectionToken
          });
          schedule();
          return;
        }
        let admittedSequence: string | undefined;
        const admitted = !fault && !stopping && (!importOperation || importAuthorityPublished) &&
          output.admission.trySubmitControl(sequence => {
            owner.runtime.submitGodModeMove(sequence, message.snakeId, message.x, message.y, message.selectionToken!);
            admittedSequence = sequence;
          });
        if (!admitted || !admittedSequence) {
          sockets.sendJsonTo(connection, {
            type: 'godModeResult', requestId: message.requestId, action: 'move',
            snakeId: message.snakeId, applied: false,
            reason: fault ?? (stopping ? 'server is stopping' : 'authoritative command queue is busy')
          });
          return;
        }
        pendingGodModeMoves.set(admittedSequence, {
          connection, requestId: message.requestId, snakeId: message.snakeId, token: message.selectionToken
        });
        schedule();
      },
      onNewRun(connection, message) {
        startFreshReplacement(connection, 'newRun', randomBytes(4).readUInt32LE(), message);
      },
      onViz(connection, message) { route(() => {
        if (message.enabled) visualizationConnections.add(connection);
        else visualizationConnections.delete(connection);
        if (visualizationConnections.size === 0) latestVisualization = undefined;
        flushVisualization();
      }); }
    });
    await new Promise<void>((done, reject) => { server.once('error', reject); server.listen(config.port, config.host, () => { server.off('error', reject); done(); }); });
    const address = server.address();
    if (!address || typeof address === 'string') throw new Error('Rust server has no TCP address');
    owner.runtime.start();
    stopAuthorityWatch = watchRunningAuthority(owner.runtime, error => {
      console.error('[rust.authority-watchdog]', error.message);
      fail(error);
      // The stalled in-process coordinator cannot be safely stopped or reused.
      process.exit(1);
    });
    timer = setInterval(schedule, 16);
    schedule();
    return { port: address.port, close };
  } catch (error) { await close(); throw error; }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  void (async () => {
    const config = parseProductionCli(process.argv.slice(2), process.env);
    if (config === null) { console.info(PRODUCTION_CLI_HELP); return; }
    const server = await startRustServer(config);
    const hosts = config.host === '0.0.0.0'
      ? ['127.0.0.1', ...Object.values(networkInterfaces()).flatMap(addresses => addresses?.filter(address => address.family === 'IPv4' && !address.internal).map(address => address.address) ?? [])]
      : [config.host];
    for (const host of new Set(hosts)) {
      const authorityHost = isIP(host) === 6 ? `[${host}]` : host;
      const ws = config.publicWsUrl || `ws://${authorityHost}:${server.port}`;
      if (server.startupFault) console.error(`Rust startup fault: ${server.startupFault}. Health: http://${authorityHost}:${server.port}/api/health`);
      else console.info(`Rust server: http://${authorityHost}:${server.port}/?server=${encodeURIComponent(ws)} (WebSocket ${ws})`);
    }
    process.once('SIGINT', () => { void server.close(); });
    process.once('SIGTERM', () => { void server.close(); });
  })().catch(error => { console.error(error); process.exitCode = 1; });
}
