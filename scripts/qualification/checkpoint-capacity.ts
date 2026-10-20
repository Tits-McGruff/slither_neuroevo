/** Actual native checkpoint capacity, isolated SQLite commits and opaque archive round-trip. */
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdir, open, readFile, realpath, rm, stat, writeFile } from 'node:fs/promises';
import { dirname, isAbsolute, join, relative, resolve } from 'node:path';
import { CheckpointPersistenceClient } from '../../server/rustEngine/checkpointPersistenceClient.ts';
import { parseManagedCheckpointDescriptor, parseManagedGenerationCommit } from '../../server/rustEngine/checkpointPersistenceProtocol.ts';
import { admitDiskOperation, checkpointDiskAdmissionRequest } from '../../server/rustEngine/diskAdmission.ts';

/** Parse a bounded scalar report; never read population or archive bytes in Node. */
async function report(path: string): Promise<Record<string, unknown>> {
  assert((await stat(path)).size <= 1024 * 1024, 'bounded qualification report');
  const value: unknown = JSON.parse(await readFile(path, 'utf8'));
  assert(value !== null && typeof value === 'object' && !Array.isArray(value));
  return value as Record<string, unknown>;
}

/** Run one native phase with inherited diagnostics and no shell interpretation. */
async function nativePhase(binary: string, args: string[]): Promise<void> {
  await new Promise<void>((resolvePhase, reject) => {
    const child = spawn(binary, args, {stdio: ['ignore', 'ignore', 'inherit'], windowsHide: true});
    child.once('error', reject);
    child.once('exit', (code, signal) => code === 0 ? resolvePhase() : reject(new Error(`native capacity phase failed: ${code}/${signal}`)));
  });
}

/** Keep reports outside the exclusively owned, removable fixture tree. */
function outside(root: string, output: string): boolean {
  const path = relative(root, output);
  return path.startsWith('..') || isAbsolute(path);
}

/** Qualify one full selected population/codec policy and retain compact evidence. */
async function main(): Promise<void> {
  const args = new Map<string, string>();
  const allowed = ['--binary', '--workspace', '--output', '--population', '--graph', '--mode', '--envelope-mib'];
  for (let index = 2; index < process.argv.length; index += 2) {
    const flag = process.argv[index]; const value = process.argv[index + 1];
    assert(flag && allowed.includes(flag) && value !== undefined && !args.has(flag), 'unique known flag/value');
    args.set(flag, value);
  }
  /** Require a fully specified run so retained evidence has no implicit dimensions. */
  const get = (flag: string): string => {const value = args.get(flag); assert(value, `missing ${flag}`); return value;};
  const binary = await realpath(get('--binary'));
  const root = resolve(get('--workspace')); const output = resolve(get('--output'));
  const population = get('--population'); const graph = get('--graph'); const mode = get('--mode'); const envelope = get('--envelope-mib');
  assert(['55', '300'].includes(population)); assert(['small', 'large'].includes(graph));
  assert(['compressed', 'raw'].includes(mode)); assert(['512', '768'].includes(envelope));
  assert(outside(root, output), 'retained report outside fixture root');
  await realpath(dirname(output));
  await mkdir(root); const owned = await realpath(root);
  const managed = join(owned, 'source'); const imported = join(owned, 'import');
  let source: CheckpointPersistenceClient | undefined; let destination: CheckpointPersistenceClient | undefined;
  let peakHostRssBytes = process.memoryUsage.rss();
  const timer = setInterval(() => {peakHostRssBytes = Math.max(peakHostRssBytes, process.memoryUsage.rss());}, 100);
  const started = performance.now();
  /** Share only admitted scalar dimensions between the two native subprocesses. */
  const common = ['--population', population, '--graph', graph, '--envelope-mib', envelope, '--memory-gib', '4'];
  try {
    const disk = await admitDiskOperation(owned, checkpointDiskAdmissionRequest(BigInt(envelope) * 1024n ** 2n));
    const savePath = join(owned, 'save.json');
    await nativePhase(binary, [...common, '--mode', mode, '--workspace', managed, '--report', savePath]);
    const save = await report(savePath);
    assert.equal(save['initialRestoreExact'], true); assert.equal(save['generationRestoreExact'], true);
    const initial = parseManagedCheckpointDescriptor(save['runStart']); const generation = parseManagedCheckpointDescriptor(save['generation']);
    const fields = save['generationCommit']; assert(fields !== null && typeof fields === 'object');
    const commit = parseManagedGenerationCommit({...fields, hallOfFameWeights: save['hallOfFameWeights']}, generation);
    assert(commit);
    source = new CheckpointPersistenceClient({databasePath: join(owned, 'source.sqlite'), managedRootPath: managed});
    await source.commit(initial, null, true); await source.commit(generation, commit);
    const lease = await source.acquireCurrentExportLease();
    const leasePath = join(owned, 'lease.json'); await writeFile(leasePath, JSON.stringify(lease), {flag: 'wx'});
    const archivePath = join(owned, 'archive.json');
    let archive: Record<string, unknown>;
    try {
      await nativePhase(binary, [...common, '--mode', mode === 'raw' ? 'archive-raw' : 'archive', '--workspace', imported,
        '--source', managed, '--lease', leasePath, '--report', archivePath]);
      archive = await report(archivePath);
    } finally {await source.releaseExportLease(lease.operationId);}
    assert.equal(archive['sourceSha256'], save['sourceSha256']);
    assert.equal(archive['continuationSha256'], save['continuationSha256']);
    assert.equal(archive['importLimitsPreserved'], true); assert.equal(archive['idempotentImportExact'], true);
    assert.equal(archive['nativeAdmissionBeforeAllocation'], true);
    const restored = parseManagedCheckpointDescriptor(archive['descriptor']);
    destination = new CheckpointPersistenceClient({databasePath: join(owned, 'import.sqlite'), managedRootPath: join(imported, 'managed')});
    await destination.commitImport(restored, archive['inventory']);
    const selected = await destination.selectCurrent();
    assert(selected); assert.equal(selected.logicalRootSha256, generation.logicalRootSha256);
    assert.equal(archive['schemaVersion'], 2);
    const handoffs: unknown[] = archive['postImportGenerations'] as unknown[];
    assert(Array.isArray(handoffs) && handoffs.length === 3, 'three post-import handoffs');
    let latest = restored;
    for (const value of handoffs) {
      assert(value && typeof value === 'object');
      const handoff = value as Record<string, unknown>;
      assert.equal(handoff['independentRestoreExact'], true);
      const next = parseManagedCheckpointDescriptor(handoff['descriptor']);
      assert.equal(BigInt(`0x${next.generation}`), BigInt(`0x${latest.generation}`) + 1n);
      const fields = handoff['generationCommit']; assert(fields && typeof fields === 'object');
      const commit = parseManagedGenerationCommit({...fields, hallOfFameWeights: handoff['hallOfFameWeights']}, next);
      assert(commit); await destination.commit(next, commit); latest = next;
    }
    const final = await destination.selectCurrent();
    assert(final); assert.equal(final.logicalRootSha256, latest.logicalRootSha256);
    const hostHighWaterBytes = process.resourceUsage().maxRSS * 1024;
    assert(Number.isSafeInteger(hostHighWaterBytes) && hostHighWaterBytes > 0, 'OS host RSS high-water counter');
    peakHostRssBytes = Math.max(peakHostRssBytes, hostHighWaterBytes);
    const evidence = {schemaVersion: 2, population: Number(population), graph, mode, envelopeMiB: Number(envelope),
      sqlitePostImportGenerations: handoffs.length, finalCheckpointId: final.logicalRootSha256,
      hostRssMeasurement: 'os-high-water-and-100ms-samples',

      native: save, archive, sqliteCommitted: true, sqliteImported: true, peakHostRssBytes,
      checkpointDiskDecision: JSON.parse(JSON.stringify(disk, (_key, value: unknown) => typeof value === 'bigint' ? value.toString() : value)) as unknown,
      totalSeconds: (performance.now() - started) / 1000};
    const file = await open(output, 'wx');
    try {await file.writeFile(JSON.stringify(evidence, null, 2) + '\n'); await file.sync();} finally {await file.close();}
    await destination.close(); destination = undefined; await source.close(); source = undefined;
    assert.equal(await realpath(owned), owned); assert(!outside(owned, managed)); assert(!outside(owned, imported));
    await rm(owned, {recursive: true});
    console.log(`Qualified ${population} ${graph} checkpoint ${mode}; retained ${output}`);
  } finally {
    clearInterval(timer); await destination?.close(); await source?.close();
  }
}

await main();
