/** Retest only an owned retained synthetic full-byte fixture through production recovery. */
import assert from 'node:assert/strict';
import Database from 'better-sqlite3';
import {execFileSync} from 'node:child_process';
import {existsSync} from 'node:fs';
import {lstat, readFile, readdir, realpath, rm, open, mkdir} from 'node:fs/promises';
import {dirname, isAbsolute, relative, resolve, sep} from 'node:path';
import {platform, release, totalmem} from 'node:os';
import type {DistillationDataset, TeacherCaptureContract} from '../../src/protocol/distillation.ts';
import {DistillationSpoolClient} from '../../server/rustEngine/distillationSpoolClient.ts';
import {distillationSpoolLimitsForGiB} from '../../server/rustEngine/distillationProtocol.ts';
import {digest, freeBytes, object, settled, word} from './spoolFixture.ts';

/** Verify original failed evidence and every entry's ownership before any mutable worker. */
async function ownership(workspace: string, failurePath: string): Promise<{
  root: string; dev: bigint; ino: bigint; epoch: string; original: DistillationDataset; predecessor: string;
}> {
  const requested = await lstat(workspace); assert(requested.isDirectory() && !requested.isSymbolicLink());
  const root = await realpath(workspace); const identity = await lstat(root, {bigint: true});
  const previousBytes = await readFile(failurePath, 'utf8'); assert(Buffer.byteLength(previousBytes) <= 1024 ** 2);
  const previous = JSON.parse(previousBytes) as {gate: string; status: string; workspaceRemoved: boolean;
    selectedByteQuotaLoaded: boolean; options: {quotaGiB: number; objectBytes: number; objects: number};
    phases: {rotation: {dataset: DistillationDataset}}};
  assert.equal(previous.gate, 'distillation-spool-capacity-v1'); assert.equal(previous.status, 'failed');
  assert.equal(previous.workspaceRemoved, false); assert.equal(previous.selectedByteQuotaLoaded, true);
  assert.equal(previous.options.quotaGiB, 64); assert.equal(previous.options.objectBytes, 1024 ** 2);
  assert.equal(previous.options.objects, 65536);
  const original = previous.phases.rotation.dataset; const epoch = original.contract.captureEpoch;
  assert(/^[0-9a-f]{32}$/u.test(epoch)); assert.notEqual(epoch, '0'.repeat(32));
  const fixture = (JSON.parse(await readFile(resolve(import.meta.dirname, '../../native/fixtures/sensor-contract-shapes.json'), 'utf8')) as
    Array<{profile: string; bins: number; canonicalJson: string; sha256: string}>).find(value => value.profile === 'motion' && value.bins === 32);
  assert(fixture);
  const metadata = resolve(root, 'metadata.sqlite'); const metadataInfo = await lstat(metadata);
  assert(metadataInfo.isFile() && !metadataInfo.isSymbolicLink());
  const db = new Database(metadata, {readonly: true, fileMustExist: true});
  try {
    assert.equal(db.pragma('application_id', {simple: true}), 0x534c5448);
    assert.equal(db.pragma('user_version', {simple: true}), 2);
    const datasets = db.prepare('SELECT epoch, contract, sensors, run_seed, state FROM datasets').all() as
      Array<{epoch: string; contract: string; sensors: string; run_seed: number; state: string}>;
    assert.equal(datasets.length, 256); const epochs = new Set(datasets.map(value => value.epoch)); assert(epochs.has(epoch));
    for (const row of datasets) {
      const contract = JSON.parse(row.contract) as TeacherCaptureContract;
      assert.equal(contract.captureEpoch, row.epoch); assert.equal(contract.runId, 'spool-capacity-qualification');
      assert.equal(contract.configSha256, '23'.repeat(32)); assert.equal(contract.teacherSha256, '37'.repeat(32));
      assert.equal(contract.sensorSha256, fixture.sha256); assert.equal(row.sensors, fixture.canonicalJson);
      assert.equal(contract.inputSize, 504); assert.equal(contract.teacherRevision, 2); assert.equal(contract.teacherMode, 1);
      assert.equal(contract.controlIntervalSeconds, 1 / 60); assert.equal(row.run_seed, 42);
      assert(['stopped', 'interrupted'].includes(row.state));
    }
    const inventory = db.prepare('SELECT COUNT(*) count, COALESCE(SUM(bytes), 0) bytes, MIN(bytes) minimum, MAX(bytes) maximum FROM chunks')
      .get() as {count: number; bytes: number; minimum: number; maximum: number};
    assert.deepEqual(inventory, {count: 65536, bytes: 64 * 1024 ** 3, minimum: 1024 ** 2, maximum: 1024 ** 2});
    const entries = await readdir(root, {withFileTypes: true}); assert(entries.length <= 2 * 65536 + 16);
    for (const entry of entries) {
      assert(entry.isFile() && !entry.isSymbolicLink(), 'mixed or linked fixture entry');
      if (/^metadata\.sqlite(?:-wal|-shm)?$/u.test(entry.name)) continue;
      assert(/^[0-9a-f]{32}-[0-9a-f]{16}-[0-9a-f]{64}\.teacher(?:\.[0-9a-f]{16}\.pending)?$/u.test(entry.name));
      assert(epochs.has(entry.name.slice(0, 32)), 'foreign capture object in fixture');
    }
  } finally {db.close();}
  return {root, dev: identity.dev, ino: identity.ino, epoch, original,
    predecessor: digest(Buffer.from(previousBytes.replace(/\r\n/gu, '\n')))};
}

/** Fsync an exclusive report outside the disposable fixture before any cleanup. */
async function save(path: string, value: unknown): Promise<void> {
  await mkdir(dirname(path), {recursive: true}); const file = await open(path, 'wx');
  try {await file.writeFile(JSON.stringify(value, null, 2) + '\n'); await file.sync();} finally {await file.close();}
}

/** Load/hash the full retained quota, then verify reduced-quota recovery and lease expiry. */
async function run(): Promise<void> {
  const args = process.argv.slice(2);
  assert.equal(args.length, 6, 'required: --workspace PATH --failed-report PATH --output PATH');
  assert.equal(args[0], '--workspace'); assert.equal(args[2], '--failed-report'); assert.equal(args[4], '--output');
  const workspace = resolve(args[1]!); const output = resolve(args[5]!); const progress = output + '.progress.json';
  const outside = relative(workspace, output);
  assert(outside && (outside === '..' || outside.startsWith('..' + sep) || isAbsolute(outside)));
  assert(!existsSync(output) && !existsSync(progress), 'create-new reports required');
  const owned = await ownership(workspace, resolve(args[3]!)); const size = 1024 ** 2;
  const beforeFree = await freeBytes(owned.root); assert(beforeFree >= 1024n ** 3n);
  const source: Record<string, string> = {};
  for (const path of ['scripts/qualification/spool-capacity-recover.ts', 'scripts/qualification/spoolFixture.ts',
    'server/rustEngine/distillationSpoolClient.ts', 'server/rustEngine/distillationSpoolWorker.ts',
    'server/rustEngine/distillationSpoolStore.ts', 'server/rustEngine/distillationProtocol.ts',
    'server/rustEngine/distillationSpoolLimits.ts', 'src/protocol/distillation.ts', 'native/fixtures/sensor-contract-shapes.json']) {
    source[path] = digest(Buffer.from((await readFile(resolve(import.meta.dirname, '../..', path), 'utf8')).replace(/\r\n/gu, '\n')));
  }
  let client: DistillationSpoolClient | undefined; let failure: Error | undefined;
  const phases: Record<string, unknown> = {}; const started = performance.now(); const cpuStart = process.cpuUsage();
  let peakRss = process.memoryUsage().rss;
  const timer = setInterval(() => {peakRss = Math.max(peakRss, process.memoryUsage().rss);}, 250);
  const last = Number(BigInt('0x' + owned.original.productionWatermark));
  try {
    let phase = performance.now(); client = new DistillationSpoolClient(owned.root, distillationSpoolLimitsForGiB(64)); await client.ready;
    phases['fullRestart'] = {milliseconds: performance.now() - phase, recoveryBudgetMs: client.recoveryBudgetMs};
    assert.equal(client.recoveryBudgetMs, 660000);
    let dataset = await client.dataset(owned.epoch); settled(dataset, last, 65536, size);
    assert.equal(dataset.state, owned.original.state); assert.deepEqual(dataset.contract, owned.original.contract);
    assert.equal((await client.datasets('0'.repeat(32), 256)).datasets.length, 256);
    const lease = await client.lease(owned.epoch, [word(1), word(last)], 300000);
    for (const sequence of [1, last]) assert.equal(digest((await client.download(owned.epoch, word(sequence), lease.id)).bytes), digest(object(sequence, size)));
    phases['fullDataset'] = dataset;
    await client.close(); client = undefined;
    phase = performance.now(); client = new DistillationSpoolClient(owned.root, distillationSpoolLimitsForGiB(1)); await client.ready;
    phases['smallerRestart'] = {milliseconds: performance.now() - phase, recoveryBudgetMs: client.recoveryBudgetMs};
    assert.equal(client.recoveryBudgetMs, 660000, 'price retained data before quota reduction');
    dataset = await client.dataset(owned.epoch); settled(dataset, last, 1024, size);
    assert.equal(dataset.state, owned.original.state); assert.deepEqual(dataset.contract, owned.original.contract);
    assert.equal((await client.datasets('0'.repeat(32), 256)).datasets.length, 256);
    const leaseChecks: Array<{sequence: number; result: string}> = [];
    for (const sequence of [1, last]) {
      try {
        const value = await client.download(owned.epoch, word(sequence), lease.id);
        assert.equal(digest(value.bytes), digest(object(sequence, size))); leaseChecks.push({sequence, result: 'exact'});
      } catch (error) {
        assert(Date.now() >= lease.expiresAt, 'a still-live pin was lost');
        assert(error instanceof Error && /expired|not found|unavailable/u.test(error.message));
        leaseChecks.push({sequence, result: 'expired-as-declared'});
      }
    }
    const fresh = await client.lease(owned.epoch, [word(last)], 300000);
    assert.equal(digest((await client.download(owned.epoch, word(last), fresh.id)).bytes), digest(object(last, size)));
    await client.releaseLease(fresh.id); await client.releaseLease(lease.id);
    phases['leaseChecks'] = {originalExpiryMs: lease.expiresAt, checks: leaseChecks, freshRecentPinExact: true};
    phases['finalDataset'] = dataset; phases['finalPolicy'] = await client.storagePolicy();
    assert(await freeBytes(owned.root) >= 1024n ** 3n);
  } catch (error) {failure = error instanceof Error ? error : new Error(String(error));}
  finally {
    clearInterval(timer); peakRss = Math.max(peakRss, process.memoryUsage().rss);
    try {await client?.close();} catch (error) {failure ??= error instanceof Error ? error : new Error(String(error));}
  }
  const cpu = process.cpuUsage(cpuStart);
  const report = {schemaVersion: 1, gate: 'distillation-spool-retained-full-byte-recovery-v1',
    status: failure ? 'failed' : 'checks-passed-cleanup-pending', qualificationComplete: false,
    scope: 'Previously filled synthetic opaque 64-GiB/65,536-object fixture through the production worker. Every retained file is verified before readiness. Inventory-priced startup has a finite 660-second ceiling; requests retain 60 seconds and leases retain their declared wall-time expiry. No native sample codec, HTTP, trainer or integrated soak is qualified.',
    source: {revision: execFileSync('git', ['rev-parse', 'HEAD'], {encoding: 'utf8'}).trim(), canonicalLfSha256: source},
    predecessorCanonicalLfSha256: owned.predecessor, environment: {node: process.version, platform: platform(), release: release(), totalMemoryBytes: totalmem()},
    options: {quotaGiB: 64, restartQuotaGiB: 1, objectBytes: size, objects: 65536},
    elapsedMs: performance.now() - started, processCpuMs: {user: cpu.user / 1000, system: cpu.system / 1000},
    sampledPeakRssBytes: peakRss, physicalFreeBytesBefore: beforeFree.toString(), phases, failure: failure?.message ?? null, workspaceRemoved: false};
  // Retain complete measurement evidence durably before deleting any successful load.
  await save(progress, report);
  assert.equal(await realpath(workspace), owned.root); const current = await lstat(owned.root, {bigint: true});
  assert(!current.isSymbolicLink()); assert.equal(current.dev, owned.dev); assert.equal(current.ino, owned.ino);
  if (!failure) await rm(owned.root, {recursive: true, force: true});
  await save(output, {...report, status: failure ? 'failed' : 'passed', qualificationComplete: !failure,
    workspaceRemoved: !existsSync(workspace), physicalFreeBytesAfter: (await freeBytes(dirname(owned.root))).toString()});
  console.info('[spool-recovery]', failure ? 'failed' : 'passed', output); if (failure) throw failure;
}
await run();
