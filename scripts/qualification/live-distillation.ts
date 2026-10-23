import { distillationSpoolLimitsForGiB } from '../../server/rustEngine/distillationProtocol.ts';
import { validateRustCalculationWorkers } from '../../server/rustWorkers.ts';
/** Qualify live committed Rust samples, HTTP, PyTorch restart and four Rust shadows. */
import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { createHash, randomBytes } from 'node:crypto';
import { createServer, request as httpRequest } from 'node:http';
import { mkdir, mkdtemp, readFile, rm, writeFile, rename, realpath } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve, relative, isAbsolute, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { existsSync } from 'node:fs';
import { loadPanicBinding } from '../../server/test/panicRuntime.ts';
import type { ExperimentalFreshRunNativeHandle } from '../../server/rustEngine/experimentalFreshRunSession.ts';
import type { ExperimentalRunningAuthorityNativeHandle } from '../../server/rustEngine/backgroundRuntime.ts';
import type { ExperimentalEngineInit } from '../../server/rustEngine/experimentalNativeBridge.ts';
import { CheckpointPersistenceClient } from '../../server/rustEngine/checkpointPersistenceClient.ts';
import type { ManagedCheckpointDescriptor } from '../../server/rustEngine/checkpointPersistenceProtocol.ts';
import { parseManagedCheckpointDescriptor } from '../../server/rustEngine/checkpointPersistenceProtocol.ts';
import { BackgroundCommandAdmission } from '../../server/rustEngine/commandAdmission.ts';
import { BackgroundGenerationRouter } from '../../server/rustEngine/backgroundGeneration.ts';
import { DistillationSpoolClient } from '../../server/rustEngine/distillationSpoolClient.ts';
import { DistillationCapture } from '../../server/rustEngine/distillationCapture.ts';
import { DistillationHttpRouter } from '../../server/rustEngine/distillationHttp.ts';
import { SavedBrainLibrary } from '../../server/rustEngine/savedBrainLibrary.ts';
import type { BrainArtifactDescriptor } from '../../src/protocol/brainArtifacts.ts';
import type { SavedBrainRecordV1, SavedBrainSpawnReceiptV1 } from '../../server/protocol.ts';
import type { RustBackgroundEvent } from '../../src/protocol/rustBackground.ts';
import { parseRustStartupMetadata } from '../../server/rustEngine/startupMetadata.ts';
import { sensorWallTiming } from './sensorWallTiming.ts';
import { sixHourNativeWork, summarizeSensorWall, type SensorWallRead, type SensorGenerationBarrier } from './sensorWallSummary.ts';
import { parseInspectorSnapshot } from '../../src/protocol/inspector.ts';
import { admitDiskOperation, checkpointDiskAdmissionRequest } from '../../server/rustEngine/diskAdmission.ts';

/** Explicit candidate-only constructor, absent from normal production startup. */
interface QualifiedSession extends ExperimentalFreshRunNativeHandle {
  /** Independently restore an exact qualification boundary with its declared environment. */
  initializeSensorCheckpointForTest(directory: string, descriptor: ManagedCheckpointDescriptor,
    admission: {profile: string; bins: number; mathBackend: string; rayMinimum: number; rayCap: number; rayBatch: number; envelopeMiB?: number}): Promise<unknown>;
  /** Admit a Motion/B32 capacity candidate without changing production or scratch limits. */
  initializeCapacityForTest(request: {population: number; graph: string; envelopeMiB: number; rawFallback: boolean;
    rayMinimum: number; rayCap: number; rayBatch: number}): Promise<{capacityAuditJson: string}>;
  /** Delegate all state construction to the real Rust qualification constructor. */
  initializeBaselineV2ForTest(request: {profile: string; bins: number; mode: number; cadence: number;
    rayMinimum: number; rayCap: number; rayBatch: number; workload?: string; trainedCohort?: string}): Promise<{trainedCohortJson?: string}>;
}
/** Metadata-only native capacity audit, returned before checkpoint publication. */
interface CapacityAudit {
  /** Complete evolved population admitted by Rust. */
  population: number;
  /** Exact graph parameter count per population owner. */
  parametersPerGenome: number;
  /** Current and successor owners both fit the process memory ceiling. */
  twoStateBudgetFits: boolean;
  /** All decoded checkpoint roles fit the selected candidate envelope. */
  decodedEnvelopeFits: boolean;
  /** Explicit native limit actually retained by the pending boundary. */
  limits: {maxArchiveBytes: number; maxTotalDecodedBytes: number};
  /** Per-worker scratch remains the production 512 MiB allowance. */
  workerScratchBytes: number;
  /** Checkpoint scratch remains the production 512 MiB allowance. */
  checkpointScratchBytes: number;
}
/** Production-sized bounded queues, without any external controller. */
const INIT: ExperimentalEngineInit = {contractVersion: 1, maxInboundBatches: 16, maxInboundCommands: 16,
  maxInboundOwnedBytes: 2 * 1024 ** 2, maxBatchCommands: 1, maxBatchOwnedBytes: 1024 ** 2,
  maxOutputReliable: 32, maxOutputReliableOwnedBytes: 16 * 1024 ** 2, maxOutputDiscrete: 4,
  maxOutputDiscreteOwnedBytes: 1024 ** 2, maxOutputTotalOwnedBytes: 32 * 1024 ** 2,
  maxOutputEventOwnedBytes: 1024 ** 2, maxOutputFrameConnections: 4};

/** Run only the explicitly supplied Python interpreter, bounded in output and wall time. */
async function python(pythonPath: string, checkout: string, args: string[], deadlineMs = 630_000, signal?: AbortSignal): Promise<void> {
  await new Promise<void>((done, reject) => {
    const child = spawn(pythonPath, ['-m', 'pyrl_trainer.live_distillation_gate', ...args], {cwd: checkout,
      windowsHide: true, stdio: ['ignore', 'pipe', 'pipe']});
    let output = ''; let failure: Error | undefined;
    /** Stop only this gate's process tree, including the Windows venv launcher. */
    const terminate = (error: Error): void => {
      if (failure) return; failure = error;
      if (process.platform === 'win32' && child.pid !== undefined) {
        try {execFileSync('taskkill', ['/PID', String(child.pid), '/T', '/F'], {windowsHide: true, stdio: 'ignore', timeout: 5000});}
        catch {child.kill();}
      } else child.kill('SIGKILL');
    };
    const abort = (): void => terminate(new Error('qualification Python process cancelled'));
    if (signal?.aborted) abort(); else signal?.addEventListener('abort', abort, {once: true});
    const timer = setTimeout(() => terminate(new Error('qualification Python process deadline')), deadlineMs);
    for (const stream of [child.stdout, child.stderr]) stream.on('data', (bytes: Buffer) => {
      output += bytes.toString();
      if (Buffer.byteLength(output) > 256 * 1024) terminate(new Error('qualification Python output admission'));
    });
    child.once('error', error => {clearTimeout(timer); signal?.removeEventListener('abort', abort); failure = error; reject(error);});
    child.once('close', code => {
      clearTimeout(timer); signal?.removeEventListener('abort', abort); if (failure) {reject(failure); return;}
      if (code !== 0) reject(new Error('Python qualification failed: ' + output.slice(-16_384)));
      else {console.info(output.trim()); done();}
    });
  });
}

/** Bounded JSON or opaque-byte response; every request stays inside the diagnostic namespace. */
async function exchange(port: number, path: string, method: string, body?: unknown, expectedStatus = 200): Promise<Buffer> {
  const bytes = body === undefined ? undefined : Buffer.from(JSON.stringify(body));
  return await new Promise((done, reject) => {
    const request = httpRequest({hostname: '127.0.0.1', port, path, method,
      headers: bytes ? {'Content-Type': 'application/json', 'Content-Length': bytes.byteLength} : {},
      signal: AbortSignal.timeout(35_000)}, response => {
      const chunks: Buffer[] = []; let length = 0;
      response.on('data', (part: Buffer) => {length += part.byteLength;
        if (length > 64 * 1024 ** 2) response.destroy(new Error('qualification response admission')); else chunks.push(part);});
      response.once('error', reject);
      response.once('end', () => {
        try {const result = Buffer.concat(chunks); assert.equal(response.statusCode, expectedStatus, result.toString('utf8').slice(0, 4096)); done(result);}
        catch (error) {reject(error);}
      });
    }); request.once('error', reject); request.end(bytes);
  });
}

/** POST one bounded metadata command through the real HTTP router. */
async function post(port: number, path: string, body: unknown, expectedStatus = 200): Promise<Record<string, unknown>> {
  return JSON.parse((await exchange(port, path, 'POST', body, expectedStatus)).toString('utf8')) as Record<string, unknown>;
}

/** Wait for an actual cold-operation/publication boundary with one liveness deadline. */
async function until(check: () => boolean, description: string): Promise<void> {
  const deadline = performance.now() + 5000;
  while (!check()) {if (performance.now() >= deadline) throw new Error(description); await new Promise(done => setTimeout(done, 10));}
}

/** Content identity for retained gate/source evidence. */
async function digest(file: string): Promise<string> {return createHash('sha256').update(await readFile(file)).digest('hex');}

/** Verify the exact task-owned temporary directory before recursive cleanup. */
async function removeRoot(root: string): Promise<void> {
  if (dirname(resolve(root)) !== resolve(tmpdir()) || !root.startsWith(join(tmpdir(), 'slither-live-distillation-'))) throw new Error('unsafe qualification cleanup');
  await rm(root, {recursive: true, force: true});
}

/** Read a coherent cumulative timing histogram without advancing native authority. */
async function wallRead(native: ExperimentalRunningAuthorityNativeHandle): Promise<SensorWallRead> {
  for (let attempt = 0; attempt < 10; attempt++) {
    const beforeMs = performance.now(); const health = native.health(true); const afterMs = performance.now();
    assert.equal(health.lifecycle, 'running'); assert.equal(health.faultCode, undefined);
    if (health.stepTimingHistogramConsistent) {
      const {worldEpoch, generation, completedStep, calculationWorkers, schedulerCompletedSteps,
        schedulerDroppedWallMicros, schedulerOverloaded, lifecycle, stepTimingSamples,
        stepTimingHistogramConsistent, stepTimingBucketUpperMicros, stepTimingBucketCounts} = health;
      assert(stepTimingBucketUpperMicros && stepTimingBucketCounts);
      return {beforeMs, afterMs, health: {worldEpoch, generation, completedStep, calculationWorkers,
        schedulerCompletedSteps, schedulerDroppedWallMicros, schedulerOverloaded, lifecycle, stepTimingSamples,
        stepTimingHistogramConsistent, stepTimingBucketUpperMicros, stepTimingBucketCounts}};
    }
    await new Promise(done => setTimeout(done, 5));
  }
  throw new Error('contention histogram remained incoherent');
}

/** Require explicit checkout/interpreter/report, so this gate cannot touch an owner database. */
async function run(): Promise<void> {
  const args = new Map<string, string>();
  for (let index = 2; index < process.argv.length; index += 2) {
    const key = process.argv[index]!; const value = process.argv[index + 1];
    if (!['--pyrl-root', '--python', '--device', '--workers', '--report', '--spool-gib', '--workload', '--student-artifact', '--contention-seconds', '--generation-boundaries', '--soak-hours', '--workspace', '--trained-cohort', '--replay-mib', '--capacity-graph', '--checkpoint-envelope-mib'].includes(key) || value === undefined || args.has(key)) throw new Error('invalid qualification arguments');
    args.set(key, value);
  }
  for (const key of ['--pyrl-root', '--python', '--device', '--workers', '--report']) assert(args.has(key), 'required ' + key);
  const checkout = resolve(args.get('--pyrl-root')!); const pythonPath = resolve(args.get('--python')!);
  const device = args.get('--device')!; const workers = validateRustCalculationWorkers(Number(args.get('--workers')));
  const spoolGiB = Number(args.get('--spool-gib') ?? '4');
  const replayMiB = Number(args.get('--replay-mib') ?? '256');
  assert(Number.isSafeInteger(replayMiB) && replayMiB >= 64 && replayMiB <= 2048, 'replay budget must be 64..2048 MiB');
  const spoolLimits = distillationSpoolLimitsForGiB(spoolGiB);
  assert(['cpu', 'cuda:0'].includes(device));
  const workload = args.get('--workload');
  assert(workload === undefined || workload === 'P0' || workload === 'P1');
  const capacityGraph = args.get('--capacity-graph');
  assert(capacityGraph === undefined || capacityGraph === 'modest' || capacityGraph === 'large');
  assert(capacityGraph === undefined || workload !== undefined, 'capacity graph requires P0/P1 workload');
  assert(!args.has('--checkpoint-envelope-mib') || capacityGraph !== undefined, 'candidate envelope requires explicit capacity graph');
  const envelopeMiB = Number(args.get('--checkpoint-envelope-mib') ?? '512');
  assert(envelopeMiB === 512 || envelopeMiB === 768, 'checkpoint envelope must be 512 or 768 MiB');
  const checkpointDiskRequest = checkpointDiskAdmissionRequest(BigInt(envelopeMiB) * 1024n ** 2n);
  const sixHour = args.has('--soak-hours');
  assert(!sixHour || args.get('--soak-hours') === '6');
  assert(sixHour === args.has('--workspace'), 'six-hour measurement requires an exclusive persistent workspace');
  const seconds = Number(args.get('--contention-seconds') ?? (sixHour ? '21600' : '60'));
  const timing = sensorWallTiming(seconds, sixHour);
  assert(sixHour || seconds <= 300);
  const boundaries = Number(args.get('--generation-boundaries') ?? (sixHour ? '72' : '0'));
  assert(Number.isSafeInteger(boundaries) && (boundaries === 0 ||
    (boundaries >= 2 && boundaries <= timing.generationBoundaryLimit && workload !== undefined)));
  assert(!sixHour || workload !== undefined && boundaries >= 2);
  assert(workload ? args.has('--student-artifact') : !args.has('--student-artifact') && !args.has('--contention-seconds'));
  const studentArtifact = resolve(args.get('--student-artifact') ?? 'native/fixtures/portable-brain-v1.slither-brain');
  const studentArtifactSha256 = await digest(studentArtifact);
  assert(!args.has('--trained-cohort') || workload !== undefined, 'trained cohort requires P0/P1 workload');
  const trainedCohort = args.has('--trained-cohort') ? resolve(args.get('--trained-cohort')!) : undefined;
  assert(!capacityGraph || !trainedCohort, 'capacity constructor requires fresh native weights');
  const cohortIndexSha256 = trainedCohort ? await digest(trainedCohort) : undefined;
  const output = resolve(args.get('--report')!); assert(!existsSync(output), 'qualification report already exists');
  const hostSha256 = await digest(fileURLToPath(import.meta.url));
  const trainerGateSha256 = await digest(join(checkout, 'pyrl_trainer/live_distillation_gate.py'));
  let root: string;
  if (sixHour) {
    root = resolve(args.get('--workspace')!);
    const parent = await realpath(dirname(root)); const temporary = await realpath(tmpdir());
    const fromTemporary = relative(temporary, parent);
    assert(isAbsolute(fromTemporary) || fromTemporary === '..' || fromTemporary.startsWith('..' + sep),
      'six-hour workspace must be outside disposable temporary storage');
    assert.equal(parent, dirname(root), 'workspace parent must resolve without symlinks');
    await mkdir(root); // Exclusive: an existing directory must never become this gate's workspace.
  } else root = await mkdtemp(join(tmpdir(), 'slither-live-distillation-'));
  let measured = false;
  const managed = join(root, 'managed'); await mkdir(managed);
  let runtime: ExperimentalRunningAuthorityNativeHandle | undefined;
  let persistence: CheckpointPersistenceClient | undefined; let capture: DistillationCapture | undefined;
  let library: SavedBrainLibrary | undefined;
  let spool: DistillationSpoolClient | undefined; let timer: NodeJS.Timeout | undefined;
  let activePump: Promise<void> | undefined; let pumpError: unknown; let hostPeakRssBytes = process.memoryUsage.rss();
  let router: DistillationHttpRouter | undefined;
  const routes = new Map<string, number>(); let forbiddenRequests = 0;
  const server = createServer((request, response) => {
    const url = new URL(request.url!, 'http://localhost');
    if (!url.pathname.startsWith('/api/distillation/v1/')) {forbiddenRequests++; response.writeHead(404); response.end(); return;}
    const route = request.method + ' ' + url.pathname.replace(/[0-9a-f]{16,64}/gu, ':id');
    assert(routes.size < 64 || routes.has(route)); routes.set(route, (routes.get(route) ?? 0) + 1);
    if (router) router.handle(request, response, url); else {response.writeHead(503); response.end();}
  });
  server.on('upgrade', (_request, socket) => {forbiddenRequests++; socket.destroy();});
  try {
    await admitDiskOperation(managed, checkpointDiskRequest);
    const binding = loadPanicBinding() as ReturnType<typeof loadPanicBinding> & {
      validateBrainArtifactFile(path: string, registerShadow?: boolean): string;
      nativeAddonSourceSha256(): string; nativeAddonBuildTarget(): string;
    };
    const Constructor = binding.ExperimentalStage6aFreshRunSession as new (run: string, seed: string, memory: string, workers: number) => QualifiedSession;
    const runId = 'live-distillation-' + randomBytes(8).toString('hex');
    const session = new Constructor(runId, '0000002a', '0000000100000000', workers);
    const request = workload
      ? {profile: 'motion', bins: capacityGraph ? 32 : 16, mode: 1, cadence: 1 / 60, rayMinimum: 1, rayCap: 512, rayBatch: 131072, workload}
      : {profile: 'basic', bins: 8, mode: 0, cadence: 0.06, rayMinimum: 1, rayCap: 256, rayBatch: 65536};
    const initialized: {trainedCohortJson?: string; capacityAuditJson?: string} = capacityGraph
      ? await session.initializeCapacityForTest({population: workload === 'P0' ? 55 : 300, graph: capacityGraph,
        envelopeMiB, rawFallback: false, rayMinimum: request.rayMinimum, rayCap: request.rayCap, rayBatch: request.rayBatch})
      : await session.initializeBaselineV2ForTest({...request, ...(trainedCohort ? {trainedCohort} : {})});
    const capacityAudit = initialized.capacityAuditJson ? JSON.parse(initialized.capacityAuditJson) as CapacityAudit : undefined;
    assert.equal(Boolean(capacityAudit), Boolean(capacityGraph));
    if (capacityAudit) {
      assert.equal(capacityAudit.population, workload === 'P0' ? 55 : 300);
      assert.equal(capacityAudit.decodedEnvelopeFits, true); assert.equal(capacityAudit.twoStateBudgetFits, true);
      assert.equal(capacityAudit.limits.maxArchiveBytes, envelopeMiB * 1024 ** 2);
      assert.equal(capacityAudit.limits.maxTotalDecodedBytes, envelopeMiB * 1024 ** 2);
      assert.equal(capacityAudit.workerScratchBytes, 512 * 1024 ** 2);
      assert.equal(capacityAudit.checkpointScratchBytes, 512 * 1024 ** 2);
      if (capacityGraph === 'large') assert.equal(capacityAudit.parametersPerGenome, 494306);
    }
    const cohort = initialized.trainedCohortJson ? JSON.parse(initialized.trainedCohortJson) as {
      schemaVersion: number; indexSha256: string; sourceReservationBytes: number; assignment: string;
      families: {artifact: BrainArtifactDescriptor; populationSlots: number}[];
    } : undefined;
    assert.equal(Boolean(cohort), Boolean(trainedCohort));
    if (cohort) {
      assert.equal(cohort.schemaVersion, 1); assert.equal(cohort.indexSha256, cohortIndexSha256);
      assert(cohort.families.length >= 2 && cohort.families.length <= 16);
      assert(cohort.sourceReservationBytes > 0 && cohort.sourceReservationBytes <= 512 * 1024 ** 2);
      assert.equal(cohort.families.reduce((sum, family) => sum + family.populationSlots, 0), workload === 'P0' ? 55 : 300);
    }
    const metadata = parseRustStartupMetadata(session.startupMetadata());
    if (workload) {
      assert.equal(metadata.parameterCount, capacityAudit?.parametersPerGenome ?? 25042); assert.equal(metadata.sensorVersion, 4);
      assert.equal(metadata.graphSpec.nodes.find(node => node.type === 'Input')?.outputSize, capacityGraph ? 504 : 264);
      const student = JSON.parse(binding.validateBrainArtifactFile(studentArtifact)) as BrainArtifactDescriptor;
      assert.equal(student.sensorSha256, metadata.sensorContract?.sha256, 'student artifact must match the exact native sensor contract');
      const settings = new Map(metadata.settings.map(entry => [entry.path, entry.value]));
      assert.equal(settings.get('snakeCount'), workload === 'P0' ? 55 : 300);
      assert.equal(settings.get('generationSeconds'), 604800); assert.equal(settings.get('simSpeed'), 1);
      assert.equal(settings.get('evolvedRespawn.enabled'), true); assert.equal(settings.get('brain.controlDt'), 1 / 60);
    }
    persistence = new CheckpointPersistenceClient({databasePath: join(root, 'game.sqlite'), managedRootPath: managed});
    const descriptor = parseManagedCheckpointDescriptor(await session.publishRunStartCheckpoint({managedDirectory: managed, operationId: randomBytes(16).toString('hex')}));
    await persistence.commit(descriptor); session.acknowledgeRunStartPersistence(descriptor);
    let latestNativeCheckpoint = descriptor;
    await session.activateRunningAuthority(); runtime = await session.createBackgroundRuntime(INIT, () => {}) as ExperimentalRunningAuthorityNativeHandle;
    const native = runtime; const admission = new BackgroundCommandAdmission(native);
    let configRevision = '0000000000000001';
    const receipts = new Map<string, (event: RustBackgroundEvent) => void>();
    /** Correlate only this gate's bounded cold commands; the ordinary pump still handles every event. */
    const command = (submit: (sequence: string) => void): Promise<RustBackgroundEvent> => new Promise((done, reject) => {
      let sequence: string | undefined;
      const timeout = setTimeout(() => {if (sequence) receipts.delete(sequence); reject(new Error('qualification native command deadline'));}, 5000);
      try {
        assert(admission.trySubmitControl(value => {submit(value); sequence = value;}), 'qualification command queue busy');
        assert(sequence); receipts.set(sequence, event => {clearTimeout(timeout); done(event);});
      } catch (error) {clearTimeout(timeout); reject(error);}
    });
    spool = new DistillationSpoolClient(join(root, 'diagnostics'), spoolLimits); const storage = spool;
    capture = new DistillationCapture(() => native, admission, storage, () => ({runId, seed: 42}));
    const generation = new BackgroundGenerationRouter({native, admission, persistence, managedDirectory: managed, maxAssignments: 1,
      send: () => {throw new Error('qualification must not assign controllers');},
      admitCheckpoint: async () => {await admitDiskOperation(managed, checkpointDiskRequest);}});
    library = new SavedBrainLibrary(persistence, storage, managed, () => native, admission,
      () => ({expectedRunId: runId, expectedConfigRevision: configRevision, expectedWorldEpoch: native.health().worldEpoch}),
      () => !generation.active, () => {});
    const savedLibrary = library;
    router = new DistillationHttpRouter(storage, capture, savedLibrary);
    // DLL images have separate immutable candidate caches. Independently validate
    // the worker's complete opaque file in the real fixture image before activation.
    const prepare = storage.prepareShadow.bind(storage);
    storage.prepareShadow = async id => {
      const record = await prepare(id);
      const file = join(root, 'diagnostics', 'candidates', id + '.slither-brain');
      assert.deepEqual(JSON.parse(binding.validateBrainArtifactFile(file, true)), record.descriptor);
      return record;
    };
    const diagnostics = capture;
    timer = setInterval(() => {
      hostPeakRssBytes = Math.max(hostPeakRssBytes, process.memoryUsage.rss());
      if (activePump || pumpError) return;
      activePump = (async () => {
        generation.flush();
        for (const event of native.drainOutputs(32, INIT.maxOutputEventOwnedBytes).events) {
          if (event.kind === 'fault') throw new Error(event.faultDetail ?? 'native fault');
          if (event.kind.startsWith('controller') || event.kind === 'externalObservations') throw new Error('unexpected controller traffic');
          savedLibrary.event(event); diagnostics.event(event);
          if (event.commandSequence) {const receive = receipts.get(event.commandSequence); receipts.delete(event.commandSequence); receive?.(event);}
          if (event.kind === 'generationCheckpointPublished') {
            latestNativeCheckpoint = parseManagedCheckpointDescriptor((event.checkpoint as {descriptor: unknown}).descriptor);
          }
          await generation.handle(event);
        }
      })().catch(error => {pumpError = error;}).finally(() => {activePump = undefined;});
    }, 5);
    native.start(); await new Promise<void>(done => server.listen(0, '127.0.0.1', done));
    const address = server.address(); assert(address && typeof address !== 'string');
    const dataset = await capture.start({expectedRunId: runId, expectedConfigRevision: '0000000000000001',
      baselineSlots: Array.from({length: 10}, (_, slot) => slot), pendingBytes: workload ? 16 * 1024 ** 2 : 1024 ** 2,
      queueChunks: workload ? 1024 : 32, queueBytes: workload ? 16 * 1024 ** 2 : 4 * 1024 ** 2});
    const origin = 'http://127.0.0.1:' + address.port;
    const producerStoragePolicy = JSON.parse((await exchange(address.port, '/api/distillation/v1/storage', 'GET')).toString('utf8')) as {maxBytes: string};
    assert.equal(BigInt('0x' + producerStoragePolicy.maxBytes), BigInt(spoolGiB) * 1024n ** 3n);
    const jobDirectory = join(root, 'job');
    const common = ['--origin', origin, '--capture-epoch', dataset.contract.captureEpoch,
      '--artifact', studentArtifact, '--directory', jobDirectory, '--device', device, '--spool-gib', String(spoolGiB), '--replay-mib', String(replayMiB),
      ];
    const trainingCommon = [...common, '--max-wall-seconds', workload ? '600' : '180'];
    const before = native.health();
    console.info('[live-distillation] training', device, 'workers=' + workers);
    await python(pythonPath, checkout, ['--phase', 'train', ...trainingCommon]);
    if (pumpError) throw pumpError;
    await python(pythonPath, checkout, ['--phase', 'resume', ...trainingCommon]);
    if (pumpError) throw pumpError;
    const train = JSON.parse(await readFile(join(jobDirectory, 'train-report.json'), 'utf8')) as Record<string, unknown>;
    const resume = JSON.parse(await readFile(join(jobDirectory, 'resume-report.json'), 'utf8')) as Record<string, unknown>;
    assert.equal(resume['updates'], 250); assert.equal(resume['restartExact'], true);
    for (const phase of [train, resume]) {
      assert.equal(phase['replayCapacityBytes'], replayMiB * 1024 ** 2);
      assert(Number(phase['replayBytes']) <= replayMiB * 1024 ** 2);
    }
    const identity = resume['candidateSha256']; assert.equal(typeof identity, 'string'); assert(/^[0-9a-f]{64}$/u.test(identity as string));
    const receipt = resume['receipt'] as {processEpoch: string; baselineSlot: number};
    for (let slot = 1; slot < 4; slot++) {
      const activated = await post(address.port, '/api/distillation/v1/shadows/' + slot + '/activate', {
        requestId: randomBytes(16).toString('hex'), expectedProcessEpoch: receipt.processEpoch, expectedActivationSequence: '0000000000000000',
        expectedRunId: runId, expectedConfigRevision: '0000000000000001', captureEpoch: dataset.contract.captureEpoch,
        baselineSlot: receipt.baselineSlot, artifactSha256: identity});
      assert.equal(activated['enabled'], true);
    }
    let metrics = await capture.shadows.metrics(); const limit = performance.now() + 10_000;
    while (metrics.slots.some(slot => !slot || !slot.enabled || BigInt('0x' + slot.totals.samples) === 0n) && performance.now() < limit) {
      if (pumpError) throw pumpError;
      await new Promise(done => setTimeout(done, 50)); metrics = await capture.shadows.metrics();
    }
    assert(metrics.slots.every(slot => slot?.enabled && slot.artifactSha256 === identity && BigInt('0x' + slot.totals.samples) > 0n));
    if (workload) {
      const shadowsBefore = metrics.slots;
      const captureBefore = await storage.dataset(dataset.contract.captureEpoch);
      const first = await wallRead(native); const reads = [first];
      const generationBarriers: SensorGenerationBarrier[] = [];
      const barrierIntervalMs = seconds * 1000 / (boundaries + 1);
      const frame = new Uint8Array(metadata.maximumFrameBytes);
      let frameSequence = '0'.repeat(16); let frameCopies = 0; let frameBytes = 0;
      let finished = false; let trainerError: unknown; let nativeCompletion: Record<string, unknown> | undefined;
      const trainingAbort = new AbortController();
      const stopTraining = (): void => trainingAbort.abort();
      process.once('SIGINT', stopTraining); process.once('SIGTERM', stopTraining);
      const training = python(pythonPath, checkout, ['--phase', 'sustain', ...common, '--seconds', String(seconds),
        '--max-wall-seconds', String(timing.maxWallSeconds), ...(sixHour ? ['--soak-hours', '6', '--wait-for-native-work'] : [])], timing.processDeadlineMs, trainingAbort.signal)
        .catch(error => {trainerError = error;}).finally(() => {finished = true;});
      let nextRead = first.afterMs + timing.readIntervalMs;
      let nextProgress = first.afterMs + 60_000;
      try {
        while (!finished) {
          if (pumpError) throw pumpError;
          const copied = native.copyLatestFrame(frame, frameSequence);
          assert.notEqual(copied.status, 'tooSmall');
          if (copied.status === 'copied') {
            assert(copied.display.sequence > frameSequence); frameSequence = copied.display.sequence;
            frameCopies++; frameBytes += copied.display.frameByteLength;
          }
          if (generationBarriers.length < boundaries &&
              performance.now() >= first.afterMs + barrierIntervalMs * (generationBarriers.length + 1)) {
            const source = await wallRead(native); const beforeReadIndex = reads.length; reads.push(source);
            const nextGeneration = (BigInt('0x' + source.health.generation) + 1n).toString(16).padStart(16, '0');
            const accepted = await command(sequence => native.submitFinishGeneration(sequence, randomBytes(16).toString('hex'), runId, source.health.generation));
            assert.equal(accepted.kind, 'finishGenerationAccepted'); assert(accepted.finishGenerationEffectiveStep);
            // A fresh deadline bounds the actual native publication, SQLite fsync and resume barrier.
            const barrierDeadline = performance.now() + 30_000;
            while (native.health().generation !== nextGeneration || generation.active) {
              if (pumpError) throw pumpError;
              assert(performance.now() < barrierDeadline, 'generation durability barrier deadline');
              await new Promise(done => setTimeout(done, 10));
            }
            const target = await wallRead(native); const afterReadIndex = reads.length; reads.push(target);
            const selected: ManagedCheckpointDescriptor | null = await persistence!.selectCurrent(runId); assert(selected);
            assert.deepEqual(selected, latestNativeCheckpoint, 'SQLite pointer must equal the actual native publication');
            const history = await persistence!.readBrowserHistory(runId);
            const terminal = history.find(row => BigInt(row.gen) === BigInt('0x' + source.health.generation));
            assert(terminal?.experiment, 'modern generation must durably retain its experiment sidecar');
            generationBarriers.push({beforeReadIndex, afterReadIndex, effectiveStep: accepted.finishGenerationEffectiveStep,
              checkpoint: {runId: selected.runId, boundaryKind: selected.boundaryKind, generation: selected.generation,
                completedStep: selected.completedStep, logicalRootSha256: selected.logicalRootSha256},
              terminal: {generation: source.health.generation, completedStep: terminal.experiment.completedStep,
                reason: terminal.experiment.reason, population: terminal.experiment.population, actualSeconds: terminal.experiment.actualSeconds}});
            console.info('[live-distillation] durable generation', nextGeneration);
          }
          if (performance.now() >= nextRead) {
            assert(reads.length < 4095); reads.push(await wallRead(native)); nextRead = performance.now() + timing.readIntervalMs;
          }
          if (sixHour && !nativeCompletion && reads.length > 1 && generationBarriers.length === boundaries) {
            const work = sixHourNativeWork(first, reads.at(-1)!);
            if (work.workComplete) {
              nativeCompletion = {schemaVersion: 1, captureEpoch: dataset.contract.captureEpoch,
                requiredSteps: work.requiredSteps, completedSteps: work.completedSteps};
              // Publish complete bytes atomically; the private trainer never observes a partial receipt.
              await writeFile(join(jobDirectory, 'native-work-complete.tmp'), JSON.stringify(nativeCompletion) + '\n', {flag: 'wx'});
              await rename(join(jobDirectory, 'native-work-complete.tmp'), join(jobDirectory, 'native-work-complete.json'));
            }
          }
          if (sixHour && performance.now() >= nextProgress) {
            const latest = reads.at(-1)!;
            await writeFile(join(root, 'live-progress.tmp'), JSON.stringify({schemaVersion: 1,
              status: 'running', hostPid: process.pid, activeWallSeconds: (performance.now() - first.afterMs) / 1000,
              health: latest.health, reads: reads.length, committedGenerations: generationBarriers.length,
              frameCopies, hostPeakRssBytes}) + '\n');
            await rename(join(root, 'live-progress.tmp'), join(root, 'live-progress.json'));
            nextProgress = performance.now() + 60_000;
          }
          await new Promise(done => setTimeout(done, 34));
        }
      } catch (error) {trainingAbort.abort(); throw error;}
      finally {
        await training;
        process.removeListener('SIGINT', stopTraining); process.removeListener('SIGTERM', stopTraining);
      }
      if (trainerError) throw trainerError;
      reads.push(await wallRead(native));
      assert.equal(generationBarriers.length, boundaries, 'all requested generations must commit during the trainer window');
      const summary = summarizeSensorWall(reads, seconds, boundaries ? {runId, barriers: generationBarriers} : undefined, sixHour);
      const sustain = JSON.parse(await readFile(join(jobDirectory, 'sustain-report.json'), 'utf8')) as Record<string, unknown>;
      if (sixHour) {
        assert(nativeCompletion, 'native six-hour completion must precede trainer exit');
        assert.deepEqual(sustain['nativeWorkCompletion'], nativeCompletion);
      }
      assert.equal(sustain['replayCapacityBytes'], replayMiB * 1024 ** 2);
      assert(Number(sustain['peakReplayBytes']) <= replayMiB * 1024 ** 2);
      assert.equal(sustain['requestedSeconds'], seconds); assert(Number(sustain['activeWallSeconds']) >= seconds);
      assert(Number(sustain['optimizerUpdates']) > 0 && Number(sustain['fetches']) > 0);
      assert(BigInt(String(sustain['finalCursor'])) > BigInt(String(sustain['initialCursor'])));
      metrics = await capture.shadows.metrics();
      const shadowProgress = metrics.slots.map((slot, index) => {
        const previous = shadowsBefore[index]; assert(slot?.enabled && previous?.enabled);
        assert.equal(slot.artifactSha256, identity); assert.equal(slot.activationSequence, previous.activationSequence);
        const samples = BigInt('0x' + slot.totals.samples) - BigInt('0x' + previous.totals.samples);
        assert(samples > 0n, 'every shadow must evaluate during the trainer window'); return samples.toString();
      });
      let captureAfter = await capture.stop(dataset.contract.captureEpoch);
      const settleDeadline = performance.now() + 10_000;
      while (captureAfter.state !== 'stopped' || captureAfter.resolvedThrough !== captureAfter.productionWatermark) {
        assert(performance.now() < settleDeadline, 'contention capture settlement deadline');
        await capture.poll(); await new Promise(done => setTimeout(done, 10));
        captureAfter = await storage.dataset(dataset.contract.captureEpoch);
      }
      assert.equal(captureAfter.error, null); assert(frameCopies > 0); assert.equal(forbiddenRequests, 0);
      const queueDiagnostics = native.queueDiagnostics();
      assert.equal(hostSha256, await digest(fileURLToPath(import.meta.url)));
      assert.equal(trainerGateSha256, await digest(join(checkout, 'pyrl_trainer/live_distillation_gate.py')));
      assert.equal(studentArtifactSha256, await digest(studentArtifact));
      if (trainedCohort) assert.equal(cohortIndexSha256, await digest(trainedCohort));
      if (pumpError) throw pumpError;
      let durability: {requestedBoundaries: number; barriers: SensorGenerationBarrier[]; persistenceRestartExact: boolean;
        restoredGeneration: string; restoredCompletedStep: string; restoredSourceSha256: string; invalidRestoreAdmissionsRejected: number} | undefined;
      if (boundaries) {
        // Stop only this gate's authority before restarting its metadata worker and validating a cold restore.
        clearInterval(timer); await activePump;
        native.requestStop(); await native.join();
        await library.close(); await persistence.close();
        persistence = new CheckpointPersistenceClient({databasePath: join(root, 'game.sqlite'), managedRootPath: managed});
        const recovered = await persistence.selectCurrent(runId); assert(recovered);
        assert.deepEqual(recovered, latestNativeCheckpoint);
        const recoveredHistory = await persistence.readBrowserHistory(runId);
        for (const barrier of generationBarriers) {
          const row = recoveredHistory.find(entry => BigInt(entry.gen) === BigInt('0x' + barrier.terminal.generation));
          assert(row?.experiment); assert.equal(row.experiment.completedStep, barrier.terminal.completedStep);
          assert.equal(row.experiment.reason, 'manual');
        }
        const restored = new Constructor(runId, '0000002a', '0000000100000000', workers);
        const restoreAdmission = {profile: request.profile, bins: request.bins, mathBackend: 'sse2',
          rayMinimum: request.rayMinimum, rayCap: request.rayCap, rayBatch: request.rayBatch,
          ...(capacityGraph ? {envelopeMiB} : {})};
        const rejectedAdmissions = [{bins: 7}, {bins: 16.5}, {mathBackend: 'js'}, {profile: 'v3'},
          {rayMinimum: Number.NaN}, {rayCap: 0}, {unexpected: true}];
        for (const invalid of rejectedAdmissions) {
          await assert.rejects(async () => restored.initializeSensorCheckpointForTest(managed, recovered,
            {...restoreAdmission, ...invalid}), /qualified restore/u);
        }
        const restoredSnapshot = await restored.initializeSensorCheckpointForTest(managed, recovered,
          restoreAdmission) as {generation: unknown; completedStep: unknown};
        assert.equal(restoredSnapshot.generation, recovered.generation);
        assert.equal(restoredSnapshot.completedStep, recovered.completedStep);
        const restoredMetadata = parseRustStartupMetadata(restored.startupMetadata());
        assert.equal(restoredMetadata.parameterCount, metadata.parameterCount);
        assert.equal(restoredMetadata.sensorVersion, 4);
        durability = {requestedBoundaries: boundaries, barriers: generationBarriers, persistenceRestartExact: true,
          restoredGeneration: restoredSnapshot.generation as string, restoredCompletedStep: restoredSnapshot.completedStep as string,
          restoredSourceSha256: binding.nativeAddonSourceSha256(), invalidRestoreAdmissionsRejected: rejectedAdmissions.length};
      }
      const contentionReport = {schemaVersion: 1, gate: sixHour ? 'live-trainer-six-hour-window-v1' : boundaries ? 'live-trainer-generation-durability-v1' : 'live-trainer-four-shadow-contention-v1',
        device, workers, seed: 42, seconds, ...(sixHour ? {sixHourWindow: true, persistentWorkspace: root} : {}), ...(durability ? {durability} : {}),
        fullCapacitySoakQualified: false,
        scope: 'Real background scheduler, seeded ' + (capacityGraph ?? 'modest') + ' population starting from ' + (cohort ? 'diverse Rust-admitted learned weights' : 'random weights') + ' with evolved respawn, ten stochastic v2 teachers, actual durable HTTP capture intake, one default TBPTT/Adam trainer and four identical learned native shadows. Window includes trainer process startup/restore/shutdown; trainer active duration is separately recorded. Optional manual generations include real native/SQLite handoffs and a cold boundary restore. Excludes crowded geometry, diverse mature policies, full library/spool capacity and production-default selection. A six-hour window is identified explicitly and is not a full-capacity integrated soak.',
        source: {slitherRevision: execFileSync('git', ['rev-parse', 'HEAD'], {encoding: 'utf8'}).trim(),
          pyrlRevision: execFileSync('git', ['rev-parse', 'HEAD'], {cwd: checkout, encoding: 'utf8'}).trim(),
          hostSha256, trainerGateSha256, studentArtifactSha256,
          nativeSourceSha256: binding.nativeAddonSourceSha256(), target: binding.nativeAddonBuildTarget()},
        workload: {scenario: workload, request, metadata, ...(cohort ? {trainedCohort: cohort} : {}),
          ...(capacityAudit ? {capacityGraph, checkpointEnvelopeMiB: envelopeMiB, capacityAudit} : {})}, spoolGiB, replayMiB, producerStoragePolicy,
        hostPeakRssBytes, train, resume: {...resume, candidatePath: undefined}, sustain, summary,
        reads: reads.map((read, index) => {
          const health = {...read.health};
          if (index > 0 && index < reads.length - 1) {delete health.stepTimingBucketUpperMicros; delete health.stepTimingBucketCounts;}
          return {...read, health};
        }), shadows: {before: shadowsBefore, after: metrics.slots, sampleProgress: shadowProgress},
        capture: {before: captureBefore, after: captureAfter,
          noDrops: [captureAfter.nativeDroppedChunks, captureAfter.nativeDroppedSamples, captureAfter.spoolDroppedChunks]
            .every(value => value === '0'.repeat(16))},
        frames: {copies: frameCopies, bytes: frameBytes, reusableBufferBytes: frame.byteLength},
        queueDiagnostics, http: {forbiddenRequests, routes: Object.fromEntries(routes)}};
      await mkdir(dirname(output), {recursive: true});
      await writeFile(output, JSON.stringify(contentionReport, null, 2) + '\n', {flag: 'wx'});
      measured = true; console.info('[live-distillation] contention measured', output); return;
    }
    // Train an active-graph-compatible student too. Differing architectures may
    // be retained, while the single-graph runtime must reject their deployment.
    const denseDirectory = join(root, 'dense-job');
    const denseCommon = ['--origin', origin, '--capture-epoch', dataset.contract.captureEpoch,
      '--artifact', resolve('native/fixtures/portable-dense-basic8-v1.slither-brain'), '--directory', denseDirectory, '--device', device, '--spool-gib', String(spoolGiB), '--replay-mib', String(replayMiB)];
    await python(pythonPath, checkout, ['--phase', 'train', ...denseCommon]);
    await python(pythonPath, checkout, ['--phase', 'resume', ...denseCommon]);
    if (pumpError) throw pumpError;
    const denseResume = JSON.parse(await readFile(join(denseDirectory, 'resume-report.json'), 'utf8')) as Record<string, unknown>;
    assert.equal(denseResume['updates'], 250); assert.equal(denseResume['restartExact'], true);
    assert.deepEqual(denseResume['producerStoragePolicy'], producerStoragePolicy);
    const denseId = denseResume['candidateSha256'] as string; assert(/^[0-9a-f]{64}$/u.test(denseId));
    const save = async (id: string, name: string): Promise<SavedBrainRecordV1> =>
      await post(address.port, '/api/distillation/v1/library/' + id + '/save', {name}, 201) as unknown as SavedBrainRecordV1;
    const gruRecord = await save(identity as string, 'live-trained GRU');
    const denseRecord = await save(denseId, 'live-trained compatible Dense');
    for (const record of [gruRecord, denseRecord]) {
      assert.equal(record.provenance.sourceKind, 'distilled');
      assert(record.provenance.details && typeof record.provenance.details === 'object');
      assert.equal((record.provenance.details as Record<string, unknown>)['updates'], '00000000000000fa');
    }
    assert.equal(gruRecord.artifact.parameterCount, 1962); assert.equal(denseRecord.artifact.parameterCount, 146);
    const exported = await exchange(address.port, '/api/distillation/v1/library/' + denseId + '/export', 'GET');
    assert.equal(createHash('sha256').update(exported).digest('hex'), denseId);
    assert.deepEqual(exported, await readFile(join(denseDirectory, 'candidate.slither-brain')));
    await until(() => !savedLibrary.busy, 'saved export lease did not settle');
    const settings = await command(sequence => native.submitLiveSettings(sequence, [{path: 'brain.controlDt', value: 0.06}]));
    assert.equal(settings.kind, 'liveSettingsApplied'); assert(settings.settingsConfigRevision);
    configRevision = settings.settingsConfigRevision;
    const spawnBody = (): object => {const status = savedLibrary.status(); return {...status.scope,
      requestId: randomBytes(16).toString('hex'), expectedProcessEpoch: status.processEpoch, expectedDeploymentSequence: status.deploymentSequence};};
    const rejected = await post(address.port, '/api/distillation/v1/library/' + identity + '/spawn', spawnBody(), 400);
    assert.match(String(rejected['error']), /graph/u);
    await until(() => !savedLibrary.busy, 'rejected spawn lease did not settle');
    const body = spawnBody();
    const spawned = await post(address.port, '/api/distillation/v1/library/' + denseId + '/spawn', body) as unknown as SavedBrainSpawnReceiptV1;
    assert.equal(spawned.brainId, denseId); assert(spawned.snakeId > 0);
    await until(() => !savedLibrary.busy, 'saved spawn lease did not settle');
    assert.deepEqual(await post(address.port, '/api/distillation/v1/library/' + denseId + '/spawn', body), spawned);
    const select = async (): Promise<NonNullable<RustBackgroundEvent['entitySelection']>> => {
      const selected = await command(sequence => native.submitEntitySelection(sequence, 'frame', spawned.snakeId, runId, native.health().generation));
      assert.equal(selected.kind, 'entitySelected'); assert(selected.entitySelection); return selected.entitySelection;
    };
    const selected = await select();
    const originCopy = {kind: 'saved-brain', brainId: denseId, weightsSha256: denseRecord.artifact.weightsSha256, sourceKind: 'distilled'};
    assert.deepEqual(selected.retainedOrigin, originCopy);
    const deleted = JSON.parse((await exchange(address.port, '/api/distillation/v1/library/' + denseId, 'DELETE')).toString('utf8')) as {deleted: boolean};
    assert.equal(deleted.deleted, true);
    await until(() => !existsSync(join(managed, denseRecord.artifactRelativeFilename)) && !existsSync(join(managed, denseRecord.weights.relativeFilename)), 'deleted catalog files remained referenced');
    assert.deepEqual((await select()).retainedOrigin, originCopy);
    const inspecting = await command(sequence => native.submitInspector(sequence, runId, selected.token, spawned.snakeId));
    assert.equal(inspecting.kind, 'inspectorChanged');
    let observed: ReturnType<typeof parseInspectorSnapshot> | undefined;
    await until(() => {
      const wire = native.latestInspector('0000000000000000'); if (!wire) return false;
      const value = parseInspectorSnapshot(JSON.parse(wire));
      if (value.subscriptionSequence !== inspecting.commandSequence || value.sample?.snakeId !== spawned.snakeId) return false;
      observed = value; return value.sample.neuralOutputs !== null;
    }, 'deployed trained brain produced no actual inference sample');
    assert.equal(observed!.sample!.raw.kind, 'retained-neural');
    assert(observed!.sample!.neuralOutputs!.every(Number.isFinite));
    assert.equal((await command(sequence => native.submitInspector(sequence, runId))).kind, 'inspectorChanged');
    const deployment = {trainedUpdates: denseResume['updates'], restartExact: denseResume['restartExact'],
      producerStoragePolicy: denseResume['producerStoragePolicy'],
      brainId: denseId, weightsSha256: denseRecord.artifact.weightsSha256, parameterCount: denseRecord.artifact.parameterCount,
      mismatchedGraphRejected: true, exportExact: true, retryReceiptExact: true, catalogDeleted: true, coldFilesReclaimed: true,
      retainedOriginAfterDeletion: (await select()).retainedOrigin, receipt: spawned,
      deliveredSampleStep: observed!.sample!.sampleStep, rawOutputs: observed!.sample!.neuralOutputs};
    const after = native.health(); assert.equal(after.faultCode, undefined); assert(BigInt('0x' + after.completedStep) > BigInt('0x' + before.completedStep));
    assert.equal(forbiddenRequests, 0); if (pumpError) throw pumpError;
    assert.deepEqual(train['producerStoragePolicy'], producerStoragePolicy);
    assert.deepEqual(resume['producerStoragePolicy'], producerStoragePolicy);
    delete resume['candidatePath'];
    const report = {schemaVersion: 1, gate: 'live-canonical-distillation-v2', device, workers, seed: 42,
      source: {slitherRevision: execFileSync('git', ['rev-parse', 'HEAD'], {encoding: 'utf8'}).trim(),
        pyrlRevision: execFileSync('git', ['rev-parse', 'HEAD'], {cwd: checkout, encoding: 'utf8'}).trim(),
        hostSha256, trainerGateSha256,
        nativeSourceSha256: binding.nativeAddonSourceSha256(), target: binding.nativeAddonBuildTarget()},
      workload: {population: 55, baselines: 10, profile: 'basic', bins: 8, populationGraph: 'Dense 72 to 2',
        studentGraph: 'GRU8 and Dense2', studentParameters: 1962, studentState: 8, cadenceSeconds: 0.06},
      spoolGiB, producerStoragePolicy, hostPeakRssBytes, train, resume, deployment, authority: {completedStepBefore: before.completedStep, completedStepAfter: after.completedStep,
        generationAfter: after.generation, stepTimingP95Micros: after.stepTimingP95Micros, stepTimingP99Micros: after.stepTimingP99Micros,
        schedulerDroppedWallMicros: after.schedulerDroppedWallMicros}, shadows: metrics.slots,
      http: {forbiddenRequests, routes: Object.fromEntries(routes)}};
    await mkdir(dirname(output), {recursive: true});
    await writeFile(output, JSON.stringify(report, null, 2) + '\n', {flag: 'wx'});
    console.info('[live-distillation] passed', output);
  } finally {
    clearInterval(timer); await activePump;
    server.closeAllConnections(); await new Promise<void>(done => server.close(() => done()));
    if (runtime) {runtime.requestStop(); await runtime.join();}
    await library?.close();
    if (capture) await capture.close(); else await spool?.close(); await persistence?.close();
    if (!sixHour) await removeRoot(root);
    else {
      await writeFile(join(root, 'host-completion.json'), JSON.stringify({schemaVersion: 1,
        status: measured ? 'measured' : 'failed', hostPid: process.pid, report: output}) + '\n', {flag: 'wx'});
      console.info('[live-distillation] persistent evidence workspace', root);
    }
  }
}

await run();
