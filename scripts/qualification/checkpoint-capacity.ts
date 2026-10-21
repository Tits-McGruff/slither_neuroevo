/** Actual native checkpoint capacity, isolated SQLite commits and opaque archive round-trip. */
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdir, open, readFile, realpath, rm, stat, writeFile } from 'node:fs/promises';
import { dirname, isAbsolute, join, relative, resolve } from 'node:path';
import { CheckpointPersistenceClient } from '../../server/rustEngine/checkpointPersistenceClient.ts';
import { parseManagedCheckpointDescriptor, parseManagedGenerationCommit } from '../../server/rustEngine/checkpointPersistenceProtocol.ts';
import { admitDiskOperation, checkpointDiskAdmissionRequest } from '../../server/rustEngine/diskAdmission.ts';
import type { SavedBrainSummaryV1 } from '../../server/protocol.ts';

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

/** Scalars from one Rust-written portable input; parameter bytes remain in native files. */
interface LibraryFixtureRecord {
  /** Exact original identity independently checked by the production codec. */
  brainId: string;
  /** Exact canonical packed Float32 identity. */
  weightsSha256: string;
  /** Controlled input path below the exclusive managed fixture. */
  relativeFilename: string;
  /** Explicit synthetic/manual display label. */
  name: string;
  /** Exact original length as u64 hex. */
  artifactByteCount: string;
  /** Native canonical parameter count. */
  parameterCount: number;
}

/** Compare complete archive membership without retaining full graph metadata in the host. */
function libraryProjection(record: SavedBrainSummaryV1): Record<string, unknown> {
  return {brainId: record.brainId, name: record.name, createdAtMs: record.createdAtMs,
    artifactByteCount: record.artifactByteCount, weights: record.weights,
    weightsSha256: record.artifact.weightsSha256, sensorSha256: record.artifact.sensorSha256,
    parameterCount: record.artifact.parameterCount, sourceKind: record.artifact.sourceKind};
}

/** Fill the actual production catalog and prove its 101st distinct valid save rejects. */
async function populateLibrary(client: CheckpointPersistenceClient, managed: string): Promise<Record<string, unknown>[]> {
  const index = await report(join(managed, 'library-inputs/index.json'));
  assert.equal(index['schemaVersion'], 1);
  const records = index['records']; assert(Array.isArray(records) && records.length === 101);
  const expected = [];
  for (const [ordinal, value] of records.entries()) {
    assert(value && typeof value === 'object'); const record = value as LibraryFixtureRecord;
    assert.match(record.brainId, /^[0-9a-f]{64}$/u); assert.match(record.weightsSha256, /^[0-9a-f]{64}$/u);
    assert.equal(record.relativeFilename, `library-inputs/${String(ordinal).padStart(3, '0')}.slither-brain`);
    const path = join(managed, record.relativeFilename); assert(!outside(managed, path));
    const request = {action: 'save' as const, path, name: record.name};
    if (ordinal === 100) {
      await assert.rejects(client.savedBrainCatalog(request), /record quota/u);
      await assert.rejects(stat(join(managed, `${record.brainId}.slither-brain`)), {code: 'ENOENT'});
      await assert.rejects(stat(join(managed, `${record.weightsSha256}.hof-weights-v1`)), {code: 'ENOENT'});
      continue;
    }
    const result = await client.savedBrainCatalog(request); assert(result.action === 'save');
    assert.equal(result.record.brainId, record.brainId);
    assert.equal(result.record.artifact.weightsSha256, record.weightsSha256);
    assert.equal(result.record.artifact.parameterCount, record.parameterCount);
    assert.equal(result.record.artifactByteCount, record.artifactByteCount);
    assert.equal(result.record.provenance.sourceKind, 'manual');
    assert.deepEqual(result.record.provenance.details,
      {qualification: 'checkpoint-library-inspector-v1', ordinal, learned: false});
    expected.push(libraryProjection(result.record));
  }
  assert.equal(new Set(expected.map(value => value['brainId'])).size, 100);
  assert.equal(new Set(expected.map(value => value['weightsSha256'])).size, 100);
  return expected.sort((a, b) => String(a['brainId']).localeCompare(String(b['brainId'])));
}

/** Exercise metadata pagination and exact imported membership after all population handoffs. */
async function verifyLibrary(client: CheckpointPersistenceClient, expected: Record<string, unknown>[]): Promise<number> {
  const actual = []; let after: string | null = null; let pages = 0;
  do {
    const response = await client.savedBrainCatalog({action: 'list', after, limit: 25});
    assert(response.action === 'list'); actual.push(...response.page.entries.map(libraryProjection));
    after = response.page.next; pages++;
    assert(pages <= 4 && actual.length <= 100, 'bounded full catalog pagination');
  } while (after !== null);
  assert.equal(pages, 4); assert.deepEqual(actual, expected);
  return pages;
}

/** Qualify one full selected population/codec policy and retain compact evidence. */
async function main(): Promise<void> {
  const args = new Map<string, string>();
  const allowed = ['--binary', '--workspace', '--output', '--population', '--graph', '--mode', '--envelope-mib', '--coexistence'];
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
  const coexistence = args.get('--coexistence') ?? 'none';
  assert(['none', 'library-inspector'].includes(coexistence));
  const libraryEnabled = coexistence === 'library-inspector';
  assert(outside(root, output), 'retained report outside fixture root');
  await realpath(dirname(output));
  await mkdir(root); const owned = await realpath(root);
  const managed = join(owned, 'source'); const imported = join(owned, 'import');
  let source: CheckpointPersistenceClient | undefined; let destination: CheckpointPersistenceClient | undefined;
  let peakHostRssBytes = process.memoryUsage.rss();
  const timer = setInterval(() => {peakHostRssBytes = Math.max(peakHostRssBytes, process.memoryUsage.rss());}, 100);
  const started = performance.now();
  /** Share only admitted scalar dimensions between the two native subprocesses. */
  const common = ['--population', population, '--graph', graph, '--envelope-mib', envelope, '--memory-gib', '4', '--coexistence', coexistence];
  try {
    const request = checkpointDiskAdmissionRequest(BigInt(envelope) * 1024n ** 2n);
    // Original inputs, native originals/shared weights, archive and overlapping import copies.
    const libraryReserve = libraryEnabled ? 8n * 101n * (4_000_000n + 1024n ** 2n + 88n) : 0n;
    const disk = await admitDiskOperation(owned, {...request, sourceSpoolBytes: request.sourceSpoolBytes + libraryReserve});
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
    const expectedLibrary = libraryEnabled ? await populateLibrary(source, managed) : [];
    if (libraryEnabled) await verifyLibrary(source, expectedLibrary);
    const lease = await source.acquireCurrentExportLease();
    const savedBrainsPath = join(owned, 'saved-brains.json');
    if (libraryEnabled) {
      const selected = await source.savedBrainCatalog({action: 'acquireArchive',
        brainIds: expectedLibrary.map(value => String(value['brainId'])), leaseId: lease.operationId});
      assert(selected.action === 'acquireArchive' && selected.entries.length === 100);
      await writeFile(savedBrainsPath, JSON.stringify(selected.entries.map(entry => ({brainId: entry.brainId,
        name: entry.name, createdAtMs: entry.createdAtMs, artifactByteCount: entry.artifactByteCount}))), {flag: 'wx'});
    }
    const leasePath = join(owned, 'lease.json'); await writeFile(leasePath, JSON.stringify(lease), {flag: 'wx'});
    const archivePath = join(owned, 'archive.json');
    let archive: Record<string, unknown>;
    try {
      await nativePhase(binary, [...common, '--mode', mode === 'raw' ? 'archive-raw' : 'archive', '--workspace', imported,
        '--source', managed, '--lease', leasePath, '--report', archivePath,
        ...(libraryEnabled ? ['--saved-brains', savedBrainsPath] : [])]);
      archive = await report(archivePath);
    } finally {
      if (libraryEnabled) await source.savedBrainCatalog({action: 'release', leaseId: lease.operationId});
      await source.releaseExportLease(lease.operationId);
    }
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
    const libraryPages = libraryEnabled ? await verifyLibrary(destination, expectedLibrary) : 0;
    if (libraryEnabled) {
      const receipt = archive['coexistence'] as Record<string, unknown>;
      assert.equal(receipt['savedBrainCount'], 100);
      for (const key of ['sourceInspector', 'replacementInspector']) {
        const inspector = receipt[key] as Record<string, unknown>;
        assert(inspector['wireBytes'] as number > 0); assert.equal(inspector['pureProbe'], false);
        assert.equal(inspector['tracePresent'], true);
      }
      for (const value of handoffs) {
        const inspector = (value as Record<string, unknown>)['inspector'] as Record<string, unknown>;
        assert(inspector['wireBytes'] as number > 0); assert.equal(inspector['pureProbe'], false);
      }
    }
    let deletedLibraryWeights = 0; let preservedSharedWeights = 0;
    if (libraryEnabled) {
      const retainedWeights = new Set<string>();
      for (const value of [save['hallOfFameWeights'], ...handoffs.map(value => (value as Record<string, unknown>)['hallOfFameWeights'])]) {
        if (value && typeof value === 'object') {
          const filename = (value as Record<string, unknown>)['relativeFilename'];
          assert(typeof filename === 'string'); retainedWeights.add(filename);
        }
      }
      for (const entry of expectedLibrary) {
        const deleted = await destination.savedBrainCatalog({action: 'delete', brainId: String(entry['brainId'])});
        assert(deleted.action === 'delete' && deleted.deleted);
        await assert.rejects(stat(join(imported, 'managed', `${String(entry['brainId'])}.slither-brain`)), {code: 'ENOENT'});
        const weights = entry['weights'] as {relativeFilename: string};
        assert.match(weights.relativeFilename, /^[0-9a-f]{64}\.hof-weights-v1$/u);
        const weightPath = join(imported, 'managed', weights.relativeFilename);
        if (retainedWeights.has(weights.relativeFilename)) { assert((await stat(weightPath)).isFile()); preservedSharedWeights++; }
        else { await assert.rejects(stat(weightPath), {code: 'ENOENT'}); deletedLibraryWeights++; }
      }
      for (const filename of retainedWeights) assert((await stat(join(imported, 'managed', filename))).isFile());
      const empty = await destination.savedBrainCatalog({action: 'list', after: null, limit: 25});
      assert(empty.action === 'list'); assert.deepEqual(empty.page, {entries: [], next: null});
      const retained = await destination.selectCurrent(); assert(retained);
      assert.equal(retained.logicalRootSha256, final.logicalRootSha256);
      assert((await stat(join(imported, 'managed', retained.relativeFilename))).isFile());
    }
    const hostHighWaterBytes = process.resourceUsage().maxRSS * 1024;
    assert(Number.isSafeInteger(hostHighWaterBytes) && hostHighWaterBytes > 0, 'OS host RSS high-water counter');
    peakHostRssBytes = Math.max(peakHostRssBytes, hostHighWaterBytes);
    const evidence = {schemaVersion: 2, population: Number(population), graph, mode, envelopeMiB: Number(envelope),
      sqlitePostImportGenerations: handoffs.length, finalCheckpointId: final.logicalRootSha256,
      coexistence, library: libraryEnabled ? {recordCount: 100, overflowRejected: true, overflowPublishedFiles: false,
        importedPages: libraryPages, deletedRecords: 100, deletedLibraryWeights, preservedSharedWeights,
        finalCatalogEmpty: true, populationCheckpointPreserved: true,
        brainIds: expectedLibrary.map(value => value['brainId']),
        originalBytes: expectedLibrary.reduce((sum, value) => sum + Number(BigInt(`0x${String(value['artifactByteCount'])}`)), 0)} : null,
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
