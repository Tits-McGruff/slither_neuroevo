/** Load the production opaque spool without constructing a game or sample codec. */
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { existsSync } from 'node:fs';
import { lstat, mkdir, readFile, realpath, rm, writeFile } from 'node:fs/promises';
import { availableParallelism, cpus, platform, release, totalmem } from 'node:os';
import { basename, dirname, isAbsolute, relative, resolve, sep } from 'node:path';
import type { TeacherCaptureContract } from '../../src/protocol/distillation.ts';
import { DistillationSpoolClient } from '../../server/rustEngine/distillationSpoolClient.ts';
import { DISTILLATION_ZERO, distillationSpoolLimitsForGiB, TEACHER_BATCH_BYTES } from '../../server/rustEngine/distillationProtocol.ts';
import { costs, digest, drain, freeBytes, object, sample, settled, word } from './spoolFixture.ts';

/** Explicit load choices; object count defaults to the selected production ceiling. */
interface Options {
  /** Exclusively created directory, removed after closing its owned worker. */
  workspace: string;
  /** Create-new report outside the disposable directory. */
  output: string;
  /** Actual producer quota, without changing any other production bound. */
  quotaGiB: number;
  /** Smaller restart quota exercises real oldest-unpinned rotation. */
  restartQuotaGiB: number;
  /** Physical bytes in each synthetic opaque object. */
  objectBytes: number;
  /** Initial object count; a partial count is explicitly a smoke run. */
  objects: number;
  /** Bound total work in addition to the client's unchanged operation deadlines. */
  deadlineMs: number;
  /** Keep a failed task-owned load only when it is explicitly needed for diagnosis. */
  retainOnFailure: boolean;
}
/** Validate arguments before creating any storage or reading source fixtures. */
function options(): Options {
  const fields = new Map<string, string>();
  const allowed = ['--workspace', '--output', '--quota-gib', '--restart-quota-gib', '--object-kib', '--objects', '--deadline-seconds', '--retain-on-failure'];
  const args = process.argv.slice(2);
  for (let i = 0; i < args.length; i += 2) {
    const name = args[i]!; const value = args[i + 1];
    if (!allowed.includes(name) || value === undefined || fields.has(name)) throw new Error('unknown, duplicate or incomplete spool qualification option');
    fields.set(name, value);
  }
  /** Read one closed-range decimal integer before any filesystem mutation. */
  const integer = (key: string, fallback: number, minimum: number, maximum: number): number => {
    const raw = fields.get(key) ?? String(fallback);
    if (!/^[0-9]+$/u.test(raw)) throw new Error('invalid integer ' + key);
    const value = Number(raw);
    if (!Number.isSafeInteger(value) || value < minimum || value > maximum) throw new Error('out-of-range ' + key);
    return value;
  };
  const quotaGiB = integer('--quota-gib', 4, 1, 64);
  const restartQuotaGiB = integer('--restart-quota-gib', 1, 1, quotaGiB);
  const objectKiB = integer('--object-kib', 1024, 64, 1024);
  if (![64, 128, 256, 512, 1024].includes(objectKiB)) throw new Error('object size must be 64, 128, 256, 512 or 1024 KiB');
  const policy = distillationSpoolLimitsForGiB(quotaGiB);
  const objectBytes = objectKiB * 1024;
  const capacity = Math.min(policy.maxChunks, Math.floor(policy.maxBytes / objectBytes));
  const objects = integer('--objects', capacity, 64, capacity);
  const workspace = fields.get('--workspace'); const output = fields.get('--output');
  if (!workspace || !output) throw new Error('--workspace and --output are required');
  const root = resolve(workspace); const report = resolve(output);
  const nested = relative(root, report);
  if (!isAbsolute(nested) && nested !== '..' && !nested.startsWith('..' + sep)) throw new Error('report must be outside disposable storage');
  if (existsSync(root) || existsSync(report)) throw new Error('workspace and report must be new');
  const retain = fields.get('--retain-on-failure') ?? 'false';
  if (retain !== 'true' && retain !== 'false') throw new Error('retain-on-failure must be true or false');
  return {workspace: root, output: report, quotaGiB, restartQuotaGiB, objectBytes, objects, retainOnFailure: retain === 'true',
    deadlineMs: integer('--deadline-seconds', 1800, 1, 10_800) * 1000};
}
/** Run only task-owned storage; normal worker deadlines and all production quotas remain active. */
async function run(): Promise<void> {
  const args = options(); const policy = distillationSpoolLimitsForGiB(args.quotaGiB);
  const capacity = Math.min(policy.maxChunks, Math.floor(policy.maxBytes / args.objectBytes));
  const full = args.objects === capacity;
  const pressure = Math.min(256, args.objects);
  const required = BigInt(Math.min(policy.maxBytes, (args.objects + pressure) * args.objectBytes)) +
    BigInt(policy.diskReserveBytes) + 64n * 1024n ** 2n;
  const beforeFree = await freeBytes(dirname(args.workspace)); assert(beforeFree >= required, 'physical space must preserve the production reserve');
  const fixturePath = resolve(import.meta.dirname, '../../native/fixtures/sensor-contract-shapes.json');
  const fixture = (JSON.parse(await readFile(fixturePath, 'utf8')) as Array<{profile: string; bins: number; canonicalJson: string; sha256: string}>)
    .find(entry => entry.profile === 'motion' && entry.bins === 32);
  assert(fixture, 'Rust Motion/B32 structural fixture required');
  const contract: TeacherCaptureContract = {captureEpoch: randomBytes(16).toString('hex'), runId: 'spool-capacity-qualification',
    configSha256: '23'.repeat(32), sensorSha256: fixture.sha256, teacherSha256: '37'.repeat(32), teacherRevision: 2,
    teacherMode: 1, controlIntervalSeconds: 1 / 60, inputSize: 504};
  const sources: Record<string, string> = {};
  for (const path of ['scripts/qualification/spool-capacity.ts', 'scripts/qualification/spoolFixture.ts', 'server/rustEngine/distillationSpoolClient.ts',
    'server/rustEngine/distillationSpoolWorker.ts', 'server/rustEngine/distillationSpoolStore.ts',
    'server/rustEngine/distillationProtocol.ts', 'server/rustEngine/distillationSpoolLimits.ts',
    'src/protocol/distillation.ts', 'src/protocol/sensors.ts', 'native/fixtures/sensor-contract-shapes.json']) {
    sources[path] = digest(Buffer.from((await readFile(resolve(path), 'utf8')).replace(/\r\n/gu, '\n')));
  }
  await mkdir(args.workspace); const ownedRoot = await realpath(args.workspace);
  let client: DistillationSpoolClient | undefined;
  let peakRss = process.memoryUsage().rss; const started = performance.now(); const cpuStart = process.cpuUsage();
  const memoryTimer = setInterval(() => {peakRss = Math.max(peakRss, process.memoryUsage().rss);}, 250);
  const phases: Record<string, unknown> = {}; const ingestion = costs(); let initiallyStored = 0;
  let failure: Error | undefined;
  /** Stop at a bounded batch boundary while retaining normal worker deadlines. */
  const deadline = (): void => { if (performance.now() - started > args.deadlineMs) throw new Error('spool qualification total deadline'); };
  try {
    let phase = performance.now(); client = new DistillationSpoolClient(ownedRoot, policy); await client.ready;
    phases['emptyStartupMs'] = performance.now() - phase; phases['admittedPolicy'] = await client.storagePolicy();
    const batch = Math.floor(TEACHER_BATCH_BYTES / args.objectBytes);
    for (let first = 1; first <= args.objects;) {
      deadline(); const count = Math.min(batch, args.objects - first + 1); phase = performance.now();
      const stored = await client.ingest(drain(contract, fixture.canonicalJson, first, count, args.objectBytes), 42);
      initiallyStored = stored.retainedChunks;
      settled(stored, first + count - 1, first + count - 1, args.objectBytes);
      sample(ingestion, performance.now() - phase); first += count;
      peakRss = Math.max(peakRss, process.memoryUsage().rss);
      if ((first - 1) % 1024 === 0 || first > args.objects) console.info('[spool-capacity] stored', first - 1, '/', args.objects);
    }
    settled(await client.dataset(contract.captureEpoch), args.objects, args.objects, args.objectBytes);
    const metadataReads = Array.from({length: 32}, () => client!.dataset(contract.captureEpoch));
    await assert.rejects(client.dataset(contract.captureEpoch), /request admission/u);
    for (const dataset of await Promise.all(metadataReads)) settled(dataset, args.objects, args.objects, args.objectBytes);
    phases['metadataAdmission'] = {concurrentReads: 32, nextRequestRejected: true};
    const maximumPins = Math.min(policy.maxPinnedChunks, Math.floor(policy.maxPinnedBytes / args.objectBytes));
    const pinCount = Math.min(maximumPins, Math.floor(args.objects / 2));
    const leases: string[] = [];
    for (let first = 1; first <= pinCount; first += 256) {
      deadline();
      const selection = Array.from({length: Math.min(256, pinCount - first + 1)}, (_, i) => word(first + i));
      leases.push((await client.lease(contract.captureEpoch, selection, policy.maxLeaseMs)).id);
    }
    const overlap = await client.lease(contract.captureEpoch, [word(1)], policy.maxLeaseMs); leases.push(overlap.id);
    if (pinCount === maximumPins) await assert.rejects(client.lease(contract.captureEpoch, [word(pinCount + 1)], policy.maxLeaseMs), /pin quota/u);
    const retainedLeaseCount = leases.length;
    while (leases.length < policy.maxLeases) { deadline(); leases.push((await client.lease(contract.captureEpoch, [word(1)], policy.maxLeaseMs)).id); }
    await assert.rejects(client.lease(contract.captureEpoch, [word(1)], policy.maxLeaseMs), /count quota/u);
    for (const id of leases.splice(retainedLeaseCount)) { deadline(); await client.releaseLease(id); }
    phases['leaseAdmission'] = {uniquePinnedChunks: pinCount, uniquePinnedBytes: pinCount * args.objectBytes,
      overlappingPinsCountedOnce: true, pinCeilingExercised: pinCount === maximumPins, leaseCountCeilingRejected: true};
    phase = performance.now();
    for (let offset = 0; offset < pressure; offset += batch) {
      deadline(); await client.ingest(drain(contract, fixture.canonicalJson, args.objects + offset + 1,
        Math.min(batch, pressure - offset), args.objectBytes), 42);
    }
    const last = args.objects + pressure; const retained = Math.min(capacity, last);
    const sealed = await client.ingest(drain(contract, fixture.canonicalJson, last + 1, 0, args.objectBytes, true), 42);
    settled(sealed, last, retained, args.objectBytes); assert.equal(sealed.retentionFloor, word(1)); assert.equal(sealed.state, 'stopped');
    if (full) await assert.rejects(client.lease(contract.captureEpoch, [word(pinCount + 1)], policy.maxLeaseMs), /unavailable/u);
    const download = await client.download(contract.captureEpoch, word(1), overlap.id);
    assert.equal(digest(download.bytes), digest(object(1, args.objectBytes)));
    const downloads = Array.from({length: 4}, () => client!.download(contract.captureEpoch, word(1), overlap.id));
    await assert.rejects(client.download(contract.captureEpoch, word(1), overlap.id), /concurrency/u);
    for (const downloaded of await Promise.all(downloads)) assert.equal(digest(downloaded.bytes), digest(object(1, args.objectBytes)));
    const expiry = await client.lease(contract.captureEpoch, [word(1)], 1);
    await new Promise<void>(done => setTimeout(done, 10));
    await assert.rejects(client.download(contract.captureEpoch, word(1), expiry.id), /expired/u);
    await client.releaseLease(expiry.id);
    await client.renewLease(overlap.id, policy.maxLeaseMs);
    phases['downloadAdmission'] = {fourConcurrentExactDownloads: true, fifthRejected: true, expiredLeaseRejected: true, renewalAccepted: true};
    let after = DISTILLATION_ZERO; let listed = 0; let pages = 0;
    do {
      const page = await client.list(contract.captureEpoch, after, 256);
      assert(page.chunks.length <= 256);
      for (const chunk of page.chunks) { assert(chunk.sequence > after); after = chunk.sequence; listed++; }
      pages++; assert.equal(page.nextCursor, after);
      if (!page.hasMore) break;
      assert(page.chunks.length > 0); deadline();
    } while (pages <= Math.ceil(policy.maxChunks / 256));
    assert.equal(listed, retained); assert.equal(after, word(last));
    phases['pagination'] = {listedObjects: listed, pages, strictlyOrdered: true};
    let epoch = BigInt('0x' + contract.captureEpoch);
    for (let i = 1; i < policy.maxDatasets; i++) {
      do { epoch = (epoch + 1n) & ((1n << 128n) - 1n); } while (epoch === 0n);
      const empty = {...contract, captureEpoch: epoch.toString(16).padStart(32, '0')};
      await client.ingest(drain(empty, fixture.canonicalJson, 1, 0, args.objectBytes, true), 42); deadline();
    }
    do { epoch = (epoch + 1n) & ((1n << 128n) - 1n); } while (epoch === 0n);
    await assert.rejects(client.ingest(drain({...contract, captureEpoch: epoch.toString(16).padStart(32, '0')},
      fixture.canonicalJson, 1, 0, args.objectBytes, true), 42), /manifest quota/u);
    const datasets = await client.datasets('0'.repeat(32), 256);
    assert.equal(datasets.datasets.length, policy.maxDatasets); assert.equal(datasets.hasMore, false);
    phases['datasetAdmission'] = {retainedDatasets: datasets.datasets.length, nextDatasetRejected: true};
    phases['rotation'] = {milliseconds: performance.now() - phase, fullQuotaExercised: full, objectsEvicted: last - retained,
      pinnedDownloadExact: true, dataset: sealed};
    for (const id of leases) { deadline(); await client.renewLease(id, policy.maxLeaseMs); }
    await client.close(); client = undefined; deadline();
    phase = performance.now(); client = new DistillationSpoolClient(ownedRoot, policy); await client.ready;
    phases['loadedRestartMs'] = performance.now() - phase; phases['loadedRecoveryBudgetMs'] = client.recoveryBudgetMs;
    const loadedDatasets = await client.datasets('0'.repeat(32), 256);
    assert.equal(loadedDatasets.datasets.length, policy.maxDatasets); assert.equal(loadedDatasets.hasMore, false);
    phases['loadedDatasetCount'] = loadedDatasets.datasets.length;
    const restored = await client.dataset(contract.captureEpoch); settled(restored, last, retained, args.objectBytes);
    assert.equal(restored.state, 'stopped');
    assert.equal(digest((await client.download(contract.captureEpoch, word(1), overlap.id)).bytes), digest(object(1, args.objectBytes)));
    for (const id of leases) { deadline(); await client.renewLease(id, policy.maxLeaseMs); }
    await client.close(); client = undefined; deadline();
    const smaller = distillationSpoolLimitsForGiB(args.restartQuotaGiB);
    phase = performance.now(); client = new DistillationSpoolClient(ownedRoot, smaller); await client.ready;
    phases['smallerQuotaRestartMs'] = performance.now() - phase; phases['smallerRecoveryBudgetMs'] = client.recoveryBudgetMs;
    const smallerRetained = Math.min(retained, Math.floor(smaller.maxBytes / args.objectBytes));
    const result = await client.dataset(contract.captureEpoch); settled(result, last, smallerRetained, args.objectBytes);
    assert.equal(result.state, 'stopped');
    assert.equal((await client.datasets('0'.repeat(32), 256)).datasets.length, policy.maxDatasets);
    assert.equal(digest((await client.download(contract.captureEpoch, word(1), overlap.id)).bytes), digest(object(1, args.objectBytes)));
    for (const id of leases) { deadline(); await client.releaseLease(id); }
    await client.releaseLease(overlap.id);
    phases['finalPolicy'] = await client.storagePolicy(); phases['finalDataset'] = result;
    assert(await freeBytes(ownedRoot) >= BigInt(policy.diskReserveBytes), 'filesystem reserve lost');
  } catch (error) {failure = error instanceof Error ? error : new Error(String(error));}
  finally {
    clearInterval(memoryTimer); peakRss = Math.max(peakRss, process.memoryUsage().rss);
    try { await client?.close(); } catch (error) { failure ??= error instanceof Error ? error : new Error(String(error)); }
    assert.equal(await realpath(args.workspace), ownedRoot); assert(!(await lstat(args.workspace)).isSymbolicLink());
    assert.equal(dirname(ownedRoot).toLowerCase(), (await realpath(dirname(args.workspace))).toLowerCase());
    assert.equal(basename(ownedRoot).toLowerCase(), basename(args.workspace).toLowerCase());
    if (!failure || !args.retainOnFailure) await rm(ownedRoot, {recursive: true, force: true});
  }
  const cpu = process.cpuUsage(cpuStart);
  const report = {schemaVersion: 1, gate: 'distillation-spool-capacity-v1', status: failure ? 'failed' : 'passed',
    qualificationComplete: !failure && full, initialObjectsStored: initiallyStored,
    initialObjectCountMeaning: 'Last acknowledged durable inventory before pressure writes; unacknowledged partial work is not counted.',
    retentionCeilingLoaded: initiallyStored === capacity,
    selectedByteQuotaLoaded: initiallyStored * args.objectBytes === policy.maxBytes, retainedObjectCeilingLoaded: initiallyStored === policy.maxChunks,
    scope: 'Synthetic opaque storage objects through the production isolated spool worker. No native sample codec, game, HTTP transport, trainer, shadows or integrated soak is qualified. Partial object counts are smoke runs. Requests retain the production 60-second deadline; startup uses the bounded actual-inventory recovery allowance. Shared-host RSS and physical-filesystem measurements are descriptive.',
    source: {revision: execFileSync('git', ['rev-parse', 'HEAD'], {encoding: 'utf8'}).trim(),
        workingTree: execFileSync('git', ['status', '--porcelain'], {encoding: 'utf8'}).trim(), canonicalLfSha256: sources},
    environment: {node: process.version, platform: platform(), release: release(), cpu: cpus()[0]?.model,
      availableParallelism: availableParallelism(), totalMemoryBytes: totalmem()},
    options: {quotaGiB: args.quotaGiB, restartQuotaGiB: args.restartQuotaGiB, objectBytes: args.objectBytes, objects: args.objects, retainOnFailure: args.retainOnFailure},
    physicalFreeBytesBefore: beforeFree.toString(), physicalFreeBytesAfter: (await freeBytes(dirname(args.workspace))).toString(),
    elapsedMs: performance.now() - started, processCpuMs: {user: cpu.user / 1000, system: cpu.system / 1000, total: (cpu.user + cpu.system) / 1000},
    sampledPeakRssBytes: peakRss, ingestion, phases,
    failure: failure?.message ?? null, workspaceRemoved: !existsSync(args.workspace)};
  await mkdir(dirname(args.output), {recursive: true}); await writeFile(args.output, JSON.stringify(report, null, 2) + '\n', {flag: 'wx'});
  console.info('[spool-capacity]', report.status, args.output); if (failure) throw failure;
}

await run();
