/** Profile real archive operations in a separate production-server process. */
import { spawn, type ChildProcess } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createReadStream, createWriteStream, existsSync } from 'node:fs';
import { mkdir, realpath, rm, stat, statfs, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { Writable } from 'node:stream';
import { fileURLToPath } from 'node:url';
import { DEFAULT_CONFIG } from '../../server/config.ts';
import { startRustServer } from '../../server/rustServer.ts';
import { CheckpointPersistenceClient } from '../../server/rustEngine/checkpointPersistenceClient.ts';
import type { RustArchiveWorkProgress } from '../../server/rustEngine/backgroundRuntime.ts';

/** Small authoritative health projection; all memory belongs to the child server. */
interface Health {
  /** Authority status. */
  ok: boolean;
  /** Exact native provenance. */
  nativeBuildIdentifier: string;
  /** Current generation. */
  generation: string;
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
async function stop(child: ChildProcess): Promise<void> {
  if (child.exitCode !== null || child.signalCode !== null) return;
  const closed = new Promise<void>((done, reject) => {
    child.once('close', code => code === 0 ? done() : reject(new Error(`profile server exited ${code}`)));
  });
  const timeout = setTimeout(() => child.kill(), 10_000);
  try { child.send({ type: 'stop' }); await closed; }
  finally { clearTimeout(timeout); }
}

/** Observe actual metadata-worker requests without changing their arguments or results. */
async function childServer(databasePath: string): Promise<void> {
  const originalCommit = CheckpointPersistenceClient.prototype.commitImport;
  const originalAcquire = CheckpointPersistenceClient.prototype.acquireCurrentExportLease;
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
  const server = await startRustServer({ ...DEFAULT_CONFIG, host: '127.0.0.1', port: 0,
    resume: 'fresh', seed: 1511506142, dbPath: databasePath, rustCalculationWorkers: 6, logLevel: 'error' });
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
        process.off('message', received); reject(new Error('profile child exceeded ten minutes'));
      }, 600_000);
      process.on('message', received);
    });
  } finally {
    await server.close();
    if (process.connected) process.disconnect();
  }
}

/** Stream a file digest with bounded parent-process storage. */
async function digest(path: string): Promise<string> {
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
async function profile(archivePath: string, outputRoot: string): Promise<void> {
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
  const child = spawn(process.execPath, ['--import', 'tsx', fileURLToPath(import.meta.url), '--child', databasePath], {
    env: { ...process.env, SLITHER_TRACE_ARCHIVE_PHASES: '1' }, stdio: ['ignore', 'pipe', 'pipe', 'ipc']
  });
  let errors = '';
  const boundaries: Record<string, unknown>[] = [];
  child.stderr?.on('data', part => { errors = `${errors}${String(part)}`.slice(-8192); });
  child.stdout?.resume();
  child.on('message', value => {
    const message = value as Record<string, unknown>;
    if (message['type'] === 'boundary') boundaries.push(message);
  });
  let sampling = false;
  let sampler: Promise<void> | undefined;
  const samples: Sample[] = [];
  const traces = new Map<string, RustArchiveWorkProgress>();
  const healthLatencyMs: number[] = [];
  let samplingFailure: unknown;
  let baseline: Health | undefined;
  let final: Health | undefined;
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
    baseline = await health();
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
    const importedHealth = await health();
    if (importedHealth.archiveWork?.phaseTrace) traces.set(importedHealth.archiveWork.operationId, importedHealth.archiveWork);
    const exported = await fetch(`http://127.0.0.1:${port}/api/export/latest`, { signal: AbortSignal.timeout(180_000) });
    if (!exported.ok || !exported.body || exported.headers.get('x-slither-checkpoint-id') !== receipt.checkpointId) {
      throw new Error('profile export selected a different checkpoint');
    }
    await exported.body.pipeTo(Writable.toWeb(createWriteStream(exportPath, { flags: 'wx' })));
    if ((await stat(exportPath)).size !== Number(exported.headers.get('content-length'))) throw new Error('export length mismatch');
    final = await health();
    if (final.archiveWork?.phaseTrace) traces.set(final.archiveWork.operationId, final.archiveWork);
    sampling = false;
    await sampler;
    if (samplingFailure) throw samplingFailure;
    if (await digest(archivePath) !== originalSha256) throw new Error('source save changed');
    const jobs = [...traces.values()].map(job => ({ ...job, phases: job.phaseTrace!.intervals.map(interval => {
      if (!interval.finishedMicros) throw new Error('completed profile contains an open phase');
      const start = Number(BigInt(`0x${interval.startedMicros}`));
      const end = Number(BigInt(`0x${interval.finishedMicros}`));
      const matched = samples.filter(sample => sample.operationId === job.operationId && sample.elapsedMicros >= start && sample.elapsedMicros <= end);
      return { phase: interval.phase, durationMs: (end - start) / 1000, samples: matched.length,
        sampledPeakRssBytes: matched.length ? Math.max(...matched.map(sample => sample.rssBytes)) : null };
    }) }));
    if (jobs.length !== 2 || jobs.some(job => !job.finished)) throw new Error('both complete archive jobs were not observed');
    healthLatencyMs.sort((left, right) => left - right);
    await writeFile(resolve(createdRoot, 'profile.json'), JSON.stringify({ originalSha256, archiveBytes,
      receipt, exportedSaveRoot: exported.headers.get('x-slither-save-root'), exportSha256: await digest(exportPath),
      baseline, afterImport: importedHealth, final, jobs, boundaries,
      sampleCount: samples.length, requestedSampleIntervalMs: 10,
      healthP95Ms: healthLatencyMs[Math.ceil(healthLatencyMs.length * 0.95) - 1],
      healthMaxMs: Math.max(...healthLatencyMs),
      scope: 'Separate parent HTTP/file client and normal production child server. RSS comes from the child. Rust phases use its monotonic job clock; short phases without a matched periodic sample report null. Worker-request boundary memory includes IPC and FULL commit. This is preparatory sampled phase evidence, not archive I/O overhead accounting, all legacy readers, player latency or final A4 acceptance.' }, null, 2) + '\n', { flag: 'wx' });
  } catch (error) {
    console.error('Archive profiling operation failed:', error);
    throw error;
  } finally {
    sampling = false;
    await sampler;
    try { await stop(child); }
    finally { await cleanup(outputRoot, createdRoot, databasePath, exportPath); }
  }
}

/** Dispatch the private child mode or require an explicit archive and absent output directory. */
async function main(): Promise<void> {
  const [first, second, third, fourth, ...extra] = process.argv.slice(2);
  if (first === '--child' && second && !third && process.send) { await childServer(resolve(second)); return; }
  if (first !== '--archive-path' || !second || third !== '--output-root' || !fourth || extra.length) {
    throw new Error('usage: --archive-path EXISTING_SAVE_OVER_50_MIB --output-root NEW_DIRECTORY');
  }
  await profile(resolve(second), resolve(fourth));
  console.log('Archive phase profile completed; disposable server/database/export removed.');
}

void main().catch(error => { console.error(error); process.exitCode = 1; });
