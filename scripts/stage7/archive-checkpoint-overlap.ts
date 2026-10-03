/** Measure actual P2 export-source assembly against unchanged FULL generation barriers. */
import { spawn } from 'node:child_process';
import { createWriteStream, existsSync } from 'node:fs';
import { mkdir, realpath, rm, stat, statfs, writeFile } from 'node:fs/promises';
import { cpus, loadavg, totalmem } from 'node:os';
import { basename, dirname, resolve } from 'node:path';
import { Writable } from 'node:stream';
import { fileURLToPath } from 'node:url';
import type { RustArchiveWorkProgress } from '../../server/rustEngine/backgroundRuntime.ts';
import { DEFAULT_CONFIG } from '../../server/config.ts';
import { ARCHIVE_TEMP_QUOTA_BYTES, OPERATING_DISK_RESERVE_BYTES, SQLITE_WAL_ALLOWANCE_BYTES } from '../../server/rustEngine/diskAdmission.ts';
import { computeNativeSourceIdentity } from '../../server/rustEngine/nativeSourceIdentity.ts';
import { childServer, control, digest, stop } from './archive-phase-profile.ts';
import { archiveOverlap } from './archive-overlap-summary.ts';
import { configure } from './realtime-workload.ts';

/** Bounded scalar child observations; no population or file body crosses IPC. */
type Observation =
  | { type: 'checkpoint-start'; atMs: number }
  | { type: 'checkpoint-barrier'; startedMs: number; finishedMs: number; durationMs: number;
    startClock: { beforeMs: number; afterMs: number }; finishClock: { beforeMs: number; afterMs: number } }
  | { type: 'checkpoint-commit'; atMs: number; requestMs: number; generation: string;
    checkpointId: string; operationId: string; runId: string }
  | { type: 'export-clock'; operationId: string; beforeMs: number; afterMs: number;
    generation: string; checkpointId: string }
  | { type: 'boundary'; phase: string; bodyBytes?: number; durationMs: number }
  | { type: 'action'; phase: string; kind: string; durationMs: number };

/** Required actual production health fields for workload and progress checks. */
interface Health {
  /** Honest authority status. */
  ok: boolean;
  /** Diagnostic if the authority has stopped. */
  interfaceFault?: string;
  /** Exact native build provenance. */
  nativeBuildIdentifier: string;
  /** Exact current generation. */
  generation: string;
  /** Actual completed steps. */
  completedStep: string;
  /** Discarded scheduler time. */
  schedulerDroppedWallMicros: string;
  /** Current scheduler overload status. */
  schedulerOverloaded: boolean;
  /** Current or last archive trace, from the actual native job. */
  archiveWork: RustArchiveWorkProgress | null;
  /** Actual step, process and durability observations. */
  telemetry: { process: { rssBytes: number; maxRssBytes: number; eventLoopDelayP95Ms: number;
    eventLoopDelayP99Ms: number; eventLoopDelayMaxMs: number };
    step: { p95Ms: number; p99Ms: number; maxMs: number };
    checkpointBarrier: { samples: number; p95Ms: number; maxMs: number } };
}

/** One attempt retains misses so the report cannot silently select favorable timings. */
interface Attempt {
  /** Actual child transition event that prompted the request. */
  transitionStartedMs: number;
  /** Complete original attachment's bytes and independently streamed digest. */
  archive?: { bytes: number; sha256: string; checkpointId: string; saveRoot: string };
  /** Actual native job origin bracket in the same clock as the checkpoint barrier. */
  clock?: Extract<Observation, { type: 'export-clock' }>;
  /** Complete actual native stage trace. */
  job?: RustArchiveWorkProgress;
  /** Full transition through publication, FULL commit, delivery and running successor. */
  barrier?: Extract<Observation, { type: 'checkpoint-barrier' }>;
  /** Actual durable metadata commit associated with that transition. */
  commit?: Extract<Observation, { type: 'checkpoint-commit' }>;
  /** Conservative overlap bounds; possible overlap alone does not pass. */
  overlap?: ReturnType<typeof archiveOverlap>;
  /** Unambiguous success or a retained timing miss. */
  proven: boolean;
}

/** Yield without moving any authoritative game clock. */
function pause(milliseconds: number): Promise<void> { return new Promise(done => setTimeout(done, milliseconds)); }

/** Use the empirical upper percentile, including the maximum for small samples. */
function percentile(values: readonly number[], fraction: number): number {
  if (!values.length) throw new Error('missing measured latency samples');
  const sorted = [...values].sort((left, right) => left - right);
  return sorted[Math.ceil(sorted.length * fraction) - 1]!;
}

/** Remove only the verified new scratch root after the exact server has stopped. */
async function cleanup(outputRoot: string, createdRoot: string): Promise<void> {
  if (await realpath(outputRoot) !== createdRoot || dirname(createdRoot) !== await realpath(resolve('data'))) {
    throw new Error('overlap cleanup parent changed');
  }
  await rm(createdRoot, { recursive: true });
}

/** Require a new invocation-owned scratch directory and separate permanent report. */
async function measure(outputRoot: string, reportPath: string, requestedSamples: number, rustWorkers: number): Promise<void> {
  if (dirname(outputRoot) !== resolve('data') || !basename(outputRoot).startsWith('codex-browser-fixture-overlap-') ||
      existsSync(outputRoot) || existsSync(reportPath) || reportPath.startsWith(`${outputRoot}/`) ||
      reportPath.startsWith(`${outputRoot}\\`)) throw new Error('require absent task-owned scratch and separate new report');
  await mkdir(dirname(outputRoot), { recursive: true });
  const filesystem = await statfs(dirname(outputRoot), { bigint: true });
  if (filesystem.bavail * filesystem.bsize < 12n * 1024n ** 3n) throw new Error('overlap run requires 12 GiB free');
  await mkdir(outputRoot);
  const createdRoot = await realpath(outputRoot);
  const databasePath = resolve(createdRoot, 'overlap.db');
  const exportPath = resolve(createdRoot, 'export.slither-save');
  const child = spawn(process.execPath, ['--import', 'tsx', fileURLToPath(import.meta.url), '--child', databasePath, String(rustWorkers)], {
    env: { ...process.env, SLITHER_TRACE_ARCHIVE_PHASES: '1' }, stdio: ['ignore', 'pipe', 'pipe', 'ipc']
  });
  /** Keep only the last bounded diagnostics if this exact child fails. */
  let childErrors = '';
  /** Record a failure without abandoning an in-flight export or starting another server. */
  let failure: unknown;
  /** Measurement starts only after evolved generation two and both controller assignments. */
  let measuring = false;
  /** At most one actual HTTP export is in flight. */
  let active: Promise<void> | undefined;
  /** Actual received child journal, capped independently of the health samples. */
  const journal: Observation[] = [];
  /** Preserve every triggered attempt, including timing misses. */
  const attempts: Attempt[] = [];
  /** Latest complete trace for each actual export operation. */
  const traces = new Map<string, RustArchiveWorkProgress>();
  /** Local health request latencies throughout the measured window. */
  const healthLatencies: number[] = [];
  /** Bounded actual player and protocol-bot peers. */
  const controllers: Awaited<ReturnType<typeof control>>[] = [];
  /** One readiness exchange settles on child errors, exit or a hard deadline too. */
  const ready = Promise.withResolvers<number>();
  void ready.promise.catch(() => {});
  const readyTimeout = setTimeout(() => ready.reject(new Error('overlap server startup timed out')), 120_000);
  /** Last actual health response, shared with the sampler. */
  let latest: Health | undefined;
  /** Stop the read-only sampler after the last pending request has settled. */
  let sampling = true;
  /** One sampler handle; it is always joined before child shutdown. */
  let sampler: Promise<void> | undefined;
  /** Actual loopback port received from this exact production child. */
  let port = 0;
  /** Bounded end of the measured interval after warm-up. */
  let deadline = 0;
  child.stderr?.on('data', part => { childErrors = `${childErrors}${String(part)}`.slice(-8192); });
  child.stdout?.resume();
  child.on('error', error => { failure = error; ready.reject(error); });
  child.on('exit', (code, signal) => {
    if (sampling) {
      failure = new Error(`overlap server exited ${code}/${signal}: ${childErrors}`);
      ready.reject(failure);
    }
  });
  /** Fetch bounded honest health without changing scheduling or checkpoint work. */
  async function health(): Promise<Health> {
    const started = performance.now();
    const response = await fetch(`http://127.0.0.1:${port}/api/health`, { signal: AbortSignal.timeout(5000) });
    const value = await response.json() as Health;
    if (!response.ok || !value.ok) throw new Error(`overlap authority fault: ${value.interfaceFault ?? response.status}`);
    if (measuring) {
      healthLatencies.push(performance.now() - started);
      if (healthLatencies.length > 60_000) throw new Error('health sample cap reached');
    }
    if (value.archiveWork?.phaseTrace) {
      if (value.archiveWork.phaseTrace.truncated) throw new Error('native phase trace was truncated');
      traces.set(value.archiveWork.operationId, value.archiveWork);
    }
    latest = value;
    return value;
  }
  /** Wait on real observations with a short read-only interval and fixed deadline. */
  async function until<T>(read: () => T | undefined, label: string): Promise<T> {
    for (;;) {
      if (failure) throw failure;
      const value = read();
      if (value !== undefined) return value;
      if (performance.now() >= deadline) throw new Error(`overlap measurement timed out: ${label}`);
      await pause(10);
    }
  }
  /** Download the original response while the server runs its normal transition. */
  async function exportAt(transitionStartedMs: number): Promise<void> {
    const attempt: Attempt = { transitionStartedMs, proven: false };
    attempts.push(attempt);
    const response = await fetch(`http://127.0.0.1:${port}/api/export/latest`, { signal: AbortSignal.timeout(180_000) });
    if (!response.ok || !response.body) throw new Error(`overlap download rejected ${response.status}: ${(await response.text()).slice(0, 1024)}`);
    await response.body.pipeTo(Writable.toWeb(createWriteStream(exportPath, { flags: 'wx' })));
    const bytes = (await stat(exportPath)).size;
    const checkpointId = response.headers.get('x-slither-checkpoint-id') ?? '';
    const saveRoot = response.headers.get('x-slither-save-root') ?? '';
    if (bytes <= 50 * 1024 ** 2 || bytes !== Number(response.headers.get('content-length')) ||
        !/^[a-f0-9]{64}$/u.test(checkpointId) || !/^[a-f0-9]{64}$/u.test(saveRoot)) {
      throw new Error('overlap attachment is not a complete original large save');
    }
    attempt.archive = { bytes, checkpointId, saveRoot, sha256: await digest(exportPath) };
    attempt.clock = await until(() => journal.find((item): item is NonNullable<Attempt['clock']> =>
      item.type === 'export-clock' && item.checkpointId === checkpointId &&
      item.beforeMs >= transitionStartedMs), 'actual export clock');
    attempt.job = await until(() => {
      const job = traces.get(attempt.clock!.operationId);
      return job?.finished ? job : undefined;
    }, 'complete native export trace');
    attempt.barrier = await until(() => journal.find((item): item is NonNullable<Attempt['barrier']> =>
      item.type === 'checkpoint-barrier' && item.startedMs >= transitionStartedMs &&
      item.startedMs - transitionStartedMs < 100), 'actual full checkpoint barrier');
    attempt.commit = await until(() => journal.find((item): item is NonNullable<Attempt['commit']> =>
      item.type === 'checkpoint-commit' && item.atMs >= attempt.barrier!.startedMs &&
      item.atMs <= attempt.barrier!.finishedMs), 'actual FULL metadata commit');
    const phases = attempt.job.phaseTrace!.intervals.filter(phase => phase.phase === 'export-source-population');
    if (phases.length !== 1 || attempt.job.kind !== 'export' || !attempt.job.started) throw new Error('missing exact source-assembly stage');
    attempt.overlap = archiveOverlap(attempt.clock, phases[0]!, attempt.barrier);
    // A lease selected after the successor committed is a timing miss too.
    attempt.proven = attempt.overlap.guaranteedMs > 0 &&
      BigInt(`0x${attempt.commit.generation}`) === BigInt(`0x${attempt.clock.generation}`) + 1n;
    await pause(1000); // Include late applications of input received before response completion.
    await rm(exportPath);
    console.log(`attempt=${attempts.length} sourceGeneration=${BigInt(`0x${attempt.clock.generation}`)} guaranteedOverlap=${attempt.overlap.guaranteedMs.toFixed(2)}ms fullBarrier=${attempt.barrier.durationMs.toFixed(2)}ms proven=${attempt.proven}`);
  }
  child.on('message', value => {
    const message = value as Observation | { type: 'ready'; port: number };
    if (message.type === 'ready') { ready.resolve(message.port); return; }
    if (!measuring) return;
    if (message.type !== 'action' || message.phase === 'export-download') {
      if (journal.length >= 60_000) failure = new Error('child observation cap reached');
      else journal.push(message);
    }
    if (message.type === 'checkpoint-start' && !active && !failure &&
        attempts.filter(attempt => attempt.proven).length < requestedSamples && attempts.length < requestedSamples + 2) {
      active = exportAt(message.atMs).catch(error => { failure = error; }).finally(() => { active = undefined; });
    }
  });
  try {
    port = await ready.promise;
    clearTimeout(readyTimeout);
    await configure(port, 'P2', rustWorkers);
    deadline = performance.now() + 600_000;
    let nextWarmProgress = performance.now() + 30_000;
    for (;;) {
      const value = await health();
      if (BigInt(`0x${value.generation}`) >= 2n) break;
      if (performance.now() >= deadline) throw new Error('P2 generation one did not complete');
      if (performance.now() >= nextWarmProgress) {
        console.log(`warming generation=${BigInt(`0x${value.generation}`)} step=${BigInt(`0x${value.completedStep}`)}`);
        nextWarmProgress += 30_000;
      }
      await pause(250);
    }
    for (const kind of ['ui', 'bot'] as const) controllers.push(await control(port, kind, 0));
    const initial = await health();
    const started = performance.now();
    deadline = started + (requestedSamples + 3) * 90_000;
    measuring = true;
    sampler = (async () => {
      while (sampling) {
        try { await health(); }
        catch (error) { failure = error; break; }
        await pause(active ? 10 : 250);
      }
    })();
    let nextProgress = started + 30_000;
    while (attempts.filter(attempt => attempt.proven).length < requestedSamples || active) {
      if (failure) throw failure;
      if (performance.now() >= deadline || (attempts.length >= requestedSamples + 2 && !active)) {
        throw new Error('bounded attempts did not prove the requested actual overlaps');
      }
      if (performance.now() >= nextProgress) {
        console.log(`elapsed=${Math.round((performance.now() - started) / 1000)}s generation=${BigInt(`0x${latest!.generation}`)} proven=${attempts.filter(attempt => attempt.proven).length}/${requestedSamples}`);
        nextProgress += 30_000;
      }
      await pause(100);
    }
    const final = await health();
    measuring = false;
    sampling = false;
    await sampler;
    if (failure) throw failure;
    for (const controller of controllers) {
      controller.check();
      if (controller.report.assignments < attempts.length + 1) throw new Error('controller was not reassigned across every measured generation');
    }
    const durations = attempts.filter(attempt => attempt.proven).map(attempt => attempt.barrier!.durationMs);
    const actionLatencies = ['player', 'reinforcementLearning'].map(kind => {
      const values = journal.filter((item): item is Extract<Observation, { type: 'action' }> =>
        item.type === 'action' && item.kind === kind).map(item => item.durationMs);
      return { kind, samples: values.length, p95Ms: percentile(values, 0.95), maxMs: Math.max(...values) };
    });
    const downloads = journal.filter(item => item.type === 'boundary' && item.phase === 'http-file-download');
    if (downloads.length !== attempts.length || downloads.some((item, index) =>
      item.type !== 'boundary' || item.bodyBytes !== attempts[index]!.archive!.bytes)) throw new Error('actual download bytes were not observed exactly once');
    const wallSeconds = (performance.now() - started) / 1000;
    const barrierP95Ms = percentile(durations, 0.95);
    const barrierMaxMs = Math.max(...durations);
    const healthP95Ms = percentile(healthLatencies, 0.95);
    const finalFilesystem = await statfs(createdRoot, { bigint: true });
    await mkdir(dirname(reportPath), { recursive: true });
    await writeFile(reportPath, JSON.stringify({ requestedSamples, attempts,
      host: { platform: process.platform, arch: process.arch, cpuModel: cpus()[0]?.model,
        logicalCpuCount: cpus().length, totalMemoryBytes: totalmem(), loadAverageAtEnd: loadavg() },
      filesystem: { availableBytesBefore: (filesystem.bavail * filesystem.bsize).toString(),
        availableBytesAfterMeasurement: (finalFilesystem.bavail * finalFilesystem.bsize).toString() },
      diskPolicy: { operatingReserveBytes: OPERATING_DISK_RESERVE_BYTES.toString(),
        archiveTempQuotaBytes: ARCHIVE_TEMP_QUOTA_BYTES.toString(), sqliteWalAllowanceBytes: SQLITE_WAL_ALLOWANCE_BYTES.toString(),
        checkpointBudgetMiB: DEFAULT_CONFIG.checkpointBudgetMiB },
      nativeSourceSha256: computeNativeSourceIdentity(resolve('native')).sha256,
      runnerSha256: await digest(fileURLToPath(import.meta.url)),
      observerSha256: await digest(resolve('scripts/stage7/archive-phase-profile.ts')),
      configuredWorkload: { scenario: 'P2', neuralSnakes: 55, baselineBots: 10, pellets: 3500,
        sensorCount: 147, generationSeconds: 60, simSpeed: 1, rustWorkers },
      initial: { ...initial, archiveWork: undefined }, final: { ...final, archiveWork: undefined },
      wallSeconds, deltaSteps: (BigInt(`0x${final.completedStep}`) - BigInt(`0x${initial.completedStep}`)).toString(),
      droppedWallMicros: (BigInt(`0x${final.schedulerDroppedWallMicros}`) - BigInt(`0x${initial.schedulerDroppedWallMicros}`)).toString(),
      barrierP95Ms, barrierMaxMs, healthSamples: healthLatencies.length, healthP95Ms,
      healthMaxMs: Math.max(...healthLatencies), controls: controllers.map(controller => controller.report),
      actionLatencies, journal: journal.filter(item => item.type !== 'action'),
      meetsMeasuredBudgets: barrierP95Ms <= 1000 && barrierMaxMs <= 2000 && healthP95Ms <= 100 &&
        actionLatencies.every(item => item.p95Ms <= 100) && final.telemetry.process.eventLoopDelayP95Ms <= 20 &&
        final.telemetry.process.eventLoopDelayP99Ms <= 50,
      scope: 'Actual evolved P2 production game at 1x on the listed host. Downloads are triggered from observed generation-transition events; no native operation, scheduler, disk admission, publication, FULL/WAL metadata transaction, retention, delivery or resume is delayed or replaced. Export clock brackets bound the native archive origin; the overlap window lies inside conservative bounds on the actual router barrier start/finish. Guaranteed overlap is valid for every permitted clock origin. Barrier duration is separately the original complete production telemetry duration. All attempts are retained. Local WebSocket input uses independent 30-Hz player and observation-driven protocol-bot peers; actual server-receipt-to-Rust-application latency is attributed to the response receive window with a one-second tail. This does not establish physical browser/LAN, PyRL training, isolated archive overhead, legacy-reader coverage or complete A4 acceptance.' }, null, 2) + '\n', { flag: 'wx' });
    console.log(`report=${reportPath} barrierP95=${barrierP95Ms.toFixed(2)}ms barrierMax=${barrierMaxMs.toFixed(2)}ms healthP95=${healthP95Ms.toFixed(2)}ms`);
  } catch (error) {
    measuring = false;
    await active;
    sampling = false;
    await sampler;
    await mkdir(dirname(reportPath), { recursive: true });
    if (!existsSync(reportPath)) await writeFile(reportPath, JSON.stringify({ status: 'failed',
      error: error instanceof Error ? error.message : String(error), requestedSamples, rustWorkers, attempts,
      latest: latest ? { ...latest, archiveWork: undefined } : null,
      journal: journal.filter(item => item.type !== 'action'),
      controls: controllers.map(controller => controller.report), childErrors,
      nativeSourceSha256: computeNativeSourceIdentity(resolve('native')).sha256,
      runnerSha256: await digest(fileURLToPath(import.meta.url)) }, null, 2) + '\n', { flag: 'wx' });
    throw error;
  } finally {
    clearTimeout(readyTimeout);
    measuring = false;
    sampling = false;
    await active;
    await sampler;
    for (const controller of controllers) controller.close();
    await stop(child);
    await cleanup(outputRoot, createdRoot);
  }
}

/** Dispatch the bounded isolated child or require explicit absent destinations. */
async function main(): Promise<void> {
  const args = process.argv.slice(2);
  if (args[0] === '--child' && args[1] && args[2] && /^[456]$/u.test(args[2]) && args.length === 3 && process.send) {
    await childServer(resolve(args[1]), true, Number(args[2])); return;
  }
  const values = new Map<string, string>();
  for (let index = 0; index < args.length; index += 2) {
    const key = args[index]; const value = args[index + 1];
    if (!key || !value || !['--output-root', '--report-path', '--samples', '--rust-workers'].includes(key) || values.has(key)) throw new Error('invalid overlap options');
    values.set(key, value);
  }
  const root = values.get('--output-root'); const report = values.get('--report-path');
  const count = values.get('--samples') ?? '8';
  const workers = values.get('--rust-workers') ?? '6';
  if (!root || !report || !/^(?:[3-9]|10)$/u.test(count) || !/^[456]$/u.test(workers)) {
    throw new Error('require --output-root NEW_DATA_FIXTURE --report-path NEW_REPORT [--samples 3..10] [--rust-workers 4|5|6]');
  }
  await measure(resolve(root), resolve(report), Number(count), Number(workers));
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  void main().catch(error => { console.error(error); process.exitCode = 1; });
}
