import type { RustImportBranchNotice, RustRecoveryNotice } from '../src/protocol/rustBackground.ts';
import type { ExperimentalServerRuntime } from './rustEngine/experimentalStartup.ts';
import { createServer } from 'node:http';
import { createReadStream } from 'node:fs';
import { lstat, stat, statfs, unlink } from 'node:fs/promises';
import { dirname, extname, resolve, sep } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { networkInterfaces } from 'node:os';
import { isIP } from 'node:net';
import { randomBytes, randomUUID } from 'node:crypto';
import { DEFAULT_CONFIG, parseConfig, type ServerConfig } from './config.ts';
import { WsHub } from './wsHub.ts';
import { createExperimentalServerRuntime } from './rustEngine/experimentalStartup.ts';
import { BackgroundOutputPump } from './rustEngine/backgroundOutput.ts';
import { ExternalControllerRouting } from './rustEngine/externalRouting.ts';
import { createRustStats, createRustWelcome } from './rustEngine/browserMetadata.ts';
import { ExperimentalRuntimeTelemetry } from './rustEngine/runtimeTelemetry.ts';
import type { CheckpointRetentionInventory } from './rustEngine/checkpointRetention.ts';
import type { ManagedCheckpointExportLease, ManagedImportBranchResult } from './rustEngine/checkpointPersistenceProtocol.ts';
import {
  parseManagedCheckpointDescriptor,
  parseManagedImportInventoryDescriptor
} from './rustEngine/checkpointPersistenceProtocol.ts';
import { spoolArchiveUpload } from './rustEngine/archiveUpload.ts';
import { parseRustStartupMetadata } from './rustEngine/startupMetadata.ts';

/** Repository-owned built browser assets. */
const CLIENT_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../dist');
/** P0 external routing cap, within the native admitted controller capacity. */
const MAX_CONTROLLERS = 16;
/** Browser asset MIME types emitted by Vite. */
const CONTENT_TYPES: Readonly<Record<string, string>> = { '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8', '.css': 'text/css; charset=utf-8', '.svg': 'image/svg+xml', '.png': 'image/png', '.ico': 'image/x-icon' };

/** Reject an export before writing when its scalar worst-case files do not fit. */
async function admitExportSpace(directory: string, lease: ManagedCheckpointExportLease): Promise<void> {
  const population = BigInt(`0x${lease.descriptor.populationCount}`);
  const weights = BigInt(`0x${lease.descriptor.weightCount}`);
  const hallOfFameCount = BigInt(`0x${lease.inventory.hallOfFameCount}`);
  if (population === 0n || weights % population !== 0n) {
    throw new Error('export checkpoint has an invalid population weight shape');
  }
  const hallOfFameRawBytes = hallOfFameCount * (weights / population) * 4n;
  const projectedAdditionalBytes = BigInt(`0x${lease.descriptor.storedByteCount}`) +
    BigInt(`0x${lease.inventory.storedByteCount}`) + hallOfFameRawBytes * 2n + 16n * 1024n * 1024n;
  const space = await statfs(directory, { bigint: true });
  if (space.bavail * space.bsize < projectedAdditionalBytes) {
    throw new Error('insufficient free disk for exact checkpoint export');
  }
}

/** Explicit experimental process ownership returned to tests and the CLI. */
export interface ExperimentalRustServer {
  /** Actual bound port, including an OS-selected test port. */
  port: number;
  /** Explicit health-only startup failure, without an active simulation. */
  startupFault?: string;
  /** Stop sockets, join native execution, then close the metadata worker. */
  close(): Promise<void>;
}

/** Keep bounded diagnostics reachable after failed restore without starting any game or socket authority. */
async function startFaultedServer(config: ServerConfig, error: unknown): Promise<ExperimentalRustServer> {
  const reason = (error instanceof Error ? error.message : String(error)).slice(0, 512);
  const server = createServer((_request, response) => {
    response.writeHead(503, { 'Content-Type': 'application/json', 'Access-Control-Allow-Origin': '*' });
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
    lostCompletedGenerations: through >= recovered ? { from: recovery.recoveredDescriptor.generation,
      through: through.toString(16).padStart(16, '0') } : null };
}

/** Small terminal archive-import response emitted only after all cleanup completes. */
interface ArchiveImportSuccess {
  ok: true;
  runId: string;
  generation: string;
  completedStep: string;
  checkpointId: string;
  saveLogicalRootSha256: string;
  branched: boolean;
  sourceRunId?: string;
}

/** Project durable import lineage for health and welcome messages. */
function importBranchNotice(value: ManagedImportBranchResult | null): RustImportBranchNotice | undefined {
  if (!value) return undefined;
  return { sourceRunId: value.sourceRunId, branchRunId: value.branchRunId,
    sourceGeneration: value.sourceGeneration, sourceCheckpointId: value.sourceCheckpointId };
}

/** Start the fixed native P0 profile from fresh or retained managed authority. */
export async function startExperimentalRustServer(config: ServerConfig): Promise<ExperimentalRustServer> {
  if (resolve(config.dbPath) === resolve(DEFAULT_CONFIG.dbPath)) {
    throw new Error('experimental Rust startup requires a dedicated managed --db-path');
  }
  if (config.inferenceBackend !== 'native' || config.mtEnabled || config.controllerInputHoldMs !== 500 ||
      config.controllerDisconnectGraceMs !== 30_000 || config.checkpointEveryGenerations !== 1) {
    throw new Error('experimental Rust startup supports native scalar P0, default controller timing, and every-generation checkpoints');
  }
  let schedule = (): void => {};
  let owner: ExperimentalServerRuntime;
  try {
    if (typeof config.resume === 'number') throw new Error('numeric reference snapshot IDs are not managed checkpoint IDs');
    owner = await createExperimentalServerRuntime({ databasePath: config.dbPath,
      managedDirectory: `${resolve(config.dbPath)}.checkpoints`,
      ...(config.resume === 'latest' ? { restoreLatest: true } : {}),
      ...(config.resume.startsWith('sha256:') ? { restoreCheckpointId: config.resume.slice(7) } : {}),
      ...(config.seed === undefined ? {} : { seed: config.seed }), onWake: () => schedule() });
  } catch (error) { return startFaultedServer(config, error); }
  let recovery = recoveryNotice(owner);
  let importBranch = importBranchNotice(owner.importBranch);
  if (recovery) console.warn('[rust.recovery]', recovery);
  let activeMetadata = owner.metadata;
  let activeCheckpointId = owner.runStart.checkpointId;
  let retention: CheckpointRetentionInventory;
  let retentionCleanup: { deletedCheckpointCount: number; deletedStoredByteCount: string };
  try {
    const initialCleanup = await owner.persistence.applyRetention();
    retention = initialCleanup.inventory;
    retentionCleanup = { deletedCheckpointCount: initialCleanup.deletedCheckpointCount,
      deletedStoredByteCount: initialCleanup.deletedStoredByteCount };
  }
  catch (error) { await owner.close().catch(() => {}); return startFaultedServer(config, error); }
  const telemetry = new ExperimentalRuntimeTelemetry(owner.runtime.health(), owner.metadata.fixedStepSeconds);
  let fault: string | undefined;
  let stopping = false;
  let scheduled: NodeJS.Immediate | undefined;
  let timer: NodeJS.Timeout | undefined;
  let draining: Promise<boolean> | undefined;
  let lastFrame = 0;
  let lastStats = 0;
  let pumpStart = performance.now();
  let pumps = 0;
  let pumpsPerSecond = 0;
  let pinning: Promise<void> | undefined;
  let retentionMaintenance: Promise<void> | undefined;
  let exportOperation: Promise<void> | undefined;
  let activeExportResponse: import('node:http').ServerResponse | undefined;
  let importOperation: Promise<void> | undefined;
  let activeImportRequest: import('node:http').IncomingMessage | undefined;
  let activeImportResponse: import('node:http').ServerResponse | undefined;
  let importAuthorityPublished = false;
  const disconnectedDuringImport = new Set<number>();
  let executeImport: ((request: import('node:http').IncomingMessage,
    resumeAsBranch: boolean) => Promise<ArchiveImportSuccess>) | undefined;

  /** Keep population-sized bytes in Rust/filesystem/browser networking for one exact lease. */
  const serveExport = async (
    response: import('node:http').ServerResponse<import('node:http').IncomingMessage>
  ): Promise<void> => {
    const lease = await owner.persistence.acquireCurrentExportLease();
    const readyPath = resolve(owner.managedDirectory, `.${lease.operationId}.slither-save.ready`);
    try {
      await admitExportSpace(owner.managedDirectory, lease);
      const ready = await owner.runtime.prepareExportArchive(
        owner.managedDirectory, lease.operationId, lease.descriptor, lease.inventory
      );
      if (ready.operationId !== lease.operationId ||
          ready.checkpointId !== lease.descriptor.logicalRootSha256 ||
          ready.relativeFilename !== `.${lease.operationId}.slither-save.ready` ||
          !/^[0-9a-f]{64}$/u.test(ready.logicalRootSha256) ||
          !/^slither-neuroevo-[0-9a-f]{12}-gen-[0-9]+-v1\.slither-save$/u.test(ready.downloadFilename) ||
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
        const finish = (error?: Error): void => {
          if (settled) return;
          settled = true;
          stream.destroy();
          if (error) reject(error);
          else done();
        };
        stream.once('error', finish);
        response.once('finish', () => finish());
        response.once('close', () => finish());
        stream.pipe(response);
      });
    } finally {
      try {
        await unlink(readyPath).catch(error => {
          if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error;
        });
      } finally {
        await owner.persistence.releaseExportLease(lease.operationId);
      }
    }
  };
  const server = createServer((request, response) => {
    response.setHeader('Access-Control-Allow-Origin', '*');
    response.setHeader('Access-Control-Allow-Methods', 'GET, POST, OPTIONS');
    response.setHeader('Access-Control-Allow-Headers', 'Content-Type');
    if (request.method === 'OPTIONS') { response.writeHead(204); response.end(); return; }
    const requestUrl = new URL(request.url ?? '/', 'http://localhost');
    const pathname = requestUrl.pathname;
    if (pathname === '/api/health' || pathname === '/health') {
      const nativeHealth = owner.runtime.health();
      response.writeHead(fault ? 503 : 200, { 'Content-Type': 'application/json' });
      response.end(JSON.stringify({ ok: !fault, authority: 'rust', runId: activeMetadata.runId,
        seed: activeMetadata.seed, startupCheckpointId: activeCheckpointId, ...nativeHealth,
        telemetry: telemetry.snapshot(nativeHealth), outbound: hub?.getOutboundDiagnostics(),
        retention, retentionCleanup,
        ...(recovery ? { recovery } : {}), ...(importBranch ? { importBranch } : {}),
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
      if (importOperation || exportOperation || pinning || retentionMaintenance) {
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
      importOperation = executeImport(request, importMode === 'branch').then(result => {
        finishImport();
        if (response.destroyed) return;
        response.writeHead(200, { 'Content-Type': 'application/json' });
        response.end(JSON.stringify(result));
      }).catch(error => {
        finishImport();
        if (response.destroyed) return;
        if (response.headersSent) {
          response.destroy();
          return;
        }
        const message = error instanceof Error ? error.message : String(error);
        const requiresBranch = message.includes('resume it as a branch');
        response.writeHead(requiresBranch ? 409 : fault ? 503 : 400, { 'Content-Type': 'application/json' });
        response.end(JSON.stringify({ ok: false, message,
          ...(requiresBranch ? { code: 'IMPORT_REQUIRES_BRANCH' } : {}) }));
      });
      return;
    }
    if (request.method === 'GET' && pathname === '/api/export/latest') {
      if (fault || stopping) {
        response.writeHead(503, { 'Content-Type': 'application/json' });
        response.end(JSON.stringify({ ok: false, message: fault ?? 'server is stopping' }));
        return;
      }
      if (exportOperation || importOperation || pinning || retentionMaintenance) {
        response.writeHead(409, { 'Content-Type': 'application/json' });
        response.end(JSON.stringify({ ok: false, message: 'another persistence operation is in progress' }));
        return;
      }
      activeExportResponse = response;
      exportOperation = serveExport(response).catch(error => {
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
    if (request.method === 'POST' && pathname === '/api/checkpoints/current/pin') {
      if (fault || stopping) {
        response.writeHead(503, { 'Content-Type': 'application/json' });
        response.end(JSON.stringify({ ok: false, message: fault ?? 'server is stopping' }));
        return;
      }
      if (pinning || importOperation || exportOperation || retentionMaintenance) {
        response.writeHead(409, { 'Content-Type': 'application/json' });
        response.end(JSON.stringify({ ok: false, message: 'another persistence operation is in progress' }));
        return;
      }
      pinning = owner.persistence.pinCurrentCheckpoint().then(async pinned => {
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
      response.end(JSON.stringify({ error: 'not available in the experimental P0 runtime' })); return;
    }
    void (async () => {
      const path = resolve(CLIENT_ROOT, `.${decodeURIComponent(pathname === '/' ? '/index.html' : pathname)}`);
      if (!path.startsWith(`${CLIENT_ROOT}${sep}`) || !(await stat(path)).isFile()) {
        response.writeHead(404); response.end(); return;
      }
      response.writeHead(200, { 'Content-Type': CONTENT_TYPES[extname(path)] ?? 'application/octet-stream' });
      const stream = createReadStream(path);
      stream.on('error', () => response.destroy());
      response.on('close', () => stream.destroy());
      stream.pipe(response);
    })().catch(() => { if (!response.headersSent) response.writeHead(404); response.end(); });
  });
  let hub: WsHub | undefined;
  let closePromise: Promise<void> | undefined;
  /** Join every owner exactly once, including startup failure. */
  const close = (): Promise<void> => {
    closePromise ??= (async () => {
      stopping = true;
      if (timer) clearInterval(timer);
      activeImportRequest?.destroy();
      activeImportResponse?.destroy();
      activeExportResponse?.destroy();
      await importOperation?.catch(() => {});
      await pinning?.catch(() => {});
      await retentionMaintenance?.catch(() => {});
      await exportOperation?.catch(() => {});
      if (scheduled) clearImmediate(scheduled);
      hub?.closeAll();
      owner.runtime.requestStop();
      await draining?.catch(() => {});
      try { await owner.close(); }
      finally {
        telemetry.close();
        if (server.listening) await new Promise<void>((done, reject) => server.close(error => error ? reject(error) : done()));
      }
    })();
    return closePromise;
  };
  try {
    hub = new WsHub(server, { ...createRustWelcome(activeMetadata), ...(recovery ? { recovery } : {}),
      ...(importBranch ? { importBranch } : {}) }, { maxConnections: 64 });
    const sockets = hub;
    let routing!: ExternalControllerRouting;
    const output = new BackgroundOutputPump({
      owner, maxControllers: MAX_CONTROLLERS,
      send: (id, message) => sockets.sendJsonTo(Number(BigInt(`0x${id}`)), message),
      event(event) {
        routing.event(event);
        const now = performance.now();
        if (event.display) {
          telemetry.observeDisplay(event.display);
          sockets.updateWelcome({ frameByteLength: event.display.frameByteLength });
          if (now - lastStats >= 1000 / config.uiFrameRateHz) {
            lastStats = now;
            sockets.broadcastStats(createRustStats(event.display, activeMetadata, pumpsPerSecond));
          }
        }
      },
      hasFrameRecipients: () => sockets.hasFrameRecipients() && performance.now() - lastFrame >= 1000 / config.uiFrameRateHz,
      frame(lease) { lastFrame = performance.now(); sockets.broadcastFrame(lease.bytes, lease.release); },
      observeCheckpointBarrier: durationMs => {
        telemetry.observeCheckpointBarrier(durationMs);
        if (retentionMaintenance) {
          fault ??= 'checkpoint retention maintenance overlapped a durable generation';
          owner.runtime.requestStop();
          return;
        }
        retentionMaintenance = owner.persistence.applyRetention().then(result => {
          retention = result.inventory;
          retentionCleanup = { deletedCheckpointCount: result.deletedCheckpointCount,
            deletedStoredByteCount: result.deletedStoredByteCount };
        }).catch(error => {
          fault ??= error instanceof Error ? error.message : String(error);
          owner.runtime.requestStop();
        }).finally(() => { retentionMaintenance = undefined; });
      }
    });
    routing = new ExternalControllerRouting({ native: owner.runtime, admission: output.admission,
      maxControllers: MAX_CONTROLLERS, maxActionsPerSecond: config.maxActionsPerSecond, maxActionsPerTick: config.maxActionsPerTick,
      send: (connection, message) => sockets.sendJsonTo(connection, message),
      observeActionLatency: (kind, durationMs) => telemetry.observeAction(kind, durationMs),
      observeLifecycleLatency: (kind, operation, durationMs) =>
        telemetry.observeControllerLifecycle(kind, operation, durationMs),
      observeDisconnect: kind => telemetry.observeControllerDisconnect(kind) });
    /** Keep health available after a terminal native/interface failure. */
    const fail = (error: unknown): void => {
      if (!fault) sockets.broadcastError(error instanceof Error ? error.message : String(error));
      fault ??= error instanceof Error ? error.message : String(error);
      owner.runtime.requestStop();
    };
    /** Keep upload bytes and the complete replacement outside JavaScript memory. */
    executeImport = async (request, resumeAsBranch): Promise<ArchiveImportSuccess> => {
      const operationId = randomBytes(16).toString('hex');
      const branchRunId = resumeAsBranch ? randomUUID() : null;
      let uploadPath: string | undefined;
      let inventoryPath: string | undefined;
      let prepared = false;
      let staged = false;
      let committed = false;
      try {
        const upload = await spoolArchiveUpload({
          source: request,
          contentLength: request.headers['content-length'],
          scratchDirectory: owner.managedDirectory,
          operationId
        });
        uploadPath = upload.readyPath;
        const imported = await owner.runtime.prepareImportArchive(
          upload.readyPath,
          owner.managedDirectory,
          owner.managedDirectory,
          operationId
        );
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
        await output.stagePreparedImport();
        staged = true;
        const durable = await owner.persistence.commitImport(descriptor, inventory, branchRunId);
        committed = true;
        if ((durable.importBranch?.branchRunId ?? null) !== branchRunId) {
          throw new Error('committed import branch identity is inconsistent');
        }
        await output.publishPreparedImport(durable.descriptor, branchRunId ?? undefined);
        activeMetadata = branchRunId === null ? metadata : { ...metadata, runId: branchRunId };
        activeCheckpointId = durable.checkpointId;
        recovery = undefined;
        importBranch = importBranchNotice(durable.importBranch ?? null);
        routing.resetAfterImport();
        disconnectedDuringImport.clear();
        importAuthorityPublished = true;
        const welcome = { ...createRustWelcome(activeMetadata), ...(importBranch ? { importBranch } : {}) };
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
          branched: branchRunId !== null,
          ...(branchRunId === null ? {} : { sourceRunId: descriptor.runId })
        };
      } catch (error) {
        if (staged && !committed) await output.cancelPreparedImport().catch(fail);
        else if (prepared && !staged) {
          try { owner.runtime.discardPreparedImport(); } catch { /* Candidate may already be gone. */ }
        }
        if (!committed) {
          for (const connection of disconnectedDuringImport) routing.disconnect(connection);
          disconnectedDuringImport.clear();
          routing.flush();
        }
        if (committed) fail(error);
        throw error;
      } finally {
        for (const path of [uploadPath, inventoryPath]) {
          if (path) await unlink(path).catch(error => {
            if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error;
          });
        }
      }
    };
    /** Run one bounded drain without overlapping asynchronous persistence. */
    schedule = (): void => {
      if (scheduled || draining || fault || (stopping && !importOperation)) return;
      scheduled = setImmediate(() => {
        scheduled = undefined;
        if (fault || (stopping && !importOperation)) return;
        pumps++;
        const now = performance.now();
        if (now - pumpStart >= 1000) { pumpsPerSecond = pumps * 1000 / (now - pumpStart); pumpStart = now; pumps = 0; }
        draining = output.drain();
        void draining.then(() => routing.flush()).catch(fail).finally(() => { draining = undefined; });
      });
    };
    /** Reject secondary commands explicitly until their planned migration slice. */
    const unsupported = (connection: number): void => { sockets.sendJsonTo(connection, { type: 'error', message: fault ?? 'command unavailable in experimental P0' }); };
    /** Convert unexpected admission failures into a terminal interface fault. */
    const route = (action: () => void): void => { try { action(); } catch (error) { fail(error); } schedule(); };
    sockets.setHandlers({
      onJoin(connection, message, client) { route(() => {
        if (fault || stopping || (importOperation && !importAuthorityPublished)) unsupported(connection);
        else routing.join(connection, message, client);
      }); },
      onAction(connection, message) { route(() => {
        if (!fault && !stopping && (!importOperation || importAuthorityPublished)) routing.action(connection, message);
      }); },
      onDisconnect(connection) { route(() => {
        if (stopping || fault) return;
        if (importOperation && !importAuthorityPublished) disconnectedDuringImport.add(connection);
        else routing.disconnect(connection);
      }); },
      onReset: unsupported, onSettings: unsupported, onGodMode: unsupported, onNewRun: unsupported,
      onViz: unsupported
    });
    await new Promise<void>((done, reject) => { server.once('error', reject); server.listen(config.port, config.host, () => { server.off('error', reject); done(); }); });
    const address = server.address();
    if (!address || typeof address === 'string') throw new Error('experimental server has no TCP address');
    owner.runtime.start();
    timer = setInterval(schedule, 16);
    schedule();
    return { port: address.port, close };
  } catch (error) { await close(); throw error; }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const config = parseConfig(process.argv.slice(2), process.env);
  void startExperimentalRustServer(config).then(server => {
    const hosts = config.host === '0.0.0.0'
      ? ['127.0.0.1', ...Object.values(networkInterfaces()).flatMap(addresses => addresses?.filter(address => address.family === 'IPv4' && !address.internal).map(address => address.address) ?? [])]
      : [config.host];
    for (const host of new Set(hosts)) {
      const authorityHost = isIP(host) === 6 ? `[${host}]` : host;
      const ws = config.publicWsUrl || `ws://${authorityHost}:${server.port}`;
      if (server.startupFault) console.error(`Rust startup fault: ${server.startupFault}. Health: http://${authorityHost}:${server.port}/api/health`);
      else console.info(`Rust P0: http://${authorityHost}:${server.port}/?server=${encodeURIComponent(ws)} (WebSocket ${ws})`);
    }
    process.once('SIGINT', () => { void server.close(); });
    process.once('SIGTERM', () => { void server.close(); });
  }).catch(error => { console.error(error); process.exitCode = 1; });
}
