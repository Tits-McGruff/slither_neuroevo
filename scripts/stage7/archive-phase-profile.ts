/** Profile real archive operations in a separate production-server process. */
import { spawn, type ChildProcess } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createReadStream, createWriteStream, existsSync } from 'node:fs';
import { copyFile, mkdir, realpath, rm, stat, statfs, writeFile } from 'node:fs/promises';
import { IncomingMessage, Server, ServerResponse } from 'node:http';
import { cpus, totalmem } from 'node:os';
import { createRequire } from 'node:module';
import { resolve } from 'node:path';
import { Writable } from 'node:stream';
import { fileURLToPath } from 'node:url';
import WebSocket from 'ws';
import { DEFAULT_CONFIG } from '../../server/config.ts';
import { startRustServer } from '../../server/rustServer.ts';
import { CheckpointPersistenceClient } from '../../server/rustEngine/checkpointPersistenceClient.ts';
import type { RustArchiveWorkProgress } from '../../server/rustEngine/backgroundRuntime.ts';
import { ExperimentalRuntimeTelemetry } from '../../server/rustEngine/runtimeTelemetry.ts';
import { computeNativeSourceIdentity } from '../../server/rustEngine/nativeSourceIdentity.ts';
import type { ExperimentalRunningAuthorityNativeHandle } from '../../server/rustEngine/backgroundRuntime.ts';
import { BackgroundGenerationRouter } from '../../server/rustEngine/backgroundGeneration.ts';
import { ExperimentalFreshRunSession } from '../../server/rustEngine/experimentalFreshRunSession.ts';

/** Small authoritative health projection; all memory belongs to the child server. */
interface Health {
  /** Authority status. */
  ok: boolean;
  /** Exact native provenance. */
  nativeBuildIdentifier: string;
  /** Current generation. */
  generation: string;
  /** Honest compatibility classification after an old JSON population upload. */
  legacyConversion?: { sourceFormat: string; completeness: string; exactContinuation: boolean };
  /** Most recent exact Rust archive job. */
  archiveWork: RustArchiveWorkProgress | null;
  /** Child-process memory and event-loop measurements. */
  telemetry: { process: { rssBytes: number; heapUsedBytes: number; externalBytes: number;
    eventLoopDelayP95Ms: number; eventLoopDelayP99Ms: number } };
}

/** One bounded memory observation aligned to the Rust job's own clock. */
interface Sample {
  /** Exact archive operation. */
  operationId: string;
  /** Microseconds read with this health response. */
  elapsedMicros: number;
  /** Child server resident bytes. */
  rssBytes: number;
  /** Child main-thread JS heap bytes. */
  heapUsedBytes: number;
  /** Child external-memory bytes. */
  externalBytes: number;
}

/** Stop only this invocation's child and require normal shutdown. */
export async function stop(child: ChildProcess): Promise<void> {
  if (child.exitCode !== null || child.signalCode !== null) return;
  const closed = new Promise<void>((done, reject) => {
    child.once('close', code => code === 0 ? done() : reject(new Error(`profile server exited ${code}`)));
  });
  const timeout = setTimeout(() => child.kill(), 10_000);
  try { child.send({ type: 'stop' }); await closed; }
  finally { clearTimeout(timeout); }
}

/** Observe actual metadata-worker requests without changing their arguments or results. */
export async function childServer(databasePath: string, checkpointTimings = false, rustWorkers = 6, resumeLegacy = false): Promise<void> {
  if (![4, 5, 6].includes(rustWorkers)) throw new Error('archive profile requires four, five or six calculation workers');
  const originalCommit = CheckpointPersistenceClient.prototype.commitImport;
  const originalAcquire = CheckpointPersistenceClient.prototype.acquireCurrentExportLease;
  const originalDispatch = Server.prototype.emit;
  const originalIncoming = IncomingMessage.prototype.emit;
  const originalWrite = ServerResponse.prototype.write;
  const originalAction = ExperimentalRuntimeTelemetry.prototype.observeAction;
  const originalBarrier = ExperimentalRuntimeTelemetry.prototype.observeCheckpointBarrier;
  const originalGeneration = BackgroundGenerationRouter.prototype.handle;
  const originalCheckpointCommit = CheckpointPersistenceClient.prototype.commit;
  const addon = checkpointTimings ? createRequire(import.meta.url)(resolve('native/index.js')) as {
    ExperimentalRunningAuthority: { prototype: ExperimentalRunningAuthorityNativeHandle }
  } : undefined;
  const originalExport = addon?.ExperimentalRunningAuthority.prototype.prepareExportArchive;
  if (addon && originalExport) {
    /** Bracket the actual synchronous router start and finish clocks conservatively. */
    let transitionClock: { beforeMs: number; afterMs: number } | undefined;
    /** Original telemetry duration, captured during the unchanged finish handler. */
    let observedBarrierDuration: number | undefined;
    addon.ExperimentalRunningAuthority.prototype.prepareExportArchive = function(...args) {
      const beforeMs = performance.now();
      const result = originalExport.apply(this, args);
      process.send?.({ type: 'export-clock', operationId: args[1], beforeMs, afterMs: performance.now(),
        checkpointId: args[2].logicalRootSha256, generation: args[2].generation });
      return result;
    };
    BackgroundGenerationRouter.prototype.handle = function(event) {
      const beforeMs = performance.now();
      const result = originalGeneration.call(this, event);
      const afterMs = performance.now();
      if (event.kind === 'generationTransitionPending') {
        transitionClock = { beforeMs, afterMs };
        observedBarrierDuration = undefined;
        process.send?.({ type: 'checkpoint-start', atMs: beforeMs });
      }
      if (event.kind === 'generationStartPublished' && transitionClock && observedBarrierDuration !== undefined) {
        process.send?.({ type: 'checkpoint-barrier', startedMs: transitionClock.afterMs, finishedMs: beforeMs,
          durationMs: observedBarrierDuration, startClock: transitionClock, finishClock: { beforeMs, afterMs } });
        transitionClock = undefined;
      }
      return result;
    };
    ExperimentalRuntimeTelemetry.prototype.observeCheckpointBarrier = function(durationMs) {
      originalBarrier.call(this, durationMs);
      observedBarrierDuration = durationMs;
    };
    CheckpointPersistenceClient.prototype.commit = async function(...args) {
      const startedMs = performance.now();
      const result = await originalCheckpointCommit.apply(this, args);
      process.send?.({ type: 'checkpoint-commit', atMs: performance.now(), requestMs: performance.now() - startedMs,
        generation: result.descriptor.generation, checkpointId: result.descriptor.logicalRootSha256,
        operationId: result.descriptor.operationId, runId: result.descriptor.runId });
      return result;
    };
  }
  /** Retain response intervals so late applications remain attributed to their input window. */
  const transfers: Array<{ phase: 'upload-import' | 'export-download'; started: number; finished?: number }> = [];
  ExperimentalRuntimeTelemetry.prototype.observeAction = function(kind, durationMs) {
    const receivedAt = performance.now() - durationMs;
    originalAction.call(this, kind, durationMs);
    const transfer = transfers.findLast(window => receivedAt >= window.started &&
      (window.finished === undefined || receivedAt <= window.finished));
    process.send?.({ type: 'action', phase: transfer?.phase ?? 'idle', kind, durationMs });
  };
  /** Scalar observations only; no body bytes or request objects enter reports. */
  const uploads = new WeakMap<IncomingMessage, { started: number; rssBefore: number;
    peakRssBytes: number; bodyBytes: number; samples: number }>();
  /** Actual response writes after preparation; all file bytes remain in the original stream. */
  const downloads = new WeakMap<ServerResponse, { started?: number; rssBefore?: number;
    peakRssBytes: number; bodyBytes: number; samples: number }>();
  Server.prototype.emit = function(event: string | symbol, ...args: unknown[]): boolean {
    if (event === 'request') {
      const request = args[0] as IncomingMessage;
      const response = args[1] as ServerResponse;
      if (request.url === '/api/import/archive' || request.url === '/api/export/latest') {
        const transfer = { phase: request.method === 'POST' ? 'upload-import' as const : 'export-download' as const,
          started: performance.now(), finished: undefined as number | undefined };
        transfers.push(transfer);
        response.once('close', () => { transfer.finished = performance.now(); });
      }
      if (request.method === 'POST' && request.url === '/api/import/archive') {
        const rss = process.memoryUsage.rss();
        const observation = { started: performance.now(), rssBefore: rss,
          peakRssBytes: rss, bodyBytes: 0, samples: 1 };
        uploads.set(request, observation);
        request.once('end', () => {
          observation.peakRssBytes = Math.max(observation.peakRssBytes, process.memoryUsage.rss());
          process.send?.({ type: 'boundary', phase: 'http-upload-spooling',
            durationMs: performance.now() - observation.started, ...observation, samples: observation.samples + 1 });
          uploads.delete(request);
        });
        response.once('close', () => uploads.delete(request));
      }
      if (request.method === 'GET' && request.url === '/api/export/latest') {
        const observation = { started: undefined as number | undefined, rssBefore: undefined as number | undefined,
          peakRssBytes: 0, bodyBytes: 0, samples: 0 };
        downloads.set(response, observation);
        response.once('finish', () => {
          observation.peakRssBytes = Math.max(observation.peakRssBytes, process.memoryUsage.rss());
          process.send?.({ type: 'boundary', phase: 'http-file-download',
            durationMs: observation.started === undefined ? null : performance.now() - observation.started,
            ...observation, samples: observation.samples + 1 });
          downloads.delete(response);
        });
        response.once('close', () => downloads.delete(response));
      }
    }
    return Reflect.apply(originalDispatch, this, [event, ...args]) as boolean;
  };
  IncomingMessage.prototype.emit = function(event: string | symbol, ...args: unknown[]): boolean {
    const upload = uploads.get(this);
    if (upload && event === 'data') {
      const chunk = args[0] as Buffer;
      upload.bodyBytes += chunk.byteLength;
      upload.peakRssBytes = Math.max(upload.peakRssBytes, process.memoryUsage.rss());
      upload.samples++;
    }
    // Delegate the original dispatch; adding a data listener would start flowing
    // the request before the real disk spooler is ready to consume it.
    const result = Reflect.apply(originalIncoming, this, [event, ...args]) as boolean;
    if (upload && event === 'data') {
      upload.peakRssBytes = Math.max(upload.peakRssBytes, process.memoryUsage.rss());
      upload.samples++;
    }
    return result;
  };
  ServerResponse.prototype.write = function(...args: unknown[]): boolean {
    const download = downloads.get(this);
    if (download) {
      const rss = process.memoryUsage.rss();
      if (download.started === undefined) { download.started = performance.now(); download.rssBefore = rss; }
      download.bodyBytes += Buffer.isBuffer(args[0]) ? args[0].byteLength : Buffer.byteLength(String(args[0]));
      download.peakRssBytes = Math.max(download.peakRssBytes, rss);
      download.samples++;
    }
    const result = Reflect.apply(originalWrite, this, args) as boolean;
    if (download) {
      download.peakRssBytes = Math.max(download.peakRssBytes, process.memoryUsage.rss());
      download.samples++;
    }
    return result;
  };
  /** Report endpoint memory and duration around the real worker exchange. */
  async function measure<T>(phase: string, operation: () => Promise<T>): Promise<T> {
    const started = performance.now();
    const rssBefore = process.memoryUsage.rss();
    let peakRssBytes = rssBefore;
    let samples = 1;
    const timer = setInterval(() => { peakRssBytes = Math.max(peakRssBytes, process.memoryUsage.rss()); samples++; }, 10);
    try { return await operation(); }
    finally {
      clearInterval(timer);
      peakRssBytes = Math.max(peakRssBytes, process.memoryUsage.rss());
      process.send?.({ type: 'boundary', phase, durationMs: performance.now() - started,
        rssBefore, peakRssBytes, samples: samples + 1 });
    }
  }
  CheckpointPersistenceClient.prototype.commitImport = function(...args) {
    return measure('sqlite-import-worker-request', () => originalCommit.apply(this, args));
  };
  CheckpointPersistenceClient.prototype.acquireCurrentExportLease = function() {
    return measure('sqlite-export-source-worker-request', () => originalAcquire.call(this));
  };
  const originalLegacy = ExperimentalFreshRunSession.prototype.initializeFromLegacySqlite;
  const originalRunStart = ExperimentalFreshRunSession.prototype.commitPendingRunStart;
  const originalActivation = ExperimentalFreshRunSession.prototype.activateRunningAuthority;
  if (resumeLegacy) {
    ExperimentalFreshRunSession.prototype.initializeFromLegacySqlite = function(...args) {
      return measure('legacy-database-decode-and-candidate', () => originalLegacy.apply(this, args));
    };
    ExperimentalFreshRunSession.prototype.commitPendingRunStart = function(...args) {
      return measure('managed-publication-and-sqlite-commit', () => originalRunStart.apply(this, args));
    };
    ExperimentalFreshRunSession.prototype.activateRunningAuthority = function() {
      return measure('candidate-activation', () => originalActivation.call(this));
    };
    process.send?.({ type: 'boundary', phase: 'before-legacy-startup', rssBefore: process.memoryUsage.rss(),
      peakRssBytes: process.memoryUsage.rss(), samples: 1 });
  }
  const { seed: _defaultSeed, ...resumeConfig } = DEFAULT_CONFIG;
  const server = await startRustServer({ ...(resumeLegacy ? resumeConfig : DEFAULT_CONFIG), host: '127.0.0.1', port: 0,
    resume: resumeLegacy ? 'latest' : 'fresh', ...(resumeLegacy ? {} : { seed: 1511506142 }),
    dbPath: databasePath, rustCalculationWorkers: rustWorkers, logLevel: 'error' });
  try {
    if (server.startupFault) throw new Error(`profile startup fault: ${server.startupFault}`);
    process.send?.({ type: 'ready', port: server.port });
    await new Promise<void>((done, reject) => {
      /** Release this child's IPC listener on either normal stop or deadline. */
      const received = (message: unknown): void => {
        if ((message as { type?: unknown })?.type === 'stop') {
          clearTimeout(timeout); process.off('message', received); done();
        }
      };
      const timeout = setTimeout(() => {
        process.off('message', received); reject(new Error('profile child exceeded its bounded lifetime'));
      }, checkpointTimings ? 1_200_000 : 600_000);
      process.on('message', received);
    });
  } finally {
    await server.close();
    Server.prototype.emit = originalDispatch;
    IncomingMessage.prototype.emit = originalIncoming;
    ServerResponse.prototype.write = originalWrite;
    ExperimentalRuntimeTelemetry.prototype.observeAction = originalAction;
    ExperimentalRuntimeTelemetry.prototype.observeCheckpointBarrier = originalBarrier;
    BackgroundGenerationRouter.prototype.handle = originalGeneration;
    CheckpointPersistenceClient.prototype.commit = originalCheckpointCommit;
    CheckpointPersistenceClient.prototype.commitImport = originalCommit;
    CheckpointPersistenceClient.prototype.acquireCurrentExportLease = originalAcquire;
    ExperimentalFreshRunSession.prototype.initializeFromLegacySqlite = originalLegacy;
    ExperimentalFreshRunSession.prototype.commitPendingRunStart = originalRunStart;
    ExperimentalFreshRunSession.prototype.activateRunningAuthority = originalActivation;
    if (addon && originalExport) addon.ExperimentalRunningAuthority.prototype.prepareExportArchive = originalExport;
    if (process.connected) process.disconnect();
  }
}

/** Keep a real player timer independent of sensors and a protocol bot observation-driven. */
export async function control(port: number, kind: 'ui' | 'bot', expectedReplacements = 1): Promise<{ close(): void; check(): void;
  report: { kind: 'ui' | 'bot'; actionsSent: number; assignments: number; replacements: number } }> {
  const socket = new WebSocket(`ws://127.0.0.1:${port}`);
  const ready = Promise.withResolvers<void>();
  const report = { kind, actionsSent: 0, assignments: 0, replacements: 0 };
  let snakeId: number | undefined;
  let tick = 0;
  let lastBotTick = -1;
  let fault: Error | undefined;
  /** Send only current routing scalars through the unchanged production protocol. */
  function send(): void {
    if (snakeId === undefined || socket.readyState !== WebSocket.OPEN) return;
    socket.send(JSON.stringify({ type: 'action', snakeId, tick, turn: 0.25, boost: 0 }));
    report.actionsSent++;
  }
  socket.on('open', () => socket.send(JSON.stringify({ type: 'hello', version: 2, clientType: kind })));
  socket.on('message', (data, binary) => {
    if (binary) return;
    const message = JSON.parse(data.toString()) as Record<string, unknown>;
    if (message['type'] === 'welcome' || message['type'] === 'stateReplaced') {
      if (message['type'] === 'stateReplaced') { snakeId = undefined; report.replacements++; }
      socket.send(JSON.stringify({ type: 'join', mode: 'player', name: `ArchiveProfile-${kind}` }));
    }
    if (message['type'] === 'assign') {
      snakeId = Number(message['snakeId']); tick = 0; lastBotTick = -1; report.assignments++;
    }
    if (message['type'] === 'sensors' && message['snakeId'] === snakeId) {
      tick = Number(message['tick']); ready.resolve();
      if (kind === 'bot' && tick !== lastBotTick) { lastBotTick = tick; send(); }
    }
    if (message['type'] === 'error') { fault = new Error(String(message['message'])); ready.reject(fault); }
  });
  socket.on('error', error => { fault = error; ready.reject(error); });
  const timer = kind === 'ui' ? setInterval(send, 1000 / 30) : undefined;
  const timeout = setTimeout(() => ready.reject(new Error(`${kind} profile controller did not assign`)), 5000);
  try { await ready.promise; }
  catch (error) { if (timer) clearInterval(timer); socket.terminate(); throw error; }
  finally { clearTimeout(timeout); }
  return { report, check() {
    if (fault) throw fault;
    if (socket.readyState !== WebSocket.OPEN) throw new Error(`${kind} profile controller disconnected`);
    if (report.replacements !== expectedReplacements || report.assignments < expectedReplacements + 1) {
      throw new Error(`${kind} profile controller did not retain the expected lifecycle`);
    }
  }, close() {
    if (timer) clearInterval(timer);
    socket.terminate();
  } };
}

/** Stream a file digest with bounded parent-process storage. */
export async function digest(path: string): Promise<string> {
  const hash = createHash('sha256');
  for await (const bytes of createReadStream(path)) hash.update(bytes);
  return hash.digest('hex');
}

/** Remove exact invocation-owned files after verifying the original absolute parent. */
async function cleanup(outputRoot: string, createdRoot: string, databasePath: string, exportPath: string): Promise<void> {
  if (await realpath(outputRoot) !== createdRoot || databasePath !== resolve(createdRoot, 'profile.db') ||
      exportPath !== resolve(createdRoot, 'export.slither-save')) throw new Error('profile cleanup parent changed');
  await rm(`${databasePath}.checkpoints`, { recursive: true, force: true });
  for (const path of [databasePath, `${databasePath}-wal`, `${databasePath}-shm`, exportPath]) await rm(path, { force: true });
}

/** Exercise actual upload, replacement and download while sampling only the child server. */
async function profile(archivePath: string, outputRoot: string, legacyJson = false): Promise<void> {
  if (existsSync(outputRoot)) throw new Error('profile output directory already exists');
  const archiveBytes = (await stat(archivePath)).size;
  if (archiveBytes <= 50 * 1024 * 1024) throw new Error('profile requires an actual save over 50 MiB');
  await mkdir(outputRoot, { recursive: true });
  const createdRoot = await realpath(outputRoot);
  const filesystem = await statfs(createdRoot, { bigint: true });
  if (filesystem.bavail * filesystem.bsize < 12n * 1024n ** 3n) throw new Error('profile requires 12 GiB free');
  const databasePath = resolve(createdRoot, 'profile.db');
  const exportPath = resolve(createdRoot, 'export.slither-save');
  const originalSha256 = await digest(archivePath);
  const profilerSha256 = await digest(fileURLToPath(import.meta.url));
  const nativeSourceSha256 = computeNativeSourceIdentity(resolve('native')).sha256;
  const child = spawn(process.execPath, ['--import', 'tsx', fileURLToPath(import.meta.url), '--child', databasePath], {
    env: { ...process.env, SLITHER_TRACE_ARCHIVE_PHASES: '1' }, stdio: ['ignore', 'pipe', 'pipe', 'ipc']
  });
  let errors = '';
  const boundaries: Record<string, unknown>[] = [];
  const actions: Array<{ phase: string; kind: string; durationMs: number }> = [];
  child.stderr?.on('data', part => { errors = `${errors}${String(part)}`.slice(-8192); });
  child.stdout?.resume();
  child.on('message', value => {
    const message = value as Record<string, unknown>;
    if (message['type'] === 'boundary') boundaries.push(message);
    if (message['type'] === 'action' && actions.length < 60_000) actions.push(message as unknown as typeof actions[number]);
  });
  let sampling = false;
  let sampler: Promise<void> | undefined;
  const samples: Sample[] = [];
  const traces = new Map<string, RustArchiveWorkProgress>();
  const healthLatencyMs: number[] = [];
  let samplingFailure: unknown;
  let baseline: Health | undefined;
  let final: Health | undefined;
  let viewer: WebSocket | undefined;
  const controllers: Array<Awaited<ReturnType<typeof control>>> = [];
  try {
    const port = await new Promise<number>((done, reject) => {
      const timeout = setTimeout(() => reject(new Error(`profile startup timed out: ${errors}`)), 30_000);
      child.once('exit', code => { clearTimeout(timeout); reject(new Error(`profile exited ${code}: ${errors}`)); });
      child.on('message', value => {
        const message = value as { type?: string; port?: number };
        if (message.type === 'ready' && message.port) { clearTimeout(timeout); done(message.port); }
      });
    });
    /** Read scalar memory and bounded timing intervals from the actual server. */
    async function health(): Promise<Health> {
      const started = performance.now();
      const response = await fetch(`http://127.0.0.1:${port}/api/health`, { signal: AbortSignal.timeout(10_000) });
      const value = await response.json() as Health;
      healthLatencyMs.push(performance.now() - started);
      if (!response.ok || !value.ok) throw new Error('profile authority faulted');
      return value;
    }
    // Use the normal live protocol after replacement: the source archive may
    // retain an accelerated fixture rate, which would advance the selected
    // checkpoint during export and distort normal-speed memory measurements.
    viewer = new WebSocket(`ws://127.0.0.1:${port}`);
    const connected = Promise.withResolvers<void>();
    const slowed = Promise.withResolvers<void>();
    viewer.on('message', (data, binary) => {
      if (binary) return;
      const message = JSON.parse(data.toString()) as Record<string, unknown>;
      if (message['type'] === 'welcome' || message['type'] === 'stateReplaced') {
        viewer!.send(JSON.stringify({ type: 'join', mode: 'spectator' }));
        if (message['type'] === 'welcome') connected.resolve();
        else viewer!.send(JSON.stringify({ type: 'settings', requestId: 'profile-normal-rate',
          updates: [{ path: 'simSpeed', value: 1 }] }));
      }
      if (message['type'] === 'settingsApplied' && message['requestId'] === 'profile-normal-rate') {
        if (message['applied'] === true) slowed.resolve();
        else slowed.reject(new Error('profile normal-speed request rejected'));
      }
    });
    viewer.on('error', error => { connected.reject(error); slowed.reject(error); });
    // Retain failure until the dependent wait without creating an unhandled rejection.
    void slowed.promise.catch(() => {});
    viewer.on('open', () => viewer!.send(JSON.stringify({ type: 'hello', version: 2, clientType: 'ui' })));
    await Promise.race([connected.promise, new Promise<never>((_, reject) => {
      const timer = setTimeout(() => reject(new Error('profile viewer timed out')), 5000);
      void connected.promise.finally(() => clearTimeout(timer)).catch(() => {});
    })]);
    baseline = await health();
    for (const kind of ['ui', 'bot'] as const) controllers.push(await control(port, kind));
    sampling = true;
    sampler = (async () => {
      while (sampling) {
        try {
          const value = await health();
          if (value.archiveWork?.phaseTrace) {
            if (value.archiveWork.phaseTrace.truncated) throw new Error('phase history was truncated');
            traces.set(value.archiveWork.operationId, value.archiveWork);
            samples.push({ operationId: value.archiveWork.operationId,
              elapsedMicros: Number(BigInt(`0x${value.archiveWork.phaseTrace.elapsedMicros}`)),
              rssBytes: value.telemetry.process.rssBytes, heapUsedBytes: value.telemetry.process.heapUsedBytes,
              externalBytes: value.telemetry.process.externalBytes });
          }
          if (samples.length > 60_000) throw new Error('profile sample cap reached');
        } catch (error) { samplingFailure = error; break; }
        await new Promise<void>(done => setTimeout(done, 10));
      }
    })();
    const imported = await fetch(`http://127.0.0.1:${port}/api/import/archive`, { method: 'POST',
      headers: { 'Content-Type': 'application/vnd.slither-neuroevo.save', 'Content-Length': String(archiveBytes) },
      body: createReadStream(archivePath) as unknown as BodyInit, duplex: 'half',
      signal: AbortSignal.timeout(180_000) } as RequestInit & { duplex: 'half' });
    const receipt = await imported.json() as { ok: boolean; checkpointId: string; saveLogicalRootSha256: string };
    if (!imported.ok || !receipt.ok) throw new Error(`profile import failed: ${JSON.stringify(receipt)}`);
    await Promise.race([slowed.promise, new Promise<never>((_, reject) => {
      const timer = setTimeout(() => reject(new Error('profile replacement settings timed out')), 5000);
      void slowed.promise.finally(() => clearTimeout(timer)).catch(() => {});
    })]);
    const importedHealth = await health();
    if (legacyJson && (importedHealth.legacyConversion?.sourceFormat !== 'browser-json' ||
        importedHealth.legacyConversion.completeness !== 'population-only' ||
        importedHealth.legacyConversion.exactContinuation !== false)) {
      throw new Error('legacy profile did not publish honest population-only provenance');
    }
    if (importedHealth.archiveWork?.phaseTrace) traces.set(importedHealth.archiveWork.operationId, importedHealth.archiveWork);
    const exported = await fetch(`http://127.0.0.1:${port}/api/export/latest`, { signal: AbortSignal.timeout(180_000) });
    if (!exported.ok || !exported.body || exported.headers.get('x-slither-checkpoint-id') !== receipt.checkpointId) {
      throw new Error('profile export selected a different checkpoint');
    }
    await exported.body.pipeTo(Writable.toWeb(createWriteStream(exportPath, { flags: 'wx' })));
    const exportBytes = (await stat(exportPath)).size;
    if (exportBytes !== Number(exported.headers.get('content-length'))) throw new Error('export length mismatch');
    // Confirm applications after the response too, retaining their original
    // receive-window label rather than hiding slower input behind completion.
    await new Promise<void>(done => setTimeout(done, 1000));
    final = await health();
    if (final.archiveWork?.phaseTrace) traces.set(final.archiveWork.operationId, final.archiveWork);
    sampling = false;
    await sampler;
    if (samplingFailure) throw samplingFailure;
    if (await digest(archivePath) !== originalSha256) throw new Error('source save changed');
    if (!legacyJson && await digest(exportPath) !== originalSha256) throw new Error('exact re-export changed the original archive');
    if (boundaries.filter(boundary => boundary['phase'] === 'http-upload-spooling' &&
        boundary['bodyBytes'] === archiveBytes).length !== 1) throw new Error('actual upload bytes were not observed exactly once');
    if (boundaries.filter(boundary => boundary['phase'] === 'http-file-download' &&
        boundary['bodyBytes'] === exportBytes).length !== 1) throw new Error('actual download bytes were not observed exactly once');
    const jobs = [...traces.values()].map(job => ({ ...job, phases: job.phaseTrace!.intervals.map(interval => {
      if (!interval.finishedMicros) throw new Error('completed profile contains an open phase');
      if (!job.phaseTrace!.rssSamplerStarted || !interval.startRssBytes || !interval.finishRssBytes ||
          !interval.sampledPeakRssBytes || BigInt(`0x${interval.rssSamples}`) < 2n) {
        throw new Error('completed profile lacks native stage memory observations');
      }
      const start = Number(BigInt(`0x${interval.startedMicros}`));
      const end = Number(BigInt(`0x${interval.finishedMicros}`));
      const matched = samples.filter(sample => sample.operationId === job.operationId && sample.elapsedMicros >= start && sample.elapsedMicros <= end);
      return { phase: interval.phase, durationMs: (end - start) / 1000, samples: matched.length,
        sampledPeakRssBytes: matched.length ? Math.max(...matched.map(sample => sample.rssBytes)) : null,
        nativeStartRssBytes: interval.startRssBytes ? Number(BigInt(`0x${interval.startRssBytes}`)) : null,
        nativeFinishRssBytes: interval.finishRssBytes ? Number(BigInt(`0x${interval.finishRssBytes}`)) : null,
        nativeSampledPeakRssBytes: interval.sampledPeakRssBytes ? Number(BigInt(`0x${interval.sampledPeakRssBytes}`)) : null,
        nativeRssSamples: Number(BigInt(`0x${interval.rssSamples}`)) };
    }) }));
    if (jobs.length !== 2 || jobs.some(job => !job.finished)) throw new Error('both complete archive jobs were not observed');
    const actionLatencies = ['upload-import', 'export-download'].flatMap(phase =>
      ['player', 'reinforcementLearning'].map(kind => {
        const values = actions.filter(action => action.phase === phase && action.kind === kind)
          .map(action => action.durationMs).sort((left, right) => left - right);
        if (values.length === 0 || actions.length === 60_000) throw new Error(`incomplete ${phase}/${kind} action measurements`);
        return { phase, kind, samples: values.length, p95Ms: values[Math.ceil(values.length * 0.95) - 1],
          maxMs: values.at(-1) };
      }));
    for (const controller of controllers) controller.check();
    healthLatencyMs.sort((left, right) => left - right);
    await writeFile(resolve(createdRoot, 'profile.json'), JSON.stringify({ originalSha256, archiveBytes, exportBytes,
      sourceFormat: legacyJson ? 'legacy-json' : 'archive-v1',
      nativeSourceSha256, profilerSha256, host: { platform: process.platform, arch: process.arch,
        cpuModel: cpus()[0]?.model, totalMemoryBytes: totalmem() },
      receipt, exportedSaveRoot: exported.headers.get('x-slither-save-root'), exportSha256: await digest(exportPath),
      baseline, afterImport: importedHealth, final, jobs, boundaries,
      controls: controllers.map(controller => controller.report), actionLatencies,
      postResponseObservationMs: 1000,
      sampleCount: samples.length, requestedSampleIntervalMs: 10,
      healthP95Ms: healthLatencyMs[Math.ceil(healthLatencyMs.length * 0.95) - 1],
      healthMaxMs: Math.max(...healthLatencyMs),
      scope: 'Separate parent HTTP/file client and production child server with opt-in Rust diagnostic sampling. Native readings observe whole-process RSS at each stage entry/exit and on a requested two-millisecond cadence; sampled peaks are lower bounds and include Node, workers, the current game and any staged candidate. HTTP samples are separate and may miss short stages. Upload spooling uses passive actual chunk/end observations, without a data listener or retained body. Worker-request boundary memory includes IPC and FULL commit. Real WebSocket player actions use an independent 30-Hz timer; the protocol bot sends one action per delivered observation. Action latency is actual server receipt to Rust application, attributed by its receive timestamp to the live HTTP response interval, with a one-second post-response observation tail. It is not physical browser/LAN or PyRL training evidence. This does not prove isolated archive I/O overhead, all legacy readers, checkpoint-overlap durability or final A4 acceptance.' }, null, 2) + '\n', { flag: 'wx' });
  } catch (error) {
    console.error('Archive profiling operation failed:', error);
    throw error;
  } finally {
    sampling = false;
    await sampler;
    viewer?.terminate();
    for (const controller of controllers) controller.close();
    try { await stop(child); }
    finally { await cleanup(outputRoot, createdRoot, databasePath, exportPath); }
  }
}

/** Measure old database conversion in a fresh child, then its ordinary archive export. */
async function profileLegacyDatabase(sourcePath: string, outputRoot: string): Promise<void> {
  if (existsSync(outputRoot)) throw new Error('profile output directory already exists');
  await mkdir(outputRoot, { recursive: true });
  const createdRoot = await realpath(outputRoot);
  const filesystem = await statfs(createdRoot, { bigint: true });
  if (filesystem.bavail * filesystem.bsize < 12n * 1024n ** 3n) throw new Error('profile requires 12 GiB free');
  const databasePath = resolve(createdRoot, 'profile.db');
  const exportPath = resolve(createdRoot, 'export.slither-save');
  const sourceSha256 = await digest(sourcePath);
  await copyFile(sourcePath, databasePath);
  const child = spawn(process.execPath, ['--import', 'tsx', fileURLToPath(import.meta.url), '--child-legacy', databasePath], {
    env: { ...process.env, SLITHER_TRACE_ARCHIVE_PHASES: '1' }, stdio: ['ignore', 'pipe', 'pipe', 'ipc']
  });
  const boundaries: Record<string, unknown>[] = [];
  const actions: Array<{ kind: string; durationMs: number }> = [];
  const traces = new Map<string, RustArchiveWorkProgress>();
  const healthLatencies: number[] = [];
  let errors = '';
  child.stdout?.resume();
  child.stderr?.on('data', part => { errors = `${errors}${String(part)}`.slice(-8192); });
  child.on('message', value => {
    const message = value as Record<string, unknown>;
    if (message['type'] === 'boundary') boundaries.push(message);
    if (message['type'] === 'action') actions.push(message as unknown as typeof actions[number]);
  });
  const controllers: Array<Awaited<ReturnType<typeof control>>> = [];
  let sampling = false;
  let sampler: Promise<void> | undefined;
  let samplingFailure: unknown;
  try {
    const port = await new Promise<number>((done, reject) => {
      const timeout = setTimeout(() => reject(new Error(`legacy startup timed out: ${errors}`)), 30_000);
      child.once('exit', code => { clearTimeout(timeout); reject(new Error(`legacy child exited ${code}: ${errors}`)); });
      child.on('message', value => {
        const message = value as { type?: string; port?: number };
        if (message.type === 'ready' && message.port) { clearTimeout(timeout); done(message.port); }
      });
    });
    /** Read only scalar health after the production listener becomes available. */
    async function health(): Promise<Health> {
      const started = performance.now();
      const response = await fetch(`http://127.0.0.1:${port}/api/health`, { signal: AbortSignal.timeout(10_000) });
      const value = await response.json() as Health;
      healthLatencies.push(performance.now() - started);
      if (!response.ok || !value.ok || value.legacyConversion?.exactContinuation !== false) {
        throw new Error('legacy database conversion faulted or claimed exact continuation');
      }
      return value;
    }
    const afterStartup = await health();
    for (const kind of ['ui', 'bot'] as const) controllers.push(await control(port, kind, 0));
    sampling = true;
    sampler = (async () => {
      while (sampling) {
        try {
          const value = await health();
          if (value.archiveWork?.phaseTrace) traces.set(value.archiveWork.operationId, value.archiveWork);
        } catch (error) { samplingFailure = error; break; }
        await new Promise<void>(done => setTimeout(done, 10));
      }
    })();
    const exported = await fetch(`http://127.0.0.1:${port}/api/export/latest`, { signal: AbortSignal.timeout(180_000) });
    if (!exported.ok || !exported.body) throw new Error('converted database export failed');
    await exported.body.pipeTo(Writable.toWeb(createWriteStream(exportPath, { flags: 'wx' })));
    const exportBytes = (await stat(exportPath)).size;
    if (exportBytes !== Number(exported.headers.get('content-length'))) throw new Error('legacy export length mismatch');
    await new Promise<void>(done => setTimeout(done, 1000));
    const final = await health();
    if (final.archiveWork?.phaseTrace) traces.set(final.archiveWork.operationId, final.archiveWork);
    sampling = false;
    await sampler;
    if (samplingFailure) throw samplingFailure;
    const jobs = [...traces.values()];
    if (jobs.length !== 1 || !jobs[0]!.finished || !jobs[0]!.phaseTrace?.rssSamplerStarted ||
        jobs[0]!.phaseTrace.truncated) throw new Error('legacy export phase trace is incomplete');
    if (await digest(sourcePath) !== sourceSha256) throw new Error('original legacy database changed');
    for (const controller of controllers) controller.check();
    const actionLatencies = ['player', 'reinforcementLearning'].map(kind => {
      const values = actions.filter(action => action.kind === kind).map(action => action.durationMs).sort((a, b) => a - b);
      if (!values.length) throw new Error(`legacy export lacks ${kind} control samples`);
      return { kind, samples: values.length, p95Ms: values[Math.ceil(values.length * 0.95) - 1], maxMs: values.at(-1) };
    });
    healthLatencies.sort((a, b) => a - b);
    await writeFile(resolve(createdRoot, 'profile.json'), JSON.stringify({ sourceSha256,
      nativeSourceSha256: computeNativeSourceIdentity(resolve('native')).sha256,
      profilerSha256: await digest(fileURLToPath(import.meta.url)),
      host: { platform: process.platform, arch: process.arch, cpuModel: cpus()[0]?.model, totalMemoryBytes: totalmem() },
      sourceFormat: afterStartup.legacyConversion?.sourceFormat, afterStartup, final, boundaries, jobs,
      exportBytes, exportSha256: await digest(exportPath), actionLatencies,
      healthP95Ms: healthLatencies[Math.ceil(healthLatencies.length * 0.95) - 1],
      scope: 'Fresh production child converts a copied legacy database. Startup calls report whole-process RSS before/after and every ten milliseconds; legacy decode and initial candidate preparation are one native call. Managed publication/SQLite commit and candidate activation are separately bracketed. Ordinary export retains native two-millisecond stage observations. Live player/protocol-bot and health latency apply only after the listener opens, with a one-second response tail; no startup HTTP, browser, physical LAN or PyRL claim. Sampled peaks are lower bounds. Original source-file digest is unchanged.'
    }, null, 2) + '\n', { flag: 'wx' });
  } finally {
    sampling = false;
    await sampler;
    for (const controller of controllers) controller.close();
    try { await stop(child); }
    finally { await cleanup(outputRoot, createdRoot, databasePath, exportPath); }
  }
}

/** Dispatch the private child mode or require an explicit archive and absent output directory. */
async function main(): Promise<void> {
  const [first, second, third, fourth, ...extra] = process.argv.slice(2);
  if (first === '--child' && second && !third && process.send) { await childServer(resolve(second)); return; }
  if (first === '--child-legacy' && second && !third && process.send) { await childServer(resolve(second), false, 6, true); return; }
  if (first === '--database-path' && second && third === '--output-root' && fourth && !extra.length) {
    await profileLegacyDatabase(resolve(second), resolve(fourth));
    console.log('Legacy database profile completed; disposable server/database/export removed.');
    return;
  }
  if (first !== '--archive-path' || !second || third !== '--output-root' || !fourth ||
      (extra.length !== 0 && (extra.length !== 1 || extra[0] !== '--legacy-json'))) {
    throw new Error('usage: --archive-path EXISTING_SAVE_OVER_50_MIB --output-root NEW_DIRECTORY [--legacy-json]');
  }
  await profile(resolve(second), resolve(fourth), extra[0] === '--legacy-json');
  console.log('Archive phase profile completed; disposable server/database/export removed.');
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  void main().catch(error => { console.error(error); process.exitCode = 1; });
}
