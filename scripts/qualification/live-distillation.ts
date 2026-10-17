import { distillationSpoolLimitsForGiB } from '../../server/rustEngine/distillationProtocol.ts';
/** Qualify live committed Rust samples, HTTP, PyTorch restart and four Rust shadows. */
import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { createHash, randomBytes } from 'node:crypto';
import { createServer, request as httpRequest } from 'node:http';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { existsSync } from 'node:fs';
import { loadPanicBinding } from '../../server/test/panicRuntime.ts';
import type { ExperimentalFreshRunNativeHandle } from '../../server/rustEngine/experimentalFreshRunSession.ts';
import type { ExperimentalRunningAuthorityNativeHandle } from '../../server/rustEngine/backgroundRuntime.ts';
import type { ExperimentalEngineInit } from '../../server/rustEngine/experimentalNativeBridge.ts';
import { CheckpointPersistenceClient } from '../../server/rustEngine/checkpointPersistenceClient.ts';
import { parseManagedCheckpointDescriptor } from '../../server/rustEngine/checkpointPersistenceProtocol.ts';
import { BackgroundCommandAdmission } from '../../server/rustEngine/commandAdmission.ts';
import { BackgroundGenerationRouter } from '../../server/rustEngine/backgroundGeneration.ts';
import { DistillationSpoolClient } from '../../server/rustEngine/distillationSpoolClient.ts';
import { DistillationCapture } from '../../server/rustEngine/distillationCapture.ts';
import { DistillationHttpRouter } from '../../server/rustEngine/distillationHttp.ts';
import { SavedBrainLibrary } from '../../server/rustEngine/savedBrainLibrary.ts';
import type { SavedBrainRecordV1, SavedBrainSpawnReceiptV1 } from '../../server/protocol.ts';
import type { RustBackgroundEvent } from '../../src/protocol/rustBackground.ts';
import { parseInspectorSnapshot } from '../../src/protocol/inspector.ts';
import { admitDiskOperation, CHECKPOINT_DISK_ADMISSION_REQUEST } from '../../server/rustEngine/diskAdmission.ts';

/** Explicit candidate-only constructor, absent from normal production startup. */
interface QualifiedSession extends ExperimentalFreshRunNativeHandle {
  /** Delegate all state construction to the real Rust qualification constructor. */
  initializeBaselineV2ForTest(request: {profile: string; bins: number; mode: number; cadence: number;
    rayMinimum: number; rayCap: number; rayBatch: number}): Promise<unknown>;
}
/** Production-sized bounded queues, without any external controller. */
const INIT: ExperimentalEngineInit = {contractVersion: 1, maxInboundBatches: 16, maxInboundCommands: 16,
  maxInboundOwnedBytes: 2 * 1024 ** 2, maxBatchCommands: 1, maxBatchOwnedBytes: 1024 ** 2,
  maxOutputReliable: 32, maxOutputReliableOwnedBytes: 16 * 1024 ** 2, maxOutputDiscrete: 4,
  maxOutputDiscreteOwnedBytes: 1024 ** 2, maxOutputTotalOwnedBytes: 32 * 1024 ** 2,
  maxOutputEventOwnedBytes: 1024 ** 2, maxOutputFrameConnections: 4};

/** Run only the explicitly supplied Python interpreter, bounded in output and wall time. */
async function python(pythonPath: string, checkout: string, args: string[]): Promise<void> {
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
    const timer = setTimeout(() => terminate(new Error('qualification Python process deadline')), 630_000);
    for (const stream of [child.stdout, child.stderr]) stream.on('data', (bytes: Buffer) => {
      output += bytes.toString();
      if (Buffer.byteLength(output) > 256 * 1024) terminate(new Error('qualification Python output admission'));
    });
    child.once('error', error => {clearTimeout(timer); failure = error; reject(error);});
    child.once('close', code => {
      clearTimeout(timer); if (failure) {reject(failure); return;}
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

/** Require explicit checkout/interpreter/report, so this gate cannot touch an owner database. */
async function run(): Promise<void> {
  const args = new Map<string, string>();
  for (let index = 2; index < process.argv.length; index += 2) {
    const key = process.argv[index]!; const value = process.argv[index + 1];
    if (!['--pyrl-root', '--python', '--device', '--workers', '--report', '--spool-gib'].includes(key) || value === undefined || args.has(key)) throw new Error('invalid qualification arguments');
    args.set(key, value);
  }
  for (const key of ['--pyrl-root', '--python', '--device', '--workers', '--report']) assert(args.has(key), 'required ' + key);
  const checkout = resolve(args.get('--pyrl-root')!); const pythonPath = resolve(args.get('--python')!);
  const device = args.get('--device')!; const workers = Number(args.get('--workers'));
  const spoolGiB = Number(args.get('--spool-gib') ?? '4');
  const spoolLimits = distillationSpoolLimitsForGiB(spoolGiB);
  assert(['cpu', 'cuda:0'].includes(device)); assert([1, 3].includes(workers));
  const output = resolve(args.get('--report')!); assert(!existsSync(output), 'qualification report already exists');
  const hostSha256 = await digest(fileURLToPath(import.meta.url));
  const trainerGateSha256 = await digest(join(checkout, 'pyrl_trainer/live_distillation_gate.py'));
  const root = await mkdtemp(join(tmpdir(), 'slither-live-distillation-'));
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
    await admitDiskOperation(managed, CHECKPOINT_DISK_ADMISSION_REQUEST);
    const binding = loadPanicBinding() as ReturnType<typeof loadPanicBinding> & {
      validateBrainArtifactFile(path: string, registerShadow?: boolean): string;
      nativeAddonSourceSha256(): string; nativeAddonBuildTarget(): string;
    };
    const Constructor = binding.ExperimentalStage6aFreshRunSession as new (run: string, seed: string, memory: string, workers: number) => QualifiedSession;
    const runId = 'live-distillation-' + randomBytes(8).toString('hex');
    const session = new Constructor(runId, '0000002a', '0000000100000000', workers);
    await session.initializeBaselineV2ForTest({profile: 'basic', bins: 8, mode: 0, cadence: 0.06, rayMinimum: 1, rayCap: 256, rayBatch: 65536});
    persistence = new CheckpointPersistenceClient({databasePath: join(root, 'game.sqlite'), managedRootPath: managed});
    const descriptor = parseManagedCheckpointDescriptor(await session.publishRunStartCheckpoint({managedDirectory: managed, operationId: randomBytes(16).toString('hex')}));
    await persistence.commit(descriptor); session.acknowledgeRunStartPersistence(descriptor);
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
      admitCheckpoint: async () => {await admitDiskOperation(managed, CHECKPOINT_DISK_ADMISSION_REQUEST);}});
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
          await generation.handle(event);
        }
      })().catch(error => {pumpError = error;}).finally(() => {activePump = undefined;});
    }, 5);
    native.start(); await new Promise<void>(done => server.listen(0, '127.0.0.1', done));
    const address = server.address(); assert(address && typeof address !== 'string');
    const dataset = await capture.start({expectedRunId: runId, expectedConfigRevision: '0000000000000001',
      baselineSlots: Array.from({length: 10}, (_, slot) => slot), pendingBytes: 1024 ** 2, queueChunks: 32, queueBytes: 4 * 1024 ** 2});
    const origin = 'http://127.0.0.1:' + address.port;
    const producerStoragePolicy = JSON.parse((await exchange(address.port, '/api/distillation/v1/storage', 'GET')).toString('utf8')) as {maxBytes: string};
    assert.equal(BigInt('0x' + producerStoragePolicy.maxBytes), BigInt(spoolGiB) * 1024n ** 3n);
    const jobDirectory = join(root, 'job');
    const common = ['--origin', origin, '--capture-epoch', dataset.contract.captureEpoch,
      '--artifact', resolve('native/fixtures/portable-brain-v1.slither-brain'), '--directory', jobDirectory, '--device', device, '--spool-gib', String(spoolGiB)];
    const before = native.health();
    console.info('[live-distillation] training', device, 'workers=' + workers);
    await python(pythonPath, checkout, ['--phase', 'train', ...common]);
    if (pumpError) throw pumpError;
    await python(pythonPath, checkout, ['--phase', 'resume', ...common]);
    if (pumpError) throw pumpError;
    const train = JSON.parse(await readFile(join(jobDirectory, 'train-report.json'), 'utf8')) as Record<string, unknown>;
    const resume = JSON.parse(await readFile(join(jobDirectory, 'resume-report.json'), 'utf8')) as Record<string, unknown>;
    assert.equal(resume['updates'], 250); assert.equal(resume['restartExact'], true);
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
    // Train an active-graph-compatible student too. Differing architectures may
    // be retained, while the single-graph runtime must reject their deployment.
    const denseDirectory = join(root, 'dense-job');
    const denseCommon = ['--origin', origin, '--capture-epoch', dataset.contract.captureEpoch,
      '--artifact', resolve('native/fixtures/portable-dense-basic8-v1.slither-brain'), '--directory', denseDirectory, '--device', device];
    await python(pythonPath, checkout, ['--phase', 'train', ...denseCommon]);
    await python(pythonPath, checkout, ['--phase', 'resume', ...denseCommon]);
    if (pumpError) throw pumpError;
    const denseResume = JSON.parse(await readFile(join(denseDirectory, 'resume-report.json'), 'utf8')) as Record<string, unknown>;
    assert.equal(denseResume['updates'], 250); assert.equal(denseResume['restartExact'], true);
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
    await removeRoot(root);
  }
}

await run();
