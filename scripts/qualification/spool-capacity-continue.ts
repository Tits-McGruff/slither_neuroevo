/** Continue only a retained synthetic qualification fixture; interrupted captures stay closed. */
import assert from 'node:assert/strict';
import Database from 'better-sqlite3';
import { randomBytes } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { lstat, readFile, realpath, rm, writeFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { dirname, isAbsolute, relative, resolve, sep } from 'node:path';
import { platform, release, totalmem } from 'node:os';
import type { TeacherCaptureContract, DistillationDataset } from '../../src/protocol/distillation.ts';
import { DistillationSpoolClient } from '../../server/rustEngine/distillationSpoolClient.ts';
import { distillationSpoolLimitsForGiB, TEACHER_BATCH_BYTES } from '../../server/rustEngine/distillationProtocol.ts';
import { costs, digest, drain, freeBytes, object, sample, settled, word } from './spoolFixture.ts';

/** Inspect ownership before opening any mutable production worker. */
async function run(): Promise<void> {
  const args = process.argv.slice(2);
  assert.equal(args.length, 6, 'required: --workspace PATH --failed-report PATH --output PATH');
  assert.equal(args[0], '--workspace'); assert.equal(args[2], '--failed-report'); assert.equal(args[4], '--output');
  const workspace = resolve(args[1]!); const output = resolve(args[5]!);
  const outputRelative = relative(workspace, output);
  assert(outputRelative && (outputRelative === '..' || outputRelative.startsWith('..' + sep) || isAbsolute(outputRelative)));
  assert(!existsSync(output), 'create-new report required');
  assert(!(await lstat(workspace)).isSymbolicLink()); const root = await realpath(workspace);
  const identity = await lstat(root); assert(identity.isDirectory());
  const failureBytes = await readFile(resolve(args[3]!), 'utf8');
  const previous = JSON.parse(failureBytes) as {gate: string; status: string; workspaceRemoved: boolean;
    failure: string; options: {quotaGiB: number; objectBytes: number; objects: number; retainOnFailure: boolean}};
  assert.equal(previous.gate, 'distillation-spool-capacity-v1'); assert.equal(previous.status, 'failed');
  assert.equal(previous.workspaceRemoved, false); assert.equal(previous.options.retainOnFailure, true);
  assert(previous.failure.includes(root), 'original failed report must name this exact owned workspace');
  const {quotaGiB, objectBytes, objects} = previous.options;
  assert.equal(quotaGiB, 64); assert.equal(objectBytes, 65536); assert.equal(objects, 65536);
  const policy = distillationSpoolLimitsForGiB(quotaGiB);
  const fixtures = JSON.parse(await readFile(resolve('native/fixtures/sensor-contract-shapes.json'), 'utf8')) as
    Array<{profile: string; bins: number; sha256: string; canonicalJson: string}>;
  const fixture = fixtures.find(value => value.profile === 'motion' && value.bins === 32); assert(fixture);
  const metadata = resolve(root, 'metadata.sqlite');
  const metadataStat = await lstat(metadata); assert(metadataStat.isFile() && !metadataStat.isSymbolicLink());
  const db = new Database(metadata, {readonly: true, fileMustExist: true});
  let retainedBefore = 0; let oldContracts: TeacherCaptureContract[] = [];
  try {
    assert.equal(db.pragma('application_id', {simple: true}), 0x534c5448);
    assert.equal(db.pragma('user_version', {simple: true}), 2);
    const rows = db.prepare('SELECT contract, sensors, run_seed, state FROM datasets').all() as
      Array<{contract: string; sensors: string; run_seed: number; state: string}>;
    assert(rows.length > 0 && rows.length < policy.maxDatasets);
    oldContracts = rows.map(row => {
      const contract = JSON.parse(row.contract) as TeacherCaptureContract;
      assert.equal(contract.runId, 'spool-capacity-qualification'); assert.equal(contract.configSha256, '23'.repeat(32));
      assert.equal(contract.teacherSha256, '37'.repeat(32)); assert.equal(contract.sensorSha256, fixture.sha256);
      assert.equal(contract.inputSize, 504); assert.equal(contract.teacherRevision, 2); assert.equal(contract.teacherMode, 1);
      assert.equal(contract.controlIntervalSeconds, 1 / 60); assert.equal(row.sensors, fixture.canonicalJson);
      assert.equal(row.run_seed, 42); assert(['active', 'interrupted', 'stopped'].includes(row.state)); return contract;
    });
    const inventory = db.prepare('SELECT COUNT(*) count, COALESCE(SUM(bytes), 0) bytes, MIN(bytes) minimum, MAX(bytes) maximum FROM chunks')
      .get() as {count: number; bytes: number; minimum: number; maximum: number};
    retainedBefore = inventory.count; assert(retainedBefore > 0 && retainedBefore < objects);
    assert.equal(inventory.bytes, retainedBefore * objectBytes);
    assert.equal(inventory.minimum, objectBytes); assert.equal(inventory.maximum, objectBytes);
  } finally {db.close();}
  const beforeFree = await freeBytes(root);
  assert(beforeFree >= BigInt((objects - retainedBefore) * objectBytes + policy.diskReserveBytes + 64 * 1024 ** 2));
  const contract = {...oldContracts[0]!, captureEpoch: randomBytes(16).toString('hex')};
  assert(!oldContracts.some(value => value.captureEpoch === contract.captureEpoch));
  const sources: Record<string, string> = {};
  for (const path of ['scripts/qualification/spool-capacity-continue.ts', 'scripts/qualification/spoolFixture.ts',
    'server/rustEngine/distillationSpoolClient.ts', 'server/rustEngine/distillationSpoolWorker.ts',
    'server/rustEngine/distillationSpoolStore.ts', 'server/rustEngine/distillationProtocol.ts',
    'server/rustEngine/distillationSpoolLimits.ts', 'native/fixtures/sensor-contract-shapes.json']) {
    sources[path] = digest(Buffer.from((await readFile(resolve(path), 'utf8')).replace(/\r\n/gu, '\n')));
  }
  let client: DistillationSpoolClient | undefined; let failure: Error | undefined;
  const started = performance.now(); const cpuStart = process.cpuUsage(); const ingestion = costs();
  const phases: Record<string, unknown> = {}; let peakRss = process.memoryUsage().rss; let acknowledged = retainedBefore;
  const timer = setInterval(() => {peakRss = Math.max(peakRss, process.memoryUsage().rss);}, 250);
  /** Verify bounded public global inventory rather than equating submitted writes with durable work. */
  const inventory = async (): Promise<DistillationDataset[]> => {
    const page = await client!.datasets('0'.repeat(32), 256); assert(!page.hasMore);
    const datasets: DistillationDataset[] = [];
    for (const value of page.datasets) datasets.push(await client!.dataset(value.captureEpoch)); return datasets;
  };
  try {
    let phase = performance.now(); client = new DistillationSpoolClient(root, policy); await client.ready;
    phases['initialRecoveryMs'] = performance.now() - phase;
    const recovered = await inventory(); assert.equal(recovered.reduce((sum, value) => sum + value.retainedChunks, 0), retainedBefore);
    for (const value of recovered) assert(value.state !== 'active');
    const old = recovered.find(value => value.retainedChunks > 0); assert(old);
    const oldSequence = old.retentionFloor;
    const oldLease = await client.lease(old.contract.captureEpoch, [oldSequence], policy.maxLeaseMs);
    const oldDigest = digest((await client.download(old.contract.captureEpoch, oldSequence, oldLease.id)).bytes);
    assert.equal(oldDigest, digest(object(Number(BigInt('0x' + oldSequence)), objectBytes)));
    const append = objects - retainedBefore; const batch = Math.floor(TEACHER_BATCH_BYTES / objectBytes);
    for (let first = 1; first <= append;) {
      assert(performance.now() - started < 3600000, 'continuation total deadline');
      const count = Math.min(batch, append - first + 1); phase = performance.now();
      const stored = await client.ingest(drain(contract, fixture.canonicalJson, first, count, objectBytes), 42);
      settled(stored, first + count - 1, first + count - 1, objectBytes);
      acknowledged = retainedBefore + stored.retainedChunks; sample(ingestion, performance.now() - phase); first += count;
      if (stored.retainedChunks % 1024 === 0 || first > append) {
        await client.renewLease(oldLease.id, policy.maxLeaseMs);
        console.info('[spool-continuation] durable', acknowledged, '/', objects);
      }
    }
    const sealed = await client.ingest(drain(contract, fixture.canonicalJson, append + 1, 0, objectBytes, true), 42);
    settled(sealed, append, append, objectBytes); assert.equal(sealed.state, 'stopped');
    const newLease = await client.lease(contract.captureEpoch, [word(1)], policy.maxLeaseMs);
    const selected = [[old.contract.captureEpoch, oldSequence, oldLease.id, oldDigest],
      [contract.captureEpoch, word(1), newLease.id, digest(object(1, objectBytes))]] as const;
    const full = await inventory(); assert.equal(full.reduce((sum, value) => sum + value.retainedChunks, 0), objects);
    phases['fullInventory'] = full;
    for (const quota of [64, 1]) {
      for (const [, , lease] of selected) await client.renewLease(lease, policy.maxLeaseMs);
      await client.close(); client = undefined; phase = performance.now();
      client = new DistillationSpoolClient(root, distillationSpoolLimitsForGiB(quota)); await client.ready;
      const datasets = await inventory(); const count = datasets.reduce((sum, value) => sum + value.retainedChunks, 0);
      assert.equal(count, Math.min(objects, Math.floor(quota * 1024 ** 3 / objectBytes)));
      assert.equal(datasets.reduce((sum, value) => sum + Number(BigInt('0x' + value.retainedBytes)), 0), count * objectBytes);
      for (const [epoch, sequence, lease, sha256] of selected) assert.equal(digest((await client.download(epoch, sequence, lease)).bytes), sha256);
      phases['restart' + quota] = {milliseconds: performance.now() - phase, exactPinnedDownloads: true, datasets};
    }
    for (const [, , lease] of selected) await client.releaseLease(lease);
    assert(await freeBytes(root) >= BigInt(policy.diskReserveBytes));
  } catch (error) {failure = error instanceof Error ? error : new Error(String(error));}
  finally {
    clearInterval(timer); peakRss = Math.max(peakRss, process.memoryUsage().rss);
    try {await client?.close();} catch (error) {failure ??= error instanceof Error ? error : new Error(String(error));}
    assert.equal(await realpath(workspace), root); const current = await lstat(root);
    assert(!current.isSymbolicLink()); assert.equal(current.dev, identity.dev); assert.equal(current.ino, identity.ino);
    if (!failure) await rm(root, {recursive: true, force: true});
  }
  const cpu = process.cpuUsage(cpuStart);
  await writeFile(output, JSON.stringify({schemaVersion: 1, gate: 'distillation-spool-retained-continuation-v1',
    status: failure ? 'failed' : 'passed', qualificationComplete: !failure && acknowledged === objects,
    scope: 'Synthetic opaque storage through the production worker; global object ceiling across interrupted and new captures. Not full 64 GiB bytes, native codec, HTTP, trainer or integrated soak.',
    source: {revision: execFileSync('git', ['rev-parse', 'HEAD'], {encoding: 'utf8'}).trim(), canonicalLfSha256: sources},
    predecessorCanonicalLfSha256: digest(Buffer.from(failureBytes.replace(/\r\n/gu, '\n'))),
    environment: {node: process.version, platform: platform(), release: release(), totalMemoryBytes: totalmem()},
    options: {quotaGiB, objectBytes, objects, restartQuotaGiB: 1}, retainedBefore, acknowledged,
    elapsedMs: performance.now() - started, sampledPeakRssBytes: peakRss, processCpuMs: {user: cpu.user / 1000, system: cpu.system / 1000},
    ingestion, phases, failure: failure?.message ?? null, workspaceRemoved: !existsSync(workspace),
    physicalFreeBytesBefore: beforeFree.toString(), physicalFreeBytesAfter: (await freeBytes(dirname(root))).toString()}, null, 2) + '\n', {flag: 'wx'});
  console.info('[spool-continuation]', failure ? 'failed' : 'passed', output); if (failure) throw failure;
}
await run();
